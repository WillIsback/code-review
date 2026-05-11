use crate::{config::Config, error::CoreError};
use serde::Deserialize;

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
}

/// Returns `true` when the vLLM /v1/models endpoint responds with 2xx.
pub async fn check_reachable(cfg: &Config) -> bool {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(cfg.connect_timeout_secs))
        .build()
        .expect("valid client");
    client.get(cfg.models_url()).send().await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Returns `VLLM_MODEL` env var if set, otherwise auto-detects from server.
pub async fn resolve_model(cfg: &Config) -> Result<String, CoreError> {
    if let Some(model) = std::env::var("VLLM_MODEL").ok().filter(|s| !s.trim().is_empty()) {
        Ok(model)
    } else {
        detect_model(cfg).await
    }
}

/// Returns the first model ID advertised by the vLLM server.
pub async fn detect_model(cfg: &Config) -> Result<String, CoreError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(cfg.connect_timeout_secs))
        .build()
        .expect("valid client");
    let resp: ModelsResponse = client
        .get(cfg.models_url())
        .send()
        .await
        .map_err(|_| CoreError::VllmUnreachable { url: cfg.models_url() })?
        .json()
        .await?;
    resp.data
        .into_iter()
        .next()
        .map(|m| m.id)
        .ok_or(CoreError::NoModelsAvailable)
}
