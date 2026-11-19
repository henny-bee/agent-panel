use super::*;

#[gpui::test]
fn failed_completion_keyboard_recovery_reveals_controls(cx: &mut TestAppContext) {
    let s = setup(cx);
    s.window
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
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    visual.refresh().unwrap();
    s.window
        .update(visual, |_, w, _| w.activate_window())
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    let chat = s.manager.read_with(visual, |m, _| m.chats[&s.id].clone());
    let first_retry = chat.read_with(visual, |c, _| c.queue_test_focus("first", 0));
    let first_discard = chat.read_with(visual, |c, _| c.queue_test_focus("first", 1));
    let second_retry = chat.read_with(visual, |c, _| c.queue_test_focus("second", 0));
    let second_discard = chat.read_with(visual, |c, _| c.queue_test_focus("second", 1));
    s.window
        .update(visual, |_, w, cx| w.focus(&first_retry, cx))
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    s.window
        .update(visual, |_, w, cx| w.focus_prev(cx))
        .unwrap();
    visual.refresh().unwrap();
    let before = visual.debug_bounds("mono-queue-retry-first").unwrap().top();
    visual.simulate_keystrokes("down");
    visual.refresh().unwrap();
    let after = visual.debug_bounds("mono-queue-retry-first").unwrap().top();
    assert!(after < before, "down must scroll the focused failure list");
    s.window
        .update(visual, |_, w, cx| w.focus_next(cx))
        .unwrap();
    s.window
        .update(visual, |_, w, _| assert!(first_retry.is_focused(w)))
        .unwrap();
    s.window
        .update(visual, |_, w, cx| w.focus_next(cx))
        .unwrap();
    s.window
        .update(visual, |_, w, _| assert!(first_discard.is_focused(w)))
        .unwrap();
    s.window
        .update(visual, |_, w, cx| w.focus_next(cx))
        .unwrap();
    visual.refresh().unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    s.window
        .update(visual, |_, w, _| assert!(second_retry.is_focused(w)))
        .unwrap();
    let list = visual.debug_bounds("mono-internal-queue-list").unwrap();
    let retry = visual.debug_bounds("mono-queue-retry-second").unwrap();
    assert!(
        retry.top() >= list.top() && retry.bottom() <= list.bottom(),
        "{retry:?} outside {list:?}"
    );
    s.manager.update(visual, |m, cx| {
        m.roster.get_mut(&s.id).unwrap().outbox.push(Message {
            id: "third".into(),
            text: "completion".into(),
            internal: true,
            sent_at: None,
            error: Some("new failure".into()),
            attachments: Vec::new(),
        });
        m.configure_chat(&s.id, cx);
    });
    visual.run_until_parked();
    visual.refresh().unwrap();
    let list_focus = chat.read_with(visual, |c, _| c.queue_test_list_focus());
    s.window
        .update(visual, |_, w, _| {
            assert!(list_focus.is_focused(w), "focus follows the reset list")
        })
        .unwrap();
    let list = visual.debug_bounds("mono-internal-queue-list").unwrap();
    let first = visual.debug_bounds("mono-queue-retry-first").unwrap();
    assert!(first.top() >= list.top() && first.bottom() <= list.bottom());
    s.manager
        .update(visual, |m, cx| m.queue_changed(&s.id, "third", true, cx));
    visual.run_until_parked();
    visual.refresh().unwrap();
    s.window
        .update(visual, |_, w, cx| w.focus(&second_discard, cx))
        .unwrap();
    s.window
        .update(visual, |_, w, _| assert!(second_discard.is_focused(w)))
        .unwrap();
    visual.refresh().unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    visual.simulate_keystrokes("enter");
    let handle: gpui::AnyWindowHandle = s.window.into();
    handle
        .update(visual, |_, w, cx| {
            w.dispatch_event(
                gpui::PlatformInput::KeyUp(gpui::KeyUpEvent {
                    keystroke: gpui::Keystroke::parse("enter").unwrap(),
                }),
                cx,
            );
        })
        .unwrap();
    visual.run_until_parked();
    assert!(!s.manager.read_with(visual, |m, _| {
        m.roster
            .get(&s.id)
            .unwrap()
            .outbox
            .iter()
            .any(|m| m.id == "second")
    }));
    visual.refresh().unwrap();
    let list = visual.debug_bounds("mono-internal-queue-list").unwrap();
    let retry = visual.debug_bounds("mono-queue-retry-first").unwrap();
    assert!(
        retry.top() >= list.top() && retry.bottom() <= list.bottom(),
        "remaining action {retry:?} outside {list:?}"
    );
    chat.update(visual, |c, _| c.busy = true);
    s.window
        .update(visual, |_, w, cx| w.focus(&first_retry, cx))
        .unwrap();
    visual.refresh().unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    handle
        .update(visual, |_, w, cx| {
            w.dispatch_event(
                gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                    keystroke: gpui::Keystroke::parse("space").unwrap(),
                    is_held: false,
                    prefer_character_input: false,
                }),
                cx,
            );
            w.dispatch_event(
                gpui::PlatformInput::KeyUp(gpui::KeyUpEvent {
                    keystroke: gpui::Keystroke::parse("space").unwrap(),
                }),
                cx,
            );
        })
        .unwrap();
    visual.run_until_parked();
    assert!(s.manager.read_with(visual, |m, _| {
        m.roster
            .get(&s.id)
            .unwrap()
            .outbox
            .iter()
            .any(|m| m.id == "first" && m.error.is_none())
    }));
    s.manager.update(visual, |m, cx| {
        m.roster.get_mut(&s.id).unwrap().outbox[0].error = Some("Long failure. ".repeat(500));
        m.configure_chat(&s.id, cx);
    });
    visual.run_until_parked();
    visual.refresh().unwrap();
    let first_retry = chat.read_with(visual, |c, _| c.queue_test_focus("first", 0));
    s.window
        .update(visual, |_, w, cx| w.focus(&first_retry, cx))
        .unwrap();
    visual.refresh().unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    s.window
        .update(visual, |_, w, cx| w.focus_prev(cx))
        .unwrap();
    visual.refresh().unwrap();
    visual.simulate_keystrokes("pagedown");
    visual.refresh().unwrap();
    let list = visual.debug_bounds("mono-internal-queue-list").unwrap();
    let retry = visual.debug_bounds("mono-queue-retry-first").unwrap();
    assert!(
        retry.top() < list.top(),
        "test must scroll before clearing the queue"
    );
    s.manager
        .update(visual, |m, cx| m.queue_changed(&s.id, "first", true, cx));
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("mono-internal-queue-list").is_none());
    let input_focus = chat.read_with(visual, |c, cx| c.input.focus_handle(cx));
    s.window
        .update(visual, |_, w, _| assert!(input_focus.is_focused(w)))
        .unwrap();
    s.manager.update(visual, |m, cx| {
        m.roster.get_mut(&s.id).unwrap().outbox.push(Message {
            id: "new".into(),
            text: "completion".into(),
            internal: true,
            sent_at: None,
            error: Some("Long failure. ".repeat(500)),
            attachments: Vec::new(),
        });
        m.configure_chat(&s.id, cx);
    });
    visual.run_until_parked();
    visual.refresh().unwrap();
    let list = visual.debug_bounds("mono-internal-queue-list").unwrap();
    let retry = visual.debug_bounds("mono-queue-retry-new").unwrap();
    assert!(
        retry.top() >= list.top() && retry.bottom() <= list.bottom(),
        "new action {retry:?} outside {list:?}"
    );
}

