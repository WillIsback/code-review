use toolkit_core::config::Config;
use toolkit_core::vllm::{self, ChatMessage};

/// Construct the user prompt for a diff chunk review.
fn chunk_user_prompt(chunk: &str) -> String {
    format!(
        "Review this diff chunk. List findings only as bullets:\n\
         - CODE: `file:line` — <issue> [Critical|High|Medium|Low]\n\
         - SEC: `file:line` — <issue> [Critical|High|Medium|Low]\n\
         No headings, no prose, bullets only.\n\n```diff\n{chunk}\n```"
    )
}

/// Split a diff string into chunks capped at `max_words` words per chunk.
pub fn split_diff_into_chunks(diff: &str, max_words: usize) -> Vec<String> {
    if diff.trim().is_empty() {
        return vec![];
    }
    let lines: Vec<&str> = diff.lines().collect();
    let mut chunks = vec![];
    let mut current: Vec<&str> = vec![];
    let mut count = 0usize;

    for line in lines {
        let words = line.split_whitespace().count();
        if count + words > max_words && !current.is_empty() {
            chunks.push(current.join("\n"));
            current.clear();
            count = 0;
        }
        current.push(line);
        count += words;
    }
    if !current.is_empty() {
        chunks.push(current.join("\n"));
    }
    chunks
}

const SINGLE_ROUND_SYSTEM_PROMPT: &str =
    "You are a senior software engineer performing a pull request code review. \
     Be precise, actionable, and output only the requested format. No preamble, no conclusion.";

const SINGLE_ROUND_USER_TEMPLATE: &str = concat!(
    "Review this diff and output exactly this structure:\n\n",
    "---\n",
    "findings:\n",
    "  critical: <count>\n",
    "  high: <count>\n",
    "  medium: <count>\n",
    "  low: <count>\n",
    "top_files:\n",
    "  - <up to 3 files with most findings>\n",
    "risk_score: <critical|high|medium|low|none>\n",
    "---\n\n",
    "## 🔍 AI Code Review\n\n",
    "### 🛠 Code Quality Issues\n\n",
    "| # | Location | Issue | Severity |\n",
    "|---|----------|-------|----------|\n",
    "| 1 | `file:line` | Description | 🔴 Critical / 🟠 High / 🟡 Medium / 🟢 Low |\n\n",
    "### 🔒 Security Issues\n\n",
    "| # | Location | Issue | Risk Level |\n",
    "|---|----------|-------|------------|\n",
    "| 1 | `file:line` | Description | 🔴 Critical / 🟠 High / 🟡 Medium / 🟢 Low |\n\n",
    "Rules:\n",
    "- Sort rows: Critical first, then High, Medium, Low\n",
    "- If a section has no findings, write: | — | — | No issues found. | — |\n",
    "- Location always in backticks\n",
    "- top_files: files with most findings, max 3\n",
    "- risk_score: highest severity present; none if no findings\n\n",
    "Diff to review:\n```diff\n"
);

/// Review a diff using the appropriate strategy:
/// - 1 file  → single round (one direct `chat_complete` call)
/// - N files → two rounds (per-chunk bullets + reasoning summarization)
pub async fn review_diff(diff: &str, model: &str, cfg: &Config) -> Option<String> {
    if diff.trim().is_empty() {
        return None;
    }

    let file_count = diff.matches("\n\n# File:").count();
    println!("Files in diff: {file_count} — using {} strategy",
        if file_count <= 1 { "single-round" } else { "two-round" });

    if file_count <= 1 {
        single_round_review(diff, model, cfg).await
    } else {
        multi_round_review(diff, model, cfg).await
    }
}

async fn single_round_review(diff: &str, model: &str, cfg: &Config) -> Option<String> {
    let messages = vec![
        ChatMessage {
            role: "system",
            content: SINGLE_ROUND_SYSTEM_PROMPT.to_string(),
        },
        ChatMessage {
            role: "user",
            content: format!("{SINGLE_ROUND_USER_TEMPLATE}{diff}\n```"),
        },
    ];
    match vllm::chat_complete(&messages, model, 2048, 0.1, cfg).await {
        Ok(text) => Some(text),
        Err(e) => {
            eprintln!("Warning: single-round review failed: {e}");
            None
        }
    }
}

