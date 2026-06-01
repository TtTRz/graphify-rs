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

/// Word-boundary substrings to match against the filename stem (without extension).
/// Uses `_` and `.` as word boundaries to avoid false positives like
/// "secret_resolver.rs" or "tokenizer.rs".
const SENSITIVE_WORDS: &[&str] = &[
    "credentials",
    "secret",
    "passwd",
    "password",
    "private_key",
    "service.account",
    "access_token",
    "auth_token",
    "refresh_token",
    "id_token",
    "api_token",
    "oauth_token",
    "bearer_token",
];

/// Path segments (directory names) that indicate a sensitive location.
/// Only matches directory names, not arbitrary substrings in the path.
const SENSITIVE_DIR_SEGMENTS: &[&str] =
    &["secrets", "credentials", ".ssh", ".gnupg", ".aws", ".kube"];

/// Check if `word` appears as a complete word in `s`, using `_` and `.` as boundaries.
fn matches_word(s: &str, word: &str) -> bool {
    let mut start = 0;
    while let Some(pos) = s[start..].find(word) {
        let abs_pos = start + pos;
        let after = abs_pos + word.len();

        let boundary_before =
            abs_pos == 0 || s.as_bytes()[abs_pos - 1] == b'_' || s.as_bytes()[abs_pos - 1] == b'.';
        let boundary_after =
            after >= s.len() || s.as_bytes()[after] == b'_' || s.as_bytes()[after] == b'.';

        if boundary_before && boundary_after {
            return true;
        }
        start = abs_pos + 1;
    }
    false
}

/// Returns `true` when the file at `path` looks like it contains secrets.
///
/// Checks are performed against the file *name* and the full path string
/// (lowercased).
pub fn is_sensitive(path: &Path) -> bool {
    let filename = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n.to_ascii_lowercase(),
        None => return false,
    };

    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let dot_ext = format!(".{}", ext.to_ascii_lowercase());
        if SENSITIVE_EXTENSIONS.contains(&dot_ext.as_str()) {
            return true;
        }
    }

    for name in SENSITIVE_FILENAMES {
        if filename == *name {
            return true;
        }
    }

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    for word in SENSITIVE_WORDS {
        if matches_word(&stem, word) {
            return true;
        }
    }

    for component in path.ancestors().skip(1) {
        if let Some(dir_name) = component.file_name().and_then(|n| n.to_str()) {
            let dir_lower = dir_name.to_ascii_lowercase();
            if SENSITIVE_DIR_SEGMENTS.contains(&dir_lower.as_str()) {
                return true;
            }
        }
    }

    false
}

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
    fn no_false_positive_tokenizer() {
        // "token" alone should not trigger — only compound words like "api_token"
        assert!(!is_sensitive(Path::new("tokenizer.rs")));
        assert!(!is_sensitive(Path::new("token_handler.go")));
        assert!(!is_sensitive(Path::new("token_provider.ts")));
    }

    #[test]
    fn no_false_positive_secret_dir_in_filename() {
        // A file with "secret" as a word boundary in its stem IS still detected,
        // which is correct — "secret_manager" does contain the word "secret".
        // The improvement is that partial matches like "secretresolver" no longer trigger.
        assert!(is_sensitive(Path::new("src/utils/secret_resolver.rs")));
        assert!(!is_sensitive(Path::new("src/utils/secretresolver.rs")));
        assert!(!is_sensitive(Path::new("src/utils/mysecret.rs")));
    }

    #[test]
    fn case_insensitive() {
        assert!(is_sensitive(Path::new(".ENV")));
        assert!(is_sensitive(Path::new("SERVER.PEM")));
        assert!(is_sensitive(Path::new("API_TOKEN.json")));
    }
}
