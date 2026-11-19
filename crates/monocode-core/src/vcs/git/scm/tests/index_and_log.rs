use super::*;

#[test]
fn review_versions_invalidate_only_changed_file_sides() {
    let dir = tmp();
    let root = dir.path();
    assert!(init_git_commit(
        root,
        &[("a.txt", "base\n"), ("b.txt", "base\n")]
    ));
    std::fs::write(root.join("a.txt"), "stage\n").unwrap();
    assert!(git(root, &["add", "a.txt"]));
    std::fs::write(root.join("a.txt"), "work1\n").unwrap();
    std::fs::write(root.join("b.txt"), "other\n").unwrap();
    let first = diff_files(root).unwrap();
    let versions = file_diff_versions(root, &first).unwrap();
    assert_eq!(versions, file_diff_versions(root, &first).unwrap());

    std::fs::write(root.join("a.txt"), "work2\n").unwrap();
    let second = diff_files(root).unwrap();
    assert_eq!(first, second, "line counts cannot detect this edit");
    let edited = file_diff_versions(root, &second).unwrap();
    assert_ne!(versions["unstaged:a.txt"], edited["unstaged:a.txt"]);
    assert_eq!(versions["staged:a.txt"], edited["staged:a.txt"]);
    assert_eq!(versions["unstaged:b.txt"], edited["unstaged:b.txt"]);

    assert!(git(root, &["add", "a.txt"]));
    let staged = file_diff_versions(root, &second).unwrap();
    assert_ne!(edited["staged:a.txt"], staged["staged:a.txt"]);
    assert_ne!(edited["unstaged:a.txt"], staged["unstaged:a.txt"]);
    assert_eq!(edited["unstaged:b.txt"], staged["unstaged:b.txt"]);
}

#[test]
fn review_index_revalidates_closed_views_and_subdirectories() {
    let dir = tmp();
    let root = dir.path();
    std::fs::create_dir(root.join("src")).unwrap();
    assert!(init_git_commit(root, &[("src/a.txt", "base\n")]));
    std::fs::write(root.join("src/a.txt"), "edit\n").unwrap();
    let before = diff_files(root).unwrap();
    let added_name = if cfg!(windows) {
        " new file.txt"
    } else {
        " new\tfile.txt"
    };
    std::fs::write(root.join("src").join(added_name), "new\n").unwrap();
    let next = review_index(root, Some(&before)).unwrap();
    assert_eq!(next.files.len(), 2);
    assert_eq!(next.files[1].additions, before.files[0].additions);
    let sub = review_index(&root.join("src"), None).unwrap();
    assert_eq!(sub.files[0].relative, added_name);
    assert_eq!(sub.files[1].relative, "a.txt");
    let versions = file_diff_versions(&root.join("src"), &sub).unwrap();
    assert!(versions.contains_key(&format!("unstaged:{added_name}")));
    assert!(versions.contains_key("unstaged:a.txt"));
    std::fs::remove_file(root.join("src/a.txt")).unwrap();
    let deleted = review_index(root, None).unwrap();
    assert_eq!(deleted.files[1].status, "deleted");
    assert!(file_diff_versions(root, &deleted).is_ok());
}

#[test]
fn review_versions_support_repositories_with_only_untracked_files() {
    let dir = tmp();
    assert!(init_git(dir.path(), "main"));
    std::fs::write(dir.path().join("new.txt"), "new\n").unwrap();
    let snapshot = review_index(dir.path(), None).unwrap();
    assert!(file_diff_versions(dir.path(), &snapshot)
        .unwrap()
        .contains_key("unstaged:new.txt"));
}

#[test]
fn diff_stats_are_zero_outside_a_repo() {
    let dir = tmp();
    std::fs::write(dir.path().join("notes.txt"), "hello\n").unwrap();
    assert_eq!(diff_stats(dir.path()).unwrap(), DiffStats::default());
}

#[test]
fn diff_stats_count_unstaged_and_untracked() {
    let dir = tmp();
    let dir = dir.path();
    assert!(init_git_commit(dir, &[("a.txt", "alpha\nbeta\ngamma\n")]));
    std::fs::write(dir.join("a.txt"), "alpha\ngamma\ndelta\n").unwrap();
    std::fs::write(dir.join("new.txt"), "hello\nworld\n").unwrap();
    std::fs::write(dir.join("ignored.txt"), "nope\n").unwrap();
    std::fs::write(dir.join(".gitignore"), "ignored.txt\n").unwrap();
    // a.txt: -beta +delta; new.txt: +2; .gitignore: +1 untracked
    assert_eq!(
        diff_stats(dir).unwrap(),
        DiffStats {
            files: 3,
            additions: 4,
            deletions: 1
        }
    );
}

