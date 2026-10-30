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

pub struct Sibling {
    pub name: &'static str,
    /// What it does, for messages ("the video editor").
    pub kind: &'static str,
    /// How macOS finds it.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub bundle_id: &'static str,
    /// Its own files, then what else it opens that leaf9000 doesn't.
    pub extensions: &'static [&'static str],
    pub repo: &'static str,
}

pub const SIBLINGS: [Sibling; 4] = [
    Sibling {
        name: "text9000",
        kind: "writing app",
        bundle_id: "app.text9000.editor",
        extensions: &["t9k", "folio"],
        repo: "https://github.com/solacecasino/text9000",
    },
    Sibling {
        name: "splice9000",
        kind: "video editor",
        bundle_id: "app.splice9000.editor",
        extensions: &[
            "splice", "mp4", "mov", "m4v", "mkv", "webm", "avi", "mpg", "mpeg", "mts", "m2ts", "wmv", "flv", "3gp",
        ],
        repo: "https://github.com/solacecasino/splice9000",
    },
    Sibling {
        name: "wave9000",
        kind: "audio workstation",
        bundle_id: "app.wave9000.workstation",
        extensions: &[
            "wave", "wav", "mp3", "flac", "aiff", "aif", "m4a", "aac", "ogg", "oga", "opus", "wma", "caf",
        ],
        repo: "https://github.com/solacecasino/wave9000",
    },
    // Photos themselves are pictures leaf9000 shows; a grace9000 edit is
    // grace9000's.
    Sibling {
        name: "grace9000",
        kind: "photo editor",
        bundle_id: "app.grace9000.editor",
        extensions: &["grace"],
        repo: "https://github.com/solacecasino/grace9000",
    },
];

pub fn sibling(name: &str) -> Option<&'static Sibling> {
    SIBLINGS.iter().find(|s| s.name == name)
}

/// The sister app that opens this file, by its extension.
pub fn for_file(path: &Path) -> Option<&'static Sibling> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    SIBLINGS.iter().find(|s| s.extensions.contains(&ext.as_str()))
}

/// Where a sister app is installed, if it is. `configured` is the path
/// from settings.toml's `[apps]`, which wins.
pub fn find(s: &Sibling, configured: Option<&str>) -> Option<PathBuf> {
    if let Some(path) = configured.map(PathBuf::from).filter(|p| p.exists()) {
        return Some(path);
    }
    os::find(s)
}

/// Opens a sister app, with a file if given. Returns why it couldn't.
pub fn open(s: &Sibling, file: Option<&Path>, configured: Option<&str>) -> Result<(), String> {
    let Some(app) = find(s, configured) else {
        return Err(format!("{} isn't installed. It's the {} in the 9000 series: {}/releases/latest", s.name, s.kind, s.repo));
    };
    os::launch(&app, file).map_err(|e| format!("Couldn't open {}: {e}", s.name))
}

#[cfg(target_os = "macos")]
mod os {
    use super::Sibling;
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSString;
    use std::path::{Path, PathBuf};

    pub fn find(s: &Sibling) -> Option<PathBuf> {
        let url = NSWorkspace::sharedWorkspace().URLForApplicationWithBundleIdentifier(&NSString::from_str(s.bundle_id))?;
        url.path().map(|p| PathBuf::from(p.to_string()))
    }

    pub fn launch(app: &Path, file: Option<&Path>) -> std::io::Result<()> {
        let mut cmd = std::process::Command::new("/usr/bin/open");
        cmd.arg("-a").arg(app);
        if let Some(file) = file {
            cmd.arg(file);
        }
        let status = cmd.status()?;
        if status.success() { Ok(()) } else { Err(std::io::Error::other(format!("open exited with {status}"))) }
    }
}

#[cfg(not(target_os = "macos"))]
mod os {
    use super::Sibling;
    use std::path::{Path, PathBuf};

    fn exe_name(s: &Sibling) -> String {
        if cfg!(windows) { format!("{}.exe", s.name) } else { s.name.to_owned() }
    }

    pub fn find(s: &Sibling) -> Option<PathBuf> {
        let exe = exe_name(s);
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(path) = std::env::var_os("PATH") {
            candidates.extend(std::env::split_paths(&path).map(|dir| dir.join(&exe)));
        }
        // The downloads are folders you unpack anywhere: look beside
        // leaf9000's own folder (Downloads/splice9000-0.0.8-windows-x64/...).
        if let Some(parent) = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.parent()?.to_path_buf()))
            && let Ok(entries) = std::fs::read_dir(&parent)
        {
            let mut beside: Vec<PathBuf> = entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().to_ascii_lowercase().starts_with(s.name))
                .map(|e| e.path().join(&exe))
                .collect();
            // The newest version sorts last.
            beside.sort();
            candidates.extend(beside.into_iter().rev());
        }
        if cfg!(windows) {
            if let Some(local) = std::env::var_os("LOCALAPPDATA") {
                candidates.push(PathBuf::from(local).join("Programs").join(s.name).join(&exe));
            }
        } else {
            candidates.extend(desktop_entry(s));
        }
        candidates.into_iter().find(|p| p.is_file())
    }

    /// The program a sister app's menu entry starts (its install.sh writes
    /// one with the full path).
    #[cfg_attr(windows, allow(dead_code))]
    fn desktop_entry(s: &Sibling) -> Vec<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let data = std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map_or_else(|| home.join(".local/share"), PathBuf::from);
        [data, PathBuf::from("/usr/local/share"), PathBuf::from("/usr/share")]
            .iter()
            .filter_map(|dir| std::fs::read_to_string(dir.join("applications").join(format!("{}.desktop", s.name))).ok())
            .filter_map(|entry| {
                let exec = entry.lines().find_map(|l| l.strip_prefix("Exec="))?;
                Some(PathBuf::from(super::exec_program(exec)))
            })
            .collect()
    }

    pub fn launch(app: &Path, file: Option<&Path>) -> std::io::Result<()> {
        let mut cmd = std::process::Command::new(app);
        if let Some(file) = file {
            cmd.arg(file);
        }
        cmd.spawn().map(drop)
    }
}

/// The program in a desktop entry's Exec= line: the first word, or the
/// quoted path. Desktop files escape twice (the file's strings, then the
/// quoted argument), so `\\$` in the file is `$`.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn exec_program(exec: &str) -> String {
    let unescaped = exec.trim().replace("\\\\", "\\");
    let exec = unescaped.as_str();
    if let Some(quoted) = exec.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = quoted.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => out.extend(chars.next()),
                c => out.push(c),
            }
        }
        out
    } else {
        exec.split_whitespace().next().unwrap_or_default().to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_go_to_the_right_sister() {
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
    fn desktop_entry_programs() {
        assert_eq!(exec_program(r#""/home/me/apps/splice 9000/splice9000""#), "/home/me/apps/splice 9000/splice9000");
        assert_eq!(exec_program(r#""/a/\\$x/wave9000" %f"#), "/a/$x/wave9000");
        assert_eq!(exec_program("grace9000 %F"), "grace9000");
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
