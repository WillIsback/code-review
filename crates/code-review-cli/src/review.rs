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

/// Split a diff string into per-file sections.
/// Returns Vec<(filename, file_diff)> parsed from `# File:` headers.
pub fn split_diff_by_file(diff: &str) -> Vec<(String, String)> {
    if diff.trim().is_empty() {
        return vec![];
    }

    let mut results: Vec<(String, String)> = Vec::new();
    let marker = "# File: ";

    // Find all positions of "# File: " in the diff
    let mut positions: Vec<usize> = Vec::new();
    let mut search_from = 0;
    while let Some(pos) = diff[search_from..].find(marker) {
        positions.push(search_from + pos);
        search_from = search_from + pos + marker.len();
    }

    if positions.is_empty() {
        return vec![];
    }

    for (idx, &start) in positions.iter().enumerate() {
        let header_start = start + marker.len();
        let line_end = diff[header_start..]
            .find('\n')
            .map(|p| header_start + p)
            .unwrap_or(diff.len());
        let filename = diff[header_start..line_end].trim().to_string();

        let body_start = line_end;
        let body_end = if idx + 1 < positions.len() {
            positions[idx + 1]
        } else {
            diff.len()
        };
        let body = diff[body_start..body_end].to_string();

        results.push((filename, body));
    }

    results
}

/// Group per-file diffs into chunks that respect file boundaries.
/// No file is ever split across chunks. Files exceeding max_words go solo.
pub fn group_files_into_chunks(files: &[(String, String)], max_words: usize) -> Vec<String> {
    if files.is_empty() {
        return vec![];
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut current_chunk = String::new();
    let mut current_words = 0usize;

    for (name, body) in files {
        let file_text = format!("# File: {name}\n{body}");
        let file_words = file_text.split_whitespace().count();

        if current_words > 0 && current_words + file_words > max_words {
            chunks.push(current_chunk.trim().to_string());
            current_chunk = String::new();
            current_words = 0;
        }

        if !current_chunk.is_empty() {
            current_chunk.push_str("\n\n");
        }
        current_chunk.push_str(&file_text);
        current_words += file_words;
    }

    if !current_chunk.is_empty() {
        chunks.push(current_chunk.trim().to_string());
    }

    chunks
}

const SINGLE_ROUND_SYSTEM_PROMPT: &str =
    "You are a senior software engineer performing a thorough pull request code review. \
     Your goal is to help the author ship better code by providing detailed, actionable feedback. \
     For each finding, explain WHY it matters and HOW to fix it. \
     Be constructive: acknowledge good patterns alongside issues. \
     Output the requested format. \
     Only report issues you can directly verify from the provided code. Do NOT speculate about behavior in code you cannot see. \
     Do NOT invent API features, language semantics, or framework behaviors you are not certain about. \
     If the code is correct and well-written, reporting fewer or zero findings is better than inflating issues.";

const SINGLE_ROUND_USER_TEMPLATE: &str = concat!(
    "Review this diff and output exactly this structure:\n\n",
    "---\n",
    "findings_total: <count or 0 if no issues found>\n",
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
    "| <n> | `file:line` | Description | 🔴 Critical / 🟠 High / 🟡 Medium / 🟢 Low |\n\n",
    "For each issue above, add a detail block:\n\n",
    "#### Issue N: <short title>\n",
    "**Why it matters:** Explain the impact (bug risk, performance, maintainability).\n",
    "**Suggestion:** Provide a concrete fix or improvement, with a short code snippet if helpful.\n\n",
    "### 🔒 Security Issues\n\n",
    "| # | Location | Issue | Risk Level |\n",
    "|---|----------|-------|------------|\n",
    "| <n> | `file:line` | Description | 🔴 Critical / 🟠 High / 🟡 Medium / 🟢 Low |\n\n",
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
    "- An empty findings table is a valid and positive outcome when code quality is good.\n",
    "- Do NOT report issues based on assumptions about external APIs, frameworks, or language features you cannot verify from the diff.\n",
    "- Prefer fewer high-confidence findings over many speculative ones.\n\n",
    "Diff to review:\n```diff\n"
);

/// Review a diff using the appropriate strategy:
/// - 1 file  -> single round (one direct `chat_complete` call)
/// - N files -> two rounds (per-chunk bullets + verification against source)
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
    let files = split_diff_by_file(diff);
    let chunks = if files.is_empty() {
        split_diff_into_chunks(diff, 2000) // fallback for non-standard diff format
    } else {
        group_files_into_chunks(&files, 2000)
    };
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

    // Round 2: verify findings against source code, fallback to summarize
    let verified = verify_findings(&valid_reviews, diff, model, client, cfg).await;
    match verified {
        Some(report) => Some(report),
        None => {
            // Fallback to summarization if verification fails
            match summarize_review(&valid_reviews, model, client, cfg).await {
                Some(summary) => Some(summary),
                None => {
                    eprintln!("Warning: summarization also failed, falling back to raw output.");
                    Some(format!(
                        "> ⚠️ Verification and summarization failed — raw chunk output below.\n\n{}",
                        reviews.join("\n\n---\n\n")
                    ))
                }
            }
        }
    }
}

