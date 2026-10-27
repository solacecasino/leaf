//! Decodes and scales pages on a background thread. The UI says which pages
//! it wants, each at the size it'll be shown, most important first; the
//! thread works through them and wakes the UI with each finished page. With
//! nothing to do, it lays out more of a text book.

use crate::book::{Book, Page};
use crate::upscale::{Style, Upscaler};
use crate::state::Fit;
use image::RgbImage;
use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread;

/// Full-size pages kept for re-scaling when the window resizes, and for the
/// bands of a webtoon strip (two, so the strip's next page is ready too).
const DECODED_KEPT: usize = 2;
/// The height, in pixels, of the bands a webtoon strip is made in: only the
/// bands near the window are kept, not whole strips several screens tall.
pub const BAND: u32 = 512;

/// A page scaled for the window, as 0RGB pixels.
pub struct Scaled {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u32>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub struct Target {
    pub w: u32,
    pub h: u32,
    pub fit: Fit,
}

/// Which part of a page: all of it, one band of a webtoon strip, or all of
/// it enlarged for the magnifier.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Part {
    Whole,
    Band(u32),
    Zoom,
}

/// A page, or part of one.
pub type Piece = (usize, Part);

/// What a page is like, for laying out spreads and telling manga from comics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageInfo {
    /// The page's own size (its proportions are what matter).
    pub w: u32,
    pub h: u32,
    /// Black and white, like most manga, rather than colour.
    pub gray: bool,
}

impl PageInfo {
    /// Wider than tall: a two-page spread printed as one image.
    pub fn wide(&self) -> bool {
        self.w > self.h + self.h / 10
    }

    /// Much taller than wide: a strip of a webtoon.
    pub fn tall(&self) -> bool {
        self.h > self.w * 2
    }
}

pub enum Loaded {
    Page(Done),
    /// More of book `.0` (a text book) is laid out.
    Grew(u64),
}

pub struct Done {
    pub book: u64,
    pub piece: Piece,
    pub target: Target,
    pub image: Result<Arc<Scaled>, String>,
    pub info: Option<PageInfo>,
}

#[derive(Default)]
struct Job {
    book: Option<(u64, Arc<Book>)>,
    wanted: VecDeque<(Piece, Target)>,
    busy: Option<(u64, Piece, Target)>,
    /// An upscaler adapter to make enlarged pictures with, and how; and a
    /// count of changes to it, so pictures made before a change aren't reused.
    upscaler: Option<(Arc<Upscaler>, Style)>,
    upscaler_changes: u64,
}

type Shared = Arc<(Mutex<Job>, Condvar)>;

pub struct Loader {
    shared: Shared,
}

impl Loader {
    /// `deliver` hands each finished page to the UI; it returns false once
    /// the UI is gone, which ends the thread.
    pub fn new(deliver: impl Fn(Loaded) -> bool + Send + 'static) -> Self {
        let shared: Shared = Arc::default();
        let worker = shared.clone();
        thread::Builder::new()
            .name("pages".into())
            .spawn(move || work(worker, deliver))
            .expect("start the page loader");
        Self { shared }
    }

    /// Replaces the wanted list. A piece already being worked on isn't queued again.
    pub fn want(&self, id: u64, book: &Arc<Book>, pieces: Vec<(Piece, Target)>) {
        let (lock, cv) = &*self.shared;
        let mut job = lock.lock().unwrap_or_else(|e| e.into_inner());
        let busy = job.busy;
        job.book = Some((id, book.clone()));
        job.wanted = pieces.into_iter().filter(|&(p, t)| busy != Some((id, p, t))).collect();
        cv.notify_one();
    }

    /// Makes enlarged pictures with `upscaler` from now on (or Leaf's own
    /// scaler, with None).
    pub fn set_upscaler(&self, upscaler: Option<(Arc<Upscaler>, Style)>) {
        let mut job = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        let same = match (&job.upscaler, &upscaler) {
            (Some((a, s)), Some((b, t))) => Arc::ptr_eq(a, b) && s == t && a.usable() == b.usable(),
            (None, None) => true,
            _ => false,
        };
        if !same {
            job.upscaler = upscaler;
            job.upscaler_changes += 1;
        }
    }
}

struct Made {
    id: u64,
    changes: u64,
    piece: Piece,
    target: Target,
    image: Weak<Scaled>,
    info: Option<PageInfo>,
}

/// A page as decoded (or drawn, for PDF and text pages, at the size given).
struct Decoded {
    id: u64,
    page: usize,
    drawn_for: Option<Target>,
    image: Arc<RgbImage>,
    info: PageInfo,
}

