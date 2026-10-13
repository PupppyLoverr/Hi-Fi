use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocKind {
    Pdf,
    Docx,
    Pptx,
    Xlsx,
}

impl DocKind {
    pub fn from_path(path: impl AsRef<Path>) -> Option<Self> {
        match path
            .as_ref()
            .extension()?
            .to_str()?
            .to_ascii_lowercase()
            .as_str()
        {
            "pdf" => Some(Self::Pdf),
            "docx" => Some(Self::Docx),
            "pptx" => Some(Self::Pptx),
            "xlsx" => Some(Self::Xlsx),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Docx => "docx",
            Self::Pptx => "pptx",
            Self::Xlsx => "xlsx",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Document {
    pub kind: DocKind,
    pub title: String,
    #[serde(default)]
    pub blocks: Vec<Block>,
    #[serde(default)]
    pub slides: Vec<Slide>,
    #[serde(default)]
    pub sheets: Vec<Sheet>,
}

impl Document {
    pub fn pdf(title: impl Into<String>, blocks: Vec<Block>) -> Self {
        Self {
            kind: DocKind::Pdf,
            title: title.into(),
            blocks,
            slides: Vec::new(),
            sheets: Vec::new(),
        }
    }

    pub fn docx(title: impl Into<String>, blocks: Vec<Block>) -> Self {
        Self {
            kind: DocKind::Docx,
            title: title.into(),
            blocks,
            slides: Vec::new(),
            sheets: Vec::new(),
        }
    }

    pub fn pptx(title: impl Into<String>, slides: Vec<Slide>) -> Self {
        Self {
            kind: DocKind::Pptx,
            title: title.into(),
            blocks: Vec::new(),
            slides,
            sheets: Vec::new(),
        }
    }

    pub fn xlsx(title: impl Into<String>, sheets: Vec<Sheet>) -> Self {
        Self {
            kind: DocKind::Xlsx,
            title: title.into(),
            blocks: Vec::new(),
            slides: Vec::new(),
            sheets,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Heading { level: u8, text: String },
    Paragraph { text: String },
    Bullet { text: String },
    Numbered { text: String },
    Quote { text: String },
    Code { text: String },
    Table { rows: Vec<Vec<String>> },
    PageBreak,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slide {
    pub title: String,
    #[serde(default)]
    pub bullets: Vec<String>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sheet {
    pub name: String,
    pub rows: Vec<Vec<Cell>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Cell {
    Number(f64),
    Text(String),
}
