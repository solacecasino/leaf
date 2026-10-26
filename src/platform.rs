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
