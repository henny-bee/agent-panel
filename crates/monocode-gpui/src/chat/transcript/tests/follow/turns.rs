use super::*;

#[gpui::test]
fn ordinary_send_keeps_the_previous_anchor_until_its_prompt_arrives(cx: &mut TestAppContext) {
    for reading_history in [false, true] {
        let window = chat_mode(cx, false);
        window
            .update(cx, |view, _, cx| {
                view.smooth_reveal = false;
                view.patch(
                    0,
                    vec![
                        row("user", "Earlier question"),
                        row("assistant", &"Earlier answer.\n\n".repeat(30)),
                    ],
                    cx,
                );
                view.streaming(true, cx);
                view.visible = false;
                view.patch(
                    2,
                    vec![row("user", "Previous prompt"), row("assistant", "Done.")],
                    cx,
                );
                view.streaming(false, cx);
                view.visible = true;
            })
            .unwrap();
        render(window, cx);
        let before = window
            .update(cx, |view, _, cx| {
                if reading_history {
                    view.list.scroll_by(px(-80.));
                }
                let before = view.list.logical_scroll_top();
                view.streaming(true, cx);
                assert_eq!(view.anchor_turn, Some(2));
                assert_eq!(view.list.is_following_tail(), !reading_history);
                before
            })
            .unwrap();
        // The background worker may not deliver the saved user row in the
        // same frame as begin_turn. That intermediate frame must stay still.
        render(window, cx);
        window
            .update(cx, |view, _, cx| {
                let after = view.list.logical_scroll_top();
                assert_eq!(after.item_ix, before.item_ix);
                assert_eq!(after.offset_in_item, before.offset_in_item);
                assert_eq!(view.follow.offset, 0.);
                assert!(view.prompt_rise.is_none());
                view.patch(4, vec![row("user", "New prompt")], cx);
                assert_eq!(view.anchor_turn, Some(4));
                assert_eq!(view.prompt_rise.map(|(at, _)| at), Some(4));
                assert!(view.prompt_rise_pending);
                assert!(view.list.is_following_tail());
                assert!(view.follow.measured.is_none());
            })
            .unwrap();
    }
}

#[gpui::test]
fn mono_new_turn_keeps_its_pending_message_below_the_previous_answer(cx: &mut TestAppContext) {
    for reduced in [false, true] {
        cx.update(|cx| cx.set_reduce_motion(reduced));
        let view = cx.new(|_| {
            let mut view = TranscriptView::new();
            view.mono_mode("cat", 0xe86b5f, "MonoCat");
            view
        });
        view.update(cx, |view, cx| {
            view.patch(
                0,
                vec![row("user", "Previous prompt"), row("assistant", "Final answer")],
                cx,
            );
            view.set_pending(
                vec![PendingMessage {
                    id: "next".into(),
                    text: "Next prompt".into(),
                    error: None,
                    sent_at: None,
                    attachments: Vec::new(),
                }],
                cx,
            );
            let assert_order = |view: &TranscriptView| {
                let answer = view.rows.iter().position(|r| {
                    r.key.0 == 1 && matches!(r.body, Body::Markdown(_))
                }).unwrap();
                let pending = view.rows.iter().position(|r| matches!(r.body, Body::Pending(0))).unwrap();
                assert!(pending > answer, "a new turn must stay below the previous final answer, including while delivery starts");
            };
            assert_order(view);
            assert!(!view.stacks_on_next(0), "a new turn does not stack onto the finished prompt");
            view.streaming(true, cx);
            assert_order(view);

            let entrance = view.pending_entrances.get("next").copied();
            view.submit_prompt(
                "Next prompt",
                &monocode_core::session::agent::Settings {
                    mono_message_id: Some("next".into()),
                    ..Default::default()
                },
                cx,
            );
            assert!(!view.rows.iter().any(|r| matches!(r.body, Body::Pending(_))),
                "the optimistic prompt replaces its pending bubble before storage acknowledges it");
            assert_eq!(view.entrances.get(&2).copied(), entrance,
                "delivery must keep the pending bubble's animation clock");
            let keys: Vec<_> = view.rows.iter().map(|r| r.key).collect();
            let accepted = view.source[2..].to_vec();
            view.patch(2, accepted, cx);
            view.set_pending(Vec::new(), cx);
            assert_eq!(view.rows.iter().map(|r| r.key).collect::<Vec<_>>(), keys,
                "storage acceptance must not move or duplicate the prompt");
            assert_eq!(view.mono_activity(), Some((3, "Thinking…".into())));
            assert_eq!(view.entrances.get(&2).copied(), entrance);

            view.set_pending(vec![PendingMessage {
                id: "follow-up".into(), text: "Another detail".into(), error: None,
                sent_at: None, attachments: Vec::new(),
            }], cx);
            assert!(view.stacks_on_next(2));
            let follow_up = view.rows.iter().position(|r| matches!(r.body, Body::Pending(0))).unwrap();
            let footer = view.rows.iter().position(|r| matches!(r.body, Body::Actions { complete: false, .. })).unwrap();
            assert!(follow_up < footer);
        });
    }
}

