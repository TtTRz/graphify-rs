# LLM Provider Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Support multiple LLM providers (Anthropic OAuth, OpenAI, Ollama, OpenAI-compatible) for semantic extraction via `graphify.toml` config.

**Architecture:** Dual-path design — Anthropic (Messages API + OAuth) and OpenAI-compatible (Chat Completions API). Shared prompt construction and response parsing. Config-driven provider selection with backward-compatible `ANTHROPIC_API_KEY` env var fallback.

**Tech Stack:** Rust, reqwest, serde, tokio, toml, dirs

---

### Task 1: Add provider types and OAuth reader

**Files:**
- Create: `crates/graphify-extract/src/semantic/provider.rs`
- Create: `crates/graphify-extract/src/semantic/anthropic_oauth.rs`
- Modify: `crates/graphify-extract/Cargo.toml`

- [ ] **Step 1: Create `semantic/` module directory**

```bash
mkdir -p crates/graphify-extract/src/semantic
```

- [ ] **Step 2: Move `semantic.rs` → `semantic/mod.rs`**

```bash
git mv crates/graphify-extract/src/semantic.rs crates/graphify-extract/src/semantic/mod.rs
```

- [ ] **Step 3: Verify existing tests still pass**

Run: `cargo test -p graphify-extract`
Expected: All tests pass (file move is transparent to Rust module system)

- [ ] **Step 4: Add `dirs` dependency to graphify-extract**

In `crates/graphify-extract/Cargo.toml`, add to `[dependencies]`:

```toml
dirs = "5"
```

- [ ] **Step 5: Create `provider.rs`**

Create `crates/graphify-extract/src/semantic/provider.rs`:

```rust
use anyhow::{Context, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LLMProvider {
    Anthropic,
    OpenAI,
    Ollama,
    OpenAICompatible,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthType {
    ApiKey,
    Bearer,
}

#[derive(Debug, Clone)]
pub struct LLMProviderConfig {
    pub provider: LLMProvider,
    pub model: String,
    pub api_key: Option<String>,
    pub base_url: String,
    pub auth_type: AuthType,
}

impl LLMProviderConfig {
    const ANTHROPIC_DEFAULT_URL: &str = "https://api.anthropic.com";
    const OPENAI_DEFAULT_URL: &str = "https://api.openai.com/v1";
    const OLLAMA_DEFAULT_URL: &str = "http://localhost:11434/v1";

    pub fn resolve(
        provider_str: &str,
        model: &str,
        anthropic_api_key: Option<&str>,
        anthropic_base_url: Option<&str>,
        openai_api_key: Option<&str>,
        openai_base_url: Option<&str>,
        ollama_base_url: Option<&str>,
        openai_compatible_api_key: Option<&str>,
        openai_compatible_base_url: Option<&str>,
    ) -> Result<Self> {
        let provider = match provider_str {
            "anthropic" => LLMProvider::Anthropic,
            "openai" => LLMProvider::OpenAI,
            "ollama" => LLMProvider::Ollama,
            "openai_compatible" => LLMProvider::OpenAICompatible,
            other => anyhow::bail!(
                "Unknown LLM provider: '{}'. Supported: anthropic, openai, ollama, openai_compatible",
                other
            ),
        };

        if model.is_empty() {
            anyhow::bail!("LLM model is required in [llm] config");
        }

        let (api_key, base_url, auth_type) = match provider {
            LLMProvider::Anthropic => {
                let (key, at) = if let Some(k) = anthropic_api_key {
                    (Some(k.to_string()), AuthType::ApiKey)
                } else if let Ok(k) = std::env::var("ANTHROPIC_API_KEY") {
                    (Some(k), AuthType::ApiKey)
                } else if let Some(token) =
                    super::anthropic_oauth::read_claude_code_oauth_token()
                {
                    (Some(token), AuthType::Bearer)
                } else {
                    (None, AuthType::ApiKey)
                };
                let url = anthropic_base_url
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| Self::ANTHROPIC_DEFAULT_URL.to_string());
                (key, url, at)
            }
            LLMProvider::OpenAI => {
                let key = openai_api_key
                    .map(|s| s.to_string())
                    .or_else(|| std::env::var("OPENAI_API_KEY").ok());
                let url = openai_base_url
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| Self::OPENAI_DEFAULT_URL.to_string());
                (key, url, AuthType::Bearer)
            }
            LLMProvider::Ollama => {
                let url = ollama_base_url
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| Self::OLLAMA_DEFAULT_URL.to_string());
                (None, url, AuthType::Bearer)
            }
            LLMProvider::OpenAICompatible => {
                let key = openai_compatible_api_key.map(|s| s.to_string());
                let url = openai_compatible_base_url
                    .map(|s| s.to_string())
                    .context(
                        "openai_compatible_base_url is required for openai_compatible provider",
                    )?;
                (key, url, AuthType::Bearer)
            }
        };

        Ok(LLMProviderConfig {
            provider,
            model: model.to_string(),
            api_key,
            base_url,
            auth_type,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_anthropic_with_api_key() {
        let config = LLMProviderConfig::resolve(
            "anthropic",
            "claude-sonnet-4.6",
            Some("sk-test-key"),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(config.provider, LLMProvider::Anthropic);
        assert_eq!(config.model, "claude-sonnet-4.6");
        assert_eq!(config.api_key.as_deref(), Some("sk-test-key"));
        assert_eq!(config.base_url, "https://api.anthropic.com");
        assert_eq!(config.auth_type, AuthType::ApiKey);
    }

    #[test]
    fn resolve_openai_with_base_url_override() {
        let config = LLMProviderConfig::resolve(
            "openai",
            "gpt-4o",
            None,
            None,
            Some("sk-openai-key"),
            Some("https://custom.api.com/v1"),
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(config.provider, LLMProvider::OpenAI);
        assert_eq!(config.base_url, "https://custom.api.com/v1");
        assert_eq!(config.auth_type, AuthType::Bearer);
    }

    #[test]
    fn resolve_ollama_defaults() {
        let config = LLMProviderConfig::resolve(
            "ollama",
            "llama3",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(config.provider, LLMProvider::Ollama);
        assert_eq!(config.base_url, "http://localhost:11434/v1");
        assert!(config.api_key.is_none());
    }

    #[test]
    fn resolve_openai_compatible_requires_base_url() {
        let result = LLMProviderConfig::resolve(
            "openai_compatible",
            "my-model",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("openai_compatible_base_url"));
    }

    #[test]
    fn resolve_openai_compatible_with_base_url() {
        let config = LLMProviderConfig::resolve(
            "openai_compatible",
            "my-model",
            None,
            None,
            None,
            None,
            None,
            Some("optional-key"),
            Some("http://localhost:8000/v1"),
        )
        .unwrap();
        assert_eq!(config.provider, LLMProvider::OpenAICompatible);
        assert_eq!(config.base_url, "http://localhost:8000/v1");
        assert_eq!(config.api_key.as_deref(), Some("optional-key"));
    }

    #[test]
    fn reject_unknown_provider() {
        let result = LLMProviderConfig::resolve(
            "unknown",
            "model",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown LLM provider"));
    }

    #[test]
    fn reject_empty_model() {
        let result = LLMProviderConfig::resolve(
            "anthropic",
            "",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("model is required"));
    }
}
```

