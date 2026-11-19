use super::*;

#[gpui::test]
fn heartbeat_runs_keep_the_monos_selected_permissions(cx: &mut TestAppContext) {
    for mode in [
        AccessMode::ReadOnly,
        AccessMode::Supervised,
        AccessMode::AutoAcceptEdits,
        AccessMode::Auto,
        AccessMode::FullAccess,
    ] {
        let s = setup(cx);
        s.manager.update(cx, |m, cx| {
            m.chats[&s.id].update(cx, |chat, _| {
                // Avoid catalog probes or provider requests: exercise the actual
                // background runner, stopping before it dispatches a prompt.
                chat.settings.access = mode;
                chat.settings.usage_limit = Some(Default::default());
            });
            m.start_heartbeat(&s.id, "Check for follow-ups".into(), cx);
        });
        cx.run_until_parked();
        s.manager.read_with(cx, |m, cx| {
            assert!(m.error.is_none(), "heartbeat failed: {:?}", m.error);
            let run = &m.running[&m.active_run.expect("heartbeat session was created")];
            assert_eq!(run.view.read(cx).settings.access, mode);
            assert!(
                !run.view.read(cx).busy,
                "the fixture must not start a provider"
            );
        });
    }
}

#[gpui::test]
fn settings_match_react_dialog_and_left_navigation_geometry(cx: &mut TestAppContext) {
    let s = setup(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    let dialog = visual.debug_bounds("mono-settings-dialog").unwrap();
    let nav = visual.debug_bounds("mono-settings-navigation").unwrap();
    let viewport = s
        .window
        .update(visual, |_, w, _| w.viewport_size())
        .unwrap();
    assert_eq!(
        dialog.size.width + px(2.),
        px(880.).min(viewport.width - px(24.))
    );
    assert!((dialog.center().x - viewport.width / 2.).abs() <= px(1.));
    assert_eq!(nav.size.width, px(192.));
    for section in ["General", "Soul", "Memory"] {
        let row = visual
            .debug_bounds(Box::leak(
                format!("mono-settings-{section}").into_boxed_str(),
            ))
            .unwrap();
        let icon = visual
            .debug_bounds(Box::leak(
                format!("mono-settings-icon-{section}").into_boxed_str(),
            ))
            .unwrap();
        let label = visual
            .debug_bounds(Box::leak(
                format!("mono-settings-label-{section}").into_boxed_str(),
            ))
            .unwrap();
        assert_eq!(row.size.height, px(32.));
        assert_eq!(icon.left() - row.left(), px(10.));
        assert_eq!(label.left() - icon.right(), px(10.));
    }
    assert_eq!(
        visual.debug_bounds("mono-look").unwrap().size,
        gpui::size(px(80.), px(80.))
    );
    assert_eq!(
        visual.debug_bounds("mono-name-field").unwrap().size,
        gpui::size(px(256.), px(32.))
    );
}

#[gpui::test]
fn settings_color_popover_follows_its_mascot_when_the_window_resizes(cx: &mut TestAppContext) {
    let s = setup(cx);
    cx.update(|cx| {
        crate::app::shell::commands::bind_keys(cx);
        crate::ui::popover::bind(cx);
    });
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    let face = visual.debug_bounds("mono-look").unwrap();
    visual.simulate_click(face.center(), Default::default());
    visual.run_until_parked();
    for size in [
        gpui::size(px(1300.), px(900.)),
        gpui::size(px(1050.), px(760.)),
    ] {
        visual.simulate_resize(size);
        visual.run_until_parked();
        let face = visual.debug_bounds("mono-look").unwrap();
        let picker = visual.debug_bounds("mono-settings-look-picker").unwrap();
        assert!((picker.center().x - face.center().x).abs() <= px(1.));
        assert!(picker.top() >= face.bottom());
    }
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    visual.update(|_, cx| {
        let m = s.manager.read(cx);
        assert!(m.picker.is_none());
        assert!(m.settings.is_some());
    });
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    visual.update(|_, cx| assert!(s.manager.read(cx).settings.is_none()));
}

#[gpui::test]
fn memory_editor_saves_with_enter_and_stays_open_for_the_next_fact(cx: &mut TestAppContext) {
    let s = setup(cx);
    cx.update(input::bind);
    s.window
        .update(cx, |_, _, cx| {
            s.manager.update(cx, |m, cx| {
                m.settings = Some(SettingsSection::Memory);
                m.memory_adding = true;
                cx.notify();
            })
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    s.window
        .update(visual, |_, w, cx| {
            let field = s.manager.read(cx).fields["memory-fact"].clone();
            w.focus(&field.focus_handle(cx), cx);
        })
        .unwrap();
    visual.simulate_input("Keep UI aligned with React.");
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert!(
        mono::files::read(&s.root, &s.id, None, "MEMORY.md")
            .unwrap()
            .text
            .unwrap()
            .contains("Keep UI aligned with React.")
    );
    visual.update(|_, cx| {
        let m = s.manager.read(cx);
        assert!(m.memory_adding);
        assert!(m.fields["memory-fact"].read(cx).content.is_empty());
    });
}

#[gpui::test]
fn overview_activity_returns_to_its_previous_tab_and_uses_the_shared_reveal(
    cx: &mut TestAppContext,
) {
    let s = setup(cx);
    s.window
        .update(cx, |_, window, cx| {
            s.manager.update(cx, |m, cx| {
                m.select_overview(OverviewTab::Documents, cx);
                assert!(m.overview_reveal.is_some());
                m.select_overview(OverviewTab::Activity, cx);
                assert!(m.overview_previous == OverviewTab::Documents);
                m.select_overview(m.overview_previous, cx);
                assert!(m.overview_tab == OverviewTab::Documents);
                cx.set_reduce_motion(true);
                assert!(
                    m.overview_reveal
                        .as_ref()
                        .unwrap()
                        .progress(OverviewTab::Documents, window, cx)
                        .is_none()
                );
            });
        })
        .unwrap();
}

#[gpui::test]
fn floating_chat_opens_after_releasing_the_manager_and_shares_the_runtime(cx: &mut TestAppContext) {
    let s = setup(cx);
    cx.update(|cx| {
        s.manager
            .update(cx, |m, cx| m.open_floating(s.id.clone(), cx))
    });
    cx.run_until_parked();
    cx.update(|cx| {
        s.manager
            .update(cx, |m, cx| m.open_floating(s.id.clone(), cx))
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let manager = s.manager.read(cx);
        assert_eq!(manager.floating.len(), 1);
        assert_eq!(manager.chats.len(), 1);
        assert!(manager.chats.contains_key(&s.id));
        // The panel owns the chat; the workspace draws a placeholder rather
        // than the same transcript in a second window.
        assert!(manager.floating_shows(&s.id, cx));
    });
    cx.update(|cx| {
        s.manager
            .update(cx, |m, cx| m.hide_floating(&s.id.clone(), cx))
    });
    cx.run_until_parked();
    cx.update(|cx| assert!(!s.manager.read(cx).floating_shows(&s.id, cx)));
}

#[gpui::test]
fn failed_completion_focus_scrolls_after_moving_between_windows(cx: &mut TestAppContext) {
    let s = setup(cx);
    let workspace: gpui::AnyWindowHandle = s.window.into();
    workspace
        .update(cx, |_, window, cx| {
            s.manager.update(cx, |m, cx| {
                m.dismiss_settings(window, cx);
                for id in ["first", "second"] {
                    m.roster.get_mut(&s.id).unwrap().outbox.push(Message {
                        id: id.into(),
                        text: "completion".into(),
                        internal: true,
                        sent_at: None,
                        error: Some("Long failure. ".repeat(500)),
                        attachments: Vec::new(),
                    });
                }
                m.configure_chat(&s.id, cx);
            });
        })
        .unwrap();
    let mut main = gpui::VisualTestContext::from_window(workspace, cx);
    main.run_until_parked();
    main.refresh().unwrap();
    s.manager
        .update(&mut main, |m, cx| m.open_floating(s.id.clone(), cx));
    main.run_until_parked();
    let panel = s.manager.read_with(&main, |m, _| m.floating[0]);
    let mut floating = gpui::VisualTestContext::from_window(panel, cx);
    panel
        .update(&mut floating, |_, w, _| w.activate_window())
        .unwrap();
    floating.run_until_parked();
    floating.refresh().unwrap();
    let chat = s
        .manager
        .read_with(&floating, |m, _| m.chats[&s.id].clone());
    let second = chat.read_with(&floating, |c, _| c.queue_test_focus("second", 0));
    panel
        .update(&mut floating, |_, w, cx| w.focus(&second, cx))
        .unwrap();
    floating.refresh().unwrap();
    floating.run_until_parked();
    floating.refresh().unwrap();
    let list = floating.debug_bounds("mono-internal-queue-list").unwrap();
    let retry = floating.debug_bounds("mono-queue-retry-second").unwrap();
    assert!(
        retry.top() >= list.top() && retry.bottom() <= list.bottom(),
        "floating action {retry:?} outside {list:?}"
    );

    s.manager
        .update(&mut floating, |m, cx| m.hide_floating(&s.id, cx));
    floating.run_until_parked();
    workspace
        .update(&mut main, |_, w, _| w.activate_window())
        .unwrap();
    main.run_until_parked();
    main.refresh().unwrap();
    let first = chat.read_with(&main, |c, _| c.queue_test_focus("first", 0));
    workspace
        .update(&mut main, |_, w, cx| w.focus(&first, cx))
        .unwrap();
    main.refresh().unwrap();
    main.run_until_parked();
    main.refresh().unwrap();
    let list = main.debug_bounds("mono-internal-queue-list").unwrap();
    let retry = main.debug_bounds("mono-queue-retry-first").unwrap();
    assert!(
        retry.top() >= list.top() && retry.bottom() <= list.bottom(),
        "workspace action {retry:?} outside {list:?}"
    );
}

#[gpui::test]
fn floating_chat_leaves_the_workspace_selection_alone(cx: &mut TestAppContext) {
    let s = setup(cx);
    let other = cx.update(|cx| {
        s.manager.update(cx, |m, cx| {
            let before = m.active.clone();
            let id = m.create_floating(Vec::new(), cx);
            assert_eq!(
                m.active, before,
                "a panel's new Mono stays out of the workspace"
            );
            m.open_floating(id.clone(), cx);
            assert_eq!(m.active, before);
            id
        })
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let manager = s.manager.read(cx);
        assert!(manager.floating_shows(&other, cx));
        assert!(!manager.floating_shows(&s.id, cx));
    });
}

#[gpui::test]
fn note_actions_preserve_tags_when_updating_the_body(cx: &mut TestAppContext) {
    let s = setup(cx);
    let note = call(
        &s,
        "note-create",
        "notes.write",
        json!({"title":"QA note","body":"First","tags":["work","mono"]}),
        cx,
    )
    .try_recv()
    .unwrap()
    .unwrap();
    let updated = call(
        &s,
        "note-update",
        "notes.write",
        json!({"id":note["id"],"title":"QA note","body":"Second"}),
        cx,
    )
    .try_recv()
    .unwrap()
    .unwrap();
    assert_eq!(updated["tags"], json!(["work", "mono"]));
    assert_eq!(updated["body"], "Second");
}

#[gpui::test]
fn identity_and_context_editors_survive_switches_and_save_to_their_own_files(
    cx: &mut TestAppContext,
) {
    let s = setup(cx);
    cx.update(|cx| {
        s.manager.update(cx, |m, cx| {
            m.settings = Some(SettingsSection::Soul);
            let mono = m.current().unwrap().clone();
            let _ = m.soul_page(&mono, cx);
            let key = format!("file:{}:SOUL.md", s.id);
            m.fields[&key].update(cx, |f, cx| {
                f.set_text("# Soul\nKeep replies concise.\n".into(), cx)
            });
            m.save_open_file(cx);
        })
    });
    cx.run_until_parked();
    assert_eq!(
        mono::files::read(&s.root, &s.id, None, "SOUL.md")
            .unwrap()
            .text
            .as_deref(),
        Some("# Soul\nKeep replies concise.\n")
    );
    let saved = Roster::load(&locked(&s.store).unwrap()).unwrap();
    assert_eq!(saved.monos.len(), 1);
    assert_eq!(saved.monos[0].session_id, Some(s.session));
    assert!(saved.hidden_sessions.contains(&s.session));
    let handle: gpui::AnyWindowHandle = s.window.into();
    handle
        .update(cx, |_, w, cx| {
            s.manager
                .update(cx, |m, cx| m.create(vec![s.project.clone()], w, cx))
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        s.manager.update(cx, |m, cx| {
            let mono = m.current().unwrap().clone();
            assert_ne!(mono.id, s.id);
            assert_ne!(mono.mascot, m.roster.get(&s.id).unwrap().mascot);
            m.settings = Some(SettingsSection::Soul);
            let _ = m.soul_page(&mono, cx);
            let key = format!("file:{}:SOUL.md", mono.id);
            assert!(
                !m.fields[&key]
                    .read(cx)
                    .content
                    .contains("Keep replies concise")
            );
        })
    });
}
