use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::EmbeddingProvider;

pub struct OllamaProvider {
    client: reqwest::Client,
    endpoint: String,
    model: String,
    dimensions: usize,
}

#[derive(Serialize)]
struct EmbedRequest {
    model: String,
    input: Vec<String>,
}

#[derive(Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

impl OllamaProvider {
    pub fn new(endpoint: &str, model: &str, dimensions: usize) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint: endpoint.to_string(),
            model: model.to_string(),
            dimensions,
        }
    }
}

#[async_trait::async_trait]
impl EmbeddingProvider for OllamaProvider {
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/api/embed", self.endpoint);
        let request = EmbedRequest {
            model: self.model.clone(),
            input: texts.to_vec(),
        };

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("failed to send embed request to Ollama")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama embed request failed with status {status}: {body}");
        }

        let embed_response: EmbedResponse = response
            .json()
            .await
            .context("failed to parse Ollama embed response")?;

        Ok(embed_response.embeddings)
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    async fn health_check(&self) -> Result<()> {
        let url = format!("{}/api/tags", self.endpoint);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("failed to reach Ollama server")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Ollama health check failed with status {}",
                response.status()
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_ollama_embed() {
        let provider = OllamaProvider::new("http://localhost:11434", "nomic-embed-text", 768);
        provider.health_check().await.unwrap();
        let vecs = provider
            .embed_batch(&["hello world".to_string()])
            .await
            .unwrap();
        assert_eq!(vecs.len(), 1);
        assert_eq!(vecs[0].len(), 768);
    }

    #[tokio::test]
    #[ignore]
    async fn test_ollama_embed_batch() {
        let provider = OllamaProvider::new("http://localhost:11434", "nomic-embed-text", 768);
        let texts = vec!["hello world".to_string(), "foo bar".to_string()];
        let vecs = provider.embed_batch(&texts).await.unwrap();
        assert_eq!(vecs.len(), 2);
        assert_eq!(vecs[0].len(), 768);
        assert_eq!(vecs[1].len(), 768);
    }
}