- [ ] **Step 6: Create `anthropic_oauth.rs`**

Create `crates/graphify-extract/src/semantic/anthropic_oauth.rs`:

```rust
use std::path::PathBuf;
use tracing::debug;

/// Read OAuth access token from Claude Code's local credential storage.
///
/// Checks `~/.claude/credentials.json` for an OAuth token.
/// Returns `None` if the file doesn't exist or no token field is found.
pub fn read_claude_code_oauth_token() -> Option<String> {
    let home = dirs::home_dir()?;
    let cred_path = home.join(".claude").join("credentials.json");
    read_token_from_file(&cred_path)
}

fn read_token_from_file(path: &std::path::Path) -> Option<String> {
    if !path.exists() {
        debug!("Claude Code credentials not found at {}", path.display());
        return None;
    }

    let content = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;

    // Try common field names for OAuth access tokens
    for field in &["access_token", "oauthToken", "apiKey", "token"] {
        if let Some(val) = json.get(*field).and_then(|v| v.as_str()) {
            if !val.is_empty() {
                debug!("Found Claude Code OAuth token in field '{}'", field);
                return Some(val.to_string());
            }
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
        write!(f, r#"{{"access_token": ""}}"#).unwrap();

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
}
```

- [ ] **Step 7: Add `tempfile` dev-dependency to graphify-extract**

In `crates/graphify-extract/Cargo.toml`, add:

```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 8: Register sub-modules in `mod.rs`**

Add at the top of `crates/graphify-extract/src/semantic/mod.rs` (before the existing `use` statements):

```rust
pub mod anthropic_oauth;
pub mod provider;
```

- [ ] **Step 9: Verify all tests pass**

Run: `cargo test -p graphify-extract`
Expected: All tests pass (new provider tests + existing semantic tests)

- [ ] **Step 10: Commit**

```bash
git add crates/graphify-extract/
git commit -m "feat: add LLMProvider/LLMProviderConfig types and Claude Code OAuth reader"
```

---

### Task 2: Refactor — extract Anthropic path, add OpenAI-compatible path, wire dispatch

**Files:**
- Create: `crates/graphify-extract/src/semantic/anthropic.rs`
- Create: `crates/graphify-extract/src/semantic/openai_compat.rs`
- Modify: `crates/graphify-extract/src/semantic/mod.rs`

- [ ] **Step 1: Create `anthropic.rs`**

Create `crates/graphify-extract/src/semantic/anthropic.rs`:

```rust
use std::path::Path;

