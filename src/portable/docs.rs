//! The writing in documents, on systems without AppKit's importers: Word
//! (.docx and the older .doc), OpenDocument (.odt), RTF and HTML, as
//! paragraphs. Their formatting (bold, sizes, columns) isn't kept; their
//! words and paragraphs are.

use std::io::{Read, Seek};
use std::path::Path;

/// The biggest document part read, as for EPUB chapters.
const MAX_PART: u64 = 64 << 20;

/// A document's text, paragraphs separated by blank lines.
pub fn text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let text = if bytes.starts_with(b"PK") {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).map_err(|e| e.to_string())?;
        if let Some(xml) = part(&mut zip, "word/document.xml") {
            word_xml(&xml)
        } else if let Some(xml) = part(&mut zip, "content.xml") {
            odf_xml(&xml)
        } else {
            return Err("not a document Leaf knows".into());
        }
    } else if bytes.starts_with(b"{\\rtf") {
        rtf(&String::from_utf8_lossy(&bytes))
    } else if bytes.starts_with(b"\xD0\xCF\x11\xE0") {
        doc(&bytes)?
    } else if ["html", "htm", "xhtml"].contains(&ext.as_str()) {
        crate::epub::plain_text(&String::from_utf8_lossy(&bytes)).replace('\n', "\n\n")
    } else {
        return Err("not a document Leaf knows".into());
    };
    if text.trim().is_empty() { Err("the book is empty".into()) } else { Ok(text) }
}

fn part<R: Read + Seek>(zip: &mut zip::ZipArchive<R>, name: &str) -> Option<String> {
    let f = zip.by_name(name).ok()?;
    let mut s = String::new();
    f.take(MAX_PART).read_to_string(&mut s).ok()?;
    Some(s)
}

/// Text from XML where `para` ends a paragraph, `text` holds words, and
/// some tags stand for a character (a tab, a line break).
fn xml_text(xml: &str, para: &[&str], text: &str, chars: &[(&str, char)]) -> String {
    let mut out = String::new();
    let mut inside = false;
    let mut i = 0;
    while let Some(lt) = xml[i..].find('<') {
        let start = i + lt;
        if inside {
            out.push_str(&entities(&xml[i..start]));
        }
        let Some(gt) = xml[start..].find('>') else { break };
        let tag = &xml[start + 1..start + gt];
        let name = tag.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
        if name == text {
            inside = !tag.starts_with('/') && !tag.ends_with('/');
        } else if tag.starts_with('/') && para.contains(&name) {
            out.push_str("\n\n");
        } else if let Some(&(_, c)) = chars.iter().find(|(n, _)| *n == name) {
            out.push(c);
        }
        i = start + gt + 1;
    }
    tidy(&out)
}

fn word_xml(xml: &str) -> String {
    xml_text(xml, &["w:p"], "w:t", &[("w:tab", '\t'), ("w:br", '\n')])
}

fn odf_xml(xml: &str) -> String {
    // OpenDocument puts words straight inside text:p and text:h.
    let mut out = String::new();
    let mut depth = 0usize;
    let mut i = 0;
    while let Some(lt) = xml[i..].find('<') {
        let start = i + lt;
        if depth > 0 {
            out.push_str(&entities(&xml[i..start]));
        }
        let Some(gt) = xml[start..].find('>') else { break };
        let tag = &xml[start + 1..start + gt];
        let name = tag.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
        match name {
            "text:p" | "text:h" if tag.starts_with('/') => {
                depth = depth.saturating_sub(1);
                out.push_str("\n\n");
            }
            "text:p" | "text:h" if !tag.ends_with('/') => depth += 1,
            "text:tab" => out.push('\t'),
            "text:line-break" => out.push('\n'),
            "text:s" => out.push(' '),
            _ => {}
        }
        i = start + gt + 1;
    }
    tidy(&out)
}

