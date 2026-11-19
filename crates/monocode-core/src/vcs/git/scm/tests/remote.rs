use super::*;

#[test]
fn sync_counts_unpushed_commits() {
    let (repo, origin) = (tmp(), tmp());
    let repo = repo.path();
    assert!(init_git_commit(repo, &[("a.txt", "alpha\n")]));
    with_origin(repo, origin.path());
    std::fs::write(repo.join("a.txt"), "beta\n").unwrap();
    stage_file(repo, "a.txt").unwrap();
    commit(repo, "second", false).unwrap();
    let index = index(repo);
    assert_eq!(index.remote.as_deref(), Some("origin"));
    assert_eq!(index.upstream.as_deref(), Some("origin/main"));
    assert_eq!(index.default_branch.as_deref(), Some("main"));
    assert_eq!(
        (index.ahead, index.behind, index.ahead_of_default),
        (1, 0, 1)
    );
    assert!(!index.head_pushed);
    push(repo).unwrap();
    let index = super::super::diff_index(repo).unwrap();
    assert_eq!(index.ahead, 0);
    assert!(index.head_pushed);
}

#[test]
fn feature_branch_without_upstream_counts_and_ranges() {
    let (repo, origin) = (tmp(), tmp());
    let repo = repo.path();
    assert!(init_git_commit(repo, &[("a.txt", "alpha\n")]));
    with_origin(repo, origin.path());
    assert!(git(repo, &["checkout", "-b", "feature"]));
    assert_eq!(
        range_context(repo).unwrap_err(),
        "No commits to include in a pull request"
    );
    std::fs::write(repo.join("a.txt"), "beta\n").unwrap();
    stage_file(repo, "a.txt").unwrap();
    commit(repo, "feature work", false).unwrap();
    let index = index(repo);
    assert_eq!(index.branch.as_deref(), Some("feature"));
    assert_eq!(index.upstream, None);
    assert_eq!((index.ahead, index.ahead_of_default), (1, 1));
    let range = range_context(repo).unwrap();
    assert_eq!(range.base, "main");
    assert_eq!(range.head, "feature");
    assert!(range.commit_summary.contains("feature work"));
    assert!(range.diff_patch.contains("+beta"));
    // First push sets the upstream.
    push(repo).unwrap();
    assert_eq!(
        super::super::diff_index(repo).unwrap().upstream.as_deref(),
        Some("origin/feature")
    );
}

#[test]
fn push_without_remote_errors() {
    let dir = tmp();
    assert!(init_git_commit(dir.path(), &[("a.txt", "a\n")]));
    assert_eq!(push(dir.path()).unwrap_err(), "No git remote to push to");
}

#[test]
fn sync_pulls_then_pushes() {
    let (origin, a, b) = (tmp(), tmp(), tmp());
    let (a, b): (PathBuf, PathBuf) = (a.path().into(), b.path().into());
    assert!(init_git_commit(&a, &[("a.txt", "alpha\n")]));
    with_origin(&a, origin.path());
    let url = origin.path().to_string_lossy().into_owned();
    assert!(git(
        origin.path(),
        &["symbolic-ref", "HEAD", "refs/heads/main"]
    ));
    assert!(git(
        &b,
        &[
            "clone",
            "-c",
            "core.autocrlf=false",
            "-b",
            "main",
            &url,
            "."
        ]
    ));
    for args in [
        ["config", "user.name", "MonoCode"],
        ["config", "user.email", "monocode@test"],
        ["config", "commit.gpgsign", "false"],
    ] {
        assert!(git(&b, &args));
    }

    std::fs::write(a.join("a.txt"), "beta\n").unwrap();
    stage_file(&a, "a.txt").unwrap();
    commit(&a, "from-a", false).unwrap();
    sync(&a).unwrap();
    pull(&b).unwrap();
    assert_eq!(std::fs::read_to_string(b.join("a.txt")).unwrap(), "beta\n");
    assert_eq!((index(&b).ahead, index(&b).behind), (0, 0));

    std::fs::write(b.join("b.txt"), "from-b\n").unwrap();
    stage_file(&b, "b.txt").unwrap();
    commit(&b, "from-b", false).unwrap();
    std::fs::write(a.join("c.txt"), "from-a-again\n").unwrap();
    stage_file(&a, "c.txt").unwrap();
    commit(&a, "from-a-again", false).unwrap();
    sync(&a).unwrap();
    sync(&b).unwrap();
    assert!(b.join("c.txt").exists());
    sync(&a).unwrap();
    assert!(a.join("b.txt").exists());
}

