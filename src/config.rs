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
        assert_eq!(
            llm.ollama_base_url.as_deref(),
            Some("http://localhost:11434")
        );
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
