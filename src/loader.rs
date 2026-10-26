//! Decodes and scales pages on a background thread. The UI says which pages
//! it wants, most important first; the thread works through them and wakes
//! the UI with each finished page.

use crate::book::Book;
use crate::state::Fit;
use image::RgbImage;
use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread;

/// Decoded full-size pages kept for re-scaling when the window resizes.
const DECODED_KEPT: usize = 1;

/// A page scaled for the window, as 0RGB pixels.
pub struct Scaled {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u32>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Target {
    pub w: u32,
    pub h: u32,
    pub fit: Fit,
}

pub struct Done {
    pub book: u64,
    pub page: usize,
    pub target: Target,
    pub image: Result<Arc<Scaled>, String>,
}

#[derive(Default)]
struct Job {
    book: Option<(u64, Arc<Book>)>,
    target: Target,
    wanted: VecDeque<usize>,
    busy: Option<(u64, usize, Target)>,
}

type Shared = Arc<(Mutex<Job>, Condvar)>;

pub struct Loader {
    shared: Shared,
}

impl Loader {
    /// `deliver` hands each finished page to the UI; it returns false once
    /// the UI is gone, which ends the thread.
    pub fn new(deliver: impl Fn(Done) -> bool + Send + 'static) -> Self {
        let shared: Shared = Arc::default();
        let worker = shared.clone();
        thread::Builder::new()
            .name("pages".into())
            .spawn(move || work(worker, deliver))
            .expect("start the page loader");
        Self { shared }
    }

    /// Replaces the wanted list. A page already being worked on isn't queued again.
    pub fn want(&self, id: u64, book: &Arc<Book>, target: Target, pages: Vec<usize>) {
        let (lock, cv) = &*self.shared;
        let mut job = lock.lock().unwrap_or_else(|e| e.into_inner());
        let busy = job.busy;
        job.book = Some((id, book.clone()));
        job.target = target;
        job.wanted = pages.into_iter().filter(|&p| busy != Some((id, p, target))).collect();
        cv.notify_one();
    }
}

fn work(shared: Shared, deliver: impl Fn(Done) -> bool) {
    let mut decoded: VecDeque<(u64, usize, Arc<RgbImage>)> = VecDeque::new();
    // Pages already scaled, held weakly: a repeat request (one asked again
    // before the first answer reached the UI) is answered without redoing
    // the work, and a page the UI let go of costs nothing here.
    let mut made: VecDeque<(u64, usize, Target, Weak<Scaled>)> = VecDeque::new();
    loop {
        let (id, book, page, target) = {
            let (lock, cv) = &*shared;
            let mut job = lock.lock().unwrap_or_else(|e| e.into_inner());
            job.busy = None;
            loop {
                if let (Some(&page), Some((id, book))) = (job.wanted.front(), job.book.clone()) {
                    job.wanted.pop_front();
                    job.busy = Some((id, page, job.target));
                    break (id, book, page, job.target);
                }
                job = cv.wait(job).unwrap_or_else(|e| e.into_inner());
            }
        };
        let earlier = made.iter().find(|m| (m.0, m.1, m.2) == (id, page, target)).and_then(|m| m.3.upgrade());
        if let Some(image) = earlier {
            if !deliver(Done { book: id, page, target, image: Ok(image) }) {
                return;
            }
            continue;
        }
        let started = std::time::Instant::now();
        // A broken image must never take the reader down with it.
        let image = catch_unwind(AssertUnwindSafe(|| {
            load(&mut decoded, id, &book, page).map(|img| Arc::new(scale(&img, target)))
        }))
        .unwrap_or_else(|_| Err(format!("{}: the decoder crashed", book.pages[page])));
        if let Ok(image) = &image {
            made.push_back((id, page, target, Arc::downgrade(image)));
            if made.len() > 8 {
                made.pop_front();
            }
        }
        if std::env::var_os("LEAF_TIMING").is_some() {
            eprintln!("leaf: page {} decoded and scaled in {:?}", page + 1, started.elapsed());
        }
        if !deliver(Done { book: id, page, target, image }) {
            return;
        }
    }
}

fn load(decoded: &mut VecDeque<(u64, usize, Arc<RgbImage>)>, id: u64, book: &Book, page: usize) -> Result<Arc<RgbImage>, String> {
    if let Some((_, _, img)) = decoded.iter().find(|(b, p, _)| *b == id && *p == page) {
        return Ok(img.clone());
    }
    let bytes = book.read(page)?;
    let img = image::load_from_memory(&bytes).map_err(|e| format!("{}: {e}", book.pages[page]))?;
    let img = Arc::new(img.into_rgb8());
    decoded.push_back((id, page, img.clone()));
    while decoded.len() > DECODED_KEPT {
        decoded.pop_front();
    }
    Ok(img)
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
/// to appear.
fn scale(img: &RgbImage, t: Target) -> Scaled {
    let (iw, ih) = img.dimensions();
    let (w, h) = scaled_size(iw, ih, t);
    let src = img.as_raw();

    let across = taps(iw, w);
    let mut mid = vec![0u8; w as usize * ih as usize * 3];
    for (row, out) in src.chunks_exact(iw as usize * 3).zip(mid.chunks_exact_mut(w as usize * 3)) {
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
    let mut px = Vec::with_capacity(w as usize * h as usize);
    for (start, weights) in taps(ih, h) {
        acc.fill(0);
        for (k, &wt) in weights.iter().enumerate() {
            let row = &mid[(start + k) * stride..][..stride];
            for (a, &v) in acc.iter_mut().zip(row) {
                *a += v as i32 * wt;
            }
        }
        px.extend(acc.as_chunks::<3>().0.iter().map(|c| {
            (to_u8(c[0]) as u32) << 16 | (to_u8(c[1]) as u32) << 8 | to_u8(c[2]) as u32
        }));
    }
    Scaled { w, h, px }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_color_stays_flat() {
        let img = RgbImage::from_pixel(37, 53, image::Rgb([200, 100, 7]));
        for (w, h) in [(10, 10), (37, 53), (90, 120)] {
            let out = scale(&img, Target { w, h, fit: Fit::Page });
            assert!(out.px.iter().all(|&p| p == 0xc86407), "{w}x{h}");
            assert_eq!(out.px.len(), (out.w * out.h) as usize);
        }
    }

    #[test]
    fn fits() {
        let page = Target { w: 1000, h: 1000, fit: Fit::Page };
        assert_eq!(scaled_size(1200, 1700, page), (706, 1000));
        assert_eq!(scaled_size(1200, 1700, Target { fit: Fit::Width, ..page }), (1000, 1417));
    }
}
