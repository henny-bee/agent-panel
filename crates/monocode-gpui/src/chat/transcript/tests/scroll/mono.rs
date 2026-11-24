use super::*;

#[gpui::test]
fn mono_turn_actions_stay_while_moving_between_its_rows_in_any_event_order(
    cx: &mut TestAppContext,
) {
    let view = cx.new(|_| {
        let mut view = TranscriptView::new();
        view.mono_mode("cat", 0xe86b5f, "MonoCat");
        view
    });
    view.update(cx, |view, cx| {
        view.patch(
            0,
            vec![row("user", "Hi"), row("assistant", "Hello there")],
            cx,
        );
        // The next row reports its enter before the last one leaves.
        view.mono_hover(false, 1, 1, (1, 16), true, cx);
        view.mono_hover(false, 1, 1, (1, usize::MAX - 1), true, cx);
        view.mono_hover(false, 1, 1, (1, 16), false, cx);
        assert_eq!(view.hovered_turn.map(|(t, _)| t), Some(1));
        view.mono_hover(false, 1, 1, (1, usize::MAX - 1), false, cx);
        assert!(view.hovered_turn.is_none());
    });
}

#[gpui::test]
fn mono_messages_stack_past_a_reaction_and_queued_bubbles_hand_over(cx: &mut TestAppContext) {
    let view = cx.new(|_| {
        let mut view = TranscriptView::new();
        view.mono_mode("cat", 0xe86b5f, "MonoCat");
        view
    });
    view.update(cx, |view, cx| {
        view.patch(
            0,
            vec![
                row("user", "nice"),
                row("assistant", "<reaction>👍</reaction>"),
                row("user", "what else"),
            ],
            cx,
        );
        assert!(
            view.stacks_on_next(0),
            "a reaction-only reply keeps the stack"
        );
        assert!(!view.stacks_on_next(2));
        view.set_pending(
            vec![PendingMessage {
                id: "q".into(),
                text: "Queued".into(),
                error: None,
                sent_at: None,
                attachments: Vec::new(),
            }],
            cx,
        );
        assert!(matches!(view.rows.last().unwrap().body, Body::Pending(0)));
        let born = view.pending_entrances["q"];
        view.streaming(true, cx);
        view.patch(3, vec![row("user", "Queued")], cx);
        view.set_pending(Vec::new(), cx);
        assert_eq!(
            view.entrances.get(&3),
            Some(&born),
            "delivery continues the same entrance without restarting it"
        );
        assert!(!view.rows.iter().any(|r| matches!(r.body, Body::Pending(_))));
        assert_eq!(view.list.item_count(), view.rows.len() + 1);
    });
}

#[gpui::test]
fn mono_turns_merge_follow_ups_hide_live_prose_and_queue_above_the_work(cx: &mut TestAppContext) {
    let view = cx.new(|_| {
        let mut view = TranscriptView::new();
        view.mono_mode("cat", 0xe86b5f, "MonoCat");
        view
    });
    let start = serde_json::to_string(&ToolCall {
        id: "t".into(),
        name: "exec_command".into(),
        input: r#"{"cmd":"'/Applications/Monocode Native.app/Contents/MacOS/monocode-gpui' app sessions.start --json '{}'"}"#.into(),
        output: r#"{"ok":true,"result":{"sessionId":776,"accepted":true}}"#.into(),
        state: ToolState::Completed,
        panel: None,
        display: None,
        images: Vec::new(),
    })
    .unwrap();
    view.update(cx, |view, cx| {
        view.patch(
            0,
            vec![
                row("user", "Start a Luna session"),
                row("assistant", "I'll start a Luna session."),
                row("tool", &start),
                row("assistant", "Started session 776."),
                row("mono-system", "Session 776 finished"),
                row("assistant", "Luna replied."),
            ],
            cx,
        );
        let headers = view
            .rows
            .iter()
            .filter(|r| matches!(r.body, Body::WorkLine { .. }))
            .count();
        let actions: Vec<_> = view
            .rows
            .iter()
            .filter_map(|r| match r.body {
                Body::Actions { turn, .. } => Some(turn),
                _ => None,
            })
            .collect();
        assert_eq!(
            headers, 1,
            "the follow-up continues the turn under one header"
        );
        assert_eq!(
            actions,
            vec![1],
            "one set of actions, at the end of the group"
        );
        assert!(
            !view.mono_prose_hidden(1),
            "the opening sentence shows before the work"
        );
        assert!(!view.mono_prose_hidden(3));
        assert!(
            view.rows
                .iter()
                .any(|r| matches!(r.body, Body::Launch(776))),
            "the accepted launch gets its card"
        );
        view.streaming(true, cx);
        assert!(
            view.rows
                .iter()
                .any(|r| matches!(r.body, Body::Launch(776))),
            "starting the next provider run must keep the existing launch card"
        );
        view.patch(6, vec![row("user", "And another thing")], cx);
        view.patch(7, vec![row("assistant", "Working on it")], cx);
        assert!(
            view.mono_prose_hidden(7),
            "a Mono message appears whole, not while it streams"
        );
        view.patch(8, vec![row("tool", &start)], cx);
        assert!(
            !view.mono_prose_hidden(7),
            "the finished opening sentence shows while the Mono works"
        );
        view.set_pending(
            vec![PendingMessage {
                id: "q".into(),
                text: "One more".into(),
                error: None,
                sent_at: None,
                attachments: Vec::new(),
            }],
            cx,
        );
        let pending = view
            .rows
            .iter()
            .position(|r| matches!(r.body, Body::Pending(0)))
            .unwrap();
        let header = view
            .rows
            .iter()
            .rposition(|r| matches!(r.body, Body::WorkLine { .. }))
            .unwrap();
        assert!(
            pending > header,
            "a message queued after the Mono spoke follows its words"
        );
    });
}

