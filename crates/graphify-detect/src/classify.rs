//! File-type classification based on extension and content heuristics.

use std::fs;
use std::path::Path;

use regex::RegexBuilder;
use serde::{Deserialize, Serialize};

use crate::constants::{
    CODE_EXTENSIONS, DOC_EXTENSIONS, IMAGE_EXTENSIONS, OFFICE_EXTENSIONS, PAPER_EXTENSIONS,
    PAPER_PEEK_CHARS, PAPER_SIGNAL_THRESHOLD, PAPER_SIGNALS,
};

// ---------------------------------------------------------------------------
// FileType
// ---------------------------------------------------------------------------

/// The broad category a file belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileType {
    /// Source code (Rust, Python, TypeScript, etc.)
    Code,
    /// Documentation (Markdown, RST, plain text, etc.)
    Document,
    /// Academic paper (PDF, LaTeX, etc.)
    Paper,
    /// Image file (PNG, JPG, SVG, etc.)
    Image,
}

impl std::fmt::Display for FileType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileType::Code => write!(f, "code"),
            FileType::Document => write!(f, "document"),
            FileType::Paper => write!(f, "paper"),
            FileType::Image => write!(f, "image"),
        }
    }
}

// ---------------------------------------------------------------------------
// DetectedFile
// ---------------------------------------------------------------------------

/// A discovered file together with its classification.
#[derive(Debug, Clone)]
pub struct DetectedFile {
    pub path: std::path::PathBuf,
    pub file_type: FileType,
}

// ---------------------------------------------------------------------------
// classify_file
// ---------------------------------------------------------------------------

/// Classify a file by its extension (and, for ambiguous cases, a peek at its
/// content).  Returns `None` if the file type is not recognised.
pub fn classify_file(path: &Path) -> Option<FileType> {
    let ext = extension_lower(path)?;

    if CODE_EXTENSIONS.contains(&ext.as_str()) {
        return Some(FileType::Code);
    }

    if PAPER_EXTENSIONS.contains(&ext.as_str()) {
        // PDFs inside Xcode asset catalogs are treated as images, skip them.
        if is_inside_xcode_asset(path) {
            return None;
        }
        return Some(FileType::Paper);
    }

    if DOC_EXTENSIONS.contains(&ext.as_str()) {
        // Text files that look like academic papers get classified as Paper.
        if looks_like_paper(path) {
            return Some(FileType::Paper);
        }
        return Some(FileType::Document);
    }

    if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        // Images inside Xcode asset catalogs are skipped.
        if is_inside_xcode_asset(path) {
            return None;
        }
        return Some(FileType::Image);
    }

    if OFFICE_EXTENSIONS.contains(&ext.as_str()) {
        return Some(FileType::Document);
    }

    None
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return the lowercase extension including the leading dot, e.g. `".rs"`.
fn extension_lower(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e.to_ascii_lowercase()))
}

/// Check whether the path lives inside an Xcode asset catalog.
fn is_inside_xcode_asset(path: &Path) -> bool {
    for ancestor in path.ancestors() {
        if let Some(name) = ancestor.file_name().and_then(|n| n.to_str()) {
            if name.ends_with(".imageset") || name.ends_with(".xcassets") {
                return true;
            }
        }
    }
    false
}

