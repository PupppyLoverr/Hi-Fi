use crate::{Block, Cell, DocKind, Document, Sheet, Slide};
use anyhow::{Context, Result, bail};
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::events::Event;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub(super) fn write(path: &Path, doc: &Document) -> Result<()> {
    let parts = match doc.kind {
        DocKind::Docx => docx_parts(doc),
        DocKind::Pptx => pptx_parts(doc),
        DocKind::Xlsx => xlsx_parts(doc),
        DocKind::Pdf => bail!("PDF is not an OOXML document"),
    };
    let file = File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, contents) in parts {
        zip.start_file(name, options)?;
        zip.write_all(contents.as_bytes())?;
    }
    zip.finish()?;
    Ok(())
}

pub(super) fn read(path: &Path, kind: DocKind) -> Result<Document> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut archive = ZipArchive::new(file).context("reading OOXML package")?;
    let core = zip_string(&mut archive, "docProps/core.xml").unwrap_or_default();
    let title = parse_xml(&core)
        .ok()
        .and_then(|root| find(&root, "dc:title").map(text_content))
        .filter(|s| !s.is_empty())
        .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_default();
    match kind {
        DocKind::Docx => read_docx(&mut archive, title),
        DocKind::Pptx => read_pptx(&mut archive, title),
        DocKind::Xlsx => read_xlsx(&mut archive, title),
        DocKind::Pdf => bail!("PDF is not an OOXML document"),
    }
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn parts(title: &str) -> Vec<(String, String)> {
    vec![
        (
            "[Content_Types].xml".into(),
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/></Types>"
                .into(),
        ),
        (
            "_rels/.rels".into(),
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/></Relationships>".into(),
        ),
        ("docProps/core.xml".into(), format!("<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>{}</dc:title></cp:coreProperties>", xml(title))),
    ]
}

