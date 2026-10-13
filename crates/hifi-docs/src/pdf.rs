use crate::{Block, DocKind, Document};
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::Path;

pub(super) fn write(path: &Path, doc: &Document) -> Result<()> {
    let mut pages: Vec<Vec<(String, u8)>> = vec![Vec::new()];
    for block in &doc.blocks {
        if matches!(block, Block::PageBreak) {
            if !pages.last().unwrap().is_empty() {
                pages.push(Vec::new());
            }
            continue;
        }
        let (text, size, prefix) = match block {
            Block::Heading { level, text } => {
                (text.as_str(), if *level <= 1 { 17 } else { 14 }, "")
            }
            Block::Paragraph { text } => (text.as_str(), 11, ""),
            Block::Bullet { text } => (text.as_str(), 11, "- "),
            Block::Numbered { text } => (text.as_str(), 11, "1. "),
            Block::Quote { text } => (text.as_str(), 11, "    "),
            Block::Code { text } => (text.as_str(), 10, ""),
            Block::Table { rows } => {
                for row in rows {
                    push_wrapped(&mut pages, &row.join("  |  "), 10, 82);
                }
                pages.last_mut().unwrap().push((String::new(), 4));
                continue;
            }
            Block::PageBreak => unreachable!(),
        };
        let content = format!("{prefix}{text}");
        push_wrapped(&mut pages, &content, size, if size >= 14 { 66 } else { 88 });
        push_line(&mut pages, String::new(), 5);
    }
    if pages.len() > 1 && pages.last().is_some_and(Vec::is_empty) {
        pages.pop();
    }
    if pages.is_empty() {
        pages.push(Vec::new());
    }
    let mut objects = Vec::new();
    objects.push("<< /Type /Catalog /Pages 2 0 R >>".to_string());
    let kids = (0..pages.len())
        .map(|i| format!("{} 0 R", 5 + 2 * i))
        .collect::<Vec<_>>()
        .join(" ");
    objects.push(format!(
        "<< /Type /Pages /Kids [{kids}] /Count {} >>",
        pages.len()
    ));
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into());
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>".into());
    let mut bold = false;
    for page in &pages {
        let mut stream = String::new();
        let mut y = 790i32;
        for (line, size) in page {
            if !line.is_empty() {
                let mut x = 54.0f32;
                let mut after_bold = false;
                for (text, is_bold) in pdf_segments(line, &mut bold) {
                    if text.is_empty() {
                        continue;
                    }
                    let separated = after_bold && text.starts_with(char::is_whitespace);
                    let rendered = if separated {
                        x += 3.0;
                        text.trim_start()
                    } else {
                        &text
                    };
                    stream.push_str(&format!(
                        "BT /{} {} Tf 1 0 0 1 {x:.1} {y} Tm ({}) Tj ET\n",
                        if is_bold { "F2" } else { "F1" },
                        size,
                        pdf_escape(rendered)
                    ));
                    x += pdf_text_width(rendered, *size);
                    after_bold = is_bold;
                }
            }
            y -= i32::from(*size) + 7;
        }
        let page_id = objects.len() + 1;
        let content_id = page_id + 1;
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {content_id} 0 R >>"));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{}endstream",
            stream.len(),
            stream
        ));
    }
    let info_id = objects.len() + 1;
    objects.push(format!(
        "<< /Title ({}) /Producer (Hi-Fi) >>",
        pdf_escape(&doc.title)
    ));
    let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = vec![0usize];
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, object).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets.iter().skip(1) {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Info {info_id} 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    fs::write(path, out).with_context(|| format!("writing {}", path.display()))
}

fn push_wrapped(pages: &mut Vec<Vec<(String, u8)>>, text: &str, size: u8, width: usize) {
    let mut line = String::new();
    for word in text.split_whitespace() {
        let line_len = line.replace("**", "").chars().count();
        let word_len = word.replace("**", "").chars().count();
        if !line.is_empty() && line_len + 1 + word_len > width {
            push_line(pages, line, size);
            line = String::new();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        push_line(pages, line, size);
    }
}

fn push_line(pages: &mut Vec<Vec<(String, u8)>>, line: String, size: u8) {
    let used = pages
        .last()
        .unwrap()
        .iter()
        .map(|(_, line_size)| u32::from(*line_size) + 7)
        .sum::<u32>();
    if 790u32.saturating_sub(used + u32::from(size) + 7) < 55 {
        pages.push(Vec::new());
    }
    pages.last_mut().unwrap().push((line, size));
}

fn pdf_segments(text: &str, bold: &mut bool) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("**") {
        if start > 0 {
            out.push((rest[..start].to_string(), *bold));
        }
        *bold = !*bold;
        rest = &rest[start + 2..];
    }
    if !rest.is_empty() {
        out.push((rest.to_string(), *bold));
    }
    out
}

fn pdf_text_width(text: &str, size: u8) -> f32 {
    let units = text
        .chars()
        .map(|ch| match ch {
            ' ' => 278,
            '!' | '\'' | ',' | '.' | ':' | ';' | 'I' | 'i' | 'j' | 'l' | '|' => 278,
            'f' | 'r' | 't' => 333,
            'm' => 833,
            'w' => 722,
            'M' => 833,
            'W' => 944,
            'A' | 'B' | 'C' | 'D' | 'G' | 'H' | 'N' | 'O' | 'Q' | 'R' | 'U' | 'V' | 'X' => 722,
            'F' | 'P' | 'S' | 'T' | 'Y' | 'Z' => 611,
            'K' | 'x' | 'y' => 500,
            'b' | 'd' | 'h' | 'k' | 'n' | 'o' | 'p' | 'q' | 'u' | 'v' | '0'..='9' => 556,
            _ => 556,
        })
        .sum::<u32>();
    units as f32 * f32::from(size) / 1000.0
}

