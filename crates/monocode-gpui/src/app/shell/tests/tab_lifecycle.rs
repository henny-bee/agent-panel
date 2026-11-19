use super::*;

#[gpui::test]
fn tab_lifecycle_close_all_stays_in_the_selected_project(cx: &mut TestAppContext) {
    let (window, a, b) = window(cx);
    window
        .update(cx, |shell, w, cx| {
            shell.open(a, w, cx);
            shell.run_menu(MenuTarget::Tab(a), "all", None, w, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(shell.workspace.project, "/work/another-project");
            assert_eq!(shell.visible_tabs(cx).len(), 1);
            assert_eq!(shell.workspace.tabs.len(), 2);
            let active = shell.workspace.active.unwrap();
            assert!(![a, b].contains(&active));
            assert!(!shell.sessions[&active].read(cx).summary.has_messages);
            assert!(shell.sessions.contains_key(&a) && shell.sessions.contains_key(&b));
            assert!(shell.history.back.iter().all(|id| *id == b));
            assert!(shell.history.forward.is_empty());
            let saved = locked(&shell.store).unwrap().ui_state().unwrap().unwrap();
            let saved: Workspace = serde_json::from_str(&saved).unwrap();
            assert_eq!(saved.project, shell.workspace.project);
            assert_eq!(saved.tabs, shell.workspace.tabs);
            assert_eq!(saved.active, shell.workspace.active);
        })
        .unwrap();
}

#[gpui::test]
fn tab_lifecycle_new_and_split_requests_do_not_undo_project_navigation(cx: &mut TestAppContext) {
    let (window, a, b) = window(cx);
    for split in [false, true] {
        window
            .update(cx, |shell, w, cx| {
                shell.open(b, w, cx);
                if split {
                    shell.split_session(SplitDir::Right, cx);
                } else {
                    shell.create(Kind::Codex, cx);
                }
                shell.select_project("/work/another-project".into(), w, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                assert_eq!(shell.workspace.project, "/work/another-project");
                assert_eq!(shell.workspace.active, Some(a));
                assert_eq!(shell.workspace.tabs, [a, b]);
                assert!(shell.workspace.layouts.is_empty());
            })
            .unwrap();
    }
}

#[gpui::test]
fn tab_lifecycle_closing_an_inactive_tab_keeps_notes_and_focus(cx: &mut TestAppContext) {
    let (window, a, b) = window(cx);
    window
        .update(cx, |shell, w, cx| {
            shell.open_notes(w, cx);
            shell.close(a, w, cx);
            assert!(shell.notes.is_some());
            assert_eq!(shell.workspace.active, Some(b));
            assert_eq!(shell.workspace.project, "/work/monocode");
        })
        .unwrap();
}

#[gpui::test]
fn tab_lifecycle_closing_an_inactive_split_pane_keeps_notes(cx: &mut TestAppContext) {
    let (window, a, b) = window(cx);
    window
        .update(cx, |shell, w, cx| {
            let summary = locked(&shell.store)
                .unwrap()
                .create("codex", "/work/another-project")
                .unwrap();
            let c = summary.id;
            shell.add(summary, cx);
            shell.open(a, w, cx);
            shell.workspace.split(c, SplitDir::Right);
            shell.open(b, w, cx);
            shell.open_notes(w, cx);
            shell.close_pane(a, w, cx);
            assert!(shell.notes.is_some());
            assert_eq!(shell.workspace.active, Some(b));
            assert_eq!(shell.workspace.tabs, [c, b]);
            assert_eq!(shell.workspace.project, "/work/monocode");
        })
        .unwrap();
}

#[gpui::test]
fn tab_lifecycle_pending_project_blanks_are_not_duplicated(cx: &mut TestAppContext) {
    let (window, _, _) = window(cx);
    window
        .update(cx, |shell, w, cx| {
            for _ in 0..5 {
                shell.select_project("/work/empty".into(), w, cx);
            }
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(shell.workspace.project, "/work/empty");
            assert_eq!(
                shell
                    .sessions
                    .values()
                    .filter(|s| s.read(cx).summary.cwd == "/work/empty")
                    .count(),
                1
            );
            assert!(shell.workspace.active.is_some());
        })
        .unwrap();
}

#[gpui::test]
fn tab_lifecycle_close_all_shortcut_keeps_the_project(cx: &mut TestAppContext) {
    let (window, a, _) = window(cx);
    cx.update(bind_keys);
    window
        .update(cx, |shell, w, cx| shell.open(a, w, cx))
        .unwrap();
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    cx.simulate_keystrokes(window.into(), &format!("{modifier}-shift-w"));
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(shell.workspace.project, "/work/another-project");
            assert_eq!(shell.visible_tabs(cx).len(), 1);
            assert_eq!(shell.workspace.tabs.len(), 2);
            let active = shell.workspace.active.unwrap();
            assert!(!shell.sessions[&active].read(cx).summary.has_messages);
        })
        .unwrap();
}

