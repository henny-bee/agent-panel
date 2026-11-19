use super::*;
use gpui::{TestAppContext, WindowHandle};
use monocode_core::store::Store;
use std::sync::Mutex;

fn conversation(cx: &mut TestAppContext) -> WindowHandle<Conversation> {
    let store = Store::open(Path::new(":memory:")).unwrap();
    let summary = store.create("codex", "/tmp").unwrap();
    cx.update(|cx| {
        input::bind(cx);
        cx.set_global(crate::ui::preferences::Preferences {
            follow_up: "queue".into(),
            ..Default::default()
        });
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| {
                let mut view = Conversation::new(summary, Arc::new(Mutex::new(store)), cx);
                view.loaded = true;
                view
            })
        })
        .unwrap()
    })
}

#[gpui::test]
fn queue_entrances_slide_above_a_stationary_composer_for_the_first_and_later_messages(
    cx: &mut TestAppContext,
) {
    use std::time::Duration;
    let window = conversation(cx);
    cx.run_until_parked();
    window
        .update(cx, |view, _, cx| {
            view.summary.has_messages = true;
            view.busy = true;
            view.follow_ups.state.paused = true;
            cx.notify();
        })
        .unwrap();
    cx.refresh().unwrap();
    let bounds = |window: WindowHandle<Conversation>, cx: &mut TestAppContext| {
        window
            .update(cx, |view, window, _| {
                let composer = window
                    .painted_quads()
                    .into_iter()
                    .find(|quad| {
                        (quad.corner_radii.top_left.0 / window.scale_factor() - 12.).abs() < 0.1
                            && quad.bounds.size.width.0 / window.scale_factor() > 300.
                    })
                    .expect("the composer surface must be painted");
                (
                    f32::from(view.composer_anchor.get().unwrap().top()),
                    composer.bounds.top().0 / window.scale_factor(),
                )
            })
            .unwrap()
    };
    let (mut queue_top, input_top) = bounds(window, cx);
    for (index, text) in ["First queued message", "A second queued message"]
        .into_iter()
        .enumerate()
    {
        window
            .update(cx, |view, _, cx| {
                view.enqueue_follow_up(text.into(), None, cx);
                view.follow_ups.motion.elapsed(Duration::ZERO);
                cx.notify();
            })
            .unwrap();
        cx.refresh().unwrap();
        let (starting, composer) = bounds(window, cx);
        assert!(
            (composer - input_top).abs() < 1.,
            "the input must stay still"
        );
        assert!(
            (starting - queue_top).abs() < 8.,
            "the entrance starts behind the input"
        );

        window
            .update(cx, |view, _, cx| {
                view.follow_ups.motion.elapsed(Duration::from_millis(40));
                cx.notify();
            })
            .unwrap();
        cx.refresh().unwrap();
        let (middle, composer) = bounds(window, cx);
        assert!((composer - input_top).abs() < 1.);
        assert!(middle < starting - 5., "the queue must slide upward");

        window
            .update(cx, |view, _, cx| {
                view.follow_ups.motion.elapsed(Duration::from_secs(1));
                cx.notify();
            })
            .unwrap();
        cx.refresh().unwrap();
        let (settled, composer) = bounds(window, cx);
        assert!((composer - input_top).abs() < 1.);
        assert!(
            settled < middle - 5.,
            "the spring must settle into the full row height"
        );
        assert!(queue_top - settled >= if index == 0 { 60. } else { 30. });
        queue_top = settled;
    }
    cx.update(|cx| cx.set_reduce_motion(true));
    window
        .update(cx, |view, _, cx| {
            view.enqueue_follow_up("Reduced motion".into(), None, cx);
            cx.notify();
        })
        .unwrap();
    cx.refresh().unwrap();
    let (settled, composer) = bounds(window, cx);
    assert!((composer - input_top).abs() < 1.);
    assert!(
        queue_top - settled >= 30.,
        "Reduce Motion must reveal the full row immediately"
    );
}