#[gpui::test]
fn sidebar_hidden_launches_still_list_in_the_mono_panel_after_a_restart(cx: &mut TestAppContext) {
    let s = setup(cx);
    let started = call(
        &s,
        "hidden-launch",
        "sessions.start",
        json!({"project":s.project,"prompt":"Review quietly","title":"Quiet review","draft":true}),
        cx,
    )
    .try_recv()
    .unwrap()
    .unwrap();
    let child = started["sessionId"].as_i64().unwrap();
    let owner = cx.update(|cx| s.manager.read(cx).owner.upgrade().unwrap());
    cx.update(|cx| {
        assert!(s.manager.read(cx).hidden_session(child));
        owner.update(cx, |shell, _| shell.mono_forget_session(child));
        assert!(
            !owner
                .read(cx)
                .mono_session_views(cx)
                .iter()
                .any(|v| v.id == child)
        );
    });
    s.manager.update(cx, |m, cx| m.configure_chat(&s.id, cx));
    cx.run_until_parked();
    cx.update(|cx| {
        let views = owner.read(cx).mono_session_views(cx);
        let view = views
            .iter()
            .find(|v| v.id == child)
            .expect("the launch lists again");
        assert!(view.title.contains("Quiet review"), "{}", view.title);
        assert!(
            owner
                .update(cx, |shell, cx| shell.mono_ensure_session(child, cx))
                .is_some()
        );
        assert!(s.manager.read(cx).hidden_session(child));
    });
}

#[gpui::test]
fn mono_app_launch_visibility_can_be_toggled_and_survives_roster_reload(cx: &mut TestAppContext) {
    let s = setup(cx);
    let started = call(
        &s,
        "visibility-draft",
        "sessions.start",
        json!({"project":s.project,"prompt":"Review quietly","draft":true}),
        cx,
    )
    .try_recv()
    .unwrap()
    .unwrap();
    let child = started["sessionId"].as_i64().unwrap();
    cx.update(|cx| assert!(s.manager.read(cx).hidden_session(child)));
    s.manager
        .update(cx, |m, cx| m.sidebar_preferences(&s.id, true, true, cx));
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!s.manager.read(cx).hidden_session(child));
        assert!(s.manager.read(cx).is_mono_session(s.session));
    });
    assert!(
        monocode_core::session::mono_model::Roster::load(&locked(&s.store).unwrap())
            .unwrap()
            .get(&s.id)
            .unwrap()
            .show_started_sessions_in_sidebar
    );
    s.manager
        .update(cx, |m, cx| m.sidebar_preferences(&s.id, false, false, cx));
    cx.run_until_parked();
    cx.update(|cx| assert!(s.manager.read(cx).hidden_session(child)));
    assert!(
        !monocode_core::session::mono_model::Roster::load(&locked(&s.store).unwrap())
            .unwrap()
            .get(&s.id)
            .unwrap()
            .show_started_sessions_in_sidebar
    );
}

#[gpui::test]
fn queued_messages_are_saved_before_dispatch_and_do_not_run_while_blocked(cx: &mut TestAppContext) {
    let s = setup(cx);
    let message = mono::new_id();
    cx.update(|cx| {
        s.manager.update(cx, |m, cx| {
            m.roster.get_mut(&s.id).unwrap().outbox.push(Message {
                sent_at: None,
                id: message.clone(),
                text: "Keep this queued".into(),
                internal: false,
                error: Some("Retry required".into()),
                attachments: Vec::new(),
            });
            m.save(cx);
            m.configure_chat(&s.id, cx);
        })
    });
    cx.run_until_parked();
    let saved = Roster::load(&locked(&s.store).unwrap()).unwrap();
    assert_eq!(saved.monos[0].outbox[0].id, message);
    assert_eq!(
        locked(&s.store)
            .unwrap()
            .load(s.session)
            .unwrap()
            .rows
            .len(),
        0
    );
}
