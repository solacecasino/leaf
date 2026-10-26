//! EPUBs: the pages in the book's own reading order (its spine), not file
//! name order. An EPUB of pictures (most manga and comic EPUBs) becomes its
//! images; one of writing becomes its text, laid out by Core Text, with its
//! pictures on pages of their own where they appear.

use crate::book::Block;
use std::collections::HashMap;
use std::io::Read;

/// Pictures smaller than this in a book of writing are ornaments (a
/// flourish, a dingbat), not illustrations, and get no page.
const MIN_PICTURE: u64 = 16 * 1024;

pub enum Content {
    /// Image entries in reading order.
    Images(Vec<String>),
    /// The book's writing and pictures in reading order.
    Text(Vec<Block>),
}

/// What an EPUB says about itself beyond its pages.
#[derive(Default)]
pub struct Meta {
    /// The spine reads right to left (`page-progression-direction="rtl"`):
    /// a manga.
    pub rtl: bool,
}

/// What's in an EPUB. `read` returns a zip entry's bytes; `exists` whether
/// an entry exists.
pub fn read<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>) -> Result<(Content, Meta), String> {
    let sizes: HashMap<String, u64> = (0..zip.len()).filter_map(|i| zip.by_index_raw(i).ok().and_then(|e| Some((e.name().ok()?.into_owned(), e.size())))).collect();
    let mut entry = |name: &str| -> Option<String> {
        let mut f = zip.by_name(name).ok()?;
        if f.size() > 64 << 20 {
            return None;
        }
        let mut s = String::new();
        f.read_to_string(&mut s).ok()?;
        Some(s)
    };
    let container = entry("META-INF/container.xml").ok_or("this EPUB has no META-INF/container.xml")?;
    let opf_path = tags(&container, "rootfile").into_iter().find_map(|t| attr(&t, "full-path")).ok_or("this EPUB doesn't say where its contents are")?;
    let opf = entry(&opf_path).ok_or("this EPUB's contents file is missing")?;
    let base = dir_of(&opf_path);

    let manifest: HashMap<String, String> = tags(&opf, "item")
        .iter()
        .filter_map(|t| Some((attr(t, "id")?, join(&base, &attr(t, "href")?))))
        .collect();
    let spine: Vec<String> = tags(&opf, "itemref").iter().filter_map(|t| manifest.get(&attr(t, "idref")?).cloned()).collect();
    let rtl = tags(&opf, "spine").iter().any(|t| attr(t, "page-progression-direction").is_some_and(|d| d.eq_ignore_ascii_case("rtl")));

    let mut images = Vec::new();
    let mut blocks = Vec::new();
    for chapter in &spine {
        if crate::book::is_image_name(chapter) {
            images.push(chapter.clone());
            blocks.push(Block::Picture(chapter.clone()));
            continue;
        }
        let Some(html) = entry(chapter) else { continue };
        for block in chapter_blocks(&html, &dir_of(chapter)) {
            if let Block::Picture(p) = &block {
                images.push(p.clone());
            }
            blocks.push(block);
        }
    }
    images.dedup();
    let meta = Meta { rtl };

    // A picture book has a picture per chapter and little writing.
    let letters: usize = blocks.iter().map(|b| if let Block::Words(w) = b { w.chars().filter(|c| !c.is_whitespace()).count() } else { 0 }).sum();
    if !images.is_empty() && letters < 80 * spine.len().max(1) {
        return Ok((Content::Images(images), meta));
    }
    if letters == 0 {
        return Err("this EPUB has no pages Leaf can show".into());
    }
    // Writing: neighbouring runs joined into one, and only real pictures kept.
    let mut out: Vec<Block> = Vec::new();
    for block in blocks {
        match (block, out.last_mut()) {
            (Block::Words(w), Some(Block::Words(prev))) => {
                prev.push_str("\n\n");
                prev.push_str(&w);
            }
            (Block::Picture(p), _) if sizes.get(&p).is_none_or(|&s| s < MIN_PICTURE) => {}
            (Block::Picture(p), Some(Block::Picture(prev))) if *prev == p => {}
            (block, _) => out.push(block),
        }
    }
    Ok((Content::Text(out), meta))
}

