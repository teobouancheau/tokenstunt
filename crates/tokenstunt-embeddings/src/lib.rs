mod ollama;
mod openai;

pub use ollama::OllamaProvider;
pub use openai::OpenAiCompatProvider;

use anyhow::{Result, bail};

#[async_trait::async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    fn dimensions(&self) -> usize;
    async fn health_check(&self) -> Result<()>;
}

pub fn load_provider(
    provider: &str,
    endpoint: &str,
    model: &str,
    dimensions: usize,
    api_key: Option<&str>,
) -> Result<Box<dyn EmbeddingProvider>> {
    match provider {
        "ollama" => Ok(Box::new(OllamaProvider::new(endpoint, model, dimensions))),
        "openai-compat" => Ok(Box::new(OpenAiCompatProvider::new(
            endpoint, model, dimensions, api_key,
        ))),
        other => bail!("unknown embedding provider: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_provider_ollama() {
        let provider = load_provider(
            "ollama",
            "http://localhost:11434",
            "nomic-embed-text",
            768,
            None,
        );
        assert!(provider.is_ok());
        assert_eq!(provider.unwrap().dimensions(), 768);
    }

    #[test]
    fn test_load_provider_openai_compat() {
        let provider = load_provider(
            "openai-compat",
            "http://localhost:8080",
            "text-embedding-3-small",
            1536,
            Some("sk-test"),
        );
        assert!(provider.is_ok());
        assert_eq!(provider.unwrap().dimensions(), 1536);
    }

    #[test]
    fn test_load_provider_unknown() {
        let result = load_provider("unknown", "http://localhost", "model", 768, None);
        let err = result.err().expect("should return an error");
        assert!(
            err.to_string()
                .contains("unknown embedding provider: unknown")
        );
    }
}
