use anyhow::{Context, Result};
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
        let url = if self.endpoint.ends_with("/embeddings") {
            self.endpoint.clone()
        } else {
            format!("{}/v1/embeddings", self.endpoint.trim_end_matches('/'))
        };
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

    fn model_name(&self) -> &str {
        &self.model
    }

    async fn health_check(&self) -> Result<()> {
        let base = self
            .endpoint
            .trim_end_matches("/v1/embeddings")
            .trim_end_matches('/');
        let url = format!("{}/v1/models", base);
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
    async fn test_embed_batch_single_text() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/embeddings")
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "model": "text-embedding-3-small",
                "input": ["hello world"]
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "data": [{"embedding": [0.1, 0.2, 0.3]}]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = OpenAiCompatProvider::new(&server.url(), "text-embedding-3-small", 3, None);
        let vecs = provider
            .embed_batch(&["hello world".to_string()])
            .await
            .unwrap();

        assert_eq!(vecs.len(), 1);
        assert_eq!(vecs[0], vec![0.1, 0.2, 0.3]);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_embed_batch_multiple_texts() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/embeddings")
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "model": "text-embedding-3-small",
                "input": ["hello", "world"]
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "data": [
                        {"embedding": [0.1, 0.2]},
                        {"embedding": [0.3, 0.4]}
                    ]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = OpenAiCompatProvider::new(&server.url(), "text-embedding-3-small", 2, None);
        let vecs = provider
            .embed_batch(&["hello".to_string(), "world".to_string()])
            .await
            .unwrap();

        assert_eq!(vecs.len(), 2);
        assert_eq!(vecs[0], vec![0.1, 0.2]);
        assert_eq!(vecs[1], vec![0.3, 0.4]);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_health_check_success() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/v1/models")
            .with_status(200)
            .with_body("{}")
            .create_async()
            .await;

        let provider = OpenAiCompatProvider::new(&server.url(), "text-embedding-3-small", 3, None);
        provider.health_check().await.unwrap();
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_health_check_failure() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/v1/models")
            .with_status(500)
            .with_body("internal server error")
            .create_async()
            .await;

        let provider = OpenAiCompatProvider::new(&server.url(), "text-embedding-3-small", 3, None);
        let result = provider.health_check().await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("health check failed"));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_embed_batch_server_error() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/embeddings")
            .with_status(500)
            .with_body("rate limit exceeded")
            .create_async()
            .await;

        let provider = OpenAiCompatProvider::new(&server.url(), "text-embedding-3-small", 3, None);
        let result = provider.embed_batch(&["hello".to_string()]).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("failed with status"));
        assert!(err.contains("rate limit exceeded"));
        mock.assert_async().await;
    }
}
