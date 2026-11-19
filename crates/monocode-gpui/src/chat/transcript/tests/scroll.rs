mod anchors;
mod mono;
mod phases;
use super::super::*;
use gpui::{TestAppContext, WindowBounds, WindowOptions, point};

fn row(role: &str, text: &str) -> Row {
    Row {
        role: role.into(),
        text: text.into(),
        first: true,
        sent_at: None,
    }
}
fn settle(view: &mut TranscriptView, cx: &mut Context<TranscriptView>) {
    for reveal in view.reveals.values_mut() {
        reveal.settle(Instant::now());
    }
    view.prompt_rise = None;
    view.motions.clear();
    view.rebuild(0, cx);
}

#[gpui::test]
fn expanded_user_chunks_paint_one_aligned_bubble(cx: &mut TestAppContext) {
    // Compare settled bubble geometry rather than per-row resize glides.
    cx.update(|cx| cx.set_reduce_motion(true));
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(900.), px(1000.)),
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| TranscriptView::new()),
        )
        .unwrap()
    });
    // Newline splitting produces three rows, with the widest text in the
    // middle. Check both natural-width and capped, wrapped messages.
    for (wide, width) in [
        ("a medium length line".to_owned(), 900.),
        ("wide text ".repeat(20), 900.),
        ("wide text ".repeat(20), 400.),
    ] {
        cx.simulate_window_resize(window.into(), size(px(width), px(1000.)));
        let mut transcript = monocode_core::session::transcript::Transcript::default();
        transcript.begin("user");
        transcript.append(&format!(
            "{}\n{wide}\n{}",
            "x\n".repeat(12),
            "y\n".repeat(12)
        ));
        assert!(transcript.rows.len() >= 3);
        window
            .update(cx, |view, _, cx| {
                view.patch(0, transcript.rows, cx);
                if !view.expanded_prompts.contains(&0) {
                    view.toggle_prompt(0, cx);
                }
                settle(view, cx);
            })
            .unwrap();
        // The viewport probe invalidates retained list heights after a
        // resize; draw again to inspect the resulting geometry.
        for _ in 0..2 {
            window.update(cx, |_, _, cx| cx.notify()).unwrap();
            cx.refresh().unwrap();
            cx.run_until_parked();
        }
        window
            .update(cx, |view, window, _| {
                let mut bubbles: Vec<_> = window
                    .painted_quads()
                    .into_iter()
                    .filter(|quad| quad.background == user_bubble_fill().into())
                    .collect();
                bubbles.sort_by(|a, b| a.bounds.top().0.total_cmp(&b.bounds.top().0));
                assert_eq!(bubbles.len(), view.rows.len());
                assert!(bubbles.len() >= 3);
                let first = &bubbles[0];
                let last = bubbles.last().unwrap();
                assert!(first.corner_radii.top_left.0 > 0.);
                assert!(last.corner_radii.bottom_left.0 > 0.);
                for pair in bubbles.windows(2) {
                    let (a, b) = (&pair[0], &pair[1]);
                    assert_eq!(a.bounds.left(), b.bounds.left(), "shared left edge");
                    assert_eq!(a.bounds.size.width, b.bounds.size.width, "shared width");
                    assert_eq!(a.bounds.bottom(), b.bounds.top(), "no gaps between chunks");
                    assert_eq!(a.corner_radii.bottom_left.0, 0.);
                    assert_eq!(b.corner_radii.top_left.0, 0.);
                }
                assert!(first.bounds.size.width.0 / window.scale_factor() <= 576.);
            })
            .unwrap();
    }
    window
        .update(cx, |view, _, cx| {
            // A source patch updates rows in place; it does not truncate
            // the previous prompt's continuation rows.
            *view = TranscriptView::new();
            view.patch(0, vec![row("user", "Hi")], cx);
            settle(view, cx);
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    window
        .update(cx, |_, window, _| {
            let bubble = window
                .painted_quads()
                .into_iter()
                .find(|quad| quad.background == user_bubble_fill().into())
                .unwrap();
            assert!(
                bubble.bounds.size.width.0 / window.scale_factor() < 96.,
                "short messages still shrink to their text"
            );
            assert_eq!(
                bubble.corner_radii.top_left.0 * 2.,
                bubble.bounds.size.height.0
            );
            assert_eq!(
                bubble.corner_radii.top_left,
                bubble.corner_radii.bottom_left
            );
        })
        .unwrap();
}

#[gpui::test]
fn settled_tab_switches_preserve_rows_layout_and_scroll(cx: &mut TestAppContext) {
    let view = cx.new(|_| TranscriptView::new());
    view.update(cx, |view, cx| {
        view.patch(
            0,
            vec![
                row("user", "Question"),
                row("assistant", "A settled **answer**."),
            ],
            cx,
        );
        settle(view, cx);
        view.list.pause_following_tail();
        view.list.scroll_to(ListOffset {
            item_ix: 1,
            offset_in_item: px(7.),
        });
        let position = view.list.logical_scroll_top();
        let rows = view.rows.as_ptr();
        let rebuilds = view.rebuild_passes;
        view.phase_layouts.insert(
            999,
            PhaseLayout {
                width: px(600.),
                height: 42.,
                rows: HashMap::new(),
            },
        );
        for _ in 0..20 {
            view.visible(false, cx);
            view.animate_frame(cx);
            view.visible(true, cx);
        }
        assert_eq!(view.rebuild_passes, rebuilds);
        assert_eq!(view.rows.as_ptr(), rows);
        assert!(view.phase_layouts.contains_key(&999));
        let restored = view.list.logical_scroll_top();
        assert_eq!(restored.item_ix, position.item_ix);
        assert_eq!(restored.offset_in_item, position.offset_in_item);
        assert!(!view.list.is_following_tail());
    });
}

#[gpui::test]
fn exit_rows_survive_a_patch_that_merges_messages(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(false));
    let window = cx.update(|cx| {
        cx.open_window(WindowOptions::default(), |_, cx| {
            cx.new(|_| TranscriptView::new())
        })
        .unwrap()
    });
    window
        .update(cx, |view, _, cx| {
            view.patch(
                0,
                vec![row("user", "Question"), row("assistant", "Answer")],
                cx,
            );
            settle(view, cx);
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, _, cx| {
            // The second source row now continues the first message, so
            // the answer's row exits while only one message remains.
            assert!(view.rows.iter().any(|r| r.key.0 == 1));
            let mut continuation = row("assistant", " continued");
            continuation.first = false;
            view.patch(0, vec![row("assistant", "Answer"), continuation], cx);
            assert_eq!(view.messages.len(), 1);
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| {
            assert!(view.rows.iter().all(|r| r.message < view.messages.len()));
        })
        .unwrap();
}

#[gpui::test]
fn hiding_settles_live_text_and_showing_reuses_hidden_updates(cx: &mut TestAppContext) {
    let view = cx.new(|_| TranscriptView::new());
    view.update(cx, |view, cx| {
        view.streaming = true;
        view.live_start = 1;
        view.patch(
            0,
            vec![
                row("user", "Question"),
                row("assistant", "The complete live answer has arrived."),
            ],
            cx,
        );
        // Reproduce a response still catching up to its received text.
        view.reveals.insert(
            1,
            PacedText::new(
                "The complete live answer has arrived.",
                true,
                Instant::now(),
            ),
        );
        assert!(view.reveals[&1].behind());
        let before = view.rebuild_passes;
        view.visible(false, cx);
        assert_eq!(view.rebuild_passes, before + 1);
        assert!(!view.reveals[&1].behind());
        view.patch(1, vec![row("assistant", "Updated while hidden.")], cx);
        let updated = view.rebuild_passes;
        view.visible(true, cx);
        assert_eq!(view.rebuild_passes, updated);
        assert!(
            view.rows
                .iter()
                .any(|row| matches!(&row.body, Body::Markdown(md)
            if crate::chat::markdown::selection_text(md).contains("Updated while hidden.")))
        );
    });
}

#[test]
fn saved_work_line_uses_turn_provenance_and_never_invents_legacy_model_or_duration() {
    assert_eq!(
        work_title(Some("Saved Model"), Some(9500), false),
        "Saved Model worked for 10s"
    );
    assert_eq!(work_title(None, Some(60000), false), "Worked for 1m");
    assert_eq!(work_title(None, None, false), "Worked");
    assert_eq!(work_title(Some(""), None, false), "Worked");
    assert_eq!(
        work_title(Some("Saved Model"), Some(0), false),
        "Saved Model worked for 1s"
    );
    assert_eq!(
        work_title(Some("Live Model"), None, true),
        "Live Model working…"
    );
}

#[gpui::test]
fn fresh_follow_ups_keep_provider_metrics_and_timing_on_the_original_turn(cx: &mut TestAppContext) {
    let view = cx.new(|_| TranscriptView::new());
    view.update(cx, |view, cx| {
        view.identity("codex", "Chosen Model");
        view.streaming(true, cx);
        view.patch(
            0,
            vec![row("user", "Prompt"), row("assistant", "Working")],
            cx,
        );
        view.record_provider_turn("provider-turn".into());
        view.patch(
            2,
            vec![row("user", "Also this"), row("assistant", "Done")],
            cx,
        );
        view.metrics(monocode_core::session::agent::TurnMetrics {
            output_tokens: 20,
            ..Default::default()
        });
        view.streaming(false, cx);
        assert_eq!(view.turns[&1].model, "Chosen Model");
        assert_eq!(
            view.turns[&1].provider_turn_id.as_deref(),
            Some("provider-turn")
        );
        assert_eq!(view.turns[&1].metrics.unwrap().output_tokens, 20);
        assert!(view.turns[&1].has_duration());
        assert!(view.turns[&1].completed_at.is_some());
        assert_eq!(view.turns[&3].model, "Chosen Model");
        assert!(
            !view.turns[&3].has_duration(),
            "a steer has provenance but no separate run duration"
        );
        assert!(view.turns[&3].metrics.is_none());
    });
}
