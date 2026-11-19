use super::*;

#[gpui::test]
fn the_terminal_dock_toggles_adds_closes_and_moves(cx: &mut TestAppContext) {
    let (window, _, b) = window(cx);
    cx.update(|cx| {
        bind_keys(cx);
        crate::terminal::bind(cx);
        cx.set_reduce_motion(true);
    });
    window
        .update(cx, |shell, w, cx| {
            shell.open(b, w, cx);
            shell.focus_active(w, cx);
        })
        .unwrap();
    let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    cx.run_until_parked();
    assert!(cx.debug_bounds("terminal-dock").is_none());
    cx.simulate_keystrokes("secondary-j");
    cx.run_until_parked();
    let project = window
        .update(cx, |shell, w, cx| {
            let project = shell.workspace.project.clone();
            let dock = &shell.workspace.docks[&project];
            assert!(dock.open);
            assert_eq!(dock.terminals.as_slice(), std::slice::from_ref(&project));
            assert_eq!(shell.terminals[&project].len(), 1);
            // The new terminal has the keys.
            assert_eq!(shell.dock_focused(w, cx), Some(0));
            project
        })
        .unwrap();
    let dock = cx.debug_bounds("terminal-dock").unwrap();
    assert!((dock.size.height - px(220.)).abs() < px(2.));
    // ⌘T in the dock opens another terminal, not a session tab.
    let tabs = window.update(cx, |s, _, _| s.workspace.tabs.len()).unwrap();
    cx.simulate_keystrokes("secondary-t");
    cx.run_until_parked();
    window
        .update(cx, |shell, w, cx| {
            assert_eq!(shell.workspace.tabs.len(), tabs);
            assert_eq!(shell.workspace.docks[&project].terminals.len(), 2);
            assert_eq!(shell.workspace.docks[&project].active, 1);
            assert_eq!(shell.dock_focused(w, cx), Some(1));
        })
        .unwrap();
    assert!(cx.debug_bounds("terminal-tab-1").is_some());
    // ⌘W closes the focused terminal; the session tab stays.
    cx.simulate_keystrokes("secondary-w");
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| {
            assert_eq!(shell.workspace.tabs.len(), tabs);
            assert_eq!(shell.workspace.docks[&project].terminals.len(), 1);
            assert_eq!(shell.terminals[&project].len(), 1);
        })
        .unwrap();
    // Docked right, it takes width: a narrow window gives up its sidebar.
    cx.simulate_resize(gpui::size(px(1000.), px(800.)));
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            shell.workspace.docks.get_mut(&project).unwrap().size = 360.;
            shell.set_dock_side(DockSide::Right, cx);
            assert!(!shell.workspace.sidebar_visible);
        })
        .unwrap();
    cx.run_until_parked();
    let dock = cx.debug_bounds("terminal-dock").unwrap();
    assert!((dock.size.width - px(360.)).abs() < px(2.), "{dock:?}");
    // ⌘J hides it and the composer takes the keys back.
    window
        .update(cx, |shell, w, cx| shell.focus_terminal(w, cx))
        .unwrap();
    cx.simulate_keystrokes("secondary-j");
    cx.run_until_parked();
    window
        .update(cx, |shell, w, cx| {
            assert!(!shell.workspace.docks[&project].open);
            assert_eq!(shell.dock_focused(w, cx), None);
            assert!(shell.workspace.sidebar_visible);
        })
        .unwrap();
    assert!(cx.debug_bounds("terminal-dock").is_none());
}

