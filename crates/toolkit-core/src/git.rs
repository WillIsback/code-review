use std::path::Path;

/// Returns paths of staged, unstaged, and untracked files relative to repo root.
/// Returns empty vec if the working tree is clean or no git repo is found.
pub fn dirty_files(repo_path: &Path) -> Vec<String> {
    let Ok(repo) = git2::Repository::discover(repo_path) else {
        return vec![];
    };
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .exclude_submodules(true)
        .include_ignored(false);
    let Ok(statuses) = repo.statuses(Some(&mut opts)) else {
        return vec![];
    };
    statuses
        .iter()
        .filter(|s| s.status() != git2::Status::CURRENT)
        .filter_map(|s| s.path().map(String::from))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_vec_for_current_repo() {
        // The ai-devops-toolkit repo is at this path
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent().unwrap().parent().unwrap();
        let files = dirty_files(root);
        // Just assert no panic; content depends on local state
        let _ = files;
    }

    #[test]
    fn returns_empty_for_nonexistent_path() {
        let files = dirty_files(std::path::Path::new("/nonexistent/path/12345"));
        assert!(files.is_empty());
    }
}
