mod apply;
mod cli;
mod detect;
mod process;
mod resolver;

use clap::Parser;
use cli::Cli;
use toolkit_core::{config::Config, git, vllm};

#[tokio::main]
async fn main() {
    // Load global config first, then project .env overrides it
    if let Some(home) = std::env::var_os("HOME") {
        let _ = dotenvy::from_path(
            std::path::Path::new(&home).join(".config/docgen/.env")
        );
    }
    let _ = dotenvy::dotenv();

    let args = Cli::parse();
    let cfg = Config::from_env();

    // 1. Dirty-tree guard
    let dirty = git::dirty_files(std::path::Path::new("."));
    if !dirty.is_empty() {
        eprintln!(
            "error: working tree has uncommitted changes. Commit or stash before running docgen:"
        );
        for f in &dirty {
            eprintln!("  {f}");
        }
        std::process::exit(1);
    }

    // 2. vLLM reachability
    if !vllm::check_reachable(&cfg).await {
        eprintln!("error: vLLM not reachable at {}", cfg.vllm_base_url);
        std::process::exit(1);
    }

    // 3. Model detection
    let model = match vllm::resolve_model(&cfg).await {
        Ok(m) => {
            println!("Auto-detected model: {m}");
            m
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    // 4. File resolution
    let files = resolver::resolve_files(&args.target, args.recursive);
    if files.is_empty() {
        println!("warning: no Python or TypeScript files found.");
        std::process::exit(2);
    }

    // 5. Filter files needing docstrings (rayon parallel read)
    use rayon::prelude::*;
    let to_process: Vec<_> = files
        .into_par_iter()
        .filter(|f| {
            std::fs::read_to_string(f)
                .map(|src| detect::needs_docstrings(f, &src, args.force))
                .unwrap_or(false)
        })
        .collect();

    if to_process.is_empty() {
        println!("nothing to do — all files are already documented.");
        std::process::exit(2);
    }

    println!(
        "Processing {} file(s) in batches of {}...",
        to_process.len(),
        cfg.batch_size
    );

    // 6. LLM processing
    let patches = process::process_files(
        to_process,
        args.fmt.as_deref(),
        args.force,
        &model,
        &cfg,
    )
    .await;

    if patches.is_empty() {
        eprintln!("warning: no files were successfully processed.");
        std::process::exit(1);
    }

    // 7. Git apply
    println!("Applying changes via git branch...");
    if let Err(e) = apply::apply_with_git(patches, std::path::Path::new(".")) {
        eprintln!("error: git error: {e}");
        std::process::exit(1);
    }
    println!("Done. Docstrings applied.");
}