#[gpui::test]
fn mono_pending_messages_show_thinking_and_keep_the_same_height_on_delivery(
    cx: &mut TestAppContext,
) {
    for reduced in [false, true] {
        cx.update(|cx| cx.set_reduce_motion(reduced));
        for history in [false, true] {
            let window = chat(cx);
            window
                .update(cx, |view, _, cx| {
                    if history {
                        view.patch(
                            0,
                            vec![
                                row("user", "Previous prompt"),
                                row("assistant", "Final answer"),
                            ],
                            cx,
                        );
                    }
                    view.set_pending(
                        vec![PendingMessage {
                            id: "next".into(),
                            text: "Next prompt".into(),
                            error: None,
                            sent_at: None,
                            attachments: Vec::new(),
                        }],
                        cx,
                    );
                    view.pending_entrances.insert(
                        "next".into(),
                        Instant::now() - std::time::Duration::from_secs(1),
                    );
                })
                .unwrap();
            render(window, cx);
            let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
            assert!(
                visual.debug_bounds("mono-turn-ticker").is_some(),
                "a new message shows Thinking before the outbox is saved"
            );
            let span = |visual: &mut gpui::VisualTestContext| {
                let bottom = visual.debug_bounds("mono-turn-ticker").unwrap().bottom();
                let top = window
                    .update(visual, |_, window, _| {
                        let scale = window.scale_factor();
                        window
                            .painted_quads()
                            .into_iter()
                            .filter(|q| {
                                let height = q.bounds.size.height.0 / scale;
                                (35. ..=37.).contains(&height)
                                    && q.corner_radii.top_left.0 * 2. == q.bounds.size.height.0
                            })
                            .map(|q| q.bounds.top().0 / scale)
                            .reduce(f32::max)
                            .expect("the latest user bubble is painted")
                    })
                    .unwrap();
                f32::from(bottom) - top
            };
            let before = span(visual);
            window
                .update(visual, |view, _, cx| {
                    view.streaming(true, cx);
                    view.submit_prompt(
                        "Next prompt",
                        &monocode_core::session::agent::Settings {
                            mono_message_id: Some("next".into()),
                            ..Default::default()
                        },
                        cx,
                    );
                })
                .unwrap();
            render(window, visual);
            assert!(visual.debug_bounds("mono-turn-ticker").is_some());
            let after = span(visual);
            assert!(
                (before - after).abs() < 0.25,
                "delivery must preserve the bubble and Thinking header's height: {before} -> {after}"
            );
        }
    }
}

