//! Books of writing on systems without Core Text: laid out with
//! cosmic-text in the system's fonts, the same way as on macOS. Pages are a
//! fixed size in points, so a book has the same page numbers at any window
//! size; each page is laid out once and drawn at the size it's shown. Like
//! the Mac's, the layout happens as the book is read: the first page now,
//! the rest in the background.

use crate::book::{Block, TextPage};
use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Wrap};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// The page, in points, as on macOS.
const PAGE: (f32, f32) = (540.0, 780.0);
const MARGIN: f32 = 46.0;
const LINE_HEIGHT: f32 = 1.35;

pub struct Text {
    inner: Mutex<Inner>,
    laid: AtomicUsize,
    done: AtomicBool,
    estimate: AtomicUsize,
}

enum Piece {
    /// A paragraph of writing.
    Words(String),
    Picture(String),
}

/// One page: pictures alone; writing as runs of lines from paragraphs.
#[derive(Clone)]
enum Page {
    /// (paragraph, first line, end line, top of the first line on the page)
    Words(Vec<(usize, usize, usize, f32)>),
    Picture(String),
}

struct Inner {
    fonts: Option<FontSystem>,
    cache: SwashCache,
    pieces: Vec<Piece>,
    pages: Vec<Page>,
    /// Where the layout has got to: a piece, a line in it, and how far down
    /// the page being filled is.
    piece: usize,
    line: usize,
    filling: Vec<(usize, usize, usize, f32)>,
    y: f32,
    chars_done: usize,
    chars_total: usize,
}

impl Text {
    pub fn from_blocks(blocks: Vec<Block>) -> Result<Text, String> {
        let mut pieces = Vec::new();
        for block in blocks {
            match block {
                Block::Words(words) => {
                    pieces.extend(words.split('\n').map(str::trim_end).filter(|p| !p.trim().is_empty()).map(|p| Piece::Words(p.to_owned())))
                }
                Block::Picture(name) => pieces.push(Piece::Picture(name)),
            }
        }
        if pieces.is_empty() {
            return Err("the book is empty".into());
        }
        let chars_total = pieces.iter().map(|p| if let Piece::Words(w) = p { w.len() } else { 0 }).sum();
        let text = Text {
            inner: Mutex::new(Inner {
                fonts: None,
                cache: SwashCache::new(),
                pieces,
                pages: Vec::new(),
                piece: 0,
                line: 0,
                filling: Vec::new(),
                y: 0.0,
                chars_done: 0,
                chars_total,
            }),
            laid: AtomicUsize::new(0),
            done: AtomicBool::new(false),
            estimate: AtomicUsize::new(1),
        };
        text.ensure(0)?;
        Ok(text)
    }

    pub fn pages(&self) -> usize {
        self.laid.load(Ordering::Acquire)
    }

    pub fn complete(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }

    pub fn estimate(&self) -> usize {
        if self.complete() { self.pages() } else { self.estimate.load(Ordering::Relaxed).max(self.pages() + 1) }
    }

    pub fn ensure(&self, page: usize) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        while inner.pages.len() <= page && !self.complete() {
            if !inner.step() {
                self.done.store(true, Ordering::Release);
            }
            self.laid.store(inner.pages.len(), Ordering::Release);
        }
        if inner.chars_done > 0 && !self.complete() {
            let per_page = inner.chars_done as f64 / inner.pages.len().max(1) as f64;
            let left = (inner.chars_total - inner.chars_done) as f64 / per_page.max(1.0);
            self.estimate.store(inner.pages.len() + left.ceil() as usize, Ordering::Relaxed);
        }
        Ok(())
    }

    pub fn render(&self, page: usize, target: crate::loader::Target) -> Result<TextPage, String> {
        self.ensure(page)?;
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let runs = match inner.pages.get(page).ok_or("a missing page")?.clone() {
            Page::Picture(name) => return Ok(TextPage::Picture(name)),
            Page::Words(runs) => runs,
        };
        let colours = crate::settings::get().text.clone();
        let (w, h) = crate::loader::scaled_size(PAGE.0 as u32, PAGE.1 as u32, target);
        let scale = w as f32 / PAGE.0;
        let paper = colours.paper.0;
        let mut px = vec![paper; w as usize * h as usize];
        let ink = colours.ink.0;
        let Inner { fonts, cache, pieces, .. } = &mut *inner;
        let fonts = fonts.get_or_insert_with(FontSystem::new);
        for (piece, first, end, top) in runs {
            let Some(Piece::Words(words)) = pieces.get(piece) else { continue };
            let buffer = paragraph(fonts, words);
            for run in buffer.layout_runs().skip(first).take(end - first) {
                let dy = top - first_line_top(&buffer, first) + MARGIN;
                for glyph in run.glyphs {
                    let g = glyph.physical((MARGIN * scale, (run.line_y + dy) * scale), scale);
                    cache.with_pixels(fonts, g.cache_key, Color(ink | 0xff00_0000), |x, y, c| {
                        let (x, y) = (g.x + x, g.y + y);
                        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                            return;
                        }
                        let i = y as usize * w as usize + x as usize;
                        px[i] = blend(px[i], ink, c.a());
                    });
                }
            }
        }
        let rgb = px.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]).collect();
        image::RgbImage::from_raw(w, h, rgb).map(TextPage::Words).ok_or_else(|| "a page of the wrong size".into())
    }
}