/// A chapter's writing, split where its pictures go.
fn chapter_blocks(html: &str, here: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let lower = html.to_ascii_lowercase();
    let body = lower.find("<body").unwrap_or(0);
    let mut from = body;
    // Each picture tag ends the writing before it.
    let mut pictures: Vec<(usize, usize, String)> = Vec::new();
    for name in ["<img", "<image"] {
        let mut at = body;
        while let Some(i) = lower[at..].find(name) {
            let start = at + i;
            let end = html[start..].find('>').map_or(html.len(), |e| start + e + 1);
            at = end;
            if !lower[start + name.len()..].starts_with(|c: char| c.is_whitespace() || c == '/' || c == '>') {
                continue;
            }
            let t = &html[start + name.len()..end.saturating_sub(1).max(start + name.len())];
            if let Some(src) = attr(t, "src").or_else(|| attr(t, "xlink:href")).or_else(|| attr(t, "href")) {
                pictures.push((start, end, join(here, &src)));
            }
        }
    }
    pictures.sort_by_key(|p| p.0);
    let push_words = |part: &str, out: &mut Vec<Block>| {
        let words = plain_text(part);
        if !words.trim().is_empty() {
            out.push(Block::Words(words.trim().to_owned()));
        }
    };
    for (start, end, src) in pictures {
        if start < from {
            continue;
        }
        push_words(&html[from..start], &mut out);
        out.push(Block::Picture(src));
        from = end;
    }
    push_words(&html[from..], &mut out);
    out
}

/// Every `<name ...>` tag's inside, from a little XML or HTML.
fn tags(xml: &str, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let lower = xml.to_ascii_lowercase();
    let open = format!("<{name}");
    let mut at = 0;
    while let Some(i) = lower[at..].find(&open) {
        let start = at + i + open.len();
        at = start;
        // "<item" mustn't match "<itemref".
        if !lower[start..].starts_with(|c: char| c.is_whitespace() || c == '/' || c == '>') {
            continue;
        }
        let Some(end) = xml[start..].find('>') else { break };
        out.push(xml[start..start + end].to_owned());
        at = start + end;
    }
    out
}

/// An attribute's value from a tag's inside, any quote style.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut at = 0;
    while let Some(i) = lower[at..].find(name) {
        let start = at + i;
        at = start + name.len();
        let before_ok = start == 0 || lower.as_bytes()[start - 1].is_ascii_whitespace();
        let rest = tag[at..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let rest = rest[1..].trim_start();
        let quote = rest.chars().next()?;
        let value = if quote == '"' || quote == '\'' {
            rest[1..].split(quote).next()?
        } else {
            rest.split(|c: char| c.is_whitespace() || c == '/').next()?
        };
        return Some(entities(value));
    }
    None
}

fn dir_of(path: &str) -> String {
    path.rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_default()
}