#[gpui::test]
fn mono_steered_bubbles_align_and_stay_together_after_the_answer_and_reload(
    cx: &mut TestAppContext,
) {
    for reduced in [false, true] {
        cx.update(|cx| cx.set_reduce_motion(reduced));
        let window = chat(cx);
        window
            .update(cx, |view, _, cx| {
                view.smooth_reveal = false;
                view.streaming(true, cx);
                view.patch(
                    0,
                    // The Mono is still thinking: nothing said yet.
                    vec![row("user", "what"), row("assistant", "")],
                    cx,
                );
                view.entrances.clear();
                view.set_pending(
                    vec![PendingMessage {
                        id: "steer".into(),
                        text: "is".into(),
                        error: None,
                        sent_at: None,
                        attachments: Vec::new(),
                    }],
                    cx,
                );
                view.pending_entrances.insert(
                    "steer".into(),
                    Instant::now() - std::time::Duration::from_secs(1),
                );
            })
            .unwrap();
        let assert_alignment = |cx: &mut TestAppContext| {
            render(window, cx);
            window
                .update(cx, |_, window, _| {
                    let scale = window.scale_factor();
                    let mut bubbles: Vec<_> = window
                        .painted_quads()
                        .into_iter()
                        .filter(|q| {
                            (35. ..=37.).contains(&(q.bounds.size.height.0 / scale))
                                && q.corner_radii.top_left.0 * 2. == q.bounds.size.height.0
                        })
                        .map(|q| {
                            (
                                q.bounds.top().0 / scale,
                                q.bounds.bottom().0 / scale,
                                q.bounds.right().0 / scale,
                            )
                        })
                        .collect();
                    bubbles.sort_by(|a, b| a.0.total_cmp(&b.0));
                    assert_eq!(bubbles.len(), 2, "one bubble per user message");
                    assert!(
                        (bubbles[0].2 - bubbles[1].2).abs() < 0.25,
                        "all user bubbles share the same right edge: {bubbles:?}"
                    );
                    assert!(
                        (bubbles[1].0 - bubbles[0].1 - 4.).abs() < 0.25,
                        "consecutive bubbles keep compact spacing: {bubbles:?}"
                    );
                })
                .unwrap();
        };
        let assert_group = |view: &TranscriptView| {
            let users: Vec<_> = view
                .rows
                .iter()
                .enumerate()
                .filter(|(_, row)| matches!(row.body, Body::Plain(_)))
                .map(|(ix, row)| (ix, row.key.0))
                .collect();
            assert_eq!(
                users.iter().map(|(_, at)| *at).collect::<Vec<_>>(),
                vec![0, 2]
            );
            let headers: Vec<_> = view
                .rows
                .iter()
                .enumerate()
                .filter(|(_, row)| matches!(row.body, Body::WorkLine { .. }))
                .collect();
            assert_eq!(headers.len(), 1, "one Mono reply for the whole burst");
            assert!(
                users[1].0 < headers[0].0,
                "both user messages stay before the Mono reply"
            );
            assert_eq!(view.mono_group_start(3), 1);
            let (response, _) = view.response_rows(1);
            assert!(
                response
                    .iter()
                    .any(|row| row.text == "Final answer to both messages")
            );
            assert!(view.stacks_on_next(0));
            assert!(
                !view
                    .rows
                    .iter()
                    .any(|row| row.key.0 == 1 && matches!(row.body, Body::Markdown(_))),
                "nothing was said between the user's messages"
            );
        };
        assert_alignment(cx);
        window
            .update(cx, |view, _, cx| {
                view.patch(
                    2,
                    vec![
                        row("user", "is"),
                        row("assistant", "Final answer to both messages"),
                    ],
                    cx,
                );
                view.set_pending(Vec::new(), cx);
                assert_group(view);
            })
            .unwrap();
        assert_alignment(cx);
        let (rows, receipts) = window
            .update(cx, |view, _, cx| {
                view.streaming(false, cx);
                assert_group(view);
                (
                    view.source.clone(),
                    serde_json::to_string(&view.turns).unwrap(),
                )
            })
            .unwrap();
        assert_alignment(cx);
        let restored = cx.new(|cx| {
            let mut view = TranscriptView::new();
            view.mono_mode("cat", 0xe86b5f, "MonoCat");
            view.turns = serde_json::from_str(&receipts).unwrap();
            view.restore_history(rows, cx);
            view
        });
        restored.update(cx, |view, cx| {
            assert_group(view);
            view.streaming(true, cx);
            view.patch(
                4,
                vec![row("user", "A new conversation turn"), row("assistant", "")],
                cx,
            );
            assert_eq!(
                view.mono_group_start(5),
                5,
                "a message after the final answer starts its own turn"
            );
        });
    }
}