#[test]
fn diff_stats_are_zero_when_clean() {
    let dir = tmp();
    assert!(init_git_commit(dir.path(), &[("a.txt", "alpha\n")]));
    assert_eq!(diff_stats(dir.path()).unwrap(), DiffStats::default());
}

#[test]
fn diff_index_lists_modified_and_untracked() {
    let dir = tmp();
    let dir = dir.path();
    assert!(init_git_commit(dir, &[("a.txt", "alpha\nbeta\ngamma\n")]));
    std::fs::write(dir.join("a.txt"), "alpha\ngamma\ndelta\n").unwrap();
    std::fs::write(dir.join("new.txt"), "hello\nworld\n").unwrap();

    let index = index(dir);
    assert_eq!(index.branch.as_deref(), Some("main"));
    assert_eq!(index.files.len(), 2);
    assert_eq!((index.additions, index.deletions), (3, 1));
    let modified = &index.files[0];
    assert_eq!(modified.relative, "a.txt");
    assert_eq!(modified.status, "modified");
    assert_eq!((modified.additions, modified.deletions), (1, 1));
    assert_eq!(modified.path, path_to_js(&dir.join("a.txt")));
    let untracked = &index.files[1];
    assert_eq!(untracked.relative, "new.txt");
    assert_eq!(untracked.status, "untracked");
    assert_eq!((untracked.additions, untracked.deletions), (2, 0));
    assert!(untracked.unstaged && !untracked.staged);
}

#[test]
fn diff_index_reports_added_and_deleted() {
    let dir = tmp();
    let dir = dir.path();
    assert!(init_git_commit(
        dir,
        &[("a.txt", "alpha\n"), ("b.txt", "b\n")]
    ));
    std::fs::write(dir.join("added.txt"), "x\n").unwrap();
    stage_file(dir, "added.txt").unwrap();
    std::fs::remove_file(dir.join("b.txt")).unwrap();
    assert_eq!(file(dir, "added.txt").status, "added");
    assert_eq!(file(dir, "b.txt").status, "deleted");
}

#[test]
fn diff_index_keeps_literal_non_ascii_paths() {
    let dir = tmp();
    let dir = dir.path();
    assert!(init_git_commit(
        dir,
        &[("한글.md", "before\n"), ("my file.txt", "one\n")]
    ));
    assert!(git(dir, &["config", "core.quotePath", "true"]));
    std::fs::write(dir.join("한글.md"), "before\nafter\n").unwrap();
    std::fs::write(dir.join("my file.txt"), "one\ntwo\n").unwrap();

    let hangul = file(dir, "한글.md");
    assert_eq!(hangul.status, "modified");
    assert_eq!(hangul.additions, 1);
    assert!(hangul.unstaged);
    let spaced = file(dir, "my file.txt");
    assert_eq!(spaced.status, "modified");
    assert_eq!(spaced.additions, 1);
}

#[test]
fn diff_index_keeps_paths_relative_to_nested_workspace() {
    let dir = tmp();
    let dir = dir.path();
    let workspace = dir.join("sub");
    std::fs::create_dir_all(workspace.join("sub")).unwrap();
    assert!(init_git_commit(
        dir,
        &[("sub/file.txt", "original\n"), ("sub/second.txt", "old\n")],
    ));
    std::fs::write(workspace.join("file.txt"), "staged\n").unwrap();
    assert!(git(dir, &["add", "--", "sub/file.txt"]));
    std::fs::write(workspace.join("second.txt"), "unstaged\n").unwrap();
    // Without --relative this collides with the tracked sub/file.txt key.
    std::fs::write(workspace.join("sub/file.txt"), "untracked\n").unwrap();

    let changes = index(&workspace);
    assert_eq!(changes.files.len(), 3);
    for (relative, staged, unstaged) in [
        ("file.txt", true, false),
        ("second.txt", false, true),
        ("sub/file.txt", false, true),
    ] {
        let file = changes
            .files
            .iter()
            .find(|file| file.relative == relative)
            .unwrap();
        assert_eq!(file.path, path_to_js(&workspace.join(relative)));
        assert_eq!((file.staged, file.unstaged), (staged, unstaged));
    }
}

