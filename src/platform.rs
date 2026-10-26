//! The few things that differ by system.

use std::alloc::{GlobalAlloc, Layout, System};

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
