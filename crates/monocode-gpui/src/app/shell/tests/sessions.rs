use super::*;

#[gpui::test]
fn switching_session_in_current_tab_keeps_its_slot_and_selection_settled(cx: &mut TestAppContext) {
    let (window, first, current) = same_project_window(cx);
    cx.update(|cx| {
        cx.set_reduce_motion(false);
        input::bind(cx);
        bind_keys(cx);
    });
    let next = window
        .update(cx, |shell, w, cx| {
            let mut summary = locked(&shell.store)
                .unwrap()
                .create("codex", &shell.workspace.project)
                .unwrap();
            summary.has_messages = true;
            summary.updated_at = 3;
            let next = summary.id;
            shell.add(summary, cx);
            for (id, updated_at) in [(first, 1), (current, 2)] {
                shell.sessions[&id].update(cx, |session, _| {
                    session.summary.has_messages = true;
                    session.summary.updated_at = updated_at;
                });
            }
            shell.open(current, w, cx);
            next
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let original_button = visual.debug_bounds("new-tab").unwrap();
    for (step, expected) in [(-1, next), (1, current), (-1, next), (1, current)] {
        window
            .update(visual, |shell, w, cx| {
                shell.workspace.preferences.tab_animations = true;
                shell.step_session(step, true, w, cx);
                assert_eq!(shell.workspace.active, Some(expected));
                let tabs = shell.visible_tabs(cx);
                assert_eq!(
                    tabs,
                    [TitleTab::Session(first), TitleTab::Session(expected)]
                );
                let (slots, moving) = shell.title_tab_motion.sync(
                    tabs.into_iter().map(|tab| (tab, ())).collect(),
                    true,
                    std::time::Instant::now(),
                );
                assert!(
                    !moving,
                    "replacing a session must not open or close a tab slot"
                );
                assert!(slots.iter().all(|slot| matches!(
                    slot,
                    crate::ui::tabs::motion::Slot::Live { open, .. } if *open == 1.
                )));
                assert_eq!(
                    shell.title_tab_selection.1,
                    Some(TitleTab::Session(expected))
                );
                assert!(
                    shell.title_tab_selection.0.is_none(),
                    "the selected slot did not change"
                );
            })
            .unwrap();
        visual.run_until_parked();
        assert_eq!(visual.debug_bounds("new-tab").unwrap(), original_button);
    }
    // Also exercise the actual shortcuts from the focused composer.
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    for (key, expected) in [("up", next), ("down", current)] {
        visual.simulate_keystrokes(&format!("{modifier}-{key}"));
        visual.run_until_parked();
        window
            .update(visual, |shell, _, _| {
                assert_eq!(shell.workspace.active, Some(expected));
                assert_eq!(shell.workspace.tabs.len(), 2);
            })
            .unwrap();
    }
}

#[gpui::test]
fn tab_project_and_session_keys_follow_monocode(cx: &mut TestAppContext) {
    let (window, a, b) = same_project_window(cx);
    cx.update(|cx| {
        input::bind(cx);
        bind_keys(cx);
    });
    let m = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let active = |cx: &mut TestAppContext| {
        window
            .update(cx, |shell, _, _| shell.workspace.active)
            .unwrap()
    };
    let keys = |keys: String, cx: &mut TestAppContext| {
        cx.simulate_keystrokes(window.into(), &keys);
        cx.run_until_parked();
    };
    window
        .update(cx, |shell, w, cx| shell.focus_active(w, cx))
        .unwrap();
    // Tabs are [a, b]: slots, the last slot, and wrapping next/previous.
    keys(format!("{m}-1"), cx);
    assert_eq!(active(cx), Some(a));
    keys(format!("{m}-9"), cx);
    assert_eq!(active(cx), Some(b));
    keys(format!("{m}-}}"), cx);
    assert_eq!(active(cx), Some(a));
    keys(format!("{m}-shift-["), cx);
    assert_eq!(active(cx), Some(b));
    keys("ctrl-tab".into(), cx);
    assert_eq!(active(cx), Some(a));

    // An empty composer hands Cmd+Shift+Right to project navigation...
    let project = |cx: &mut TestAppContext| {
        window
            .update(cx, |shell, _, _| shell.workspace.project.clone())
            .unwrap()
    };
    let before = project(cx);
    keys(format!("{m}-shift-right"), cx);
    let after = project(cx);
    assert_ne!(after, before);
    // ...but with a draft it selects text, as in any field.
    window
        .update(cx, |shell, w, cx| {
            let id = shell.workspace.active.unwrap();
            let input = shell.sessions[&id].read(cx).input.clone();
            w.focus(&input.focus_handle(cx), cx);
            input.update(cx, |i, cx| i.replace_text_in_range(None, "draft", w, cx));
        })
        .unwrap();
    keys(format!("{m}-shift-right"), cx);
    assert_eq!(project(cx), after);

    // Close the other tabs, keeping the active one.
    keys(format!("{m}-alt-t"), cx);
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(
                shell.visible_tabs(cx),
                [TitleTab::Session(shell.workspace.active_tab().unwrap())]
            );
        })
        .unwrap();
}