fn entities(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// Paragraphs with single blank lines between them, nothing at the ends.
fn tidy(text: &str) -> String {
    text.split("\n\n").map(str::trim).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("\n\n")
}

/// RTF: the words, with control words dropped, \par as a paragraph,
/// \'hh and \uN characters, and the font and colour tables skipped.
fn rtf(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::new();
    // For each open group: whether its text is skipped.
    let mut skip = vec![false];
    let mut skip_after_unicode = 0usize;
    let mut i = 0;
    while i < b.len() {
        let here = *skip.last().unwrap_or(&false);
        match b[i] {
            b'{' => {
                skip.push(here);
                i += 1;
            }
            b'}' => {
                skip.pop();
                if skip.is_empty() {
                    skip.push(false);
                }
                i += 1;
            }
            b'\\' => {
                i += 1;
                let Some(&c) = b.get(i) else { break };
                if c == b'\'' {
                    if let Some(v) = src.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
                        && !here
                    {
                        if skip_after_unicode > 0 {
                            skip_after_unicode -= 1;
                        } else {
                            // Windows-1252, as most RTF is.
                            out.push(cp1252(v));
                        }
                    }
                    i += 3;
                } else if c.is_ascii_alphabetic() {
                    let start = i;
                    while i < b.len() && b[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    let word = &src[start..i];
                    let num_start = i;
                    if i < b.len() && (b[i] == b'-' || b[i].is_ascii_digit()) {
                        i += 1;
                        while i < b.len() && b[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                    let num: Option<i32> = src[num_start..i].parse().ok();
                    if b.get(i) == Some(&b' ') {
                        i += 1;
                    }
                    match word {
                        "fonttbl" | "colortbl" | "stylesheet" | "info" | "pict" | "header" | "footer" | "listtable"
                        | "listoverridetable" | "rsidtbl" | "generator" | "xmlnstbl" | "themedata" | "datastore" => {
                            if let Some(s) = skip.last_mut() {
                                *s = true;
                            }
                        }
                        "par" | "sect" | "page" if !here => out.push_str("\n\n"),
                        "line" if !here => out.push('\n'),
                        "tab" if !here => out.push('\t'),
                        "emdash" if !here => out.push('—'),
                        "endash" if !here => out.push('–'),
                        "lquote" if !here => out.push('‘'),
                        "rquote" if !here => out.push('’'),
                        "ldblquote" if !here => out.push('“'),
                        "rdblquote" if !here => out.push('”'),
                        "u" if !here => {
                            if let Some(n) = num {
                                out.extend(char::from_u32(if n < 0 { (n + 65536) as u32 } else { n as u32 }));
                                skip_after_unicode = 1;
                            }
                        }
                        _ => {}
                    }
                } else {
                    // \* marks a group to skip when unknown; \\ \{ \} are literal.
                    if c == b'*' {
                        if let Some(s) = skip.last_mut() {
                            *s = true;
                        }
                    } else if !here && matches!(c, b'\\' | b'{' | b'}') {
                        out.push(c as char);
                    } else if !here && c == b'~' {
                        out.push('\u{a0}');
                    }
                    i += 1;
                }
            }
            b'\r' | b'\n' => i += 1,
            _ => {
                let next = src[i..].find(['\\', '{', '}', '\r', '\n']).map_or(src.len(), |n| i + n);
                if !here {
                    let mut text = &src[i..next];
                    while skip_after_unicode > 0 && !text.is_empty() {
                        let c = text.chars().next().map_or(1, char::len_utf8);
                        text = &text[c..];
                        skip_after_unicode -= 1;
                    }
                    out.push_str(text);
                }
                i = next;
            }
        }
    }
    tidy(&out)
}

fn cp1252(v: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘', '’', '“',
        '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    if (0x80..0xa0).contains(&v) { HIGH[(v - 0x80) as usize] } else { v as char }
}

/// The text of an old Word file (.doc, Word 97 and later): the
/// WordDocument stream's pieces, as its piece table lists them.
fn doc(bytes: &[u8]) -> Result<String, String> {
    let bad = || "a Word file Leaf couldn't read".to_string();
    let file = Compound::new(bytes).ok_or_else(bad)?;
    let word = file.stream("WordDocument").ok_or_else(bad)?;
    let u16_at = |b: &[u8], i: usize| b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]]));
    let u32_at = |b: &[u8], i: usize| b.get(i..i + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
    let flags = u16_at(&word, 0x0A).ok_or_else(bad)?;
    if flags & 0x0100 != 0 {
        return Err("this Word file is locked with a password".into());
    }
    let table = file.stream(if flags & 0x0200 != 0 { "1Table" } else { "0Table" }).ok_or_else(bad)?;
    // The FIB's fcClx and lcbClx: where the piece table is in the table stream.
    let fc_clx = u32_at(&word, 0x01A2).ok_or_else(bad)? as usize;
    let lcb_clx = u32_at(&word, 0x01A6).ok_or_else(bad)? as usize;
    let clx = table.get(fc_clx..fc_clx + lcb_clx).ok_or_else(bad)?;
    // Skip any Prc entries (0x01) to the Pcdt (0x02).
    let mut at = 0;
    while clx.get(at) == Some(&0x01) {
        let n = u16_at(clx, at + 1).ok_or_else(bad)? as usize;
        at += 3 + n;
    }
    if clx.get(at) != Some(&0x02) {
        return Err(bad());
    }
    let lcb = u32_at(clx, at + 1).ok_or_else(bad)? as usize;
    let plc = clx.get(at + 5..at + 5 + lcb).ok_or_else(bad)?;
    // A PlcPcd: n+1 character positions, then n 8-byte piece descriptors.
    let n = (lcb - 4) / 12;
    let mut out = String::new();
    for k in 0..n {
        let (cp0, cp1) = (u32_at(plc, k * 4).ok_or_else(bad)?, u32_at(plc, k * 4 + 4).ok_or_else(bad)?);
        let fc = u32_at(plc, (n + 1) * 4 + k * 8 + 2).ok_or_else(bad)?;
        let chars = cp1.saturating_sub(cp0) as usize;
        if fc & 0x4000_0000 != 0 {
            // Compressed: one byte a character, at half the offset.
            let start = ((fc & 0x3FFF_FFFF) / 2) as usize;
            out.extend(word.get(start..start + chars).ok_or_else(bad)?.iter().map(|&c| cp1252(c)));
        } else {
            let start = fc as usize;
            let units: Vec<u16> = word.get(start..start + chars * 2).ok_or_else(bad)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            out.push_str(&String::from_utf16_lossy(&units));
        }
    }
    // Word's special characters: paragraph and cell ends, breaks, fields.
    let mut text = String::with_capacity(out.len());
    let mut field = 0;
    for c in out.chars() {
        match c {
            '\u{13}' => field += 1, // a field's code, up to its separator
            '\u{14}' => field = 0,
            '\u{15}' => field = 0,
            _ if field > 0 => {}
            '\r' | '\u{7}' | '\u{c}' => text.push_str("\n\n"),
            '\u{b}' => text.push('\n'),
            '\u{1}' | '\u{8}' => {}
            c => text.push(c),
        }
    }
    Ok(tidy(&text))
}

/// A compound file (the container old Office files are), read leniently:
/// files from some writers (macOS's own among them) break rules a strict
/// reader insists on, and Word opens them anyway.
struct Compound<'a> {
    bytes: &'a [u8],
    sector: usize,
    fat: Vec<u32>,
    mini_fat: Vec<u32>,
    mini_stream: Vec<u8>,
    /// Streams by name: (first sector, size).
    entries: Vec<(String, u32, u64)>,
}