/// `under` with `over` on top at coverage `a`.
fn blend(under: u32, over: u32, a: u8) -> u32 {
    let mix = |s: u32| {
        let (u, o) = ((under >> s) & 0xff, (over >> s) & 0xff);
        ((o * a as u32 + u * (255 - a as u32)) / 255) << s
    };
    mix(16) | mix(8) | mix(0)
}

/// A paragraph laid out at the page's width, in the book face.
fn paragraph(fonts: &mut FontSystem, words: &str) -> Buffer {
    let settings = crate::settings::get();
    let size = settings.text.size as f32;
    let mut buffer = Buffer::new(fonts, Metrics::new(size, size * LINE_HEIGHT));
    buffer.set_wrap(Wrap::WordOrGlyph);
    buffer.set_size(Some(PAGE.0 - 2.0 * MARGIN), None);
    let face = settings.text.font.clone();
    let has_face = fonts.db().faces().any(|f| f.families.iter().any(|(name, _)| *name == face));
    let family = if has_face { Family::Name(&face) } else { Family::Serif };
    buffer.set_text(words, &Attrs::new().family(family), Shaping::Advanced, None);
    buffer.shape_until_scroll(fonts, false);
    buffer
}

fn first_line_top(buffer: &Buffer, line: usize) -> f32 {
    buffer.layout_runs().nth(line).map_or(0.0, |r| r.line_top)
}

impl Inner {
    /// Fills one more page. False once there's nothing left.
    fn step(&mut self) -> bool {
        let fonts = self.fonts.get_or_insert_with(FontSystem::new);
        let height = PAGE.1 - 2.0 * MARGIN;
        let size = crate::settings::get().text.size as f32;
        let gap = size * 0.53;
        loop {
            let Some(piece) = self.pieces.get(self.piece) else {
                // The last page, if anything is on it.
                if self.filling.is_empty() {
                    return false;
                }
                self.pages.push(Page::Words(std::mem::take(&mut self.filling)));
                return true;
            };
            match piece {
                Piece::Picture(name) => {
                    if !self.filling.is_empty() {
                        self.pages.push(Page::Words(std::mem::take(&mut self.filling)));
                        self.y = 0.0;
                        return true;
                    }
                    self.pages.push(Page::Picture(name.clone()));
                    self.piece += 1;
                    return true;
                }
                Piece::Words(words) => {
                    let buffer = paragraph(fonts, words);
                    let lines: Vec<(f32, f32)> = buffer.layout_runs().map(|r| (r.line_top, r.line_height)).collect();
                    let first = self.line;
                    let start_top = lines.get(first).map_or(0.0, |l| l.0);
                    let mut end = first;
                    while end < lines.len() && self.y + (lines[end].0 - start_top) + lines[end].1 <= height {
                        end += 1;
                    }
                    if end == first && self.filling.is_empty() {
                        // A line taller than a page: put it alone rather than loop.
                        end = (first + 1).min(lines.len());
                    }
                    if end > first {
                        // Writing laid out, in proportion to the lines placed.
                        self.chars_done += words.len() * (end - first) / lines.len().max(1);
                        self.filling.push((self.piece, first, end, self.y));
                        self.y += lines[end - 1].0 + lines[end - 1].1 - start_top;
                    }
                    if end >= lines.len() {
                        self.piece += 1;
                        self.line = 0;
                        self.y += gap;
                        if self.y < height {
                            continue;
                        }
                    } else {
                        self.line = end;
                    }
                    self.pages.push(Page::Words(std::mem::take(&mut self.filling)));
                    self.y = 0.0;
                    return true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::Target;
    use crate::state::Fit;

    #[test]
    fn lays_out_and_draws_pages() {
        let words = "A paragraph of a book that goes on for a while, with café and naïve words. ".repeat(30);
        let blocks = vec![Block::Words(format!("{words}\n{words}\n{words}")), Block::Picture("map.png".into()), Block::Words(words.clone())];
        let book = Text::from_blocks(blocks).unwrap();
        assert_eq!(book.pages(), 1, "only the first page at first");
        assert!(!book.complete() && book.estimate() > 2);
        let TextPage::Words(page) = book.render(0, Target { w: 540, h: 780, fit: Fit::Page }).unwrap() else { panic!() };
        assert_eq!(page.dimensions(), (540, 780));
        let ink = page.pixels().filter(|p| p[0] < 100).count();
        assert!(ink > 2000, "the words are drawn ({ink} dark pixels)");
        // The margins stay paper.
        assert!((0..540).all(|x| page.get_pixel(x, 10)[0] > 200));
        // The same page, smaller, has the same words.
        let TextPage::Words(small) = book.render(0, Target { w: 270, h: 390, fit: Fit::Page }).unwrap() else { panic!() };
        assert_eq!(small.dimensions(), (270, 390));
        book.ensure(usize::MAX).unwrap();
        assert!(book.complete());
        let n = book.pages();
        let pictures: Vec<usize> =
            (0..n).filter(|&p| matches!(book.render(p, Target { w: 54, h: 78, fit: Fit::Page }), Ok(TextPage::Picture(_)))).collect();
        assert_eq!(pictures.len(), 1);
        assert!(pictures[0] > 0 && pictures[0] < n - 1, "the picture is between the writing");
    }
}
