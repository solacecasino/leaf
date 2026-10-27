//! RAR, 7-Zip and TAR comics where libarchive isn't there (Windows, and
//! Linux without it): RARLAB's own unrar code for RAR, and pure-Rust readers
//! for 7-Zip and TAR. The same interface as archive.rs.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// The biggest single page Leaf will unpack, as with libarchive.
const MAX_PAGE: u64 = 256 * 1024 * 1024;

pub struct Listing {
    pub book: Book,
    pub pages: Vec<String>,
    pub comic_info: Option<String>,
}

pub struct Book {
    path: PathBuf,
    kind: Kind,
}

enum Kind {
    /// Each page's entry name, in the archive's own order of entries.
    Rar(Vec<(String, usize)>),
    SevenZip(Vec<String>),
    /// Each page's data offset and size: a TAR is read in place.
    Tar(Vec<(u64, u64)>),
}

fn too_big(size: u64) -> Result<(), String> {
    if size > MAX_PAGE { Err(format!("a page of {} MB is more than Leaf will unpack", size / 1_048_576)) } else { Ok(()) }
}

fn is_comic_info(name: &str) -> bool {
    name.rsplit(['/', '\\']).next().is_some_and(|n| n.eq_ignore_ascii_case("ComicInfo.xml"))
}

impl Book {
    pub fn open(path: &Path, is_page: impl Fn(&str) -> bool) -> Result<Listing, String> {
        let mut head = [0u8; 8];
        let n = File::open(path).and_then(|mut f| f.read(&mut head)).map_err(|e| e.to_string())?;
        let head = &head[..n];
        let mut listing = if head.starts_with(b"Rar!") {
            rar_listing(path, &is_page)?
        } else if head.starts_with(b"7z\xBC\xAF\x27\x1C") {
            seven_listing(path, &is_page)?
        } else {
            tar_listing(path, &is_page)?
        };
        // Pages in natural order, as everywhere else.
        let mut order: Vec<usize> = (0..listing.pages.len()).collect();
        order.sort_by(|&a, &b| crate::book::natural_cmp(&listing.pages[a], &listing.pages[b]));
        listing.pages = order.iter().map(|&i| listing.pages[i].clone()).collect();
        listing.book.kind = match listing.book.kind {
            Kind::Rar(v) => Kind::Rar(order.iter().map(|&i| v[i].clone()).collect()),
            Kind::SevenZip(v) => Kind::SevenZip(order.iter().map(|&i| v[i].clone()).collect()),
            Kind::Tar(v) => Kind::Tar(order.iter().map(|&i| v[i]).collect()),
        };
        Ok(listing)
    }

    pub fn read(&mut self, page: usize) -> Result<Vec<u8>, String> {
        match &self.kind {
            Kind::Rar(entries) => {
                let (name, index) = entries.get(page).ok_or("a missing page")?;
                rar_read(&self.path, name, *index)
            }
            Kind::SevenZip(names) => {
                let name = names.get(page).ok_or("a missing page")?;
                let mut reader = sevenz_rust2::ArchiveReader::open(&self.path, sevenz_rust2::Password::empty()).map_err(|e| e.to_string())?;
                let size = reader.archive().files.iter().find(|f| f.name() == name).map_or(0, |f| f.size);
                too_big(size)?;
                reader.read_file(name).map_err(|e| e.to_string())
            }
            Kind::Tar(entries) => {
                let &(offset, size) = entries.get(page).ok_or("a missing page")?;
                too_big(size)?;
                let mut f = File::open(&self.path).map_err(|e| e.to_string())?;
                f.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
                let mut out = Vec::with_capacity(size as usize);
                f.take(size).read_to_end(&mut out).map_err(|e| e.to_string())?;
                Ok(out)
            }
        }
    }
}

fn rar_listing(path: &Path, is_page: &dyn Fn(&str) -> bool) -> Result<Listing, String> {
    let archive = unrar::Archive::new(path).open_for_listing().map_err(|e| e.to_string())?;
    let mut pages = Vec::new();
    let mut entries = Vec::new();
    let mut info_index = None;
    let mut encrypted = false;
    for (index, header) in archive.enumerate() {
        let header = header.map_err(|e| e.to_string())?;
        let name = header.filename.to_string_lossy().replace('\\', "/");
        if !header.is_file() {
            continue;
        }
        if is_page(&name) {
            encrypted |= header.is_encrypted();
            pages.push(name.clone());
            entries.push((name, index));
        } else if is_comic_info(&name) && header.unpacked_size < 1 << 20 {
            info_index = Some((name, index));
        }
    }
    if encrypted {
        return Err("this archive is locked with a password".into());
    }
    let comic_info = info_index.and_then(|(name, index)| rar_read(path, &name, index).ok()).map(|b| String::from_utf8_lossy(&b).into_owned());
    Ok(Listing { book: Book { path: path.to_path_buf(), kind: Kind::Rar(entries) }, pages, comic_info })
}

/// One entry of a RAR, read by walking its headers to it.
fn rar_read(path: &Path, name: &str, index: usize) -> Result<Vec<u8>, String> {
    let mut archive = unrar::Archive::new(path).open_for_processing().map_err(|e| e.to_string())?;
    let mut i = 0;
    while let Some(header) = archive.read_header().map_err(|e| e.to_string())? {
        let here = header.entry().filename.to_string_lossy().replace('\\', "/");
        if i == index && here == name {
            too_big(header.entry().unpacked_size)?;
            return header.read().map(|(data, _)| data).map_err(|e| e.to_string());
        }
        archive = header.skip().map_err(|e| e.to_string())?;
        i += 1;
    }
    Err("the archive ended early".into())
}

