//! Parser trait for pluggable extraction backends.
//!
//! The [`Parser`] trait allows swapping between regex-based extraction (current
//! default) and future tree-sitter–backed extraction without changing the
//! pipeline.

use std::path::Path;

use graphify_core::model::ExtractionResult;

/// A source-file parser that produces graph nodes and edges.
///
/// Implementations must be `Send + Sync` so they can be shared across threads
/// when processing files in parallel.
pub trait Parser: Send + Sync {
    /// Parse a single source file and return the extracted entities and
    /// relationships.
    fn parse(&self, path: &Path, source: &[u8]) -> ExtractionResult;

    /// File extensions this parser can handle (e.g. `[".py", ".pyi"]`).
    fn supported_extensions(&self) -> &[&str];
}

/// The default regex-based parser that delegates to [`crate::ast_extract`].
pub struct RegexParser;

impl Parser for RegexParser {
    fn parse(&self, path: &Path, source: &[u8]) -> ExtractionResult {
        let lang = crate::language_for_path(path).unwrap_or("generic");
        let source_str = String::from_utf8_lossy(source);
        crate::ast_extract::extract_file(path, &source_str, lang)
    }

    fn supported_extensions(&self) -> &[&str] {
        // All extensions from the DISPATCH table
        &[
            ".py", ".js", ".jsx", ".ts", ".tsx", ".go", ".rs", ".java", ".c", ".h", ".cpp", ".cc",
            ".cxx", ".hpp", ".rb", ".cs", ".kt", ".kts", ".scala", ".php", ".swift", ".lua",
            ".toc", ".zig", ".ps1", ".ex", ".exs", ".m", ".mm", ".jl", ".sql",
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn regex_parser_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RegexParser>();
    }

    #[test]
    fn regex_parser_produces_output() {
        let parser = RegexParser;
        let source = b"def hello():\n    pass\n";
        let result = parser.parse(Path::new("test.py"), source);
        // Should have at least the file node + function node
        assert!(!result.nodes.is_empty());
    }
}
