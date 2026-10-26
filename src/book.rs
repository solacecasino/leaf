//! A book is an ordered list of pages: images from a folder or an archive,
//! PDF pages, or text laid out into pages. What a file is gets decided by
//! its first bytes, then its name.

use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use zip::ZipArchive;

use crate::adapters::{Link, OPEN_TIMEOUT};
use crate::loader::Target;
use crate::state::Mode;
use image::RgbImage;
use serde_json::json;

/// Images Leaf decodes itself, then the ones macOS decodes for it.
const IMAGE_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "gif", "bmp", "heic", "heif", "avif", "jxl", "tif", "tiff", "psd", "jp2", "j2k", "tga",
    "ico", "icns", "dng", "cr2", "cr3", "nef", "arw", "raf", "orf", "rw2", "exr", "hdr", "pbm", "pgm", "ppm",
];
/// Plain text, laid out by Leaf.
const TEXT_EXTS: &[&str] = &["txt", "text", "md", "markdown", "nfo", "log"];
/// Documents macOS imports with their formatting.
const DOCUMENT_EXTS: &[&str] = &["rtf", "rtfd", "doc", "docx", "odt", "html", "htm", "xhtml", "webarchive"];

/// The biggest page or text file Leaf will read into memory.
const MAX_READ: u64 = 256 * 1024 * 1024;

pub struct Book {
    /// What gets remembered: the folder or the file.
    pub path: PathBuf,
    pub name: String,
    /// Each page's name in the archive or folder; empty for PDF and text
    /// books, whose pages are numbered.
    names: Vec<String>,
    /// What the file itself says about how to read it.
    pub hints: Hints,
    kind: Kind,
}

/// A piece of a book of writing: some words, or a picture (an entry in the
/// EPUB) that gets a page of its own.
pub enum Block {
    Words(String),
    Picture(String),
}

#[derive(Default, Clone, Copy)]
pub struct Hints {
    /// What the book's own information says it is: ComicInfo.xml's Manga,
    /// language, publisher or format, or an EPUB that reads right to left.
    pub mode: Option<Mode>,
    /// Writing, not pictures: one page at a time, nothing to detect.
    pub text: bool,
    /// A PDF, which can be a comic, a manga scan or a document, so its
    /// pages' colour doesn't say which: one page at a time unless it says.
    pub pdf: bool,
}

enum Kind {
    Folder(Vec<PathBuf>),
    Zip(Mutex<ZipArchive<BufReader<File>>>),
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    Archive(Mutex<crate::archive::Book>),
    #[cfg(target_os = "macos")]
    Pdf(crate::native::Pdf),
    /// Writing laid out into pages, and the EPUB its pictures come from.
    #[cfg(target_os = "macos")]
    Text(crate::native::Text, Option<Mutex<ZipArchive<BufReader<File>>>>),
    /// A format an adapter reads for Leaf (`opens` in adapter.toml).
    Adapter(Arc<Link>),
}

/// One page, either as a file's bytes to decode or already drawn.
pub enum Page {
    Bytes(Vec<u8>),
    Drawn(RgbImage),
}

enum Sniff {
    Zip,
    /// RAR, 7-Zip, TAR and compressed TAR: what libarchive reads.
    Archive,
    Pdf,
    Image,
    Rtf,
    /// An old Word file (OLE container).
    Ole,
    Unknown,
}

fn sniff(path: &Path) -> Sniff {
    let mut head = [0u8; 512];
    let n = File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    let head = &head[..n];
    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        Sniff::Zip
    } else if head.starts_with(b"Rar!")
        || head.starts_with(b"7z\xBC\xAF\x27\x1C")
        || head.get(257..262) == Some(b"ustar")
        || head.starts_with(b"\x1f\x8b")
        || head.starts_with(b"BZh")
        || head.starts_with(b"\xFD7zXZ")
    {
        Sniff::Archive
    } else if head.starts_with(b"%PDF") {
        Sniff::Pdf
    } else if head.starts_with(b"{\\rtf") {
        Sniff::Rtf
    } else if head.starts_with(b"\xD0\xCF\x11\xE0") {
        Sniff::Ole
    } else if image::guess_format(head).is_ok() || has_ext(path, IMAGE_EXTS) {
        Sniff::Image
    } else {
        Sniff::Unknown
    }
}

