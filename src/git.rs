//! Git repository discovery from the filesystem only.
//!
//! No `git` subprocess is spawned: the tracer reads `.git` (a directory for a
//! normal checkout, a file for linked worktrees and submodules) and `HEAD`.
//! Linked worktrees resolve back to the *main* repository so `{repo}` stays
//! stable when you work inside `.worktrees/<branch>`.

use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInfo {
    /// Working-tree root of the main repository.
    pub root: PathBuf,
    /// Main repository directory name, e.g. `llama-rs`.
    pub name: String,
    /// Git admin directory backing this working tree.
    pub gitdir: PathBuf,
    /// Current branch, or `None` when HEAD is detached.
    pub branch: Option<String>,
    /// Linked worktree name when this is a worktree checkout.
    pub worktree: Option<String>,
}

/// Walk up from `start` looking for a repository.
pub fn resolve(start: &Path) -> Option<RepoInfo> {
    let start = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
    let mut dir: &Path = &start;
    loop {
        let dotgit = dir.join(".git");
        if dotgit.is_dir() {
            return Some(RepoInfo {
                root: dir.to_path_buf(),
                name: base_name(dir),
                gitdir: dotgit.clone(),
                branch: read_head(&dotgit.join("HEAD")),
                worktree: None,
            });
        }
        if dotgit.is_file() {
            let gitdir = parse_gitdir_file(&dotgit, dir)?;
            if let Some((main_gitdir, worktree)) = split_worktree(&gitdir) {
                let main_root = main_gitdir
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| main_gitdir.clone());
                // A relative gitdir pointer leaves `..` components; normalize
                // so the repository name is derived from a real directory.
                let main_root = main_root.canonicalize().unwrap_or(main_root);
                return Some(RepoInfo {
                    root: main_root.clone(),
                    name: base_name(&main_root),
                    branch: read_head(&gitdir.join("HEAD")),
                    gitdir,
                    worktree: Some(worktree),
                });
            }
            // Submodule or other `.git` file: the checkout is its own root.
            return Some(RepoInfo {
                root: dir.to_path_buf(),
                name: base_name(dir),
                branch: read_head(&gitdir.join("HEAD")),
                gitdir,
                worktree: None,
            });
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent,
            _ => return None,
        }
    }
}

fn base_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| path.display().to_string())
}

fn parse_gitdir_file(dotgit: &Path, base: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(dotgit).ok()?;
    let raw = text.lines().next()?.trim().strip_prefix("gitdir:")?.trim();
    if raw.is_empty() {
        return None;
    }
    let path = Path::new(raw);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    })
}

/// Turn `<main>/.git/worktrees/<name>` into `(<main>/.git, <name>)`.
fn split_worktree(gitdir: &Path) -> Option<(PathBuf, String)> {
    let comps: Vec<Component<'_>> = gitdir.components().collect();
    let idx = comps.iter().rposition(|c| c.as_os_str() == "worktrees")?;
    // Need a component after "worktrees" and at least `.git` before it.
    if idx + 1 >= comps.len() || idx == 0 {
        return None;
    }
    let name = comps[idx + 1].as_os_str().to_string_lossy().into_owned();
    let main_gitdir: PathBuf = comps[..idx].iter().collect();
    Some((main_gitdir, name))
}

fn read_head(head: &Path) -> Option<String> {
    let text = std::fs::read_to_string(head).ok()?;
    text.trim()
        .strip_prefix("ref: refs/heads/")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn head_ref(repo: &Path, branch: &str) {
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(
            repo.join(".git/HEAD"),
            format!("ref: refs/heads/{branch}\n"),
        )
        .unwrap();
    }

    #[test]
    fn finds_plain_repo_from_subdir() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("llama-rs");
        head_ref(&repo, "develop");
        let sub = repo.join("src/backend/cuda");
        fs::create_dir_all(&sub).unwrap();

        let info = resolve(&sub).unwrap();
        assert_eq!(info.name, "llama-rs");
        assert_eq!(info.root, repo.canonicalize().unwrap());
        assert_eq!(info.branch.as_deref(), Some("develop"));
        assert_eq!(info.worktree, None);
    }

    #[test]
    fn linked_worktree_resolves_to_main_repo() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("llama-rs");
        head_ref(&repo, "develop");

        // <main>/.git/worktrees/BACK-565
        let wt_admin = repo.join(".git/worktrees/BACK-565");
        fs::create_dir_all(&wt_admin).unwrap();
        fs::write(
            wt_admin.join("HEAD"),
            "ref: refs/heads/feature/BACK-565-thing\n",
        )
        .unwrap();

        // .worktrees/BACK-565/.git -> gitdir pointer (relative, like git writes)
        let wt = repo.join(".worktrees/BACK-565");
        fs::create_dir_all(&wt).unwrap();
        fs::write(wt.join(".git"), format!("gitdir: {}\n", wt_admin.display())).unwrap();

        let info = resolve(&wt).unwrap();
        assert_eq!(info.name, "llama-rs");
        assert_eq!(info.root, repo.canonicalize().unwrap());
        assert_eq!(info.branch.as_deref(), Some("feature/BACK-565-thing"));
        assert_eq!(info.worktree.as_deref(), Some("BACK-565"));
    }

    #[test]
    fn relative_gitdir_pointer_is_resolved() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("thing");
        head_ref(&repo, "main");
        let wt_admin = repo.join(".git/worktrees/w1");
        fs::create_dir_all(&wt_admin).unwrap();
        fs::write(wt_admin.join("HEAD"), "ref: refs/heads/topic\n").unwrap();
        let wt = repo.join("wt");
        fs::create_dir_all(&wt).unwrap();
        fs::write(wt.join(".git"), "gitdir: ../.git/worktrees/w1\n").unwrap();

        let info = resolve(&wt).unwrap();
        assert_eq!(info.name, "thing");
        assert_eq!(info.branch.as_deref(), Some("topic"));
    }

    #[test]
    fn detached_head_has_no_branch() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("detached");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(repo.join(".git/HEAD"), "0123456789abcdef\n").unwrap();
        let info = resolve(&repo).unwrap();
        assert_eq!(info.branch, None);
    }

    #[test]
    fn no_repo_returns_none() {
        let tmp = TempDir::new().unwrap();
        let plain = tmp.path().join("a/b/c");
        fs::create_dir_all(&plain).unwrap();
        assert!(resolve(&plain).is_none());
    }
}
