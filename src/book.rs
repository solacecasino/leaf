//! A book is an ordered list of page images, from a folder or an archive.
//! What a file is gets decided by its first bytes, not its extension.

use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use zip::ZipArchive;

use crate::adapters::{Link, OPEN_TIMEOUT};
use serde_json::json;

const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp"];

pub struct Book {
    /// What gets remembered: the folder or the archive.
    pub path: PathBuf,
    pub name: String,
    pub pages: Vec<String>,
    kind: Kind,
}

enum Kind {
    Folder(Vec<PathBuf>),
    Zip(Mutex<ZipArchive<BufReader<File>>>),
    /// A format an adapter reads for Leaf (`opens` in adapter.toml).
    Adapter(Arc<Link>),
}

enum Sniff {
    Zip,
    Rar,
    SevenZip,
    Pdf,
    Image,
    Unknown,
}

fn sniff(path: &Path) -> Sniff {
    let mut head = [0u8; 12];
    let n = File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    let head = &head[..n];
    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        Sniff::Zip
    } else if head.starts_with(b"Rar!") {
        Sniff::Rar
    } else if head.starts_with(b"7z\xBC\xAF") {
        Sniff::SevenZip
    } else if head.starts_with(b"%PDF") {
        Sniff::Pdf
    } else if image::guess_format(head).is_ok() {
        Sniff::Image
    } else {
        Sniff::Unknown
    }
}

fn is_image_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let hidden = lower.split('/').any(|part| part.starts_with('.') || part == "__macosx");
    !hidden && IMAGE_EXTS.iter().any(|ext| lower.rsplit('.').next() == Some(ext))
}

fn stem(path: &Path) -> String {
    path.file_stem().or(path.file_name()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

impl Book {
    /// Opens a folder, an archive, or a single image (which opens its folder
    /// at that image). Returns the page to start on when the path implies one.
    pub fn open(path: &Path) -> Result<(Book, Option<usize>), String> {
        let meta = fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if meta.is_dir() {
            return Ok((Self::folder(path)?, None));
        }
        match sniff(path) {
            Sniff::Zip => Ok((Self::zip(path)?, None)),
            Sniff::Image => {
                let parent = path.parent().ok_or("the image has no folder")?;
                let book = Self::folder(parent)?;
                let start = match &book.kind {
                    Kind::Folder(files) => files.iter().position(|f| f == path),
                    _ => None,
                };
                Ok((book, start))
            }
            Sniff::Rar => Err("RAR comics (.cbr) aren't supported in this prototype yet".into()),
            Sniff::SevenZip => Err("7-Zip comics (.cb7) aren't supported in this prototype yet".into()),
            Sniff::Pdf => Err("PDFs aren't supported in this prototype yet".into()),
            Sniff::Unknown => Err(format!("{} isn't a book Leaf can read", stem(path))),
        }
    }

    /// Whether Leaf can't read `path` itself, so an adapter might.
    pub fn needs_adapter(path: &Path) -> bool {
        path.is_file() && matches!(sniff(path), Sniff::Rar | Sniff::SevenZip | Sniff::Pdf | Sniff::Unknown)
    }

    /// The adapter this book is read through, if any.
    pub fn adapter(&self) -> Option<&str> {
        match &self.kind {
            Kind::Adapter(link) => Some(link.name()),
            _ => None,
        }
    }

    /// Opens a book through an adapter: it lists the pages, then hands Leaf
    /// each page's image as it's needed.
    pub fn through_adapter(path: &Path, link: Arc<Link>) -> Result<Book, String> {
        let answer = link.request("book.open", json!({ "path": path }), OPEN_TIMEOUT)?;
        let pages: Vec<String> = answer["pages"]
            .as_array()
            .ok_or("the adapter's book.open answer has no `pages`")?
            .iter()
            .map(|p| p.as_str().map(String::from).unwrap_or_else(|| p.to_string()))
            .collect();
        if pages.is_empty() {
            return Err(format!("{} has no pages", stem(path)));
        }
        let name = answer["name"].as_str().map(String::from).unwrap_or_else(|| stem(path));
        Ok(Book { path: path.to_path_buf(), name, pages, kind: Kind::Adapter(link) })
    }

    fn folder(dir: &Path) -> Result<Book, String> {
        let mut files = Vec::new();
        walk(dir, dir, 0, &mut files);
        files.sort_by(|a, b| natural_cmp(&a.0, &b.0));
        if files.is_empty() {
            return Err(format!("{} has no page images", stem(dir)));
        }
        let (pages, paths) = files.into_iter().unzip();
        Ok(Book { path: dir.to_path_buf(), name: stem(dir), pages, kind: Kind::Folder(paths) })
    }

    fn zip(path: &Path) -> Result<Book, String> {
        let file = File::open(path).map_err(|e| e.to_string())?;
        let archive = ZipArchive::new(BufReader::new(file)).map_err(|e| format!("{}: {e}", stem(path)))?;
        let mut pages: Vec<String> = archive
            .file_names()
            .filter_map(Result::ok)
            .filter(|n| is_image_name(n))
            .map(|n| n.into_owned())
            .collect();
        pages.sort_by(|a, b| natural_cmp(a, b));
        if pages.is_empty() {
            return Err(format!("{} has no page images", stem(path)));
        }
        Ok(Book { path: path.to_path_buf(), name: stem(path), pages, kind: Kind::Zip(Mutex::new(archive)) })
    }

    /// The raw bytes of one page. Called from the page-loading thread only.
    pub fn read(&self, page: usize) -> Result<Vec<u8>, String> {
        let name = &self.pages[page];
        match &self.kind {
            Kind::Folder(paths) => fs::read(&paths[page]).map_err(|e| format!("{name}: {e}")),
            Kind::Zip(archive) => {
                let mut archive = archive.lock().unwrap_or_else(|e| e.into_inner());
                let mut entry = archive.by_name(name).map_err(|e| format!("{name}: {e}"))?;
                let mut bytes = Vec::with_capacity(entry.size() as usize);
                entry.read_to_end(&mut bytes).map_err(|e| format!("{name}: {e}"))?;
                Ok(bytes)
            }
            Kind::Adapter(link) => {
                let answer = link.request("book.page", json!({ "path": self.path, "page": page, "name": name }), OPEN_TIMEOUT)?;
                if let Some(file) = answer["file"].as_str() {
                    fs::read(file).map_err(|e| format!("{name}: {e}"))
                } else if let Some(data) = answer["base64"].as_str() {
                    base64(data).ok_or_else(|| format!("{name}: the adapter sent bad base64"))
                } else {
                    Err(format!("{name}: the adapter's answer has no `file` or `base64`"))
                }
            }
        }
    }
}

/// Decodes standard base64 (padding and line breaks allowed).
fn base64(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let digits: Vec<u8> = text.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=').collect();
    let mut out = Vec::with_capacity(digits.len() * 3 / 4);
    for chunk in digits.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= value(c)? << (18 - 6 * i);
        }
        let bytes = n.to_be_bytes();
        out.extend_from_slice(&bytes[1..chunk.len()]);
    }
    Some(out)
}

