use super::*;

#[gpui::test]
fn mono_name_badge_stays_visible_after_the_live_ticker_finishes(cx: &mut TestAppContext) {
    let window = chat(cx);
    window
        .update(cx, |view, _, cx| {
            view.streaming(true, cx);
            view.patch(0, vec![row("user", "Hello"), row("assistant", "")], cx);
            view.entrances.clear();
        })
        .unwrap();
    render(window, cx);
    assert!(
        gpui::VisualTestContext::from_window(window.into(), cx)
            .debug_bounds("mono-signature-pill")
            .is_some()
    );
    window
        .update(cx, |view, _, cx| {
            view.patch(1, vec![row("assistant", "Here is the reply")], cx);
            view.streaming(false, cx);
            assert!(
                view.entrances.contains_key(&1),
                "the response itself still enters"
            );
        })
        .unwrap();
    render(window, cx);
    assert!(
        gpui::VisualTestContext::from_window(window.into(), cx)
            .debug_bounds("mono-signature-pill")
            .is_some()
    );
    assert!(
        gpui::VisualTestContext::from_window(window.into(), cx)
            .debug_bounds("mono-turn-ticker")
            .is_none()
    );
}

#[gpui::test]
fn mono_badge_ticker_tracks_work_at_the_top_of_the_turn(cx: &mut TestAppContext) {
    let window = chat(cx);
    window
        .update(cx, |view, _, cx| {
            view.patch(
                0,
                vec![
                    row("user", "First message"),
                    row("assistant", "Already answered"),
                ],
                cx,
            );
        })
        .unwrap();
    render(window, cx);
    window
        .update(cx, |view, _, cx| view.streaming(true, cx))
        .unwrap();
    render(window, cx);
    {
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        assert!(
            visual.debug_bounds("mono-turn-ticker").is_none(),
            "the completed turn shows no status while the new prompt is being delivered"
        );
    }
    window
        .update(cx, |view, _, cx| {
            view.patch(
                2,
                vec![row("user", "Next message"), row("assistant", "")],
                cx,
            );
            view.entrances.clear();
        })
        .unwrap();
    render(window, cx);
    {
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        assert!(
            visual.debug_bounds("mono-turn-ticker").is_some(),
            "Thinking appears as soon as the live prompt arrives"
        );
    }
    for (role, text, label) in [
        ("assistant", "", "Thinking…"),
        ("reasoning", "Let me consider this.", "Thinking…"),
        ("tool", "{}", "Thinking…"),
        ("assistant", "   ", "Thinking…"),
        ("assistant", "I'll check the files.", "Typing…"),
    ] {
        window
            .update(cx, |view, _, cx| {
                view.patch(3, vec![row(role, text)], cx);
                assert_eq!(view.mono_activity(), Some((3, label.into())));
            })
            .unwrap();
        render(window, cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        assert!(visual.debug_bounds("mono-turn-ticker").is_some());
        assert!(visual.debug_bounds("mono-signature-pill").is_some());
        assert!(visual.debug_bounds("mono-status").is_none());
    }
    // The ticker stays beside the badge above the opening reply.
    for (name, input, label) in [
        (
            "exec_command",
            r#"{"cmd":"cargo check"}"#,
            "Running a build…",
        ),
        ("Read", r#"{"file_path":"notes.md"}"#, "Reading file…"),
        ("Write", r#"{"file_path":"notes.md"}"#, "Editing file…"),
        ("Grep", r#"{"pattern":"Mono"}"#, "Exploring project…"),
    ] {
        window
            .update(cx, |view, _, cx| {
                let tool = ToolCall {
                    id: "work".into(),
                    name: name.into(),
                    input: input.into(),
                    output: String::new(),
                    state: ToolState::Running,
                    panel: None,
                    display: None,
                };
                view.patch(
                    4,
                    vec![row("tool", &serde_json::to_string(&tool).unwrap())],
                    cx,
                );
                view.entrances.clear();
                assert_eq!(view.mono_activity(), Some((3, label.into())));
            })
            .unwrap();
        render(window, cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        let ticker = visual.debug_bounds("mono-turn-ticker").unwrap();
        let opening = visual.debug_bounds("mono-prose-3-16").unwrap();
        assert!(
            ticker.bottom() <= opening.top(),
            "activity stays in the header above the reply"
        );
    }
    window
        .update(cx, |view, _, cx| {
            view.patch(5, vec![row("assistant", "Done.")], cx);
            assert_eq!(view.mono_activity(), Some((3, "Typing…".into())));
            view.streaming(false, cx);
            assert_eq!(view.mono_activity(), None);
            assert!(
                view.rows
                    .iter()
                    .any(|row| row.key.0 == 5 && matches!(row.body, Body::Markdown(_)))
            );
        })
        .unwrap();
    render(window, cx);
    assert!(
        gpui::VisualTestContext::from_window(window.into(), cx)
            .debug_bounds("mono-status")
            .is_none()
    );
    assert!(
        gpui::VisualTestContext::from_window(window.into(), cx)
            .debug_bounds("mono-turn-ticker")
            .is_none()
    );
}

#[gpui::test]
fn mono_reply_dividers_separate_messages_and_let_cards_do_the_job(cx: &mut TestAppContext) {
    for mono in [false, true] {
        for card in [None, Some("document"), Some("session"), Some("choices")] {
            let view = cx.new(|_| {
                let mut view = TranscriptView::new();
                if mono {
                    view.mono_mode("cat", 0xe86b5f, "MonoCat");
                }
                view
            });
            view.update(cx, |view, cx| {
                let mut source = vec![
                    row("user", "Check the result"),
                    row(
                        "assistant",
                        "The first reply.\n\nAnother paragraph in the same reply.",
                    ),
                ];
                if let Some(card) = card {
                    source.push(match card {
                        "document" => {
                            row("mono-document", r#"{"documentId":"doc","title":"Report"}"#)
                        }
                        "session" => row("mono-launch", r#"{"sessionId":7}"#),
                        _ => row(
                            "mono-card",
                            r#"{"type":"choices","title":"Next steps","options":[]}"#,
                        ),
                    });
                }
                source.extend([
                    row("mono-system", "The session finished"),
                    row("assistant", "The follow-up reply.\n\nIts second paragraph."),
                ]);
                for _ in 0..2 {
                    view.patch(0, source.clone(), cx);
                    let dividers = view
                        .rows
                        .iter()
                        .filter(|row| matches!(row.body, Body::MonoDivider))
                        .count();
                    assert_eq!(
                        dividers,
                        usize::from(mono && card.is_none()),
                        "Mono={mono}, ending={card:?}"
                    );
                }
            });
        }
    }
}

#[gpui::test]
fn mono_reply_divider_waits_for_the_reply_and_disappears_when_a_card_arrives(
    cx: &mut TestAppContext,
) {
    let view = cx.new(|_| {
        let mut view = TranscriptView::new();
        view.mono_mode("cat", 0xe86b5f, "MonoCat");
        view
    });
    view.update(cx, |view, cx| {
        let count = |view: &TranscriptView| {
            view.rows
                .iter()
                .filter(|row| matches!(row.body, Body::MonoDivider))
                .count()
        };
        view.patch(
            0,
            vec![
                row("user", "Start a session"),
                row("assistant", "Waiting for it."),
            ],
            cx,
        );
        view.streaming(true, cx);
        view.patch(
            2,
            vec![
                row("mono-system", "Session finished"),
                row("assistant", "The result is ready."),
            ],
            cx,
        );
        assert_eq!(
            count(view),
            0,
            "the live notification has no visible reply yet"
        );
        view.streaming(false, cx);
        assert_eq!(count(view), 1);
        view.patch(
            2,
            vec![
                row("mono-document", r#"{"documentId":"doc","title":"Report"}"#),
                row("mono-system", "Session finished"),
                row("assistant", "The result is ready."),
            ],
            cx,
        );
        assert_eq!(
            count(view),
            0,
            "an arriving card replaces the divider without an exit ghost"
        );
        view.patch(
            view.source.len(),
            vec![
                row("user", "Another question"),
                row("assistant", "Another reply."),
            ],
            cx,
        );
        assert_eq!(
            count(view),
            0,
            "a user message already separates distinct turns"
        );
    });
}

#[gpui::test]
fn mono_reply_divider_paints_a_short_dithered_zigzag(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let window = chat(cx);
    window
        .update(cx, |view, _, cx| {
            view.patch(
                0,
                vec![
                    row("user", "Check it"),
                    row("assistant", "The opening reply.\n\nStill the same reply."),
                    row("tool", "{}"),
                    row("assistant", "The finished reply."),
                ],
                cx,
            );
        })
        .unwrap();
    render(window, cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    let divider = visual.debug_bounds("mono-response-divider").unwrap();
    let opening = visual.debug_bounds("mono-prose-1-17").unwrap();
    let reply = visual.debug_bounds("mono-prose-3-16").unwrap();
    assert!(divider.top() >= opening.bottom());
    assert!(divider.bottom() <= reply.top());
    window
        .update(visual, |_, window, _| {
            let scale = window.scale_factor();
            let ink: Vec<_> = window
                .painted_quads()
                .into_iter()
                .filter(|quad| {
                    let bounds = quad.bounds / scale;
                    bounds.size.height.0 == 3.
                        && bounds.top().0 >= divider.top().as_f32()
                        && bounds.bottom().0 <= divider.bottom().as_f32()
                        && bounds.left().0 >= divider.left().as_f32()
                        && bounds.right().0 <= divider.left().as_f32() + 144.
                })
                .collect();
            assert!(
                ink.len() > 20,
                "the zigzag is drawn in separate chart-sized dither cells"
            );
            let left = ink
                .iter()
                .map(|quad| quad.bounds.left().0 / scale)
                .fold(f32::INFINITY, f32::min);
            let right = ink
                .iter()
                .map(|quad| quad.bounds.right().0 / scale)
                .fold(f32::NEG_INFINITY, f32::max);
            assert!((right - left - 144.).abs() <= 3.);
            assert!(
                ink.iter()
                    .all(|quad| quad.bounds.size.width.0 / scale <= 18.),
                "the fill keeps gaps rather than becoming a solid rule"
            );
        })
        .unwrap();
}

#[gpui::test]
fn mono_notifications_show_only_the_final_message_live_and_after_reload(cx: &mut TestAppContext) {
    let visible = |view: &TranscriptView, at: usize| {
        view.rows
            .iter()
            .any(|row| row.key.0 == at && matches!(row.body, Body::Markdown(_)))
    };
    for history in [false, true] {
        for tools in [false, true] {
            let view = cx.new(|_| {
                let mut view = TranscriptView::new();
                view.mono_mode("cat", 0xe86b5f, "MonoCat");
                view
            });
            let (source, opening, progress, answer, report) = view.update(cx, |view, cx| {
                let previous = if history {
                    vec![
                        row("user", "Review the heartbeat"),
                        row("assistant", "Waiting for the review."),
                    ]
                } else {
                    Vec::new()
                };
                view.patch(0, previous, cx);
                view.streaming(true, cx);
                let prompt = view.source.len();
                let opening = prompt + 1;
                view.patch(
                    prompt,
                    vec![
                        row("mono-system", "The review session finished"),
                        row(
                            "assistant",
                            "I'll verify the findings before summarizing them.",
                        ),
                    ],
                    cx,
                );
                let progress = view.source.len();
                view.patch(
                    progress,
                    vec![row("assistant", "Verifying the findings.")],
                    cx,
                );
                if tools {
                    view.patch(view.source.len(), vec![row("tool", "{}")], cx);
                }
                assert!(view.mono_prose_hidden(opening));
                assert!(!visible(view, opening));
                assert!(!visible(view, progress));
                if history {
                    assert!(visible(view, 1), "the earlier direct reply stays visible");
                }

                let answer = view.source.len();
                view.patch(
                    answer,
                    vec![
                        row("assistant", "The review found three heartbeat bugs.\n\n"),
                        Row {
                            first: false,
                            ..row("assistant", "The full report is in Documents.")
                        },
                    ],
                    cx,
                );
                assert!(
                    !visible(view, answer),
                    "wait for the final message to finish"
                );
                view.streaming(false, cx);
                assert!(!visible(view, opening));
                assert!(!visible(view, progress));
                assert!(visible(view, answer));
                let activity = view.response_rows(view.mono_group_start(opening)).0;
                assert!(
                    activity
                        .iter()
                        .any(|row| row.text.starts_with("I'll verify")),
                    "notification commentary remains in activity history"
                );
                let report = view.source.len() + 1;
                view.patch(
                    report - 1,
                    vec![
                        row("mono-system", "[Scheduled Mono report]"),
                        row("assistant", "The daily check is complete."),
                    ],
                    cx,
                );
                assert!(
                    visible(view, answer),
                    "a later report preserves the earlier final reply"
                );
                assert!(visible(view, report));
                (view.source.clone(), opening, progress, answer, report)
            });

            let reloaded = cx.new(|_| {
                let mut view = TranscriptView::new();
                view.mono_mode("cat", 0xe86b5f, "MonoCat");
                view
            });
            reloaded.update(cx, |view, cx| {
                view.patch(0, source, cx);
                assert!(!visible(view, opening));
                assert!(!visible(view, progress));
                assert!(visible(view, answer), "the saved final reply stays visible");
                assert!(visible(view, report));
                if history {
                    assert!(visible(view, 1));
                }
            });
        }
    }
}

#[gpui::test]
fn a_notification_continuing_a_turn_does_not_move_the_chat(cx: &mut TestAppContext) {
    let window = chat(cx);
    let mut tool = ToolCall::new("launch", "exec_command");
    tool.input = serde_json::json!({
        "cmd": "'/Applications/Monocode Native.app/Contents/MacOS/monocode-gpui' app sessions.start --json '{}'"
    }).to_string();
    tool.output =
        serde_json::json!({"ok": true, "result": {"sessionId": 776, "accepted": true}}).to_string();
    tool.state = ToolState::Completed;
    window
        .update(cx, |view, _, cx| {
            view.set_launches(
                HashMap::from([(
                    776,
                    LaunchView {
                        title: "Luna test".into(),
                        state: "done",
                        harness: "codex".into(),
                        cwd: "/repo".into(),
                    },
                )]),
                cx,
            );
            view.patch(
                0,
                vec![
                    row("user", "Wait for Luna"),
                    row("tool", &serde_json::to_string(&tool).unwrap()),
                    row("assistant", "Waiting for Luna's reply."),
                ],
                cx,
            );
        })
        .unwrap();
    render(window, cx);
    let before = gpui::VisualTestContext::from_window(window.into(), cx)
        .debug_bounds("mono-launch-card-776")
        .expect("a native launch receipt renders its session card");
    window
        .update(cx, |view, _, cx| {
            assert_eq!(view.follow.offset, 0.);
            view.streaming(true, cx);
        })
        .unwrap();
    render(window, cx);
    let waiting = gpui::VisualTestContext::from_window(window.into(), cx)
        .debug_bounds("mono-launch-card-776")
        .expect("the card stays visible before the notification prompt is stored");
    assert!((waiting.top() - before.top()).abs() < px(0.25));
    window
        .update(cx, |view, _, cx| {
            view.patch(
                3,
                vec![row("mono-system", "Luna replied"), row("assistant", "")],
                cx,
            );
        })
        .unwrap();
    render(window, cx);
    let notified = gpui::VisualTestContext::from_window(window.into(), cx)
        .debug_bounds("mono-launch-card-776")
        .expect("the session card remains above the notification's reply");
    assert!((notified.top() - before.top()).abs() < px(0.25));
    window.update(cx, |view, _, _| {
        assert!(view.follow.offset.abs() < 0.25,
            "a hidden notification and replacement footer have no new height, but introduced {}px of movement", view.follow.offset);
    }).unwrap();
    window
        .update(cx, |view, _, cx| {
            view.patch(
                4,
                vec![
                    row("assistant", "I'll check the result."),
                    row("tool", "{}"),
                    row("assistant", "Luna is ready for your test."),
                ],
                cx,
            );
            view.streaming(false, cx);
            let launch = view
                .rows
                .iter()
                .position(|row| matches!(row.body, Body::Launch(776)))
                .unwrap();
            let reply = view
                .rows
                .iter()
                .position(|row| row.key.0 == 6 && matches!(row.body, Body::Markdown(_)))
                .unwrap();
            assert!(
                launch < reply,
                "the completion reply follows the original launch card"
            );
            assert_eq!(
                view.rows
                    .iter()
                    .filter(|row| matches!(row.body, Body::Launch(776)))
                    .count(),
                1
            );
            assert!(
                !view
                    .rows
                    .iter()
                    .any(|row| row.key.0 == 4 && matches!(row.body, Body::Markdown(_)))
            );
        })
        .unwrap();
    // Measure the final spacing with the reply entrance settled.
    cx.update(|cx| cx.set_reduce_motion(true));
    render(window, cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    let card = visual.debug_bounds("mono-launch-card-776").unwrap();
    let opening = visual.debug_bounds("mono-prose-2-16").unwrap();
    let answer = visual.debug_bounds("mono-prose-6-16").unwrap();
    let above = card.top() - opening.bottom();
    let below = answer.top() + px(12.) - card.bottom();
    assert_eq!(above, px(12.));
    assert_eq!(below, above, "the card sits equally far from both replies");
}
