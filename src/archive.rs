//! RAR, 7-Zip and TAR comics (`.cbr`, `.cb7`, `.cbt`) through libarchive,
//! which macOS ships (and most Linux systems have), so Leaf adds nothing to
//! read them.
//!
//! libarchive reads an archive from start to end. Leaf keeps one reader open
//! and moves it forward, so reading on (the usual case) never re-reads
//! anything; going back reopens the archive and skips to the page, which
//! for most archives only reads the file headers.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[repr(C)]
struct Archive {
    _private: [u8; 0],
}

#[repr(C)]
struct Entry {
    _private: [u8; 0],
}

const OK: c_int = 0;
const EOF: c_int = 1;
const WARN: c_int = -20;
const REGULAR_FILE: u32 = 0o100000;

/// libarchive's calls Leaf uses. On macOS libarchive is part of the system
/// and linked in; on Linux it's looked for when the first archive opens, so
/// Leaf still starts (and reads everything else) on a system without it.
macro_rules! libarchive {
    ($(fn $name:ident($($arg:ident: $ty:ty),*) -> $ret:ty;)*) => {
        struct Lib {
            $($name: unsafe extern "C" fn($($ty),*) -> $ret,)*
        }

        #[cfg(target_os = "macos")]
        #[link(name = "archive")]
        unsafe extern "C" {
            $(fn $name($($arg: $ty),*) -> $ret;)*
        }

        #[cfg(target_os = "macos")]
        fn load() -> Result<Lib, String> {
            Ok(Lib { $($name,)* })
        }

        #[cfg(target_os = "linux")]
        fn load() -> Result<Lib, String> {
            // SAFETY: dlopen and dlsym with C strings; each symbol is cast to
            // the signature libarchive declares for it.
            unsafe {
                let handle = [c"libarchive.so.13", c"libarchive.so"]
                    .iter()
                    .map(|n| libc::dlopen(n.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL))
                    .find(|h| !h.is_null())
                    .ok_or("RAR, 7-Zip and TAR comics need libarchive: install your system's libarchive13 (or libarchive) package")?;
                Ok(Lib {
                    $($name: {
                        let symbol = libc::dlsym(handle, concat!(stringify!($name), "\0").as_ptr().cast());
                        if symbol.is_null() {
                            return Err(format!("this system's libarchive is too old (no {})", stringify!($name)));
                        }
                        std::mem::transmute::<*mut c_void, unsafe extern "C" fn($($ty),*) -> $ret>(symbol)
                    },)*
                })
            }
        }
    };
}

libarchive! {
    fn archive_read_new() -> *mut Archive;
    fn archive_read_support_filter_all(a: *mut Archive) -> c_int;
    fn archive_read_support_format_all(a: *mut Archive) -> c_int;
    fn archive_read_open_filename(a: *mut Archive, path: *const c_char, block: usize) -> c_int;
    fn archive_read_next_header(a: *mut Archive, entry: *mut *mut Entry) -> c_int;
    fn archive_read_data(a: *mut Archive, buf: *mut c_void, len: usize) -> isize;
    fn archive_read_data_skip(a: *mut Archive) -> c_int;
    fn archive_read_free(a: *mut Archive) -> c_int;
    fn archive_error_string(a: *mut Archive) -> *const c_char;
    fn archive_entry_pathname_utf8(e: *mut Entry) -> *const c_char;
    fn archive_entry_pathname(e: *mut Entry) -> *const c_char;
    fn archive_entry_filetype(e: *mut Entry) -> u32;
    fn archive_entry_size(e: *mut Entry) -> i64;
    fn archive_entry_is_encrypted(e: *mut Entry) -> c_int;
}

static LIB: OnceLock<Result<Lib, String>> = OnceLock::new();

/// Whether the system has libarchive.
#[cfg(target_os = "linux")]
pub fn available() -> bool {
    lib().is_ok()
}

fn lib() -> Result<&'static Lib, String> {
    LIB.get_or_init(load).as_ref().map_err(Clone::clone)
}

/// The biggest single page Leaf will unpack. A real page is a few MB; this
/// stops a crafted archive from claiming gigabytes.
pub const MAX_PAGE: u64 = 256 * 1024 * 1024;

/// An open archive positioned somewhere in its entries.
struct Reader {
    lib: &'static Lib,
    a: *mut Archive,
    /// The index of the entry the next header read returns.
    next: usize,
}

