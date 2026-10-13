mod editor;
mod model;
mod ooxml;
mod pdf;

pub use editor::editor_html;
pub use model::{Block, Cell, DocKind, Document, Sheet, Slide};

use anyhow::{Context, Result, bail};
use std::fs;
use std::path::Path;

pub fn write(path: &Path, doc: &Document) -> Result<()> {
    let kind = DocKind::from_path(path).context("unsupported document extension")?;
    if kind != doc.kind {
        bail!(
            "document kind {:?} does not match {}",
            doc.kind,
            path.display()
        );
    }
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    match kind {
        DocKind::Pdf => pdf::write(path, doc),
        DocKind::Docx | DocKind::Pptx | DocKind::Xlsx => ooxml::write(path, doc),
    }
}

pub fn read(path: &Path) -> Result<Document> {
    let kind = DocKind::from_path(path).context("unsupported document extension")?;
    match kind {
        DocKind::Pdf => pdf::read(path),
        DocKind::Docx | DocKind::Pptx | DocKind::Xlsx => ooxml::read(path, kind),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(ext: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("hifi-docs-{}-{nonce}.{ext}", std::process::id()))
    }

    #[test]
    fn docx_round_trip_preserves_bold_and_tables() {
        let path = temp_path("docx");
        let doc = Document::docx(
            "Quarterly plan",
            vec![
                Block::Heading {
                    level: 1,
                    text: "A **clear** plan".into(),
                },
                Block::Paragraph {
                    text: "Revenue grew **12%**.".into(),
                },
                Block::Bullet {
                    text: "Ship the export".into(),
                },
                Block::Table {
                    rows: vec![
                        vec!["Quarter".into(), "Revenue".into()],
                        vec!["Q1".into(), "120".into()],
                    ],
                },
            ],
        );
        write(&path, &doc).unwrap();
        assert_eq!(read(&path).unwrap(), doc);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn pptx_round_trip_preserves_bold_bullets_and_notes() {
        let path = temp_path("pptx");
        let doc = Document::pptx(
            "Launch review",
            vec![Slide {
                title: "The **launch**".into(),
                bullets: vec!["First milestone".into(), "**Ship** in June".into()],
                notes: "Confirm owners.".into(),
            }],
        );
        write(&path, &doc).unwrap();
        assert_eq!(read(&path).unwrap(), doc);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn xlsx_round_trip_preserves_formulas() {
        let path = temp_path("xlsx");
        let doc = Document::xlsx(
            "Forecast",
            vec![Sheet {
                name: "Revenue".into(),
                rows: vec![
                    vec![Cell::Text("Month".into()), Cell::Text("Total".into())],
                    vec![Cell::Text("January".into()), Cell::Number(120.0)],
                    vec![
                        Cell::Text("Quarter".into()),
                        Cell::Text("=SUM(B2:B2)".into()),
                    ],
                ],
            }],
        );
        write(&path, &doc).unwrap();
        assert_eq!(read(&path).unwrap(), doc);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn ooxml_round_trip_unescapes_text_and_attribute_entities() {
        let text = "Notion & <Bear> \"quoted\" 'apostrophe'";

        let docx_path = temp_path("docx");
        let docx = Document::docx("Entities", vec![Block::Paragraph { text: text.into() }]);
        write(&docx_path, &docx).unwrap();
        assert_eq!(read(&docx_path).unwrap(), docx);
        let _ = std::fs::remove_file(docx_path);

        let pptx_path = temp_path("pptx");
        let pptx = Document::pptx(
            "Entities",
            vec![Slide {
                title: text.into(),
                bullets: vec![text.into()],
                notes: text.into(),
            }],
        );
        write(&pptx_path, &pptx).unwrap();
        assert_eq!(read(&pptx_path).unwrap(), pptx);
        let _ = std::fs::remove_file(pptx_path);

        let xlsx_path = temp_path("xlsx");
        let xlsx = Document::xlsx(
            "Entities",
            vec![Sheet {
                name: text.into(),
                rows: vec![vec![Cell::Text(text.into())]],
            }],
        );
        write(&xlsx_path, &xlsx).unwrap();
        assert_eq!(read(&xlsx_path).unwrap(), xlsx);
        let _ = std::fs::remove_file(xlsx_path);
    }

    #[test]
    fn pdf_xref_offsets_point_to_objects() {
        let path = temp_path("pdf");
        let doc = Document::pdf(
            "Brief",
            vec![
                Block::Heading {
                    level: 1,
                    text: "A short report".into(),
                },
                Block::Paragraph {
                    text: "Revenue grew **12%** this quarter.".into(),
                },
            ],
        );
        write(&path, &doc).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"%PDF-1.4"));
        assert!(String::from_utf8_lossy(&bytes).contains("/F2 11 Tf"));
        let xref = bytes.windows(5).position(|w| w == b"xref\n").unwrap();
        let table = std::str::from_utf8(&bytes[xref..]).unwrap();
        let mut lines = table.lines();
        assert_eq!(lines.next(), Some("xref"));
        assert_eq!(lines.next(), Some("0 8"));
        let _free = lines.next();
        for object in 1..8 {
            let offset: usize = lines.next().unwrap()[..10].parse().unwrap();
            assert!(bytes[offset..].starts_with(format!("{object} 0 obj").as_bytes()));
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn pdf_content_paginates_long_documents() {
        let path = temp_path("pdf");
        let blocks = (0..120)
            .map(|index| Block::Paragraph {
                text: format!(
                    "Section {index}: the report contains several lines of useful detail."
                ),
            })
            .collect();
        write(&path, &Document::pdf("Long report", blocks)).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let source = String::from_utf8_lossy(&bytes);
        assert!(source.matches("/Type /Page /Parent").count() > 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn pdf_read_unescapes_titles_and_joins_wrapped_lines() {
        let path = temp_path("pdf");
        let text = "A wrapped paragraph that is deliberately long enough to span multiple lines when written to an A4 PDF page.";
        write(
            &path,
            &Document::pdf(
                "Rust (Programming Language)",
                vec![Block::Paragraph { text: text.into() }],
            ),
        )
        .unwrap();
        let readback = read(&path).unwrap();
        assert_eq!(readback.title, "Rust (Programming Language)");
        assert_eq!(
            readback.blocks,
            vec![Block::Paragraph { text: text.into() }]
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn editor_html_contains_content_and_save_message() {
        let doc = Document::docx(
            "Project brief",
            vec![Block::Paragraph {
                text: "Delivery begins Monday".into(),
            }],
        );
        let html = editor_html(&doc, "/tmp/brief.docx", None, false);
        assert!(html.contains("Delivery begins Monday"));
        assert!(html.contains("doc_save"));
        assert!(html.contains("Save"));
    }
}
