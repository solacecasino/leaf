//! Leaf's log: what adapters print and anything that went wrong, written to
//! `leaf.log` in the config folder (started fresh each launch). Help → Show
//! Log opens it, so problems can be read without a terminal. Paths in it
//! show ~ for the home folder, since people paste logs into public issues.
//!
//! A crash leaves `crash.txt` behind, which the next launch reports.

use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static FILE: Mutex<Option<File>> = Mutex::new(None);
static START: OnceLock<Instant> = OnceLock::new();
/// Recent problems, for Copy Debug Info.
static PROBLEMS: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
const PROBLEMS_KEPT: usize = 50;
/// The log stops growing here, so a chatty adapter can't fill the disk.
const MAX_LOG: u64 = 8 * 1024 * 1024;
static WRITTEN: Mutex<u64> = Mutex::new(0);

pub fn path() -> PathBuf {
    crate::state::dir().join("leaf.log")
}

fn crash_file() -> PathBuf {
    crate::state::dir().join("crash.txt")
}

pub fn init() {
    START.get_or_init(Instant::now);
    let _ = std::fs::create_dir_all(crate::state::dir());
    *FILE.lock().unwrap_or_else(|e| e.into_inner()) = File::create(path()).ok();
}

/// Adds a line, from `source` ("leaf" or an adapter's name).
pub fn push(source: &str, text: impl AsRef<str>) {
    let secs = START.get_or_init(Instant::now).elapsed().as_secs_f64();
    let text = crate::platform::hide_home(text.as_ref());
    let line = format!("[{secs:8.2}s] {source}: {text}");
    eprintln!("{line}");
    let lower = text.to_lowercase();
    if ["error", "couldn't", "can't", "stopped", "crash", "isn't", "unknown", "fail"].iter().any(|w| lower.contains(w)) {
        let mut problems = PROBLEMS.lock().unwrap_or_else(|e| e.into_inner());
        if problems.len() >= PROBLEMS_KEPT {
            problems.pop_front();
        }
        problems.push_back(format!("{source}: {text}"));
    }
    let mut written = WRITTEN.lock().unwrap_or_else(|e| e.into_inner());
    if *written > MAX_LOG {
        return;
    }
    if let Some(file) = FILE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let _ = writeln!(file, "{line}");
        *written += line.len() as u64 + 1;
        if *written > MAX_LOG {
            let _ = writeln!(file, "(the log is full; later lines are left out)");
        }
    }
}

/// The last `n` problems logged this run.
pub fn recent_problems(n: usize) -> Vec<String> {
    let problems = PROBLEMS.lock().unwrap_or_else(|e| e.into_inner());
    problems.iter().rev().take(n).rev().cloned().collect()
}

/// Writes what went wrong to `crash.txt` when Leaf panics, so the next
/// start can say so. Your place in every book is saved as you read, so
/// nothing else needs recovering.
pub fn catch_crashes() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current().name().unwrap_or("unnamed").to_owned();
        let text = format!(
            "Leaf {} on {} {}, thread {thread}: {info}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        let _ = std::fs::create_dir_all(crate::state::dir());
        let _ = std::fs::write(crash_file(), crate::platform::hide_home(&text));
        push("leaf", format!("crashed: {info}"));
        default(info);
    }));
}

/// The crash from last time, once: the file is removed.
pub fn last_crash() -> Option<String> {
    let file = crash_file();
    let text = std::fs::read_to_string(&file).ok()?;
    let _ = std::fs::remove_file(file);
    Some(text)
}
