use crate::config::Config;
use crate::vllm::{self, ChatMessage};

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
    "You are a senior software engineer performing a thorough pull request code review. \
     Your goal is to help the author ship better code by providing detailed, actionable feedback. \
     For each finding, explain WHY it matters and HOW to fix it. \
     Be constructive: acknowledge good patterns alongside issues. \
     Output the requested format.";

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
    "### 📋 Summary\n\n",
    "Write 2-4 sentences summarizing the overall changes, their purpose, and the general code quality.\n\n",
    "### 🛠 Code Quality Issues\n\n",
    "| # | Location | Issue | Severity |\n",
    "|---|----------|-------|----------|\n",
    "| 1 | `file:line` | Description | 🔴 Critical / 🟠 High / 🟡 Medium / 🟢 Low |\n\n",
    "For each issue above, add a detail block:\n\n",
    "#### Issue N: <short title>\n",
    "**Why it matters:** Explain the impact (bug risk, performance, maintainability).\n",
    "**Suggestion:** Provide a concrete fix or improvement, with a short code snippet if helpful.\n\n",
    "### 🔒 Security Issues\n\n",
    "| # | Location | Issue | Risk Level |\n",
    "|---|----------|-------|------------|\n",
    "| 1 | `file:line` | Description | 🔴 Critical / 🟠 High / 🟡 Medium / 🟢 Low |\n\n",
    "For each security issue above, add a detail block:\n\n",
    "#### Security Issue N: <short title>\n",
    "**Risk:** Describe the attack vector or vulnerability.\n",
    "**Remediation:** Provide a concrete fix with code snippet if applicable.\n\n",
    "### ✅ What Looks Good\n\n",
    "List 2-3 positive aspects of the code (good patterns, clean structure, proper error handling, etc.).\n\n",
    "Rules:\n",
    "- Sort table rows: Critical first, then High, Medium, Low\n",
    "- EVERY table row MUST have a matching detail block below its table\n",
    "- If a section has no findings, write: | — | — | No issues found. | — |\n",
    "- Location always in backticks\n",
    "- top_files: files with most findings, max 3\n",
    "- risk_score: highest severity present; none if no findings\n",
    "- Be specific: reference actual variable names, function names, and line numbers from the diff\n",
    "- Source files are provided after the diff for verification context only. Do NOT review unchanged code in source files.\n",
    "- Use source files to verify whether issues in the diff are real — reference specific line numbers when confirming issues.\n\n",
    "Diff to review:\n```diff\n"
);

/// Review a diff using the appropriate strategy:
/// - 1 file  → single round (one direct `chat_complete` call)
/// - N files → two rounds (per-chunk bullets + reasoning summarization)
pub async fn review_diff(
    diff: &str,
    model: &str,
    client: &reqwest::Client,
    cfg: &Config,
) -> Option<String> {
    if diff.trim().is_empty() {
        return None;
    }

    let file_count = diff.matches("\n\n# File:").count();
    println!(
        "Files in diff: {file_count} — using {} strategy",
        if file_count <= 1 {
            "single-round"
        } else {
            "two-round"
        }
    );

    if file_count <= 1 {
        single_round_review(diff, model, client, cfg).await
    } else {
        multi_round_review(diff, model, client, cfg).await
    }
}

async fn single_round_review(
    diff: &str,
    model: &str,
    client: &reqwest::Client,
    cfg: &Config,
) -> Option<String> {
    // Load source files for context
    let file_paths = crate::source::extract_modified_files(diff);
    let source_files = crate::source::read_source_files(&file_paths);
    let source_context =
        crate::source::build_context_with_budget(&source_files, cfg.review_max_context);

    let messages = vec![
        ChatMessage {
            role: "system",
            content: SINGLE_ROUND_SYSTEM_PROMPT.to_string(),
        },
        ChatMessage {
            role: "user",
            content: format!("{SINGLE_ROUND_USER_TEMPLATE}{diff}\n```{source_context}"),
        },
    ];
    match vllm::chat_complete(&messages, model, 4096, 0.2, client, cfg).await {
        Ok(text) => Some(text),
        Err(e) => {
            eprintln!("Warning: single-round review failed: {e}");
            None
        }
    }
}