#[gpui::test]
fn bulk_close_shortcuts_keep_pinned_tabs(cx: &mut TestAppContext) {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    for keys in ["shift-w", "alt-t"] {
        let (window, a, b) = same_project_window(cx);
        cx.update(bind_keys);
        window
            .update(cx, |shell, w, cx| {
                shell.toggle_pin_tab(TitleTab::Session(b), cx);
                shell.open(a, w, cx);
            })
            .unwrap();
        cx.simulate_keystrokes(window.into(), &format!("{modifier}-{keys}"));
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                assert!(shell.workspace.tabs.contains(&b), "{keys} closed the pin");
                assert_eq!(shell.workspace.pinned_tabs, [TitleTab::Session(b)]);
                assert!(shell.visible_tabs(cx).contains(&TitleTab::Session(b)));
            })
            .unwrap();
    }
}

#[gpui::test]
fn tab_lifecycle_close_stays_in_project_and_keeps_running_drafts(cx: &mut TestAppContext) {
    let (window, a, b) = window(cx);
    window
        .update(cx, |shell, w, cx| {
            shell.sessions[&b].update(cx, |s, cx| {
                s.busy = true;
                s.input.update(cx, |i, cx| {
                    i.replace_text_in_range(None, "Keep this draft", w, cx)
                });
            });
            shell.close(b, w, cx);
            assert!(shell.focus.is_focused(w));
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, w, cx| {
            assert_eq!(shell.workspace.project, "/work/monocode");
            let blank = shell.workspace.active.unwrap();
            assert_ne!(blank, b);
            assert_eq!(shell.workspace.tabs, [a, blank]);
            assert!(!shell.sessions[&blank].read(cx).summary.has_messages);
            assert!(shell.sessions[&b].read(cx).busy);
            shell.open(b, w, cx);
            assert_eq!(
                shell.sessions[&b].read(cx).input.read(cx).content.as_ref(),
                "Keep this draft"
            );
        })
        .unwrap();
}

#[gpui::test]
fn tab_lifecycle_opening_a_conversation_replaces_the_blank_tab(cx: &mut TestAppContext) {
    let (window, a, b) = window(cx);
    window
        .update(cx, |shell, w, cx| {
            shell.sessions[&b].update(cx, |s, _| s.summary.has_messages = true);
            shell.close(b, w, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, w, cx| {
            let blank = shell.workspace.active.unwrap();
            assert_eq!(shell.workspace.tabs, [a, blank]);
            shell.open(b, w, cx);
            assert_eq!(shell.workspace.tabs, [a, b]);
            assert_eq!(shell.workspace.active, Some(b));
        })
        .unwrap();
}

#[gpui::test]
fn tab_lifecycle_close_prefers_right_then_left_in_the_same_project(cx: &mut TestAppContext) {
    let (window, a, b) = window(cx);
    window
        .update(cx, |shell, w, cx| {
            let mut extra = Vec::new();
            for _ in 0..2 {
                let summary = locked(&shell.store)
                    .unwrap()
                    .create("codex", "/work/monocode")
                    .unwrap();
                extra.push(summary.id);
                shell.add(summary, cx);
                shell.open(*extra.last().unwrap(), w, cx);
            }
            let (c, d) = (extra[0], extra[1]);
            shell.open(c, w, cx);
            shell.close(c, w, cx);
            assert_eq!(shell.workspace.active, Some(d));
            shell.close(b, w, cx);
            assert_eq!(shell.workspace.active, Some(d));
            let before = shell.workspace.tabs.clone();
            shell.close(c, w, cx);
            shell.run_menu(MenuTarget::Tab(c), "others", None, w, cx);
            shell.run_menu(MenuTarget::Tab(d), "unknown", None, w, cx);
            assert_eq!(shell.workspace.tabs, before);
            assert_eq!(shell.workspace.active, Some(d));
            shell.run_menu(MenuTarget::Tab(a), "others", None, w, cx);
            assert_eq!(shell.workspace.tabs, [a, d]);
            assert_eq!(shell.workspace.active, Some(d));
            assert_eq!(shell.workspace.project, "/work/monocode");
        })
        .unwrap();
}

#[gpui::test]
fn tab_lifecycle_project_switches_remember_the_last_focused_tab(cx: &mut TestAppContext) {
    let (window, a, b) = window(cx);
    window
        .update(cx, |shell, w, cx| {
            let summary = locked(&shell.store)
                .unwrap()
                .create("codex", "/work/monocode")
                .unwrap();
            let c = summary.id;
            shell.add(summary, cx);
            shell.open(c, w, cx);
            shell.open(b, w, cx);
            shell.select_project("/work/another-project".into(), w, cx);
            assert_eq!(shell.workspace.active, Some(a));
            shell.select_project("/work/monocode".into(), w, cx);
            assert_eq!(shell.workspace.active, Some(b));
            let saved = serde_json::to_string(&shell.workspace).unwrap();
            let summaries: Vec<_> = shell
                .sessions
                .iter()
                .map(|(id, s)| (*id, s.read(cx).summary.cwd.clone()))
                .collect();
            let restored = Workspace::restore(Some(&saved), &summaries, "/work/monocode");
            assert_eq!(restored.project_focus["/work/monocode"], b);
            assert_eq!(restored.project_focus["/work/another-project"], a);
            assert_eq!(restored.active, Some(b));
        })
        .unwrap();
}

#[gpui::test]
fn tab_lifecycle_closing_a_file_only_tab_restores_the_project_chat(cx: &mut TestAppContext) {
    let (window, a, b) = window(cx);
    window
        .update(cx, |shell, w, cx| {
            shell.open_review(
                crate::git::review::ReviewTab::Commit {
                    cwd: "/work/monocode".into(),
                    sha: "abcdef1".into(),
                    short_sha: "abcdef1".into(),
                    subject: "Fix".into(),
                },
                w,
                cx,
            );
            let review = shell.reviews[&b].clone();
            review.update(cx, |r, cx| {
                r.state.chat_hidden = true;
                r.close_active(cx);
            });
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(shell.workspace.project, "/work/monocode");
            assert!(shell.workspace.tabs.contains(&a));
            assert!(!shell.reviews.contains_key(&b));
            let active = shell.workspace.active.unwrap();
            assert_eq!(
                shell.sessions[&active].read(cx).summary.cwd,
                "/work/monocode"
            );
        })
        .unwrap();
}

#[gpui::test]
fn projects_without_tabs_land_on_one_reusable_blank_session(cx: &mut TestAppContext) {
    let (window, _, b) = window(cx);
    let blank = |shell: &Shell, cx: &App| {
        let id = shell
            .workspace
            .active
            .expect("a project always has an active session");
        let summary = &shell.sessions[&id].read(cx).summary;
        assert_eq!(summary.cwd, "/work/empty");
        assert!(!summary.has_messages);
        id
    };
    window
        .update(cx, |shell, w, cx| {
            shell.workspace.add_project("/work/empty");
            shell.select_project("/work/empty".into(), w, cx);
        })
        .unwrap();
    cx.run_until_parked();
    let (first, count) = window
        .update(cx, |shell, w, cx| {
            let id = blank(shell, cx);
            // Close its tab, leave, and come back: the same blank returns.
            shell.close(id, w, cx);
            shell.open(b, w, cx);
            shell.select_project("/work/empty".into(), w, cx);
            (id, shell.sessions.len())
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(blank(shell, cx), first);
            assert_eq!(shell.sessions.len(), count);
            assert!(shell.workspace.tabs.contains(&first));
        })
        .unwrap();
}
