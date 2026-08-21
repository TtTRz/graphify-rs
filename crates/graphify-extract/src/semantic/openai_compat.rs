use std::path::Path;

use anyhow::{Context, Result};
use graphify_core::model::ExtractionResult;
use serde::{Deserialize, Serialize};

use super::provider::LLMProvider;

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u32>,
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
    reasoning_content: Option<String>,
}

pub fn normalize_chat_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/chat/completions")
    }
}

pub async fn extract_openai_compatible(
    path: &Path,
    content: &str,
    file_type: &str,
    _provider: LLMProvider,
    model: &str,
    api_key: Option<&str>,
    base_url: &str,
) -> Result<ExtractionResult> {
    let file_str = path.to_string_lossy();
    let system_prompt = super::build_system_prompt(file_type);
    let user_prompt = super::build_user_prompt(content, file_type);

    let is_o_series = model.starts_with("o1") || model.starts_with("o3");
    let (max_tokens, max_completion_tokens) = if is_o_series {
        (None, Some(8192))
    } else {
        (Some(8192), None)
    };

    let request_body = ChatRequest {
        model: model.to_string(),
        max_tokens,
        max_completion_tokens,
        messages: vec![
            ChatMessage {
                role: if is_o_series { "developer".to_string() } else { "system".to_string() },
                content: system_prompt,
            },
            ChatMessage {
                role: "user".to_string(),
                content: user_prompt,
            },
        ],
    };

    let endpoint = normalize_chat_endpoint(base_url);
    let client = reqwest::Client::new();
    let mut request = client
        .post(&endpoint)
        .header("content-type", "application/json")
        .json(&request_body);

    if let Some(key) = api_key {
        request = request.header("authorization", format!("Bearer {key}"));
    }

    let response = request.send().await.with_context(|| {
        format!("Cannot connect to {endpoint}. Make sure the server is running.")
    })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("LLM API at {endpoint} returned HTTP {status}: {body}");
    }

    let chat_resp: ChatResponse = response
        .json()
        .await
        .context("failed to parse LLM API response")?;

    let text = chat_resp
        .choices
        .first()
        .and_then(|c| {
            c.message
                .content
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .or(c.message.reasoning_content.as_deref())
        })
        .unwrap_or("{}");

    super::parse_semantic_response(text, &file_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_chat_endpoint() {
        assert_eq!(
            normalize_chat_endpoint("https://api.kimi.com/coding/v1"),
            "https://api.kimi.com/coding/v1/chat/completions"
        );
        assert_eq!(
            normalize_chat_endpoint("https://api.kimi.com/coding/v1/"),
            "https://api.kimi.com/coding/v1/chat/completions"
        );
        assert_eq!(
            normalize_chat_endpoint("https://api.kimi.com/coding/v1/chat/completions"),
            "https://api.kimi.com/coding/v1/chat/completions"
        );
        assert_eq!(
            normalize_chat_endpoint("https://api.kimi.com/coding/v1/chat/completions/"),
            "https://api.kimi.com/coding/v1/chat/completions"
        );
    }
}
