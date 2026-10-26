//! Makes demo books to try Leaf with: `cargo run --example sample` writes
//! samples/Demo Comic.cbz (colour), Demo Manga.cbz (black and white) and
//! Demo Webtoon.cbz (tall strips), so each reading mode can be seen.

use image::{Rgb, RgbImage};
use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;

// Seven-segment digits: top, top-right, bottom-right, bottom, bottom-left, top-left, middle.
const DIGITS: [u8; 10] = [0x3f, 0x06, 0x5b, 0x4f, 0x66, 0x6d, 0x7d, 0x07, 0x7f, 0x6f];

fn rect(img: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, c: Rgb<u8>) {
    for yy in y..(y + h).min(img.height()) {
        for xx in x..(x + w).min(img.width()) {
            img.put_pixel(xx, yy, c);
        }
    }
}

fn digit(img: &mut RgbImage, d: usize, x: u32, y: u32, s: u32, c: Rgb<u8>) {
    let (w, t) = (s, s / 5);
    let segs = DIGITS[d];
    let bars = [
        (x, y, w, t),
        (x + w - t, y, t, w),
        (x + w - t, y + w, t, w),
        (x, y + 2 * w - t, w, t),
        (x, y + w, t, w),
        (x, y, t, w),
        (x, y + w - t / 2, w, t),
    ];
    for (i, (bx, by, bw, bh)) in bars.into_iter().enumerate() {
        if segs & (1 << i) != 0 {
            rect(img, bx, by, bw, bh, c);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Comic,
    Manga,
    Webtoon,
}

/// One numbered page. Comics are in colour, manga in black and white,
/// webtoons are tall strips.
fn page(kind: Kind, n: usize, of: usize) -> RgbImage {
    let (w, h) = match kind {
        Kind::Webtoon => (800u32, 2400u32),
        _ => (1200u32, 1700u32),
    };
    let hue = n as f32 / of as f32;
    let mut img = RgbImage::from_fn(w, h, |_, y| {
        let t = y as f32 / h as f32;
        match kind {
            Kind::Manga => {
                let v = (245.0 - 30.0 * t) as u8;
                Rgb([v, v, v])
            }
            _ => Rgb([(250.0 - 150.0 * t * hue) as u8, (200.0 + 40.0 * t) as u8, (120.0 + 120.0 * (1.0 - hue)) as u8]),
        }
    });
    let ink = Rgb([30, 30, 34]);
    let panels: &[(u32, u32, u32, u32)] = match kind {
        Kind::Webtoon => &[(60, 200, 680, 900), (60, 1300, 680, 900)],
        _ => &[(60, 60, 1080, 520), (60, 620, 520, 1020), (620, 620, 520, 480), (620, 1140, 520, 500)],
    };
    for &(x, y, pw, ph) in panels {
        rect(&mut img, x, y, pw, 8, ink);
        rect(&mut img, x, y + ph - 8, pw, 8, ink);
        rect(&mut img, x, y, 8, ph, ink);
        rect(&mut img, x + pw - 8, y, 8, ph, ink);
    }
    // The page number, large, near the top.
    let s = 160;
    let text = n.to_string();
    let total = text.len() as u32 * (s + 50) - 50;
    let colour = if kind == Kind::Manga { ink } else { Rgb([200, 40, 60]) };
    for (i, ch) in text.chars().enumerate() {
        let d = ch.to_digit(10).unwrap() as usize;
        digit(&mut img, d, (w - total) / 2 + i as u32 * (s + 50), 240, s, colour);
    }
    img
}

fn book(name: &str, kind: Kind, pages: usize) {
    let path = format!("samples/{name}.cbz");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for n in 1..=pages {
        let mut png = Vec::new();
        page(kind, n, pages).write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        zip.start_file(format!("Chapter 1/page{n}.png"), opts).unwrap();
        zip.write_all(&png).unwrap();
    }
    zip.finish().unwrap();
    println!("wrote {path}");
}

fn main() {
    std::fs::create_dir_all("samples").unwrap();
    book("Demo Comic", Kind::Comic, 24);
    book("Demo Manga", Kind::Manga, 24);
    book("Demo Webtoon", Kind::Webtoon, 30);
}