#[test]
fn parse_gh_pr_list_prefers_open() {
    let json = r#"[{"number":2,"title":"Old","url":"https://example.com/2","state":"MERGED"},{"number":3,"title":"Now","url":"https://example.com/3","state":"OPEN"}]"#;
    let pr = parse_gh_pr_list(json).unwrap();
    assert_eq!(pr.number, 3);
    assert_eq!(pr.state, "open");
    let pr = parse_gh_pr_list(
        r#"[{"number":2,"title":"Old","url":"u","state":"MERGED"},{"number":1,"title":"Older","url":"u","state":"CLOSED"}]"#,
    )
    .unwrap();
    assert_eq!((pr.number, pr.state.as_str()), (2, "merged"));
    assert_eq!(parse_gh_pr_list("[]"), None);
}

#[test]
fn pr_head_filter_qualifies_branch_with_repo_owner() {
    assert_eq!(
        github_pr_head_filter("acme/widget", "feature/x").as_deref(),
        Some("acme:feature/x")
    );
    assert_eq!(github_pr_head_filter("bad", "feature"), None);
}

#[test]
fn split_github_repo_reads_owner_and_name() {
    assert_eq!(
        split_github_repo(" acme/widget ").unwrap(),
        ("acme".into(), "widget".into())
    );
    assert!(split_github_repo("acme").is_err());
    assert!(split_github_repo("acme/a/b").is_err());
    assert!(split_github_repo("ac me/widget").is_err());
}

#[test]
fn pr_create_rejects_empty_title() {
    let dir = tmp();
    assert_eq!(
        pr_create(dir.path(), "  ", "", "main", "feature").unwrap_err(),
        "Pull request title cannot be empty"
    );
}

#[test]
fn read_only_git_cmd_path_adds_fixed_dirs_to_inherited_path() {
    let path = git_cmd_path(&git_cmd());
    assert!(path.contains(&PathBuf::from("/opt/homebrew/bin")));
    assert!(path.contains(&PathBuf::from("/usr/local/bin")));
    if let Some(inherited) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&inherited) {
            assert!(path.contains(&dir), "missing inherited {dir:?}");
        }
    }
}

#[test]
fn local_git_never_resolves_login_shell_path() {
    for action in ["status", "diff", "add", "restore", "cat-file"] {
        let cmd = git_cmd_for_args_with_path(&[action], || {
            panic!("local git must not resolve the login-shell PATH")
        });
        assert!(git_cmd_path(&cmd).contains(&PathBuf::from("/opt/homebrew/bin")));
    }
}

#[test]
fn git_actions_that_need_helpers_use_login_shell_path() {
    for args in [
        &["commit"][..],
        &["push"],
        &["pull"],
        &["fetch"],
        &["clone"],
        &["--literal-pathspecs", "commit"],
    ] {
        let cmd = git_cmd_for_args_with_path(args, || "gui-git-path".into());
        assert!(cmd.get_envs().any(|(key, value)| {
            key == std::ffi::OsStr::new("PATH")
                && value == Some(std::ffi::OsStr::new("gui-git-path"))
        }));
    }
}

