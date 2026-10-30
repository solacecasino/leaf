//! leaf9000's place in the 9000 series: its name and colours, and the four
//! sister apps it hands work to. A document, video, sound or project file
//! (opened, dropped on the window or chosen with Open) goes to text9000,
//! splice9000, wave9000 or grace9000, whichever opens it. Books and pictures
//! stay here: leaf9000 is the series' reader.

use std::path::{Path, PathBuf};

/// What the app is called everywhere a person reads it.
pub const NAME: &str = "leaf9000";

/// An environment variable by its leaf9000 name, or by Leaf's
/// (`LEAF9000_CONFIG_DIR`, then `LEAF_CONFIG_DIR`).
pub fn env(suffix: &str) -> Option<std::ffi::OsString> {
    [format!("LEAF9000_{suffix}"), format!("LEAF_{suffix}")]
        .into_iter()
        .find_map(|name| std::env::var_os(name).filter(|v| !v.is_empty()))
}

/// leaf9000's signal colour, page gold (splice orange, wave blue, grace
/// violet, text9000's ink green).
pub const GOLD: u32 = 0xffc23d;
/// The series' graphite, behind the pages.
pub const GRAPHITE: u32 = 0x17181b;

/// Draws leaf9000's mark (an open book, the right-hand page cut on the
/// series' slant into `gold`), `size` pixels wide, centred at (`cx`, `cy`),
/// into a `w`-wide 0RGB buffer over `background`. Smoothed by sampling each
/// pixel 4×4. The same shapes as scripts/make_icon.swift, in its units.
#[allow(clippy::too_many_arguments)]
pub fn draw_mark(buf: &mut [u32], w: i64, h: i64, cx: f64, cy: f64, size: f64, background: u32, page: u32, gold: u32) {
    // The icon's units: the two pages span x 226..798 and y 234..790 (up).
    let (left, right, bottom, top, width, radius) = (226.0, 798.0, 234.0, 790.0, 268.0, 40.0);
    let (x0, cut, slant, gap) = (530.0, 430.0, 30.0, 22.0);
    let scale = (right - left) / size;
    let inside = |u: f64, v: f64, px: f64| {
        if u < px || u > px + width || v < bottom || v > top {
            return false;
        }
        let dx = (px + radius - u).max(u - (px + width - radius)).max(0.0);
        let dy = (bottom + radius - v).max(v - (top - radius)).max(0.0);
        dx * dx + dy * dy <= radius * radius
    };
    let half = size / 2.0;
    let (ys, ye) = ((cy - half).floor().max(0.0) as i64, (cy + half).ceil().min(h as f64) as i64);
    let (xs, xe) = ((cx - half).floor().max(0.0) as i64, (cx + half).ceil().min(w as f64) as i64);
    let mix = |a: u32, b: u32, t: f64| -> u32 {
        let ch = |s: u32| ((a >> s & 0xff) as f64 * (1.0 - t) + (b >> s & 0xff) as f64 * t).round() as u32;
        ch(16) << 16 | ch(8) << 8 | ch(0)
    };
    for y in ys..ye {
        for x in xs..xe {
            let (mut lit, mut golden) = (0u32, 0u32);
            for sy in 0..4 {
                for sx in 0..4 {
                    let u = left + (x as f64 + (sx as f64 + 0.5) / 4.0 - (cx - half)) * scale;
                    let v = top - (y as f64 + (sy as f64 + 0.5) / 4.0 - (cy - half)) * scale;
                    if inside(u, v, left) {
                        lit += 1;
                    } else if inside(u, v, x0) {
                        let fall = 2.0 * slant * (u - x0) / width;
                        if v >= cut + gap / 2.0 + slant - fall {
                            lit += 1;
                        } else if v <= cut - gap / 2.0 + slant - fall {
                            golden += 1;
                        }
                    }
                }
            }
            if lit + golden > 0 {
                let i = (y * w + x) as usize;
                let over = mix(background, page, lit as f64 / 16.0);
                buf[i] = mix(over, gold, golden as f64 / 16.0);
            }
        }
    }
}

