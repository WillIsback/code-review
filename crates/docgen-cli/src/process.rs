use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Semaphore;
use toolkit_core::config::Config;
use toolkit_core::vllm::{chat_complete, ChatMessage};

pub struct PatchResult {
    pub path: PathBuf,
    pub content: String,
}

pub async fn process_files(
    files: Vec<PathBuf>,
    fmt: Option<&str>,
    force: bool,
    model: &str,
    cfg: &Config,
) -> Vec<PatchResult> {
    use futures::future::join_all;
    let semaphore = Arc::new(Semaphore::new(cfg.batch_size));
    let client = Arc::new(cfg.http_client());
    let cfg = Arc::new(cfg.clone());

    let tasks: Vec<_> = files
        .into_iter()
        .map(|path| {
            let sem = Arc::clone(&semaphore);
            let cfg = Arc::clone(&cfg);
            let client = Arc::clone(&client);
            let model = model.to_string();
            let fmt = fmt.map(String::from);
            tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                let source = match tokio::fs::read_to_string(&path).await {
                    Ok(s) => s,
                    Err(_) => return None,
                };
                let format = fmt.unwrap_or_else(|| {
                    if path.extension().and_then(|e| e.to_str()) == Some("py") {
                        "mkdocs".to_string()
                    } else {
                        "tsdoc".to_string()
                    }
                });
                let language = if path.extension().and_then(|e| e.to_str()) == Some("py") {
                    "Python"
                } else {
                    "TypeScript"
                };
                let action = if force {
                    "Replace all existing docstrings and add missing ones"
                } else {
                    "Add docstrings to all functions and classes that are missing them"
                };
                let prompt = format!(
                    "{action} using {format} format in the following {language} source code. \
                     Return ONLY the complete patched source code with no explanation and no markdown fences.\n\n{source}"
                );
                let messages = [ChatMessage { role: "user", content: prompt }];
                match chat_complete(&messages, &model, 4096, 0.2, &client, &cfg).await {
                    Ok(content) => Some(PatchResult { path, content }),
                    Err(e) => {
                        eprintln!("  warning: LLM error for {}: {e}", path.display());
                        None
                    }
                }
            })
        })
        .collect();

    join_all(tasks)
        .await
        .into_iter()
        .filter_map(|r| r.ok().flatten())
        .collect()
}
