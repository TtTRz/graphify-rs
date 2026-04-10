//! URL, path, and label validation/sanitization for graphify.
//!
//! Ensures all URLs, file paths, and graph labels are safe and well-formed
//! before use. Port of Python `security.py`.

pub mod label_validator;
pub mod path_validator;
pub mod url_validator;

pub use label_validator::sanitize_label;
pub use path_validator::safe_path;
pub use url_validator::validate_url;

use thiserror::Error;

/// Security validation errors.
#[derive(Debug, Error)]
pub enum SecurityError {
    #[error("Invalid URL scheme: {0}")]
    InvalidScheme(String),

    #[error("URL resolves to private IP: {0}")]
    PrivateIp(String),

    #[error("Path traversal detected: {0}")]
    PathTraversal(String),

    #[error("Invalid path: {0}")]
    InvalidPath(String),

    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),
}
