use toolkit_core::config::Config;
use toolkit_core::vllm::{self, ChatMessage};

/// Construct the user prompt for a diff chunk review.
fn chunk_user_prompt(chunk: &str) -> String {
    format!(
        "Review this diff chunk. List only:\n- CODE: <file>:<line> - <issue>\n- SEC: <file>:<line> - <issue>\nNo headings, no prose, bullets only.\n\n```diff\n{chunk}\n```"
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

/// Send each diff chunk to vLLM and return concatenated Markdown review.
pub async fn review_diff(diff: &str, model: &str, cfg: &Config) -> Option<String> {
    if diff.trim().is_empty() {
        return None;
    }

    let chunks = split_diff_into_chunks(diff, 2000);
    let mut reviews = vec![];

    for (i, chunk) in chunks.iter().enumerate() {
        println!("Reviewing chunk {}/{}...", i + 1, chunks.len());
        let messages = vec![
            ChatMessage {
                role: "system",
                content: "You are a code reviewer. Be brief, practical, and output final answer only.".to_string(),
            },
            ChatMessage {
                role: "user",
                content: chunk_user_prompt(chunk),
            },
        ];

        match vllm::chat_complete(&messages, model, 1024, 0.1, cfg).await {
            Ok(text) => {
                reviews.push(text);
            }
            Err(e) => {
                eprintln!("Warning: Chunk {} error: {e}", i + 1);
                reviews.push(format!("Chunk {}: error during analysis", i + 1));
            }
        }
    }

    if reviews.is_empty() {
        return None;
    }

    // Reasoning summarization pass — falls back to raw concat on failure.
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

const SUMMARIZE_SYSTEM_PROMPT: &str = concat!(
    "You are a senior code reviewer. Given a list of raw review bullets ",
    "collected from multiple diff chunks, produce a single concise review with exactly ",
    "two sections:\n",
    "## Code Quality Issues\n",
    "A markdown table with columns: #, Location, Issue, Severity. ",
    "Severity values: Critical, High, Medium, Low. ",
    "Sort rows Critical first, then High, Medium, Low. ",
    "Deduplicate similar findings.\n\n",
    "## Security Issues\n",
    "A markdown table with columns: #, Location, Issue, Risk Level. ",
    "Risk Level values: Critical, High, Medium, Low. ",
    "Sort rows Critical first, then High, Medium, Low. ",
    "Deduplicate similar findings.\n\n",
    "If a section has no findings, write 'No issues found.' under the heading. ",
    "Output only the two sections. No preamble, no conclusion."
);

/// Feed all chunk bullet outputs into a reasoning LLM call and return the
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
            content: format!("Here are the raw review bullets:\n\n{combined}"),
        },
    ];
    match vllm::chat_complete_with_reasoning(&messages, model, 8192, 0.7, cfg).await {
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
    let source_label = std::env::var("REVIEW_ENGINE_LABEL")
        .unwrap_or_else(|_| "self-hosted AI reviewer".to_string());
    let mut body = format!(
        "## AI Code Review\n\n{review}\n\n---\n*This review was generated by {source_label}.*"
    );
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
    fn summarize_prompt_contains_both_sections() {
        assert!(SUMMARIZE_SYSTEM_PROMPT.contains("## Code Quality Issues"));
        assert!(SUMMARIZE_SYSTEM_PROMPT.contains("## Security Issues"));
        assert!(SUMMARIZE_SYSTEM_PROMPT.contains("Severity"));
        assert!(SUMMARIZE_SYSTEM_PROMPT.contains("Risk Level"));
        assert!(SUMMARIZE_SYSTEM_PROMPT.contains("Sort rows Critical first"));
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
    fn review_diff_calls_summarize_signature() {
        // Verify the function signature via a helper that constrains parameter types.
        // This will fail to compile if the parameter types change.
        async fn _type_check(chunks: &[String], model: &str, cfg: &Config) {
            let _ = summarize_review(chunks, model, cfg).await;
        }
    }
}
