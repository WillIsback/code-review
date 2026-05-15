mod config;
mod error;
mod github;
mod review;
mod vllm;

use config::Config;

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();

    println!("{}", "=".repeat(60));
    println!("AI Code Reviewer - Self-Hosted Runner");
    println!("{}", "=".repeat(60));

    let cfg = Config::from_env();
    let client = cfg.connect_client();
    let gh = match github::GithubConfig::from_env() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("GitHub configuration error: {e}");
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
    println!("PR:        {}#{}", gh.repository, gh.pr_number);

    // Fetch diff
    let diff = match github::fetch_pr_diff(&gh).await {
        Ok(d) if !d.trim().is_empty() => d,
        Ok(_) => {
            println!("Empty diff -- nothing to review (skipped).");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Failed to fetch diff: {e}");
            std::process::exit(1);
        }
    };

    println!("Diff size: {} chars", diff.len());

    // Review (use a client with the full vLLM timeout for LLM requests)
    let llm_client = cfg.http_client();
    let review_text = match review::review_diff(&diff, &model, &llm_client, &cfg).await {
        Some(r) => r,
        None => {
            eprintln!("Review failed or returned empty.");
            std::process::exit(1);
        }
    };

    println!("{review_text}");

    // Post comment
    match github::post_pr_comment(&review_text, &gh.repository, gh.pr_number, &gh.token).await {
        Ok(()) => println!("Review comment posted successfully."),
        Err(e) => {
            eprintln!("Review generated but comment posting failed: {e}");
            std::process::exit(1);
        }
    }
}