#[gpui::test]
fn session_menu_copies_ids_through_its_submenu(cx: &mut TestAppContext) {
    let (window, a, _) = window(cx);
    cx.update(crate::ui::popover::bind);
    window
        .update(cx, |shell, w, cx| {
            let anchor = Anchor::Point(gpui::point(px(40.), px(120.)));
            shell.open_menu(MenuTarget::Session(a), anchor, w, cx);
        })
        .unwrap();
    cx.run_until_parked();
    // Pin → Rename → Copy session ID, open it, then skip the harness ID
    // (this session has none) for the MonoCode one.
    cx.simulate_keystrokes(window.into(), "down down right down enter");
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| assert!(shell.menu.is_none()))
        .unwrap();
    let copied = cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(copied, Some(a.to_string()));
}

#[gpui::test]
fn desktop_database_sessions_load_and_copy_their_original_ids(cx: &mut TestAppContext) {
    let mut store = Store::open_monocode(std::path::Path::new(":memory:")).unwrap();
    let summary = store.create("codex", "/work/monocode").unwrap();
    let id = summary.id;
    let original = store.persisted_session_id(id).unwrap();
    store
        .write_rows(
            id,
            0,
            &[monocode_core::session::transcript::Row {
                role: "user".into(),
                text: "Existing desktop history".into(),
                first: true,
                sent_at: Some(123),
            }],
        )
        .unwrap();
    let summaries = store.sessions().unwrap();
    let mut state = Workspace::default();
    state.open(id);
    let saved = serde_json::to_string(&state).unwrap();
    let database = Arc::new(Mutex::new(store));
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| {
                Shell::new(
                    database,
                    summaries,
                    "/work/monocode".into(),
                    Some(saved),
                    cx,
                )
            })
        })
        .unwrap()
    });
    cx.refresh().unwrap();
    cx.run_until_parked();
    cx.update(crate::ui::popover::bind);
    window
        .update(cx, |shell, w, cx| {
            assert_eq!(shell.workspace.active, Some(id));
            let transcript = shell.sessions[&id].read(cx).transcript.read(cx);
            assert_eq!(transcript.source_rows()[0].text, "Existing desktop history");
            shell.open_menu(
                MenuTarget::Session(id),
                Anchor::Point(gpui::point(px(40.), px(120.))),
                w,
                cx,
            );
        })
        .unwrap();
    cx.run_until_parked();
    cx.simulate_keystrokes(window.into(), "down down right down enter");
    cx.run_until_parked();
    let copied = cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(copied, Some(original));
}

