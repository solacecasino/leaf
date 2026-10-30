//! Upscaling through an adapter (`filters = ["upscale"]`, protocol 3): when
//! a picture is shown bigger than it is, an upscaler adapter can make the
//! bigger version instead of Leaf's own scaler, on the GPU. Leaf never
//! waits for it to start: until it says it's ready, and whenever it fails,
//! pages are scaled by Leaf as always.
//!
//! The pixels go through two files in a private temporary folder, not the
//! JSON pipe: the source (RGB, 3 bytes a pixel) and the result (4 bytes a
//! pixel, 0RGB as Leaf draws it). Only the paths and sizes are in the
//! message. Both sides map the files into memory, so a 4K page's 42 MB is
//! copied once each way instead of written to a file and read back.

use crate::adapters::Link;
use crate::loader::{BAND, Scaled};
use image::RgbImage;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// How long Leaf waits for one upscaled picture before scaling it itself.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Source rows read beyond a band's own, so filters see past its edges.
const MARGIN: u32 = 8;

/// How the upscaler should treat a page: line art (manga, webtoons) has its
/// lines retraced and kept thin; colour comics are upscaled softly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Lines,
    Soft,
}

/// How hard the upscaler works the lines: `upscale_strength` in
/// settings.toml. Adapters that don't know it upscale as they always have.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Strength {
    Gentle,
    Normal,
    Strong,
}

impl Strength {
    pub fn from_setting(s: &str) -> Strength {
        match s {
            "gentle" => Strength::Gentle,
            "strong" => Strength::Strong,
            _ => Strength::Normal,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Strength::Gentle => "gentle",
            Strength::Normal => "normal",
            Strength::Strong => "strong",
        }
    }
}

pub struct Upscaler {
    pub name: String,
    link: Arc<Link>,
    dir: PathBuf,
    ready: AtomicBool,
    /// Set after a failure, so Leaf stops asking until it's switched on again.
    failed: AtomicBool,
}

