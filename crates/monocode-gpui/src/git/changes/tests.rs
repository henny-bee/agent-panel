use super::*;
#[gpui::test]
fn enabling_an_enabled_changes_panel_does_not_reload(cx: &mut gpui::TestAppContext) {
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |window, cx| {
            let git = cx.new(|cx| ProjectGit::new("~".into(), cx));
            cx.new(|cx| Changes::new(git, window, cx))
        })
        .unwrap()
    });
    window
        .update(cx, |changes, _, cx| {
            changes.set_enabled(true, cx);
            let request = changes.pr_request;
            changes.set_enabled(true, cx);
            assert_eq!(request, changes.pr_request);
            changes.set_enabled(false, cx);
            changes.set_enabled(false, cx);
            assert_eq!(request, changes.pr_request);
        })
        .unwrap();
}
#[gpui::test]
fn clicking_the_branch_name_opens_and_closes_the_branch_picker(cx: &mut gpui::TestAppContext) {
    let dir = std::env::temp_dir().join(format!("mcode-branch-switch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "init"]);
    let root = dir.to_string_lossy().into_owned();
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |window, cx| {
            let git = cx.new(|cx| ProjectGit::new(root, cx));
            cx.new(|cx| Changes::new(git, window, cx))
        })
        .unwrap()
    });
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("git-picker").is_none());

    let name = visual
        .debug_bounds("branch-switch")
        .expect("the branch name is a button");
    visual.simulate_click(name.center(), Default::default());
    visual.refresh().unwrap();
    assert!(
        visual.debug_bounds("git-picker").is_some(),
        "the branch list drops down"
    );

    let name = visual.debug_bounds("branch-switch").unwrap();
    visual.simulate_click(name.center(), Default::default());
    visual.refresh().unwrap();
    assert!(
        visual.debug_bounds("git-picker").is_none(),
        "a second click closes it"
    );
    let _ = std::fs::remove_dir_all(dir);
}
fn f(relative: &str, status: &str) -> ChangedFile {
    ChangedFile {
        path: format!("/r/{relative}"),
        relative: relative.into(),
        status: status.into(),
        additions: 0,
        deletions: 0,
        staged: false,
        unstaged: true,
    }
}
#[test]
fn tree_nests_sorts_and_rolls_status_up() {
    let files = vec![
        f("src/b.rs", "modified"),
        f("src/a.rs", "modified"),
        f("README.md", "added"),
        f("src/ui/x.rs", "untracked"),
    ];
    let tree = build_tree(&files, &[0, 1, 2, 3]);
    assert_eq!(tree.files, [2]);
    let src = &tree.dirs[0];
    assert_eq!(src.name, "src");
    assert_eq!(src.files, [1, 0]);
    assert_eq!(src.status, None);
    assert_eq!(src.dirs[0].status.as_deref(), Some("untracked"));
    assert_eq!(status_letter("untracked"), "U");
    assert_eq!(dirname("a/b/c.rs"), "a/b");
    assert_eq!(dirname("c.rs"), "");
}