#[gpui::test]
fn submitted_modes_clear_after_queueing_and_do_not_leak_into_the_next_message(
    cx: &mut TestAppContext,
) {
    for mode in ["operator", "orchestrator", "plan"] {
        let window = conversation(cx);
        cx.run_until_parked();
        window
            .update(cx, |view, _, cx| {
                view.busy = true;
                view.follow_ups.state.paused = true;
                view.toggle_mode(mode, cx);
                let selected = view.input.read(cx).content.clone();
                view.send(cx);
                assert_eq!(
                    view.input.read(cx).content,
                    selected,
                    "an empty submission keeps its mode"
                );
                view.input
                    .update(cx, |i, cx| i.set_text(format!("/{mode} first message"), cx));
                view.send(cx);
                assert!(view.input.read(cx).content.is_empty());
                assert!(
                    !view.settings.plan && !view.settings.operator && !view.settings.orchestrator
                );
                view.input
                    .update(cx, |i, cx| i.set_text("ordinary follow-up".into(), cx));
                view.send(cx);
                let messages = &view.follow_ups.state.messages;
                assert_eq!(messages[0].mode.as_deref(), Some(mode));
                assert_eq!(messages[0].text, "first message");
                assert_eq!(messages[1].mode, None);
                assert_eq!(messages[1].text, "ordinary follow-up");
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |view, _, _| {
                let store = locked(&view.store).unwrap();
                let saved = store.agent_settings(view.summary.id).unwrap().unwrap();
                assert!(!saved.plan && !saved.operator && !saved.orchestrator);
                let queue = store.follow_ups(view.summary.id).unwrap();
                assert_eq!(queue.messages[0].mode.as_deref(), Some(mode));
                assert_eq!(queue.messages[1].mode, None);
            })
            .unwrap();
    }
}

#[gpui::test]
fn busy_sends_keep_their_order_attachments_and_edits_until_accepted(cx: &mut TestAppContext) {
    let window = conversation(cx);
    let (controls, received) = std::sync::mpsc::channel();
    window
        .update(cx, |view, _, cx| {
            view.busy = true;
            view.controls = Some(controls);
            view.input
                .update(cx, |i, cx| i.set_text("first".into(), cx));
            view.attachments.push("/tmp/image.png".into());
            view.send(cx);
            view.input
                .update(cx, |i, cx| i.set_text("second".into(), cx));
            view.send(cx);
            assert!(view.input.read(cx).content.is_empty());
            assert_eq!(
                view.follow_ups
                    .state
                    .messages
                    .iter()
                    .map(|m| m.text.as_str())
                    .collect::<Vec<_>>(),
                ["first", "second"]
            );
        })
        .unwrap();
    cx.run_until_parked();
    assert!(received.try_recv().is_err());
    window
        .update(cx, |view, w, cx| {
            let id = view.follow_ups.state.messages[0].id.clone();
            view.edit_queued(id, w, cx);
            view.follow_ups
                .edit
                .as_ref()
                .unwrap()
                .input
                .update(cx, |i, cx| i.set_text("edited 🙂\nwith detail".into(), cx));
            view.follow_ups.ready = true;
            view.follow_ups.state.messages[0].steer = true;
            view.pump_follow_ups(cx);
            assert!(
                received.try_recv().is_err(),
                "editing the head must block delivery"
            );
            view.finish_queue_edit(true, w, cx);
        })
        .unwrap();
    cx.run_until_parked();
    let control = received.try_recv().unwrap();
    let Decision::Steer(message) = control.decision else {
        panic!("expected steering");
    };
    assert_eq!(message.text, "edited 🙂\nwith detail");
    assert_eq!(
        message.attachments,
        [std::path::PathBuf::from("/tmp/image.png")]
    );
    window
        .update(cx, |view, _, cx| {
            assert_eq!(
                view.follow_ups.state.messages.len(),
                2,
                "a write to the control channel is not an acceptance"
            );
            view.follow_up_delivered(&message.id, cx);
            assert_eq!(view.follow_ups.state.messages[0].text, "second");
            assert!(view.follow_ups.sending.is_none());
        })
        .unwrap();
}

#[gpui::test]
fn stop_pauses_the_queue_and_retires_its_controls(cx: &mut TestAppContext) {
    let window = conversation(cx);
    let (controls, received) = std::sync::mpsc::channel();
    window
        .update(cx, |view, _, cx| {
            view.busy = true;
            view.controls = Some(controls);
            view.input
                .update(cx, |input, cx| input.set_text("keep this".into(), cx));
            view.send(cx);
            view.stop(cx);
            assert!(view.stopping);
            assert!(view.follow_ups.state.paused);
            assert!(!view.follow_ups.ready);
            assert!(view.controls.is_none());
            assert_eq!(view.follow_ups.state.messages[0].text, "keep this");
        })
        .unwrap();
    cx.run_until_parked();
    assert!(matches!(
        received.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Disconnected)
    ));
}

#[gpui::test]
fn a_rejected_steer_keeps_its_text_for_retry(cx: &mut TestAppContext) {
    let window = conversation(cx);
    cx.update(|cx| {
        cx.update_global::<crate::ui::preferences::Preferences, _>(|prefs, _| {
            prefs.follow_up = "steer".into()
        });
    });
    let (controls, received) = std::sync::mpsc::channel();
    window
        .update(cx, |view, _, cx| {
            view.busy = true;
            view.controls = Some(controls);
            view.follow_ups.ready = true;
            view.input
                .update(cx, |input, cx| input.set_text("keep this".into(), cx));
            view.send(cx);
        })
        .unwrap();
    cx.run_until_parked();
    let id = received.try_recv().unwrap().id;
    window
        .update(cx, |view, _, cx| {
            view.fail_follow_up(&id, "Rejected by provider".into(), cx);
            assert!(view.follow_ups.state.paused);
            assert_eq!(view.follow_ups.state.messages[0].text, "keep this");
            assert_eq!(
                view.follow_ups.state.messages[0].error.as_deref(),
                Some("Rejected by provider")
            );
        })
        .unwrap();
}

