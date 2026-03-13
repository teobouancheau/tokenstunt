use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::EmbeddingProvider;

/// Known Ollama embedding models with their default dimensions.
const KNOWN_EMBED_MODELS: &[(&str, usize)] = &[
    ("nomic-embed-text", 768),
    ("mxbai-embed-large", 1024),
    ("all-minilm", 384),
    ("snowflake-arctic-embed", 1024),
    ("bge-m3", 1024),
    ("bge-large", 1024),
];

#[derive(Debug, Clone)]
pub struct DetectedOllama {
    pub endpoint: String,
    pub model: String,
    pub dimensions: usize,
}

#[derive(Deserialize)]
struct TagsResponse {
    models: Vec<TagModel>,
}

#[derive(Deserialize)]
struct TagModel {
    name: String,
}

/// Probes `http://localhost:11434/api/tags` for a running Ollama instance.
/// Returns the first known embedding model found, or None.
pub async fn detect_ollama() -> Option<DetectedOllama> {
    let endpoint = "http://localhost:11434";
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .ok()?;

    let url = format!("{endpoint}/api/tags");
    let response = client.get(&url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }

    let tags: TagsResponse = response.json().await.ok()?;
    let model_names: Vec<String> = tags.models.iter().map(|m| strip_tag(&m.name)).collect();

    for (known_model, dims) in KNOWN_EMBED_MODELS {
        if model_names.iter().any(|name| name == known_model) {
            return Some(DetectedOllama {
                endpoint: endpoint.to_string(),
                model: known_model.to_string(),
                dimensions: *dims,
            });
        }
    }

    None
}

fn strip_tag(name: &str) -> String {
    name.split(':').next().unwrap_or(name).to_string()
}

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

    fn model_name(&self) -> &str {
        &self.model
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
    async fn test_embed_batch_single_text() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/api/embed")
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "model": "nomic-embed-text",
                "input": ["hello world"]
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "embeddings": [[0.1, 0.2, 0.3]]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = OllamaProvider::new(&server.url(), "nomic-embed-text", 3);
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
            .mock("POST", "/api/embed")
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "model": "nomic-embed-text",
                "input": ["hello", "world"]
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "embeddings": [[0.1, 0.2], [0.3, 0.4]]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let provider = OllamaProvider::new(&server.url(), "nomic-embed-text", 2);
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
            .mock("GET", "/api/tags")
            .with_status(200)
            .with_body("{}")
            .create_async()
            .await;

        let provider = OllamaProvider::new(&server.url(), "nomic-embed-text", 3);
        provider.health_check().await.unwrap();
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_health_check_failure() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/api/tags")
            .with_status(500)
            .with_body("internal server error")
            .create_async()
            .await;

        let provider = OllamaProvider::new(&server.url(), "nomic-embed-text", 3);
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
            .mock("POST", "/api/embed")
            .with_status(500)
            .with_body("model not found")
            .create_async()
            .await;

        let provider = OllamaProvider::new(&server.url(), "nomic-embed-text", 3);
        let result = provider.embed_batch(&["hello".to_string()]).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("failed with status"));
        assert!(err.contains("model not found"));
        mock.assert_async().await;
    }

    #[test]
    fn test_strip_tag_with_version() {
        assert_eq!(strip_tag("nomic-embed-text:latest"), "nomic-embed-text");
    }

    #[test]
    fn test_strip_tag_without_version() {
        assert_eq!(strip_tag("nomic-embed-text"), "nomic-embed-text");
    }

    #[test]
    fn test_detect_ollama_with_known_model() {
        let tags: TagsResponse = serde_json::from_str(
            &serde_json::json!({
                "models": [
                    { "name": "llama3:latest" },
                    { "name": "nomic-embed-text:latest" }
                ]
            })
            .to_string(),
        )
        .unwrap();

        let model_names: Vec<String> = tags.models.iter().map(|m| strip_tag(&m.name)).collect();

        let mut detected = None;
        for (known_model, dims) in KNOWN_EMBED_MODELS {
            if model_names.iter().any(|name| name == known_model) {
                detected = Some(DetectedOllama {
                    endpoint: "http://localhost:11434".to_string(),
                    model: known_model.to_string(),
                    dimensions: *dims,
                });
                break;
            }
        }

        assert!(detected.is_some());
        let d = detected.unwrap();
        assert_eq!(d.model, "nomic-embed-text");
        assert_eq!(d.dimensions, 768);
    }

    #[test]
    fn test_detect_ollama_no_embed_models() {
        let tags: TagsResponse = serde_json::from_str(
            &serde_json::json!({
                "models": [
                    { "name": "llama3:latest" },
                    { "name": "codellama:latest" }
                ]
            })
            .to_string(),
        )
        .unwrap();

        let model_names: Vec<String> = tags.models.iter().map(|m| strip_tag(&m.name)).collect();

        let mut detected = None;
        for (known_model, dims) in KNOWN_EMBED_MODELS {
            if model_names.iter().any(|name| name == known_model) {
                detected = Some(DetectedOllama {
                    endpoint: String::new(),
                    model: known_model.to_string(),
                    dimensions: *dims,
                });
                break;
            }
        }

        assert!(detected.is_none());
    }
}