/// Resolves `href` (relative, maybe with `..`, `#`, or %20) against `base`.
fn join(base: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or_default();
    let href = percent_decode(href);
    let mut parts: Vec<&str> = if href.starts_with('/') { Vec::new() } else { base.split('/').filter(|p| !p.is_empty()).collect() };
    for part in href.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The readable text of (part of) an XHTML chapter: paragraphs on their
/// own lines.
pub fn plain_text(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let body = lower.find("<body").unwrap_or(0);
    let mut out = String::new();
    let mut i = body;
    let bytes = html.as_bytes();
    let mut skip_until: Option<&str> = None;
    while i < html.len() {
        if bytes[i] == b'<' {
            let end = html[i..].find('>').map_or(html.len(), |e| i + e + 1);
            let tag = lower[i + 1..end.saturating_sub(1).max(i + 1)].trim_start_matches('/');
            let name: String = tag.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
            if let Some(until) = skip_until {
                if lower[i..].starts_with(until) {
                    skip_until = None;
                }
            } else if name == "script" || name == "style" {
                skip_until = Some(if name == "script" { "</script" } else { "</style" });
            } else if matches!(
                name.as_str(),
                "p" | "div" | "br" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "tr" | "blockquote" | "section" | "hr"
            ) && !out.ends_with('\n')
            {
                out.push('\n');
            }
            i = end;
        } else {
            let next = html[i..].find('<').map_or(html.len(), |n| i + n);
            if skip_until.is_none() {
                out.push_str(&entities(&html[i..next]));
            }
            i = next;
        }
    }
    // One space between words, paragraphs on their own lines.
    out.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

fn entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|&e| e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let c = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            n if n.starts_with("#x") || n.starts_with("#X") => u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32),
            n if n.starts_with('#') => n[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn epub(files: &[(&str, &str)]) -> zip::ZipArchive<std::io::Cursor<Vec<u8>>> {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut z = zip::ZipWriter::new(&mut buf);
        for (name, body) in files {
            z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
        zip::ZipArchive::new(std::io::Cursor::new(buf.into_inner())).unwrap()
    }

    const CONTAINER: (&str, &str) = ("META-INF/container.xml", r#"<container><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#);

    #[test]
    fn picture_books_follow_the_spine() {
        let opf = r#"<package><manifest>
            <item id="c2" href="text/b.xhtml" media-type="application/xhtml+xml"/>
            <item href="text/a.xhtml" id="c1" media-type="application/xhtml+xml"/>
          </manifest><spine><itemref idref="c1"/><itemref idref="c2"/></spine></package>"#;
        let mut z = epub(&[
            CONTAINER,
            ("OEBPS/content.opf", opf),
            ("OEBPS/text/a.xhtml", r#"<html><body><img src="../images/page%2010.jpg" alt=""/></body></html>"#),
            ("OEBPS/text/b.xhtml", r#"<html><body><svg><image xlink:href="../images/p2.jpg"/></svg></body></html>"#),
        ]);
        match read(&mut z).unwrap() {
            (Content::Images(pages), meta) => {
                assert_eq!(pages, ["OEBPS/images/page 10.jpg", "OEBPS/images/p2.jpg"]);
                assert!(!meta.rtl);
            }
            (Content::Text(_), _) => panic!("read as text"),
        }
    }

    #[test]
    fn novels_become_text() {
        let opf = r#"<package><manifest><item id="c1" href="one.xhtml"/></manifest><spine><itemref idref="c1"/></spine></package>"#;
        let chapter = format!(
            "<html><head><title>x</title><style>p {{}}</style></head><body><h1>Chapter&nbsp;One</h1><p>{}</p><p>Tom &amp; Jerry&#8217;s.</p></body></html>",
            "It was a dark and stormy night. ".repeat(10)
        );
        let mut z = epub(&[CONTAINER, ("OEBPS/content.opf", opf), ("OEBPS/one.xhtml", &chapter)]);
        match read(&mut z).unwrap().0 {
            Content::Text(blocks) => {
                let [Block::Words(t)] = blocks.as_slice() else { panic!("one run of writing") };
                assert!(t.starts_with("Chapter One\nIt was a dark"), "{t}");
                assert!(t.contains("Tom & Jerry\u{2019}s."));
                assert!(!t.contains("p {"));
            }
            Content::Images(_) => panic!("read as pictures"),
        }
    }

    #[test]
    fn novels_keep_their_pictures_and_manga_say_so() {
        let opf = r#"<package><manifest><item id="c1" href="one.xhtml"/><item id="c2" href="two.xhtml"/></manifest>
            <spine page-progression-direction="rtl"><itemref idref="c1"/><itemref idref="c2"/></spine></package>"#;
        let words = "It was a dark and stormy night. ".repeat(10);
        let one = format!("<html><body><p>{words}</p><img src='map.png'/><p>After the map.</p><img src='dot.png'/></body></html>");
        let two = format!("<html><body><p>{words}</p></body></html>");
        let map = "x".repeat(40_000);
        let mut z = epub(&[CONTAINER, ("OEBPS/content.opf", opf), ("OEBPS/one.xhtml", &one), ("OEBPS/two.xhtml", &two), ("OEBPS/map.png", &map), ("OEBPS/dot.png", "tiny")]);
        let (content, meta) = read(&mut z).unwrap();
        assert!(meta.rtl);
        let Content::Text(blocks) = content else { panic!("read as pictures") };
        // The map gets a page where it appears; the tiny ornament doesn't.
        let kinds: Vec<String> = blocks.iter().map(|b| match b { Block::Words(w) => format!("words:{}", &w[..5]), Block::Picture(p) => p.clone() }).collect();
        assert_eq!(kinds, ["words:It wa", "OEBPS/map.png", "words:After"]);
    }

    #[test]
    fn paths_resolve() {
        assert_eq!(join("OEBPS/text/", "../img/a%20b.png#x"), "OEBPS/img/a b.png");
        assert_eq!(join("", "a.png"), "a.png");
        assert_eq!(attr(r#" id='x' href = "y z.html""#, "href").as_deref(), Some("y z.html"));
        assert_eq!(attr(r#" idref="c1""#, "id"), None);
    }
}