/// Heuristic: read the first N chars and count regex hits from [`PAPER_SIGNALS`].
/// Returns `true` when the hit count reaches [`PAPER_SIGNAL_THRESHOLD`].
fn looks_like_paper(path: &Path) -> bool {
    let content = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let peek: &str = if content.len() > PAPER_PEEK_CHARS {
        &content[..PAPER_PEEK_CHARS]
    } else {
        &content
    };

    let mut hits = 0usize;
    for pattern in PAPER_SIGNALS {
        if let Ok(re) = RegexBuilder::new(pattern).case_insensitive(true).build() {
            if re.is_match(peek) {
                hits += 1;
                if hits >= PAPER_SIGNAL_THRESHOLD {
                    return true;
                }
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn classify_code_extensions() {
        for ext in &[
            ".py", ".rs", ".ts", ".go", ".java", ".cpp", ".js", ".jsx", ".tsx", ".c", ".h", ".rb",
            ".swift", ".kt", ".cs", ".lua", ".zig", ".jl", ".ex", ".mm",
        ] {
            let p = PathBuf::from(format!("foo{ext}"));
            assert_eq!(
                classify_file(&p),
                Some(FileType::Code),
                "expected Code for {ext}"
            );
        }
    }

    #[test]
    fn classify_doc_extensions() {
        // Non-existent files: looks_like_paper returns false -> Document
        for ext in &[".md", ".txt", ".rst"] {
            let p = PathBuf::from(format!("README{ext}"));
            assert_eq!(
                classify_file(&p),
                Some(FileType::Document),
                "expected Document for {ext}"
            );
        }
    }

    #[test]
    fn classify_image_extensions() {
        for ext in &[".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg"] {
            let p = PathBuf::from(format!("icon{ext}"));
            assert_eq!(
                classify_file(&p),
                Some(FileType::Image),
                "expected Image for {ext}"
            );
        }
    }

    #[test]
    fn classify_office_extensions() {
        for ext in &[".docx", ".xlsx"] {
            let p = PathBuf::from(format!("report{ext}"));
            assert_eq!(
                classify_file(&p),
                Some(FileType::Document),
                "expected Document for {ext}"
            );
        }
    }

    #[test]
    fn classify_pdf_default() {
        assert_eq!(classify_file(Path::new("paper.pdf")), Some(FileType::Paper));
    }

    #[test]
    fn classify_pdf_inside_xcassets() {
        let p = PathBuf::from("Assets.xcassets/icon.imageset/logo.pdf");
        assert_eq!(classify_file(&p), None);
    }

    #[test]
    fn classify_image_inside_xcassets() {
        let p = PathBuf::from("Assets.xcassets/icon.imageset/icon.png");
        assert_eq!(classify_file(&p), None);
    }

    #[test]
    fn classify_unknown_extension() {
        assert_eq!(classify_file(Path::new("data.parquet")), None);
        assert_eq!(classify_file(Path::new("Makefile")), None);
    }

    #[test]
    fn extension_lower_works() {
        assert_eq!(
            extension_lower(Path::new("Foo.RS")),
            Some(".rs".to_string())
        );
        assert_eq!(
            extension_lower(Path::new("bar.Py")),
            Some(".py".to_string())
        );
        assert_eq!(extension_lower(Path::new("noext")), None);
    }

    #[test]
    fn looks_like_paper_on_real_content() {
        let dir = std::env::temp_dir().join("graphify_test_paper_classify");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("paper.md");
        std::fs::write(
            &path,
            "Abstract: We propose a novel method. arxiv:2301.12345. \
             Published in Proceedings of NeurIPS. doi:10.1234",
        )
        .unwrap();
        assert!(looks_like_paper(&path));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn looks_like_paper_false_for_readme() {
        let dir = std::env::temp_dir().join("graphify_test_readme_classify");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("README.md");
        std::fs::write(&path, "# My Project\n\nInstallation instructions.").unwrap();
        assert!(!looks_like_paper(&path));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_type_display() {
        assert_eq!(FileType::Code.to_string(), "code");
        assert_eq!(FileType::Document.to_string(), "document");
        assert_eq!(FileType::Paper.to_string(), "paper");
        assert_eq!(FileType::Image.to_string(), "image");
    }

    #[test]
    fn file_type_serde_roundtrip() {
        let json = serde_json::to_string(&FileType::Code).unwrap();
        assert_eq!(json, r#""code""#);
        let back: FileType = serde_json::from_str(&json).unwrap();
        assert_eq!(back, FileType::Code);
    }
}