#[gpui::test]
fn a_steer_after_the_mono_spoke_reads_in_order_as_a_new_turn(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let view = cx.new(|_| {
        let mut view = TranscriptView::new();
        view.mono_mode("cat", 0xe86b5f, "MonoCat");
        view
    });
    view.update(cx, |view, cx| {
        // Provider-side the steer joins the running turn; on screen it follows
        // the Mono's opening sentence, and the rest of the reply comes after it.
        view.turns.insert(
            3,
            monocode_core::session::agent::TurnInfo {
                joined_turn: Some(1),
                ..Default::default()
            },
        );
        view.patch(
            0,
            vec![
                row("user", "Check the recent changes"),
                row("assistant", "I'll start a background review."),
                row("user", "thanks"),
                row("assistant", "The review is running now."),
            ],
            cx,
        );
        assert_eq!(view.mono_group_start(3), 3);
        let at = |key: usize, user: bool| {
            view.rows
                .iter()
                .position(|r| {
                    r.key.0 == key
                        && if user {
                            matches!(r.body, Body::Plain(_))
                        } else {
                            matches!(r.body, Body::Markdown(_))
                        }
                })
                .unwrap()
        };
        assert!(at(0, true) < at(1, false));
        assert!(at(1, false) < at(2, true));
        assert!(at(2, true) < at(3, false));
        assert_eq!(
            view.rows
                .iter()
                .filter(|r| matches!(r.body, Body::WorkLine { .. }))
                .count(),
            2,
            "the Mono's words after the steer open a new turn"
        );
        assert!(!view.stacks_on_next(0));
    });
}

#[gpui::test]
fn mono_follow_ups_stay_above_thinking_until_the_final_answer(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let view = cx.new(|_| {
        let mut view = TranscriptView::new();
        view.mono_mode("cat", 0xe86b5f, "MonoCat");
        view
    });
    view.update(cx, |view, cx| {
        view.streaming(true, cx);
        view.patch(
            0,
            vec![row("user", "Original prompt"), row("assistant", "")],
            cx,
        );
        view.set_pending(
            vec![PendingMessage {
                id: "follow-up".into(),
                text: "Another detail".into(),
                error: None,
                sent_at: None,
                attachments: Vec::new(),
            }],
            cx,
        );
        for (text, spoke) in [("", false), ("Working on the answer", true)] {
            view.patch(1, vec![row("assistant", text)], cx);
            let pending = view
                .rows
                .iter()
                .position(|r| matches!(r.body, Body::Pending(0)))
                .unwrap();
            let thinking = view
                .rows
                .iter()
                .position(|r| matches!(r.body, Body::WorkLine { turn: 1, .. }))
                .unwrap();
            if spoke {
                // Once the Mono has said something, the chat reads in order.
                assert!(!view.stacks_on_next(0));
                assert!(
                    pending > thinking,
                    "a message sent after the Mono spoke follows its words"
                );
            } else {
                assert!(
                    view.stacks_on_next(0),
                    "the original bubble keeps compact spacing above its queued follow-up"
                );
                assert!(
                    pending < thinking,
                    "a mid-turn message joins the prompt above Thinking"
                );
            }
            assert_eq!(
                view.rows
                    .iter()
                    .filter(|r| matches!(r.body, Body::Pending(_)))
                    .count(),
                1
            );
        }
        view.streaming(false, cx);
        assert!(!view.stacks_on_next(0));
        let pending = view
            .rows
            .iter()
            .position(|r| matches!(r.body, Body::Pending(0)))
            .unwrap();
        let answer = view
            .rows
            .iter()
            .rposition(|r| matches!(r.body, Body::Markdown(_)))
            .unwrap();
        assert!(
            pending > answer,
            "a message awaiting a new provider turn follows the settled answer"
        );
    });
}
