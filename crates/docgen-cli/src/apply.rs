use crate::process::PatchResult;
use git2::Repository;
use std::path::Path;

pub fn apply_with_git(patches: Vec<PatchResult>, repo_path: &Path) -> Result<(), git2::Error> {
    let repo = Repository::discover(repo_path)?;
    let head = repo.head()?;
    let original_branch = head.shorthand().unwrap_or("").to_string();
    if original_branch.is_empty() {
        return Err(git2::Error::from_str(
            "docgen requires a named branch (HEAD is detached)",
        ));
    }

    let now = chrono::Local::now();
    let branch_name = format!("docgen/{}", now.format("%Y%m%d-%H%M%S"));

    let head_commit = head.peel_to_commit()?;
    repo.branch(&branch_name, &head_commit, false)?;
    repo.set_head(&format!("refs/heads/{branch_name}"))?;
    repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;

    // Write files and stage
    let mut index = repo.index()?;
    for patch in &patches {
        std::fs::write(&patch.path, &patch.content)
            .map_err(|e| git2::Error::from_str(&e.to_string()))?;
        let workdir = repo.workdir().unwrap_or(Path::new("."));
        let abs_path = patch
            .path
            .canonicalize()
            .map_err(|e| git2::Error::from_str(&e.to_string()))?;
        let rel = abs_path.strip_prefix(workdir).map_err(|_| {
            git2::Error::from_str(&format!(
                "patch path {} is not under repo workdir {}",
                abs_path.display(),
                workdir.display()
            ))
        })?;
        index.add_path(rel)?;
    }
    index.write()?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;
    let sig = git2::Signature::now("docgen", "docgen@localhost")?;
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        "docs: add docstrings via docgen",
        &tree,
        &[&head_commit],
    )?;

    // Checkout original branch
    repo.set_head(&format!("refs/heads/{original_branch}"))?;
    repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;

    // Merge feature branch into original
    let feature_commit = repo
        .find_branch(&branch_name, git2::BranchType::Local)?
        .get()
        .peel_to_commit()?;
    let original_commit = repo
        .find_branch(&original_branch, git2::BranchType::Local)?
        .get()
        .peel_to_commit()?;

    let ancestor_id = repo.merge_base(original_commit.id(), feature_commit.id())?;
    let ancestor = repo.find_commit(ancestor_id)?;
    let our_tree = original_commit.tree()?;
    let their_tree = feature_commit.tree()?;
    let base_tree = ancestor.tree()?;
    let mut merge_index = repo.merge_trees(&base_tree, &our_tree, &their_tree, None)?;
    let merge_tree_id = merge_index.write_tree_to(&repo)?;
    if merge_index.has_conflicts() {
        return Err(git2::Error::from_str(
            "merge conflict detected — docgen branch not merged; manual resolution required",
        ));
    }
    let merge_tree = repo.find_tree(merge_tree_id)?;

    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        &format!("docs: merge {branch_name}"),
        &merge_tree,
        &[&original_commit, &feature_commit],
    )?;

    // Update working tree to match the merge commit
    repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;

    // Delete feature branch
    repo.find_branch(&branch_name, git2::BranchType::Local)?
        .delete()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::PatchResult;
    use std::path::PathBuf;

    fn init_git_repo_with_file(dir: &std::path::Path, filename: &str, content: &str) {
        // Configure git
        std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(dir)
            .output()
            .expect("git init failed");
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir)
            .output()
            .expect("git config email failed");
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir)
            .output()
            .expect("git config name failed");

        // Write initial file and commit
        std::fs::write(dir.join(filename), content).expect("write failed");
        std::process::Command::new("git")
            .args(["add", filename])
            .current_dir(dir)
            .output()
            .expect("git add failed");
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(dir)
            .output()
            .expect("git commit failed");
    }

    #[test]
    fn test_apply_with_git_patches_file_and_creates_merge_commit() {
        let tmp = tempfile::tempdir().expect("tempdir failed");
        let dir = tmp.path();

        let filename = "index.ts";
        let original = "export function add(a: number, b: number): number { return a + b; }\n";
        let patched = "/** Adds two numbers. */\nexport function add(a: number, b: number): number { return a + b; }\n";

        init_git_repo_with_file(dir, filename, original);

        let patch = PatchResult {
            path: PathBuf::from(dir.join(filename)),
            content: patched.to_string(),
        };

        apply_with_git(vec![patch], dir).expect("apply_with_git failed");

        // Assert working tree file matches patched content
        let on_disk = std::fs::read_to_string(dir.join(filename)).expect("read failed");
        assert_eq!(
            on_disk, patched,
            "working tree file should contain patched content"
        );

        // Assert a merge commit exists (merge commits have 2 parents)
        let output = std::process::Command::new("git")
            .args(["log", "--oneline", "--all"])
            .current_dir(dir)
            .output()
            .expect("git log failed");
        let log = String::from_utf8_lossy(&output.stdout);
        assert!(
            log.contains("merge"),
            "git log should contain a merge commit, got:\n{log}"
        );
    }
}
