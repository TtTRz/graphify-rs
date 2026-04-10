//! Detection of sensitive / secret files that must never be ingested.

use std::path::Path;

/// Filename extensions that are inherently sensitive (private keys, certs, etc.).
const SENSITIVE_EXTENSIONS: &[&str] = &[
    ".pem", ".key", ".p12", ".pfx", ".cert", ".crt", ".der", ".p8",
];

/// Exact filenames (case-insensitive) that are sensitive.
const SENSITIVE_FILENAMES: &[&str] = &[
    ".env",
    ".envrc",
    ".netrc",
    ".pgpass",
    ".htpasswd",
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
    "id_rsa.pub",
    "id_dsa.pub",
    "id_ecdsa.pub",
    "id_ed25519.pub",
    "aws_credentials",
    "gcloud_credentials",
];

/// Substrings that, when found in the filename (case-insensitive), mark it as
/// sensitive.
const SENSITIVE_SUBSTRINGS: &[&str] = &[
    "credential",
    "secret",
    "passwd",
    "password",
    "token",
    "private_key",
    "service.account",
];

/// Returns `true` when the file at `path` looks like it contains secrets.
///
/// Checks are performed against the file *name* and the full path string
/// (lowercased).
pub fn is_sensitive(path: &Path) -> bool {
    let filename = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n.to_ascii_lowercase(),
        None => return false,
    };

    // Extension check
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let dot_ext = format!(".{}", ext.to_ascii_lowercase());
        if SENSITIVE_EXTENSIONS.contains(&dot_ext.as_str()) {
            return true;
        }
    }

    // Exact filename match
    for name in SENSITIVE_FILENAMES {
        if filename == *name {
            return true;
        }
    }

    // Substring match against filename
    for substr in SENSITIVE_SUBSTRINGS {
        if filename.contains(substr) {
            return true;
        }
    }

    // Substring match against full path (catches dirs like `secrets/`)
    let full_path_lower = path.to_string_lossy().to_ascii_lowercase();
    for substr in SENSITIVE_SUBSTRINGS {
        if full_path_lower.contains(substr) {
            return true;
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
    fn sensitive_extensions() {
        assert!(is_sensitive(Path::new("server.pem")));
        assert!(is_sensitive(Path::new("tls.key")));
        assert!(is_sensitive(Path::new("cert.p12")));
        assert!(is_sensitive(Path::new("bundle.crt")));
        assert!(is_sensitive(Path::new("push.p8")));
    }

    #[test]
    fn sensitive_filenames() {
        assert!(is_sensitive(Path::new(".env")));
        assert!(is_sensitive(Path::new(".envrc")));
        assert!(is_sensitive(Path::new(".netrc")));
        assert!(is_sensitive(Path::new(".pgpass")));
        assert!(is_sensitive(Path::new("id_rsa")));
        assert!(is_sensitive(Path::new("id_ed25519.pub")));
        assert!(is_sensitive(Path::new("aws_credentials")));
    }

    #[test]
    fn sensitive_substrings_in_filename() {
        assert!(is_sensitive(Path::new("db_password.txt")));
        assert!(is_sensitive(Path::new("api_token.json")));
        assert!(is_sensitive(Path::new("my_credentials.yaml")));
        assert!(is_sensitive(Path::new("private_key.pem")));
    }

    #[test]
    fn sensitive_substrings_in_path() {
        assert!(is_sensitive(&PathBuf::from("config/secrets/app.yaml")));
        assert!(is_sensitive(&PathBuf::from(
            "deploy/credentials/service.json"
        )));
    }

    #[test]
    fn not_sensitive() {
        assert!(!is_sensitive(Path::new("main.rs")));
        assert!(!is_sensitive(Path::new("README.md")));
        assert!(!is_sensitive(Path::new("src/lib.rs")));
        assert!(!is_sensitive(Path::new("package.json")));
    }

    #[test]
    fn case_insensitive() {
        assert!(is_sensitive(Path::new(".ENV")));
        assert!(is_sensitive(Path::new("SERVER.PEM")));
        assert!(is_sensitive(Path::new("API_TOKEN.json")));
    }
}
