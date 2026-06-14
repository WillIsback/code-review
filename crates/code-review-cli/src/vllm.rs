use crate::{config::Config, error::AppError};
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
async fn post_chat_completions(
    body: serde_json::Value,
    client: &reqwest::Client,
    cfg: &Config,
) -> Result<String, AppError> {
    let base = cfg.vllm_base_url.trim_end_matches('/');
    let url = if base.ends_with("/v1") {
        format!("{base}/chat/completions")
    } else {
        format!("{base}/v1/chat/completions")
    };

    let mut last_err = None;
    let max_attempts = cfg.vllm_retries.saturating_add(1);
    let resp: serde_json::Value = 'retry: {
        for attempt in 1..=max_attempts {
            let result = client
                .post(&url)
                .header("Authorization", format!("Bearer {}", cfg.vllm_api_key))
                .json(&body)
                .send()
                .await;

            match result {
                Ok(response) => match response.error_for_status() {
                    Ok(ok_resp) => match ok_resp.json().await {
                        Ok(json) => break 'retry json,
                        Err(e) => {
                            last_err = Some(e.to_string());
                        }
                    },
                    Err(e) => {
                        last_err = Some(e.to_string());
                    }
                },
                Err(e) => {
                    last_err = Some(e.to_string());
                }
            }

            if attempt < max_attempts {
                eprintln!("vLLM request attempt {attempt}/{max_attempts} failed, retrying...");
                tokio::time::sleep(std::time::Duration::from_secs(u64::from(attempt))).await;
            }
        }
        return Err(AppError::RequestFailed {
            reason: last_err.unwrap_or_else(|| "all retries exhausted".to_string()),
        });
    };

    let content = resp["choices"][0]["message"]["content"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| AppError::RequestFailed {
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
    client: &reqwest::Client,
    cfg: &Config,
) -> Result<String, AppError> {
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

    post_chat_completions(body, client, cfg).await
}

/// Returns `VLLM_MODEL` env var if set, otherwise auto-detects from server.
pub async fn resolve_model(client: &reqwest::Client, cfg: &Config) -> Result<String, AppError> {
    if let Some(model) = std::env::var("VLLM_MODEL")
        .ok()
        .filter(|s| !s.trim().is_empty())
    {
        Ok(model)
    } else {
        detect_model(client, cfg).await
    }
}

/// Returns the first model ID advertised by the vLLM server.
pub async fn detect_model(client: &reqwest::Client, cfg: &Config) -> Result<String, AppError> {
    let resp: ModelsResponse = client
        .get(cfg.models_url())
        .header("Authorization", format!("Bearer {}", cfg.vllm_api_key))
        .send()
        .await
        .map_err(|_| AppError::VllmUnreachable {
            url: cfg.models_url(),
        })?
        .error_for_status()
        .map_err(|e| AppError::RequestFailed {
            reason: e.to_string(),
        })?
        .json()
        .await?;
    resp.data
        .into_iter()
        .next()
        .map(|m| m.id)
        .ok_or(AppError::NoModelsAvailable)
}
