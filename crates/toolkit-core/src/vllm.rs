use crate::{config::Config, error::CoreError};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
}

/// A single chat message with a role and content.
#[derive(Clone)]
pub struct ChatMessage {
    pub role: &'static str,
    pub content: String,
}

/// POST a JSON body to the vLLM chat completions endpoint and extract the
/// first choice's `content` field.
async fn post_chat_completions(body: serde_json::Value, cfg: &Config) -> Result<String, CoreError> {
    let base = cfg.vllm_base_url.trim_end_matches('/');
    let url = if base.ends_with("/v1") {
        format!("{base}/chat/completions")
    } else {
        format!("{base}/v1/chat/completions")
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(cfg.vllm_timeout_secs))
        .build()
        .expect("valid client");

    let resp: serde_json::Value = client
        .post(&url)
        .header("Authorization", "Bearer none")
        .json(&body)
        .send()
        .await
        .map_err(|e| CoreError::RequestFailed { reason: e.to_string() })?
        .json()
        .await
        .map_err(|e| CoreError::RequestFailed { reason: e.to_string() })?;

    let content = resp["choices"][0]["message"]["content"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| CoreError::RequestFailed {
            reason: "empty or missing content in response".to_string(),
        })?;

    Ok(content.to_string())
}

/// Call /v1/chat/completions with thinking disabled.
/// Returns the first choice's content string, or an error.
pub async fn chat_complete(
    messages: &[ChatMessage],
    model: &str,
    max_tokens: u32,
    temperature: f32,
    cfg: &Config,
) -> Result<String, CoreError> {
    let msgs: Vec<_> = messages
        .iter()
        .map(|m| json!({"role": m.role, "content": m.content}))
        .collect();

    let body = json!({
        "model": model,
        "messages": msgs,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "chat_template_kwargs": {"enable_thinking": false}
    });

    post_chat_completions(body, cfg).await
}

fn build_reasoning_body(
    messages: &[ChatMessage],
    model: &str,
    max_tokens: u32,
    temperature: f32,
) -> serde_json::Value {
    let msgs: Vec<_> = messages
        .iter()
        .map(|m| json!({"role": m.role, "content": m.content}))
        .collect();
    json!({
        "model": model,
        "messages": msgs,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "chat_template_kwargs": {"enable_thinking": true}
    })
}

/// Call /v1/chat/completions with thinking enabled (reasoning model).
/// Discards `reasoning_content`; returns the final `content` string.
pub async fn chat_complete_with_reasoning(
    messages: &[ChatMessage],
    model: &str,
    max_tokens: u32,
    temperature: f32,
    cfg: &Config,
) -> Result<String, CoreError> {
    let body = build_reasoning_body(messages, model, max_tokens, temperature);
    post_chat_completions(body, cfg).await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasoning_body_enables_thinking() {
        let msgs = vec![ChatMessage { role: "user", content: "hello".to_string() }];
        let body = build_reasoning_body(&msgs, "test-model", 512, 0.1);
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], true);
        assert_eq!(body["model"], "test-model");
        assert_eq!(body["messages"][0]["role"], "user");
    }
}
