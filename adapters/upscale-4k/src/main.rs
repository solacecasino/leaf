//! Upscale 4K, a Leaf adapter: upscales comic and manga pages on the GPU,
//! in the spirit of Anime4K (see upscale.wgsl). Leaf starts it, sends
//! `upscale.ready` (the GPU is set up then, off Leaf's critical path), and
//! then `page.upscale` for each picture it shows bigger than it is. The
//! pixels come and go through files; the messages carry their paths.
//!
//! `upscale-4k --selftest` checks it on this computer's GPU; `upscale-4k
//! --bench [in.rgb W H] [OW OH]` times it.

mod gpu;

use gpu::{Gpu, Style};
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::time::Instant;

/// The most pixels a page may have, in or out: well past 8K, short of
/// anything a hostile message could use to exhaust memory.
const MAX_PIXELS: u64 = 120_000_000;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") => println!("upscale-4k {}", env!("CARGO_PKG_VERSION")),
        Some("--selftest") => std::process::exit(selftest()),
        Some("--bench") => std::process::exit(bench(&args[1..])),
        _ => serve(),
    }
}

/// Talks to Leaf: one JSON message a line on stdin, answers on stdout.
fn serve() {
    let mut gpu: Option<Gpu> = None;
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
        let method = message["method"].as_str().unwrap_or("");
        if method == "shutdown" {
            break;
        }
        let Some(id) = message.get("id").cloned() else { continue };
        let result = match method {
            "upscale.ready" => ready(&mut gpu).map(|g| json!({ "device": g.name })),
            "page.upscale" => ready(&mut gpu).and_then(|g| page(g, &message["params"])),
            other => Err(format!("upscale-4k doesn't know {other}")),
        };
        let answer = match result {
            Ok(result) => json!({ "id": id, "result": result }),
            Err(e) => {
                eprintln!("{method}: {e}");
                json!({ "id": id, "error": e })
            }
        };
        let _ = writeln!(stdout, "{answer}").and_then(|()| stdout.flush());
    }
}

fn ready(gpu: &mut Option<Gpu>) -> Result<&mut Gpu, String> {
    if gpu.is_none() {
        let started = Instant::now();
        let g = Gpu::new()?;
        eprintln!("using {} (ready in {:.0} ms)", g.name, started.elapsed().as_secs_f64() * 1000.0);
        *gpu = Some(g);
    }
    Ok(gpu.as_mut().expect("just set"))
}

fn page(gpu: &mut Gpu, p: &Value) -> Result<Value, String> {
    let num = |k: &str| p[k].as_u64().filter(|&v| v > 0 && v < 1 << 20).ok_or_else(|| format!("page.upscale needs a `{k}`"));
    let (iw, ih, ow, oh) = (num("width")? as u32, num("height")? as u32, num("out_width")? as u32, num("out_height")? as u32);
    if iw as u64 * ih as u64 > MAX_PIXELS || ow as u64 * oh as u64 > MAX_PIXELS {
        return Err("a picture bigger than upscale-4k takes".into());
    }
    if p["format"].as_str().unwrap_or("rgb8") != "rgb8" || p["out_format"].as_str().unwrap_or("0rgb32") != "0rgb32" {
        return Err("upscale-4k reads rgb8 and writes 0rgb32".into());
    }
    let rect: Vec<f32> = p["rect"].as_array().map(|r| r.iter().filter_map(Value::as_f64).map(|v| v as f32).collect()).unwrap_or_default();
    let rect: [f32; 4] = if rect.len() == 4 { [rect[0], rect[1], rect[2], rect[3]] } else { [0.0, 0.0, iw as f32, ih as f32] };
    let style = if p["style"].as_str() == Some("soft") { Style::Soft } else { Style::Lines };
    let input = p["input"].as_str().ok_or("page.upscale needs an `input`")?;
    let output = p["output"].as_str().ok_or("page.upscale needs an `output`")?;
    let started = Instant::now();
    let in_file = std::fs::File::open(input).map_err(|e| format!("{input}: {e}"))?;
    // SAFETY: Leaf wrote the file before asking, and waits for the answer.
    let rgb = unsafe { memmap2::Mmap::map(&in_file) }.map_err(|e| format!("{input}: {e}"))?;
    let read = started.elapsed().as_secs_f64() * 1000.0;
    // The result goes straight into the output file through shared
    // memory: Leaf maps the same file, so it's copied once each side
    // rather than written out and read back in.
    let size = ow as u64 * oh as u64 * 4;
    let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(output).map_err(|e| format!("{output}: {e}"))?;
    if file.metadata().map_err(|e| e.to_string())?.len() != size {
        file.set_len(size).map_err(|e| e.to_string())?;
    }
    // SAFETY: a file only this adapter writes, while Leaf waits for the answer.
    let mut map = unsafe { memmap2::MmapMut::map_mut(&file) }.map_err(|e| e.to_string())?;
    let t = gpu.run(&rgb, iw, ih, ow, oh, rect, style, &mut &mut map[..])?;
    Ok(json!({ "written": size, "read_ms": read, "upload_ms": t.upload, "gpu_ms": t.gpu, "write_ms": t.write, "total_ms": started.elapsed().as_secs_f64() * 1000.0 }))
}