const END: u32 = 0xFFFF_FFFE;

impl<'a> Compound<'a> {
    fn new(bytes: &'a [u8]) -> Option<Compound<'a>> {
        let u16_at = |i: usize| bytes.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]]));
        let u32_at = |i: usize| bytes.get(i..i + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
        if !bytes.starts_with(b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1") {
            return None;
        }
        let sector = 1usize << u16_at(0x1E)?.min(16);
        let mut file = Compound { bytes, sector, fat: Vec::new(), mini_fat: Vec::new(), mini_stream: Vec::new(), entries: Vec::new() };
        // The FAT's own sectors: 109 in the header, then any in DIFAT sectors.
        let mut fat_sectors: Vec<u32> = (0..109).filter_map(|i| u32_at(0x4C + i * 4)).filter(|&s| s < END - 1).collect();
        let mut difat = u32_at(0x44)?;
        let mut guard = 0;
        while difat < END - 1 && guard < 10_000 {
            let base = file.offset(difat)?;
            let per = sector / 4 - 1;
            fat_sectors.extend((0..per).filter_map(|i| u32_at(base + i * 4)).filter(|&s| s < END - 1));
            difat = u32_at(base + per * 4)?;
            guard += 1;
        }
        for s in fat_sectors {
            let base = file.offset(s)?;
            file.fat.extend((0..sector / 4).filter_map(|i| u32_at(base + i * 4)));
        }
        let dir = file.chain(u32_at(0x30)?, u64::MAX)?;
        for e in dir.chunks_exact(128) {
            let len = (u16::from_le_bytes([e[64], e[65]]) as usize).min(64);
            let units: Vec<u16> = e[..len].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            let name = String::from_utf16_lossy(&units).trim_end_matches('\0').to_owned();
            let start = u32::from_le_bytes([e[116], e[117], e[118], e[119]]);
            let size = u32::from_le_bytes([e[120], e[121], e[122], e[123]]) as u64;
            file.entries.push((name, start, size));
        }
        // Small streams live in the mini stream, the root entry's data.
        let &(_, root_start, root_size) = file.entries.first()?;
        file.mini_stream = file.chain(root_start, root_size).unwrap_or_default();
        let mini_fat = file.chain(u32_at(0x3C)?, u64::MAX).unwrap_or_default();
        file.mini_fat = mini_fat.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        Some(file)
    }

    fn offset(&self, sector: u32) -> Option<usize> {
        let at = (sector as usize + 1).checked_mul(self.sector)?;
        (at + self.sector <= self.bytes.len()).then_some(at)
    }

    /// A chain of sectors, up to `size` bytes.
    fn chain(&self, mut s: u32, size: u64) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let mut steps = 0;
        while s < END - 1 && (out.len() as u64) < size && steps <= self.fat.len() {
            let at = self.offset(s)?;
            out.extend_from_slice(&self.bytes[at..at + self.sector]);
            s = *self.fat.get(s as usize)?;
            steps += 1;
        }
        if size != u64::MAX {
            out.truncate(size as usize);
        }
        (out.len() as u64 <= MAX_PART).then_some(out)
    }

    fn stream(&self, name: &str) -> Option<Vec<u8>> {
        let &(_, start, size) = self.entries.iter().skip(1).find(|(n, ..)| n == name)?;
        if size >= 4096 {
            return self.chain(start, size);
        }
        let mut out = Vec::new();
        let (mut s, mut steps) = (start, 0);
        while s < END - 1 && (out.len() as u64) < size && steps <= self.mini_fat.len() {
            let at = s as usize * 64;
            out.extend_from_slice(self.mini_stream.get(at..at + 64)?);
            s = *self.mini_fat.get(s as usize)?;
            steps += 1;
        }
        out.truncate(size as usize);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_and_opendocument_xml() {
        let docx = r#"<w:document><w:body><w:p><w:r><w:t>Chapter One</w:t></w:r></w:p><w:p><w:r><w:t xml:space="preserve">It was </w:t></w:r><w:r><w:t>dark &amp; cold.</w:t></w:r><w:r><w:br/></w:r><w:r><w:t>Next line</w:t></w:r></w:p></w:body></w:document>"#;
        assert_eq!(word_xml(docx), "Chapter One\n\nIt was dark & cold.\nNext line");
        let odt = r#"<office:text><text:h text:outline-level="1">Title</text:h><text:p>One<text:s/>two<text:tab/>three</text:p><text:p/><text:p>Last</text:p></office:text>"#;
        assert_eq!(odf_xml(odt), "Title\n\nOne two\tthree\n\nLast");
    }

    #[test]
    fn rtf_words_without_the_codes() {
        let src = r"{\rtf1\ansi{\fonttbl\f0\fswiss Helvetica;}{\colortbl;\red0\green0\blue0;}{\*\generator Writer;}\f0\fs24 Hello, \b world\b0 !\par Caf\'e9 \u8212? done\par}";
        assert_eq!(rtf(src), "Hello, world!\n\nCafé — done");
    }

    #[test]
    fn real_documents_from_the_system() {
        // macOS's textutil writes real .docx, .odt, .rtf and .doc files.
        if std::process::Command::new("textutil").arg("-help").output().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("leaf-docs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("in.txt");
        std::fs::write(&src, "First paragraph of the book.\n\nSecond paragraph, with café.\n").unwrap();
        for ext in ["docx", "odt", "rtf", "doc", "html"] {
            let out = dir.join(format!("book.{ext}"));
            let ok = std::process::Command::new("textutil").args(["-convert", ext]).arg(&src).arg("-output").arg(&out).status().unwrap();
            assert!(ok.success());
            let text = text(&out).unwrap_or_else(|e| panic!("{ext}: {e}"));
            assert!(text.contains("First paragraph of the book."), "{ext}: {text:?}");
            assert!(text.contains("Second paragraph, with café."), "{ext}: {text:?}");
        }
    }
}
