use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum AppError {
    #[error("vLLM server unreachable at {url}")]
    #[diagnostic(help("Check that your vLLM server is running and VLLM_BASE_URL is correct"))]
    VllmUnreachable { url: String },

    #[error("No models available on vLLM server")]
    NoModelsAvailable,

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Request failed: {reason}")]
    #[diagnostic(help("Check the vLLM server logs for details"))]
    RequestFailed { reason: String },
}