enum Work {
    Piece(u64, Arc<Book>, Piece, Target, Option<(Arc<Upscaler>, Style)>, u64),
    Grow(u64, Arc<Book>),
}

fn work(shared: Shared, deliver: impl Fn(Loaded) -> bool) {
    let mut decoded: VecDeque<Decoded> = VecDeque::new();
    // Pages already scaled, held weakly: a repeat request (one asked again
    // before the first answer reached the UI) is answered without redoing
    // the work, and a page the UI let go of costs nothing here.
    let mut made: VecDeque<Made> = VecDeque::new();
    // The UI hears about layout progress now and then, not after every step.
    let mut told = std::time::Instant::now();
    loop {
        let next = {
            let (lock, cv) = &*shared;
            let mut job = lock.lock().unwrap_or_else(|e| e.into_inner());
            job.busy = None;
            loop {
                if let Some((id, book)) = job.book.clone() {
                    if let Some((piece, target)) = job.wanted.pop_front() {
                        job.busy = Some((id, piece, target));
                        break Work::Piece(id, book, piece, target, job.upscaler.clone(), job.upscaler_changes);
                    }
                    if !book.complete() {
                        break Work::Grow(id, book);
                    }
                }
                job = cv.wait(job).unwrap_or_else(|e| e.into_inner());
            }
        };
        let (id, book, piece, target, upscaler, changes) = match next {
            Work::Grow(id, book) => {
                // A text book being read: lay out a few more pages, then
                // look for wanted pages again.
                let _ = catch_unwind(AssertUnwindSafe(|| book.grow()));
                if book.complete() || told.elapsed().as_millis() > 250 {
                    if !deliver(Loaded::Grew(id)) {
                        return;
                    }
                    told = std::time::Instant::now();
                }
                continue;
            }
            Work::Piece(id, book, piece, target, upscaler, changes) => (id, book, piece, target, upscaler, changes),
        };
        let earlier = made.iter().find(|m| (m.id, m.piece, m.target, m.changes) == (id, piece, target, changes));
        if let Some((image, info)) = earlier.and_then(|m| Some((m.image.upgrade()?, m.info))) {
            if !deliver(Loaded::Page(Done { book: id, piece, target, image: Ok(image), info })) {
                return;
            }
            continue;
        }
        let started = std::time::Instant::now();
        let page = piece.0;
        // A broken image must never take the reader down with it.
        let result = catch_unwind(AssertUnwindSafe(|| {
            load(&mut decoded, id, &book, page, target).map(|(img, info, drawn)| {
                let rows = match piece.1 {
                    Part::Band(i) => Some(crate::upscale::band_rows(i)),
                    Part::Whole | Part::Zoom => None,
                };
                // A picture shown bigger than it is goes to the upscaler,
                // when there is one; anything it can't do, Leaf does.
                let (w, h) = scaled_size(img.width(), img.height(), target);
                let upscaled = match &upscaler {
                    Some((u, style)) if !drawn && w > img.width() && u.usable() => u.run(&img, w, h, rows, *style).ok(),
                    _ => None,
                };
                (Arc::new(upscaled.unwrap_or_else(|| scale(&img, target, rows))), info)
            })
        }))
        .unwrap_or_else(|_| Err(format!("{}: the decoder crashed", book.page_name(page))));
        let (image, info) = match result {
            Ok((image, info)) => {
                made.push_back(Made { id, changes, piece, target, image: Arc::downgrade(&image), info: Some(info) });
                if made.len() > 24 {
                    made.pop_front();
                }
                (Ok(image), Some(info))
            }
            Err(e) => (Err(e), None),
        };
        if std::env::var_os("LEAF_TIMING").is_some() {
            eprintln!("leaf: page {} {:?} decoded and scaled in {:?}", page + 1, piece.1, started.elapsed());
        }
        if !deliver(Loaded::Page(Done { book: id, piece, target, image, info })) {
            return;
        }
    }
}

/// A page decoded (or drawn at `target`, the last value true), and what
/// it's like.
fn load(decoded: &mut VecDeque<Decoded>, id: u64, book: &Book, page: usize, target: Target) -> Result<(Arc<RgbImage>, PageInfo, bool), String> {
    let kept = decoded.iter().find(|d| d.id == id && d.page == page && d.drawn_for.is_none_or(|t| t == target));
    if let Some(d) = kept {
        return Ok((d.image.clone(), d.info, d.drawn_for.is_some()));
    }
    let (image, drawn_for) = match book.page(page, target)? {
        // Drawn at the size it's shown; kept only for the other bands of
        // the same page.
        Page::Drawn(img) => (Arc::new(img), Some(target)),
        Page::Bytes(bytes) => (Arc::new(decode(&bytes).map_err(|e| format!("{}: {e}", book.page_name(page)))?), None),
    };
    let info = describe(&image);
    decoded.push_back(Decoded { id, page, drawn_for, image: image.clone(), info });
    while decoded.len() > DECODED_KEPT {
        decoded.pop_front();
    }
    Ok((image, info, drawn_for.is_some()))
}

