use std::path::Path;

use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    pub embeddings: Option<EmbeddingsConfig>,
    pub search: Option<SearchConfig>,
}

#[derive(Debug, Deserialize)]
pub struct EmbeddingsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub provider: String,
    pub model: String,
    pub endpoint: String,
    pub api_key: Option<String>,
    pub dimensions: usize,
    pub batch_size: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct SearchConfig {
    #[serde(default = "default_hybrid_alpha")]
    pub hybrid_alpha: f64,
    #[serde(default = "default_limit")]
    pub default_limit: usize,
}

const fn default_true() -> bool {
    true
}

const fn default_hybrid_alpha() -> f64 {
    0.4
}

const fn default_limit() -> usize {
    20
}

impl Config {
    pub fn load(root: &Path) -> Result<Self> {
        let path = crate::paths::cache_config_path(root)?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        Ok(toml::from_str(&content)?)
    }

    pub fn hybrid_alpha(&self) -> f64 {
        self.search
            .as_ref()
            .map(|s| s.hybrid_alpha)
            .unwrap_or(default_hybrid_alpha())
    }

    pub fn default_limit(&self) -> usize {
        self.search
            .as_ref()
            .map(|s| s.default_limit)
            .unwrap_or(default_limit())
    }

    pub fn generate_template(detected_embeddings: Option<&EmbeddingsConfig>) -> String {
        let mut output = String::new();
        output.push_str("# Token Stunt Configuration\n");
        output.push_str("# https://github.com/teobouancheau/tokenstunt\n\n");

        output.push_str("[search]\n");
        output.push_str("# hybrid_alpha = 0.4      # 0.0 = pure BM25, 1.0 = pure semantic\n");
        output.push_str("# default_limit = 20\n\n");

        output.push_str("[embeddings]\n");
        if let Some(emb) = detected_embeddings {
            output.push_str("enabled = true\n");
            output.push_str(&format!("provider = \"{}\"\n", emb.provider));
            output.push_str(&format!("endpoint = \"{}\"\n", emb.endpoint));
            output.push_str(&format!("model = \"{}\"\n", emb.model));
            output.push_str(&format!("dimensions = {}\n", emb.dimensions));
            if let Some(bs) = emb.batch_size {
                output.push_str(&format!("batch_size = {bs}\n"));
            } else {
                output.push_str("# batch_size = 32\n");
            }
        } else {
            output.push_str("# enabled = true\n");
            output.push_str("# provider = \"ollama\"        # or \"openai-compat\"\n");
            output.push_str("# endpoint = \"http://localhost:11434\"\n");
            output.push_str("# model = \"nomic-embed-text\"\n");
            output.push_str("# dimensions = 768\n");
            output.push_str("# batch_size = 32\n");
            output.push_str("# api_key = \"sk-...\"         # only for openai-compat\n");
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_config(root: &Path, content: &str) {
        let path = crate::paths::cache_config_path(root).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
    }

    #[test]
    fn load_returns_default_when_no_config_file() {
        let dir = TempDir::new().unwrap();
        let config = Config::load(dir.path()).unwrap();
        assert!(config.embeddings.is_none());
        assert!(config.search.is_none());
    }

    #[test]
    fn load_parses_embeddings_config() {
        let dir = TempDir::new().unwrap();
        write_config(
            dir.path(),
            r#"
[embeddings]
enabled = true
provider = "openai"
model = "text-embedding-3-small"
endpoint = "https://api.openai.com/v1/embeddings"
dimensions = 1536
"#,
        );

        let config = Config::load(dir.path()).unwrap();
        let emb = config.embeddings.unwrap();
        assert!(emb.enabled);
        assert_eq!(emb.provider, "openai");
        assert_eq!(emb.model, "text-embedding-3-small");
        assert_eq!(emb.dimensions, 1536);
        assert!(emb.api_key.is_none());
        assert!(emb.batch_size.is_none());
    }

    #[test]
    fn load_defaults_enabled_to_true_when_omitted() {
        let dir = TempDir::new().unwrap();
        write_config(
            dir.path(),
            r#"
[embeddings]
provider = "openai-compat"
model = "nomic-embed-text-v1.5"
endpoint = "http://localhost:1234/v1/embeddings"
dimensions = 768
"#,
        );

        let config = Config::load(dir.path()).unwrap();
        let emb = config.embeddings.unwrap();
        assert!(emb.enabled);
        assert_eq!(emb.provider, "openai-compat");
        assert_eq!(emb.dimensions, 768);
    }

    #[test]
    fn load_parses_full_embeddings_config() {
        let dir = TempDir::new().unwrap();
        write_config(
            dir.path(),
            r#"
[embeddings]
enabled = false
provider = "ollama"
model = "nomic-embed-text"
endpoint = "http://localhost:11434/api/embeddings"
api_key = "sk-test"
dimensions = 768
batch_size = 32
"#,
        );

        let config = Config::load(dir.path()).unwrap();
        let emb = config.embeddings.unwrap();
        assert!(!emb.enabled);
        assert_eq!(emb.api_key.as_deref(), Some("sk-test"));
        assert_eq!(emb.batch_size, Some(32));
    }

    #[test]
    fn load_parses_search_config() {
        let dir = TempDir::new().unwrap();
        write_config(
            dir.path(),
            r#"
[search]
hybrid_alpha = 0.7
default_limit = 50
"#,
        );

        let config = Config::load(dir.path()).unwrap();
        let search = config.search.unwrap();
        assert!((search.hybrid_alpha - 0.7).abs() < f64::EPSILON);
        assert_eq!(search.default_limit, 50);
    }

    #[test]
    fn search_config_defaults() {
        let dir = TempDir::new().unwrap();
        write_config(
            dir.path(),
            r#"
[search]
"#,
        );

        let config = Config::load(dir.path()).unwrap();
        let search = config.search.unwrap();
        assert!((search.hybrid_alpha - 0.4).abs() < f64::EPSILON);
        assert_eq!(search.default_limit, 20);
    }

    #[test]
    fn hybrid_alpha_accessor_with_config() {
        let dir = TempDir::new().unwrap();
        write_config(
            dir.path(),
            r#"
[search]
hybrid_alpha = 0.8
"#,
        );
        let config = Config::load(dir.path()).unwrap();
        assert!((config.hybrid_alpha() - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn hybrid_alpha_accessor_without_config() {
        let config = Config::default();
        assert!((config.hybrid_alpha() - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn default_limit_accessor_with_config() {
        let dir = TempDir::new().unwrap();
        write_config(
            dir.path(),
            r#"
[search]
default_limit = 30
"#,
        );
        let config = Config::load(dir.path()).unwrap();
        assert_eq!(config.default_limit(), 30);
    }

    #[test]
    fn default_limit_accessor_without_config() {
        let config = Config::default();
        assert_eq!(config.default_limit(), 20);
    }

    #[test]
    fn generate_template_without_detection() {
        let template = Config::generate_template(None);
        assert!(template.contains("[search]"));
        assert!(template.contains("[embeddings]"));
        assert!(template.contains("# enabled = true"));
        assert!(template.contains("# provider = \"ollama\""));
    }

    #[test]
    fn generate_template_with_detection() {
        let detected = EmbeddingsConfig {
            enabled: true,
            provider: "ollama".to_string(),
            model: "nomic-embed-text".to_string(),
            endpoint: "http://localhost:11434".to_string(),
            api_key: None,
            dimensions: 768,
            batch_size: None,
        };
        let template = Config::generate_template(Some(&detected));
        assert!(template.contains("provider = \"ollama\""));
        assert!(template.contains("model = \"nomic-embed-text\""));
        assert!(template.contains("dimensions = 768"));
        assert!(!template.contains("# provider"));
    }
}
