use std::fs;
use std::path::Path;

const MAX_SOURCE_LINES: usize = 1000;

/// Extract modified file paths from a diff string.
/// Parses `# File:` headers added by `fetch_pr_diff()`.
pub fn extract_modified_files(diff: &str) -> Vec<String> {
    diff.lines()
        .filter_map(|line| line.strip_prefix("# File: "))
        .map(|s| s.trim().to_string())
        .collect()
}

/// Returns true if a file path is safe to read (no traversal, no absolute path).
fn is_safe_path(path: &str) -> bool {
    !path.starts_with('/') && !path.starts_with('\\') && !path.contains("..")
}

/// Read source files from the filesystem (relative to CWD).
/// Returns Vec<(filename, content)>. Skips files that don't exist.
/// Rejects paths with traversal components for security.
/// Truncates files exceeding MAX_SOURCE_LINES with a note.
pub fn read_source_files(files: &[String]) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for file in files {
        if !is_safe_path(file) {
            eprintln!("Unsafe path rejected (skipping): {file}");
            continue;
        }
        let path = Path::new(file);
        if !path.exists() {
            eprintln!("Source file not found (skipping): {file}");
            continue;
        }
        match fs::read_to_string(path) {
            Ok(content) => {
                let lines: Vec<&str> = content.lines().collect();
                if lines.len() > MAX_SOURCE_LINES {
                    let truncated: String = lines[..MAX_SOURCE_LINES].join("\n");
                    result.push((
                        file.clone(),
                        format!(
                            "{truncated}\n\n[... truncated at {MAX_SOURCE_LINES} lines, {} total ...]",
                            lines.len()
                        ),
                    ));
                } else {
                    result.push((file.clone(), content));
                }
            }
            Err(e) => {
                eprintln!("Failed to read {file}: {e}");
            }
        }
    }
    result
}

/// Build source context respecting a character budget.
/// Includes files smallest-first until budget is reached.
pub fn build_context_with_budget(files: &[(String, String)], max_chars: usize) -> String {
    if files.is_empty() {
        return String::new();
    }

    // Sort by content length (smallest first)
    let mut sorted: Vec<&(String, String)> = files.iter().collect();
    sorted.sort_by_key(|(_, content)| content.len());

    let header = "\n\n[SOURCE FILES — for verification context, do NOT review unchanged code]\n";
    let mut out = String::from(header);
    let mut budget_used = header.len();
    let mut included = 0;
    let mut excluded = Vec::new();

    for (name, content) in &sorted {
        let entry = format!("\n## file: {name}\n```\n{content}\n```\n");
        if budget_used + entry.len() > max_chars && included > 0 {
            excluded.push(name.as_str());
            continue;
        }
        out.push_str(&entry);
        budget_used += entry.len();
        included += 1;
    }

    if !excluded.is_empty() {
        let note = format!(
            "\n[Context budget reached — {} file(s) excluded: {}]\n",
            excluded.len(),
            excluded.join(", ")
        );
        out.push_str(&note);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_files_from_diff() {
        let diff = "some header\n# File: src/main.rs\n+ code\n# File: src/lib.rs\n+ more";
        let files = extract_modified_files(diff);
        assert_eq!(files, vec!["src/main.rs", "src/lib.rs"]);
    }

    #[test]
    fn extract_files_empty_diff() {
        assert!(extract_modified_files("").is_empty());
        assert!(extract_modified_files("no file headers here").is_empty());
    }

    #[test]
    fn budget_excludes_large_files() {
        let files = vec![
            ("small.rs".to_string(), "x".repeat(100)),
            ("big.rs".to_string(), "y".repeat(10_000)),
        ];
        // Budget that fits header + small.rs but not big.rs
        let output = build_context_with_budget(&files, 500);
        assert!(output.contains("small.rs"));
        assert!(output.contains("excluded"));
        assert!(output.contains("big.rs"));
    }

    #[test]
    fn safe_path_rejects_traversal() {
        assert!(!is_safe_path("../etc/passwd"));
        assert!(!is_safe_path("/etc/shadow"));
        assert!(!is_safe_path("foo/../../bar"));
        assert!(!is_safe_path("\\windows\\system32"));
    }

    #[test]
    fn safe_path_accepts_relative() {
        assert!(is_safe_path("src/main.rs"));
        assert!(is_safe_path("crates/cli/src/lib.rs"));
        assert!(is_safe_path("Cargo.toml"));
    }

    #[test]
    fn read_source_rejects_unsafe_paths() {
        let files = vec!["../../../etc/passwd".to_string()];
        let result = read_source_files(&files);
        assert!(result.is_empty());
    }

    #[test]
    fn budget_includes_all_when_sufficient() {
        let files = vec![
            ("a.rs".to_string(), "code a".to_string()),
            ("b.rs".to_string(), "code b".to_string()),
        ];
        let output = build_context_with_budget(&files, 100_000);
        assert!(output.contains("a.rs"));
        assert!(output.contains("b.rs"));
        assert!(!output.contains("excluded"));
    }
}