#[test]
fn diff_index_works_without_a_head() {
    let dir = tmp();
    let dir = dir.path();
    assert!(init_git(dir, "main"));
    std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(dir.join("b.txt"), "x\n").unwrap();
    stage_file(dir, "a.txt").unwrap();
    let index = index(dir);
    assert_eq!(index.head, None);
    assert_eq!(index.branch.as_deref(), Some("main"));
    let a = &index.files[0];
    assert_eq!((a.relative.as_str(), a.status.as_str()), ("a.txt", "added"));
    assert_eq!(a.additions, 2);
    assert!(a.staged && !a.unstaged);
    assert_eq!(index.files[1].status, "untracked");
}

#[test]
fn parse_decorations_classifies_head_local_remote_and_tags() {
    let remotes = vec!["origin".to_string()];
    let (head, refs) = parse_decorations(
        "HEAD -> main, origin/main, origin/HEAD, tag: v0.1.30, fix/foo",
        Some("abc"),
        "abc",
        &remotes,
    );
    assert!(head);
    let refs: Vec<_> = refs
        .iter()
        .map(|r| (r.name.as_str(), r.kind.as_str()))
        .collect();
    assert_eq!(
        refs,
        [
            ("main", "local"),
            ("origin/main", "remote"),
            ("v0.1.30", "tag"),
            ("fix/foo", "local"),
        ]
    );
    let (head, refs) = parse_decorations("HEAD", None, "abc", &remotes);
    assert!(head && refs.is_empty());
    assert!(!parse_decorations("", Some("def"), "abc", &remotes).0);
}

#[test]
fn parse_history_log_reads_records() {
    let text = "aaaa\0aa\0bbbb cccc\0Ann\x001700000000\0HEAD -> main\0Merge it\x1e\n\
                bbbb\0\0\0Bob\0bad\0\0\x1e\nshort\0record\x1e";
    let commits = parse_history_log(text, Some("aaaa"), &[]);
    assert_eq!(commits.len(), 2);
    assert_eq!(commits[0].parents, ["bbbb", "cccc"]);
    assert_eq!(commits[0].timestamp, 1_700_000_000);
    assert_eq!(commits[0].subject, "Merge it");
    assert!(commits[0].head);
    assert_eq!(commits[1].short_sha, "bbbb");
    assert_eq!(commits[1].timestamp, 0);
    assert!(commits[1].parents.is_empty() && !commits[1].head);
}

#[test]
fn staged_only_reads_one_file_status() {
    let dir = tmp();
    let dir = dir.path();
    assert!(init_git_commit(
        dir,
        &[("a.txt", "a\n"), ("b [1].txt", "b\n")]
    ));
    assert!(!staged_only(dir, "a.txt").unwrap(), "unchanged");
    std::fs::write(dir.join("a.txt"), "a\nb\n").unwrap();
    assert!(!staged_only(dir, "a.txt").unwrap(), "unstaged");
    assert!(git(dir, &["add", "a.txt"]));
    assert!(staged_only(dir, "a.txt").unwrap(), "staged");
    std::fs::write(dir.join("a.txt"), "a\nb\nc\n").unwrap();
    assert!(
        !staged_only(dir, "a.txt").unwrap(),
        "staged and changed again"
    );
    std::fs::write(dir.join("new.txt"), "n\n").unwrap();
    assert!(!staged_only(dir, "new.txt").unwrap(), "untracked");
    // Brackets are a path, not a glob.
    std::fs::write(dir.join("b [1].txt"), "b\nc\n").unwrap();
    assert!(git(dir, &["--literal-pathspecs", "add", "b [1].txt"]));
    assert!(staged_only(dir, "b [1].txt").unwrap());
}