fn has_ext(path: &Path, exts: &[&str]) -> bool {
    path.extension().is_some_and(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

pub fn is_image_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    // "./page1.png" (as tar writes names) isn't hidden; ".thumbs/x.png" is.
    let hidden = lower.split('/').any(|part| (part.starts_with('.') && part != "." && part != "..") || part == "__macosx");
    !hidden && IMAGE_EXTS.iter().any(|ext| lower.rsplit('.').next() == Some(ext))
}

fn stem(path: &Path) -> String {
    path.file_stem().or(path.file_name()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// What a text file says, whatever its encoding: UTF-8 or UTF-16 (with a
/// byte-order mark), else read as Windows Latin.
fn decode_text(bytes: &[u8]) -> String {
    let utf16 = |be: bool| -> String {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        [0xFF, 0xFE, ..] => utf16(false),
        [0xFE, 0xFF, ..] => utf16(true),
        _ => match std::str::from_utf8(bytes) {
            Ok(s) => s.to_owned(),
            Err(_) => bytes.iter().map(|&b| b as char).collect(),
        },
    }
}

/// Whether a file of unknown type is readable text: no zero bytes, and
/// mostly valid UTF-8 at the start.
fn looks_like_text(path: &Path) -> bool {
    let mut head = vec![0u8; 4096];
    let n = File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    let head = &head[..n];
    n > 0 && !head.contains(&0) && std::str::from_utf8(&head[..head.len().saturating_sub(4)]).is_ok()
}

#[cfg(not(target_os = "macos"))]
fn mac_only(what: &str) -> String {
    format!("{what} open only in the Mac version of Leaf so far, or with an adapter")
}

impl Book {
    fn new(path: &Path, names: Vec<String>, kind: Kind) -> Book {
        Book { path: path.to_path_buf(), name: stem(path), names, hints: Hints::default(), kind }
    }

    /// The pages known so far. Every book has all of them from the start,
    /// except a text book, which is laid out as it's read.
    pub fn len(&self) -> usize {
        match &self.kind {
            #[cfg(target_os = "macos")]
            Kind::Pdf(pdf) => pdf.pages(),
            #[cfg(target_os = "macos")]
            Kind::Text(text, _) => text.pages(),
            _ => self.names.len(),
        }
    }

    /// Whether every page is known.
    pub fn complete(&self) -> bool {
        match &self.kind {
            #[cfg(target_os = "macos")]
            Kind::Text(text, _) => text.complete(),
            _ => true,
        }
    }

    /// The book's length: exact, or a guess while a text book is still
    /// being laid out.
    pub fn estimate(&self) -> usize {
        match &self.kind {
            #[cfg(target_os = "macos")]
            Kind::Text(text, _) => text.estimate(),
            _ => self.len(),
        }
    }

    /// Lays out a text book until `page` exists (or it ends).
    pub fn ensure(&self, page: usize) {
        #[cfg(target_os = "macos")]
        if let Kind::Text(text, _) = &self.kind {
            let _ = text.ensure(page);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = page;
    }

    /// Lays out a few more pages of a text book, in the background.
    pub fn grow(&self) {
        self.ensure(self.len() + 15);
    }

    /// A page's name, for messages.
    pub fn page_name(&self, page: usize) -> String {
        self.names.get(page).cloned().unwrap_or_else(|| format!("page {}", page + 1))
    }

    /// Opens a folder, an archive, a PDF, a text book, or a single image
    /// (which opens its folder at that image). Returns the page to start on
    /// when the path implies one. Call on the main thread (documents are
    /// imported by AppKit).
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
            Sniff::Archive => Ok((Self::archive(path)?, None)),
            Sniff::Pdf => Ok((Self::pdf(path)?, None)),
            Sniff::Rtf | Sniff::Ole => Ok((Self::document(path)?, None)),
            Sniff::Unknown if has_ext(path, DOCUMENT_EXTS) => Ok((Self::document(path)?, None)),
            Sniff::Unknown if has_ext(path, TEXT_EXTS) || looks_like_text(path) => Ok((Self::plain(path)?, None)),
            Sniff::Unknown => Err(format!("{} isn't a book Leaf can read", stem(path))),
        }
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
        let mut book = Book::new(path, pages, Kind::Adapter(link));
        if let Some(name) = answer["name"].as_str() {
            book.name = name.to_owned();
        }
        Ok(book)
    }

    fn folder(dir: &Path) -> Result<Book, String> {
        let mut files = Vec::new();
        walk(dir, dir, 0, &mut files);
        files.sort_by(|a, b| natural_cmp(&a.0, &b.0));
        if files.is_empty() {
            return Err(format!("{} has no page images", stem(dir)));
        }
        let (pages, paths) = files.into_iter().unzip();
        let mut book = Book::new(dir, pages, Kind::Folder(paths));
        book.hints.mode = fs::read_to_string(dir.join("ComicInfo.xml")).ok().and_then(|x| comic_info_mode(&x));
        Ok(book)
    }

    fn zip(path: &Path) -> Result<Book, String> {
        let file = File::open(path).map_err(|e| e.to_string())?;
        let mut archive = ZipArchive::new(BufReader::new(file)).map_err(|e| format!("{}: {e}", stem(path)))?;
        let names: Vec<String> = archive.file_names().filter_map(Result::ok).map(|n| n.into_owned()).collect();
        let has = |n: &str| names.iter().any(|x| x == n);

        // Word and OpenDocument files are zips too.
        if has("word/document.xml") || (has("content.xml") && has("mimetype") && !has("META-INF/container.xml")) {
            return Self::document(path);
        }
        if has("META-INF/container.xml") {
            let (content, meta) = crate::epub::read(&mut archive)?;
            match content {
                crate::epub::Content::Images(pages) => {
                    let pages: Vec<String> = pages.into_iter().filter(|p| names.contains(p)).collect();
                    if !pages.is_empty() {
                        let mut book = Book::new(path, pages, Kind::Zip(Mutex::new(archive)));
                        book.hints.mode = meta.rtl.then_some(Mode::Manga);
                        return Ok(book);
                    }
                }
                crate::epub::Content::Text(blocks) => return Self::text(path, blocks, Some(archive)),
            }
        }

        let mut pages: Vec<String> = names.iter().filter(|n| is_image_name(n)).cloned().collect();
        pages.sort_by(|a, b| natural_cmp(a, b));
        if pages.is_empty() {
            return Err(format!("{} has no page images", stem(path)));
        }
        let info = names.iter().find(|n| n.rsplit('/').next().is_some_and(|f| f.eq_ignore_ascii_case("ComicInfo.xml"))).cloned();
        let mode = info.and_then(|n| {
            let mut s = String::new();
            archive.by_name(&n).ok()?.take(1 << 20).read_to_string(&mut s).ok()?;
            comic_info_mode(&s)
        });
        let mut book = Book::new(path, pages, Kind::Zip(Mutex::new(archive)));
        book.hints.mode = mode;
        Ok(book)
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn archive(path: &Path) -> Result<Book, String> {
        let listing = crate::archive::Book::open(path, is_image_name)?;
        if listing.pages.is_empty() {
            return Err(format!("{} has no page images", stem(path)));
        }
        let mut book = Book::new(path, listing.pages, Kind::Archive(Mutex::new(listing.book)));
        book.hints.mode = listing.comic_info.and_then(|x| comic_info_mode(&x));
        Ok(book)
    }

    #[cfg(target_os = "macos")]
    fn pdf(path: &Path) -> Result<Book, String> {
        let pdf = crate::native::Pdf::open(path)?;
        if pdf.pages() == 0 {
            return Err(format!("{} has no pages", stem(path)));
        }
        let mut book = Book::new(path, Vec::new(), Kind::Pdf(pdf));
        book.hints.pdf = true;
        Ok(book)
    }

    fn plain(path: &Path) -> Result<Book, String> {
        if fs::metadata(path).map_err(|e| e.to_string())?.len() > MAX_READ {
            return Err(format!("{} is too big to read as text", stem(path)));
        }
        let text = decode_text(&fs::read(path).map_err(|e| e.to_string())?);
        Self::text(path, vec![Block::Words(text)], None)
    }

    #[cfg(target_os = "macos")]
    fn text(path: &Path, blocks: Vec<Block>, pictures: Option<ZipArchive<BufReader<File>>>) -> Result<Book, String> {
        Ok(Self::from_text(path, crate::native::Text::from_blocks(blocks)?, pictures))
    }

    #[cfg(target_os = "macos")]
    fn document(path: &Path) -> Result<Book, String> {
        Ok(Self::from_text(path, crate::native::Text::from_document(path)?, None))
    }

    #[cfg(target_os = "macos")]
    fn from_text(path: &Path, text: crate::native::Text, pictures: Option<ZipArchive<BufReader<File>>>) -> Book {
        let mut book = Book::new(path, Vec::new(), Kind::Text(text, pictures.map(Mutex::new)));
        book.hints.text = true;
        book
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn archive(_: &Path) -> Result<Book, String> {
        Err(mac_only("RAR, 7-Zip and TAR comics"))
    }

    #[cfg(not(target_os = "macos"))]
    fn pdf(_: &Path) -> Result<Book, String> {
        Err(mac_only("PDFs"))
    }

    #[cfg(not(target_os = "macos"))]
    fn text(_: &Path, _: Vec<Block>, _: Option<ZipArchive<BufReader<File>>>) -> Result<Book, String> {
        Err(mac_only("Text books"))
    }

    #[cfg(not(target_os = "macos"))]
    fn document(_: &Path) -> Result<Book, String> {
        Err(mac_only("Documents"))
    }

    /// One page. Pictures come back as their file's bytes; PDF and text
    /// pages come back drawn at `target`. Called from the page loader only.
    pub fn page(&self, page: usize, target: Target) -> Result<Page, String> {
        #[cfg(not(target_os = "macos"))]
        let _ = target;
        let name = self.names.get(page).map(String::as_str).unwrap_or("");
        match &self.kind {
            Kind::Folder(paths) => {
                let file = paths.get(page).ok_or("a missing page")?;
                let size = fs::metadata(file).map(|m| m.len()).unwrap_or(0);
                if size > MAX_READ {
                    return Err(format!("{name} is too big to show"));
                }
                fs::read(file).map(Page::Bytes).map_err(|e| format!("{name}: {e}"))
            }
            Kind::Zip(archive) => zip_entry(archive, name).map(Page::Bytes),
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            Kind::Archive(archive) => archive.lock().unwrap_or_else(|e| e.into_inner()).read(page).map(Page::Bytes).map_err(|e| format!("{name}: {e}")),
            #[cfg(target_os = "macos")]
            Kind::Pdf(pdf) => pdf.render(page, target).map(Page::Drawn),
            #[cfg(target_os = "macos")]
            Kind::Text(text, pictures) => match text.render(page, target)? {
                crate::native::TextPage::Words(img) => Ok(Page::Drawn(img)),
                crate::native::TextPage::Picture(name) => match pictures {
                    Some(zip) => zip_entry(zip, &name).map(Page::Bytes),
                    None => Err(format!("{name}: missing")),
                },
            },
            Kind::Adapter(link) => {
                let answer = link.request("book.page", json!({ "path": self.path, "page": page, "name": name }), OPEN_TIMEOUT)?;
                if let Some(file) = answer["file"].as_str() {
                    fs::read(file).map(Page::Bytes).map_err(|e| format!("{name}: {e}"))
                } else if let Some(data) = answer["base64"].as_str() {
                    base64(data).map(Page::Bytes).ok_or_else(|| format!("{name}: the adapter sent bad base64"))
                } else {
                    Err(format!("{name}: the adapter's answer has no `file` or `base64`"))
                }
            }
        }
    }
}

fn zip_entry(archive: &Mutex<ZipArchive<BufReader<File>>>, name: &str) -> Result<Vec<u8>, String> {
    let mut archive = archive.lock().unwrap_or_else(|e| e.into_inner());
    let mut entry = archive.by_name(name).map_err(|e| format!("{name}: {e}"))?;
    if entry.size() > MAX_READ {
        return Err(format!("{name} is too big to show"));
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry.by_ref().take(MAX_READ).read_to_end(&mut bytes).map_err(|e| format!("{name}: {e}"))?;
    Ok(bytes)
}

/// Publishers and imprints whose books are read one way, as ComicInfo.xml
/// names them (lowercase).
const MANGA_PUBLISHERS: &[&str] = &[
    "shueisha", "kodansha", "shogakukan", "viz", "yen press", "seven seas", "square enix", "hakusensha",
    "akita shoten", "kadokawa", "futabasha", "vertical", "tokyopop", "j-novel", "denpa", "one peace",
];
const COMIC_PUBLISHERS: &[&str] = &[
    "marvel", "dc comics", "image comics", "dark horse comics", "idw", "boom! studios", "boom studios",
    "dynamite", "valiant", "oni press", "titan comics", "archie", "aftershock", "vault comics", "mad cave",
    "fantagraphics", "drawn & quarterly", "rebellion", "2000 ad", "skybound", "vertigo",
];
const WEBTOON_PUBLISHERS: &[&str] = &["webtoon", "naver", "kakao", "lezhin", "tapas", "tappytoon", "manta"];

/// How ComicInfo.xml says to read a book, if it says: its Manga field
/// (Yes or No), a webtoon format, the publisher, or the language.
fn comic_info_mode(xml: &str) -> Option<Mode> {
    let field = |name: &str| -> Option<String> {
        let lower = xml.to_ascii_lowercase();
        let start = lower.find(&format!("<{name}>"))? + name.len() + 2;
        let end = start + lower[start..].find('<')?;
        Some(lower[start..end].trim().to_owned())
    };
    match field("manga").as_deref() {
        Some(m) if m.starts_with("yes") => return Some(Mode::Manga),
        Some("no") => return Some(Mode::Comic),
        _ => {}
    }
    if field("format").is_some_and(|f| f.contains("webtoon") || f.contains("web comic") || f.contains("webcomic")) {
        return Some(Mode::Webtoon);
    }
    for publisher in [field("publisher"), field("imprint")].into_iter().flatten() {
        let named = |list: &[&str]| list.iter().any(|p| publisher.contains(p)) ;
        if named(WEBTOON_PUBLISHERS) {
            return Some(Mode::Webtoon);
        }
        if named(MANGA_PUBLISHERS) {
            return Some(Mode::Manga);
        }
        if named(COMIC_PUBLISHERS) || publisher == "dc" {
            return Some(Mode::Comic);
        }
    }
    match field("languageiso").as_deref() {
        Some("ja") | Some("jp") => Some(Mode::Manga),
        _ => None,
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
    fn comic_info_says_how_to_read() {
        let info = |body: &str| comic_info_mode(&format!("<?xml version=\"1.0\"?><ComicInfo>{body}</ComicInfo>"));
        assert_eq!(info("<Manga>YesAndRightToLeft</Manga>"), Some(Mode::Manga));
        assert_eq!(info("<Manga>No</Manga><LanguageISO>ja</LanguageISO>"), Some(Mode::Comic));
        assert_eq!(info("<Manga>Unknown</Manga><Publisher>Image Comics</Publisher>"), Some(Mode::Comic));
        assert_eq!(info("<Publisher>DC</Publisher>"), Some(Mode::Comic));
        assert_eq!(info("<Publisher>VIZ Media</Publisher>"), Some(Mode::Manga));
        assert_eq!(info("<Format>Webtoon</Format>"), Some(Mode::Webtoon));
        assert_eq!(info("<LanguageISO>ja</LanguageISO>"), Some(Mode::Manga));
        assert_eq!(info("<Title>Something</Title>"), None);
    }

    #[test]
    fn skips_hidden_and_mac_junk() {
        assert!(is_image_name("Vol 1/001.JPG"));
        assert!(is_image_name("./page1.png"));
        assert!(!is_image_name("./.hidden/page1.png"));
        assert!(!is_image_name("__MACOSX/._001.jpg"));
        assert!(!is_image_name(".thumb.png"));
        assert!(!is_image_name("ComicInfo.xml"));
    }
}
