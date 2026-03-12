use std::path::Path;

use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    pub embeddings: Option<EmbeddingsConfig>,
}

#[derive(Debug, Deserialize)]
pub struct EmbeddingsConfig {
    pub enabled: bool,
    pub provider: String,
    pub model: String,
    pub endpoint: String,
    pub api_key: Option<String>,
    pub dimensions: usize,
    pub batch_size: Option<usize>,
}

impl Config {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join(".tokenstunt/config.toml");
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

    #[test]
    fn load_returns_default_when_no_config_file() {
        let dir = TempDir::new().unwrap();
        let config = Config::load(dir.path()).unwrap();
        assert!(config.embeddings.is_none());
    }

    #[test]
    fn load_parses_embeddings_config() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join(".tokenstunt");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("config.toml"),
            r#"
[embeddings]
enabled = true
provider = "openai"
model = "text-embedding-3-small"
endpoint = "https://api.openai.com/v1/embeddings"
dimensions = 1536
"#,
        )
        .unwrap();

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
    fn load_parses_full_embeddings_config() {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join(".tokenstunt");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("config.toml"),
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
        )
        .unwrap();

        let config = Config::load(dir.path()).unwrap();
        let emb = config.embeddings.unwrap();
        assert!(!emb.enabled);
        assert_eq!(emb.api_key.as_deref(), Some("sk-test"));
        assert_eq!(emb.batch_size, Some(32));
    }
}