fn alternate_send(cx: &mut TestAppContext, follow_up: &str, text: &str) -> bool {
    let window = conversation(cx);
    cx.update(|cx| {
        cx.update_global::<crate::ui::preferences::Preferences, _>(|prefs, _| {
            prefs.follow_up = follow_up.into()
        })
    });
    window
        .update(cx, |view, window, cx| {
            view.busy = true;
            view.input
                .update(cx, |input, cx| input.set_text(text.into(), cx));
            window.focus(&view.input.focus_handle(cx), cx);
        })
        .unwrap();
    cx.run_until_parked();
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    cx.simulate_keystrokes(window.into(), &format!("{modifier}-enter"));
    cx.run_until_parked();
    window
        .update(cx, |view, _, cx| {
            assert!(view.input.read(cx).content.is_empty());
            let messages = &view.follow_ups.state.messages;
            assert_eq!(messages.len(), 1);
            assert!(text.ends_with(messages[0].text.as_str()));
            messages[0].steer
        })
        .unwrap()
}

#[gpui::test]
fn the_submit_icon_shows_whether_enter_steers_or_queues(cx: &mut TestAppContext) {
    let window = conversation(cx);
    let action = |cx: &mut TestAppContext, follow_up: &str, alternate: bool| {
        cx.update(|cx| {
            cx.update_global::<crate::ui::preferences::Preferences, _>(|prefs, _| {
                prefs.follow_up = follow_up.into()
            })
        });
        window
            .update(cx, |view, _, cx| view.follow_up_action(alternate, cx))
            .unwrap()
            .map(|(label, _)| label)
    };
    assert_eq!(action(cx, "steer", false), None, "idle sends start a turn");
    window.update(cx, |view, _, _| view.busy = true).unwrap();
    assert_eq!(action(cx, "steer", false), Some("Steer"));
    assert_eq!(action(cx, "steer", true), Some("Queue"));
    assert_eq!(action(cx, "queue", false), Some("Queue"));
    assert_eq!(action(cx, "queue", true), Some("Steer"));
    window
        .update(cx, |view, _, _| view.settings.plan = true)
        .unwrap();
    assert_eq!(
        action(cx, "queue", true),
        Some("Queue"),
        "modes always queue"
    );
}

#[gpui::test]
fn alternate_steer_delivers_past_queued_message(cx: &mut TestAppContext) {
    let window = conversation(cx);
    let (controls, received) = std::sync::mpsc::channel();
    window
        .update(cx, |view, _, cx| {
            view.busy = true;
            view.controls = Some(controls);
            view.follow_ups.ready = true;
            view.input
                .update(cx, |input, cx| input.set_text("queued".into(), cx));
            view.send(cx);
            view.input
                .update(cx, |input, cx| input.set_text("steering".into(), cx));
            view.send_follow_up(true, cx);
        })
        .unwrap();
    cx.run_until_parked();
    let control = received
        .try_recv()
        .expect("steering message must be delivered");
    let Decision::Steer(message) = control.decision else {
        panic!("expected steering")
    };
    assert_eq!(message.text, "steering");
    window
        .update(cx, |view, _, cx| {
            view.follow_up_delivered(&message.id, cx);
            assert_eq!(view.follow_ups.state.messages[0].text, "queued");
        })
        .unwrap();
}

#[gpui::test]
fn plain_steer_delivers_past_queued_message(cx: &mut TestAppContext) {
    let window = conversation(cx);
    cx.update(|cx| {
        cx.update_global::<crate::ui::preferences::Preferences, _>(|prefs, _| {
            prefs.follow_up = "steer".into()
        })
    });
    let (controls, received) = std::sync::mpsc::channel();
    window
        .update(cx, |view, _, cx| {
            view.busy = true;
            view.controls = Some(controls);
            view.follow_ups.ready = true;
            view.input
                .update(cx, |input, cx| input.set_text("queued".into(), cx));
            view.send_follow_up(true, cx);
            view.input
                .update(cx, |input, cx| input.set_text("steering".into(), cx));
            view.send(cx);
        })
        .unwrap();
    cx.run_until_parked();
    let control = received
        .try_recv()
        .expect("steering message must be delivered");
    let Decision::Steer(message) = control.decision else {
        panic!("expected steering")
    };
    assert_eq!(message.text, "steering");
    window
        .update(cx, |view, _, cx| {
            view.follow_up_delivered(&message.id, cx);
            assert_eq!(view.follow_ups.state.messages[0].text, "queued");
        })
        .unwrap();
}