fn docx_parts(doc: &Document) -> Vec<(String, String)> {
    let mut p = parts(&doc.title);
    p[0].1 = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/><Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/><Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/><Override PartName=\"/word/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/><Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/></Types>".into();
    p[1].1 = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/></Relationships>".into();
    let mut body = String::new();
    for block in &doc.blocks {
        match block {
            Block::Heading { level, text } => {
                let style = match level.clamp(&1, &3) {
                    1 => "Heading1",
                    2 => "Heading2",
                    _ => "Heading3",
                };
                body.push_str(&word_para(text, Some(style), None));
            }
            Block::Paragraph { text } => body.push_str(&word_para(text, None, None)),
            Block::Bullet { text } => body.push_str(&word_para(text, None, Some(1))),
            Block::Numbered { text } => body.push_str(&word_para(text, None, Some(2))),
            Block::Quote { text } => body.push_str(&word_para(text, Some("Quote"), None)),
            Block::Code { text } => body.push_str(&word_para(text, Some("Code"), None)),
            Block::PageBreak => body.push_str("<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>"),
            Block::Table { rows } => {
                body.push_str("<w:tbl><w:tblPr><w:tblBorders><w:top w:val=\"single\" w:sz=\"4\" w:color=\"999999\"/><w:left w:val=\"single\" w:sz=\"4\" w:color=\"999999\"/><w:bottom w:val=\"single\" w:sz=\"4\" w:color=\"999999\"/><w:right w:val=\"single\" w:sz=\"4\" w:color=\"999999\"/><w:insideH w:val=\"single\" w:sz=\"4\" w:color=\"999999\"/><w:insideV w:val=\"single\" w:sz=\"4\" w:color=\"999999\"/></w:tblBorders></w:tblPr>");
                for row in rows {
                    body.push_str("<w:tr>");
                    for cell in row {
                        body.push_str(
                            "<w:tc><w:tcPr><w:tcW w:w=\"2400\" w:type=\"dxa\"/></w:tcPr>",
                        );
                        body.push_str(&word_para(cell, None, None));
                        body.push_str("</w:tc>");
                    }
                    body.push_str("</w:tr>");
                }
                body.push_str("</w:tbl>");
            }
        }
    }
    body.push_str("<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" w:header=\"720\" w:footer=\"720\" w:gutter=\"0\"/></w:sectPr>");
    p.push(("word/document.xml".into(), format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}</w:body></w:document>")));
    p.push(("word/_rels/document.xml.rels".into(), "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering\" Target=\"numbering.xml\"/><Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"theme/theme1.xml\"/></Relationships>".into()));
    p.push(("word/styles.xml".into(), "<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"Aptos\" w:hAnsi=\"Aptos\"/><w:sz w:val=\"22\"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after=\"120\"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/></w:style><w:style w:type=\"paragraph\" w:styleId=\"Title\"><w:name w:val=\"Title\"/><w:basedOn w:val=\"Normal\"/><w:rPr><w:b/><w:sz w:val=\"36\"/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Heading1\"><w:name w:val=\"heading 1\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/><w:rPr><w:b/><w:sz w:val=\"32\"/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Heading2\"><w:name w:val=\"heading 2\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/><w:rPr><w:b/><w:sz w:val=\"28\"/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Heading3\"><w:name w:val=\"heading 3\"/><w:basedOn w:val=\"Normal\"/><w:rPr><w:b/><w:sz w:val=\"24\"/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Quote\"><w:name w:val=\"Quote\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:ind w:left=\"360\"/></w:pPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Code\"><w:name w:val=\"Code\"/><w:basedOn w:val=\"Normal\"/><w:rPr><w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\"/></w:rPr></w:style></w:styles>".into()));
    p.push(("word/numbering.xml".into(), "<w:numbering xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:abstractNum w:abstractNumId=\"0\"><w:multiLevelType w:val=\"singleLevel\"/><w:lvl w:ilvl=\"0\"><w:start w:val=\"1\"/><w:numFmt w:val=\"bullet\"/><w:lvlText w:val=\"•\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"720\" w:hanging=\"360\"/></w:pPr></w:lvl></w:abstractNum><w:abstractNum w:abstractNumId=\"1\"><w:multiLevelType w:val=\"singleLevel\"/><w:lvl w:ilvl=\"0\"><w:start w:val=\"1\"/><w:numFmt w:val=\"decimal\"/><w:lvlText w:val=\"%1.\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"720\" w:hanging=\"360\"/></w:pPr></w:lvl></w:abstractNum><w:num w:numId=\"1\"><w:abstractNumId w:val=\"0\"/></w:num><w:num w:numId=\"2\"><w:abstractNumId w:val=\"1\"/></w:num></w:numbering>".into()));
    p.push(("word/theme/theme1.xml".into(), theme_xml()));
    p
}

fn word_para(text: &str, style: Option<&str>, num: Option<u8>) -> String {
    let mut ppr = String::new();
    if let Some(style) = style {
        ppr.push_str(&format!("<w:pStyle w:val=\"{style}\"/>"));
    }
    if let Some(num) = num {
        ppr.push_str(&format!(
            "<w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"{num}\"/></w:numPr>"
        ));
    }
    let runs = bold_runs(text, "w");
    format!("<w:p><w:pPr>{ppr}</w:pPr>{runs}</w:p>")
}

fn bold_runs(text: &str, prefix: &str) -> String {
    let parts = bold_parts(text);
    parts
        .into_iter()
        .map(|(s, bold)| {
            let b = if bold {
                format!("<{prefix}:rPr><{prefix}:b/></{prefix}:rPr>")
            } else {
                String::new()
            };
            format!(
                "<{prefix}:r>{b}<{prefix}:t xml:space=\"preserve\">{}</{prefix}:t></{prefix}:r>",
                xml(&s)
            )
        })
        .collect()
}

fn bold_parts(s: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut rest = s;
    let mut bold = false;
    while let Some(i) = rest.find("**") {
        let after = &rest[i + 2..];
        if let Some(end) = after.find("**") {
            if i > 0 {
                out.push((rest[..i].into(), bold));
            }
            out.push((after[..end].into(), !bold));
            rest = &after[end + 2..];
            bold = false;
        } else {
            out.push((rest.into(), bold));
            return out;
        }
    }
    if !rest.is_empty() {
        out.push((rest.into(), bold));
    }
    if out.is_empty() {
        out.push((String::new(), false));
    }
    out
}

fn theme_xml() -> String {
    "<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"Hi-Fi\"><a:themeElements><a:clrScheme name=\"Hi-Fi\"><a:dk1><a:srgbClr val=\"111111\"/></a:dk1><a:lt1><a:srgbClr val=\"FFFFFF\"/></a:lt1><a:dk2><a:srgbClr val=\"333333\"/></a:dk2><a:lt2><a:srgbClr val=\"F5F5F5\"/></a:lt2><a:accent1><a:srgbClr val=\"555555\"/></a:accent1><a:accent2><a:srgbClr val=\"777777\"/></a:accent2><a:accent3><a:srgbClr val=\"999999\"/></a:accent3><a:accent4><a:srgbClr val=\"BBBBBB\"/></a:accent4><a:accent5><a:srgbClr val=\"CCCCCC\"/></a:accent5><a:accent6><a:srgbClr val=\"DDDDDD\"/></a:accent6><a:hlink><a:srgbClr val=\"333333\"/></a:hlink><a:folHlink><a:srgbClr val=\"555555\"/></a:folHlink></a:clrScheme><a:fontScheme name=\"Hi-Fi\"><a:majorFont><a:latin typeface=\"Aptos Display\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:majorFont><a:minorFont><a:latin typeface=\"Aptos\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:minorFont></a:fontScheme><a:fmtScheme name=\"Hi-Fi\"><a:fillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:gradFill rotWithShape=\"1\"><a:gsLst><a:gs pos=\"0\"><a:schemeClr val=\"phClr\"><a:lumMod val=\"75000\"/></a:schemeClr></a:gs><a:gs pos=\"100000\"><a:schemeClr val=\"phClr\"><a:lumMod val=\"50000\"/></a:schemeClr></a:gs></a:gsLst><a:lin ang=\"5400000\" scaled=\"0\"/></a:gradFill><a:gradFill rotWithShape=\"1\"><a:gsLst><a:gs pos=\"0\"><a:schemeClr val=\"phClr\"><a:lumMod val=\"60000\"/></a:schemeClr></a:gs><a:gs pos=\"100000\"><a:schemeClr val=\"phClr\"><a:lumMod val=\"30000\"/></a:schemeClr></a:gs></a:gsLst><a:lin ang=\"5400000\" scaled=\"0\"/></a:gradFill></a:fillStyleLst><a:lnStyleLst><a:ln w=\"6350\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/></a:ln><a:ln w=\"12700\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/></a:ln><a:ln w=\"19050\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/></a:ln></a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"><a:lumMod val=\"95000\"/></a:schemeClr></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"><a:lumMod val=\"90000\"/></a:schemeClr></a:solidFill></a:bgFillStyleLst></a:fmtScheme></a:themeElements><a:objectDefaults/><a:extraClrSchemeLst/></a:theme>".into()
}

fn zip_string<R: Read + std::io::Seek>(zip: &mut ZipArchive<R>, name: &str) -> Result<String> {
    let mut file = zip
        .by_name(name)
        .with_context(|| format!("missing package part {name}"))?;
    let mut s = String::new();
    file.read_to_string(&mut s)?;
    Ok(s)
}

#[derive(Default, Debug)]
struct Node {
    name: String,
    attrs: HashMap<String, String>,
    text: String,
    children: Vec<Node>,
}

fn parse_xml(input: &str) -> Result<Node> {
    let mut reader = Reader::from_str(input);
    reader.config_mut().trim_text(false);
    let mut stack = vec![Node::default()];
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                let mut n = Node {
                    name: String::from_utf8_lossy(e.name().as_ref()).into_owned(),
                    ..Node::default()
                };
                for a in e.attributes().with_checks(false) {
                    let a = a?;
                    n.attrs.insert(
                        String::from_utf8_lossy(a.key.as_ref()).into_owned(),
                        a.decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())?
                            .into_owned(),
                    );
                }
                stack.push(n);
            }
            Event::Empty(e) => {
                let mut n = Node {
                    name: String::from_utf8_lossy(e.name().as_ref()).into_owned(),
                    ..Node::default()
                };
                for a in e.attributes().with_checks(false) {
                    let a = a?;
                    n.attrs.insert(
                        String::from_utf8_lossy(a.key.as_ref()).into_owned(),
                        a.decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())?
                            .into_owned(),
                    );
                }
                stack.last_mut().unwrap().children.push(n);
            }
            Event::Text(e) => {
                let decoded = e.decode()?;
                stack
                    .last_mut()
                    .unwrap()
                    .text
                    .push_str(&quick_xml::escape::unescape(&decoded)?);
            }
            Event::CData(e) => stack.last_mut().unwrap().text.push_str(&e.decode()?),
            Event::End(_) if stack.len() > 1 => {
                let n = stack.pop().unwrap();
                stack.last_mut().unwrap().children.push(n);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(stack.pop().unwrap_or_default())
}