impl Upscaler {
    pub fn new(link: Arc<Link>) -> Arc<Upscaler> {
        let dir = std::env::temp_dir().join(format!("leaf-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        Arc::new(Upscaler { name: link.name().to_owned(), link, dir, ready: AtomicBool::new(false), failed: AtomicBool::new(false) })
    }

    /// Asks the adapter to get ready (start its GPU) and calls `ready`
    /// when it is, from another thread.
    pub fn warm_up(self: &Arc<Self>, ready: impl FnOnce(Result<String, String>) + Send + 'static) {
        let me = self.clone();
        std::thread::spawn(move || {
            let result = me.link.request("upscale.ready", json!({}), Duration::from_secs(30));
            let answer = result.map(|v| v["device"].as_str().unwrap_or("ready").to_owned());
            me.ready.store(answer.is_ok(), Ordering::SeqCst);
            me.failed.store(answer.is_err(), Ordering::SeqCst);
            ready(answer);
        });
    }

    pub fn usable(&self) -> bool {
        self.ready.load(Ordering::SeqCst) && !self.failed.load(Ordering::SeqCst)
    }

    /// `img` upscaled to `w`×`h` (rows `rows` of it, for a webtoon band).
    /// Called from the page loader, one at a time.
    pub fn run(&self, img: &RgbImage, w: u32, h: u32, rows: Option<(u32, u32)>, style: Style, strength: Strength) -> Result<Scaled, String> {
        let (iw, ih) = img.dimensions();
        let (r0, r1) = rows.map_or((0, h), |(a, b)| (a.min(h), b.min(h)));
        if r1 <= r0 {
            return Ok(Scaled { w, h: 0, px: Vec::new() });
        }
        // The source rows the output rows come from, plus a margin.
        let to_src = |r: u32| r as f64 * ih as f64 / h as f64;
        let s0 = (to_src(r0).floor() as u32).saturating_sub(MARGIN);
        let s1 = ((to_src(r1).ceil() as u32) + MARGIN).min(ih);
        let started = std::time::Instant::now();
        let crop = &img.as_raw()[(s0 * iw * 3) as usize..(s1 * iw * 3) as usize];
        let (input, output) = (self.dir.join("page.rgb"), self.dir.join("page.out"));
        shared(&input, crop.len() as u64)?.copy_from_slice(crop);
        let out_file = sized(&output, (w * (r1 - r0)) as u64 * 4)?;
        let rect = [0.0, to_src(r0) - s0 as f64, iw as f64, to_src(r1) - s0 as f64];
        let params = json!({
            "input": input, "width": iw, "height": s1 - s0, "format": "rgb8",
            "output": output, "out_width": w, "out_height": r1 - r0, "out_format": "0rgb32",
            "rect": rect, "style": if style == Style::Lines { "lines" } else { "soft" }, "strength": strength.name(),
        });
        let sent = started.elapsed();
        let result = self.link.request("page.upscale", params, TIMEOUT).and_then(|answer| {
            let answered = started.elapsed();
            let n = (w * (r1 - r0)) as usize;
            // The file is reused from page to page, so the adapter says how
            // much it wrote: one that answers without writing isn't shown.
            if answer["written"].as_u64() != Some(n as u64 * 4) || out_file.metadata().map_err(|e| e.to_string())?.len() != n as u64 * 4 {
                return Err(format!("{} didn't send back the picture", self.name));
            }
            // SAFETY: the adapter has answered, so it's done with the file.
            let map = unsafe { memmap2::Mmap::map(&out_file) }.map_err(|e| e.to_string())?;
            // 0RGB, little-endian, as Leaf keeps pixels on every system it
            // runs on. An adapter can't put anything in the byte Leaf leaves
            // empty.
            // SAFETY: any four bytes are a u32; a mapping starts on a page,
            // so `body` is all of it (checked).
            let (head, body, _) = unsafe { map.align_to::<u32>() };
            if !head.is_empty() || body.len() != n {
                return Err("the upscaled picture couldn't be read".into());
            }
            let px = body.iter().map(|&p| u32::from_le(p) & 0x00ff_ffff).collect();
            if crate::series::env("TIMING").is_some() {
                eprintln!(
                    "leaf9000: upscaled {w}x{}: sent {sent:?}, answered {:?} (adapter {} ms), taken in {:?}",
                    r1 - r0,
                    answered - sent,
                    answer["total_ms"].as_f64().map_or("?".into(), |t| format!("{t:.1}")),
                    started.elapsed() - answered
                );
            }
            Ok(Scaled { w, h: r1 - r0, px })
        });
        if let Err(e) = &result {
            if !self.failed.swap(true, Ordering::SeqCst) {
                crate::log::push(crate::series::NAME, format!("upscaling stopped: {e}"));
            }
        }
        result
    }
}

impl Drop for Upscaler {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// `path`, `len` bytes long (made or resized as needed, kept otherwise:
/// its memory stays mapped in from one page to the next).
fn sized(path: &std::path::Path, len: u64) -> Result<std::fs::File, String> {
    let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() != len {
        file.set_len(len).map_err(|e| e.to_string())?;
    }
    Ok(file)
}

/// `path`, `len` bytes long, mapped in to write to.
fn shared(path: &std::path::Path, len: u64) -> Result<memmap2::MmapMut, String> {
    let file = sized(path, len)?;
    // SAFETY: Leaf's own temporary file, written before the adapter is asked.
    unsafe { memmap2::MmapMut::map_mut(&file) }.map_err(|e| e.to_string())
}

/// A band's rows, for [`Upscaler::run`].
pub fn band_rows(band: u32) -> (u32, u32) {
    (band * BAND, (band + 1) * BAND)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::{Target, scaled_size};
    use crate::state::Fit;
    use std::time::Instant;

    fn manga_page(w: u32, h: u32) -> RgbImage {
        RgbImage::from_fn(w, h, |x, y| {
            let ink = x % 400 < 3 || y % 500 < 3 || ((x as f32 - y as f32 * 0.7) - 100.0).abs() < 1.0 || (x % 6 < 2 && y % 6 < 2 && x > 410 && x < 790);
            image::Rgb(if ink { [20; 3] } else { [255; 3] })
        })
    }

    /// The protocol, with a stand-in adapter that answers with a grey
    /// picture of the size asked for (or fails, for the second one).
    #[cfg(unix)]
    #[test]
    fn upscales_through_an_adapter_and_falls_back() {
        let dir = std::env::temp_dir().join(format!("leaf-upscaler-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("fake.sh");
        std::fs::write(
            &script,
            r#"#!/bin/sh
while read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  [ -z "$id" ] && continue
  case "$line" in
    *upscale.ready*) printf '{"id":%s,"result":{"device":"Fake GPU"}}\n' "$id" ;;
    *'"strength":"'*page.upscale*|*page.upscale*'"strength":"'*)
      out=$(printf '%s' "$line" | sed -n 's/.*"output":"\([^"]*\)".*/\1/p')
      w=$(printf '%s' "$line" | sed -n 's/.*"out_width":\([0-9]*\).*/\1/p')
      h=$(printf '%s' "$line" | sed -n 's/.*"out_height":\([0-9]*\).*/\1/p')
      head -c $((w * h * 4)) /dev/zero | LC_ALL=C tr '\0' '\200' > "$out"
      printf '{"id":%s,"result":{"written":%s}}\n' "$id" $((w * h * 4)) ;;
  esac
done
"#,
        )
        .unwrap();
        std::process::Command::new("chmod").arg("+x").arg(&script).status().unwrap();
        let up = Upscaler::new(crate::adapters::test_link(&script));
        assert!(!up.usable(), "not used until it says it's ready");
        let (tx, rx) = std::sync::mpsc::channel();
        up.warm_up(move |r| drop(tx.send(r)));
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap().unwrap(), "Fake GPU");
        assert!(up.usable());
        let page = manga_page(300, 400);
        let whole = up.run(&page, 600, 800, None, Style::Lines, Strength::Normal).unwrap();
        assert_eq!((whole.w, whole.h, whole.px.len()), (600, 800, 480_000));
        // The byte Leaf leaves empty is cleared whatever the adapter sent.
        assert!(whole.px.iter().all(|&p| p == 0x0080_8080));
        let band = up.run(&page, 600, 800, Some(band_rows(1)), Style::Soft, Strength::Strong).unwrap();
        assert_eq!((band.w, band.h), (600, 288), "the last band is what's left");

        // An adapter that answers wrongly is used no more.
        let liar = dir.join("liar.sh");
        std::fs::write(&liar, "#!/bin/sh\nwhile read -r line; do id=$(printf '%s' \"$line\" | sed -n 's/.*\"id\":\\([0-9]*\\).*/\\1/p'); [ -n \"$id\" ] && printf '{\"id\":%s,\"result\":{}}\\n' \"$id\"; done\n").unwrap();
        std::process::Command::new("chmod").arg("+x").arg(&liar).status().unwrap();
        let bad = Upscaler::new(crate::adapters::test_link(&liar));
        let (tx, rx) = std::sync::mpsc::channel();
        bad.warm_up(move |r| drop(tx.send(r)));
        rx.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
        assert!(bad.run(&page, 600, 800, None, Style::Lines, Strength::Normal).is_err());
        assert!(!bad.usable(), "after a failure Leaf scales by itself");
    }

    /// Leaf's own scaler against the real upscaler, end to end, on a page
    /// shown on a 4K screen. Runs when `LEAF_UPSCALER` names its program
    /// (`cargo test --release four_k -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn four_k_pages_cpu_and_gpu() {
        let Some(program) = crate::series::env("UPSCALER") else { return };
        let page = manga_page(1200, 1700);
        let target = Target { w: 2711, h: 3840, fit: Fit::Page };
        let (w, h) = scaled_size(1200, 1700, target);
        let median = |mut v: Vec<f64>| {
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        };
        // Each timed until the picture is ready, as Leaf would show it
        // (letting it go afterwards isn't counted, for either).
        let timed = |f: &mut dyn FnMut() -> Scaled| {
            let t = Instant::now();
            let made = f();
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            drop(std::hint::black_box(made));
            ms
        };
        let cpu = median((0..10).map(|_| timed(&mut || crate::loader::scale_for_test(&page, target))).collect());
        let up = Upscaler::new(crate::adapters::test_link(std::path::Path::new(&program)));
        let (tx, rx) = std::sync::mpsc::channel();
        up.warm_up(move |r| drop(tx.send(r)));
        let device = rx.recv_timeout(Duration::from_secs(30)).unwrap().unwrap();
        for style in [Style::Lines, Style::Soft] {
            let gpu = median((0..15).map(|_| timed(&mut || up.run(&page, w, h, None, style, Strength::Normal).unwrap())).collect());
            eprintln!("{w}x{h}: Leaf's CPU scaler {cpu:.1} ms; {device}, {style:?}, end to end {gpu:.1} ms");
        }
        let band_times = (0..8).map(|i| timed(&mut || up.run(&page, w, h, Some(band_rows(i)), Style::Lines, Strength::Normal).unwrap())).collect();
        eprintln!("a 512-row band of it: {:.1} ms", median(band_times));
    }

    /// Leaf's own memory holding five 4K pages (it keeps three ready for a
    /// comic, six for two-page manga), made by its scaler and then through
    /// the upscaler. Runs when `LEAF_UPSCALER` names the upscaler's program
    /// (`cargo test --release four_k_memory -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn four_k_memory() {
        let Some(program) = crate::series::env("UPSCALER") else { return };
        let mb = || crate::platform::memory_use().map_or(0.0, |(now, _)| now as f64 / 1e6);
        let page = manga_page(1200, 1700);
        let target = Target { w: 2711, h: 3840, fit: Fit::Page };
        let (w, h) = scaled_size(1200, 1700, target);
        let start = mb();
        let held: Vec<Scaled> = (0..5).map(|_| crate::loader::scale_for_test(&page, target)).collect();
        let cpu = mb();
        drop(held);
        let up = Upscaler::new(crate::adapters::test_link(std::path::Path::new(&program)));
        let (tx, rx) = std::sync::mpsc::channel();
        up.warm_up(move |r| drop(tx.send(r)));
        rx.recv_timeout(Duration::from_secs(30)).unwrap().unwrap();
        let before = mb();
        let held: Vec<Scaled> = (0..5).map(|_| up.run(&page, w, h, None, Style::Lines, Strength::Normal).unwrap()).collect();
        let gpu = mb();
        eprintln!("five {w}x{h} pages held: Leaf's scaler +{:.0} MB; through the upscaler +{:.0} MB", cpu - start, gpu - before);
        drop(held);
    }
}
