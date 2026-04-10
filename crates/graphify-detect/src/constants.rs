//! Extension lists, thresholds, and skip-directory names.

/// Source code file extensions.
pub const CODE_EXTENSIONS: &[&str] = &[
    ".py", ".ts", ".js", ".jsx", ".tsx", ".go", ".rs", ".java", ".cpp", ".cc", ".cxx", ".c", ".h",
    ".hpp", ".rb", ".swift", ".kt", ".kts", ".cs", ".scala", ".php", ".lua", ".toc", ".zig",
    ".ps1", ".ex", ".exs", ".m", ".mm", ".jl",
];

/// Documentation file extensions.
pub const DOC_EXTENSIONS: &[&str] = &[".md", ".txt", ".rst"];

/// Academic paper extensions.
pub const PAPER_EXTENSIONS: &[&str] = &[".pdf"];

/// Image file extensions.
pub const IMAGE_EXTENSIONS: &[&str] = &[".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg"];

/// Office document extensions.
pub const OFFICE_EXTENSIONS: &[&str] = &[".docx", ".xlsx"];

/// Warn when total word count exceeds this.
pub const CORPUS_WARN_THRESHOLD: usize = 50_000;

/// Hard upper limit on word count.
pub const CORPUS_UPPER_THRESHOLD: usize = 500_000;

/// Hard upper limit on file count.
pub const FILE_COUNT_UPPER: usize = 200;

/// Directories that should always be skipped during traversal.
pub const SKIP_DIRS: &[&str] = &[
    "venv",
    ".venv",
    "env",
    ".env",
    "node_modules",
    "__pycache__",
    ".git",
    "dist",
    "build",
    "target",
    "out",
    "site-packages",
    "lib64",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".tox",
    ".eggs",
];

/// Regex patterns that signal academic paper content.
pub const PAPER_SIGNALS: &[&str] = &[
    r"arxiv",
    r"\bdoi\b",
    r"\babstract\b",
    r"proceedings",
    r"\bjournal\b",
    r"preprint",
    r"\\cite\{",
    r"\[\d+\]",
    r"\beq\.",
    r"\bequation\b",
    r"\d{4}\.\d{4,5}",
    r"we propose",
    r"literature",
];

/// The number of paper-signal hits required to classify a text file as a paper.
pub const PAPER_SIGNAL_THRESHOLD: usize = 3;

/// How many leading characters of a file to scan for paper signals.
pub const PAPER_PEEK_CHARS: usize = 3000;