#[gpui::test]
fn modifier_click_inverts_both_follow_up_preferences(cx: &mut TestAppContext) {
    for (preference, expected) in [("steer", false), ("queue", true)] {
        let window = conversation(cx);
        cx.update(|cx| {
            cx.update_global::<crate::ui::preferences::Preferences, _>(|prefs, _| {
                prefs.follow_up = preference.into()
            })
        });
        window
            .update(cx, |view, _, cx| {
                view.summary.has_messages = true;
                view.busy = true;
                view.input
                    .update(cx, |input, cx| input.set_text("message".into(), cx));
                cx.notify();
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.refresh().unwrap();
        let bounds = visual.debug_bounds("send-or-stop").unwrap();
        let modifiers = gpui::Modifiers {
            platform: cfg!(target_os = "macos"),
            control: !cfg!(target_os = "macos"),
            ..Default::default()
        };
        visual.simulate_click(bounds.center(), modifiers);
        visual.run_until_parked();
        window
            .update(visual, |view, _, _| {
                assert_eq!(view.follow_ups.state.messages[0].steer, expected)
            })
            .unwrap();
    }
}

#[gpui::test]
fn cmd_enter_queues_when_follow_ups_steer(cx: &mut TestAppContext) {
    assert!(!alternate_send(cx, "steer", "after this turn"));
}

#[gpui::test]
fn cmd_enter_steers_when_follow_ups_queue(cx: &mut TestAppContext) {
    assert!(alternate_send(cx, "queue", "change course"));
}

#[gpui::test]
fn cmd_enter_cannot_steer_a_mode_turn(cx: &mut TestAppContext) {
    assert!(!alternate_send(cx, "queue", "/plan sketch it first"));
}

#[gpui::test]
fn draft_commands_save_without_starting_an_agent_and_remove_restores_a_blank(
    cx: &mut TestAppContext,
) {
    let window = conversation(cx);
    window
        .update(cx, |view, _, cx| {
            view.input.update(cx, |i, cx| {
                i.set_text(" /DRAFT Explore this later".into(), cx)
            });
            view.attachments.push("/tmp/note.txt".into());
            view.send(cx);
            assert!(!view.busy);
            assert!(view.summary.has_messages);
            assert!(
                view.transcript.read(cx).source_rows().is_empty(),
                "saved drafts must never enter provider history"
            );
            assert_eq!(
                view.follow_ups.state.draft.as_ref().unwrap().text,
                "Explore this later"
            );
            assert!(view.input.read(cx).content.is_empty());
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, _, cx| {
            let saved = locked(&view.store)
                .unwrap()
                .follow_ups(view.summary.id)
                .unwrap();
            assert_eq!(
                saved.draft.unwrap().attachments,
                [std::path::PathBuf::from("/tmp/note.txt")]
            );
            view.input
                .update(cx, |i, cx| i.set_text("my next message".into(), cx));
            view.draft_action(false, cx);
            assert!(!view.summary.has_messages);
            assert_eq!(view.input.read(cx).content, "my next message");
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| {
            assert!(!locked(&view.store).unwrap().sessions().unwrap()[0].has_messages);
        })
        .unwrap();
}

#[gpui::test]
fn sending_a_saved_draft_preserves_the_new_composer(cx: &mut TestAppContext) {
    let window = conversation(cx);
    cx.update(|cx| {
        cx.update_global::<crate::ui::preferences::Preferences, _>(|prefs, _| {
            prefs.follow_up = "steer".into()
        })
    });
    let (controls, received) = std::sync::mpsc::channel();
    window
        .update(cx, |view, _, cx| {
            view.input
                .update(cx, |i, cx| i.set_text("/draft send later".into(), cx));
            view.send(cx);
            view.busy = true;
            view.follow_ups.ready = true;
            view.controls = Some(controls);
            view.input
                .update(cx, |i, cx| i.set_text("keep typing".into(), cx));
            view.attachments.push("/tmp/new-file.txt".into());
            view.draft_action(true, cx);
        })
        .unwrap();
    cx.run_until_parked();
    let control = received.try_recv().unwrap();
    let Decision::Steer(message) = control.decision else {
        panic!("expected saved draft");
    };
    assert_eq!(message.text, "send later");
    assert!(message.attachments.is_empty());
    window
        .update(cx, |view, _, cx| {
            assert_eq!(view.input.read(cx).content, "keep typing");
            assert_eq!(
                view.attachments,
                [std::path::PathBuf::from("/tmp/new-file.txt")]
            );
            assert!(view.follow_ups.state.draft.is_none());
        })
        .unwrap();
}