async fn multi_round_review(
    diff: &str,
    model: &str,
    client: &reqwest::Client,
    cfg: &Config,
) -> Option<String> {
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
        match vllm::chat_complete(&messages, model, 2048, 0.1, client, cfg).await {
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

    match summarize_review(&valid_reviews, model, client, cfg).await {
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
    "You are a senior software engineer performing a thorough pull request code review. \
     Your goal is to help the author ship better code by providing detailed, actionable feedback. \
     For each finding, explain WHY it matters and HOW to fix it. \
     Be constructive: acknowledge good patterns alongside issues. \
     Output the requested format.";

const SUMMARIZE_USER_TEMPLATE: &str = concat!(
    "Review these findings and output exactly this structure:\n\n",
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
    "### 📋 Summary\n\n",
    "Write 2-4 sentences summarizing the overall changes, their purpose, and the general code quality.\n\n",
    "### 🛠 Code Quality Issues\n\n",
    "| # | Location | Issue | Severity |\n",
    "|---|----------|-------|----------|\n",
    "| 1 | `file:line` | Description | 🔴 Critical / 🟠 High / 🟡 Medium / 🟢 Low |\n\n",
    "For each issue above, add a detail block:\n\n",
    "#### Issue N: <short title>\n",
    "**Why it matters:** Explain the impact (bug risk, performance, maintainability).\n",
    "**Suggestion:** Provide a concrete fix or improvement, with a short code snippet if helpful.\n\n",
    "### 🔒 Security Issues\n\n",
    "| # | Location | Issue | Risk Level |\n",
    "|---|----------|-------|------------|\n",
    "| 1 | `file:line` | Description | 🔴 Critical / 🟠 High / 🟡 Medium / 🟢 Low |\n\n",
    "For each security issue above, add a detail block:\n\n",
    "#### Security Issue N: <short title>\n",
    "**Risk:** Describe the attack vector or vulnerability.\n",
    "**Remediation:** Provide a concrete fix with code snippet if applicable.\n\n",
    "### ✅ What Looks Good\n\n",
    "List 2-3 positive aspects of the code (good patterns, clean structure, proper error handling, etc.).\n\n",
    "Rules:\n",
    "- Sort table rows: Critical first, then High, Medium, Low\n",
    "- EVERY table row MUST have a matching detail block below its table\n",
    "- Deduplicate similar findings across chunks\n",
    "- If a section has no findings, write: | — | — | No issues found. | — |\n",
    "- Location always in backticks\n",
    "- top_files: files with most findings, max 3\n",
    "- risk_score: highest severity present; none if no findings\n",
    "- Be specific: reference actual variable names, function names, and line numbers from the diff\n",
    "- Source files are provided after the diff for verification context only. Do NOT review unchanged code in source files.\n",
    "- Use source files to verify whether issues in the diff are real — reference specific line numbers when confirming issues.\n\n",
    "Findings collected from all diff chunks:\n\n"
);

/// Feed all chunk bullet outputs into an LLM call and return the
/// structured two-section Markdown summary.
pub async fn summarize_review(
    chunk_reviews: &[String],
    model: &str,
    client: &reqwest::Client,
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
            content: format!("{SUMMARIZE_USER_TEMPLATE}{combined}"),
        },
    ];
    match vllm::chat_complete(&messages, model, 4096, 0.3, client, cfg).await {
        Ok(text) => Some(text),
        Err(e) => {
            eprintln!("Warning: summarization failed: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_system_prompt_is_role_focused() {
        assert!(
            SUMMARIZE_SYSTEM_PROMPT.contains("senior software engineer"),
            "system prompt must establish engineer role"
        );
        assert!(
            SUMMARIZE_SYSTEM_PROMPT.contains("pull request code review"),
            "system prompt must state the task"
        );
        assert!(
            SUMMARIZE_SYSTEM_PROMPT.contains("actionable feedback"),
            "system prompt must encourage detailed feedback"
        );
        assert!(
            !SUMMARIZE_SYSTEM_PROMPT.contains("## Code Quality Issues"),
            "format instructions must be in the user message, not system prompt"
        );
    }

    #[test]
    fn chunks_split_on_word_count() {
        // Build a diff with many lines so word count exceeds max_words=2000
        let big = (0..5000)
            .map(|i| format!("+ word{i}"))
            .collect::<Vec<_>>()
            .join("\n");
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
        assert!(SINGLE_ROUND_USER_TEMPLATE.contains("Sort table rows: Critical first"));
    }

    #[test]
    fn single_round_template_contains_detail_sections() {
        assert!(
            SINGLE_ROUND_USER_TEMPLATE.contains("Summary"),
            "template must include a Summary section"
        );
        assert!(
            SINGLE_ROUND_USER_TEMPLATE.contains("Why it matters:"),
            "template must ask for impact explanation"
        );
        assert!(
            SINGLE_ROUND_USER_TEMPLATE.contains("Suggestion:"),
            "template must ask for actionable suggestions"
        );
        assert!(
            SINGLE_ROUND_USER_TEMPLATE.contains("What Looks Good"),
            "template must include positive highlights section"
        );
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
        assert!(
            SINGLE_ROUND_USER_TEMPLATE.contains("findings:"),
            "template must include YAML findings key"
        );
        assert!(
            SINGLE_ROUND_USER_TEMPLATE.contains("risk_score:"),
            "template must include YAML risk_score key"
        );
        assert!(
            SINGLE_ROUND_USER_TEMPLATE.contains("top_files:"),
            "template must include YAML top_files key"
        );
    }

    #[test]
    fn single_round_template_contains_emoji_severity_badges() {
        assert!(
            SINGLE_ROUND_USER_TEMPLATE.contains("🔴 Critical"),
            "template must include red circle for Critical"
        );
        assert!(
            SINGLE_ROUND_USER_TEMPLATE.contains("🟠 High"),
            "template must include orange circle for High"
        );
        assert!(
            SINGLE_ROUND_USER_TEMPLATE.contains("🟡 Medium"),
            "template must include yellow circle for Medium"
        );
        assert!(
            SINGLE_ROUND_USER_TEMPLATE.contains("🟢 Low"),
            "template must include green circle for Low"
        );
    }

    #[test]
    fn single_round_template_contains_pipe_table_syntax() {
        assert!(
            SINGLE_ROUND_USER_TEMPLATE.contains("| # | Location |"),
            "template must include pipe-syntax table header"
        );
        assert!(
            SINGLE_ROUND_USER_TEMPLATE.contains("|---|"),
            "template must include pipe-syntax table separator"
        );
    }

    #[test]
    fn system_prompts_are_role_focused_not_format_heavy() {
        for prompt in &[SINGLE_ROUND_SYSTEM_PROMPT, SUMMARIZE_SYSTEM_PROMPT] {
            assert!(
                prompt.contains("senior software engineer"),
                "system prompt must establish senior engineer role"
            );
            assert!(
                prompt.contains("pull request code review"),
                "system prompt must state the task"
            );
            assert!(
                prompt.contains("actionable feedback"),
                "system prompt must encourage detailed feedback"
            );
            assert!(
                !prompt.contains("markdown table"),
                "format instructions belong in user prompts, not system prompts"
            );
        }
    }

    #[test]
    fn summarize_template_contains_yaml_frontmatter() {
        assert!(
            SUMMARIZE_USER_TEMPLATE.contains("findings:"),
            "summarize template must include YAML findings key"
        );
        assert!(
            SUMMARIZE_USER_TEMPLATE.contains("risk_score:"),
            "summarize template must include YAML risk_score key"
        );
        assert!(
            SUMMARIZE_USER_TEMPLATE.contains("top_files:"),
            "summarize template must include YAML top_files key"
        );
    }

    #[test]
    fn summarize_template_contains_emoji_severity_badges() {
        assert!(
            SUMMARIZE_USER_TEMPLATE.contains("🔴 Critical"),
            "summarize template must include red circle for Critical"
        );
        assert!(
            SUMMARIZE_USER_TEMPLATE.contains("🟠 High"),
            "summarize template must include orange circle for High"
        );
        assert!(
            SUMMARIZE_USER_TEMPLATE.contains("🟡 Medium"),
            "summarize template must include yellow circle for Medium"
        );
        assert!(
            SUMMARIZE_USER_TEMPLATE.contains("🟢 Low"),
            "summarize template must include green circle for Low"
        );
    }

    #[test]
    fn summarize_template_has_deduplication_rule() {
        assert!(
            SUMMARIZE_USER_TEMPLATE.contains("Deduplicate similar findings across chunks"),
            "summarize template must instruct LLM to deduplicate"
        );
        assert!(
            SUMMARIZE_USER_TEMPLATE.contains("Findings collected from all diff chunks:"),
            "summarize template must end with the findings section header"
        );
    }

    #[test]
    fn summarize_template_contains_detail_sections() {
        assert!(
            SUMMARIZE_USER_TEMPLATE.contains("Summary"),
            "summarize template must include a Summary section"
        );
        assert!(
            SUMMARIZE_USER_TEMPLATE.contains("Why it matters:"),
            "summarize template must ask for impact explanation"
        );
        assert!(
            SUMMARIZE_USER_TEMPLATE.contains("Suggestion:"),
            "summarize template must ask for actionable suggestions"
        );
        assert!(
            SUMMARIZE_USER_TEMPLATE.contains("What Looks Good"),
            "summarize template must include positive highlights section"
        );
        assert!(
            SUMMARIZE_USER_TEMPLATE.contains("Remediation:"),
            "summarize template must ask for security remediation"
        );
    }

    #[test]
    fn templates_require_detail_blocks_for_every_finding() {
        assert!(
            SINGLE_ROUND_USER_TEMPLATE
                .contains("EVERY table row MUST have a matching detail block"),
            "single-round template must enforce detail blocks"
        );
        assert!(
            SUMMARIZE_USER_TEMPLATE.contains("EVERY table row MUST have a matching detail block"),
            "summarize template must enforce detail blocks"
        );
    }
}
