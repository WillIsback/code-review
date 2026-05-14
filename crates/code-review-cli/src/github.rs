use std::env;

#[derive(Debug)]
pub struct GithubConfig {
    pub repository: String,
    pub pr_number:  u64,
    pub token:      String,
}

impl GithubConfig {
    pub fn from_env() -> Result<Self, String> {
        Ok(Self {
            repository: env::var("GITHUB_REPOSITORY")
                .map_err(|_| "GITHUB_REPOSITORY must be set")?,
            pr_number: env::var("PULL_REQUEST_NUMBER")
                .map_err(|_| "PULL_REQUEST_NUMBER must be set")?
                .parse()
                .map_err(|_| "PULL_REQUEST_NUMBER must be a number")?,
            token: env::var("GITHUB_TOKEN")
                .map_err(|_| "GITHUB_TOKEN must be set")?,
        })
    }
}

/// Returns true for files that should be excluded from code review:
/// lock files (Cargo.lock, *.lock, *-lock.json, lock.yaml/yml),
/// dotfiles/dot-directories (e.g. `.gitignore`, `.github/`), and
/// documentation files (`.md`, `.mdx`).
fn should_skip_file(filename: &str) -> bool {
    // Lock files
    let basename = filename.rsplit('/').next().unwrap_or(filename);
    if basename == "Cargo.lock"
        || basename == "npm-shrinkwrap.json"
        || filename.ends_with(".lock")
        || filename.ends_with(".lockb")
        || filename.ends_with("-lock.json")
        || basename == "lock.yaml"
        || basename == "lock.yml"
    {
        return true;
    }
    // Markdown / documentation
    if filename.ends_with(".md") || filename.ends_with(".mdx") {
        return true;
    }
    // Dotfiles and dot-directories (e.g. .gitignore, .github/workflows/ci.yml)
    filename.split('/').any(|part| part.starts_with('.'))
}

/// Fetches the unified diff for a PR as a single string.
///
/// Uses `octocrab::Octocrab::all_pages` to transparently follow Link-header
/// pagination so every changed file is included regardless of PR size.
/// Lock files, dotfiles/dot-directories, and documentation files are excluded.
pub async fn fetch_pr_diff(cfg: &GithubConfig) -> Result<String, String> {
    let octocrab = octocrab::Octocrab::builder()
        .personal_token(cfg.token.clone())
        .build()
        .map_err(|e| e.to_string())?;

    let parts: Vec<&str> = cfg.repository.splitn(2, '/').collect();
    if parts.len() != 2 {
        return Err(format!("Invalid repository format: {}", cfg.repository));
    }
    let (owner, repo) = (parts[0], parts[1]);

    // list_files returns Page<DiffEntry>; all_pages follows Link headers.
    let first_page = octocrab
        .pulls(owner, repo)
        .list_files(cfg.pr_number)
        .await
        .map_err(|e| e.to_string())?;

    let entries = octocrab
        .all_pages(first_page)
        .await
        .map_err(|e| e.to_string())?;

    let mut diff = String::new();
    for entry in &entries {
        if should_skip_file(&entry.filename) {
            println!("Skipping: {}", entry.filename);
            continue;
        }
        if let Some(patch) = &entry.patch {
            diff.push_str(&format!("\n\n# File: {}\n", entry.filename));
            diff.push_str(patch);
        }
    }

    Ok(diff)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn skip_file_skips_bun_lockb() {
        assert!(should_skip_file("bun.lockb"));
        assert!(should_skip_file("packages/app/bun.lockb"));
    }

    #[test]
    fn skip_file_skips_npm_shrinkwrap() {
        assert!(should_skip_file("npm-shrinkwrap.json"));
        assert!(should_skip_file("packages/app/npm-shrinkwrap.json"));
    }

    #[test]
    fn skip_file_filters_lock_dotfiles_and_markdown() {
        // Lock files
        assert!(should_skip_file("Cargo.lock"));
        assert!(should_skip_file("package-lock.json"));
        assert!(should_skip_file("yarn.lock"));
        // Dotfiles and dot-directories
        assert!(should_skip_file(".gitignore"));
        assert!(should_skip_file(".github/workflows/ci.yml"));
        // Markdown
        assert!(should_skip_file("README.md"));
        assert!(should_skip_file("docs/guide.mdx"));
        // Source files should pass through
        assert!(!should_skip_file("src/main.rs"));
        assert!(!should_skip_file("frontend/src/components/ui/button.tsx"));
        assert!(!should_skip_file("Cargo.toml"));
    }

    #[test]
    #[serial]
    fn env_config_reads_vars() {
        unsafe {
            std::env::set_var("GITHUB_REPOSITORY", "owner/repo");
            std::env::set_var("PULL_REQUEST_NUMBER", "42");
            std::env::set_var("GITHUB_TOKEN", "ghp_test");
        }
        let cfg = GithubConfig::from_env().expect("env vars are set");
        assert_eq!(cfg.repository, "owner/repo");
        assert_eq!(cfg.pr_number, 42u64);
        assert_eq!(cfg.token, "ghp_test");
        unsafe {
            std::env::remove_var("GITHUB_REPOSITORY");
            std::env::remove_var("PULL_REQUEST_NUMBER");
            std::env::remove_var("GITHUB_TOKEN");
        }
    }

    #[test]
    #[serial]
    fn env_config_returns_error_when_missing() {
        unsafe {
            std::env::remove_var("GITHUB_REPOSITORY");
            std::env::remove_var("PULL_REQUEST_NUMBER");
            std::env::remove_var("GITHUB_TOKEN");
        }
        let result = GithubConfig::from_env();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("GITHUB_REPOSITORY"));
    }
}
