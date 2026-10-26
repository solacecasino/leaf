//! EPUBs: the pages in the book's own reading order (its spine), not file
//! name order. An EPUB of pictures (most manga and comic EPUBs) becomes its
//! images; one of writing becomes its text, laid out by Core Text.

use std::collections::HashMap;
use std::io::Read;

pub enum Content {
    /// Image entries in reading order.
    Images(Vec<String>),
    /// The book's text, chapters separated by blank lines.
    Text(String),
}

/// What's in an EPUB. `read` returns a zip entry's bytes; `exists` whether
/// an entry exists.
pub fn read<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>) -> Result<Content, String> {
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

    let mut images = Vec::new();
    let mut text = String::new();
    for chapter in &spine {
        if crate::book::is_image_name(chapter) {
            images.push(chapter.clone());
            continue;
        }
        let Some(html) = entry(chapter) else { continue };
        let here = dir_of(chapter);
        for t in tags(&html, "img").iter().chain(tags(&html, "image").iter()) {
            let src = attr(t, "src").or_else(|| attr(t, "xlink:href")).or_else(|| attr(t, "href"));
            if let Some(src) = src {
                images.push(join(&here, &src));
            }
        }
        let words = plain_text(&html);
        if !words.trim().is_empty() {
            text.push_str(words.trim());
            text.push_str("\n\n");
        }
    }
    images.dedup();

    // A picture book has a picture per chapter and little writing.
    let letters = text.chars().filter(|c| !c.is_whitespace()).count();
    if !images.is_empty() && letters < 80 * spine.len().max(1) {
        Ok(Content::Images(images))
    } else if letters > 0 {
        Ok(Content::Text(text))
    } else {
        Err("this EPUB has no pages Leaf can show".into())
    }
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

/// The readable text of an XHTML chapter: paragraphs on their own lines.
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
            Content::Images(pages) => assert_eq!(pages, ["OEBPS/images/page 10.jpg", "OEBPS/images/p2.jpg"]),
            Content::Text(_) => panic!("read as text"),
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
        match read(&mut z).unwrap() {
            Content::Text(t) => {
                assert!(t.starts_with("Chapter One\nIt was a dark"), "{t}");
                assert!(t.contains("Tom & Jerry\u{2019}s."));
                assert!(!t.contains("p {"));
            }
            Content::Images(_) => panic!("read as pictures"),
        }
    }

    #[test]
    fn paths_resolve() {
        assert_eq!(join("OEBPS/text/", "../img/a%20b.png#x"), "OEBPS/img/a b.png");
        assert_eq!(join("", "a.png"), "a.png");
        assert_eq!(attr(r#" id='x' href = "y z.html""#, "href").as_deref(), Some("y z.html"));
        assert_eq!(attr(r#" idref="c1""#, "id"), None);
    }
}
