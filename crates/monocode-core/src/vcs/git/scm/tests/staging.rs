use super::*;

#[test]
fn stage_and_unstage_file() {
    let dir = tmp();
    let dir = dir.path();
    assert!(init_git_commit(dir, &[("a.txt", "alpha\n")]));
    std::fs::write(dir.join("a.txt"), "beta\n").unwrap();
    stage_file(dir, "a.txt").unwrap();
    let staged = file(dir, "a.txt");
    assert!(staged.staged && !staged.unstaged);
    unstage_file(dir, "a.txt").unwrap();
    let unstaged = file(dir, "a.txt");
    assert!(!unstaged.staged && unstaged.unstaged);
}

#[test]
fn stage_and_unstage_directory() {
    let dir = tmp();
    let dir = dir.path();
    std::fs::create_dir_all(dir.join("src/nested")).unwrap();
    std::fs::create_dir_all(dir.join("src-other")).unwrap();
    assert!(init_git_commit(
        dir,
        &[
            ("src/app.ts", "before\n"),
            ("src/nested/deleted.ts", "delete me\n"),
            ("src-other/app.ts", "before\n"),
            ("ready.txt", "before\n"),
            (".gitignore", "src/ignored.txt\n"),
        ],
    ));
    std::fs::write(dir.join("src/app.ts"), "after\n").unwrap();
    std::fs::remove_dir_all(dir.join("src/nested")).unwrap();
    std::fs::create_dir_all(dir.join("src/added")).unwrap();
    std::fs::write(dir.join("src/added/new.ts"), "new\n").unwrap();
    std::fs::write(dir.join("src/ignored.txt"), "ignored\n").unwrap();
    std::fs::write(dir.join("src-other/app.ts"), "outside\n").unwrap();
    std::fs::write(dir.join("ready.txt"), "ready\n").unwrap();
    stage_file(dir, "ready.txt").unwrap();

    stage_file(dir, "src").unwrap();
    let staged = index(dir);
    assert_eq!(staged.files.len(), 5);
    for file in &staged.files {
        let in_folder = file.relative.starts_with("src/");
        assert_eq!(file.staged, in_folder || file.relative == "ready.txt");
        assert_eq!(file.unstaged, file.relative == "src-other/app.ts");
    }

    unstage_file(dir, "src").unwrap();
    let unstaged = index(dir);
    assert_eq!(unstaged.files.len(), 5);
    for file in &unstaged.files {
        assert_eq!(file.staged, file.relative == "ready.txt");
        assert_eq!(file.unstaged, file.relative != "ready.txt");
    }
    assert_eq!(
        std::fs::read_to_string(dir.join("src/app.ts")).unwrap(),
        "after\n"
    );
}

