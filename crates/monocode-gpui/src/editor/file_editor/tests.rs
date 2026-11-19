mod parity;
use super::*;
use crate::ui::files::file_location;
use gpui::{
    Bounds, EntityInputHandler, TestAppContext, VisualTestContext, WindowBounds, WindowOptions,
    point, size,
};
#[gpui::test]
fn file_overview_tracks_unsaved_syntax_and_staged_or_unstaged_git_baselines(
    cx: &mut TestAppContext,
) {
    let root = std::env::temp_dir().join(format!("monocode-file-overview-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let git = |args: &[&str]| {
        let result = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    };
    git(&["init"]);
    let path = root.join("sample.js");
    let base = "const value = 1;\n";
    std::fs::write(&path, base).unwrap();
    git(&["add", "."]);
    git(&["commit", "-m", "base"]);
    let changed = "const value = ;\n";
    std::fs::write(&path, changed).unwrap();
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            let project = cx.new(|cx| ProjectGit::new(root.to_string_lossy().into_owned(), cx));
            cx.new(|cx| FileEditor::new(path.to_string_lossy().into_owned(), project, cx))
        })
        .unwrap()
    });
    window
        .update(cx, |view, _, cx| view.set_visible(true, cx))
        .unwrap();
    let finish_analysis = |cx: &mut TestAppContext| {
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(200));
        cx.run_until_parked();
    };
    finish_analysis(cx);
    window
        .update(cx, |view, w, cx| {
            assert_eq!(view.git_original.as_deref(), Some(base));
            let input = view.input.read(cx);
            assert_eq!(input.editor_analysis().changes.len(), 1);
            assert!(!input.editor_analysis().diagnostics.is_empty());
            view.input.update(cx, |input, cx| {
                input.select_range(0..input.content.len(), cx);
                input.replace_text_in_range(None, base, w, cx);
            });
        })
        .unwrap();
    finish_analysis(cx);
    window
        .update(cx, |view, _, cx| {
            assert!(view.dirty);
            assert!(view.input.read(cx).editor_analysis().changes.is_empty());
            assert!(view.input.read(cx).editor_analysis().diagnostics.is_empty());
        })
        .unwrap();
    git(&["add", "."]);
    window
        .update(cx, |view, _, cx| view.refresh_git(cx))
        .unwrap();
    finish_analysis(cx);
    window
        .update(cx, |view, _, _| {
            assert_eq!(
                view.git_original.as_deref(),
                Some(base),
                "staged-only changes compare to HEAD"
            )
        })
        .unwrap();
    std::fs::write(&path, "const value = 2;\n").unwrap();
    window
        .update(cx, |view, _, cx| view.refresh_git(cx))
        .unwrap();
    finish_analysis(cx);
    window
        .update(cx, |view, _, cx| {
            assert_eq!(
                view.git_original.as_deref(),
                Some(changed),
                "mixed staged/unstaged changes compare to the index"
            );
            assert_eq!(view.input.read(cx).editor_analysis().changes.len(), 1);
        })
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn file_links_resolve_relative_paths_and_line_locations() {
    // Resolved paths use the platform's separator.
    let native = |path: &str| path.replace('/', std::path::MAIN_SEPARATOR_STR);
    assert_eq!(
        file_location("src/../main.rs:12:4", "/project"),
        (native("/project/main.rs"), Some((12, 4)))
    );
    assert_eq!(
        file_location("file:///project/my%20file.rs#L3", "/elsewhere"),
        (native("/project/my file.rs"), Some((3, 1)))
    );
    assert_eq!(
        file_location("./README.md", "/project"),
        (native("/project/README.md"), None)
    );
    assert_eq!(
        file_location("/tmp/a.rs:0", "/project"),
        (native("/tmp/a.rs"), Some((1, 1)))
    );
}

