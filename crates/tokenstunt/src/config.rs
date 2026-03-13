use std::path::Path;

use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    pub embeddings: Option<EmbeddingsConfig>,
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
    #[allow(dead_code)]
    pub batch_size: Option<usize>,
}

const fn default_true() -> bool {
    true
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
}
