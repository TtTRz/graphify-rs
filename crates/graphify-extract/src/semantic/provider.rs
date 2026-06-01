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

/// Raw LLM configuration from `graphify.toml`'s `[llm]` section.
/// Used to avoid a 9-parameter resolve() signature.
#[derive(Debug, Default)]
pub struct LLMConfigRaw {
    pub provider: String,
    pub model: String,
    pub anthropic_api_key: Option<String>,
    pub anthropic_base_url: Option<String>,
    pub openai_api_key: Option<String>,
    pub openai_base_url: Option<String>,
    pub ollama_base_url: Option<String>,
    pub openai_compatible_api_key: Option<String>,
    pub openai_compatible_base_url: Option<String>,
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

    pub fn resolve(raw: &LLMConfigRaw) -> Result<Self> {
        let provider = match raw.provider.as_str() {
            "anthropic" => LLMProvider::Anthropic,
            "openai" => LLMProvider::OpenAI,
            "ollama" => LLMProvider::Ollama,
            "openai_compatible" => LLMProvider::OpenAICompatible,
            other => anyhow::bail!(
                "Unknown LLM provider: '{other}'. Supported: anthropic, openai, ollama, openai_compatible"
            ),
        };

        if raw.model.is_empty() {
            anyhow::bail!("LLM model is required in [llm] config");
        }

        let (api_key, base_url, auth_type) = match provider {
            LLMProvider::Anthropic => {
                let (key, at) = if let Some(ref k) = raw.anthropic_api_key {
                    (Some(k.clone()), AuthType::ApiKey)
                } else if let Ok(k) = std::env::var("ANTHROPIC_API_KEY") {
                    (Some(k), AuthType::ApiKey)
                } else if let Some(token) = super::anthropic_oauth::read_claude_code_oauth_token() {
                    (Some(token), AuthType::Bearer)
                } else {
                    (None, AuthType::ApiKey)
                };
                let url = raw
                    .anthropic_base_url
                    .clone()
                    .unwrap_or_else(|| Self::ANTHROPIC_DEFAULT_URL.to_string());
                (key, url, at)
            }
            LLMProvider::OpenAI => {
                let key = raw
                    .openai_api_key
                    .clone()
                    .or_else(|| std::env::var("OPENAI_API_KEY").ok());
                let url = raw
                    .openai_base_url
                    .clone()
                    .unwrap_or_else(|| Self::OPENAI_DEFAULT_URL.to_string());
                (key, url, AuthType::Bearer)
            }
            LLMProvider::Ollama => {
                let url = raw
                    .ollama_base_url
                    .clone()
                    .unwrap_or_else(|| Self::OLLAMA_DEFAULT_URL.to_string());
                (None, url, AuthType::Bearer)
            }
            LLMProvider::OpenAICompatible => {
                let key = raw.openai_compatible_api_key.clone();
                let url = raw.openai_compatible_base_url.clone().context(
                    "openai_compatible_base_url is required for openai_compatible provider",
                )?;
                (key, url, AuthType::Bearer)
            }
        };

        Ok(LLMProviderConfig {
            provider,
            model: raw.model.clone(),
            api_key,
            base_url,
            auth_type,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(provider: &str, model: &str) -> LLMConfigRaw {
        LLMConfigRaw {
            provider: provider.to_string(),
            model: model.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn resolve_anthropic_with_api_key() {
        let r = LLMConfigRaw {
            provider: "anthropic".into(),
            model: "claude-sonnet-4.6".into(),
            anthropic_api_key: Some("sk-test-key".into()),
            ..Default::default()
        };
        let config = LLMProviderConfig::resolve(&r).unwrap();
        assert_eq!(config.provider, LLMProvider::Anthropic);
        assert_eq!(config.model, "claude-sonnet-4.6");
        assert_eq!(config.api_key.as_deref(), Some("sk-test-key"));
        assert_eq!(config.base_url, "https://api.anthropic.com");
        assert_eq!(config.auth_type, AuthType::ApiKey);
    }

    #[test]
    fn resolve_openai_with_base_url_override() {
        let r = LLMConfigRaw {
            provider: "openai".into(),
            model: "gpt-4o".into(),
            openai_api_key: Some("sk-openai-key".into()),
            openai_base_url: Some("https://custom.api.com/v1".into()),
            ..Default::default()
        };
        let config = LLMProviderConfig::resolve(&r).unwrap();
        assert_eq!(config.provider, LLMProvider::OpenAI);
        assert_eq!(config.base_url, "https://custom.api.com/v1");
        assert_eq!(config.auth_type, AuthType::Bearer);
    }

    #[test]
    fn resolve_ollama_defaults() {
        let config = LLMProviderConfig::resolve(&raw("ollama", "llama3")).unwrap();
        assert_eq!(config.provider, LLMProvider::Ollama);
        assert_eq!(config.base_url, "http://localhost:11434/v1");
        assert!(config.api_key.is_none());
    }

    #[test]
    fn resolve_openai_compatible_requires_base_url() {
        let result = LLMProviderConfig::resolve(&raw("openai_compatible", "my-model"));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("openai_compatible_base_url")
        );
    }

    #[test]
    fn resolve_openai_compatible_with_base_url() {
        let r = LLMConfigRaw {
            provider: "openai_compatible".into(),
            model: "my-model".into(),
            openai_compatible_api_key: Some("optional-key".into()),
            openai_compatible_base_url: Some("http://localhost:8000/v1".into()),
            ..Default::default()
        };
        let config = LLMProviderConfig::resolve(&r).unwrap();
        assert_eq!(config.provider, LLMProvider::OpenAICompatible);
        assert_eq!(config.base_url, "http://localhost:8000/v1");
        assert_eq!(config.api_key.as_deref(), Some("optional-key"));
    }

    #[test]
    fn reject_unknown_provider() {
        let result = LLMProviderConfig::resolve(&raw("unknown", "model"));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unknown LLM provider")
        );
    }

    #[test]
    fn reject_empty_model() {
        let result = LLMProviderConfig::resolve(&raw("anthropic", ""));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("model is required")
        );
    }
}