#[gpui::test]
fn remembered_preview_mode_keeps_focus_and_find_switches_to_source(cx: &mut TestAppContext) {
    for (extension, source_mode) in [("md", false), ("svg", false), ("md", true), ("svg", true)] {
        let path = std::env::temp_dir().join(format!(
            "monocode-preview-focus-{}.{extension}",
            std::process::id()
        ));
        std::fs::write(
            &path,
            if extension == "md" {
                "# Preview\n"
            } else {
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\"/>"
            },
        )
        .unwrap();
        let file = path.to_string_lossy().into_owned();
        let window = cx.update(|cx| {
            let prefs = crate::ui::preferences::Preferences {
                file_view_mode: if source_mode { "source" } else { "preview" }.into(),
                ..Default::default()
            };
            cx.set_global(prefs);
            input::bind(cx);
            bind(cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(600.), px(400.)),
                    ))),
                    ..Default::default()
                },
                |w, cx| {
                    let git = cx.new(|cx| ProjectGit::new("/tmp".into(), cx));
                    let view = cx.new(|cx| FileEditor::new(file, git, cx));
                    w.focus(&view.focus_handle(cx), cx);
                    view
                },
            )
            .unwrap()
        });
        let cx = &mut VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();
        window
            .update(cx, |e, w, cx| {
                assert_eq!(e.preview, !source_mode);
                assert!(e.focus.contains_focused(w, cx));
                assert_eq!(e.input.focus_handle(cx).is_focused(w), source_mode);
            })
            .unwrap();
        let m = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        cx.simulate_keystrokes(&format!("{m}-f"));
        cx.run_until_parked();
        window
            .update(cx, |e, w, cx| {
                assert!(e.search_open);
                assert!(!e.preview);
                assert!(e.search.focus_handle(cx).is_focused(w));
            })
            .unwrap();
        std::fs::remove_file(path).unwrap();
    }
}

#[gpui::test]
fn editor_load_find_save_and_external_edits_preserve_the_draft(cx: &mut TestAppContext) {
    let path = std::env::temp_dir().join(format!("monocode-editor-{}.rs", std::process::id()));
    std::fs::write(&path, "fn hello() {}\r\n// hello 🙂\r\n").unwrap();
    let file = path.to_string_lossy().into_owned();
    let window = cx.update(|cx| {
        input::bind(cx);
        bind(cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(600.), px(400.)),
                ))),
                ..Default::default()
            },
            |_, cx| {
                let git = cx.new(|cx| {
                    ProjectGit::new(std::env::temp_dir().to_string_lossy().into_owned(), cx)
                });
                cx.new(|cx| FileEditor::new(file, git, cx))
            },
        )
        .unwrap()
    });
    let cx = &mut VisualTestContext::from_window(window.into(), cx);
    cx.run_until_parked();
    window
        .update(cx, |e, w, cx| {
            assert!(e.document.is_some());
            assert!(!e.dirty);
            assert_eq!(
                e.input.read(cx).content.as_ref(),
                "fn hello() {}\n// hello 🙂\n"
            );
            e.open_search(w, cx);
            e.search.update(cx, |i, cx| i.set_text("hello".into(), cx));
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |e, w, cx| {
            assert_eq!(e.matches.len(), 2);
            e.step_match(false, cx);
            assert_eq!(e.match_index, 1);
            e.dismiss_search(w, cx);
            e.input
                .update(cx, |i, cx| i.replace_text_in_range(None, "world", w, cx));
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |e, _, cx| {
            assert!(e.dirty);
            e.save(cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "fn hello() {}\r\n// world 🙂\r\n"
    );
    window
        .update(cx, |e, w, cx| {
            assert!(!e.dirty);
            e.input
                .update(cx, |i, cx| i.replace_text_in_range(None, " draft", w, cx));
        })
        .unwrap();
    cx.run_until_parked();
    std::fs::write(&path, "external\n").unwrap();
    window.update(cx, |e, _, cx| e.load(false, cx)).unwrap();
    cx.run_until_parked();
    window
        .update(cx, |e, _, cx| {
            assert!(e.disk_changed);
            assert!(e.dirty);
            assert!(e.input.read(cx).content.contains("draft"));
            e.save(cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |e, _, _| {
            assert!(e.dirty);
            assert!(e.error.as_ref().unwrap().contains("changed on disk"));
        })
        .unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "external\n");
    std::fs::write(&path, include_bytes!("../../../assets/monocode.png")).unwrap();
    window.update(cx, |e, _, cx| e.load(true, cx)).unwrap();
    cx.run_until_parked();
    window
        .update(cx, |e, _, _| {
            assert!(e.image.is_some());
            assert!(!e.dirty);
            assert!(e.document.is_none());
        })
        .unwrap();
    std::fs::remove_file(path).unwrap();
}
