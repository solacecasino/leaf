//! What Leaf remembers between runs: the last book, and per book the page,
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

/// The one folder that holds everything Leaf keeps. Copy it to move your setup.
pub fn dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    return home().join("Library/Application Support/Leaf");
    #[cfg(windows)]
    return std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(home).join("Leaf");
    #[cfg(all(unix, not(target_os = "macos")))]
    return std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
        .join("leaf");
}

/// Loads the state. A file that can't be read is set aside, not lost, and
/// Leaf starts fresh.
pub fn load() -> State {
    let path = dir().join("state.json");
    let Ok(text) = fs::read_to_string(&path) else { return State::default() };
    match serde_json::from_str(&text) {
        Ok(state) => state,
        Err(e) => {
            eprintln!("leaf: {}: {e}; starting fresh", path.display());
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
