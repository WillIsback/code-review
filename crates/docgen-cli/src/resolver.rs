use std::path::{Path, PathBuf};

const EXTENSIONS: &[&str] = &["py", "ts", "tsx"];

/// Returns .py/.ts/.tsx files under target.
/// Flat by default (immediate children only), recursive when `recursive=true`.
pub fn resolve_files(target: &Path, recursive: bool) -> Vec<PathBuf> {
    if target.is_file() {
        let ext = target.extension().and_then(|e| e.to_str()).unwrap_or("");
        return if EXTENSIONS.contains(&ext) { vec![target.to_path_buf()] } else { vec![] };
    }
    let mut out = collect_dir(target, recursive);
    out.sort();
    out
}

fn collect_dir(dir: &Path, recursive: bool) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut out = vec![];
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && recursive {
            out.extend(collect_dir(&path, true));
        } else if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if EXTENSIONS.contains(&ext) {
                out.push(path);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn resolves_single_py_file() {
        let tmp = TempDir::new().unwrap();
        let f = tmp.path().join("foo.py");
        fs::write(&f, "").unwrap();
        let files = resolve_files(&f, false);
        assert_eq!(files, vec![f]);
    }

    #[test]
    fn skips_non_source_extensions() {
        let tmp = TempDir::new().unwrap();
        let md = tmp.path().join("README.md");
        fs::write(&md, "").unwrap();
        let files = resolve_files(tmp.path(), false);
        assert!(files.is_empty());
    }

    #[test]
    fn flat_scan_does_not_recurse() {
        let tmp = TempDir::new().unwrap();
        let sub = tmp.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(tmp.path().join("a.py"), "").unwrap();
        fs::write(sub.join("b.py"), "").unwrap();
        let files = resolve_files(tmp.path(), false);
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("a.py"));
    }

    #[test]
    fn recursive_scan_finds_nested() {
        let tmp = TempDir::new().unwrap();
        let sub = tmp.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(tmp.path().join("a.py"), "").unwrap();
        fs::write(sub.join("b.ts"), "").unwrap();
        let files = resolve_files(tmp.path(), true);
        assert_eq!(files.len(), 2);
    }
}