// ---- Test pictures, checks and timings -------------------------------------

/// A page like a scanned manga page: white paper, black lines of a few
/// widths, a circle, a patch of screentone dots, and a grey gradient.
fn test_page(w: u32, h: u32) -> Vec<u8> {
    let mut px = vec![255u8; (w * h * 3) as usize];
    let mut set = |x: u32, y: u32, v: u8| {
        if x < w && y < h {
            let i = ((y * w + x) * 3) as usize;
            px[i..i + 3].copy_from_slice(&[v, v, v]);
        }
    };
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32, y as f32);
            // Panel borders, 3 px.
            let border = (x % 400 < 3) || (y % 500 < 3);
            // A diagonal stroke, 2 px.
            let stroke = ((fx - fy * 0.7) - 100.0).abs() < 1.0;
            // A circle outline, about 2 px.
            let d = ((fx - w as f32 / 2.0).powi(2) + (fy - h as f32 / 3.0).powi(2)).sqrt();
            let circle = (d - w as f32 / 6.0).abs() < 1.2;
            // Screentone: dots every 6 px in one panel.
            let tone = x > 410 && x < 790 && y > 510 && y < 990 && (x % 6 < 2) && (y % 6 < 2);
            // A grey gradient strip.
            let grad = y > 1100 && y < 1200;
            if border || stroke || circle || tone {
                set(x, y, 20);
            } else if grad {
                set(x, y, (x * 255 / w) as u8);
            }
        }
    }
    px
}

/// The same scene, upscaled by the CPU with plain bilinear filtering, to
/// compare against.
fn bilinear(rgb: &[u8], iw: u32, ih: u32, ow: u32, oh: u32) -> Vec<u8> {
    let mut out = vec![0u8; (ow * oh) as usize];
    for y in 0..oh {
        let sy = ((y as f32 + 0.5) * ih as f32 / oh as f32 - 0.5).clamp(0.0, ih as f32 - 1.0);
        for x in 0..ow {
            let sx = ((x as f32 + 0.5) * iw as f32 / ow as f32 - 0.5).clamp(0.0, iw as f32 - 1.0);
            let (x0, y0) = (sx as u32, sy as u32);
            let (x1, y1) = ((x0 + 1).min(iw - 1), (y0 + 1).min(ih - 1));
            let g = |x: u32, y: u32| rgb[((y * iw + x) * 3) as usize] as f32;
            let (fx, fy) = (sx - x0 as f32, sy - y0 as f32);
            let v = g(x0, y0) * (1.0 - fx) * (1.0 - fy) + g(x1, y0) * fx * (1.0 - fy) + g(x0, y1) * (1.0 - fx) * fy + g(x1, y1) * fx * fy;
            out[(y * ow + x) as usize] = v.round() as u8;
        }
    }
    out
}

