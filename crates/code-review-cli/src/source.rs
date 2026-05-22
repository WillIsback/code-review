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

/// Read source files from the filesystem (relative to CWD).
/// Returns Vec<(filename, content)>. Skips files that don't exist.
/// Truncates files exceeding MAX_SOURCE_LINES with a note.
pub fn read_source_files(files: &[String]) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for file in files {
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

/// Format source files into a prompt section.
pub fn format_source_context(files: &[(String, String)]) -> String {
    if files.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n\n[SOURCE FILES -- for verification context]\n");
    for (name, content) in files {
        out.push_str(&format!("\n## file: {name}\n```\n{content}\n```\n"));
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
}
