mod ollama;
mod openai;

pub use ollama::{DetectedOllama, OllamaProvider, detect_ollama};
pub use openai::OpenAiCompatProvider;

use anyhow::{Result, bail};

#[async_trait::async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    fn dimensions(&self) -> usize;
    fn model_name(&self) -> &str;
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
        )
        .unwrap();
        assert_eq!(provider.dimensions(), 768);
        assert_eq!(provider.model_name(), "nomic-embed-text");
    }

    #[test]
    fn test_load_provider_openai_compat() {
        let provider = load_provider(
            "openai-compat",
            "http://localhost:8080",
            "text-embedding-3-small",
            1536,
            Some("sk-test"),
        )
        .unwrap();
        assert_eq!(provider.dimensions(), 1536);
        assert_eq!(provider.model_name(), "text-embedding-3-small");
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
