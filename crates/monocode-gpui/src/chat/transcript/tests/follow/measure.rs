use super::*;

#[gpui::test]
fn queued_messages_contribute_their_height_once(cx: &mut TestAppContext) {
    let window = chat(cx);
    window
        .update(cx, |view, _, cx| {
            view.streaming(true, cx);
            view.patch(
                0,
                vec![row("user", "Start writing"), row("assistant", "")],
                cx,
            );
        })
        .unwrap();
    render(window, cx);
    window
        .update(cx, |view, _, cx| {
            view.follow.reset();
            view.set_pending(
                vec![PendingMessage {
                    id: "q".into(),
                    text: "Write the document".into(),
                    error: None,
                    sent_at: None,
                    attachments: Vec::new(),
                }],
                cx,
            );
        })
        .unwrap();
    render(window, cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    let height = f32::from(
        visual
            .debug_bounds("mono-pending-message")
            .unwrap()
            .size
            .height,
    );
    window
        .update(visual, |view, _, _| {
            assert!(
                view.follow.offset <= height + 0.25,
                "one queued bubble is {height}px high but introduced {}px of movement",
                view.follow.offset
            );
        })
        .unwrap();
}

#[gpui::test]
fn changing_hidden_work_status_does_not_move_settled_messages(cx: &mut TestAppContext) {
    let window = chat(cx);
    window
        .update(cx, |view, _, cx| {
            view.patch(0, vec![row("user", "Write a document")], cx);
            view.streaming(true, cx);
            view.patch(1, vec![row("assistant", "")], cx);
        })
        .unwrap();
    render(window, cx);
    window.update(cx, |view, _, _| view.follow.reset()).unwrap();
    for text in [
        r#"{"id":"write","name":"Write","input":"{\"file_path\":\"notes.md\"}","output":"","state":"running"}"#,
        r#"{"id":"write","name":"Write","input":"{\"file_path\":\"notes.md\"}","output":"Saved the document","state":"completed"}"#,
    ] {
        window
            .update(cx, |view, _, cx| view.patch(2, vec![row("tool", text)], cx))
            .unwrap();
        render(window, cx);
        window
            .update(cx, |view, _, _| {
                assert!(
                    view.follow.offset.abs() < 0.25,
                    "a ticker-only change moved the chat {}px",
                    view.follow.offset
                );
            })
            .unwrap();
    }
}

#[test]
fn measurements_follow_surviving_rows_across_footer_changes_and_window_boundaries() {
    let measure = |rows: &[(usize, f32)]| FollowMeasure {
        viewport: size(px(900.), px(600.)),
        bottom: px(600.),
        window_size: size(px(900.), px(600.)),
        footer: 0.,
        slack: 0.,
        rows: rows
            .iter()
            .map(|(key, height)| ((*key, 0), *height))
            .collect(),
    };
    let old = measure(&[(0, 60.), (1, 28.), (2, 28.)]);
    assert_eq!(
        measure(&[(0, 60.), (1, 28.), (4, 28.)]).shift_from(&old),
        0.
    );
    assert_eq!(
        measure(&[(0, 60.), (5, 60.), (1, 28.), (2, 28.)]).shift_from(&old),
        60.
    );
    assert_eq!(
        measure(&[(1, 28.), (2, 28.), (3, 40.)]).shift_from(&old),
        40.
    );
    let mut resized = measure(&[(0, 80.), (1, 28.), (2, 28.)]);
    resized.viewport.width = px(800.);
    assert_eq!(resized.shift_from(&old), 0.);
    let mut taller = measure(&[(0, 80.), (1, 28.), (2, 28.)]);
    taller.viewport.height += px(36.);
    taller.bottom += px(36.);
    assert_eq!(taller.shift_from(&old), -16.);
    taller.window_size.height += px(36.);
    assert_eq!(taller.shift_from(&old), 0.);
    let mut shorter = measure(&[(0, 60.), (1, 28.), (2, 28.)]);
    shorter.viewport.height -= px(18.);
    shorter.bottom -= px(18.);
    assert_eq!(shorter.shift_from(&old), 18.);
    assert_eq!(
        measure(&[(6, 20.)]).shift_from(&old),
        0.,
        "disconnected history starts a fresh baseline"
    );
}

#[test]
fn the_follow_spring_never_overshoots_and_keeps_momentum_across_brief_pauses() {
    let mut follow = Follow::default();
    let mut previous = 940.;
    let mut offset = follow.step(previous, previous, 1., 1., CHAT_FOLLOW_MAX_SPEED);
    for _ in 0..240 {
        assert!(offset >= 0. && offset <= previous);
        assert!(previous - offset <= CHAT_FOLLOW_MAX_SPEED + 0.001);
        previous = offset;
        offset = follow.step(offset, 0., 1., 1., CHAT_FOLLOW_MAX_SPEED);
    }
    assert!(offset < 0.25);
    let mut follow = Follow {
        velocity: 4.,
        growth: 2.,
        offset: 0.1,
        ..Default::default()
    };
    follow.settle(Instant::now());
    assert_eq!(follow.offset, 0.);
    assert_eq!((follow.velocity, follow.growth), (4., 2.));
}

#[gpui::test]
fn a_document_and_final_reply_preserve_position_then_glide_up_without_reversing(
    cx: &mut TestAppContext,
) {
    let window = chat(cx);
    window
        .update(cx, |view, _, cx| {
            view.patch(0, vec![row("user", "Write a document")], cx);
            view.streaming(true, cx);
            view.patch(1, vec![row("assistant", "")], cx);
        })
        .unwrap();
    render(window, cx);
    let before = window
        .update(cx, |view, window, _| {
            view.follow.reset();
            view.entrances.clear();
            bubble_top(window)
        })
        .unwrap();
    window
        .update(cx, |view, _, cx| {
            view.patch(
                2,
                vec![row(
                    "mono-card",
                    r#"{"id":"doc","type":"document","documentId":"d","title":"The document"}"#,
                )],
                cx,
            );
        })
        .unwrap();
    render(window, cx);
    let entering = window
        .update(cx, |view, window, _| {
            assert!(view.follow.offset > 0.);
            bubble_top(window)
        })
        .unwrap();
    assert!(
        (before - entering).abs() < 3.,
        "adding the card starts at the current painted position: {before} -> {entering}"
    );
    window
        .update(cx, |view, _, cx| {
            let reply = "Here is another paragraph about the finished document.\n\n".repeat(100);
            view.patch(3, vec![row("assistant", &reply)], cx);
            view.streaming(false, cx);
            view.entrances.clear();
            view.follow.last_frame = None;
        })
        .unwrap();
    render(window, cx);
    let mut previous = window
        .update(cx, |view, _, _| {
            assert!(
                view.follow.offset > 2000.,
                "keep the full reveal distance even beyond the 80-row measurement window"
            );
            view.follow.offset
        })
        .unwrap();
    for _ in 0..20 {
        window
            .update(cx, |view, _, _| {
                view.follow.last_frame =
                    Some(Instant::now() - std::time::Duration::from_millis(16));
            })
            .unwrap();
        render(window, cx);
        let next = window.update(cx, |view, _, _| view.follow.offset).unwrap();
        assert!(
            next >= 0. && next <= previous,
            "reveal reversed direction: {previous} -> {next}"
        );
        previous = next;
    }
}

#[gpui::test]
fn a_large_reveal_keeps_settling_after_the_reply_grace_period(cx: &mut TestAppContext) {
    let window = chat(cx);
    window
        .update(cx, |view, _, cx| {
            view.patch(
                0,
                vec![row("user", "Earlier"), row("assistant", "Done")],
                cx,
            );
        })
        .unwrap();
    render(window, cx);
    window
        .update(cx, |view, _, cx| {
            view.follow.offset = 940.;
            view.follow.until = Some(Instant::now() - std::time::Duration::from_secs(2));
            view.follow.last_frame = Some(Instant::now() - std::time::Duration::from_millis(16));
            cx.notify();
        })
        .unwrap();
    render(window, cx);
    window
        .update(cx, |view, _, _| {
            assert!(
                view.follow.offset > 600. && view.follow.offset < 940.,
                "an existing glide should keep easing, not snap to the end: {}",
                view.follow.offset
            );
        })
        .unwrap();
}

#[gpui::test]
fn scrolling_away_hands_over_the_visible_position_once(cx: &mut TestAppContext) {
    let window = chat(cx);
    window
        .update(cx, |view, _, cx| {
            view.patch(
                0,
                (0..40)
                    .map(|ix| {
                        row(
                            if ix % 2 == 0 { "user" } else { "assistant" },
                            "A message in the history",
                        )
                    })
                    .collect(),
                cx,
            );
            view.list = view.list.clone().measure_all();
        })
        .unwrap();
    render(window, cx);
    window
        .update(cx, |view, window, cx| {
            view.list.pause_following_tail();
            view.list.scroll_to(ListOffset {
                item_ix: 35,
                offset_in_item: px(10.),
            });
            let before = view.list.scroll_px_offset_for_scrollbar().y;
            view.follow.offset = 40.;
            view.update_follow(window, cx);
            let handed_over = view.list.scroll_px_offset_for_scrollbar().y;
            assert_eq!(handed_over, before + px(40.));
            assert_eq!(view.follow.offset, 0.);
            assert!(!view.list.is_following_tail());
            view.update_follow(window, cx);
            assert_eq!(view.list.scroll_px_offset_for_scrollbar().y, handed_over);
        })
        .unwrap();
}