#[test]
fn commit_details_read_message_stat_and_web_page() {
    let dir = tmp();
    let dir = dir.path();
    assert!(init_git_commit(dir, &[("a.txt", "a\nb\n")]));
    std::fs::write(dir.join("a.txt"), "a\nc\nd\n").unwrap();
    std::fs::write(dir.join("b.txt"), "x\n").unwrap();
    assert!(git(dir, &["add", "."]));
    assert!(git(dir, &["commit", "-m", "second", "-m", "body line"]));
    assert!(git(
        dir,
        &["remote", "add", "origin", "git@github.com:o/r.git"]
    ));
    let sha = git_stdout(dir, &["rev-parse", "HEAD"]).unwrap();

    let details = commit_details(dir, &sha).unwrap();
    assert_eq!(details.message, "second\n\nbody line");
    assert_eq!(
        (details.files, details.additions, details.deletions),
        (2, 3, 1)
    );
    assert_eq!(
        details.url,
        Some(format!("https://github.com/o/r/commit/{sha}"))
    );
    assert_eq!(
        parse_shortstat(" 1 file changed, 1 deletion(-)").deletions,
        1
    );
}

#[test]
fn history_lists_commits_parents_and_head() {
    let dir = tmp();
    let dir = dir.path();
    assert!(init_git_commit(dir, &[("a.txt", "alpha\n")]));
    std::fs::write(dir.join("a.txt"), "beta\n").unwrap();
    assert!(git(dir, &["add", "."]));
    assert!(git(dir, &["commit", "-m", "second"]));

    let history = history(dir, Some(10)).unwrap();
    assert_eq!(history.commits.len(), 2);
    assert_eq!(
        history.head.as_deref(),
        Some(history.commits[0].sha.as_str())
    );
    assert_eq!(history.commits[0].subject, "second");
    assert_eq!(history.commits[1].subject, "init");
    assert_eq!(
        history.commits[0].parents,
        vec![history.commits[1].sha.clone()]
    );
    assert!(history.commits[0].head);
    assert!(!history.commits[1].head);
    assert_eq!(history.commits[0].author, "MonoCode");
    assert!(history.commits[0]
        .refs
        .iter()
        .any(|r| r.kind == "local" && r.name == "main"));
    assert_eq!(
        super::super::history(dir, Some(1)).unwrap().commits.len(),
        1
    );
}

#[test]
fn history_empty_outside_a_repo() {
    let dir = tmp();
    assert_eq!(history(dir.path(), None).unwrap(), History::default());
}

#[test]
fn history_records_merge_parents() {
    let dir = tmp();
    let dir = dir.path();
    assert!(init_git_commit(dir, &[("a.txt", "alpha\n")]));
    assert!(git(dir, &["checkout", "-b", "feature"]));
    std::fs::write(dir.join("feat.txt"), "one\n").unwrap();
    assert!(git(dir, &["add", "."]));
    assert!(git(dir, &["commit", "-m", "feature work"]));
    assert!(git(dir, &["checkout", "main"]));
    std::fs::write(dir.join("main.txt"), "two\n").unwrap();
    assert!(git(dir, &["add", "."]));
    assert!(git(dir, &["commit", "-m", "main work"]));
    assert!(git(
        dir,
        &["merge", "feature", "--no-ff", "-m", "Merge feature"]
    ));
    let history = history(dir, Some(20)).unwrap();
    let merge = history
        .commits
        .iter()
        .find(|commit| commit.subject == "Merge feature")
        .unwrap();
    assert_eq!(merge.parents.len(), 2);
}

#[test]
fn history_skips_stash_and_unmerged_branches() {
    let dir = tmp();
    let dir = dir.path();
    assert!(init_git_commit(dir, &[("a.txt", "alpha\n")]));
    assert!(git(dir, &["checkout", "-b", "feature"]));
    std::fs::write(dir.join("feat.txt"), "one\n").unwrap();
    assert!(git(dir, &["add", "."]));
    assert!(git(dir, &["commit", "-m", "feature only"]));
    std::fs::write(dir.join("wip.txt"), "stash me\n").unwrap();
    assert!(git(dir, &["stash", "push", "-u", "-m", "wip stash"]));
    assert!(git(dir, &["checkout", "main"]));
    std::fs::write(dir.join("main.txt"), "two\n").unwrap();
    assert!(git(dir, &["add", "."]));
    assert!(git(dir, &["commit", "-m", "main only"]));

    let history = history(dir, Some(20)).unwrap();
    let subjects: Vec<&str> = history
        .commits
        .iter()
        .map(|commit| commit.subject.as_str())
        .collect();
    assert!(subjects.contains(&"main only"));
    assert!(subjects.contains(&"init"));
    assert!(!subjects.iter().any(|s| s.contains("feature only")));
    assert!(!subjects
        .iter()
        .any(|s| s.contains("wip stash") || s.contains("WIP on")));
}
