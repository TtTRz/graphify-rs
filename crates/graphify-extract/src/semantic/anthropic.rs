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
            } else {
                anyhow::bail!(
                    "No API key configured for Anthropic. \
                     Set ANTHROPIC_API_KEY or configure [llm] in graphify.toml"
                );
            }
        }
        AuthType::Bearer => {
            if let Some(ref token) = config.api_key {
                request = request.header("authorization", format!("Bearer {token}"));
            } else {
                anyhow::bail!(
                    "No OAuth token configured for Anthropic. \
                     Run `claude login` or configure [llm] in graphify.toml"
                );
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
            config.model,
            status,
            body
        );
    }

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Anthropic API returned {status}: {body}");
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