#[test]
fn file_diff_reads_each_side_and_flags_binary() {
    let dir = tmp();
    assert!(init_git_commit(
        dir.path(),
        &[("a.txt", "one\ntwo\n"), ("gone.txt", "x\n")]
    ));
    std::fs::write(dir.path().join("a.txt"), "one\nTWO\n").unwrap();
    std::fs::remove_file(dir.path().join("gone.txt")).unwrap();
    std::fs::write(dir.path().join("new.txt"), "fresh\n").unwrap();
    std::fs::write(dir.path().join("bin.dat"), [0u8, 1, 2]).unwrap();
    let unstaged = file_diff(dir.path(), "a.txt", false).unwrap();
    assert_eq!(
        (unstaged.original.as_str(), unstaged.current.as_str()),
        ("one\ntwo\n", "one\nTWO\n")
    );
    assert_eq!(unstaged.status, "modified");
    assert_eq!(
        file_diff(dir.path(), "gone.txt", false).unwrap().status,
        "deleted"
    );
    let fresh = file_diff(dir.path(), "new.txt", false).unwrap();
    assert_eq!(
        (fresh.status.as_str(), fresh.original.as_str()),
        ("untracked", "")
    );
    assert!(file_diff(dir.path(), "bin.dat", false).unwrap().binary);
    assert!(file_diff(dir.path(), "../escape", false).is_err());

    assert!(git(dir.path(), &["add", "a.txt"]));
    std::fs::write(dir.path().join("a.txt"), "one\nTWO\nthree\n").unwrap();
    let staged = file_diff(dir.path(), "a.txt", true).unwrap();
    assert_eq!(
        (staged.original.as_str(), staged.current.as_str()),
        ("one\ntwo\n", "one\nTWO\n")
    );
    assert_eq!(
        file_diff(dir.path(), "a.txt", false).unwrap().original,
        "one\nTWO\n"
    );
}

#[test]
fn stage_contents_stages_a_partial_file_only() {
    let dir = tmp();
    assert!(init_git_commit(dir.path(), &[("a.txt", "one\ntwo\n")]));
    std::fs::write(dir.path().join("a.txt"), "ONE\nTWO\n").unwrap();
    stage_contents(dir.path(), "a.txt", b"ONE\ntwo\n").unwrap();
    assert_eq!(
        file_diff(dir.path(), "a.txt", true).unwrap().current,
        "ONE\ntwo\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "ONE\nTWO\n"
    );
    let file = file(dir.path(), "a.txt");
    assert!(file.staged && file.unstaged);
}

#[test]
fn commit_files_and_diffs_include_the_root_commit() {
    let dir = tmp();
    assert!(init_git_commit(dir.path(), &[("a.txt", "one\n")]));
    let root = git_stdout(dir.path(), &["rev-parse", "HEAD"]).unwrap();
    std::fs::write(dir.path().join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(dir.path().join("b.txt"), "bee\n").unwrap();
    assert!(git(dir.path(), &["add", "."]) && git(dir.path(), &["commit", "-m", "second"]));
    let head = git_stdout(dir.path(), &["rev-parse", "HEAD"]).unwrap();
    let files = commit_files(dir.path(), &head).unwrap();
    let summary: Vec<_> = files
        .iter()
        .map(|f| {
            (
                f.relative.as_str(),
                f.status.as_str(),
                f.additions,
                f.deletions,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [("a.txt", "modified", 1, 0), ("b.txt", "added", 1, 0)]
    );
    let diff = commit_file_diff(dir.path(), &head, "a.txt").unwrap();
    assert_eq!(
        (diff.original.as_str(), diff.current.as_str()),
        ("one\n", "one\ntwo\n")
    );
    let first = commit_file_diff(dir.path(), &root, "a.txt").unwrap();
    assert_eq!(
        (first.status.as_str(), first.original.as_str()),
        ("added", "")
    );
    assert_eq!(commit_files(dir.path(), &root).unwrap().len(), 1);
    assert!(commit_files(dir.path(), "not-a-sha").is_err());
}
