use std::env;

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetType {
    Pr,
    Issue,
}

impl TargetType {
    fn from_env() -> Self {
        match env::var("TARGET_TYPE").ok().as_deref() {
            Some(s) if s.eq_ignore_ascii_case("issue") => TargetType::Issue,
            _ => TargetType::Pr,
        }
    }
}

#[derive(Debug)]
pub struct GithubConfig {
    pub repository: String,
    pub token: String,
    pub target_type: TargetType,
    pub target_number: Option<u64>,
}

impl GithubConfig {
    pub fn from_env() -> Result<Self, String> {
        let raw_target = env::var("TARGET_NUMBER")
            .ok()
            .or_else(|| env::var("PULL_REQUEST_NUMBER").ok());
        let target_number = match raw_target {
            None => None,
            Some(v) if v.trim().is_empty() => None,
            Some(v) => Some(v.trim().parse::<u64>().map_err(|_| {
                format!("TARGET_NUMBER/PULL_REQUEST_NUMBER must be a number, got: {v}")
            })?),
        };
        Ok(Self {
            repository: env::var("GITHUB_REPOSITORY")
                .map_err(|_| "GITHUB_REPOSITORY must be set")?,
            token: env::var("GITHUB_TOKEN").map_err(|_| "GITHUB_TOKEN must be set")?,
            target_type: TargetType::from_env(),
            target_number,
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
pub async fn fetch_pr_diff(
    repository: &str,
    pr_number: u64,
    token: &str,
) -> Result<String, String> {
    let octocrab = octocrab::Octocrab::builder()
        .personal_token(token.to_string())
        .build()
        .map_err(|e| e.to_string())?;

    let parts: Vec<&str> = repository.splitn(2, '/').collect();
    if parts.len() != 2 {
        return Err(format!("Invalid repository format: {}", repository));
    }
    let (owner, repo) = (parts[0], parts[1]);

    // list_files returns Page<DiffEntry>; all_pages follows Link headers.
    let first_page = octocrab
        .pulls(owner, repo)
        .list_files(pr_number)
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

/// Post the review text as a PR comment via the GitHub REST API.
pub async fn post_comment(
    review: &str,
    repo: &str,
    pr_number: u64,
    token: &str,
    client: &reqwest::Client,
) -> Result<(), String> {
    const MAX_LEN: usize = 60_000;
    let mut body = review.to_string();
    if body.len() > MAX_LEN {
        let cutoff = MAX_LEN - 50;
        let safe_cut = body
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|&i| i <= cutoff)
            .last()
            .unwrap_or(0);
        body.truncate(safe_cut);
        body.push_str("\n\n[Truncated due to GitHub comment size limit]");
    }

    let url = format!("https://api.github.com/repos/{repo}/issues/{pr_number}/comments");
    let resp = client
        .post(&url)
        .header("Authorization", format!("token {token}"))
        .header("User-Agent", "code-review-cli")
        .json(&serde_json::json!({ "body": body }))
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    if resp.status().is_success() {
        Ok(())
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(format!("GitHub API returned {status}: {body}"))
    }
}

/// GET a GitHub API URL as JSON with token auth.
async fn github_get_json(
    url: &str,
    token: &str,
    client: &reqwest::Client,
) -> Result<Value, String> {
    let resp = client
        .get(url)
        .header("Authorization", format!("token {token}"))
        .header("User-Agent", "code-review-cli")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("GitHub API returned {status}: {body}"));
    }

    resp.json().await.map_err(|e| format!("invalid JSON: {e}"))
}

/// Return the number of the first PR in a `GET /commits/{sha}/pulls` response.
fn first_pr_number(prs: &Value) -> Option<u64> {
    prs.as_array()
        .and_then(|a| a.first())
        .and_then(|p| p["number"].as_u64())
}

/// Resolve a PR number from a head commit SHA (used in `workflow_run` context).
pub async fn resolve_pr_number(
    repository: &str,
    head_sha: &str,
    token: &str,
    client: &reqwest::Client,
) -> Result<u64, String> {
    let url = format!("https://api.github.com/repos/{repository}/commits/{head_sha}/pulls");
    let prs = github_get_json(&url, token, client).await?;
    first_pr_number(&prs).ok_or_else(|| format!("No pull request found for commit {head_sha}"))
}

/// Fetch a PR's head commit SHA.
pub async fn resolve_head_sha(
    repository: &str,
    pr_number: u64,
    token: &str,
    client: &reqwest::Client,
) -> Result<String, String> {
    let url = format!("https://api.github.com/repos/{repository}/pulls/{pr_number}");
    let pr = github_get_json(&url, token, client).await?;
    pr["head"]["sha"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| format!("No head.sha for PR #{pr_number}"))
}

/// Render an issue + its comments into a single prompt-ready text block.
fn format_issue_context(issue: &Value, comments: &Value) -> String {
    let title = issue["title"].as_str().unwrap_or("").trim();
    let body = issue["body"].as_str().unwrap_or("").trim();
    let author = issue["user"]["login"].as_str().unwrap_or("unknown");
    let labels: Vec<&str> = issue["labels"]
        .as_array()
        .map(|a| a.iter().filter_map(|l| l["name"].as_str()).collect())
        .unwrap_or_default();

    let mut out = String::new();
    out.push_str(&format!("Title: {title}\n"));
    out.push_str(&format!("Author: {author}\n"));
    if !labels.is_empty() {
        out.push_str(&format!("Labels: {}\n", labels.join(", ")));
    }
    out.push_str("\nBody:\n");
    out.push_str(if body.is_empty() { "(empty)" } else { body });
    out.push('\n');

    if let Some(arr) = comments.as_array() {
        if !arr.is_empty() {
            out.push_str("\nComments:\n");
            for c in arr {
                let who = c["user"]["login"].as_str().unwrap_or("unknown");
                let text = c["body"].as_str().unwrap_or("").trim();
                out.push_str(&format!("\n--- {who} ---\n{text}\n"));
            }
        }
    }

    out
}

/// Fetch an issue's title, body, and comments via the GitHub API.
pub async fn fetch_issue_context(
    repository: &str,
    issue_number: u64,
    token: &str,
    client: &reqwest::Client,
) -> Result<String, String> {
    let issue_url = format!("https://api.github.com/repos/{repository}/issues/{issue_number}");
    let issue = github_get_json(&issue_url, token, client).await?;

    let comments_url = format!(
        "https://api.github.com/repos/{repository}/issues/{issue_number}/comments?per_page=100"
    );
    let comments = match github_get_json(&comments_url, token, client).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Warning: could not fetch issue comments ({e}); continuing without them.");
            Value::Array(Vec::new())
        }
    };

