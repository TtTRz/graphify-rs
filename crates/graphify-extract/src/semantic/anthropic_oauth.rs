use std::path::{Path, PathBuf};
use tracing::debug;

/// Read OAuth access token from Claude Code's local credential storage or macOS Keychain.
pub fn read_claude_code_oauth_token() -> Option<String> {
    // 1. Check environment variables
    for env_var in &["CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_AUTH_TOKEN"] {
        if let Ok(token) = std::env::var(env_var) {
            let token = token.trim();
            if !token.is_empty() {
                debug!("Found Claude Code OAuth token in env var {env_var}");
                return Some(token.to_string());
            }
        }
    }

    // 2. On macOS, query the macOS Keychain service "Claude Code-credentials"
    #[cfg(target_os = "macos")]
    if let Some(token) = read_token_from_macos_keychain() {
        return Some(token);
    }

    // 3. Check filesystem candidate paths
    for path in get_candidate_credential_paths() {
        if let Some(token) = read_token_from_file(&path) {
            return Some(token);
        }
    }

    None
}

#[cfg(target_os = "macos")]
fn read_token_from_macos_keychain() -> Option<String> {
    let output = std::process::Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", "Claude Code-credentials", "-w"])
        .output()
        .ok()?;

    if !output.status.success() {
        debug!("No Claude Code credentials found in macOS Keychain");
        return None;
    }

    let secret = String::from_utf8(output.stdout).ok()?;
    let secret = secret.trim();
    if secret.is_empty() {
        return None;
    }

    if secret.starts_with('{') {
        parse_token_from_json_str(secret)
    } else {
        Some(secret.to_string())
    }
}

fn get_candidate_credential_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(custom_dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        let p = PathBuf::from(custom_dir);
        paths.push(p.join(".credentials.json"));
        paths.push(p.join("credentials.json"));
    }

    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".claude").join(".credentials.json"));
        paths.push(home.join(".claude").join("credentials.json"));
        paths.push(home.join(".claude").join("config.json"));
        paths.push(home.join(".claude.json"));
    }

    paths
}

fn read_token_from_file(path: &Path) -> Option<String> {
    if !path.exists() {
        debug!("Claude Code credentials not found at {}", path.display());
        return None;
    }

    let content = std::fs::read_to_string(path).ok()?;
    parse_token_from_json_str(&content)
}

pub fn parse_token_from_json_str(content: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(content).ok()?;
    let target = json.get("claudeAiOauth").unwrap_or(&json);

    if let Some(expires_at) = target
        .get("expiresAt")
        .or_else(|| json.get("expiresAt"))
        .and_then(serde_json::Value::as_i64)
    {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        if expires_at < now_ms {
            debug!("Claude Code OAuth token expired at {}", expires_at);
            return None;
        }
    }

    for field in &["accessToken", "access_token", "oauthToken", "token"] {
        if let Some(val) = target
            .get(*field)
            .or_else(|| json.get(*field))
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
    fn reads_nested_claude_ai_oauth_structure() {
        let raw = r#"{
            "claudeAiOauth": {
                "accessToken": "sk-ant-oat01-test-token",
                "refreshToken": "sk-ant-ort01-test",
                "expiresAt": 4102444800000
            }
        }"#;
        let token = parse_token_from_json_str(raw);
        assert_eq!(token.as_deref(), Some("sk-ant-oat01-test-token"));
    }

    #[test]
    fn rejects_expired_nested_token() {
        let raw = r#"{
            "claudeAiOauth": {
                "accessToken": "sk-ant-oat01-expired",
                "expiresAt": 1000
            }
        }"#;
        let token = parse_token_from_json_str(raw);
        assert!(token.is_none());
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
    fn ignores_apikey_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, r#"{{"apiKey": "sk-ant-xxx"}}"#).unwrap();

        let token = read_token_from_file(&path);
        assert!(token.is_none());
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
        let future = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
            + 3_600_000;
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