#[gpui::test]
fn closing_the_last_review_tab_removes_the_pane(cx: &mut TestAppContext) {
    let (window, _, b) = window(cx);
    let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    window
        .update(cx, |shell, window, cx| {
            shell.open(b, window, cx);
            shell.open_review(
                crate::git::review::ReviewTab::Commit {
                    cwd: "/work/monocode".into(),
                    sha: "abcdef1".into(),
                    short_sha: "abcdef1".into(),
                    subject: "Fix".into(),
                },
                window,
                cx,
            );
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            assert!(shell.reviews.contains_key(&b));
            assert_eq!(shell.workspace.reviews[&b].tabs.len(), 1);
            // A chat with a file beside it: the chat's line, then the
            // file's, which heads the tab while it has focus.
            let label = shell.workspace.reviews[&b].tabs[0].label();
            let copy = shell.tab_copy(b, cx);
            assert_eq!(copy.headline, label);
            let keys: Vec<_> = copy.panes.iter().map(|p| p.key).collect();
            assert_eq!((keys, copy.focus), (vec![(b, false), (b, true)], 1));
            shell.review_focused = false;
            shell.sessions[&b].update(cx, |s, _| s.summary.title = "Fix login".into());
            let copy = shell.tab_copy(b, cx);
            assert_eq!(copy.headline, "Fix login");
            assert_eq!((copy.panes[0].title.as_str(), copy.focus), ("Fix login", 0));
            let review = shell.reviews[&b].clone();
            review.update(cx, |r, cx| r.close_active(cx));
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            assert!(!shell.reviews.contains_key(&b));
            assert!(!shell.workspace.reviews.contains_key(&b));
            // Rendering again never rebuilds an empty pane.
            assert!(shell.review_for(b, cx).is_none());
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| assert!(!shell.reviews.contains_key(&b)))
        .unwrap();
}

#[gpui::test]
fn closing_and_reopening_tabs_preserves_busy_session_draft_and_history(cx: &mut TestAppContext) {
    let (window, a, b) = window(cx);
    window
        .update(cx, |shell, w, cx| {
            shell.sessions[&b].update(cx, |v, cx| {
                v.busy = true;
                v.input.update(cx, |i, cx| {
                    i.replace_text_in_range(None, "A retained draft 🙂", w, cx)
                });
            });
            shell.close(b, w, cx);
            assert_eq!(shell.workspace.active, None);
            assert_eq!(shell.workspace.project, "/work/monocode");
            assert!(shell.sessions[&b].read(cx).busy);
            shell.open(a, w, cx);
            shell.open(b, w, cx);
            assert_eq!(shell.workspace.active, Some(b));
            assert_eq!(
                shell.sessions[&b].read(cx).input.read(cx).content.as_ref(),
                "A retained draft 🙂"
            );
            shell.close(a, w, cx);
            shell.close(b, w, cx);
            assert!(shell.workspace.tabs.is_empty());
            assert!(shell.workspace.active.is_none());
            assert_eq!(shell.sessions.len(), 2);
            shell.open(b, w, cx);
            assert_eq!(shell.workspace.tabs, vec![b]);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            let saved = locked(&shell.store).unwrap().ui_state().unwrap().unwrap();
            let saved: Workspace = serde_json::from_str(&saved).unwrap();
            assert_eq!(saved.tabs, vec![b]);
            assert_eq!(
                shell.sessions[&a]
                    .read(cx)
                    .transcript
                    .read(cx)
                    .list
                    .item_count(),
                4,
                "reopening a closed session must load its receipt, answer, actions and footer"
            );
        })
        .unwrap();
}

#[gpui::test]
fn typing_and_streaming_do_not_rebuild_sidebar_or_tabs(cx: &mut TestAppContext) {
    let (window, _, b) = window(cx);
    let root = window.root(cx).unwrap();
    let chrome = root.read_with(cx, |s, _| (s.sidebar_frame.clone(), s.tabs_frame.clone()));
    let before = (
        chrome.0.read_with(cx, |v, _| v.renders),
        chrome.1.read_with(cx, |v, _| v.renders),
    );
    assert!(
        before.0 > 0 && before.1 > 0,
        "the cached surfaces must have actually rendered"
    );
    window
        .update(cx, |shell, w, cx| {
            shell.sessions[&b].update(cx, |v, cx| {
                v.input.update(cx, |i, cx| {
                    i.replace_text_in_range(None, "Local keystrokes", w, cx)
                });
                v.transcript.update(cx, |t, cx| {
                    t.patch(
                        0,
                        vec![monocode_core::session::transcript::Row {
                            role: "assistant".into(),
                            text: "Streaming output".into(),
                            first: true,
                            sent_at: None,
                        }],
                        cx,
                    )
                });
                cx.notify();
            })
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    assert_eq!(
        chrome.0.read_with(cx, |v, _| v.renders),
        before.0,
        "chat changes must reuse the sidebar"
    );
    assert_eq!(
        chrome.1.read_with(cx, |v, _| v.renders),
        before.1,
        "chat changes must reuse the tabs"
    );
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn title_particle_frames_only_invalidate_the_sidebar(cx: &mut TestAppContext) {
    let (window, _, id) = window(cx);
    cx.update(|cx| cx.set_reduce_motion(false));
    // Blank tabs deliberately have no sidebar card. Mount a conversation
    // before changing its title, so this exercises an existing label.
    window
        .update(cx, |shell, _, cx| {
            shell.sessions[&id].update(cx, |session, cx| {
                session.summary.has_messages = true;
                session.rename("Original session title".into(), cx);
            });
        })
        .unwrap();
    cx.run_until_parked();
    cx.refresh().unwrap();
    window
        .update(cx, |shell, _, cx| {
            shell.sessions[&id].update(cx, |session, cx| {
                session.rename("Matching the generated title effect".into(), cx);
            });
        })
        .unwrap();
    cx.run_until_parked();
    cx.refresh().unwrap();
    // Deliver the title-change frame's deferred layout/hover work before
    // measuring the subsequent decorative animation frames in isolation.
    cx.update_window(window.into(), |_, window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let (sidebar, tabs, transcript) = window
        .read_with(cx, |shell, cx| {
            (
                shell.sidebar_frame.clone(),
                shell.tabs_frame.clone(),
                shell.sessions[&id].read(cx).transcript.clone(),
            )
        })
        .unwrap();
    let before = (
        sidebar.read_with(cx, |view, _| view.renders),
        tabs.read_with(cx, |view, _| view.renders),
        transcript.read_with(cx, |view, _| view.render_passes),
    );
    for _ in 0..3 {
        cx.update_window(window.into(), |_, window, cx| {
            assert!(
                window.simulate_next_frame(cx) > 0,
                "title requested its next frame"
            );
            window.draw(cx).clear(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    assert!(sidebar.read_with(cx, |view, _| view.renders) > before.0);
    assert_eq!(tabs.read_with(cx, |view, _| view.renders), before.1);
    assert_eq!(
        transcript.read_with(cx, |view, _| view.render_passes),
        before.2
    );
}
