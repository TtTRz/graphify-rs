use std::path::Path;

use anyhow::{Context, Result};
use graphify_core::model::ExtractionResult;
use serde::{Deserialize, Serialize};

use super::provider::LLMProvider;

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
    provider: LLMProvider,
    model: &str,
    api_key: Option<&str>,
    base_url: &str,
) -> Result<ExtractionResult> {
    let file_str = path.to_string_lossy();
    let system_prompt = super::build_system_prompt(file_type);
    let user_prompt = super::build_user_prompt(content, file_type);

    let request_body = ChatRequest {
        model: model.to_string(),
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
        .post(format!("{base_url}/chat/completions"))
        .header("content-type", "application/json")
        .json(&request_body);

    if let Some(key) = api_key {
        request = request.header("authorization", format!("Bearer {key}"));
    }

    let response = request.send().await.with_context(|| {
        format!("Cannot connect to {base_url}. Make sure the server is running.")
    })?;

    if response.status().as_u16() == 401 {
        match provider {
            LLMProvider::OpenAI => {
                anyhow::bail!(
                    "OpenAI API key invalid. Set OPENAI_API_KEY or configure in graphify.toml."
                );
            }
            _ => {
                anyhow::bail!(
                    "Authentication failed for {base_url}. Check your API key in graphify.toml."
                );
            }
        }
    }

    if response.status().as_u16() == 404 {
        match provider {
            LLMProvider::Ollama => {
                anyhow::bail!("Model '{model}' not found. Run: ollama pull {model}");
            }
            LLMProvider::OpenAI => {
                anyhow::bail!(
                    "Model '{model}' not found. Check available models at platform.openai.com"
                );
            }
            _ => {
                anyhow::bail!(
                    "Model '{model}' not found at {base_url}. Check that the model is available."
                );
            }
        }
    }

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("LLM API at {base_url} returned {status}: {body}");
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