#[cfg(unix)]
#[test]
fn stage_and_unstage_literal_directory_names() {
    for folder in ["*", "folder?", "[ab]", ":(glob)*"] {
        let dir = tmp();
        let dir = dir.path();
        for directory in [folder, "a", "folderx"] {
            std::fs::create_dir(dir.join(directory)).unwrap();
        }
        let inside = format!("{folder}/inside.txt");
        let tracked = [
            inside.as_str(),
            "a/other.txt",
            "folderx/other.txt",
            "ready.txt",
        ];
        let initial: Vec<_> = tracked.iter().map(|path| (*path, "before\n")).collect();
        assert!(init_git_commit(dir, &initial));
        for path in tracked {
            std::fs::write(dir.join(path), "after\n").unwrap();
        }
        let staged_paths = || {
            git_run(dir, &["diff", "--cached", "--name-only", "-z"])
                .unwrap()
                .split('\0')
                .filter(|path| !path.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        stage_file(dir, "ready.txt").unwrap();
        stage_file(dir, folder).unwrap();
        let mut expected = vec![inside.clone(), "ready.txt".to_string()];
        expected.sort();
        assert_eq!(staged_paths(), expected, "stage folder {folder}");
        unstage_file(dir, folder).unwrap();
        assert_eq!(staged_paths(), vec!["ready.txt"], "unstage folder {folder}");
    }
}

#[test]
fn stage_all_and_unstage_all() {
    let dir = tmp();
    let dir = dir.path();
    assert!(init_git_commit(
        dir,
        &[("a.txt", "alpha\n"), ("b.txt", "b\n")]
    ));
    std::fs::write(dir.join("a.txt"), "beta\n").unwrap();
    std::fs::remove_file(dir.join("b.txt")).unwrap();
    std::fs::write(dir.join("new.txt"), "hello\n").unwrap();
    stage_all(dir).unwrap();
    assert!(index(dir).files.iter().all(|f| f.staged && !f.unstaged));
    assert_eq!(file(dir, "new.txt").status, "added");
    unstage_all(dir).unwrap();
    assert!(index(dir).files.iter().all(|f| !f.staged && f.unstaged));
    assert_eq!(file(dir, "new.txt").status, "untracked");
}

#[test]
fn discard_restores_tracked_and_deletes_untracked() {
    let dir = tmp();
    let dir = dir.path();
    assert!(init_git_commit(dir, &[("a.txt", "alpha\n")]));
    std::fs::write(dir.join("a.txt"), "beta\n").unwrap();
    std::fs::write(dir.join("new.txt"), "hello\n").unwrap();
    std::fs::create_dir(dir.join("newdir")).unwrap();
    std::fs::write(dir.join("newdir/x.txt"), "x\n").unwrap();
    discard_file(dir, "a.txt").unwrap();
    discard_file(dir, "new.txt").unwrap();
    discard_file(dir, "newdir").unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).unwrap(),
        "alpha\n"
    );
    assert!(!dir.join("new.txt").exists());
    assert!(!dir.join("newdir/x.txt").exists());
    assert!(index(dir).files.is_empty());
}

#[test]
fn discard_all_restores_unstaged_and_keeps_staged() {
    let dir = tmp();
    let dir = dir.path();
    assert!(init_git_commit(
        dir,
        &[("a.txt", "alpha\n"), ("b.txt", "one\n")]
    ));
    std::fs::write(dir.join("a.txt"), "beta\n").unwrap();
    std::fs::write(dir.join("b.txt"), "two\n").unwrap();
    stage_file(dir, "b.txt").unwrap();
    std::fs::write(dir.join("b.txt"), "three\n").unwrap();
    std::fs::write(dir.join("new.txt"), "hello\n").unwrap();
    discard_all(dir).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).unwrap(),
        "alpha\n"
    );
    assert_eq!(std::fs::read_to_string(dir.join("b.txt")).unwrap(), "two\n");
    assert!(!dir.join("new.txt").exists());
    let b = file(dir, "b.txt");
    assert!(b.staged && !b.unstaged);
    assert!(index(dir).files.iter().all(|file| !file.unstaged));
}

#[test]
fn discard_keeps_staged_hunks() {
    let dir = tmp();
    let dir = dir.path();
    assert!(init_git_commit(dir, &[("a.txt", "alpha\n")]));
    std::fs::write(dir.join("a.txt"), "beta\n").unwrap();
    stage_file(dir, "a.txt").unwrap();
    std::fs::write(dir.join("a.txt"), "gamma\n").unwrap();
    discard_file(dir, "a.txt").unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).unwrap(),
        "beta\n"
    );
    let a = file(dir, "a.txt");
    assert!(a.staged && !a.unstaged);
}

#[test]
fn stage_rejects_path_escape() {
    let dir = tmp();
    for path in ["../secret.txt", "/etc/passwd", "", "a//b"] {
        assert_eq!(stage_file(dir.path(), path).unwrap_err(), "Invalid path");
        assert!(unstage_file(dir.path(), path).is_err());
        assert!(discard_file(dir.path(), path).is_err());
    }
}

#[test]
fn commit_clears_staged_files() {
    let dir = tmp();
    let dir = dir.path();
    assert!(init_git_commit(dir, &[("a.txt", "alpha\n")]));
    std::fs::write(dir.join("a.txt"), "beta\n").unwrap();
    stage_file(dir, "a.txt").unwrap();
    commit(dir, "update a", false).unwrap();
    let index = index(dir);
    assert!(index.files.is_empty());
    assert_eq!(index.head, git_stdout(dir, &["rev-parse", "HEAD"]));
    assert_eq!(head_message(dir).unwrap(), "update a");
}