async fn multi_round_review(diff: &str, model: &str, cfg: &Config) -> Option<String> {
    let chunks = split_diff_into_chunks(diff, 2000);
    let mut reviews = vec![];

    for (i, chunk) in chunks.iter().enumerate() {
        println!("Reviewing chunk {}/{}...", i + 1, chunks.len());
        let messages = vec![
            ChatMessage {
                role: "system",
                content: SINGLE_ROUND_SYSTEM_PROMPT.to_string(),
            },
            ChatMessage {
                role: "user",
                content: chunk_user_prompt(chunk),
            },
        ];
        match vllm::chat_complete(&messages, model, 1024, 0.1, cfg).await {
            Ok(text) => reviews.push(text),
            Err(e) => {
                eprintln!("Warning: Chunk {} error: {e}", i + 1);
                reviews.push(format!("Chunk {}: error during analysis", i + 1));
            }
        }
    }

    if reviews.is_empty() {
        return None;
    }

    let valid_reviews: Vec<String> = reviews
        .iter()
        .filter(|r| !r.starts_with("Chunk ") || !r.contains(": error during analysis"))
        .cloned()
        .collect();

    match summarize_review(&valid_reviews, model, cfg).await {
        Some(summary) => Some(summary),
        None => {
            eprintln!("Warning: summarization failed, falling back to raw chunk output.");
            Some(format!(
                "> ⚠️ Summarization failed — raw chunk output below.\n\n{}",
                reviews.join("\n\n---\n\n")
            ))
        }
    }
}

const SUMMARIZE_SYSTEM_PROMPT: &str =
    "You are a senior software engineer performing a pull request code review. \
     Be precise, actionable, and output only the requested format. No preamble, no conclusion.";

/// Feed all chunk bullet outputs into an LLM call and return the
/// structured two-section Markdown summary.
pub async fn summarize_review(
    chunk_reviews: &[String],
    model: &str,
    cfg: &Config,
) -> Option<String> {
    if chunk_reviews.is_empty() {
        return None;
    }
    let combined = chunk_reviews.join("\n\n---\n\n");
    let messages = vec![
        ChatMessage {
            role: "system",
            content: SUMMARIZE_SYSTEM_PROMPT.to_string(),
        },
        ChatMessage {
            role: "user",
            content: format!(
                "Review these findings and output exactly this structure:\n\n\
                 ---\n\
                 findings:\n\
                   critical: <count>\n\
                   high: <count>\n\
                   medium: <count>\n\
                   low: <count>\n\
                 top_files:\n\
                   - <up to 3 files with most findings>\n\
                 risk_score: <critical|high|medium|low|none>\n\
                 ---\n\n\
                 ## 🔍 AI Code Review\n\n\
                 ### 🛠 Code Quality Issues\n\n\
                 | # | Location | Issue | Severity |\n\
                 |---|----------|-------|----------|\n\
                 | 1 | `file:line` | Description | 🔴 Critical / 🟠 High / 🟡 Medium / 🟢 Low |\n\n\
                 ### 🔒 Security Issues\n\n\
                 | # | Location | Issue | Risk Level |\n\
                 |---|----------|-------|------------|\n\
                 | 1 | `file:line` | Description | 🔴 Critical / 🟠 High / 🟡 Medium / 🟢 Low |\n\n\
                 Rules:\n\
                 - Sort rows: Critical first, then High, Medium, Low\n\
                 - Deduplicate similar findings across chunks\n\
                 - If a section has no findings, write: | — | — | No issues found. | — |\n\
                 - Location always in backticks\n\
                 - top_files: files with most findings, max 3\n\
                 - risk_score: highest severity present; none if no findings\n\n\
                 Findings collected from all diff chunks:\n\n{combined}",
            ),
        },
    ];
    match vllm::chat_complete(&messages, model, 2048, 0.2, cfg).await {
        Ok(text) => Some(text),
        Err(e) => {
            eprintln!("Warning: summarization failed: {e}");
            None
        }
    }
}

