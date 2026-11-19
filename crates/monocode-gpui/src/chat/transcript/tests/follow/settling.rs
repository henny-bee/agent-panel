use super::*;

#[gpui::test]
fn optimistic_prompt_anchors_once_and_worker_acceptance_keeps_its_entrance(
    cx: &mut TestAppContext,
) {
    let window = chat_mode(cx, false);
    window
        .update(cx, |view, _, cx| {
            view.patch(
                0,
                vec![
                    row("user", "Earlier question"),
                    row("assistant", "Earlier answer"),
                ],
                cx,
            );
            view.visible = true;
            view.streaming(true, cx);
            view.submit_prompt(
                "New question",
                &monocode_core::session::agent::Settings::default(),
                cx,
            );
            assert_eq!(view.anchor_turn, Some(2));
            assert_eq!(view.introduced_prompt, Some(2));
            assert!(
                view.rows
                    .iter()
                    .any(|row| matches!(&row.body, Body::Plain(text) if text == "New question"))
            );
            assert!(
                view.rows
                    .iter()
                    .any(|row| matches!(row.body, Body::WorkLine { .. }))
            );
            let entrance = view.prompt_rise;
            assert!(entrance.is_some());
            let accepted = view.source[2..].to_vec();
            view.patch(2, accepted, cx);
            assert_eq!(view.source.len(), 4);
            assert_eq!(
                view.prompt_rise, entrance,
                "saving must not restart the message animation"
            );
            assert_eq!(view.anchor_turn, Some(2));
        })
        .unwrap();
}

// Patch with row entrances disabled to isolate follow motion from disclosures.
#[gpui::test]
fn ordinary_short_replies_fill_the_prompt_spacer_without_moving_the_chat(cx: &mut TestAppContext) {
    let window = chat_mode(cx, false);
    window
        .update(cx, |view, _, cx| {
            view.smooth_reveal = false;
            view.streaming(true, cx);
            view.visible = false;
            view.patch(
                0,
                vec![row("user", "Prompt"), row("assistant", "Short reply.")],
                cx,
            );
            view.visible = true;
            view.prompt_rise = None;
        })
        .unwrap();
    render(window, cx);
    window
        .update(cx, |view, _, cx| {
            view.visible = false;
            view.patch(
                1,
                vec![row("assistant", "Short reply.\n\nAnother paragraph.")],
                cx,
            );
            view.visible = true;
        })
        .unwrap();
    render(window, cx);
    window
        .update(cx, |view, _, _| {
            assert!(
                view.follow.offset.abs() < 0.25,
                "text should consume the spacer before the prompt moves"
            );
            assert!(view.follow.measured.as_ref().unwrap().footer > BOTTOM_INSET);
        })
        .unwrap();
}

#[gpui::test]
fn ordinary_overflow_glides_and_keeps_settling_after_streaming_ends(cx: &mut TestAppContext) {
    let window = chat_mode(cx, false);
    window
        .update(cx, |view, _, cx| {
            view.anchor_prompts = false;
            view.smooth_reveal = false;
            view.streaming(true, cx);
            view.visible = false;
            view.patch(
                0,
                vec![row("user", "Prompt"), row("assistant", "A short reply.")],
                cx,
            );
            view.visible = true;
        })
        .unwrap();
    render(window, cx);
    window
        .update(cx, |view, _, cx| {
            view.visible = false;
            view.patch(
                1,
                vec![row("assistant", &"Another paragraph.\n\n".repeat(35))],
                cx,
            );
            view.visible = true;
        })
        .unwrap();
    render(window, cx);
    let before = window
        .update(cx, |view, _, _| {
            assert!(view.follow.offset > 0.);
            assert!(view.follow.offset <= 0.75 * f32::from(view.viewport.size.height));
            assert!(view.list.is_following_tail());
            view.follow.offset
        })
        .unwrap();
    window
        .update(cx, |view, _, cx| {
            view.streaming(false, cx);
            view.follow.last_frame = Some(Instant::now() - std::time::Duration::from_millis(16));
        })
        .unwrap();
    render(window, cx);
    window
        .update(cx, |view, _, _| {
            assert!(
                view.follow.offset >= 0. && view.follow.offset < before,
                "completion must continue the glide rather than snap to the end"
            );
        })
        .unwrap();
}

#[gpui::test]
fn ordinary_height_collapse_eases_back_and_a_new_prompt_resets_follow(cx: &mut TestAppContext) {
    let window = chat_mode(cx, false);
    window
        .update(cx, |view, _, cx| {
            view.anchor_prompts = false;
            view.smooth_reveal = false;
            view.streaming(true, cx);
            view.visible = false;
            view.patch(
                0,
                vec![
                    row("user", "Prompt"),
                    row("assistant", &"A paragraph.\n\n".repeat(35)),
                ],
                cx,
            );
            view.visible = true;
        })
        .unwrap();
    render(window, cx);
    window
        .update(cx, |view, _, cx| {
            view.visible = false;
            view.patch(1, vec![row("assistant", "A shorter result.")], cx);
            view.visible = true;
        })
        .unwrap();
    render(window, cx);
    let before = window
        .update(cx, |view, _, _| {
            assert!(
                view.follow.offset < 0.,
                "shrinking content should preserve its painted position"
            );
            view.follow.offset
        })
        .unwrap();
    window
        .update(cx, |view, _, _| {
            view.follow.last_frame = Some(Instant::now() - std::time::Duration::from_millis(16));
        })
        .unwrap();
    render(window, cx);
    window
        .update(cx, |view, _, cx| {
            assert!(view.follow.offset > before && view.follow.offset <= 0.);
            view.patch(2, vec![row("user", "Next prompt")], cx);
            assert_eq!(view.follow.offset, 0.);
            assert!(
                view.follow.measured.is_none(),
                "a sent prompt starts a fresh baseline"
            );
        })
        .unwrap();
}

