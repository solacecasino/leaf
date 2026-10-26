//! Leaf's log: what adapters print and anything that went wrong, written to
//! `leaf.log` in the config folder (started fresh each launch). Help → Show
//! Log opens it, so problems can be read without a terminal.

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static FILE: Mutex<Option<File>> = Mutex::new(None);
static START: OnceLock<Instant> = OnceLock::new();

pub fn path() -> PathBuf {
    crate::state::dir().join("leaf.log")
}

pub fn init() {
    START.get_or_init(Instant::now);
    let _ = std::fs::create_dir_all(crate::state::dir());
    *FILE.lock().unwrap_or_else(|e| e.into_inner()) = File::create(path()).ok();
}

/// Adds a line, from `source` ("leaf" or an adapter's name).
pub fn push(source: &str, text: impl AsRef<str>) {
    let secs = START.get_or_init(Instant::now).elapsed().as_secs_f64();
    let line = format!("[{secs:8.2}s] {source}: {}", text.as_ref());
    eprintln!("{line}");
    if let Some(file) = FILE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let _ = writeln!(file, "{line}");
    }
}
