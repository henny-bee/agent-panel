mod composer_motion;
mod menu;
mod submission;
use super::git_branch;
#[test]
fn reads_branch_from_repositories_and_linked_worktrees() {
    let root = std::env::temp_dir().join(format!("monocode-branch-{}", std::process::id()));
    let repo = root.join("repo");
    let nested = repo.join("src/deep");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/feature/x\n").unwrap();
    assert_eq!(git_branch(&nested).as_deref(), Some("feature/x"));

    let linked = root.join("linked");
    let meta = repo.join(".git/worktrees/linked");
    std::fs::create_dir_all(&linked).unwrap();
    std::fs::create_dir_all(&meta).unwrap();
    std::fs::write(linked.join(".git"), format!("gitdir: {}\n", meta.display())).unwrap();
    std::fs::write(meta.join("HEAD"), "0123456789abcdef\n").unwrap();
    assert_eq!(git_branch(&linked).as_deref(), Some("0123456"));
    std::fs::remove_dir_all(root).unwrap();
}
