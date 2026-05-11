mod github;
mod review;

use toolkit_core::{config::Config, vllm};

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();

    println!("{}", "=".repeat(60));
    println!("AI Code Reviewer - Self-Hosted Runner");
    println!("{}", "=".repeat(60));

    let cfg = Config::from_env();
    let gh = github::GithubConfig::from_env();

    // Detect model
    let model = match vllm::resolve_model(&cfg).await {
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
    println!("PR:        {}#{}", gh.repository, gh.pr_number);

    // Fetch diff
    let diff = match github::fetch_pr_diff(&gh).await {
        Ok(d) if !d.trim().is_empty() => d,
        Ok(_) => {
            eprintln!("Empty diff -- nothing to review.");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Failed to fetch diff: {e}");
            std::process::exit(1);
        }
    };

    println!("Diff size: {} chars", diff.len());

    // Review
    let review_text = match review::review_diff(&diff, &model, &cfg).await {
        Some(r) => r,
        None => {
            eprintln!("Review failed or returned empty.");
            std::process::exit(1);
        }
    };

    println!("{review_text}");

    // Post comment
    if review::post_pr_comment(&review_text, &gh.repository, gh.pr_number, &gh.token).await {
        println!("Review comment posted successfully.");
    } else {
        eprintln!("Review generated but comment posting failed.");
        std::process::exit(1);
    }
}