use anyhow::{Context, Result};
use graphify_core::model::ExtractionResult;
use serde::{Deserialize, Serialize};

use super::provider::{AuthType, LLMProviderConfig};

#[derive(Serialize)]
struct MessageRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<AnthropicMessage>,
    system: String,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct MessageResponse {
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
struct ContentBlock {
    text: Option<String>,
}

pub async fn extract_anthropic(
    path: &Path,
    content: &str,
    file_type: &str,
    config: &LLMProviderConfig,
) -> Result<ExtractionResult> {
    let file_str = path.to_string_lossy();
    let system_prompt = super::build_system_prompt(file_type);
    let user_prompt = super::build_user_prompt(content, file_type);

    let request_body = MessageRequest {
        model: config.model.clone(),
        max_tokens: 4096,
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: user_prompt,
        }],
        system: system_prompt,
    };

    let client = reqwest::Client::new();
    let mut request = client
        .post(format!("{}/v1/messages", config.base_url))
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&request_body);

    match config.auth_type {
        AuthType::ApiKey => {
            if let Some(ref key) = config.api_key {
                request = request.header("x-api-key", key);
            }
        }
        AuthType::Bearer => {
            if let Some(ref token) = config.api_key {
                request = request.header("authorization", format!("Bearer {}", token));
            }
        }
    }

    let response = request
        .send()
        .await
        .context("failed to send request to Anthropic API")?;

    if response.status().as_u16() == 401 {
        anyhow::bail!(
            "Anthropic API key invalid or OAuth token expired. \
             Run `claude login` to refresh, or set ANTHROPIC_API_KEY."
        );
    }

    if response.status().as_u16() == 400 || response.status().as_u16() == 404 {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!(
            "Model '{}' not found. Check available models at docs.anthropic.com\nAPI returned {}: {}",
            config.model, status, body
        );
    }

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Anthropic API returned {}: {}", status, body);
    }

    let msg: MessageResponse = response
        .json()
        .await
        .context("failed to parse Anthropic API response")?;

    let text = msg
        .content
        .first()
        .and_then(|b| b.text.as_deref())
        .unwrap_or("{}");

    super::parse_semantic_response(text, &file_str)
}
```

- [ ] **Step 2: Create `openai_compat.rs`**

Create `crates/graphify-extract/src/semantic/openai_compat.rs`:

```rust
use std::path::Path;

use anyhow::{Context, Result};
use graphify_core::model::ExtractionResult;
use serde::{Deserialize, Serialize};

use super::provider::LLMProviderConfig;

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<ChatMessage>,
}

#[derive(Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessageResponse,
}

#[derive(Deserialize)]
struct ChatMessageResponse {
    content: Option<String>,
}

pub async fn extract_openai_compatible(
    path: &Path,
    content: &str,
    file_type: &str,
    config: &LLMProviderConfig,
) -> Result<ExtractionResult> {
    let file_str = path.to_string_lossy();
    let system_prompt = super::build_system_prompt(file_type);
    let user_prompt = super::build_user_prompt(content, file_type);

    let request_body = ChatRequest {
        model: config.model.clone(),
        max_tokens: 4096,
        messages: vec![
            ChatMessage {
                role: "system".to_string(),
                content: system_prompt,
            },
            ChatMessage {
                role: "user".to_string(),
                content: user_prompt,
            },
        ],
    };

    let client = reqwest::Client::new();
    let mut request = client
        .post(format!("{}/chat/completions", config.base_url))
        .header("content-type", "application/json")
        .json(&request_body);

    if let Some(ref key) = config.api_key {
        request = request.header("authorization", format!("Bearer {}", key));
    }

    let response = request.send().await.with_context(|| {
        format!(
            "Cannot connect to {}. Make sure the server is running.",
            config.base_url
        )
    })?;

    if response.status().as_u16() == 401 {
        match config.provider {
            super::provider::LLMProvider::OpenAI => {
                anyhow::bail!(
                    "OpenAI API key invalid. Set OPENAI_API_KEY or configure in graphify.toml."
                );
            }
            _ => {
                anyhow::bail!(
                    "Authentication failed for {}. Check your API key in graphify.toml.",
                    config.base_url
                );
            }
        }
    }

    if response.status().as_u16() == 404 {
        match config.provider {
            super::provider::LLMProvider::Ollama => {
                anyhow::bail!(
                    "Model '{}' not found. Run: ollama pull {}",
                    config.model,
                    config.model
                );
            }
            super::provider::LLMProvider::OpenAI => {
                anyhow::bail!(
                    "Model '{}' not found. Check available models at platform.openai.com",
                    config.model
                );
            }
            _ => {
                anyhow::bail!(
                    "Model '{}' not found at {}. Check that the model is available.",
                    config.model,
                    config.base_url
                );
            }
        }
    }

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("LLM API at {} returned {}: {}", config.base_url, status, body);
    }

    let chat_resp: ChatResponse = response
        .json()
        .await
        .context("failed to parse LLM API response")?;

    let text = chat_resp
        .choices
        .first()
        .and_then(|c| c.message.content.as_deref())
        .unwrap_or("{}");

    super::parse_semantic_response(text, &file_str)
}
```

- [ ] **Step 3: Refactor `mod.rs` — replace old `extract_semantic` with dispatch, keep shared code**

Replace `crates/graphify-extract/src/semantic/mod.rs` entirely with:

```rust
//! Semantic extraction via LLM APIs (Pass 2).
//!
//! Supports multiple LLM providers through a dual-path architecture:
//! - Anthropic (Messages API + OAuth token support)
//! - OpenAI-compatible (Chat Completions API: OpenAI, Ollama, vLLM, etc.)

