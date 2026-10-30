//! What leaf9000 remembers between runs: the last book, and per book the page,
//! reading direction and fit. Stored as readable JSON in the config folder.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    #[default]
    Page,
    Width,
}

/// How a book is read.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// One page at a time, left to right.
    Comic,
    /// Two pages side by side, right to left.
    Manga,
    /// One long strip, scrolled down.
    Webtoon,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct BookState {
    pub page: usize,
    #[serde(default)]
    pub rtl: bool,
    #[serde(default)]
    pub fit: Fit,
    /// How far down the page, 0.0 (top) to 1.0 (bottom), in fit-width and
    /// webtoon mode.
    #[serde(default)]
    pub scroll: f32,
    /// How the book is read; None until Leaf has looked at it.
    #[serde(default)]
    pub mode: Option<Mode>,
    /// The mode was picked by hand, so Leaf doesn't second-guess it.
    #[serde(default)]
    pub chosen: bool,
    /// Two pages side by side; None follows the mode.
    #[serde(default)]
    pub spreads: Option<bool>,
    /// Pairs start with the cover instead of after it.
    #[serde(default)]
    pub pair_cover: bool,
}

#[derive(Serialize, Deserialize, Default)]
pub struct State {
    #[serde(default)]
    pub last: Option<PathBuf>,
    #[serde(default)]
    pub books: HashMap<String, BookState>,
    /// Books opened lately, newest first, for File → Open Recent.
    #[serde(default)]
    pub recent: Vec<PathBuf>,
    /// Logical window size.
    #[serde(default)]
    pub window: Option<(f64, f64)>,
    /// The display scale the window was last on, to size the first page before the window opens.
    #[serde(default)]
    pub scale: Option<f64>,
    /// Upscaling with an upscaler adapter: on unless switched off (View → Upscale).
    #[serde(default)]
    pub upscale: Option<bool>,
}

const RECENT_KEPT: usize = 10;

impl State {
    pub fn opened(&mut self, path: &std::path::Path) {
        self.last = Some(path.to_path_buf());
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_path_buf());
        self.recent.truncate(RECENT_KEPT);
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// The one folder that holds everything leaf9000 keeps. Copy it to move
/// your setup. `LEAF9000_CONFIG_DIR` (or Leaf's `LEAF_CONFIG_DIR`) points it
/// at another one, for testing or a second setup.
pub fn dir() -> PathBuf {
    match crate::series::env("CONFIG_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => named(crate::series::NAME),
    }
}

/// Where Leaf, before it joined the 9000 series, kept everything.
pub fn leaf_dir() -> PathBuf {
    named(if cfg!(any(target_os = "macos", windows)) { "Leaf" } else { "leaf" })
}

fn named(name: &str) -> PathBuf {
    #[cfg(target_os = "macos")]
    return home().join("Library/Application Support").join(name);
    #[cfg(windows)]
    return std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(home).join(name);
    #[cfg(all(unix, not(target_os = "macos")))]
    return std::env::var_os("XDG_CONFIG_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
        .join(name);
}

/// On leaf9000's first start, copies Leaf's folder (where you were in every
/// book, settings, keys, adapters and their approvals) into its own, so
/// everything carries over. Leaf's folder is left as it was. Returns true
/// when it copied.
pub fn adopt(new: &std::path::Path, old: &std::path::Path) -> bool {
    if new.exists() || !old.is_dir() {
        return false;
    }
    // Copied beside the target, then renamed into place, so a copy cut off
    // halfway is never taken for a finished one.
    let partial = new.with_extension("partial");
    let _ = fs::remove_dir_all(&partial);
    match crate::adapters::copy_dir(old, &partial).and_then(|()| fs::rename(&partial, new)) {
        Ok(()) => true,
        Err(e) => {
            let _ = fs::remove_dir_all(&partial);
            eprintln!("leaf9000: couldn't copy Leaf's settings from {}: {e}", old.display());
            false
        }
    }
}

/// Loads the state. A file that can't be read is set aside, not lost, and
/// Leaf starts fresh.
pub fn load() -> State {
    let path = dir().join("state.json");
    let Ok(text) = fs::read_to_string(&path) else { return State::default() };
    match serde_json::from_str(&text) {
        Ok(state) => state,
        Err(e) => {
            eprintln!("leaf9000: {}: {e}; starting fresh", path.display());
            let _ = fs::rename(&path, path.with_extension("json.bad"));
            State::default()
        }
    }
}

/// Writes through a temporary file, so a crash mid-write can't leave a
/// half-written state behind.
pub fn save(state: &State) {
    let dir = dir();
    let _ = fs::create_dir_all(&dir);
    let tmp = dir.join("state.json.tmp");
    let Ok(text) = serde_json::to_string_pretty(state) else { return };
    if fs::write(&tmp, text).is_ok() {
        let _ = fs::rename(&tmp, dir.join("state.json"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state.json from Leaf 0.0.1 still loads, with the new fields empty.
    #[test]
    fn reads_older_state() {
        let old = r#"{"last":"/b.cbz","books":{"/b.cbz":{"page":7,"rtl":true,"fit":"page","scroll":0.0}},"recent":["/b.cbz"],"window":[700,950],"scale":2.0}"#;
        let s: State = serde_json::from_str(old).unwrap();
        let b = &s.books["/b.cbz"];
        assert_eq!((b.page, b.rtl, b.mode, b.chosen, b.spreads), (7, true, None, false, None));
        let prototype = r#"{"last":"/b.cbz","books":{"/b.cbz":{"page":3}}}"#;
        assert_eq!(serde_json::from_str::<State>(prototype).unwrap().books["/b.cbz"].page, 3);
    }
}

#[cfg(test)]
mod adopt_tests {
    use super::*;

    #[test]
    fn leafs_folder_comes_along_once() {
        let root = std::env::temp_dir().join(format!("leaf9000-adopt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (old, new) = (root.join("Leaf"), root.join("leaf9000"));
        fs::create_dir_all(old.join("adapters/upscale-4k")).unwrap();
        fs::write(old.join("state.json"), "{}").unwrap();
        fs::write(old.join("adapters/upscale-4k/adapter.toml"), "x").unwrap();
        assert!(adopt(&new, &old));
        assert!(new.join("state.json").exists() && new.join("adapters/upscale-4k/adapter.toml").exists());
        assert!(old.join("state.json").exists(), "Leaf's folder stays");
        // Never again once leaf9000 has its own.
        fs::write(old.join("keys.toml"), "x").unwrap();
        assert!(!adopt(&new, &old));
        assert!(!new.join("keys.toml").exists());
        assert!(!adopt(&root.join("other"), &root.join("missing")));
        let _ = fs::remove_dir_all(&root);
    }
}