#[gpui::test]
fn sidebar_hover_updates_labels_without_rebuilding_the_chat(cx: &mut TestAppContext) {
    let (window, _, active) = window(cx);
    let root = window.root(cx).unwrap();
    let transcript = root.read_with(cx, |s, cx| s.sessions[&active].read(cx).transcript.clone());
    let before = transcript.read_with(cx, |t, _| t.render_passes);
    cx.update_window(window.into(), |_, w, cx| {
        w.simulate_mouse_move(gpui::point(px(40.), px(133.)), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(root.read_with(cx, |s, _| s.sidebar_hover), Some("Inbox"));
    cx.update_window(window.into(), |_, w, cx| {
        w.simulate_mouse_move(gpui::point(px(40.), px(166.)), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(root.read_with(cx, |s, _| s.sidebar_hover), Some("Notes"));
    assert_eq!(transcript.read_with(cx, |t, _| t.render_passes), before);
}

#[gpui::test]
fn many_blank_tabs_fit_without_creating_sidebar_conversations(cx: &mut TestAppContext) {
    let (window, _, _) = window(cx);
    window
        .update(cx, |shell, _, cx| {
            for _ in 0..18 {
                let summary = locked(&shell.store)
                    .unwrap()
                    .create("codex", "/work/monocode")
                    .unwrap();
                let id = summary.id;
                assert!(!summary.has_messages);
                shell.add(summary, cx);
                shell.workspace.open(id);
            }
            shell.invalidate_chrome(cx);
            cx.notify();
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, w, cx| {
            // The tabs share the strip instead of scrolling it.
            assert_eq!(
                shell.tab_scroll.max_offset().x,
                px(0.),
                "viewport {:?}, strip {:?}, children {}, first {:?}, last {:?}",
                w.viewport_size(),
                shell.tab_scroll.bounds(),
                shell.tab_scroll.children_count(),
                shell.tab_scroll.bounds_for_item(0),
                shell
                    .tab_scroll
                    .bounds_for_item(shell.workspace.tabs.len() - 1)
            );
            let id = shell.workspace.active.unwrap();
            let anchor = Anchor::Point(gpui::point(px(400.), px(30.)));
            shell.open_menu(MenuTarget::Tab(id), anchor, w, cx);
            // Closing every tab preserves one fresh blank tab, as in Monocode.
            for id in shell.workspace.tabs.clone() {
                shell.close(id, w, cx);
            }
            assert_eq!(shell.workspace.tabs.len(), 1);
            assert!(!shell.tab_can_close(shell.workspace.tabs[0], cx));
        })
        .unwrap();
    cx.run_until_parked();
}

#[gpui::test]
fn selected_file_reference_preserves_the_live_draft_and_requests_focus(cx: &mut TestAppContext) {
    let (window, _, active) = window(cx);
    window
        .update(cx, |shell, _, cx| {
            shell.sessions[&active].update(cx, |chat, cx| {
                chat.input.update(cx, |input, cx| {
                    input.set_text("Existing draft\n".into(), cx)
                });
            });
            shell.add_to_chat(active, "@src/main.rs (lines 2-4)", cx);
            let chat = shell.sessions[&active].read(cx);
            assert_eq!(
                chat.input.read(cx).content,
                "Existing draft\n\n@src/main.rs (lines 2-4)\n\n"
            );
            assert_eq!(shell.pending_composer_focus, Some(active));
        })
        .unwrap();
}

#[gpui::test]
fn leaving_the_original_layout_defaults_to_boxy_only_before_the_notice(cx: &mut TestAppContext) {
    let (window, _, _) = window(cx);
    window
        .update(cx, |shell, _, cx| {
            shell.workspace.classic_layout = true;
            shell.workspace.new_layout_boxy = false;
            shell.workspace.new_layout_prompt_dismissed = false;
            shell.toggle_layout(cx);
            assert!(!shell.workspace.classic_layout);
            assert!(shell.workspace.new_layout_boxy);
            assert!(shell.workspace.new_layout_prompt_dismissed);

            shell.workspace.classic_layout = true;
            shell.workspace.new_layout_boxy = false;
            shell.toggle_layout(cx);
            assert!(!shell.workspace.new_layout_boxy);
        })
        .unwrap();
}

#[gpui::test]
fn workspace_style_shortcut_only_switches_the_focused_layout(cx: &mut TestAppContext) {
    let (window, _, _) = window(cx);
    window
        .update(cx, |shell, _, cx| {
            shell.workspace.classic_layout = false;
            shell.workspace.new_layout_boxy = false;
            shell.toggle_workspace_style(cx);
            assert!(shell.workspace.new_layout_boxy);
            shell.toggle_workspace_style(cx);
            assert!(!shell.workspace.new_layout_boxy);

            shell.workspace.classic_layout = true;
            shell.toggle_workspace_style(cx);
            assert!(!shell.workspace.new_layout_boxy);
        })
        .unwrap();
}

#[gpui::test]
fn slash_menu_stays_below_tabs_with_centered_and_docked_input(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, id) = window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    for boxy in [false, true] {
        for height in [700., 500.] {
            visual.simulate_resize(gpui::size(px(1200.), px(height)));
            for has_messages in [false, true] {
                window
                    .update(visual, |shell, window, cx| {
                        shell.workspace.new_layout_boxy = boxy;
                        shell.layout_changed(cx);
                        shell.open(id, window, cx);
                        shell.sessions[&id].update(cx, |view, cx| {
                            view.summary.has_messages = has_messages;
                            window.focus(&view.input.focus_handle(cx), cx);
                            view.input
                                .update(cx, |input, cx| input.set_text("/".into(), cx));
                            cx.notify();
                        });
                    })
                    .unwrap();
                visual.run_until_parked();
                visual.refresh().unwrap();
                let tabs = visual.debug_bounds("title-tab-strip").unwrap();
                let menu = visual.debug_bounds("composer-completion-list").unwrap();
                let composer = visual.debug_bounds("composer-box").unwrap();
                assert!(
                    menu.top() >= tabs.bottom(),
                    "slash menu must leave the tabs visible"
                );
                assert!(menu.bottom() <= composer.top() - px(6.));
                assert!((menu.left() - composer.left()).abs() <= px(2.));
                assert!((menu.right() - composer.right()).abs() <= px(2.));
            }
        }
    }
}