fn find<'a>(node: &'a Node, name: &str) -> Option<&'a Node> {
    if node.name == name {
        return Some(node);
    }
    node.children.iter().find_map(|c| find(c, name))
}

fn text_content(node: &Node) -> String {
    let mut s = node.text.clone();
    for child in &node.children {
        s.push_str(&text_content(child));
    }
    s
}

fn read_docx<R: Read + std::io::Seek>(zip: &mut ZipArchive<R>, title: String) -> Result<Document> {
    let document = parse_xml(&zip_string(zip, "word/document.xml")?)?;
    let body = find(&document, "w:body").context("DOCX has no document body")?;
    let mut blocks = Vec::new();
    for child in &body.children {
        if child.name == "w:p" {
            if find(child, "w:br")
                .is_some_and(|br| br.attrs.get("w:type").is_some_and(|t| t == "page"))
            {
                blocks.push(Block::PageBreak);
                continue;
            }
            let text = word_text(child);
            if text.is_empty() {
                continue;
            }
            let style = find(child, "w:pStyle")
                .and_then(|n| n.attrs.get("w:val"))
                .map(String::as_str)
                .unwrap_or("");
            let num = find(child, "w:numId")
                .and_then(|n| n.attrs.get("w:val"))
                .is_some_and(|n| n == "1" || n == "2");
            let block = match style {
                "Heading1" | "Title" => Block::Heading { level: 1, text },
                "Heading2" => Block::Heading { level: 2, text },
                "Heading3" => Block::Heading { level: 3, text },
                "Quote" => Block::Quote { text },
                "Code" => Block::Code { text },
                _ if num => {
                    let id = find(child, "w:numId")
                        .and_then(|n| n.attrs.get("w:val"))
                        .map(String::as_str);
                    if id == Some("2") {
                        Block::Numbered { text }
                    } else {
                        Block::Bullet { text }
                    }
                }
                _ => Block::Paragraph { text },
            };
            blocks.push(block);
        } else if child.name == "w:tbl" {
            let rows = child
                .children
                .iter()
                .filter(|n| n.name == "w:tr")
                .map(|row| {
                    row.children
                        .iter()
                        .filter(|n| n.name == "w:tc")
                        .map(|cell| find(cell, "w:p").map(word_text).unwrap_or_default())
                        .collect()
                })
                .collect();
            blocks.push(Block::Table { rows });
        }
    }
    Ok(Document::docx(title, blocks))
}

