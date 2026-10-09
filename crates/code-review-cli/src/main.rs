mod config;
mod error;
mod github;
mod report;
mod review;
mod source;
mod vllm;

use config::Config;
use github::{GithubConfig, TargetType};

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();

    println!("{}", "=".repeat(60));
    println!("AI Code Reviewer");
    println!("{}", "=".repeat(60));

    let cfg = Config::from_env();
    let client = cfg.connect_client();
    let gh = match GithubConfig::from_env() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("GitHub configuration error: {e}");
            std::process::exit(1);
        }
    };

    // Use a client with the full vLLM timeout for LLM/API requests
    let llm_client = cfg.http_client();

    let target_number = match resolve_target_number(&gh, &llm_client).await {
        Ok(n) => n,
        Err(e) => {
            eprintln!("Could not resolve target number: {e}");
            std::process::exit(1);
        }
    };

    // Detect model
    let model = match vllm::resolve_model(&client, &cfg).await {
        Ok(m) => {
            println!("Auto-detected model: {m}");
            m
        }
        Err(e) => {
            eprintln!("Could not detect model: {e}");
            std::process::exit(1);
        }
    };

    println!("vLLM URL:  {}", cfg.vllm_base_url);
    println!(
        "Target:    {} {}#{}",
        target_label(gh.target_type),
        gh.repository,
        target_number
    );

    let review_text = match gh.target_type {
        TargetType::Pr => review_pr(&gh, target_number, &model, &llm_client, &cfg).await,
        TargetType::Issue => {
            match github::fetch_issue_context(&gh.repository, target_number, &gh.token, &llm_client)
                .await
            {
                Ok(context) => review::review_issue(&context, &model, &llm_client, &cfg).await,
                Err(e) => {
                    eprintln!("Failed to fetch issue context: {e}");
                    None
                }
            }
        }
    };

    let review_text = match review_text {
        Some(r) => r,
        None => {
            eprintln!("Review failed or returned empty.");
            std::process::exit(1);
        }
    };

    println!("{review_text}");

    // Post comment (the issues endpoint serves both PRs and issues).
    // Updates the previous review comment in place when one exists.
    match github::post_or_update_comment(
        &review_text,
        &gh.repository,
        target_number,
        &gh.token,
        &llm_client,
    )
    .await
    {
        Ok(()) => println!("Review comment posted/updated successfully."),
        Err(e) => {
            eprintln!("Review generated but comment posting failed: {e}");
            std::process::exit(1);
        }
    }
}

fn target_label(target_type: TargetType) -> &'static str {
    match target_type {
        TargetType::Pr => "PR",
        TargetType::Issue => "Issue",
    }
}

async fn resolve_target_number(gh: &GithubConfig, client: &reqwest::Client) -> Result<u64, String> {
    if let Some(n) = gh.target_number {
        return Ok(n);
    }
    match gh.target_type {
        TargetType::Pr => {
            let sha = std::env::var("HEAD_SHA")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| {
                    "TARGET_NUMBER/PULL_REQUEST_NUMBER not set and HEAD_SHA missing".to_string()
                })?;
            github::resolve_pr_number(&gh.repository, &sha, &gh.token, client).await
        }
        TargetType::Issue => Err("TARGET_NUMBER must be set in issue mode".to_string()),
    }
}

async fn review_pr(
    gh: &GithubConfig,
    pr_number: u64,
    model: &str,
    client: &reqwest::Client,
    cfg: &Config,
) -> Option<String> {
    let diff = match github::fetch_pr_diff(&gh.repository, pr_number, &gh.token).await {
        Ok(d) if !d.trim().is_empty() => d,
        Ok(_) => {
            println!("Empty diff -- nothing to review (skipped).");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Failed to fetch diff: {e}");
            return None;
        }
    };
    println!("Diff size: {} chars", diff.len());

    let head_sha = match std::env::var("HEAD_SHA")
        .ok()
        .filter(|s| !s.trim().is_empty())
    {
        Some(s) => s,
        None => {
            match github::resolve_head_sha(&gh.repository, pr_number, &gh.token, client).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!(
                    "Warning: could not resolve head SHA ({e}); continuing without source context."
                );
                    String::new()
                }
            }
        }
    };

    let paths = source::extract_modified_files(&diff);
    let (source_files, unfetched) = if head_sha.is_empty() {
        (Vec::new(), paths)
    } else {
        let (fetched, failed) =
            github::fetch_source_files(&gh.repository, &head_sha, &paths, &gh.token, client).await;
        if !failed.is_empty() {
            eprintln!(
                "Warning: {} file(s) could not be fetched for verification.",
                failed.len()
            );
        }
        (fetched, failed)
    };

    review::review_diff(&diff, &source_files, &unfetched, model, client, cfg).await
}
