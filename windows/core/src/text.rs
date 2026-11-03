//! Short text excerpts for classification and the preview pane. Port of `TextExtractor.swift`.
//!
//! The local model reads little at a time, so only the beginning of a file is used.
//! Unreadable formats return None (the file is then classified by name and folder).

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use quick_xml::events::Event;

pub const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "md", "markdown", "csv", "tsv", "json", "yaml", "yml", "log", "html", "htm", "xml",
    "tex", "ini",
];
pub const CODE_EXTENSIONS: &[&str] = &[
    "swift", "js", "ts", "tsx", "jsx", "py", "java", "kt", "c", "cpp", "h", "m", "go", "rs", "rb",
    "php", "css", "scss", "sh", "sql", "cs", "ps1", "bat", "cmd", "toml",
];
/// zip containers with XML inside
pub const OFFICE_ZIP_EXTENSIONS: &[&str] = &["docx", "pptx", "xlsx", "hwpx", "odt", "ods", "odp"];
pub const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "heic", "heif", "tiff", "tif", "webp", "gif", "bmp",
];

/// How much of a text/code file is read.
pub const HEAD_BYTES: usize = 64 * 1024;
/// How much XML is read out of an office file (all members together).
pub const XML_CAP: usize = 400_000;
/// PDFs larger than this are not parsed (pdf-extract loads the whole file).
pub const PDF_MAX_BYTES: u64 = 48 * 1024 * 1024;
/// A PDF that takes longer than this is given up on.
pub const PDF_TIMEOUT: Duration = Duration::from_secs(20);
/// Characters handed to the model.
pub const DEFAULT_LIMIT: usize = 1500;

/// Can this format be read for classification?
pub fn can_extract(ext: &str) -> bool {
    ext == "pdf"
        || ext == "rtf"
        || TEXT_EXTENSIONS.contains(&ext)
        || CODE_EXTENSIONS.contains(&ext)
        || OFFICE_ZIP_EXTENSIONS.contains(&ext)
        || (IMAGE_EXTENSIONS.contains(&ext) && ocr_available())
}

/// Image OCR is compiled in (Windows + feature `ocr`).
pub fn ocr_available() -> bool {
    cfg!(all(windows, feature = "ocr"))
}

/// Up to `limit` characters of text from the file (whitespace tidied, "…" when cut).
pub fn extract(path: &Path, ext: &str, limit: usize) -> Option<String> {
    let raw = match ext {
        "pdf" => pdf_text(path, 3),
        "rtf" => head_bytes(path, HEAD_BYTES).map(|b| rtf_text(&b)),
        e if TEXT_EXTENSIONS.contains(&e) || CODE_EXTENSIONS.contains(&e) => {
            head_bytes(path, HEAD_BYTES).map(|b| decode_text(&b))
        }
        e if OFFICE_ZIP_EXTENSIONS.contains(&e) => office_text(path, e),
        e if IMAGE_EXTENSIONS.contains(&e) => ocr_text(path),
        _ => None,
    }?;
    tidy(&raw, limit)
}

/// Collapse runs of spaces/tabs, at most one blank line, trim, cut to `limit` chars + "…".
pub fn tidy(raw: &str, limit: usize) -> Option<String> {
    let mut out = String::with_capacity(raw.len().min(limit * 4 + 8));
    let mut pending_space = false;
    let mut newlines = 0usize;
    for c in raw.chars() {
        match c {
            '\r' => {}
            '\n' => {
                pending_space = false;
                newlines += 1;
            }
            ' ' | '\t' | '\u{a0}' | '\u{3000}' => pending_space = true,
            c if c.is_control() => pending_space = true,
            c => {
                if !out.is_empty() {
                    if newlines > 0 {
                        out.push_str(if newlines >= 2 { "\n\n" } else { "\n" });
                    } else if pending_space {
                        out.push(' ');
                    }
                }
                newlines = 0;
                pending_space = false;
                out.push(c);
            }
        }
    }
    if out.is_empty() {
        return None;
    }
    if out.chars().count() > limit {
        let mut cut: String = out.chars().take(limit).collect();
        cut.push('…');
        return Some(cut);
    }
    Some(out)
}

fn head_bytes(path: &Path, n: usize) -> Option<Vec<u8>> {
    let f = File::open(path).ok()?;
    let mut buf = Vec::with_capacity(n.min(1 << 20));
    f.take(n as u64).read_to_end(&mut buf).ok()?;
    if buf.is_empty() {
        None
    } else {
        Some(buf)
    }
}