/// Who leaf9000 is, for core9000: the 9000 series' shared adapter host
/// pieces, family protocol and sisters.
pub static APP: core9000::App = core9000::App {
    name: NAME,
    version: env!("CARGO_PKG_VERSION"),
    env_prefix: "LEAF9000",
    legacy_env_prefix: "LEAF",
    protocol: crate::adapters::PROTOCOL,
    permissions: &[
        (crate::adapters::BOOK_READ, "see which book is open and the page you're on"),
        (crate::adapters::BOOK_CONTROL, "open books and turn pages"),
        (crate::adapters::BOOK_PAGES, "see the pages you read (to change how they look)"),
    ],
    events_permission: crate::adapters::BOOK_READ,
};

/// A sister app (from core9000, shared by the whole series).
pub type Sibling = core9000::sisters::Sister;

/// The four sisters leaf9000 hands work to.
pub fn siblings() -> impl Iterator<Item = &'static Sibling> {
    core9000::sisters::others()
}

pub fn sibling(name: &str) -> Option<&'static Sibling> {
    siblings().find(|s| s.name.eq_ignore_ascii_case(name))
}

/// The sister app a file goes to: one owning it (projects, documents), or
/// video and sound. leaf9000 reads photos and text itself.
pub fn for_file(path: &Path) -> Option<&'static Sibling> {
    use core9000::sisters::{SOUND, VIDEO, is};
    core9000::sisters::for_file(path).filter(|s| is(path, s.owns) || is(path, VIDEO) || is(path, SOUND))
}

/// Where a sister app is installed, if it is. `configured` is the path
/// from settings.toml's `[apps]`, which wins.
pub fn find(s: &Sibling, configured: Option<&str>) -> Option<PathBuf> {
    s.find(configured.map(Path::new))
}

/// Opens a sister app, with a file if given. Returns why it couldn't.
pub fn open(s: &Sibling, file: Option<&Path>, configured: Option<&str>) -> Result<(), String> {
    s.open(file.as_slice(), configured.map(Path::new))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_go_to_the_right_sister() {
        core9000::init(&APP);
        let name = |p: &str| for_file(Path::new(p)).map(|s| s.name);
        assert_eq!(name("/notes/script.t9k"), Some("text9000"));
        assert_eq!(name("old.FOLIO"), Some("text9000"));
        assert_eq!(name("/m/Clip.MOV"), Some("splice9000"));
        assert_eq!(name("cut.splice"), Some("splice9000"));
        assert_eq!(name("song.flac"), Some("wave9000"));
        assert_eq!(name("look.grace"), Some("grace9000"));
        // What leaf9000 reads itself stays here.
        for own in ["Vol 01.cbz", "a.cbr", "novel.epub", "scan.pdf", "page.jpg", "IMG_1.CR3", "notes.txt", "draft.docx", "folder"] {
            assert_eq!(name(own), None, "{own}");
        }
    }

    #[test]
    fn the_mark_has_pages_and_gold() {
        let (w, h) = (120i64, 120i64);
        let mut buf = vec![0u32; (w * h) as usize];
        draw_mark(&mut buf, w, h, 60.0, 60.0, 100.0, 0, 0xffffff, GOLD);
        let at = |x: i64, y: i64| buf[(y * w + x) as usize];
        assert_eq!(at(30, 60), 0xffffff, "the left page");
        assert_eq!(at(80, 30), 0xffffff, "the top of the right page");
        assert_eq!(at(80, 95), GOLD, "the bottom of the right page");
        assert_eq!(at(60, 60), 0, "the gutter");
        assert_eq!(at(2, 2), 0, "outside");
    }

    #[test]
    fn old_variable_names_still_work() {
        // SAFETY: tests touching these variables run only here.
        unsafe {
            std::env::set_var("LEAF_SERIES_TEST", "old");
            assert_eq!(env("SERIES_TEST").as_deref(), Some(std::ffi::OsStr::new("old")));
            std::env::set_var("LEAF9000_SERIES_TEST", "new");
            assert_eq!(env("SERIES_TEST").as_deref(), Some(std::ffi::OsStr::new("new")));
        }
    }
}
