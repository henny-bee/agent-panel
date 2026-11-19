use super::*;

#[gpui::test]
fn app_receipt_lookup_does_not_wait_for_storage_on_the_ui_thread(cx: &mut TestAppContext) {
    let s = setup(cx);
    let store = s.store.clone();
    let (held, acquired) = mpsc::sync_channel(0);
    let (release, released) = mpsc::channel();
    let holder = std::thread::spawn(move || {
        let _guard = store.lock().unwrap();
        held.send(()).unwrap();
        // A regression must fail rather than hang the whole test suite.
        released.recv_timeout(Duration::from_secs(2)).is_ok()
    });
    acquired.recv().unwrap();
    let (reply, response) = mpsc::channel();
    let call = mono::bridge::Call {
        session: s.session,
        request_id: "contended-lookup".into(),
        action: "sessions.list".into(),
        input: json!({}),
        reply,
    };
    s.window
        .update(cx, |_, window, cx| {
            s.manager
                .update(cx, |manager, cx| manager.app_call(call, window, cx));
        })
        .unwrap();
    assert!(response.try_recv().is_err());
    let _ = release.send(());
    assert!(
        holder.join().unwrap(),
        "app request blocked the UI waiting for the store"
    );
    cx.run_until_parked();
    assert!(response.try_recv().unwrap().is_ok());
}

#[gpui::test]
fn durable_app_receipts_survive_cache_eviction_and_invalid_receipts_do_not_repeat_mutations(
    cx: &mut TestAppContext,
) {
    let s = setup(cx);
    let input = json!({"name":"Digest","instructions":"Summarize changes","schedule":{"kind":"daily","time":"09:00"}});
    let first = call(&s, "durable-habit", "habits.add", input.clone(), cx)
        .try_recv()
        .unwrap()
        .unwrap();
    s.manager.update(cx, |m, _| m.app_results.clear());
    let retry = call(&s, "durable-habit", "habits.add", input, cx)
        .try_recv()
        .unwrap()
        .unwrap();
    assert_eq!(first, retry);
    assert_eq!(habits::load(&s.root, &s.id).unwrap().len(), 1);
    assert!(
        call(
            &s,
            "durable-habit",
            "habits.add",
            json!({"name":"different"}),
            cx
        )
        .try_recv()
        .unwrap()
        .is_err()
    );
    let key = format!(
        "mono-app-receipt:{}:{}",
        s.session,
        mono::files::content_hash("corrupt-habit")
    );
    locked(&s.store)
        .unwrap()
        .save_setting(&key, "invalid receipt")
        .unwrap();
    let error = call(&s, "corrupt-habit", "habits.add", json!({"name":"Another","instructions":"Summarize changes","schedule":{"kind":"daily","time":"09:00"}}), cx).try_recv().unwrap().unwrap_err();
    assert!(error.contains("Could not read app result"));
    assert_eq!(habits::load(&s.root, &s.id).unwrap().len(), 1);
}

#[gpui::test]
fn app_actions_are_project_scoped_and_mutation_retries_are_idempotent(cx: &mut TestAppContext) {
    let s = setup(cx);
    assert!(
        call(
            &s,
            "outside",
            "sessions.list",
            json!({"project":"/not-assigned"}),
            cx
        )
        .try_recv()
        .unwrap()
        .is_err()
    );
    let input = json!({"name":"Morning digest","instructions":"Summarize changes without editing","schedule":{"kind":"daily","time":"09:00"}});
    let first = call(&s, "habit", "habits.add", input.clone(), cx)
        .try_recv()
        .unwrap()
        .unwrap();
    let retry = call(&s, "habit", "habits.add", input, cx)
        .try_recv()
        .unwrap()
        .unwrap();
    assert_eq!(first, retry);
    assert_eq!(habits::load(&s.root, &s.id).unwrap().len(), 1);
    assert!(
        call(&s, "habit", "habits.add", json!({"name":"different"}), cx)
            .try_recv()
            .unwrap()
            .is_err()
    );
    let start=call(&s,"draft","sessions.start",json!({"project":s.project,"prompt":"Review the project","title":"Review draft","draft":true}),cx).try_recv().unwrap().unwrap();
    let child = start["sessionId"].as_i64().unwrap();
    cx.update(|cx| {
        let m = s.manager.read(cx);
        assert_eq!(m.roster.get(&s.id).unwrap().launches.len(), 1);
        let owner = m.owner.upgrade().unwrap();
        let view = owner.read(cx).mono_session_entity(child).unwrap();
        assert!(view.read(cx).has_saved_draft());
        assert_eq!(
            locked(&s.store)
                .unwrap()
                .follow_ups(child)
                .unwrap()
                .draft
                .unwrap()
                .text,
            "Review the project"
        );
        assert!(!view.read(cx).busy);
        assert!(m.hidden_session(child));
    });
}

