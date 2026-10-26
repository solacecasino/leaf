//! Makes a demo comic to try Leaf with: `cargo run --example sample`
//! writes samples/Demo Manga.cbz, 24 numbered pages.

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

fn main() {
    std::fs::create_dir_all("samples").unwrap();
    let file = std::fs::File::create("samples/Demo Manga.cbz").unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let (w, h) = (1200u32, 1700u32);
    for page in 1..=24usize {
        let hue = page as f32 / 24.0;
        let mut img = RgbImage::from_fn(w, h, |_, y| {
            let t = y as f32 / h as f32;
            let r = (240.0 - 60.0 * t * hue) as u8;
            let g = (235.0 - 40.0 * t) as u8;
            let b = (225.0 - 50.0 * t * (1.0 - hue)) as u8;
            Rgb([r, g, b])
        });
        let ink = Rgb([30, 30, 34]);
        // Manga-style panel borders.
        for (x, y, pw, ph) in [(60, 60, 1080, 520), (60, 620, 520, 1020), (620, 620, 520, 480), (620, 1140, 520, 500)] {
            rect(&mut img, x, y, pw, 8, ink);
            rect(&mut img, x, y + ph - 8, pw, 8, ink);
            rect(&mut img, x, y, 8, ph, ink);
            rect(&mut img, x + pw - 8, y, 8, ph, ink);
        }
        // The page number, large, in the top panel.
        let s = 160;
        let text = page.to_string();
        let total = text.len() as u32 * (s + 50) - 50;
        for (i, ch) in text.chars().enumerate() {
            let d = ch.to_digit(10).unwrap() as usize;
            digit(&mut img, d, (w - total) / 2 + i as u32 * (s + 50), 160, s, Rgb([200, 40, 60]));
        }
        let mut png = Vec::new();
        img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        zip.start_file(format!("Chapter 1/page{page}.png"), opts).unwrap();
        zip.write_all(&png).unwrap();
    }
    zip.finish().unwrap();
    println!("wrote samples/Demo Manga.cbz");
}