// SAFETY: a libarchive reader can move between threads; Leaf uses it from
// one at a time (behind a Mutex).
unsafe impl Send for Reader {}

impl Reader {
    fn open(path: &Path) -> Result<Reader, String> {
        let c = CString::new(path.as_os_str().as_encoded_bytes()).map_err(|_| "a path with a zero byte")?;
        let lib = lib()?;
        // SAFETY: a fresh reader, freed in Drop.
        unsafe {
            let a = (lib.archive_read_new)();
            if a.is_null() {
                return Err("out of memory".into());
            }
            let r = Reader { lib, a, next: 0 };
            (lib.archive_read_support_filter_all)(a);
            (lib.archive_read_support_format_all)(a);
            if (lib.archive_read_open_filename)(a, c.as_ptr(), 1 << 16) != OK {
                return Err(r.error());
            }
            Ok(r)
        }
    }

    fn error(&self) -> String {
        // SAFETY: libarchive returns a C string it owns, or null.
        let s = unsafe { (self.lib.archive_error_string)(self.a) };
        if s.is_null() { "the archive couldn't be read".into() } else { unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned() }
    }

    /// The next entry's header, or None at the end.
    fn next_header(&mut self) -> Result<Option<Header>, String> {
        let mut e: *mut Entry = std::ptr::null_mut();
        // SAFETY: e is filled by libarchive and valid until the next header read.
        unsafe {
            match (self.lib.archive_read_next_header)(self.a, &mut e) {
                OK | WARN => {}
                EOF => return Ok(None),
                _ => return Err(self.error()),
            }
            self.next += 1;
            let mut name = (self.lib.archive_entry_pathname_utf8)(e);
            if name.is_null() {
                name = (self.lib.archive_entry_pathname)(e);
            }
            let name = if name.is_null() { String::new() } else { CStr::from_ptr(name).to_string_lossy().into_owned() };
            Ok(Some(Header {
                name,
                file: (self.lib.archive_entry_filetype)(e) == REGULAR_FILE,
                size: (self.lib.archive_entry_size)(e).max(0) as u64,
                encrypted: (self.lib.archive_entry_is_encrypted)(e) != 0,
            }))
        }
    }

    /// The current entry's contents, after its header was read.
    fn data(&mut self, size: u64) -> Result<Vec<u8>, String> {
        if size > MAX_PAGE {
            return Err(format!("a page of {} MB is more than Leaf will unpack", size / 1_048_576));
        }
        let mut out = Vec::with_capacity(size as usize);
        let mut buf = vec![0u8; 1 << 16];
        loop {
            // SAFETY: buf is writable for its length.
            let n = unsafe { (self.lib.archive_read_data)(self.a, buf.as_mut_ptr().cast(), buf.len()) };
            match n {
                0 => return Ok(out),
                n if n < 0 => return Err(self.error()),
                n => {
                    out.extend_from_slice(&buf[..n as usize]);
                    if out.len() as u64 > MAX_PAGE {
                        return Err("a page bigger than Leaf will unpack".into());
                    }
                }
            }
        }
    }

    fn skip(&mut self) {
        // SAFETY: skipping the current entry's data is always allowed.
        unsafe { (self.lib.archive_read_data_skip)(self.a) };
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        // SAFETY: a was made by archive_read_new and is freed once.
        unsafe { (self.lib.archive_read_free)(self.a) };
    }
}

struct Header {
    name: String,
    file: bool,
    size: u64,
    encrypted: bool,
}

/// A comic archive libarchive reads.
pub struct Book {
    path: PathBuf,
    /// For each page (in Leaf's order), its index among the archive's entries.
    entries: Vec<usize>,
    reader: Option<Reader>,
}

pub struct Listing {
    pub book: Book,
    pub pages: Vec<String>,
    /// ComicInfo.xml, if the archive has one.
    pub comic_info: Option<String>,
}