const VERIFY_SYSTEM_PROMPT: &str =
    "You are a senior software engineer verifying code review findings against actual source code. \
     Your job is to FILTER, not to ADD. For each finding, determine if it is correct based on the \
     evidence in the source code provided. Be skeptical of findings that make claims about APIs, \
     frameworks, or language features that cannot be verified from the code. \
     Reject any finding that is speculative or factually incorrect.";

const VERIFY_USER_TEMPLATE: &str = concat!(
    "Verify each finding below against the source code provided.\n\n",
    "For each finding, classify as:\n",
    "- CONFIRMED -- the issue is real and verifiable in the source code\n",
    "- REJECTED -- the issue is incorrect, speculative, or based on wrong assumptions\n",
    "- DOWNGRADED -- the issue exists but the severity is inflated (specify correct severity)\n\n",
    "Then produce the final report containing ONLY confirmed (and downgraded) findings,\n",
    "using this exact structure:\n\n",
    "---\n",
    "findings_total: <count of confirmed + downgraded findings, or 0>\n",
    "top_files:\n",
    "  - <up to 3 files with most confirmed findings>\n",
    "risk_score: <critical|high|medium|low|none>\n",
    "---\n\n",
    "## 🔍 AI Code Review\n\n",
    "### 📋 Summary\n",
    "Write 2-4 sentences summarizing the changes and verified findings.\n\n",
    "### 🛠 Code Quality Issues\n",
    "| # | Location | Issue | Severity |\n",
    "|---|----------|-------|----------|\n",
    "(only confirmed/downgraded findings)\n\n",
    "For each confirmed issue:\n",
    "#### Issue N: <title>\n",
    "**Verified:** Explain what in the source code confirms this issue.\n",
    "**Suggestion:** Concrete fix.\n\n",
    "### 🔒 Security Issues\n",
    "| # | Location | Issue | Risk Level |\n",
    "|---|----------|-------|------------|\n",
    "(only confirmed/downgraded findings)\n\n",
    "### ✅ What Looks Good\n",
    "List 2-3 positive aspects.\n\n",
    "Rules:\n",
    "- Do NOT add new findings -- only verify the ones provided\n",
    "- REJECTED findings must NOT appear in the final report\n",
    "- An empty table is a valid outcome if all findings were rejected\n",
    "- Sort confirmed findings: Critical first, then High, Medium, Low\n\n",
    "[FINDINGS TO VERIFY]\n\n",
);

const SUMMARIZE_SYSTEM_PROMPT: &str =
    "You are a senior software engineer performing a thorough pull request code review. \
     Your goal is to help the author ship better code by providing detailed, actionable feedback. \
     For each finding, explain WHY it matters and HOW to fix it. \
     Be constructive: acknowledge good patterns alongside issues. \
     Output the requested format. \
     Only report issues you can directly verify from the provided code. Do NOT speculate about behavior in code you cannot see. \
     Do NOT invent API features, language semantics, or framework behaviors you are not certain about. \
     If the code is correct and well-written, reporting fewer or zero findings is better than inflating issues.";

const SUMMARIZE_USER_TEMPLATE: &str = concat!(
    "Review these findings and output exactly this structure:\n\n",
    "---\n",
    "findings_total: <count or 0 if no issues found>\n",
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
    "| <n> | `file:line` | Description | 🔴 Critical / 🟠 High / 🟡 Medium / 🟢 Low |\n\n",
    "For each issue above, add a detail block:\n\n",
    "#### Issue N: <short title>\n",
    "**Why it matters:** Explain the impact (bug risk, performance, maintainability).\n",
    "**Suggestion:** Provide a concrete fix or improvement, with a short code snippet if helpful.\n\n",
    "### 🔒 Security Issues\n\n",
    "| # | Location | Issue | Risk Level |\n",
    "|---|----------|-------|------------|\n",
    "| <n> | `file:line` | Description | 🔴 Critical / 🟠 High / 🟡 Medium / 🟢 Low |\n\n",
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
    "- An empty findings table is a valid and positive outcome when code quality is good.\n",
    "- Do NOT report issues based on assumptions about external APIs, frameworks, or language features you cannot verify from the diff.\n",
    "- Prefer fewer high-confidence findings over many speculative ones.\n\n",
    "Findings collected from all diff chunks:\n\n"
);

