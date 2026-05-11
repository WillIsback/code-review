use crate::process::PatchResult;
use git2::Repository;
use std::path::Path;

pub fn apply_with_git(patches: Vec<PatchResult>, repo_path: &Path) -> Result<(), git2::Error> {
    let repo = Repository::discover(repo_path)?;
    let head = repo.head()?;
    let original_branch = head.shorthand().unwrap_or("").to_string();
    if original_branch.is_empty() {
        return Err(git2::Error::from_str(
            "docgen requires a named branch (HEAD is detached)"
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
        let rel = patch.path
            .strip_prefix(workdir)
            .map_err(|_| git2::Error::from_str(
                &format!("patch path {} is not under repo workdir {}", patch.path.display(), workdir.display())
            ))?;
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
            "merge conflict detected — docgen branch not merged; manual resolution required"
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

    // Delete feature branch
    repo.find_branch(&branch_name, git2::BranchType::Local)?
        .delete()?;

    Ok(())
}