impl Book {
    /// Lists the pages: images, in natural order. `is_page` decides what an
    /// image is.
    pub fn open(path: &Path, is_page: impl Fn(&str) -> bool) -> Result<Listing, String> {
        let mut r = Reader::open(path)?;
        let mut found: Vec<(String, usize)> = Vec::new();
        let mut comic_info = None;
        let mut encrypted = false;
        while let Some(h) = r.next_header()? {
            let index = r.next - 1;
            if h.file && is_page(&h.name) {
                encrypted |= h.encrypted;
                found.push((h.name, index));
                r.skip();
            } else if h.file && h.name.rsplit('/').next().is_some_and(|n| n.eq_ignore_ascii_case("ComicInfo.xml")) && h.size < 1 << 20 {
                comic_info = r.data(h.size).ok().map(|b| String::from_utf8_lossy(&b).into_owned());
            } else {
                r.skip();
            }
        }
        if encrypted {
            return Err("this archive is locked with a password".into());
        }
        found.sort_by(|a, b| crate::book::natural_cmp(&a.0, &b.0));
        let (pages, entries) = found.into_iter().unzip();
        Ok(Listing { book: Book { path: path.to_path_buf(), entries, reader: None }, pages, comic_info })
    }

    /// One page's bytes. Moves the open reader forward when it can.
    pub fn read(&mut self, page: usize) -> Result<Vec<u8>, String> {
        let target = *self.entries.get(page).ok_or("a missing page")?;
        let reuse = self.reader.as_ref().is_some_and(|r| r.next <= target);
        if !reuse {
            self.reader = Some(Reader::open(&self.path)?);
        }
        let r = self.reader.as_mut().expect("just opened");
        loop {
            let h = r.next_header()?.ok_or("the archive ended early")?;
            if r.next - 1 == target {
                let data = r.data(h.size);
                if data.is_err() {
                    self.reader = None;
                }
                return data;
            }
            r.skip();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A TAR made with the system's tar, read back through libarchive, in
    /// order and out of order.
    #[test]
    fn reads_tar_pages_forward_and_back() {
        let dir = std::env::temp_dir().join(format!("leaf-tar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("in")).unwrap();
        for (name, body) in [("p10.png", "ten"), ("p2.png", "two"), ("p1.png", "one"), ("notes.txt", "x")] {
            std::fs::write(dir.join("in").join(name), body).unwrap();
        }
        std::fs::write(dir.join("in/ComicInfo.xml"), "<Manga>YesAndRightToLeft</Manga>").unwrap();
        let tar = dir.join("book.cbt");
        let ok = std::process::Command::new("tar").arg("-cf").arg(&tar).arg("-C").arg(dir.join("in")).arg(".").status().unwrap();
        assert!(ok.success());

        let mut listing = Book::open(&tar, |n| n.ends_with(".png")).unwrap();
        assert_eq!(listing.pages.iter().map(|p| p.trim_start_matches("./")).collect::<Vec<_>>(), ["p1.png", "p2.png", "p10.png"]);
        assert!(listing.comic_info.unwrap().contains("YesAndRightToLeft"));
        let book = &mut listing.book;
        assert_eq!(book.read(0).unwrap(), b"one");
        assert_eq!(book.read(2).unwrap(), b"ten");
        assert_eq!(book.read(1).unwrap(), b"two");
        assert!(book.read(9).is_err());
        assert!(Book::open(&dir.join("missing.cbr"), |_| true).is_err());
    }

    /// A real RAR, made with RARLAB's `rar` when it's installed (the
    /// release build installs it). `LEAF_REQUIRE_RAR=1` makes its absence
    /// a failure rather than a skip.
    #[test]
    fn reads_a_real_rar() {
        let dir = std::env::temp_dir().join(format!("leaf-rar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("in/Chapter 1")).unwrap();
        for (name, body) in [("p10.png", "ten"), ("p2.png", "two"), ("p1.png", "one")] {
            std::fs::write(dir.join("in/Chapter 1").join(name), body).unwrap();
        }
        let rar = dir.join("book.cbr");
        let made = std::process::Command::new("rar").arg("a").arg("-r").arg("-idq").arg(&rar).arg("Chapter 1").current_dir(dir.join("in")).status();
        if !made.is_ok_and(|s| s.success()) {
            assert!(std::env::var_os("LEAF_REQUIRE_RAR").is_none(), "rar isn't installed");
            eprintln!("skipped: rar isn't installed");
            return;
        }
        let mut listing = Book::open(&rar, |n| n.ends_with(".png")).unwrap();
        assert_eq!(listing.pages, ["Chapter 1/p1.png", "Chapter 1/p2.png", "Chapter 1/p10.png"]);
        assert_eq!(listing.book.read(2).unwrap(), b"ten");
        assert_eq!(listing.book.read(0).unwrap(), b"one");
    }
}
