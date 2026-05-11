use std::env;

pub struct GithubConfig {
    pub repository: String,
    pub pr_number:  u64,
    pub token:      String,
}

impl GithubConfig {
    pub fn from_env() -> Self {
        Self {
            repository: env::var("GITHUB_REPOSITORY")
                .expect("GITHUB_REPOSITORY must be set"),
            pr_number: env::var("PULL_REQUEST_NUMBER")
                .expect("PULL_REQUEST_NUMBER must be set")
                .parse()
                .expect("PULL_REQUEST_NUMBER must be a number"),
            token: env::var("GITHUB_TOKEN")
                .expect("GITHUB_TOKEN must be set"),
        }
    }
}

/// Fetches the unified diff for a PR as a single string.
///
/// Uses `octocrab::Octocrab::all_pages` to transparently follow Link-header
/// pagination so every changed file is included regardless of PR size.
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

    #[test]
    fn env_config_reads_vars() {
        unsafe {
            std::env::set_var("GITHUB_REPOSITORY", "owner/repo");
            std::env::set_var("PULL_REQUEST_NUMBER", "42");
            std::env::set_var("GITHUB_TOKEN", "ghp_test");
        }
        let cfg = GithubConfig::from_env();
        assert_eq!(cfg.repository, "owner/repo");
        assert_eq!(cfg.pr_number, 42u64);
        assert_eq!(cfg.token, "ghp_test");
        unsafe {
            std::env::remove_var("GITHUB_REPOSITORY");
            std::env::remove_var("PULL_REQUEST_NUMBER");
            std::env::remove_var("GITHUB_TOKEN");
        }
    }
}