/// Grey levels of a 0RGB result (its green byte).
fn greys(out: &[u8]) -> Vec<u8> {
    out.chunks_exact(4).map(|p| p[1]).collect()
}

/// Pixels on a row that are neither paper nor ink: how soft its edges are.
fn soft_pixels(row: &[u8]) -> usize {
    row.iter().filter(|&&v| v > 50 && v < 205).count()
}

fn selftest() -> i32 {
    let mut gpu = match Gpu::new() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("FAIL: {e}");
            return 1;
        }
    };
    println!("GPU: {}", gpu.name);
    let (iw, ih, ow, oh) = (1200u32, 1700u32, 2400u32, 3400u32);
    let page = test_page(iw, ih);
    let mut failures = 0;
    let mut check = |ok: bool, what: &str| {
        println!("{} {what}", if ok { "ok  " } else { "FAIL" });
        failures += usize::from(!ok);
    };
    for style in [Style::Lines, Style::Soft] {
        let mut out = Vec::new();
        let t = gpu.run(&page, iw, ih, ow, oh, [0.0, 0.0, iw as f32, ih as f32], style, &mut out);
        check(t.is_ok(), &format!("{style:?}: runs"));
        check(out.len() == (ow * oh * 4) as usize, &format!("{style:?}: the right size"));
        if out.len() != (ow * oh * 4) as usize {
            continue;
        }
        check(out.chunks_exact(4).all(|p| p[3] == 0), &format!("{style:?}: 0RGB, top byte zero"));
        let g = greys(&out);
        let reference = bilinear(&page, iw, ih, ow, oh);
        // Paper stays paper, away from any line.
        let paper = (200..260).all(|y| (1000..1060).all(|x| g[y * ow as usize + x] == 255));
        check(paper, &format!("{style:?}: blank paper stays white"));
        // The gradient stays a gradient (no posterising, no banding).
        // (Sampled away from the borders and the stroke that cross it.)
        let row = 2300 * ow as usize;
        let clear = |x: usize| (x / 2).saturating_sub(4)..(x / 2 + 5);
        let ramp: Vec<u8> = (100..2300)
            .step_by(50)
            .filter(|&x| clear(x).all(|sx| page[((1150 * iw as usize) + sx.min(iw as usize - 1)) * 3] != 20))
            .map(|x| g[row + x])
            .collect();
        check(ramp.windows(2).all(|w| w[1] >= w[0]), &format!("{style:?}: a grey gradient stays smooth"));
        // Edges: fewer in-between greys than a plain bilinear upscale.
        let (ours, theirs): (usize, usize) = (40..2000usize).step_by(37).fold((0, 0), |(a, b), y| {
            let r = y * ow as usize..(y + 1) * ow as usize;
            (a + soft_pixels(&g[r.clone()]), b + soft_pixels(&reference[r]))
        });
        println!("     soft edge pixels: {ours} (bilinear: {theirs})");
        if style == Style::Lines {
            check(ours * 10 < theirs * 7, "Lines: edges at least 30% crisper than bilinear");
        } else {
            check(ours <= theirs, "Soft: edges no softer than bilinear");
        }
        // Lines keep their weight: as much ink as bilinear, give or take.
        let ink = |v: &[u8]| v.iter().filter(|&&p| p < 128).count() as f64;
        let ratio = ink(&g) / ink(&reference);
        println!("     ink compared with bilinear: {ratio:.2}");
        check((0.85..1.15).contains(&ratio), &format!("{style:?}: lines keep their weight"));
        // The same input gives the same output.
        let mut again = Vec::new();
        let _ = gpu.run(&page, iw, ih, ow, oh, [0.0, 0.0, iw as f32, ih as f32], style, &mut again);
        check(again == out, &format!("{style:?}: the same every time"));
    }
    // A band (part of the output), as Leaf asks for a webtoon's.
    let mut band = Vec::new();
    let r = gpu.run(&page[..(iw * 300 * 3) as usize], iw, 300, ow, 512, [0.0, 10.0, iw as f32, 266.0], Style::Lines, &mut band);
    check(r.is_ok() && band.len() == (ow * 512 * 4) as usize, "a band of a page");
    check(gpu.run(&page, iw, ih + 1, ow, oh, [0.0; 4], Style::Lines, &mut Vec::new()).is_err(), "a size that doesn't match is refused");
    println!("{}", if failures == 0 { "all passed" } else { "some failed" });
    i32::from(failures > 0)
}