fn seven_listing(path: &Path, is_page: &dyn Fn(&str) -> bool) -> Result<Listing, String> {
    let mut reader = sevenz_rust2::ArchiveReader::open(path, sevenz_rust2::Password::empty()).map_err(|e| match e {
        sevenz_rust2::Error::PasswordRequired => "this archive is locked with a password".to_string(),
        e => e.to_string(),
    })?;
    let files: Vec<(String, u64, bool)> = reader.archive().files.iter().map(|f| (f.name().replace('\\', "/"), f.size, f.has_stream() && !f.is_directory())).collect();
    let pages: Vec<String> = files.iter().filter(|(n, _, file)| *file && is_page(n)).map(|(n, ..)| n.clone()).collect();
    let comic_info = files
        .iter()
        .find(|(n, size, file)| *file && is_comic_info(n) && *size < 1 << 20)
        .and_then(|(n, ..)| reader.read_file(n).ok())
        .map(|b| String::from_utf8_lossy(&b).into_owned());
    Ok(Listing { book: Book { path: path.to_path_buf(), kind: Kind::SevenZip(pages.clone()) }, pages, comic_info })
}

fn tar_listing(path: &Path, is_page: &dyn Fn(&str) -> bool) -> Result<Listing, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut archive = tar::Archive::new(file);
    let mut pages = Vec::new();
    let mut entries = Vec::new();
    let mut comic_info = None;
    for entry in archive.entries().map_err(|_| "not an archive Leaf can read".to_string())? {
        let mut entry = entry.map_err(|_| "not an archive Leaf can read".to_string())?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let name = entry.path().map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
        let size = entry.header().size().unwrap_or(0);
        if is_page(&name) {
            pages.push(name);
            entries.push((entry.raw_file_position(), size));
        } else if is_comic_info(&name) && size < 1 << 20 {
            let mut s = String::new();
            let _ = entry.read_to_string(&mut s);
            comic_info = Some(s);
        }
    }
    if pages.is_empty() && entries.is_empty() && comic_info.is_none() {
        // tar reads nearly anything without complaint; an empty listing
        // of something that isn't a TAR shouldn't look like an empty book.
        let mut head = [0u8; 512];
        let n = File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
        if n < 512 || &head[257..262] != b"ustar" {
            return Err("not an archive Leaf can read".into());
        }
    }
    Ok(Listing { book: Book { path: path.to_path_buf(), kind: Kind::Tar(entries) }, pages, comic_info })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pages_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("leaf-portable-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("in/Chapter 1")).unwrap();
        for (name, body) in [("p10.png", "ten"), ("p2.png", "two"), ("p1.png", "one")] {
            std::fs::write(dir.join("in/Chapter 1").join(name), body).unwrap();
        }
        std::fs::write(dir.join("in/ComicInfo.xml"), "<Manga>Yes</Manga>").unwrap();
        dir
    }

    fn check(archive: &Path) {
        let mut listing = Book::open(archive, |n| n.ends_with(".png")).unwrap();
        let pages: Vec<&str> = listing.pages.iter().map(|p| p.trim_start_matches("./")).collect();
        assert_eq!(pages, ["Chapter 1/p1.png", "Chapter 1/p2.png", "Chapter 1/p10.png"]);
        assert!(listing.comic_info.as_deref().unwrap_or("").contains("Manga"));
        assert_eq!(listing.book.read(2).unwrap(), b"ten");
        assert_eq!(listing.book.read(0).unwrap(), b"one");
        assert!(listing.book.read(7).is_err());
    }

    #[test]
    fn reads_tar() {
        let dir = pages_dir("tar");
        let out = dir.join("book.cbt");
        let mut b = tar::Builder::new(File::create(&out).unwrap());
        b.append_dir_all(".", dir.join("in")).unwrap();
        b.finish().unwrap();
        drop(b);
        check(&out);
        assert!(Book::open(&dir.join("in/ComicInfo.xml"), |_| true).is_err(), "not a TAR");
    }

    #[test]
    fn reads_7zip() {
        let dir = pages_dir("7z");
        let out = dir.join("book.cb7");
        // bsdtar writes 7-Zip; it's on macOS and most Linux systems.
        let made = std::process::Command::new("bsdtar").args(["--format", "7zip", "-cf"]).arg(&out).arg("-C").arg(dir.join("in")).arg(".").status();
        if !made.is_ok_and(|s| s.success()) {
            eprintln!("skipped: no bsdtar");
            return;
        }
        check(&out);
    }

    #[test]
    fn reads_rar() {
        let dir = pages_dir("rar");
        let out = dir.join("book.cbr");
        let made = std::process::Command::new("rar").args(["a", "-r", "-idq"]).arg(&out).arg(".").current_dir(dir.join("in")).status();
        if !made.is_ok_and(|s| s.success()) {
            assert!(std::env::var_os("LEAF_REQUIRE_RAR").is_none(), "rar isn't installed");
            eprintln!("skipped: no rar");
            return;
        }
        check(&out);
    }
}