/// Decodes a picture with Leaf's own decoders, within limits a hostile file
/// can't talk its way past, then with macOS's for everything else.
pub fn decode(bytes: &[u8]) -> Result<RgbImage, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(40_000);
    limits.max_image_height = Some(120_000);
    limits.max_alloc = Some(1 << 30);
    reader.limits(limits);
    match reader.decode() {
        Ok(img) => Ok(img.into_rgb8()),
        Err(e) => {
            // The system's decoders on a Mac; Leaf's own for the rest elsewhere.
            #[cfg(target_os = "macos")]
            let more = crate::native::decode(bytes);
            #[cfg(not(target_os = "macos"))]
            let more = crate::portable::images::decode(bytes);
            more.map_err(|more| if matches!(e, image::ImageError::Unsupported(_)) { more } else { e.to_string() })
        }
    }
}

/// A page's size, and whether it's black and white: fewer than 3% (or
/// settings.toml's `color_threshold`) of a grid of sample points are
/// clearly coloured.
fn describe(img: &RgbImage) -> PageInfo {
    let threshold = crate::settings::get().color_threshold;
    let (w, h) = img.dimensions();
    let (step_x, step_y) = ((w / 64).max(1), (h / 64).max(1));
    let (mut seen, mut colour) = (0u32, 0u32);
    for y in (0..h).step_by(step_y as usize) {
        for x in (0..w).step_by(step_x as usize) {
            let p = img.get_pixel(x, y);
            let (hi, lo) = (p[0].max(p[1]).max(p[2]), p[0].min(p[1]).min(p[2]));
            seen += 1;
            if hi - lo > 48 {
                colour += 1;
            }
        }
    }
    PageInfo { w, h, gray: (colour as f64) * 100.0 < seen as f64 * threshold }
}

pub fn scaled_size(iw: u32, ih: u32, t: Target) -> (u32, u32) {
    let s = match t.fit {
        Fit::Page => (t.w as f64 / iw as f64).min(t.h as f64 / ih as f64),
        Fit::Width => t.w as f64 / iw as f64,
    };
    (((iw as f64 * s).round() as u32).max(1), ((ih as f64 * s).round() as u32).max(1))
}

/// Fixed-point weights: 1.0 is 1 << 14.
const ONE: i32 = 1 << 14;

/// For each output pixel along one axis: the first source pixel it reads and
/// its weights. A triangle filter as wide as the scale factor, so shrinking
/// averages every source pixel (no shimmering moiré on screentone) and
/// enlarging is bilinear.
fn taps(src: u32, dst: u32) -> Vec<(usize, Vec<i32>)> {
    let scale = src as f64 / dst as f64;
    let radius = scale.max(1.0);
    (0..dst)
        .map(|i| {
            let center = (i as f64 + 0.5) * scale;
            let lo = (center - radius).floor().max(0.0) as usize;
            let hi = ((center + radius).ceil() as usize).min(src as usize).max(lo + 1);
            let raw: Vec<f64> =
                (lo..hi).map(|j| (1.0 - ((j as f64 + 0.5) - center).abs() / radius).max(0.0)).collect();
            let sum: f64 = raw.iter().sum::<f64>().max(f64::MIN_POSITIVE);
            let mut w: Vec<i32> = raw.iter().map(|r| (r / sum * ONE as f64).round() as i32).collect();
            // Make the weights sum to exactly one, so flat areas stay flat.
            let fix = ONE - w.iter().sum::<i32>();
            if let Some(big) = w.iter_mut().max() {
                *big += fix;
            }
            (lo, w)
        })
        .collect()
}

fn to_u8(acc: i32) -> u8 {
    ((acc + ONE / 2) >> 14).clamp(0, 255) as u8
}

