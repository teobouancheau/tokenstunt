use anyhow::{Result, Context};
use serde::{Deserialize, Serialize};

use crate::EmbeddingProvider;

pub struct OpenAiCompatProvider {
    client: reqwest::Client,
    endpoint: String,
    model: String,
    dimensions: usize,
    api_key: Option<String>,
}

#[derive(Serialize)]
struct EmbeddingRequest {
    model: String,
    input: Vec<String>,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

impl OpenAiCompatProvider {
    pub fn new(endpoint: &str, model: &str, dimensions: usize, api_key: Option<&str>) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint: endpoint.to_string(),
            model: model.to_string(),
            dimensions,
            api_key: api_key.map(String::from),
        }
    }
}

#[async_trait::async_trait]
impl EmbeddingProvider for OpenAiCompatProvider {
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/v1/embeddings", self.endpoint);
        let request = EmbeddingRequest {
            model: self.model.clone(),
            input: texts.to_vec(),
        };

        let mut builder = self.client.post(&url).json(&request);
        if let Some(key) = &self.api_key {
            builder = builder.bearer_auth(key);
        }

        let response = builder
            .send()
            .await
            .context("failed to send embedding request to OpenAI-compatible endpoint")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "OpenAI-compatible embedding request failed with status {status}: {body}"
            );
        }

        let embed_response: EmbeddingResponse = response
            .json()
            .await
            .context("failed to parse OpenAI-compatible embedding response")?;

        let embeddings = embed_response
            .data
            .into_iter()
            .map(|d| d.embedding)
            .collect();

        Ok(embeddings)
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    async fn health_check(&self) -> Result<()> {
        let url = format!("{}/v1/models", self.endpoint);
        let mut builder = self.client.get(&url);
        if let Some(key) = &self.api_key {
            builder = builder.bearer_auth(key);
        }

        let response = builder
            .send()
            .await
            .context("failed to reach OpenAI-compatible endpoint")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "OpenAI-compatible health check failed with status {}",
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
    async fn test_openai_compat_embed() {
        let provider = OpenAiCompatProvider::new(
            "http://localhost:8080",
            "text-embedding-3-small",
            1536,
            None,
        );
        provider.health_check().await.unwrap();
        let vecs = provider
            .embed_batch(&["hello world".to_string()])
            .await
            .unwrap();
        assert_eq!(vecs.len(), 1);
        assert_eq!(vecs[0].len(), 1536);
    }
}
