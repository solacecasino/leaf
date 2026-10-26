//! The few things that differ by system.

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::Path;
use std::process::Command;

/// Shows a folder in Finder (Explorer, the file manager), or opens a file in
/// its default app. `text` opens it as text, for config files and the log.
pub fn open(path: &Path, text: bool) {
    #[cfg(target_os = "macos")]
    let result = if text { Command::new("open").arg("-t").arg(path).spawn() } else { Command::new("open").arg(path).spawn() };
    #[cfg(windows)]
    let result = {
        let _ = text;
        Command::new("explorer").arg(path).spawn()
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = {
        let _ = text;
        Command::new("xdg-open").arg(path).spawn()
    };
    if let Err(e) = result {
        crate::log::push("leaf", format!("couldn't open {}: {e}", path.display()));
    }
}

/// Asks a yes-or-no question with a native dialog. The safe answer (`no`)
/// is the default button, so Return or Escape declines: a dialog that pops up
/// while you're typing elsewhere can't be accepted by accident. Saying yes
/// takes a click on `yes`.
pub fn confirm(title: &str, text: &str, yes: &str, no: &str) -> bool {
    rfd::MessageDialog::new()
        .set_title(title)
        .set_description(text)
        .set_buttons(rfd::MessageButtons::OkCancelCustom(no.to_owned(), yes.to_owned()))
        .show()
        == rfd::MessageDialogResult::Custom(yes.to_owned())
}

/// Opens a web page in the browser. Only ever on the user's click: Leaf
/// itself never goes online.
pub fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = Command::new("open").arg(url).spawn();
    #[cfg(windows)]
    let _ = Command::new("explorer").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = Command::new("xdg-open").arg(url).spawn();
}

/// Allocations at least this big get their own pages from the system.
#[cfg(unix)]
const OWN_PAGES: usize = 256 * 1024;

/// Page images and the scaler's scratch buffers are many MB each. The system
/// allocator keeps that space dirty after it's freed (macOS showed 60 MB of
/// it), so big blocks are mapped and unmapped directly: freeing one hands the
/// memory straight back. Everything small goes to the system allocator.
pub struct Allocator;

unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        #[cfg(unix)]
        if layout.size() >= OWN_PAGES && layout.align() <= 4096 {
            let p = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    layout.size(),
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANON,
                    -1,
                    0,
                )
            };
            return if p == libc::MAP_FAILED { std::ptr::null_mut() } else { p.cast() };
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        #[cfg(unix)]
        if layout.size() >= OWN_PAGES && layout.align() <= 4096 {
            unsafe { libc::munmap(ptr.cast(), layout.size()) };
            return;
        }
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // Fresh mappings are already zeroed.
        #[cfg(unix)]
        if layout.size() >= OWN_PAGES && layout.align() <= 4096 {
            return unsafe { self.alloc(layout) };
        }
        unsafe { System.alloc_zeroed(layout) }
    }
}

/// `text` with the home folder written as `~`, so a pasted path doesn't
/// give away someone's user name.
pub fn hide_home(text: &str) -> String {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    match home.map(|h| h.to_string_lossy().trim_end_matches(['/', '\\']).to_owned()) {
        Some(home) if home.len() > 1 => text.replace(&home, "~"),
        _ => text.to_owned(),
    }
}

/// Leaf's memory now and at its most, as the system's own monitor counts it.
pub fn memory_use() -> Option<(u64, u64)> {
    #[cfg(target_os = "macos")]
    {
        let mut info = std::mem::MaybeUninit::<libc::rusage_info_v4>::zeroed();
        // SAFETY: the buffer is a zeroed rusage_info_v4, the size flavor 4 fills.
        let ok = unsafe { libc::proc_pid_rusage(std::process::id() as i32, libc::RUSAGE_INFO_V4, info.as_mut_ptr().cast()) == 0 };
        // SAFETY: filled in when the call succeeds.
        let info = unsafe { info.assume_init() };
        ok.then_some((info.ri_phys_footprint, info.ri_lifetime_max_phys_footprint))
    }
    #[cfg(target_os = "linux")]
    {
        // VmRSS and VmHWM, in kB.
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let kb = |key: &str| -> Option<u64> {
            let line = status.lines().find(|l| l.starts_with(key))?;
            line.split_whitespace().nth(1)?.parse::<u64>().ok().map(|k| k * 1024)
        };
        Some((kb("VmRSS:")?, kb("VmHWM:")?))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    None
}

/// Puts text on the clipboard.
pub fn copy(text: &str) {
    use std::io::Write;
    #[cfg(target_os = "macos")]
    let program = "pbcopy";
    #[cfg(windows)]
    let program = "clip";
    // Wayland's clipboard tool, else X11's.
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = if std::env::var_os("WAYLAND_DISPLAY").is_some() { "wl-copy" } else { "xclip" };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = Command::new(program);
    #[cfg(all(unix, not(target_os = "macos")))]
    if program == "xclip" {
        command.args(["-selection", "clipboard"]);
    }
    #[cfg(any(target_os = "macos", windows))]
    let mut command = Command::new(program);
    let child = command.stdin(std::process::Stdio::piped()).spawn();
    if let Ok(mut child) = child {
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}

/// Where a removed adapter goes, as the message says it.
pub const TRASH: &str = if cfg!(windows) { "Leaf's Removed folder" } else { "the Trash" };

/// Moves a folder to the Trash, where it can still be recovered, renaming
/// it if the name is taken there. On Windows, where the Recycle Bin can't
/// be reached without more code than it's worth, it goes to a "Removed"
/// folder in Leaf's config folder instead.
pub fn move_to_trash(dir: &Path) -> Result<(), String> {
    let trash = if cfg!(windows) {
        crate::state::dir().join("Removed")
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir).join(".Trash")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(std::env::temp_dir)
            .join("Trash/files")
    };
    move_into(dir, &trash)
}

fn move_into(dir: &Path, trash: &Path) -> Result<(), String> {
    let _ = std::fs::create_dir_all(trash);
    let name = dir.file_name().ok_or("nothing to move")?.to_string_lossy().into_owned();
    let mut to = trash.join(&name);
    let mut n = 2;
    while to.exists() {
        to = trash.join(format!("{name} {n}"));
        n += 1;
    }
    std::fs::rename(dir, &to).map_err(|e| format!("Couldn't move {name} to {TRASH}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_the_user_name() {
        let home = std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap();
        assert_eq!(hide_home(&format!("Config folder: {home}/Library/Leaf")), "Config folder: ~/Library/Leaf");
    }

    #[test]
    fn reads_its_own_memory() {
        if cfg!(any(target_os = "macos", target_os = "linux")) {
            // The most-so-far figure is updated lazily, so it can trail a moment.
            let (now, peak) = memory_use().unwrap();
            assert!(now > 0 && peak > 0);
        }
    }

    #[test]
    fn trash_keeps_both_copies() {
        let root = std::env::temp_dir().join(format!("leaf-trash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for _ in 0..2 {
            std::fs::create_dir_all(root.join("thing")).unwrap();
            move_into(&root.join("thing"), &root.join("Trash")).unwrap();
        }
        assert!(root.join("Trash/thing").is_dir() && root.join("Trash/thing 2").is_dir());
    }
}