fn pdf_escape(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '(' => "\\(".into(),
            ')' => "\\)".into(),
            '\\' => "\\\\".into(),
            '\n' | '\r' => " ".into(),
            c if c.is_ascii() => c.to_string(),
            _ => "?".into(),
        })
        .collect()
}

pub(super) fn read(path: &Path) -> Result<Document> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    if !bytes.starts_with(b"%PDF-") {
        bail!("not a PDF file");
    }
    let source = String::from_utf8_lossy(&bytes);
    let title = source
        .split_once("/Title")
        .and_then(|(_, tail)| extract_pdf_strings(tail).into_iter().next())
        .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let mut blocks = Vec::new();
    let mut rest = source.as_ref();
    while let Some((_, tail)) = rest.split_once("stream\n") {
        let Some((stream, after)) = tail.split_once("\nendstream") else {
            break;
        };
        let mut paragraph = String::new();
        let mut previous_y = None;
        for (y, line) in extract_pdf_lines(stream) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(last_y) = previous_y
                && !(0.0..=26.0).contains(&(last_y - y))
            {
                blocks.push(Block::Paragraph {
                    text: std::mem::take(&mut paragraph),
                });
            }
            if !paragraph.is_empty() {
                paragraph.push(' ');
            }
            paragraph.push_str(line);
            previous_y = Some(y);
        }
        if !paragraph.is_empty() {
            blocks.push(Block::Paragraph { text: paragraph });
        }
        rest = after;
    }
    Ok(Document {
        kind: DocKind::Pdf,
        title,
        blocks,
        slides: Vec::new(),
        sheets: Vec::new(),
    })
}

fn extract_pdf_strings(stream: &str) -> Vec<String> {
    let mut output = Vec::new();
    let bytes = stream.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let Some(start) = bytes[cursor..].iter().position(|byte| *byte == b'(') else {
            break;
        };
        let start = cursor + start;
        let mut index = start + 1;
        let mut depth = 1;
        let mut text = Vec::new();
        while index < bytes.len() && depth > 0 {
            let byte = bytes[index];
            index += 1;
            match byte {
                b'\\' if index < bytes.len() => {
                    let escaped = bytes[index];
                    index += 1;
                    match escaped {
                        b'n' => text.push(b'\n'),
                        b'r' => text.push(b'\r'),
                        b't' => text.push(b'\t'),
                        b'b' => text.push(8),
                        b'f' => text.push(12),
                        b'(' | b')' | b'\\' => text.push(escaped),
                        b'\n' => {}
                        b'\r' => {
                            if bytes.get(index) == Some(&b'\n') {
                                index += 1;
                            }
                        }
                        b'0'..=b'7' => {
                            let mut value = u16::from(escaped - b'0');
                            for _ in 0..2 {
                                if let Some(digit @ b'0'..=b'7') = bytes.get(index).copied() {
                                    value = value * 8 + u16::from(digit - b'0');
                                    index += 1;
                                } else {
                                    break;
                                }
                            }
                            text.push(value as u8);
                        }
                        other => text.push(other),
                    }
                }
                b'(' => {
                    depth += 1;
                    text.push(byte);
                }
                b')' => {
                    depth -= 1;
                    if depth > 0 {
                        text.push(byte);
                    }
                }
                other => text.push(other),
            }
        }
        if depth == 0 {
            output.push(String::from_utf8_lossy(&text).into_owned());
            cursor = index;
        } else {
            break;
        }
    }
    output
}

fn extract_pdf_lines(stream: &str) -> Vec<(f32, String)> {
    let mut lines: Vec<(f32, String)> = Vec::new();
    let mut previous_segment: Option<(f32, f32)> = None;
    let mut rest = stream;
    while let Some((_, tail)) = rest.split_once("BT") {
        let Some((text_object, after)) = tail.split_once("ET") else {
            break;
        };
        if let Some((position, _)) = text_object.split_once(" Tm") {
            let coordinates: Vec<_> = position.split_whitespace().rev().take(2).collect();
            let x = coordinates
                .get(1)
                .and_then(|value| value.parse::<f32>().ok());
            let y = coordinates
                .first()
                .and_then(|value| value.parse::<f32>().ok());
            let size = text_object
                .split_once(" Tf")
                .and_then(|(font, _)| font.split_whitespace().last())
                .and_then(|value| value.parse::<u8>().ok())
                .unwrap_or(11);
            let text = extract_pdf_strings(text_object).join("");
            if let (Some(x), Some(y)) = (x, y)
                && !text.is_empty()
            {
                if let Some((last_y, line)) = lines.last_mut()
                    && (*last_y - y).abs() < 0.1
                {
                    if let Some((previous_x, previous_width)) = previous_segment
                        && x - (previous_x + previous_width) > 1.5
                    {
                        line.push(' ');
                    }
                    line.push_str(&text);
                } else {
                    lines.push((y, text.clone()));
                }
                previous_segment = Some((x, pdf_text_width(&text, size)));
            }
        }
        rest = after;
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::extract_pdf_strings;

    #[test]
    fn extracts_simple_pdf_strings() {
        assert_eq!(
            extract_pdf_strings("BT (one) Tj ET\nBT (two\\(x\\)) Tj ET"),
            ["one", "two(x)"]
        );
    }

    #[test]
    fn unescapes_control_and_octal_sequences() {
        assert_eq!(
            extract_pdf_strings(r"(line\n\(nested\)\\\101)"),
            ["line\n(nested)\\A"]
        );
    }
}