#[gpui::test]
fn mono_launches_ignore_tab_reveal_and_keep_the_current_chat(cx: &mut TestAppContext) {
    let s = setup(cx);
    let before = s
        .window
        .update(cx, |shell, _, _| shell.workspace.open_sessions())
        .unwrap();
    for show in [false, true] {
        s.manager
            .update(cx, |m, cx| m.sidebar_preferences(&s.id, show, false, cx));
        let started = call(
            &s,
            if show {
                "listed-reveal"
            } else {
                "hidden-reveal"
            },
            "sessions.start",
            json!({"project":s.project,"prompt":"Test quietly","draft":true,"reveal":true}),
            cx,
        )
        .try_recv()
        .unwrap()
        .unwrap();
        let child = started["sessionId"].as_i64().unwrap();
        s.window
            .update(cx, |shell, _, cx| {
                assert_eq!(
                    shell.workspace.open_sessions(),
                    before,
                    "a Mono launch must not create or select a tab"
                );
                assert!(
                    shell.mono_session_entity(child).is_some(),
                    "the background session still exists"
                );
                let m = s.manager.read(cx);
                assert_eq!(m.active.as_deref(), Some(s.id.as_str()));
                assert_eq!(m.hidden_session(child), !show);
            })
            .unwrap();
    }
}

#[gpui::test]
fn internal_completion_queue_stays_hidden_until_delivery_fails(cx: &mut TestAppContext) {
    let s = setup(cx);
    let handle: gpui::AnyWindowHandle = s.window.into();
    handle
        .update(cx, |_, window, cx| {
            s.manager.update(cx, |m, cx| {
                m.dismiss_settings(window, cx);
                m.roster.get_mut(&s.id).unwrap().outbox.push(Message {
                    id: "completion".into(),
                    text: "[Delegated work completed]".into(),
                    internal: true,
                    sent_at: None,
                    error: None,
                    attachments: Vec::new(),
                });
                // Keep the notification queued without starting a provider.
                m.configure_chat(&s.id, cx);
            });
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(
        visual.debug_bounds("mono-internal-queue").is_none(),
        "a routine completion must not flash a Pending card in the composer"
    );
    assert!(
        visual.debug_bounds("mono-internal-queue-list").is_none(),
        "no invisible queue tab stop"
    );
    s.manager.update(visual, |m, cx| {
        m.roster.get_mut(&s.id).unwrap().outbox[0].error = Some("Delivery failed".into());
        m.configure_chat(&s.id, cx);
    });
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(
        visual.debug_bounds("mono-internal-queue").is_some(),
        "a failed notification keeps its retry and discard controls"
    );
}

#[gpui::test]
fn long_completion_error_keeps_composer_in_view(cx: &mut TestAppContext) {
    let s = setup(cx);
    let handle: gpui::AnyWindowHandle = s.window.into();
    handle
        .update(cx, |_, window, cx| {
            s.manager.update(cx, |m, cx| {
                m.dismiss_settings(window, cx);
                m.roster.get_mut(&s.id).unwrap().outbox.push(Message {
                    id: "completion".into(),
                    text: "completion".into(),
                    internal: true,
                    sent_at: None,
                    error: Some("Long failure message. ".repeat(500)),
                    attachments: Vec::new(),
                });
                m.configure_chat(&s.id, cx);
            });
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    visual.refresh().unwrap();
    let composer = visual.debug_bounds("mono-composer").unwrap();
    let viewport = s
        .window
        .update(visual, |_, w, _| w.viewport_size())
        .unwrap();
    assert!(
        composer.bottom() <= viewport.height,
        "composer {composer:?}, viewport {viewport:?}"
    );
}