/// Collects images under `dir`, a few levels deep so chapter folders work.
fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        if path.is_dir() {
            if depth < 3 && !rel.rsplit('/').next().unwrap_or("").starts_with('.') {
                walk(root, &path, depth + 1, out);
            }
        } else if is_image_name(&rel) {
            out.push((rel, path));
        }
    }
}

/// Orders "page2" before "page10", ignoring case.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    loop {
        match (a.first(), b.first()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let na = a.iter().take_while(|c| c.is_ascii_digit()).count();
                let nb = b.iter().take_while(|c| c.is_ascii_digit()).count();
                let trim = |d: &[u8]| -> usize { d.iter().take_while(|&&c| c == b'0').count() };
                let (da, db) = (&a[trim(&a[..na])..na], &b[trim(&b[..nb])..nb]);
                let order = da.len().cmp(&db.len()).then_with(|| da.cmp(db));
                if order != Ordering::Equal {
                    return order;
                }
                a = &a[na..];
                b = &b[nb..];
            }
            (Some(x), Some(y)) => {
                let order = x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                a = &a[1..];
                b = &b[1..];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut names = vec!["p10.jpg", "p2.jpg", "P1.jpg", "p02b.jpg", "ch1/p3.jpg"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, ["ch1/p3.jpg", "P1.jpg", "p2.jpg", "p02b.jpg", "p10.jpg"]);
    }

    #[test]
    fn decodes_base64() {
        assert_eq!(base64("aGk=").unwrap(), b"hi");
        assert_eq!(base64("TGVhZg==").unwrap(), b"Leaf");
        assert_eq!(base64("TGVh\nZmxldA").unwrap(), b"Leaflet");
        assert!(base64("*").is_none());
    }

    #[test]
    fn skips_hidden_and_mac_junk() {
        assert!(is_image_name("Vol 1/001.JPG"));
        assert!(!is_image_name("__MACOSX/._001.jpg"));
        assert!(!is_image_name(".thumb.png"));
        assert!(!is_image_name("ComicInfo.xml"));
    }
}
