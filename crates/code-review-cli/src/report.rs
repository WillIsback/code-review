//! Deterministic post-processing of LLM review output.
//!
//! The two-round strategy asks the model for a verification pass followed by a
//! final report. Real-world runs leak internal scaffolding into the published
//! comment: the raw verification reasoning (including chain-of-thought and
//! self-corrections), a YAML front-matter block, duplicated table rows, and
//! dangling headings when the generation hits the token cap. This module
//! cleans all of that deterministically — no LLM involved — and reconciles the
//! claimed `findings_total` with the actual number of table rows.

/// Header that starts the publishable final report section.
const REPORT_HEADER: &str = "## 🔍 AI Code Review";

/// Keep only the final report section of a verifier output: everything from
/// the LAST `## 🔍 AI Code Review` header to the end of the text. Verification
/// reasoning, verdict tables and YAML front-matter that precede the report are
/// dropped. If the header is absent (single-round / summarize outputs), the
/// input is returned unchanged.
pub fn extract_final_report(raw: &str) -> String {
    match raw.rfind(REPORT_HEADER) {
        Some(pos) => raw[pos..].trim_start().to_string(),
        None => raw.to_string(),
    }
}

/// Remove trailing heading lines that are followed by nothing (a symptom of a
/// generation truncated by the token cap, e.g. a dangling `#### Issue`).
pub fn trim_trailing_empty_headings(md: &str) -> String {
    let trimmed = md.trim_end();
    let mut lines: Vec<&str> = trimmed.lines().collect();
    while let Some(last) = lines.last() {
        if last.trim_start().starts_with('#') {
            lines.pop();
        } else {
            break;
        }
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Normalize a table row for comparison: strip the leading `#` index column
/// and the `Location` column, lowercase and collapse whitespace in the
/// remaining cells. Two rows describing the same issue at different locations
/// are therefore duplicates; the first occurrence (with its location) is kept.
fn normalize_row(row: &str) -> String {
    let cells: Vec<String> = row
        .trim()
        .trim_start_matches('|')
        .trim_end_matches('|')
        .split('|')
        .skip(2) // ignore the leading # column and the Location column
        .map(|c| {
            c.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase()
        })
        .collect();
    cells.join("|")
}

fn is_table_row(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with('|') && t.ends_with('|')
}

fn is_separator_row(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.contains("---")
}

/// Drop duplicate data rows inside each markdown table. Two rows are
/// duplicates when every cell except the leading `#` index column matches
/// (case- and whitespace-insensitive). Header and separator rows are kept.
pub fn dedup_exact_table_rows(md: &str) -> (String, usize) {
    let mut out: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut removed = 0usize;

    for line in md.lines() {
        if is_table_row(line) && !is_separator_row(line) {
            let key = normalize_row(line);
            if seen.contains(&key) {
                removed += 1;
                continue;
            }
            seen.push(key);
        } else if !is_table_row(line) {
            // Leaving a table resets dedup scope to the next table.
            seen.clear();
        }
        out.push(line.to_string());
    }

    let mut result = out.join("\n");
    if !result.is_empty() {
        result.push('\n');
    }
    (result, removed)
}

/// Count data rows in the report's tables (rows starting a table are counted
/// except headers/separators).
fn count_table_rows(md: &str) -> usize {
    md.lines()
        .filter(|l| is_table_row(l) && !is_separator_row(l))
        .filter(|l| !l.trim().starts_with("| #") && !l.trim().starts_with("|#"))
        .count()
}

/// Extract the claimed `findings_total` from a YAML front-matter block, if any.
fn claimed_findings_total(md: &str) -> Option<usize> {
    for line in md.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("findings_total:") {
            return rest.trim().parse().ok();
        }
    }
    None
}

/// Reconcile the claimed findings count with the actual number of table rows.
/// Returns `(actual_rows, claimed)`.
pub fn reconcile_findings_total(md: &str) -> (usize, Option<usize>) {
    (count_table_rows(md), claimed_findings_total(md))
}

/// Build a caveat disclosing that some modified files could not be fetched,
/// so findings touching them were only verified against the diff.
pub fn coverage_caveat(unfetched: &[String]) -> String {
    let list = unfetched.join(", ");
    format!(
        "> ⚠️ Coverage: {} modified file(s) could not be fetched for verification: {}. \
         Findings touching these files are based on the diff only.",
        unfetched.len(),
        list
    )
}

/// Full cleaning pipeline for LLM review output. Returns the publishable
/// Markdown plus human-readable notes about what was changed.
pub fn sanitize(raw: &str) -> (String, Vec<String>) {
    let mut notes = Vec::new();
    let claimed = claimed_findings_total(raw);

    let extracted = extract_final_report(raw);
    if extracted != raw.trim_start() {
        notes.push("Removed verification scaffolding (raw reasoning, front-matter) — only the final report is published.".to_string());
    }

    let (deduped, removed) = dedup_exact_table_rows(&extracted);
    if removed > 0 {
        notes.push(format!("Removed {removed} duplicated table row(s)."));
    }

    let trimmed = trim_trailing_empty_headings(&deduped);
    if trimmed != deduped {
        notes.push("Trimmed dangling heading(s) left by a truncated generation.".to_string());
    }

    let (actual, _) = reconcile_findings_total(&trimmed);
    if let Some(claimed) = claimed {
        if claimed != actual {
            notes.push(format!(
                "Reconciled findings count: report claims {claimed} findings but contains {actual} table rows."
            ));
        }
    }
    println!("Report findings (table rows): {actual}");

    (trimmed, notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "## 🔍 AI Code Review";

    #[test]
    fn extract_keeps_only_final_report() {
        let raw = "### Verification of Findings\n\n#### 1. `a.py:1` — thing\n\
                   **Status:** CONFIRMED\nWait, let's re-read.\n*Correction*: blah\n\
                   **Status: REJECTED**\n\n---\n\n### Final Report\n\n---\n\
                   findings_total: 1\ntop_files:\n  - a.py\nrisk_score: low\n---\n\n\
                   ## 🔍 AI Code Review\n\n### 📋 Summary\nAll good.\n";
        let out = extract_final_report(raw);
        assert!(out.starts_with(HEADER));
        assert!(out.contains("All good."));
        assert!(!out.contains("Verification of Findings"));
        assert!(!out.contains("Wait, let's re-read"));
        assert!(!out.contains("findings_total"));
        assert!(!out.contains("Status: REJECTED"));
    }

    #[test]
    fn extract_falls_back_when_no_report_header() {
        let raw = "just some markdown\nwith no header";
        assert_eq!(extract_final_report(raw), raw);
    }

    #[test]
    fn extract_uses_last_occurrence_of_header() {
        let raw = "## 🔍 AI Code Review\n\nstale draft\n\n## 🔍 AI Code Review\n\nfinal\n";
        let out = extract_final_report(raw);
        assert!(out.starts_with(HEADER));
        assert!(out.contains("final"));
        assert!(!out.contains("stale draft"));
    }

    #[test]
    fn trim_removes_dangling_trailing_heading() {
        let md = "| 1 | `a.py:1` | bug | 🟡 Medium |\n\n#### Issue 2\n";
        let out = trim_trailing_empty_headings(md);
        assert!(out.contains("| 1 |"));
        assert!(!out.contains("#### Issue 2"));
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn trim_keeps_content_after_heading() {
        let md = "#### Issue 1\nsome detail text\n";
        assert_eq!(trim_trailing_empty_headings(md), md);
    }

    #[test]
    fn dedup_removes_identical_rows_ignoring_index_and_case() {
        let md = "| # | Location | Issue | Severity |\n\
                  |---|----------|-------|----------|\n\
                  | 14 | `tests/a.py:145` | Test dependency: calls another test. | 🟢 Low |\n\
                  | 15 | `tests/a.py:153` | Test dependency: calls another test. | 🟢 Low |\n\
                  | 16 | `tests/a.py:145` | Test dependency: calls another test. | 🟢 Low |\n\
                  | 17 | `tests/b.py:1` | Different issue. | 🟢 Low |\n";
        let (out, removed) = dedup_exact_table_rows(md);
        assert_eq!(removed, 2);
        assert!(out.contains("| 14 |"));
        assert!(out.contains("`tests/b.py:1`"));
        assert!(!out.contains("| 15 |"));
        assert!(!out.contains("| 16 |"));
    }

    #[test]
    fn dedup_does_not_cross_table_boundaries() {
        let md = "| # | Location | Issue | Severity |\n\
                  |---|----------|-------|----------|\n\
                  | 1 | `a.py:1` | bug | 🟡 Medium |\n\
                  \n\
                  | # | Location | Issue | Risk Level |\n\
                  |---|----------|-------|------------|\n\
                  | 1 | `a.py:1` | bug | 🟡 Medium |\n";
        let (_, removed) = dedup_exact_table_rows(md);
        assert_eq!(removed, 0);
    }

    #[test]
    fn reconcile_counts_rows_and_parses_claim() {
        let md = "---\nfindings_total: 5\n---\n\n## 🔍 AI Code Review\n\n\
                  | # | Location | Issue | Severity |\n|---|---|---|---|\n\
                  | 1 | `a.py:1` | x | 🟡 Medium |\n| 2 | `b.py:2` | y | 🟢 Low |\n";
        let (actual, claimed) = reconcile_findings_total(md);
        assert_eq!(actual, 2);
        assert_eq!(claimed, Some(5));
    }

    #[test]
    fn sanitize_full_pipeline_on_realistic_verifier_output() {
        let raw = "### Verification of Findings\n\n#### 1. `a.py:1` — Hardcoded secret\n\
                   **Status:** CONFIRMED\nReasoning: yes.\n\n#### 2. `b.py:2` — thing\n\
                   **Status:** CONFIRMED\nWait, let's re-read.\n**Status: REJECTED**\n\n\
                   ---\n\n### Final Report\n\n---\nfindings_total: 18\n---\n\n\
                   ## 🔍 AI Code Review\n\n### 📋 Summary\nBad.\n\n\
                   | # | Location | Issue | Severity |\n|---|---|---|---|\n\
                   | 1 | `a.py:1` | leak | 🔴 Critical |\n\
                   | 2 | `t.py:9` | calls another test | 🟢 Low |\n\
                   | 3 | `t.py:9` | calls another test | 🟢 Low |\n\n#### Issue\n";
        let (out, notes) = sanitize(raw);
        assert!(out.starts_with(HEADER));
        assert!(!out.contains("Verification of Findings"));
        assert!(!out.contains("findings_total"));
        assert!(!out.contains("| 3 |"));
        assert!(!out.ends_with("#### Issue"));
        assert!(notes.iter().any(|n| n.contains("duplicated table row")));
        assert!(notes.iter().any(|n| n.contains("dangling heading")));
        assert!(notes.iter().any(|n| n.contains("claims 18")));
    }

    #[test]
    fn sanitize_is_noop_for_clean_single_round_output() {
        let raw = "## 🔍 AI Code Review\n\n### 📋 Summary\nFine.\n";
        let (out, notes) = sanitize(raw);
        assert_eq!(
            out.trim_end(),
            "## 🔍 AI Code Review\n\n### 📋 Summary\nFine."
        );
        assert!(notes.is_empty());
    }

    #[test]
    fn coverage_caveat_lists_files_and_count() {
        let caveat = coverage_caveat(&["app/a.py".to_string(), "app/b.py".to_string()]);
        assert!(caveat.contains("2 modified file(s)"));
        assert!(caveat.contains("app/a.py, app/b.py"));
        assert!(caveat.contains("diff only"));
    }
}