pub mod anthropic;
pub mod anthropic_oauth;
pub mod openai_compat;
pub mod provider;

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use graphify_core::confidence::Confidence;
use graphify_core::id::make_id;
use graphify_core::model::{ExtractionResult, GraphEdge, GraphNode, NodeType};
use serde::Deserialize;
use tracing::debug;

pub use provider::{AuthType, LLMProvider, LLMProviderConfig};

// ---------------------------------------------------------------------------
// Shared response types
// ---------------------------------------------------------------------------

/// Entities and relationships extracted by the LLM.
#[derive(Deserialize, Debug)]
struct SemanticOutput {
    #[serde(default)]
    entities: Vec<SemanticEntity>,
    #[serde(default)]
    relationships: Vec<SemanticRelation>,
}

#[derive(Deserialize, Debug)]
struct SemanticEntity {
    name: String,
    #[serde(default = "default_entity_type")]
    entity_type: String,
}

fn default_entity_type() -> String {
    "concept".to_string()
}

#[derive(Deserialize, Debug)]
struct SemanticRelation {
    source: String,
    target: String,
    #[serde(default = "default_relation")]
    relation: String,
}

fn default_relation() -> String {
    "related_to".to_string()
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Extract semantic concepts from a document, paper, or image using an LLM.
///
/// Dispatches to the appropriate provider based on `config.provider`.
pub async fn extract_semantic(
    path: &Path,
    content: &str,
    file_type: &str,
    config: &LLMProviderConfig,
) -> Result<ExtractionResult> {
    match config.provider {
        LLMProvider::Anthropic => {
            anthropic::extract_anthropic(path, content, file_type, config).await
        }
        LLMProvider::OpenAI | LLMProvider::Ollama | LLMProvider::OpenAICompatible => {
            openai_compat::extract_openai_compatible(path, content, file_type, config).await
        }
    }
}

// ---------------------------------------------------------------------------
// Prompt construction (shared)
// ---------------------------------------------------------------------------

fn build_system_prompt(file_type: &str) -> String {
    format!(
        "You are an expert knowledge-graph extraction engine. \
         Given a {file_type}, extract entities and their relationships. \
         Respond ONLY with a JSON object having two arrays: \
         \"entities\" (each with \"name\" and \"entity_type\") and \
         \"relationships\" (each with \"source\", \"target\", and \"relation\"). \
         Entity types should be one of: concept, class, function, module, paper, image. \
         Keep entity names concise and unique."
    )
}

fn build_user_prompt(content: &str, file_type: &str) -> String {
    let max_chars = 100_000;
    let truncated = if content.len() > max_chars {
        let mut end = max_chars;
        while end > 0 && !content.is_char_boundary(end) {
            end -= 1;
        }
        &content[..end]
    } else {
        content
    };

    format!("Extract all entities and relationships from this {file_type}:\n\n{truncated}")
}

// ---------------------------------------------------------------------------
// Response parsing (shared)
// ---------------------------------------------------------------------------

fn parse_semantic_response(text: &str, file_str: &str) -> Result<ExtractionResult> {
    let json_str = extract_json_block(text);

    let output: SemanticOutput =
        serde_json::from_str(json_str).context("failed to parse semantic extraction JSON")?;

    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    let mut name_to_id: HashMap<String, String> = HashMap::new();
    for entity in &output.entities {
        let id = make_id(&[file_str, &entity.name]);
        let node_type = match entity.entity_type.as_str() {
            "class" => NodeType::Class,
            "function" => NodeType::Function,
            "module" => NodeType::Module,
            "paper" => NodeType::Paper,
            "image" => NodeType::Image,
            _ => NodeType::Concept,
        };
        name_to_id.insert(entity.name.clone(), id.clone());
        nodes.push(GraphNode {
            id,
            label: entity.name.clone(),
            source_file: file_str.to_string(),
            source_location: None,
            node_type,
            community: None,
            extra: HashMap::new(),
        });
    }

    for rel in &output.relationships {
        let source_id = name_to_id
            .get(&rel.source)
            .cloned()
            .unwrap_or_else(|| make_id(&[file_str, &rel.source]));
        let target_id = name_to_id
            .get(&rel.target)
            .cloned()
            .unwrap_or_else(|| make_id(&[file_str, &rel.target]));

        edges.push(GraphEdge {
            source: source_id,
            target: target_id,
            relation: rel.relation.clone(),
            confidence: Confidence::Inferred,
            confidence_score: Confidence::Inferred.default_score(),
            source_file: file_str.to_string(),
            source_location: None,
            weight: 1.0,
            extra: HashMap::new(),
        });
    }

    Ok(ExtractionResult {
        nodes,
        edges,
        hyperedges: Vec::new(),
    })
}

/// Extract a JSON block from text that might be wrapped in markdown fences.
fn extract_json_block(text: &str) -> &str {
    if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        if let Some(end) = after.find("```") {
            return after[..end].trim();
        }
    }
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        if let Some(end) = after.find("```") {
            return after[..end].trim();
        }
    }
    if let Some(start) = text.find('{')
        && let Some(end) = text.rfind('}')
    {
        return &text[start..=end];
    }
    text.trim()
}

