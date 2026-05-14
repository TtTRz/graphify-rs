use tracing::debug;

/// Read OAuth access token from Claude Code's local credential storage.
///
/// Checks both `~/.claude/config.json` and `~/.claude/credentials.json` for an OAuth token.
/// Returns `None` if no file exists or no token field is found.
pub fn read_claude_code_oauth_token() -> Option<String> {
    let home = dirs::home_dir()?;
    let paths = [
        home.join(".claude").join("config.json"),
        home.join(".claude").join("credentials.json"),
    ];
    for path in &paths {
        if let Some(token) = read_token_from_file(path) {
            return Some(token);
        }
    }
    None
}

fn read_token_from_file(path: &std::path::Path) -> Option<String> {
    if !path.exists() {
        debug!("Claude Code credentials not found at {}", path.display());
        return None;
    }

    let content = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;

    // Check token expiry — expiresAt is a Unix timestamp in milliseconds
    if let Some(expires_at) = json.get("expiresAt").and_then(serde_json::Value::as_i64) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        if expires_at < now_ms {
            debug!("Claude Code OAuth token expired at {}", expires_at);
            return None;
        }
    }

    // Try common field names for OAuth access tokens
    for field in &[
        "accessToken",
        "access_token",
        "oauthToken",
        "apiKey",
        "token",
    ] {
        if let Some(val) = json
            .get(*field)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            debug!("Found Claude Code OAuth token in field '{}'", field);
            return Some(val.to_string());
        }
    }

    debug!("No OAuth token found in Claude Code credentials");
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_accesstoken_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, r#"{{"accessToken": "test-oauth-token"}}"#).unwrap();

        let token = read_token_from_file(&path);
        assert_eq!(token.as_deref(), Some("test-oauth-token"));
    }

    #[test]
    fn reads_access_token_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, r#"{{"access_token": "test-oauth-token"}}"#).unwrap();

        let token = read_token_from_file(&path);
        assert_eq!(token.as_deref(), Some("test-oauth-token"));
    }

    #[test]
    fn reads_oauthtoken_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, r#"{{"oauthToken": "test-oauth-token"}}"#).unwrap();

        let token = read_token_from_file(&path);
        assert_eq!(token.as_deref(), Some("test-oauth-token"));
    }

    #[test]
    fn reads_apikey_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, r#"{{"apiKey": "test-oauth-token"}}"#).unwrap();

        let token = read_token_from_file(&path);
        assert_eq!(token.as_deref(), Some("test-oauth-token"));
    }

    #[test]
    fn returns_none_for_missing_file() {
        let token = read_token_from_file(std::path::Path::new("/nonexistent/credentials.json"));
        assert!(token.is_none());
    }

    #[test]
    fn returns_none_for_empty_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, r#"{{"accessToken": ""}}"#).unwrap();

        let token = read_token_from_file(&path);
        assert!(token.is_none());
    }

    #[test]
    fn returns_none_for_no_matching_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, r#"{{"other_field": "value"}}"#).unwrap();

        let token = read_token_from_file(&path);
        assert!(token.is_none());
    }

    #[test]
    fn returns_none_for_expired_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        let mut f = std::fs::File::create(&path).unwrap();
        // expiresAt in the past (1 second ago in milliseconds)
        let past = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
            - 1000;
        write!(
            f,
            r#"{{"accessToken": "expired-token", "expiresAt": {}}}"#,
            past
        )
        .unwrap();

        let token = read_token_from_file(&path);
        assert!(token.is_none());
    }

    #[test]
    fn returns_token_for_valid_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        let mut f = std::fs::File::create(&path).unwrap();
        // expiresAt 1 hour in the future
        let future = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
            + 3600_000;
        write!(
            f,
            r#"{{"accessToken": "valid-token", "expiresAt": {}}}"#,
            future
        )
        .unwrap();

        let token = read_token_from_file(&path);
        assert_eq!(token.as_deref(), Some("valid-token"));
    }
}