/// Post the review text as a PR comment via the GitHub REST API.
pub async fn post_pr_comment(
    review: &str,
    repo: &str,
    pr_number: u64,
    token: &str,
) -> bool {
    const MAX_LEN: usize = 60_000;
    let mut body = review.to_string();
    if body.len() > MAX_LEN {
        // Find the last valid UTF-8 char boundary at or before MAX_LEN - 50
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

    let client = reqwest::Client::new();
    let url = format!(
        "https://api.github.com/repos/{repo}/issues/{pr_number}/comments"
    );
    client
        .post(&url)
        .header("Authorization", format!("token {token}"))
        .header("User-Agent", "code-review-cli")
        .json(&serde_json::json!({ "body": body }))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_system_prompt_is_role_focused() {
        assert!(SUMMARIZE_SYSTEM_PROMPT.contains("senior software engineer"),
            "system prompt must establish engineer role");
        assert!(SUMMARIZE_SYSTEM_PROMPT.contains("pull request code review"),
            "system prompt must state the task");
        assert!(!SUMMARIZE_SYSTEM_PROMPT.contains("## Code Quality Issues"),
            "format instructions must be in the user message, not system prompt");
    }

    #[test]
    fn chunks_split_on_word_count() {
        // Build a diff with many lines so word count exceeds max_words=2000
        let big = (0..5000).map(|i| format!("+ word{i}")).collect::<Vec<_>>().join("\n");
        let chunks = split_diff_into_chunks(&big, 2000);
        assert!(chunks.len() > 1);
    }

    #[test]
    fn empty_diff_yields_no_chunks() {
        assert!(split_diff_into_chunks("", 2000).is_empty());
        assert!(split_diff_into_chunks("   \n  ", 2000).is_empty());
    }

    #[test]
    fn small_diff_is_single_chunk() {
        let diff = "- old line\n+ new line\n";
        let chunks = split_diff_into_chunks(diff, 2000);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], diff.trim_end_matches('\n'));
    }

    #[test]
    fn chunk_prompt_contains_bullet_instructions() {
        // The per-chunk prompt must ask for CODE: and SEC: bullets, not Markdown sections.
        let chunk = "- old line\n+ new line\n";
        let prompt = chunk_user_prompt(chunk);
        assert!(prompt.contains("CODE:"));
        assert!(prompt.contains("SEC:"));
        assert!(!prompt.contains("Issues, Improvements, Positives"));
    }

    #[test]
    fn single_round_prompt_contains_both_sections() {
        assert!(SINGLE_ROUND_USER_TEMPLATE.contains("Code Quality Issues"));
        assert!(SINGLE_ROUND_USER_TEMPLATE.contains("Security Issues"));
        assert!(SINGLE_ROUND_USER_TEMPLATE.contains("Severity"));
        assert!(SINGLE_ROUND_USER_TEMPLATE.contains("Risk Level"));
        assert!(SINGLE_ROUND_USER_TEMPLATE.contains("Sort rows: Critical first"));
    }

    #[test]
    fn multi_file_diff_detected() {
        let diff = "\n\n# File: a.rs\n+ foo\n\n# File: b.rs\n+ bar\n";
        assert_eq!(diff.matches("\n\n# File:").count(), 2);
    }

    #[test]
    fn single_file_diff_detected() {
        let diff = "\n\n# File: a.rs\n+ foo\n";
        assert_eq!(diff.matches("\n\n# File:").count(), 1);
    }

    #[test]
    fn single_round_template_contains_yaml_frontmatter() {
        assert!(SINGLE_ROUND_USER_TEMPLATE.contains("findings:"),
            "template must include YAML findings key");
        assert!(SINGLE_ROUND_USER_TEMPLATE.contains("risk_score:"),
            "template must include YAML risk_score key");
        assert!(SINGLE_ROUND_USER_TEMPLATE.contains("top_files:"),
            "template must include YAML top_files key");
    }

    #[test]
    fn single_round_template_contains_emoji_severity_badges() {
        assert!(SINGLE_ROUND_USER_TEMPLATE.contains("🔴 Critical"),
            "template must include red circle for Critical");
        assert!(SINGLE_ROUND_USER_TEMPLATE.contains("🟠 High"),
            "template must include orange circle for High");
        assert!(SINGLE_ROUND_USER_TEMPLATE.contains("🟡 Medium"),
            "template must include yellow circle for Medium");
        assert!(SINGLE_ROUND_USER_TEMPLATE.contains("🟢 Low"),
            "template must include green circle for Low");
    }

    #[test]
    fn single_round_template_contains_pipe_table_syntax() {
        assert!(SINGLE_ROUND_USER_TEMPLATE.contains("| # | Location |"),
            "template must include pipe-syntax table header");
        assert!(SINGLE_ROUND_USER_TEMPLATE.contains("|---|"),
            "template must include pipe-syntax table separator");
    }

    #[test]
    fn system_prompts_are_role_focused_not_format_heavy() {
        for prompt in &[SINGLE_ROUND_SYSTEM_PROMPT, SUMMARIZE_SYSTEM_PROMPT] {
            assert!(prompt.contains("senior software engineer"),
                "system prompt must establish senior engineer role");
            assert!(prompt.contains("pull request code review"),
                "system prompt must state the task");
            assert!(!prompt.contains("markdown table"),
                "format instructions belong in user prompts, not system prompts");
        }
    }
}