// ---------------------------------------------------------------------------
// Tests (shared parsing logic)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_semantic_json() {
        let json = r#"{
            "entities": [
                {"name": "Machine Learning", "entity_type": "concept"},
                {"name": "Neural Network", "entity_type": "concept"},
                {"name": "Backpropagation", "entity_type": "concept"}
            ],
            "relationships": [
                {"source": "Neural Network", "target": "Machine Learning", "relation": "is_a"},
                {"source": "Backpropagation", "target": "Neural Network", "relation": "used_by"}
            ]
        }"#;

        let result = parse_semantic_response(json, "paper.pdf").unwrap();
        assert_eq!(result.nodes.len(), 3);
        assert_eq!(result.edges.len(), 2);
        assert!(result.nodes.iter().all(|n| n.node_type == NodeType::Concept));
        assert_eq!(result.edges[0].relation, "is_a");
    }

    #[test]
    fn parse_markdown_wrapped_json() {
        let text = r#"Here is the extraction:
```json
{
    "entities": [{"name": "Foo", "entity_type": "class"}],
    "relationships": []
}
```
"#;
        let result = parse_semantic_response(text, "doc.md").unwrap();
        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.nodes[0].label, "Foo");
        assert_eq!(result.nodes[0].node_type, NodeType::Class);
    }

    #[test]
    fn parse_empty_response() {
        let json = r#"{"entities": [], "relationships": []}"#;
        let result = parse_semantic_response(json, "empty.txt").unwrap();
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn extract_json_block_plain() {
        assert_eq!(extract_json_block(r#"{"a": 1}"#), r#"{"a": 1}"#);
    }

    #[test]
    fn extract_json_block_fenced() {
        let text = "blah\n```json\n{\"a\": 1}\n```\nmore";
        assert_eq!(extract_json_block(text), r#"{"a": 1}"#);
    }

    #[test]
    fn semantic_edges_are_inferred_confidence() {
        let json = r#"{
            "entities": [
                {"name": "A", "entity_type": "concept"},
                {"name": "B", "entity_type": "concept"}
            ],
            "relationships": [
                {"source": "A", "target": "B", "relation": "depends_on"}
            ]
        }"#;
        let result = parse_semantic_response(json, "test.md").unwrap();
        assert_eq!(result.edges[0].confidence, Confidence::Inferred);
    }

    #[test]
    fn build_prompts_contain_file_type() {
        let sys = build_system_prompt("paper");
        assert!(sys.contains("paper"));

        let user = build_user_prompt("hello world", "document");
        assert!(user.contains("document"));
        assert!(user.contains("hello world"));
    }
}
```

Note: The `use anyhow::Context` import is needed because `parse_semantic_response` uses `.context()`. It's imported at the top of the module. Actually looking again, the function returns `Result<ExtractionResult>` from anyhow, and uses `.context()`. The import `use anyhow::Result` is already there, but we also need `use anyhow::Context` for the `.context()` method. Let me add that to the imports.

Wait, I see I already have `use anyhow::Result;` but not `use anyhow::Context;`. The `.context()` method is from the `Context` trait. Let me add it. Actually, in the original `semantic.rs`, the `Context` trait was imported via `use anyhow::{Context, Result};`. I need to include it in the new mod.rs too.

Let me fix the imports in the mod.rs code above. Add `Context` to the anyhow import:

```rust
use anyhow::{Context, Result};
```

- [ ] **Step 4: Verify all tests pass**

Run: `cargo test -p graphify-extract`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/graphify-extract/
git commit -m "feat: dual-path LLM extraction — Anthropic + OpenAI-compatible"
```

---

### Task 3: Update config.rs

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Write the failing test for LLM config parsing**

Add to `src/config.rs` test module:

```rust
#[test]
fn test_parse_llm_config() {
    let toml_str = r#"
[llm]
provider = "ollama"
model = "llama3"
ollama_base_url = "http://localhost:11434"
"#;
    let cfg: Config = toml::from_str(toml_str).unwrap();
    let llm = cfg.llm.as_ref().expect("llm config should be present");
    assert_eq!(llm.provider.as_deref(), Some("ollama"));
    assert_eq!(llm.model.as_deref(), Some("llama3"));
    assert_eq!(llm.ollama_base_url.as_deref(), Some("http://localhost:11434"));
}

#[test]
fn test_parse_llm_config_anthropic() {
    let toml_str = r#"
[llm]
provider = "anthropic"
model = "claude-sonnet-4.6"
anthropic_api_key = "sk-test"
"#;
    let cfg: Config = toml_str.parse().unwrap();
    let llm = cfg.llm.as_ref().expect("llm config should be present");
    assert_eq!(llm.provider.as_deref(), Some("anthropic"));
    assert_eq!(llm.anthropic_api_key.as_deref(), Some("sk-test"));
}

#[test]
fn test_config_without_llm() {
    let toml_str = r#"
output = "my-output"
"#;
    let cfg: Config = toml::from_str(toml_str).unwrap();
    assert!(cfg.llm.is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p graphify-rs -- config`
Expected: FAIL — `LLMConfig` type doesn't exist yet

- [ ] **Step 3: Add LLMConfig struct to Config**

Update `src/config.rs` to:

```rust
use serde::Deserialize;
use std::path::Path;

/// Configuration loaded from `graphify.toml`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub output: Option<String>,
    pub no_llm: Option<bool>,
    pub code_only: Option<bool>,
    pub formats: Option<Vec<String>>,
    pub llm: Option<LLMConfig>,
}

/// LLM provider configuration from `[llm]` section.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct LLMConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub anthropic_api_key: Option<String>,
    pub anthropic_base_url: Option<String>,
    pub openai_api_key: Option<String>,
    pub openai_base_url: Option<String>,
    pub ollama_base_url: Option<String>,
    pub openai_compatible_api_key: Option<String>,
    pub openai_compatible_base_url: Option<String>,
}

/// Load configuration from `graphify.toml` in the given directory.
/// Returns default config if file doesn't exist or can't be parsed.
pub fn load_config(root: &Path) -> Config {
    let config_path = root.join("graphify.toml");
    if !config_path.exists() {
        return Config::default();
    }
    match std::fs::read_to_string(&config_path) {
        Ok(content) => toml::from_str(&content).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = Config::default();
        assert!(cfg.output.is_none());
        assert!(cfg.no_llm.is_none());
        assert!(cfg.code_only.is_none());
        assert!(cfg.formats.is_none());
        assert!(cfg.llm.is_none());
    }

    #[test]
    fn test_load_missing_config() {
        let cfg = load_config(Path::new("/nonexistent"));
        assert!(cfg.output.is_none());
    }

    #[test]
    fn test_parse_config() {
        let toml_str = r#"
output = "my-output"
no_llm = true
formats = ["json", "html"]
"#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.output.as_deref(), Some("my-output"));
        assert_eq!(cfg.no_llm, Some(true));
        assert_eq!(
            cfg.formats.as_deref(),
            Some(&["json".to_string(), "html".to_string()][..])
        );
    }

    #[test]
    fn test_parse_llm_config() {
        let toml_str = r#"
[llm]
provider = "ollama"
model = "llama3"
ollama_base_url = "http://localhost:11434"
"#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        let llm = cfg.llm.as_ref().expect("llm config should be present");
        assert_eq!(llm.provider.as_deref(), Some("ollama"));
        assert_eq!(llm.model.as_deref(), Some("llama3"));
        assert_eq!(llm.ollama_base_url.as_deref(), Some("http://localhost:11434"));
    }

    #[test]
    fn test_parse_llm_config_anthropic() {
        let toml_str = r#"
[llm]
provider = "anthropic"
model = "claude-sonnet-4.6"
anthropic_api_key = "sk-test"
"#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        let llm = cfg.llm.as_ref().expect("llm config should be present");
        assert_eq!(llm.provider.as_deref(), Some("anthropic"));
        assert_eq!(llm.anthropic_api_key.as_deref(), Some("sk-test"));
    }

    #[test]
    fn test_config_without_llm() {
        let toml_str = r#"
output = "my-output"
"#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert!(cfg.llm.is_none());
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p graphify-rs -- config`
Expected: All config tests pass

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat: add LLMConfig to graphify.toml config"
```

---

### Task 4: Update main.rs — wire LLM config into build pipeline

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Replace the semantic extraction section in `cmd_build`**

Replace lines 626–749 of `src/main.rs` (the `if !no_llm && !code_only` block) with:

```rust
    // ── Step 2b: Semantic extraction (Pass 2 — LLM API, concurrent) ──
    if !no_llm && !code_only {
        let provider_config = if let Some(ref llm) = cfg.llm {
            // Config-based resolution
            let provider = llm.provider.as_deref().unwrap_or("");
            let model = llm.model.as_deref().unwrap_or("");
            match graphify_extract::semantic::LLMProviderConfig::resolve(
                provider,
                model,
                llm.anthropic_api_key.as_deref(),
                llm.anthropic_base_url.as_deref(),
                llm.openai_api_key.as_deref(),
                llm.openai_base_url.as_deref(),
                llm.ollama_base_url.as_deref(),
                llm.openai_compatible_api_key.as_deref(),
                llm.openai_compatible_base_url.as_deref(),
            ) {
                Ok(c) => Some(c),
                Err(e) => {
                    info_print!(
                        verb,
                        "  {} Invalid [llm] config: {}",
                        "⚠".yellow(),
                        e
                    );
                    None
                }
            }
        } else {
            // Backward compat: ANTHROPIC_API_KEY env var → Anthropic provider
            std::env::var("ANTHROPIC_API_KEY").ok().map(|key| {
                graphify_extract::semantic::LLMProviderConfig {
                    provider: graphify_extract::semantic::LLMProvider::Anthropic,
                    model: "claude-sonnet-4.6".to_string(),
                    api_key: Some(key),
                    base_url: "https://api.anthropic.com".to_string(),
                    auth_type: graphify_extract::semantic::AuthType::ApiKey,
                }
            })
        };

        if let Some(ref llm_config) = provider_config {
            let doc_files: Vec<PathBuf> = detection
                .files
                .get(&graphify_detect::FileType::Document)
                .into_iter()
                .chain(detection.files.get(&graphify_detect::FileType::Paper))
                .flat_map(|v| v.iter().map(|f| root.join(f)))
                .collect();

            if !doc_files.is_empty() {
                info_print!(
                    verb,
                    "  {} on {} doc/paper files via {} ({})...",
                    "Semantic extraction".cyan(),
                    doc_files.len(),
                    match llm_config.provider {
                        graphify_extract::semantic::LLMProvider::Anthropic => "Anthropic",
                        graphify_extract::semantic::LLMProvider::OpenAI => "OpenAI",
                        graphify_extract::semantic::LLMProvider::Ollama => "Ollama",
                        graphify_extract::semantic::LLMProvider::OpenAICompatible => "OpenAI-compatible",
                    },
                    llm_config.model,
                );
                let concurrency = jobs.unwrap_or(4).min(8);
                let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency));
                let rt = tokio::runtime::Handle::current();

                let pb_sem = if !verb.is_quiet() {
                    let pb = ProgressBar::new(doc_files.len() as u64);
                    pb.set_style(
                        ProgressStyle::with_template(
                            "  {bar:40.green/dim} {pos}/{len} docs ({eta} remaining)",
                        )
                        .unwrap()
                        .progress_chars("██░"),
                    );
                    Some(pb)
                } else {
                    None
                };

                let mut handles = Vec::new();
                for doc_path in &doc_files {
                    if let Some(cached) = graphify_cache::load_cached_from::<
                        graphify_core::model::ExtractionResult,
                    >(doc_path, &root, &cache_dir)
                    {
                        extractions.push(cached);
                        if let Some(ref pb) = pb_sem {
                            pb.inc(1);
                        }
                        continue;
                    }
                    let content = match std::fs::read_to_string(doc_path) {
                        Ok(c) => c,
                        Err(_) => {
                            if let Some(ref pb) = pb_sem {
                                pb.inc(1);
                            }
                            continue;
                        }
                    };
                    let file_type = if doc_path.extension().and_then(|e| e.to_str()) == Some("pdf")
                    {
                        "paper"
                    } else {
                        "document"
                    };
                    let doc_p = doc_path.clone();
                    let cfg_clone = llm_config.clone();
                    let sem_clone = sem.clone();
                    let handle = rt.spawn(async move {
                        let _permit = sem_clone
                            .acquire()
                            .await
                            .map_err(|e| anyhow::anyhow!("semaphore closed: {e}"))?;
                        graphify_extract::semantic::extract_semantic(
                            &doc_p, &content, file_type, &cfg_clone,
                        )
                        .await
                        .map(|r| (doc_p, r))
                    });
                    handles.push(handle);
                }

                for handle in handles {
                    match handle.await {
                        Ok(Ok((doc_p, sem_result))) => {
                            verbose_print!(
                                verb,
                                "    {} → {} nodes, {} edges",
                                doc_p.file_name().unwrap_or_default().to_string_lossy(),
                                sem_result.nodes.len(),
                                sem_result.edges.len()
                            );
                            let _ = graphify_cache::save_cached_to(
                                &doc_p,
                                &sem_result,
                                &root,
                                &cache_dir,
                            );
                            extractions.push(sem_result);
                        }
                        Ok(Err(e)) => {
                            verbose_print!(verb, "    {} semantic extraction: {}", "⚠".yellow(), e);
                        }
                        Err(e) => {
                            verbose_print!(verb, "    {} task join error: {}", "⚠".yellow(), e);
                        }
                    }
                    if let Some(ref pb) = pb_sem {
                        pb.inc(1);
                    }
                }
                if let Some(pb) = pb_sem {
                    pb.finish_and_clear();
                }
            }
        } else if n_doc + n_paper > 0 {
            info_print!(
                verb,
                "  {} Configure [llm] in graphify.toml to enable semantic extraction for {} doc/paper files",
                "ℹ".blue(),
                n_doc + n_paper
            );
        }
    }