#[gpui::test]
fn desktop_work_metadata_renders_once_after_the_reply_and_never_as_prose(cx: &mut TestAppContext) {
    let view = cx.new(|_| {
        let mut view = TranscriptView::new();
        view.mono_mode("cat", 0xe86b5f, "MonoCat");
        view
    });
    view.update(cx, |view,cx| {
        view.patch(0,vec![
            row("user","Write a report"),
            row("mono-document",r#"{"id":"artifact:report","type":"document","documentId":"report","title":"Report","kind":"html"}"#),
            row("mono-launch",r#"{"sessionId":7,"title":"Review","cwd":"/repo","harness":"codex"}"#),
            row("assistant","Written and checked."),
        ],cx);
        assert_eq!(view.rows.iter().filter(|r| matches!(r.body,Body::MonoCard(_))).count(),1);
        assert_eq!(view.rows.iter().filter(|r| matches!(r.body,Body::Launch(7))).count(),1);
        assert!(!view.rows.iter().any(|r| matches!(&r.body,Body::Plain(text) if text.contains("documentId") || text.contains("sessionId"))));
        assert!(!view.rows.iter().any(|r| r.work));
        let answer=view.rows.iter().position(|r| matches!(r.body,Body::Markdown(_))).unwrap();
        let document=view.rows.iter().position(|r| matches!(r.body,Body::MonoCard(_))).unwrap();
        assert!(document > answer);
    });
}

#[gpui::test]
fn mono_document_and_session_cards_are_one_line_like_react(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let start = serde_json::to_string(&ToolCall {
        id: "t".into(),
        name: "Bash".into(),
        input: r#"{"command":"monocode app sessions.start --json {}"}"#.into(),
        output: r#"{"ok":true,"result":{"sessionId":7,"accepted":true}}"#.into(),
        state: ToolState::Completed,
        panel: None,
        display: None,
        images: Vec::new(),
    })
    .unwrap();
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(900.), px(600.)),
                ))),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|cx| {
                    let mut view = TranscriptView::new();
                    view.mono_mode("cat", 0xe86b5f, "MonoCat");
                    view.patch(
                        0,
                        vec![
                            row("user", "Start one and write it up"),
                            row("tool", &start),
                            row("assistant", "Started it."),
                            row(
                                "mono-card",
                                r#"{"id":"c","type":"document","documentId":"d","title":"Test Document"}"#,
                            ),
                        ],
                        cx,
                    );
                    view.set_launches(
                        HashMap::from([(
                            7,
                            LaunchView {
                                title: "Fix the composer".into(),
                                state: "working",
                                harness: "codex".into(),
                                cwd: "/tmp/project".into(),
                            },
                        )]),
                        cx,
                    );
                    view
                })
            },
        )
        .unwrap()
    });
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let document = visual.debug_bounds("mono-artifact-card").unwrap();
    let session = visual.debug_bounds("mono-launch-card-7").unwrap();
    // ArtifactCard: min-h-[38px] and w-fit; LaunchedSessionCard: py-1.5
    // around its project chip, both at most 24rem wide.
    assert_eq!(document.size.height, px(38.));
    assert!(document.size.width < px(384.));
    assert_eq!(session.size.height, px(40.));
    assert!(session.size.width <= px(384.));
}

#[gpui::test]
fn mono_chat_rests_on_the_composer_and_slides_sent_messages_in(cx: &mut TestAppContext) {
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(900.), px(600.)),
                ))),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| {
                    let mut view = TranscriptView::new();
                    view.mono_mode("cat", 0xe86b5f, "MonoCat");
                    view
                })
            },
        )
        .unwrap()
    });
    let bubble_top = |w: &mut Window| {
        w.painted_quads()
            .into_iter()
            .filter(|q| {
                q.corner_radii.top_left.0 > 0.
                    && q.corner_radii.top_left.0 * 2. == q.bounds.size.height.0
            })
            .map(|q| q.bounds.top().0 / w.scale_factor())
            .fold(None, |low: Option<f32>, top| {
                Some(low.map_or(top, |l| l.max(top)))
            })
            .expect("single-line bubble is fully rounded")
    };
    // Loaded history neither animates nor starts at the top.
    window
        .update(cx, |view, _, cx| {
            view.patch(0, vec![row("user", "Earlier")], cx)
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, w, _| {
            assert!(view.entrances.is_empty(), "history must not slide in");
            assert!(
                bubble_top(w) > 400.,
                "a short Mono chat sits at the bottom, like justify-end"
            );
        })
        .unwrap();
    window
        .update(cx, |view, _, cx| {
            view.patch(
                0,
                vec![
                    row("user", "Hello"),
                    row("assistant", "<reaction>👍</reaction>"),
                ],
                cx,
            );
            view.streaming(true, cx);
            view.patch(2, vec![row("user", "A sent line")], cx);
            assert!(view.entrances.contains_key(&2), "the sent message enters");
            assert_eq!(view.reactions.get(&0).map(|r| r.0.as_str()), Some("👍"));
            view.entrances
                .insert(2, Instant::now() - std::time::Duration::from_millis(10));
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    let entering = window
        .update(cx, |view, w, _| {
            assert!(
                view.follow.offset > 0.,
                "the chat glides up to the sent message instead of jumping"
            );
            bubble_top(w)
        })
        .unwrap();
    window
        .update(cx, |view, _, cx| {
            view.entrances.clear();
            cx.notify();
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    let settled = window.update(cx, |_, w, _| bubble_top(w)).unwrap();
    assert!(
        entering > settled + 10.,
        "a sent message rises into place ({entering} vs {settled})"
    );
}