fn word_text(node: &Node) -> String {
    fn walk(n: &Node, out: &mut String) {
        if n.name == "w:r" {
            let bold = find(n, "w:b").is_some();
            let text = n
                .children
                .iter()
                .filter(|c| c.name == "w:t")
                .map(text_content)
                .collect::<String>();
            if !text.is_empty() {
                if bold {
                    out.push_str("**");
                }
                out.push_str(&text);
                if bold {
                    out.push_str("**");
                }
            }
            if find(n, "w:tab").is_some() {
                out.push('\t');
            }
            return;
        }
        for c in &n.children {
            walk(c, out);
        }
    }
    let mut out = String::new();
    walk(node, &mut out);
    out
}

fn pptx_parts(doc: &Document) -> Vec<(String, String)> {
    let mut p = parts(&doc.title);
    p[0].1 = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/><Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/><Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/><Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/><Override PartName=\"/ppt/notesMasters/notesMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.notesMaster+xml\"/><Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/></Types>".into();
    p[1].1 = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"ppt/presentation.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/></Relationships>".into();
    p.push(("ppt/presentation.xml".into(), format!("<p:presentation xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"><p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst><p:notesMasterIdLst><p:notesMasterId r:id=\"rId2\"/></p:notesMasterIdLst><p:sldIdLst>{}</p:sldIdLst><p:sldSz cx=\"12192000\" cy=\"6858000\" type=\"screen16x9\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/><p:defaultTextStyle><a:defPPr><a:defRPr lang=\"en-US\"/></a:defPPr></p:defaultTextStyle></p:presentation>", doc.slides.iter().enumerate().map(|(i, _)| format!("<p:sldId id=\"{}\" r:id=\"rId{}\"/>", 256+i, i+3)).collect::<String>())));
    let mut rels = String::from(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"slideMasters/slideMaster1.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesMaster\" Target=\"notesMasters/notesMaster1.xml\"/>",
    );
    for (i, _) in doc.slides.iter().enumerate() {
        rels.push_str(&format!("<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide{}.xml\"/>", i+3, i+1));
        p.push((
            format!("ppt/slides/slide{}.xml", i + 1),
            slide_xml(&doc.slides[i]),
        ));
        p.push((format!("ppt/slides/_rels/slide{}.xml.rels", i+1), format!("<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide\" Target=\"../notesSlides/notesSlide{}.xml\"/></Relationships>", i+1)));
        p.push((
            format!("ppt/notesSlides/notesSlide{}.xml", i + 1),
            notes_xml(&doc.slides[i]),
        ));
        p.push((format!("ppt/notesSlides/_rels/notesSlide{}.xml.rels", i+1), format!("<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesMaster\" Target=\"../notesMasters/notesMaster1.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"../slides/slide{}.xml\"/></Relationships>", i+1)));
    }
    rels.push_str("</Relationships>");
    p.push(("ppt/_rels/presentation.xml.rels".into(), rels));
    p.push(("ppt/slideMasters/slideMaster1.xml".into(), "<p:sldMaster xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr></p:spTree></p:cSld><p:clrMap accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" bg1=\"lt1\" bg2=\"lt2\" folHlink=\"folHlink\" hlink=\"hlink\" tx1=\"dk1\" tx2=\"dk2\"/><p:sldLayoutIdLst><p:sldLayoutId id=\"1\" r:id=\"rId1\"/></p:sldLayoutIdLst><p:txStyles><p:titleStyle/><p:bodyStyle/><p:otherStyle/></p:txStyles></p:sldMaster>".into()));
    p.push(("ppt/slideMasters/_rels/slideMaster1.xml.rels".into(), "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"../theme/theme1.xml\"/></Relationships>".into()));
    p.push(("ppt/slideLayouts/slideLayout1.xml".into(), "<p:sldLayout xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" type=\"blank\" preserve=\"1\"><p:cSld name=\"Blank\"><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>".into()));
    p.push(("ppt/slideLayouts/_rels/slideLayout1.xml.rels".into(), "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"../slideMasters/slideMaster1.xml\"/></Relationships>".into()));
    p.push(("ppt/notesMasters/notesMaster1.xml".into(), "<p:notesMaster xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr></p:spTree></p:cSld><p:clrMap accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" bg1=\"lt1\" bg2=\"lt2\" folHlink=\"folHlink\" hlink=\"hlink\" tx1=\"dk1\" tx2=\"dk2\"/></p:notesMaster>".into()));
    p.push(("ppt/notesMasters/_rels/notesMaster1.xml.rels".into(), "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"../theme/theme1.xml\"/></Relationships>".into()));
    p.push(("ppt/theme/theme1.xml".into(), theme_xml()));
    let overrides = (1..=doc.slides.len())
        .map(|i| format!("<Override PartName=\"/ppt/slides/slide{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/><Override PartName=\"/ppt/notesSlides/notesSlide{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml\"/>"))
        .collect::<String>();
    p[0].1 = p[0].1.replace("</Types>", &format!("{overrides}</Types>"));
    p
}