```

- [ ] **Step 2: Verify compilation**

Run: `cargo build`
Expected: Builds successfully

- [ ] **Step 3: Verify all tests pass**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire LLM provider config into build pipeline"
```

---

### Task 5: Update cmd_init template

**Files:**
- Modify: `src/main.rs` (cmd_init function)

- [ ] **Step 1: Update the init template to include `[llm]` section**

In `src/main.rs`, replace the `cmd_init` function's template string with:

```rust
fn cmd_init() -> Result<()> {
    let path = Path::new("graphify.toml");
    if path.exists() {
        anyhow::bail!("graphify.toml already exists");
    }
    std::fs::write(
        path,
        r#"# graphify-rs configuration
# These values serve as defaults and can be overridden by CLI flags.

# Output directory for graph files
# output = "graphify-out"

# Disable LLM-based semantic extraction
# no_llm = false

# Only process code files (skip docs/papers)
# code_only = false

# Export formats (comma-separated). Available: json,html,graphml,cypher,svg,wiki,obsidian,report
# Leave empty or omit for all formats.
# formats = ["json", "html", "report"]

# LLM provider for semantic extraction
# [llm]
# provider = "anthropic"          # anthropic | openai | ollama | openai_compatible
# model = "claude-sonnet-4.6"  # required, no default
# anthropic_api_key = "sk-..."    # optional, falls back to ANTHROPIC_API_KEY env or Claude Code OAuth
# anthropic_base_url = "https://api.anthropic.com"  # optional override
# openai_api_key = "sk-..."       # optional, falls back to OPENAI_API_KEY env
# openai_base_url = "https://api.openai.com/v1"     # optional override
# ollama_base_url = "http://localhost:11434"          # optional override
# openai_compatible_api_key = "..."                   # optional
# openai_compatible_base_url = "http://localhost:8000/v1"  # required for openai_compatible
"#,
    )?;
    println!("{} Created graphify.toml", "✓".green());
    Ok(())
}
```

- [ ] **Step 2: Verify build**

Run: `cargo build`
Expected: Builds successfully

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "feat: add [llm] section to graphify.toml init template"
```

---

### Task 6: Final verification

- [ ] **Step 1: Run full test suite**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: No warnings

- [ ] **Step 3: Verify backward compatibility — ANTHROPIC_API_KEY still works without config**

Run: `cargo run -p graphify-rs -- build --no_llm .`
Expected: Build succeeds without LLM (no config, no env var set)

- [ ] **Step 4: Verify init template includes [llm] section**

Run: `cd /tmp && cargo run -p graphify-rs -- init` (or run in a temp dir)
Expected: graphify.toml created with commented [llm] section

- [ ] **Step 5: Final commit if any fixes needed**

```bash
git add -A
git commit -m "fix: address clippy warnings and test issues"
```
