use std::fs;
use std::path::Path;

const MAX_SOURCE_LINES: usize = 500;

/// Extract modified file paths from a diff string.
pub fn extract_modified_files(diff: &str) -> Vec<String> {
    diff.lines()
        .filter_map(|line| line.strip_prefix("# File: "))
        .map(|s| s.trim().to_string())
        .collect()
}

/// Read source files from filesystem. Truncates files > MAX_SOURCE_LINES.
pub fn read_source_files(files: &[String]) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for file in files {
        let path = Path::new(file);
        if !path.exists() {
            continue;
        }
        if let Ok(content) = fs::read_to_string(path) {
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
    }
    result
}

/// Format source files into a prompt section.
#[allow(dead_code)]
pub fn format_source_context(files: &[(String, String)]) -> String {
    if files.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "\n\n[SOURCE FILES — for verification context, do NOT review unchanged code]\n",
    );
    for (name, content) in files {
        out.push_str(&format!("\n## file: {name}\n```\n{content}\n```\n"));
    }
    out
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
    }

    #[test]
    fn format_source_empty() {
        assert!(format_source_context(&[]).is_empty());
    }

    #[test]
    fn format_source_with_files() {
        let files = vec![("main.rs".to_string(), "fn main() {}".to_string())];
        let output = format_source_context(&files);
        assert!(output.contains("## file: main.rs"));
        assert!(output.contains("fn main() {}"));
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