#[gpui::test]
fn input_height_changes_preserve_the_chats_painted_position_and_follow_momentum(
    cx: &mut TestAppContext,
) {
    let window = input_chat(cx, "First line\nSecond line\nThird line");
    cx.refresh().unwrap();
    cx.run_until_parked();
    let before = window
        .update(cx, |_, window, _| bubble_top(window))
        .unwrap();
    window
        .update(cx, |chat, _, cx| {
            let text = chat.input.update(cx, |input, cx| input.take(cx)).unwrap();
            chat.transcript.update(cx, |view, cx| {
                view.follow.frame_elapsed = Some(0.);
                view.streaming(true, cx);
                view.patch(2, vec![row("user", &text), row("assistant", "")], cx);
            });
            cx.notify();
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    let entering = window
        .update(cx, |chat, window, cx| {
            assert!(
                chat.transcript.read(cx).follow.offset > 40.,
                "clearing a multiline input must not cancel the chat's glide"
            );
            bubble_top(window)
        })
        .unwrap();
    assert!(
        (before - entering).abs() < 3.,
        "the input collapse must preserve the previous bubble's painted position on the first frame: {before} -> {entering}"
    );
    let mut previous = entering;
    for _ in 0..12 {
        let offset = window
            .update(cx, |chat, _, cx| {
                chat.transcript.update(cx, |view, cx| {
                    view.follow.frame_elapsed = Some(16.);
                    cx.notify();
                    view.follow.offset
                })
            })
            .unwrap();
        cx.refresh().unwrap();
        cx.run_until_parked();
        window
            .update(cx, |chat, window, cx| {
                let top = bubble_top(window);
                let follow = &chat.transcript.read(cx).follow;
                assert!(
                    follow.offset > offset * 0.5,
                    "the input resize must not erase the motion on the next frame"
                );
                assert!(top <= previous + 1., "the chat must keep moving upward");
                previous = top;
            })
            .unwrap();
    }
    // Growing the input preserves an in-flight glide even after the reply's grace period.
    let before_grow = previous;
    window
        .update(cx, |chat, _, cx| {
            chat.input.update(cx, |input, cx| {
                input.set_text("One\nTwo\nThree\nFour".into(), cx)
            });
            chat.transcript.update(cx, |view, cx| {
                view.streaming(false, cx);
                view.follow.until = None;
                view.follow.last_frame = None;
                view.follow.frame_elapsed = Some(0.);
                cx.notify();
            });
            cx.notify();
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    let after_grow = window
        .update(cx, |_, window, _| bubble_top(window))
        .unwrap();
    assert!(
        (before_grow - after_grow).abs() < 3.,
        "input growth must join the existing motion: {before_grow} -> {after_grow}"
    );
    // A real window resize remains a fresh layout baseline.
    cx.simulate_window_resize(window.into(), size(px(500.), px(750.)));
    cx.refresh().unwrap();
    cx.run_until_parked();
    window
        .update(cx, |chat, _, cx| {
            assert_eq!(chat.transcript.read(cx).follow.offset, 0.);
        })
        .unwrap();
}

#[gpui::test]
fn multiline_sends_keep_sliding_when_the_composer_shrinks_and_delivery_is_immediate(
    cx: &mut TestAppContext,
) {
    let texts = [
        "First line\nSecond line".to_owned(),
        "This message wraps across multiple lines in the bubble. ".repeat(5),
        (0..14)
            .map(|ix| format!("Line {ix}: 🙂 café 漢字"))
            .collect::<Vec<_>>()
            .join("\n"),
    ];
    for text in texts {
        for remove_first in [false, true] {
            let window = input_chat(cx, &text);
            cx.refresh().unwrap();
            cx.run_until_parked();
            let before = window
                .update(cx, |chat, _, cx| {
                    chat.transcript.read(cx).viewport.size.height
                })
                .unwrap();
            window.update(cx, |chat, _, cx| {
                let sent = chat.input.update(cx, |input, cx| input.take(cx)).unwrap();
                chat.transcript.update(cx, |view, cx| {
                    view.set_pending(vec![PendingMessage {id:"queued".into(),text:sent.clone().into(),error:None,sent_at:None,attachments:Vec::new()}], cx);
                    let born = Instant::now() - std::time::Duration::from_millis(10);
                    view.pending_entrances.insert("queued".into(), born);
                    if remove_first {
                        view.set_pending(Vec::new(), cx);
                    }
                    view.streaming(true, cx);
                    let mut delivered = monocode_core::session::transcript::Transcript::default();
                    delivered.begin("user");
                    delivered.append(&sent);
                    delivered.begin("assistant");
                    for row in &mut delivered.rows {row.sent_at=None;}
                    view.patch(2, delivered.rows, cx);
                    view.set_pending(Vec::new(), cx);
                    assert_eq!(view.entrances.get(&2), Some(&born),
                        "delivery must continue the queued entrance, including split multiline messages");
                });
                cx.notify();
            }).unwrap();
            // The input's new height reaches the transcript through layout.
            for _ in 0..2 {
                cx.refresh().unwrap();
                cx.run_until_parked();
                window.update(cx, |_, _, cx| cx.notify()).unwrap();
            }
            window
                .update(cx, |chat, _, cx| {
                    chat.transcript.update(cx, |view, cx| {
                        assert!(
                            view.viewport.size.height >= before + px(18.),
                            "submitting a multiline draft shrinks the composer"
                        );
                        assert!(
                            view.follow.offset > 0.,
                            "the composer collapse must preserve scroll easing"
                        );
                        // Sample the beginning of the retained entrance, independently of scroll easing.
                        view.follow.reset();
                        view.entrances
                            .insert(2, Instant::now() - std::time::Duration::from_millis(10));
                        cx.notify();
                    })
                })
                .unwrap();
            cx.refresh().unwrap();
            cx.run_until_parked();
            let top = |window: &mut Window| {
                window
                    .painted_quads()
                    .into_iter()
                    .find(|quad| {
                        (quad.corner_radii.top_left.0 / window.scale_factor() - 12.).abs() < 0.1
                            && quad.bounds.size.height.0 / window.scale_factor() > 40.
                    })
                    .map(|quad| quad.bounds.top().0 / window.scale_factor())
                    .expect("the multiline bubble is painted")
            };
            let entering = window
                .update(cx, |chat, window, cx| {
                    let entering = top(window);
                    chat.transcript.update(cx, |view, cx| {
                        view.entrances.remove(&2);
                        cx.notify();
                    });
                    entering
                })
                .unwrap();
            cx.refresh().unwrap();
            cx.run_until_parked();
            let settled = window.update(cx, |_, window, _| top(window)).unwrap();
            assert!(
                entering > settled + 5.,
                "the multiline bubble rises independently of scroll easing: {entering} -> {settled}"
            );
        }
    }
}

#[gpui::test]
fn delivery_does_not_restart_settled_bubbles_or_override_reduced_motion(cx: &mut TestAppContext) {
    for reduced in [false, true] {
        cx.update(|cx| cx.set_reduce_motion(reduced));
        for remove_first in [false, true] {
            let view = cx.new(|_| {
                let mut view = TranscriptView::new();
                view.mono_mode("cat", 0xe86b5f, "MonoCat");
                view
            });
            view.update(cx, |view, cx| {
                // UTF-8 byte chunking, independently of explicit newline chunking.
                let text = "🙂 café 漢字 ".repeat(200);
                view.set_pending(
                    vec![PendingMessage {
                        id: "q".into(),
                        text: text.clone().into(),
                        error: None,
                        sent_at: None,
                        attachments: Vec::new(),
                    }],
                    cx,
                );
                if !reduced {
                    view.pending_entrances.insert(
                        "q".into(),
                        Instant::now() - std::time::Duration::from_secs(1),
                    );
                }
                if remove_first {
                    view.set_pending(Vec::new(), cx);
                }
                view.streaming(true, cx);
                let mut delivered = monocode_core::session::transcript::Transcript::default();
                delivered.begin("user");
                delivered.append(&text);
                assert!(delivered.rows.len() > 1);
                view.patch(0, delivered.rows, cx);
                assert!(
                    !view.entrances.contains_key(&0),
                    "settled and reduced-motion messages stay settled on delivery"
                );
            });
        }
    }
}

#[gpui::test]
fn sending_the_next_message_keeps_the_previous_reply_settled(cx: &mut TestAppContext) {
    let window = chat(cx);
    window
        .update(cx, |view, _, cx| {
            view.patch(
                0,
                vec![
                    row("user", "First message"),
                    row("assistant", "The previous answer"),
                ],
                cx,
            );
        })
        .unwrap();
    render(window, cx);
    window
        .update(cx, |view, _, cx| {
            let answer: Vec<_> = view
                .rows
                .iter()
                .filter(|r| matches!(r.body, Body::Markdown(_)))
                .map(|r| r.key)
                .collect();
            assert!(!answer.is_empty());
            view.streaming(true, cx);
            assert!(
                answer.iter().all(|key| !view.motions.contains_key(key)),
                "starting a request must not animate the previous answer out"
            );
            view.patch(
                2,
                vec![row("user", "Next message"), row("assistant", "")],
                cx,
            );
            assert!(
                answer.iter().all(|key| !view.motions.contains_key(key)),
                "delivering the new prompt must not replay the previous answer's entrance"
            );
            assert!(!view.entrances.contains_key(&1));
        })
        .unwrap();
}