/// `read_text_preview`: the first `max_bytes` (capped at 4 MiB) decoded as text.
pub fn read_text_preview(path: &Path, max_bytes: usize) -> std::io::Result<String> {
    let f = File::open(path)?;
    let mut buf = Vec::new();
    f.take(max_bytes.min(4 << 20) as u64)
        .read_to_end(&mut buf)?;
    let mut s = decode_text(&buf);
    s.retain(|c| c != '\0');
    Ok(s)
}

/// Decode bytes of unknown encoding: BOM (UTF-8/UTF-16) → UTF-8 → UTF-16 without BOM →
/// CP949 (Korean Windows), whichever fits. A UTF-8 sequence cut at the end is dropped.
pub fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        let (s, _, _) = encoding_rs::UTF_16LE.decode(bytes); // BOM sniffing picks LE/BE
        return s.into_owned();
    }
    // UTF-16 without BOM is valid UTF-8 byte-wise (ASCII + NULs) — check it first
    if let Some(enc) = utf16_without_bom(bytes) {
        let (s, _) = enc.decode_without_bom_handling(bytes);
        return s.into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => return s.to_string(),
        Err(e) if e.error_len().is_none() => {
            // only the last character was cut off by the read limit
            return String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned();
        }
        Err(_) => {}
    }
    let (cp949, _, had_errors) = encoding_rs::EUC_KR.decode(bytes);
    if !had_errors {
        return cp949.into_owned();
    }
    let lossy = String::from_utf8_lossy(bytes);
    let bad = |s: &str| s.chars().filter(|c| *c == '\u{FFFD}').count();
    if bad(&cp949) <= bad(&lossy) {
        cp949.into_owned()
    } else {
        lossy.into_owned()
    }
}

fn utf16_without_bom(bytes: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    if bytes.len() < 4 {
        return None;
    }
    let pairs = bytes.len() / 2;
    let odd_zero = bytes.iter().skip(1).step_by(2).filter(|b| **b == 0).count();
    let even_zero = bytes.iter().step_by(2).filter(|b| **b == 0).count();
    if odd_zero * 10 > pairs * 3 && even_zero * 10 < pairs {
        Some(encoding_rs::UTF_16LE)
    } else if even_zero * 10 > pairs * 3 && odd_zero * 10 < pairs {
        Some(encoding_rs::UTF_16BE)
    } else {
        None
    }
}

// ───────────── PDF ─────────────

/// Text of the first `pages` pages. pdf-extract can panic or crawl on odd files, so it runs
/// on its own thread, inside `catch_unwind`, with a size cap and a timeout.
fn pdf_text(path: &Path, pages: usize) -> Option<String> {
    let md = std::fs::metadata(path).ok()?;
    if md.len() > PDF_MAX_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("mf-pdf".into())
        .spawn(move || {
            let r = std::panic::catch_unwind(move || pdf_pages(&bytes, pages))
                .ok()
                .flatten();
            let _ = tx.send(r);
        })
        .ok()?;
    rx.recv_timeout(PDF_TIMEOUT).ok().flatten()
}

fn pdf_pages(bytes: &[u8], pages: usize) -> Option<String> {
    let mut doc = pdf_extract::Document::load_mem(bytes).ok()?;
    if doc.is_encrypted() {
        doc.decrypt("").ok()?; // only PDFs without a user password
    }
    let numbers: Vec<u32> = doc.get_pages().keys().copied().take(pages).collect();
    let mut out = String::new();
    for n in numbers {
        let mut s = String::new();
        {
            let mut o = pdf_extract::PlainTextOutput::new(&mut s);
            let _ = pdf_extract::output_doc_page(&doc, &mut o, n);
        }
        out.push_str(&s);
        out.push('\n');
        if out.len() > 8000 {
            break;
        }
    }
    Some(out)
}

// ───────────── Office (zip + XML) ─────────────

fn office_members(ext: &str) -> Option<Vec<&'static str>> {
    Some(match ext {
        "docx" => vec!["word/document.xml"],
        "pptx" => vec![
            "ppt/slides/slide1.xml",
            "ppt/slides/slide2.xml",
            "ppt/slides/slide3.xml",
        ],
        "xlsx" => vec!["xl/sharedStrings.xml"],
        "hwpx" => vec!["Contents/section0.xml"],
        "odt" | "ods" | "odp" => vec!["content.xml"],
        _ => return None,
    })
}