#[test]
fn commit_rejects_empty_message() {
    let dir = tmp();
    assert_eq!(
        commit(dir.path(), "   ", false).unwrap_err(),
        "Commit message cannot be empty"
    );
}

#[test]
fn commit_amend_rewrites_head_with_staged_changes() {
    let dir = tmp();
    let dir = dir.path();
    assert!(init_git_commit(dir, &[("a.txt", "alpha\n")]));
    std::fs::write(dir.join("a.txt"), "beta\n").unwrap();
    stage_file(dir, "a.txt").unwrap();
    commit(dir, "amended", true).unwrap();
    assert!(index(dir).files.is_empty());
    assert_eq!(
        git_stdout(dir, &["rev-list", "--count", "HEAD"]).as_deref(),
        Some("1")
    );
    assert_eq!(head_message(dir).unwrap(), "amended");
    assert_eq!(
        git_stdout(dir, &["show", "HEAD:a.txt"]).as_deref(),
        Some("beta")
    );
}

#[test]
fn head_message_returns_subject_and_body() {
    let dir = tmp();
    let dir = dir.path();
    assert!(init_git_commit(dir, &[("a.txt", "alpha\n")]));
    std::fs::write(dir.join("a.txt"), "beta\n").unwrap();
    stage_file(dir, "a.txt").unwrap();
    commit(dir, "Subject line\n\nBody text", false).unwrap();
    assert_eq!(head_message(dir).unwrap(), "Subject line\n\nBody text");
}

#[test]
fn head_message_fails_without_commits() {
    let dir = tmp();
    assert!(init_git(dir.path(), "main"));
    assert_eq!(head_message(dir.path()).unwrap_err(), "No commits yet");
}

#[test]
fn signing_hint_ignores_other_errors() {
    let error = "nothing to commit, working tree clean".to_string();
    assert_eq!(with_signing_hint(error.clone()), error);
}

#[test]
fn commit_reports_signing_failure_with_hint() {
    let dir = tmp();
    let dir = dir.path();
    assert!(init_git_commit(dir, &[("a.txt", "a\n")]));
    std::fs::write(dir.join("a.txt"), "b\n").unwrap();
    for args in [
        ["config", "commit.gpgsign", "true"],
        ["config", "gpg.format", "openpgp"],
        ["config", "gpg.program", "/nonexistent/monocode-gpg"],
    ] {
        assert!(git(dir, &args));
    }
    assert!(git(dir, &["add", "."]));
    let error = commit(dir, "signed", false).unwrap_err();
    assert!(error.contains("Git couldn't sign this commit"), "{error}");
}

#[test]
fn staged_context_reads_cached_diff() {
    let dir = tmp();
    let dir = dir.path();
    assert!(init_git_commit(dir, &[("a.txt", "alpha\n")]));
    std::fs::write(dir.join("a.txt"), "beta\n").unwrap();
    stage_file(dir, "a.txt").unwrap();
    let context = staged_context(dir).unwrap();
    assert_eq!(context.branch.as_deref(), Some("main"));
    assert!(context.summary.contains("a.txt"));
    assert!(context.patch.contains("beta"));
}

#[test]
fn staged_context_falls_back_to_unstaged_and_untracked() {
    let dir = tmp();
    let dir = dir.path();
    assert!(init_git_commit(dir, &[("a.txt", "alpha\n")]));
    assert_eq!(staged_context(dir).unwrap_err(), "No changes to summarize");
    std::fs::write(dir.join("a.txt"), "gamma\n").unwrap();
    std::fs::write(dir.join("new.txt"), "n\n").unwrap();
    let context = staged_context(dir).unwrap();
    assert!(context.patch.contains("gamma"));
    assert!(context.summary.ends_with("\nUntracked files:\nnew.txt"));
}
