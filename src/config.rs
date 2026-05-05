//! Configuration file support for graphify-rs.
//!
//! Reads `graphify.toml` from the project root to provide defaults
//! that can be overridden by CLI flags.

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
    pub embed: Option<bool>,
    pub embedding_provider: Option<String>,
    pub embedding_model: Option<String>,
    pub anthropic_semantic: Option<bool>,
    pub llm: Option<bool>,
    pub llm_command: Option<String>,
    pub llm_provider: Option<String>,
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
        assert!(cfg.embed.is_none());
        assert!(cfg.embedding_provider.is_none());
        assert!(cfg.embedding_model.is_none());
        assert!(cfg.anthropic_semantic.is_none());
        assert!(cfg.llm.is_none());
        assert!(cfg.llm_command.is_none());
        assert!(cfg.llm_provider.is_none());
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
embed = true
embedding_provider = "model2vec"
embedding_model = "minishlab/potion-code-16M"
anthropic_semantic = false
llm = true
llm_command = "cat"
llm_provider = "test-cli"
"#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.output.as_deref(), Some("my-output"));
        assert_eq!(cfg.no_llm, Some(true));
        assert_eq!(
            cfg.formats.as_deref(),
            Some(&["json".to_string(), "html".to_string()][..])
        );
        assert_eq!(cfg.embed, Some(true));
        assert_eq!(cfg.embedding_provider.as_deref(), Some("model2vec"));
        assert_eq!(cfg.anthropic_semantic, Some(false));
        assert_eq!(cfg.llm, Some(true));
        assert_eq!(cfg.llm_command.as_deref(), Some("cat"));
        assert_eq!(cfg.llm_provider.as_deref(), Some("test-cli"));
        assert_eq!(
            cfg.embedding_model.as_deref(),
            Some("minishlab/potion-code-16M")
        );
    }
}