/// Verify Round 1 findings against actual source code.
/// Returns the verified review report, or None on failure.
pub async fn verify_findings(
    chunk_reviews: &[String],
    diff: &str,
    model: &str,
    client: &reqwest::Client,
    cfg: &Config,
) -> Option<String> {
    if chunk_reviews.is_empty() {
        return None;
    }

    // Read source files from filesystem
    let file_paths = crate::source::extract_modified_files(diff);
    let source_files = crate::source::read_source_files(&file_paths);

    if source_files.is_empty() {
        eprintln!("No source files readable, falling back to summarization.");
        return None; // caller will fall back to summarize_review()
    }

    let source_context = crate::source::format_source_context(&source_files);
    let combined_findings = chunk_reviews.join("\n\n---\n\n");

    let messages = vec![
        ChatMessage {
            role: "system",
            content: VERIFY_SYSTEM_PROMPT.to_string(),
        },
        ChatMessage {
            role: "user",
            content: format!("{VERIFY_USER_TEMPLATE}{combined_findings}{source_context}"),
        },
    ];

    println!(
        "Verifying {} findings against {} source files...",
        chunk_reviews.len(),
        source_files.len()
    );

    match vllm::chat_complete(&messages, model, 4096, 0.1, client, cfg).await {
        Ok(text) => Some(text),
        Err(e) => {
            eprintln!("Warning: verification failed: {e}");
            None // caller will fall back to summarize_review()
        }
    }
}

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
    match vllm::chat_complete(&messages, model, 4096, 0.1, client, cfg).await {
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
            SINGLE_ROUND_USER_TEMPLATE.contains("findings_total:"),
            "template must include YAML findings_total key"
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
            SUMMARIZE_USER_TEMPLATE.contains("findings_total:"),
            "summarize template must include YAML findings_total key"
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

    #[test]
    fn system_prompts_contain_factual_guards() {
        for prompt in &[SINGLE_ROUND_SYSTEM_PROMPT, SUMMARIZE_SYSTEM_PROMPT] {
            assert!(
                prompt
                    .contains("Only report issues you can directly verify from the provided code"),
                "system prompt must require verifiable findings"
            );
            assert!(
                prompt.contains("Do NOT speculate about behavior in code you cannot see"),
                "system prompt must forbid speculation"
            );
            assert!(
                prompt.contains(
                    "Do NOT invent API features, language semantics, or framework behaviors"
                ),
                "system prompt must forbid inventing behaviors"
            );
            assert!(
                prompt.contains("reporting fewer or zero findings is better than inflating issues"),
                "system prompt must discourage inflating issues"
            );
        }
    }

    #[test]
    fn user_templates_contain_anti_filling_rules() {
        for template in &[SINGLE_ROUND_USER_TEMPLATE, SUMMARIZE_USER_TEMPLATE] {
            assert!(
                template.contains("An empty findings table is a valid and positive outcome"),
                "template must allow empty findings"
            );
            assert!(
                template.contains("Do NOT report issues based on assumptions about external APIs"),
                "template must forbid assumption-based findings"
            );
            assert!(
                template
                    .contains("Prefer fewer high-confidence findings over many speculative ones"),
                "template must prefer quality over quantity"
            );
        }
    }

    #[test]
    fn split_by_file_parses_headers() {
        let diff = "\n\n# File: src/a.rs\n+ line a\n\n# File: src/b.rs\n+ line b\n";
        let files = split_diff_by_file(diff);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].0, "src/a.rs");
        assert_eq!(files[1].0, "src/b.rs");
        assert!(files[0].1.contains("+ line a"));
        assert!(files[1].1.contains("+ line b"));
    }

    #[test]
    fn split_by_file_handles_empty() {
        assert!(split_diff_by_file("").is_empty());
        assert!(split_diff_by_file("   ").is_empty());
    }

    #[test]
    fn split_by_file_single_file() {
        let diff = "\n\n# File: src/main.rs\n+ hello\n+ world\n";
        let files = split_diff_by_file(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "src/main.rs");
    }

    #[test]
    fn group_files_respects_boundaries() {
        let files = vec![
            ("a.rs".to_string(), "word ".repeat(800)),
            ("b.rs".to_string(), "word ".repeat(800)),
            ("c.rs".to_string(), "word ".repeat(800)),
        ];
        let chunks = group_files_into_chunks(&files, 2000);
        // a.rs + b.rs = 1600 words fits in one chunk
        // c.rs would make 2400, so c.rs goes to chunk 2
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].contains("# File: a.rs"));
        assert!(chunks[0].contains("# File: b.rs"));
        assert!(chunks[1].contains("# File: c.rs"));
    }

    #[test]
    fn group_files_large_single_file_solo() {
        let files = vec![("big.rs".to_string(), "word ".repeat(3000))];
        let chunks = group_files_into_chunks(&files, 2000);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("# File: big.rs"));
    }

    #[test]
    fn verify_system_prompt_is_skeptical() {
        assert!(VERIFY_SYSTEM_PROMPT.contains("FILTER"));
        assert!(VERIFY_SYSTEM_PROMPT.contains("skeptical"));
        assert!(!VERIFY_SYSTEM_PROMPT.contains("thorough"));
    }

    #[test]
    fn verify_template_has_classification() {
        assert!(VERIFY_USER_TEMPLATE.contains("CONFIRMED"));
        assert!(VERIFY_USER_TEMPLATE.contains("REJECTED"));
        assert!(VERIFY_USER_TEMPLATE.contains("DOWNGRADED"));
        assert!(VERIFY_USER_TEMPLATE.contains("[FINDINGS TO VERIFY]"));
    }
}