    Ok(format_issue_context(&issue, &comments))
}

/// Build the Contents API URL for a file path, percent-encoding each segment.
fn contents_url(repository: &str, path: &str, git_ref: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse("https://api.github.com").map_err(|e| e.to_string())?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "cannot build URL path segments".to_string())?;
        segments.push("repos");
        for part in repository.split('/') {
            segments.push(part);
        }
        segments.push("contents");
        for part in path.split('/') {
            segments.push(part);
        }
    }
    url.query_pairs_mut().append_pair("ref", git_ref);
    Ok(url)
}

/// Fetch modified source files at `git_ref` via the Contents API (raw bytes).
/// Rejects unsafe paths and truncates files exceeding `MAX_SOURCE_LINES`.
pub async fn fetch_source_files(
    repository: &str,
    git_ref: &str,
    paths: &[String],
    token: &str,
    client: &reqwest::Client,
) -> Vec<(String, String)> {
    let mut result = Vec::new();

    for path in paths {
        if !crate::source::is_safe_path(path) {
            eprintln!("Unsafe path rejected (skipping): {path}");
            continue;
        }

        let url = match contents_url(repository, path, git_ref) {
            Ok(u) => u,
            Err(e) => {
                eprintln!("Failed to build URL for {path}: {e}");
                continue;
            }
        };
        let resp = client
            .get(url)
            .header("Authorization", format!("token {token}"))
            .header("User-Agent", "code-review-cli")
            .header("Accept", "application/vnd.github.v3.raw")
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => match r.text().await {
                Ok(content) => {
                    let lines: Vec<&str> = content.lines().collect();
                    if lines.len() > crate::source::MAX_SOURCE_LINES {
                        let truncated = lines[..crate::source::MAX_SOURCE_LINES].join("\n");
                        result.push((
                            path.clone(),
                            format!(
                                "{truncated}\n\n[... truncated at {} lines, {} total ...]",
                                crate::source::MAX_SOURCE_LINES,
                                lines.len()
                            ),
                        ));
                    } else {
                        result.push((path.clone(), content));
                    }
                }
                Err(e) => eprintln!("Failed to read {path}: {e}"),
            },
            Ok(r) => eprintln!("Source fetch failed for {path}: HTTP {}", r.status()),
            Err(e) => eprintln!("Failed to fetch {path}: {e}"),
        }
    }

    result
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
        assert!(should_skip_file("Cargo.lock"));
        assert!(should_skip_file("package-lock.json"));
        assert!(should_skip_file("yarn.lock"));
        assert!(should_skip_file(".gitignore"));
        assert!(should_skip_file(".github/workflows/ci.yml"));
        assert!(should_skip_file("README.md"));
        assert!(should_skip_file("docs/guide.mdx"));
        assert!(!should_skip_file("src/main.rs"));
        assert!(!should_skip_file("frontend/src/components/ui/button.tsx"));
        assert!(!should_skip_file("Cargo.toml"));
    }

    #[test]
    fn first_pr_number_extracts_number() {
        let v = serde_json::json!([{ "number": 42, "title": "x" }]);
        assert_eq!(first_pr_number(&v), Some(42));
    }

    #[test]
    fn contents_url_encodes_path_segments() {
        let url = contents_url("owner/repo", "src/a b#c.rs", "deadbeef").unwrap();
        let s = url.as_str();
        assert!(s.starts_with("https://api.github.com/repos/owner/repo/contents/"));
        assert!(s.contains("src/a%20b%23c.rs"));
        assert!(s.ends_with("?ref=deadbeef"));
    }

    #[test]
    fn first_pr_number_empty_is_none() {
        let v = serde_json::json!([]);
        assert_eq!(first_pr_number(&v), None);
    }

    #[test]
    fn format_issue_context_includes_title_body_author_labels_comments() {
        let issue = serde_json::json!({
            "title": "Crash on startup",
            "body": "It crashes.",
            "user": { "login": "alice" },
            "labels": [{ "name": "bug" }]
        });
        let comments = serde_json::json!([
            { "user": { "login": "bob" }, "body": "I can reproduce." }
        ]);
        let ctx = format_issue_context(&issue, &comments);
        assert!(ctx.contains("Crash on startup"));
        assert!(ctx.contains("It crashes."));
        assert!(ctx.contains("alice"));
        assert!(ctx.contains("bug"));
        assert!(ctx.contains("bob"));
        assert!(ctx.contains("I can reproduce."));
    }

    #[test]
    fn format_issue_context_handles_empty_body_and_no_comments() {
        let issue = serde_json::json!({
            "title": "Feature request",
            "body": null,
            "user": { "login": "carol" },
            "labels": []
        });
        let comments = serde_json::json!([]);
        let ctx = format_issue_context(&issue, &comments);
        assert!(ctx.contains("Feature request"));
        assert!(ctx.contains("(empty)"));
    }

    #[test]
    #[serial]
    fn env_config_reads_vars() {
        unsafe {
            std::env::set_var("GITHUB_REPOSITORY", "owner/repo");
            std::env::set_var("TARGET_NUMBER", "42");
            std::env::set_var("GITHUB_TOKEN", "ghp_test");
            std::env::remove_var("TARGET_TYPE");
        }
        let cfg = GithubConfig::from_env().expect("env vars are set");
        assert_eq!(cfg.repository, "owner/repo");
        assert_eq!(cfg.target_number, Some(42u64));
        assert_eq!(cfg.token, "ghp_test");
        assert_eq!(cfg.target_type, TargetType::Pr);
        unsafe {
            std::env::remove_var("GITHUB_REPOSITORY");
            std::env::remove_var("TARGET_NUMBER");
            std::env::remove_var("GITHUB_TOKEN");
        }
    }

    #[test]
    #[serial]
    fn env_config_issue_type() {
        unsafe {
            std::env::set_var("GITHUB_REPOSITORY", "owner/repo");
            std::env::set_var("GITHUB_TOKEN", "ghp_test");
            std::env::set_var("TARGET_TYPE", "Issue");
        }
        let cfg = GithubConfig::from_env().expect("env vars are set");
        assert_eq!(cfg.target_type, TargetType::Issue);
        unsafe {
            std::env::remove_var("GITHUB_REPOSITORY");
            std::env::remove_var("GITHUB_TOKEN");
            std::env::remove_var("TARGET_TYPE");
        }
    }

    #[test]
    #[serial]
    fn env_config_rejects_non_numeric_target() {
        unsafe {
            std::env::set_var("GITHUB_REPOSITORY", "owner/repo");
            std::env::set_var("GITHUB_TOKEN", "ghp_test");
            std::env::set_var("TARGET_NUMBER", "abc");
            std::env::remove_var("TARGET_TYPE");
        }
        let result = GithubConfig::from_env();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must be a number"));
        unsafe {
            std::env::remove_var("GITHUB_REPOSITORY");
            std::env::remove_var("GITHUB_TOKEN");
            std::env::remove_var("TARGET_NUMBER");
        }
    }

    #[test]
    #[serial]
    fn env_config_returns_error_when_missing() {
        unsafe {
            std::env::remove_var("GITHUB_REPOSITORY");
            std::env::remove_var("GITHUB_TOKEN");
        }
        let result = GithubConfig::from_env();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("GITHUB_REPOSITORY"));
    }
}