fn slide_xml(slide: &Slide) -> String {
    let title = shape_xml(2, "Title", "title", &slide.title, false);
    let body = shape_xml(3, "Content", "body", &slide.bullets.join("\n"), true);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><p:sld xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>{title}{body}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"
    )
}

fn shape_xml(id: u8, name: &str, ph: &str, text: &str, bullets: bool) -> String {
    let (x, y, width, height) = match ph {
        "title" => (914400, 685800, 10363200, 1371600),
        _ if name == "Notes" => (685800, 685800, 5486400, 7772400),
        _ => (914400, 2286000, 10363200, 4000000),
    };
    let paragraphs = text
        .lines()
        .map(|line| {
            let props = if bullets {
                "<a:pPr lvl=\"0\"><a:buChar char=\"•\"/></a:pPr>"
            } else {
                "<a:pPr/>"
            };
            format!(
                "<a:p>{props}{}{}</a:p>",
                bold_runs(line, "a"),
                if line.is_empty() {
                    "<a:endParaRPr lang=\"en-US\"/>"
                } else {
                    ""
                }
            )
        })
        .collect::<String>();
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{name}\"/><p:cNvSpPr/><p:nvPr><p:ph type=\"{ph}\"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{width}\" cy=\"{height}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/><a:ln><a:noFill/></a:ln></p:spPr><p:txBody><a:bodyPr wrap=\"square\"/><a:lstStyle/>{paragraphs}</p:txBody></p:sp>"
    )
}