/// Times pages upscaled to 4K: the test page, or a raw RGB file.
fn bench(args: &[String]) -> i32 {
    let num = |i: usize, d: u32| args.get(i).and_then(|v| v.parse().ok()).unwrap_or(d);
    let (rgb, iw, ih, rest) = match args.first() {
        Some(path) if path.ends_with(".rgb") => match std::fs::read(path) {
            Ok(b) => (b, num(1, 0), num(2, 0), 3),
            Err(e) => {
                eprintln!("{path}: {e}");
                return 1;
            }
        },
        _ => (test_page(1200, 1700), 1200, 1700, 0),
    };
    // A 4K screen's height, the page's width in proportion.
    let oh = num(rest + 1, 3840);
    let ow = num(rest, (iw as f64 * oh as f64 / ih as f64).round() as u32);
    let mut gpu = match Gpu::new() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    println!("GPU: {}\n{iw}x{ih} -> {ow}x{oh} ({:.1} MP)", gpu.name, ow as f64 * oh as f64 / 1e6);
    let path = std::env::temp_dir().join("upscale-4k-bench.out");
    let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).open(&path).expect("temp file");
    file.set_len(ow as u64 * oh as u64 * 4).expect("temp file");
    // SAFETY: a temporary file only this process uses.
    let mut map = unsafe { memmap2::MmapMut::map_mut(&file) }.expect("maps");
    for style in [Style::Lines, Style::Soft] {
        let mut runs = Vec::new();
        for i in 0..25 {
            let started = Instant::now();
            let t = gpu.run(&rgb, iw, ih, ow, oh, [0.0, 0.0, iw as f32, ih as f32], style, &mut &mut map[..]).expect("runs");
            if i >= 5 {
                runs.push((t, started.elapsed().as_secs_f64() * 1000.0));
            }
        }
        runs.sort_by(|a, b| a.1.total_cmp(&b.1));
        let (t, total) = runs[runs.len() / 2];
        let worst = runs.last().map_or(0.0, |r| r.1);
        println!(
            "{style:?}: {total:.1} ms a page (median of 20; slowest {worst:.1}): upload {:.1}, GPU {:.1} (shaders {}), written out {:.1}",
            t.upload,
            t.gpu,
            t.shaders.map_or("not timed".into(), |s| format!("{s:.2}")),
            t.write
        );
    }
    drop(map);
    let _ = std::fs::remove_file(path);
    // The shaders alone, back to back with nothing in between, so the GPU
    // runs at full speed: what a busy reader (a webtoon scrolling past)
    // gets.
    for style in [Style::Lines, Style::Soft] {
        let mut shaders = Vec::new();
        for i in 0..40 {
            let t = gpu.run(&rgb, iw, ih, ow, oh, [0.0, 0.0, iw as f32, ih as f32], style, &mut std::io::sink()).expect("runs");
            if i >= 10 {
                shaders.extend(t.shaders);
            }
        }
        shaders.sort_by(f64::total_cmp);
        if let Some(m) = shaders.get(shaders.len() / 2) {
            println!("{style:?}, back to back: shaders {m:.2} ms a page");
        }
    }
    0
}