/// Body XML of docx/pptx/xlsx/hwpx/odt with the tags stripped (≤ 400 KB of XML read).
fn office_text(path: &Path, ext: &str) -> Option<String> {
    let members = office_members(ext)?;
    let f = File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(BufReader::new(f)).ok()?;
    let mut budget = XML_CAP;
    let mut out = String::new();
    for m in members {
        if budget == 0 {
            break;
        }
        let Ok(entry) = zip.by_name(m) else { continue };
        let mut buf = Vec::new();
        if entry.take(budget as u64).read_to_end(&mut buf).is_err() {
            continue;
        }
        budget = budget.saturating_sub(buf.len());
        out.push_str(&xml_text(&buf));
        out.push('\n');
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Text content of (possibly truncated) XML; element ends become spaces so words do not
/// run together.
pub fn xml_text(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    let mut reader = quick_xml::Reader::from_str(&s);
    {
        let cfg = reader.config_mut();
        cfg.check_end_names = false;
        cfg.allow_unmatched_ends = true;
    }
    let mut out = String::new();
    loop {
        match reader.read_event() {
            Ok(Event::Text(t)) => out.push_str(&t.xml10_content()),
            Ok(Event::CData(c)) => out.push_str(&c.into_inner()),
            Ok(Event::GeneralRef(r)) => match r.resolve_char_ref() {
                Ok(Some(c)) => out.push(c),
                _ => match r.into_inner().as_ref() {
                    "amp" => out.push('&'),
                    "lt" => out.push('<'),
                    "gt" => out.push('>'),
                    "quot" => out.push('"'),
                    "apos" => out.push('\''),
                    _ => {}
                },
            },
            Ok(Event::End(_)) | Ok(Event::Empty(_)) => {
                if !out.ends_with(' ') && !out.is_empty() {
                    out.push(' ');
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
    }
    out
}

// ───────────── RTF ─────────────

/// Plain text of an RTF document (good enough for classification): control words dropped,
/// `\par` → newline, `\'hh` bytes decoded with the document code page, `\uN` honored, and
/// font/color/style tables and `{\*…}` destinations skipped.
pub fn rtf_text(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut pending: Vec<u8> = Vec::new(); // \'hh bytes waiting for decoding
    let mut codepage: &'static encoding_rs::Encoding = encoding_rs::WINDOWS_1252;
    let mut skip_depth: Option<usize> = None;
    let mut depth = 0usize;
    let mut uc_skip = 1usize;
    let mut skip_chars = 0usize;
    let mut i = 0;
    let flush = |pending: &mut Vec<u8>, out: &mut String, cp: &'static encoding_rs::Encoding| {
        if !pending.is_empty() {
            let (s, _, _) = cp.decode(pending);
            out.push_str(&s);
            pending.clear();
        }
    };
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'{' => {
                depth += 1;
                i += 1;
                if skip_depth.is_none() && bytes.get(i..i + 2) == Some(&b"\\*"[..]) {
                    skip_depth = Some(depth);
                }
            }
            b'}' => {
                if skip_depth == Some(depth) {
                    skip_depth = None;
                }
                depth = depth.saturating_sub(1);
                i += 1;
            }
            b'\\' => {
                i += 1;
                let Some(&c) = bytes.get(i) else { break };
                if c == b'\'' {
                    let hex = bytes
                        .get(i + 1..i + 3)
                        .and_then(|h| std::str::from_utf8(h).ok());
                    if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                        if skip_depth.is_none() {
                            if skip_chars > 0 {
                                skip_chars -= 1;
                            } else {
                                pending.push(v);
                            }
                        }
                    }
                    i += 3;
                    continue;
                }
                if !c.is_ascii_alphabetic() {
                    // control symbol: \\ \{ \} \~ …
                    if skip_depth.is_none() {
                        flush(&mut pending, &mut out, codepage);
                        match c {
                            b'\\' | b'{' | b'}' => out.push(c as char),
                            b'~' => out.push(' '),
                            _ => {}
                        }
                    }
                    i += 1;
                    continue;
                }
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                let word = std::str::from_utf8(&bytes[start..i]).unwrap_or("");
                let num_start = i;
                if i < bytes.len() && (bytes[i] == b'-' || bytes[i].is_ascii_digit()) {
                    i += 1;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let num: Option<i64> = std::str::from_utf8(&bytes[num_start..i])
                    .ok()
                    .and_then(|n| n.parse().ok());
                if i < bytes.len() && bytes[i] == b' ' {
                    i += 1;
                }
                if matches!(
                    word,
                    "fonttbl" | "colortbl" | "stylesheet" | "info" | "pict" | "header" | "footer"
                ) && skip_depth.is_none()
                {
                    skip_depth = Some(depth);
                    continue;
                }
                if skip_depth.is_some() {
                    continue;
                }
                match word {
                    "ansicpg" => {
                        if let Some(cp) = num.and_then(codepage_encoding) {
                            codepage = cp;
                        }
                    }
                    "uc" => uc_skip = num.unwrap_or(1).max(0) as usize,
                    "u" => {
                        flush(&mut pending, &mut out, codepage);
                        if let Some(n) = num {
                            let v = if n < 0 { n + 65536 } else { n } as u32;
                            if let Some(ch) = char::from_u32(v) {
                                out.push(ch);
                            }
                        }
                        skip_chars = uc_skip;
                    }
                    "par" | "line" | "row" => {
                        flush(&mut pending, &mut out, codepage);
                        out.push('\n');
                    }
                    "tab" | "cell" => {
                        flush(&mut pending, &mut out, codepage);
                        out.push('\t');
                    }
                    _ => {}
                }
            }
            b'\r' | b'\n' => i += 1,
            _ => {
                if skip_depth.is_none() {
                    if skip_chars > 0 {
                        skip_chars -= 1;
                    } else {
                        flush(&mut pending, &mut out, codepage);
                        out.push(b as char);
                    }
                }
                i += 1;
            }
        }
    }
    flush(&mut pending, &mut out, codepage);
    out
}

fn codepage_encoding(cp: i64) -> Option<&'static encoding_rs::Encoding> {
    Some(match cp {
        949 => encoding_rs::EUC_KR,
        932 => encoding_rs::SHIFT_JIS,
        936 => encoding_rs::GBK,
        950 => encoding_rs::BIG5,
        1250 => encoding_rs::WINDOWS_1250,
        1251 => encoding_rs::WINDOWS_1251,
        1252 => encoding_rs::WINDOWS_1252,
        65001 => encoding_rs::UTF_8,
        _ => return None,
    })
}

// ───────────── images ─────────────

#[cfg(all(windows, feature = "ocr"))]
fn ocr_text(path: &Path) -> Option<String> {
    crate::ocr::ocr_image(path)
}

#[cfg(not(all(windows, feature = "ocr")))]
fn ocr_text(_path: &Path) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn dir() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("mf-text")
            .tempdir()
            .unwrap()
    }

    #[test]
    fn decodes_utf8_utf16_and_cp949() {
        assert_eq!(decode_text("한글 text".as_bytes()), "한글 text");
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend_from_slice("abc".as_bytes());
        assert_eq!(decode_text(&bom), "abc");
        let mut le = vec![0xFF, 0xFE];
        for u in "한글 ok".encode_utf16() {
            le.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(decode_text(&le), "한글 ok");
        let mut be = vec![0xFE, 0xFF];
        for u in "hi".encode_utf16() {
            be.extend_from_slice(&u.to_be_bytes());
        }
        assert_eq!(decode_text(&be), "hi");
        let mut no_bom = Vec::new();
        for u in "plain utf16 text".encode_utf16() {
            no_bom.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(decode_text(&no_bom), "plain utf16 text");
        let (cp949, _, _) = encoding_rs::EUC_KR.encode("회의록 2024년 보고서");
        assert_eq!(decode_text(&cp949), "회의록 2024년 보고서");
        // a UTF-8 character cut by the read limit is dropped, not turned into garbage
        let s = "가나다".as_bytes();
        assert_eq!(decode_text(&s[..s.len() - 1]), "가나");
    }

    #[test]
    fn tidy_collapses_whitespace_and_cuts() {
        assert_eq!(tidy("  a \t b\n\n\n\nc  ", 100).unwrap(), "a b\n\nc");
        assert_eq!(tidy("abcdef", 3).unwrap(), "abc…");
        assert_eq!(tidy(" \n\t ", 10), None);
    }

    #[test]
    fn text_and_code_heads() {
        let d = dir();
        let p = d.path().join("notes.md");
        let mut big = String::from("# Title\n");
        big.push_str(&"x".repeat(200_000));
        std::fs::write(&p, &big).unwrap();
        let t = extract(&p, "md", 1500).unwrap();
        assert!(t.starts_with("# Title\nxxx"));
        assert_eq!(t.chars().count(), 1501);
        assert!(
            can_extract("rs") && can_extract("docx") && can_extract("pdf") && !can_extract("exe")
        );
        assert_eq!(can_extract("png"), ocr_available());
        let preview = read_text_preview(&p, 10).unwrap();
        assert_eq!(preview, "# Title\nxx");
    }

    fn make_zip(path: &Path, members: &[(&str, &str)]) {
        let f = File::create(path).unwrap();
        let mut w = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, body) in members {
            w.start_file(*name, opts).unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        w.finish().unwrap();
    }

    #[test]
    fn docx_pptx_xlsx_hwpx() {
        let d = dir();
        let docx = d.path().join("report.docx");
        make_zip(
            &docx,
            &[
                ("[Content_Types].xml", "<Types/>"),
                (
                    "word/document.xml",
                    r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="x"><w:body><w:p><w:r><w:t>분기 보고서</w:t></w:r></w:p><w:p><w:r><w:t>Sales &amp; Marketing</w:t><w:tab/><w:t>&#54620;</w:t></w:r></w:p></w:body></w:document>"#,
                ),
            ],
        );
        let t = extract(&docx, "docx", 1500).unwrap();
        assert!(t.contains("분기 보고서"), "{t}");
        assert!(t.contains("Sales & Marketing"), "{t}");
        assert!(t.contains('한'), "{t}");
        assert!(!t.contains('<'));

        let pptx = d.path().join("deck.pptx");
        make_zip(
            &pptx,
            &[
                (
                    "ppt/slides/slide1.xml",
                    "<p:sld><a:t>Slide one</a:t></p:sld>",
                ),
                (
                    "ppt/slides/slide2.xml",
                    "<p:sld><a:t>Slide two</a:t></p:sld>",
                ),
            ],
        );
        let t = extract(&pptx, "pptx", 1500).unwrap();
        assert!(t.contains("Slide one") && t.contains("Slide two"), "{t}");

        let xlsx = d.path().join("sheet.xlsx");
        make_zip(
            &xlsx,
            &[(
                "xl/sharedStrings.xml",
                "<sst><si><t>Budget</t></si><si><t>2025</t></si></sst>",
            )],
        );
        assert!(extract(&xlsx, "xlsx", 1500)
            .unwrap()
            .contains("Budget 2025"));

        let hwpx = d.path().join("doc.hwpx");
        make_zip(
            &hwpx,
            &[(
                "Contents/section0.xml",
                "<hs:sec><hp:t>한글 문서</hp:t></hs:sec>",
            )],
        );
        assert_eq!(extract(&hwpx, "hwpx", 1500).unwrap(), "한글 문서");

        // truncated XML still yields its beginning
        let cut = xml_text(b"<a><b>hello</b><c>wor");
        assert!(cut.contains("hello") && cut.contains("wor"), "{cut}");
        // not a zip
        let bad = d.path().join("bad.docx");
        std::fs::write(&bad, "nope").unwrap();
        assert_eq!(extract(&bad, "docx", 100), None);
    }

    #[test]
    fn rtf_plain_text() {
        let rtf = br"{\rtf1\ansi\ansicpg949{\fonttbl{\f0 Malgun;}}{\colortbl;\red0;}\f0 Hello \b World\b0\par {\*\generator x;}\'c7\'d1\'b1\'db UNI?end}";
        // (the \u control word is spliced in so no unicode escape appears in this source)
        let rtf = String::from_utf8(rtf.to_vec())
            .unwrap()
            .replace("UNI", &format!("{}u54620", '\\'));
        let t = rtf_text(rtf.as_bytes());
        assert!(t.contains("Hello World"), "{t}");
        assert!(t.contains("한글"), "{t}");
        assert!(t.contains("한end"), "{t}");
        assert!(!t.contains("Malgun") && !t.contains("generator"), "{t}");
    }

    #[test]
    fn pdf_garbage_and_minimal_pdf() {
        let d = dir();
        let bad = d.path().join("bad.pdf");
        std::fs::write(&bad, b"%PDF-1.4 garbage").unwrap();
        assert_eq!(extract(&bad, "pdf", 100), None);
        let good = d.path().join("good.pdf");
        std::fs::write(&good, minimal_pdf("Quarterly invoice")).unwrap();
        let t = extract(&good, "pdf", 100).unwrap_or_default();
        assert!(t.contains("Quarterly"), "got {t:?}");
    }

    /// A one-page PDF with a Helvetica text line and a correct xref table.
    fn minimal_pdf(text: &str) -> Vec<u8> {
        let content = format!("BT /F1 24 Tf 72 720 Td ({text}) Tj ET");
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>"
                .to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes(),
        );
        for off in offsets {
            out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objs.len() + 1
            )
            .as_bytes(),
        );
        out
    }
}