fn notes_xml(slide: &Slide) -> String {
    format!(
        "<p:notes xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>{}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:notes>",
        shape_xml(2, "Notes", "body", &slide.notes, false)
    )
}

fn read_pptx<R: Read + std::io::Seek>(zip: &mut ZipArchive<R>, title: String) -> Result<Document> {
    let mut names = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .filter(|n| {
            n.starts_with("ppt/slides/slide") && n.ends_with(".xml") && !n.contains("_rels")
        })
        .collect::<Vec<_>>();
    names.sort_by_key(|n| {
        n.trim_start_matches("ppt/slides/slide")
            .trim_end_matches(".xml")
            .parse::<u32>()
            .unwrap_or(0)
    });
    let mut slides = Vec::new();
    for name in names {
        let xml = zip_string(zip, &name)?;
        let root = parse_xml(&xml)?;
        let mut slide = Slide {
            title: String::new(),
            bullets: Vec::new(),
            notes: String::new(),
        };
        let mut shapes = Vec::new();
        collect_nodes(&root, "p:sp", &mut shapes);
        for shape in shapes {
            let placeholder = find(shape, "p:ph")
                .and_then(|n| n.attrs.get("type"))
                .map(String::as_str)
                .unwrap_or("");
            let paras = find(shape, "p:txBody")
                .map(|n| {
                    n.children
                        .iter()
                        .filter(|c| c.name == "a:p")
                        .map(drawing_text)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let text = paras
                .iter()
                .filter(|s| !s.is_empty())
                .cloned()
                .collect::<Vec<_>>();
            match placeholder {
                "title" | "ctrTitle" => slide.title = text.join("\n"),
                "body" => slide.bullets.extend(text),
                _ => {}
            }
        }
        let number = name
            .trim_start_matches("ppt/slides/slide")
            .trim_end_matches(".xml");
        if let Ok(notes) = zip_string(zip, &format!("ppt/notesSlides/notesSlide{number}.xml"))
            && let Ok(root) = parse_xml(&notes)
        {
            let mut shapes = Vec::new();
            collect_nodes(&root, "p:sp", &mut shapes);
            for shape in shapes {
                if find(shape, "p:ph")
                    .and_then(|n| n.attrs.get("type"))
                    .is_some_and(|t| t == "body")
                {
                    slide.notes = find(shape, "p:txBody")
                        .map(|body| {
                            body.children
                                .iter()
                                .filter(|child| child.name == "a:p")
                                .map(drawing_text)
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default();
                }
            }
        }
        slides.push(slide);
    }
    Ok(Document::pptx(title, slides))
}

fn drawing_text(p: &Node) -> String {
    let mut runs = Vec::new();
    collect_nodes(p, "a:r", &mut runs);
    let mut s = String::new();
    for run in runs {
        let text = find(run, "a:t").map(text_content).unwrap_or_default();
        if text.is_empty() {
            continue;
        }
        if find(run, "a:b").is_some() {
            s.push_str("**");
            s.push_str(&text);
            s.push_str("**");
        } else {
            s.push_str(&text);
        }
    }
    s
}

fn collect_nodes<'a>(node: &'a Node, name: &str, out: &mut Vec<&'a Node>) {
    if node.name == name {
        out.push(node);
    }
    for c in &node.children {
        collect_nodes(c, name, out);
    }
}

fn xlsx_parts(doc: &Document) -> Vec<(String, String)> {
    let mut p = parts(&doc.title);
    p[0].1 = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/><Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/></Types>".into();
    p[1].1 = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/></Relationships>".into();
    let sheets = if doc.sheets.is_empty() {
        vec![Sheet {
            name: "Sheet1".into(),
            rows: Vec::new(),
        }]
    } else {
        doc.sheets.clone()
    };
    let sheet_list = sheets
        .iter()
        .enumerate()
        .map(|(i, s)| {
            format!(
                "<sheet name=\"{}\" sheetId=\"{}\" r:id=\"rId{}\"/>",
                xml(&s.name),
                i + 1,
                i + 1
            )
        })
        .collect::<String>();
    p.push(("xl/workbook.xml".into(), format!("<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><bookViews><workbookView activeTab=\"0\"/></bookViews><sheets>{sheet_list}</sheets><calcPr calcId=\"191029\" fullCalcOnLoad=\"1\"/></workbook>")));
    let rels = sheets.iter().enumerate().map(|(i,_)| format!("<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet{}.xml\"/>", i+1, i+1)).collect::<String>();
    p.push(("xl/_rels/workbook.xml.rels".into(), format!("<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{rels}<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/></Relationships>", sheets.len()+1)));
    p.push(("xl/styles.xml".into(), "<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Aptos\"/></font></fonts><fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills><borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs><cellXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/></cellXfs><cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles></styleSheet>".into()));
    for (i, sheet) in sheets.iter().enumerate() {
        let mut body = String::new();
        for (ri, row) in sheet.rows.iter().enumerate() {
            body.push_str(&format!("<row r=\"{}\">", ri + 1));
            for (ci, cell) in row.iter().enumerate() {
                let address = format!("{}{}", column_name(ci), ri + 1);
                match cell {
                    Cell::Number(n) => body.push_str(&format!("<c r=\"{address}\"><v>{n}</v></c>")),
                    Cell::Text(s) if s.starts_with('=') => body.push_str(&format!("<c r=\"{address}\"><f>{}</f><v></v></c>", xml(&s[1..]))),
                    Cell::Text(s) => body.push_str(&format!("<c r=\"{address}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>", xml(s))),
                }
            }
            body.push_str("</row>");
        }
        let dimension = sheet.rows.iter().map(Vec::len).max().unwrap_or(1);
        let rows = sheet.rows.len().max(1);
        p.push((format!("xl/worksheets/sheet{}.xml", i+1), format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><dimension ref=\"A1:{}{}\"/><sheetViews><sheetView workbookViewId=\"0\"/></sheetViews><sheetFormatPr defaultRowHeight=\"15\"/><sheetData>{body}</sheetData><pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/></worksheet>", column_name(dimension.saturating_sub(1)), rows)));
    }
    let overrides = (1..=sheets.len())
        .map(|i| format!("<Override PartName=\"/xl/worksheets/sheet{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>"))
        .collect::<String>();
    p[0].1 = p[0].1.replace("</Types>", &format!("{overrides}</Types>"));
    p
}

fn column_name(mut col: usize) -> String {
    let mut s = String::new();
    loop {
        s.insert(0, (b'A' + (col % 26) as u8) as char);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    s
}

fn read_xlsx<R: Read + std::io::Seek>(zip: &mut ZipArchive<R>, title: String) -> Result<Document> {
    let root = parse_xml(&zip_string(zip, "xl/workbook.xml")?)?;
    let workbook_rels = parse_xml(&zip_string(zip, "xl/_rels/workbook.xml.rels")?)?;
    let mut rel_targets = HashMap::new();
    let mut rels = Vec::new();
    collect_nodes(&workbook_rels, "Relationship", &mut rels);
    for rel in rels {
        if let (Some(id), Some(target)) = (rel.attrs.get("Id"), rel.attrs.get("Target")) {
            rel_targets.insert(id.clone(), target.clone());
        }
    }
    let shared = zip_string(zip, "xl/sharedStrings.xml")
        .ok()
        .and_then(|s| parse_xml(&s).ok())
        .map(|root| {
            let mut si = Vec::new();
            collect_nodes(&root, "si", &mut si);
            si.into_iter().map(text_content).collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut sheet_nodes = Vec::new();
    collect_nodes(&root, "sheet", &mut sheet_nodes);
    let mut sheets = Vec::new();
    for sheet in sheet_nodes {
        let name = sheet
            .attrs
            .get("name")
            .cloned()
            .unwrap_or_else(|| "Sheet1".into());
        let target = sheet
            .attrs
            .get("r:id")
            .and_then(|id| rel_targets.get(id))
            .cloned()
            .unwrap_or_default();
        let entry = if target.starts_with('/') {
            target.trim_start_matches('/').to_string()
        } else {
            format!("xl/{target}")
        };
        let xml = zip_string(zip, &entry)?;
        let sheet_root = parse_xml(&xml)?;
        let mut row_nodes = Vec::new();
        collect_nodes(&sheet_root, "row", &mut row_nodes);
        let mut rows = Vec::new();
        for row_node in row_nodes {
            let mut cells = Vec::new();
            collect_nodes(row_node, "c", &mut cells);
            let mut row = Vec::new();
            for cell in cells {
                let address = cell.attrs.get("r").map(String::as_str).unwrap_or("");
                let index = address
                    .chars()
                    .take_while(|c| c.is_ascii_alphabetic())
                    .fold(0usize, |n, c| {
                        n * 26 + (c.to_ascii_uppercase() as usize - 'A' as usize + 1)
                    })
                    .saturating_sub(1);
                while row.len() < index {
                    row.push(Cell::Text(String::new()));
                }
                let formula = find(cell, "f").map(text_content);
                let value = if let Some(f) = formula {
                    Cell::Text(format!("={f}"))
                } else if cell.attrs.get("t").is_some_and(|t| t == "inlineStr") {
                    Cell::Text(find(cell, "is").map(text_content).unwrap_or_default())
                } else if cell.attrs.get("t").is_some_and(|t| t == "s") {
                    let ix = find(cell, "v")
                        .and_then(|n| text_content(n).parse::<usize>().ok())
                        .unwrap_or(usize::MAX);
                    Cell::Text(shared.get(ix).cloned().unwrap_or_default())
                } else if let Some(v) = find(cell, "v") {
                    let text = text_content(v);
                    text.parse::<f64>()
                        .map(Cell::Number)
                        .unwrap_or(Cell::Text(text))
                } else {
                    Cell::Text(String::new())
                };
                row.push(value);
            }
            rows.push(row);
        }
        sheets.push(Sheet { name, rows });
    }
    Ok(Document::xlsx(title, sheets))
}