/// Scales in two passes, across then down, in integers: 1.6-2x faster than
/// the image crate's float resize, which was most of the time a page took
/// to appear. `rows` makes only those rows of the result (a band of a
/// webtoon strip), reading only the source rows they need.
fn scale(img: &RgbImage, t: Target, rows: Option<(u32, u32)>) -> Scaled {
    let (iw, ih) = img.dimensions();
    let (w, h) = scaled_size(iw, ih, t);
    let (r0, r1) = rows.map_or((0, h), |(a, b)| (a.min(h), b.min(h)));
    let down: Vec<(usize, Vec<i32>)> = taps(ih, h).into_iter().skip(r0 as usize).take((r1 - r0) as usize).collect();
    let src = img.as_raw();
    // The source rows those output rows read.
    let first = down.first().map_or(0, |d| d.0);
    let end = down.iter().map(|(s, wts)| s + wts.len()).max().unwrap_or(0);

    let across = taps(iw, w);
    let mut mid = vec![0u8; w as usize * (end - first) * 3];
    let rows_in = src.chunks_exact(iw as usize * 3).skip(first).take(end - first);
    for (row, out) in rows_in.zip(mid.chunks_exact_mut(w as usize * 3)) {
        for ((start, weights), px) in across.iter().zip(out.as_chunks_mut::<3>().0) {
            let (mut r, mut g, mut b) = (0, 0, 0);
            for (k, &wt) in weights.iter().enumerate() {
                let s = (start + k) * 3;
                r += row[s] as i32 * wt;
                g += row[s + 1] as i32 * wt;
                b += row[s + 2] as i32 * wt;
            }
            px.copy_from_slice(&[to_u8(r), to_u8(g), to_u8(b)]);
        }
    }

    let stride = w as usize * 3;
    let mut acc = vec![0i32; stride];
    let mut px = Vec::with_capacity(w as usize * (r1 - r0) as usize);
    for (start, weights) in down {
        acc.fill(0);
        for (k, &wt) in weights.iter().enumerate() {
            let row = &mid[(start - first + k) * stride..][..stride];
            for (a, &v) in acc.iter_mut().zip(row) {
                *a += v as i32 * wt;
            }
        }
        px.extend(acc.as_chunks::<3>().0.iter().map(|c| {
            (to_u8(c[0]) as u32) << 16 | (to_u8(c[1]) as u32) << 8 | to_u8(c[2]) as u32
        }));
    }
    Scaled { w, h: r1 - r0, px }
}

/// Leaf's own scaler, for comparing the upscaler with.
#[cfg(test)]
pub fn scale_for_test(img: &RgbImage, t: Target) -> Scaled {
    scale(img, t, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_color_stays_flat() {
        let img = RgbImage::from_pixel(37, 53, image::Rgb([200, 100, 7]));
        for (w, h) in [(10, 10), (37, 53), (90, 120)] {
            let out = scale(&img, Target { w, h, fit: Fit::Page }, None);
            assert!(out.px.iter().all(|&p| p == 0xc86407), "{w}x{h}");
            assert_eq!(out.px.len(), (out.w * out.h) as usize);
        }
    }

    #[test]
    fn bands_join_up_into_the_whole_page() {
        let img = RgbImage::from_fn(300, 2000, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, ((x * y) % 256) as u8]));
        let t = Target { w: 700, h: 1, fit: Fit::Width };
        let whole = scale(&img, t, None);
        let mut joined = Vec::new();
        let mut i = 0;
        loop {
            let band = scale(&img, t, Some((i * BAND, (i + 1) * BAND)));
            if band.h == 0 {
                break;
            }
            assert_eq!(band.w, whole.w);
            joined.extend(band.px);
            i += 1;
        }
        assert_eq!(i, whole.h.div_ceil(BAND));
        assert!(joined == whole.px, "the bands differ from the whole page");
    }

    #[test]
    fn tells_black_and_white_from_colour() {
        let gray = RgbImage::from_fn(200, 300, |x, _| image::Rgb([(x % 256) as u8; 3]));
        let colour = RgbImage::from_fn(200, 300, |x, y| image::Rgb([x as u8, y as u8, 40]));
        assert!(describe(&gray).gray);
        assert!(!describe(&colour).gray);
        let info = describe(&RgbImage::new(800, 3000));
        assert!(info.tall() && !info.wide());
        assert!(PageInfo { w: 2000, h: 1400, gray: true }.wide());
    }

    #[test]
    fn refuses_huge_images() {
        // A PNG header claiming 100000 x 100000 pixels.
        let mut png = Vec::new();
        RgbImage::new(1, 1).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        png[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        png[20..24].copy_from_slice(&100_000u32.to_be_bytes());
        assert!(decode(&png).is_err());
    }

    #[test]
    fn fits() {
        let page = Target { w: 1000, h: 1000, fit: Fit::Page };
        assert_eq!(scaled_size(1200, 1700, page), (706, 1000));
        assert_eq!(scaled_size(1200, 1700, Target { fit: Fit::Width, ..page }), (1000, 1417));
    }
}
