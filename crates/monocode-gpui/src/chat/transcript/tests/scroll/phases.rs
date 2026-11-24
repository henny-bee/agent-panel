use super::*;

#[test]
fn phase_reveals_one_clipped_body_and_keeps_visible_steps_at_their_natural_height() {
    let layout = PhaseLayout {
        width: px(600.),
        height: 100.,
        rows: HashMap::from([
            ((1, 2), (0., 20.)),
            ((2, 2), (20., 50.)),
            ((3, 2), (70., 30.)),
        ]),
    };
    assert_eq!(layout.row_progress((1, 2), 0.5), 1.);
    assert_eq!(layout.row_progress((2, 2), 0.5), 0.6);
    assert_eq!(layout.row_progress((3, 2), 0.5), 0.);
    for progress in [0., 0.2, 0.5, 0.9, 1.] {
        let visible: f32 = layout
            .rows
            .iter()
            .map(|(key, (_, height))| height * layout.row_progress(*key, progress))
            .sum();
        assert!((visible - layout.height * progress).abs() < 0.001);
    }
}

#[gpui::test]
fn clicking_a_phase_uses_one_reveal_and_rapid_toggles_preserve_its_progress(
    cx: &mut TestAppContext,
) {
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|_| TranscriptView::new())
        })
        .unwrap()
    });
    window
        .update(cx, |view, _, cx| {
            let tool = |id: &str| {
                row(
                    "tool",
                    &serde_json::to_string(&ToolCall {
                        id: id.into(),
                        name: "Read".into(),
                        input: format!(r#"{{"file_path":"{id}.rs"}}"#),
                        output: "tool output".into(),
                        state: ToolState::Completed,
                        panel: None,
                        display: None,
                        images: Vec::new(),
                    })
                    .unwrap(),
                )
            };
            view.patch(
                0,
                vec![
                    row("user", "Check"),
                    tool("one"),
                    tool("two"),
                    row("assistant", "Done"),
                ],
                cx,
            );
            view.work_folds.insert(1, true);
            view.rebuild(1, cx);
            view.motions.clear();
            view.toggle_phase(1, true, cx);
            let motions: Vec<_> = view
                .rows
                .iter()
                .filter(|r| r.in_phase())
                .map(|r| view.motions[&r.key].clone())
                .collect();
            assert_eq!(motions.len(), 2, "the two compact steps share the body");
            for motion in &motions {
                assert_eq!(motion.group, Some(1));
                assert_eq!(motion.start, motions[0].start);
                assert!(!motion.step);
            }
            view.toggle_phase(1, false, cx);
            assert!(
                view.rows.iter().any(|r| r.in_phase()),
                "closing retains the body through its exit"
            );
            assert!(
                view.motions
                    .values()
                    .all(|m| m.group == Some(1) && !m.opening)
            );
            view.toggle_phase(1, true, cx);
            assert!(
                view.motions
                    .values()
                    .all(|m| m.group == Some(1) && m.opening)
            );
            assert_eq!(view.phase_toggle, None);
            // Hold a partial reveal long enough to inspect actual list geometry.
            let start = Instant::now() - std::time::Duration::from_secs(25);
            for motion in view.motions.values_mut() {
                *motion = Motion::phase(1, true, start);
                motion.duration = std::time::Duration::from_secs(100);
            }
            view.visible = false;
            cx.notify();
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| {
            let layout = &view.phase_layouts[&1];
            let steps: Vec<_> = view
                .rows
                .iter()
                .enumerate()
                .filter(|(_, r)| r.in_phase())
                .collect();
            let (first_ix, first) = steps.first().unwrap();
            let (last_ix, last) = steps.last().unwrap();
            let first_bounds = view.list.bounds_for_item(*first_ix).unwrap();
            let last_bounds = view.list.bounds_for_item(*last_ix).unwrap();
            assert!(
                (f32::from(first_bounds.size.height) - layout.rows[&first.key].1).abs() < 0.1,
                "a revealed row must retain its natural height, rather than shrink with the group"
            );
            assert!(
                f32::from(last_bounds.size.height) < layout.rows[&last.key].1,
                "the reveal clips the bottom of the body"
            );
        })
        .unwrap();
}

#[gpui::test]
fn closing_a_phase_keeps_its_header_and_final_frame_in_place(cx: &mut TestAppContext) {
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|_| TranscriptView::new())
        })
        .unwrap()
    });
    for width in [900., 320.] {
        cx.simulate_window_resize(window.into(), size(px(width), px(1400.)));
        window
            .update(cx, |view, _, cx| {
                view.visible = false;
                let mut source = vec![
                    row("user", "Check the collapse"),
                    row("assistant", "I will inspect the tools."),
                ];
                source.extend((0..5).map(|ix| {
                    row(
                        "tool",
                        &serde_json::to_string(&ToolCall {
                            id: ix.to_string(),
                            name: "Read".into(),
                            input: format!(r#"{{"file_path":"file-{ix}.rs"}}"#),
                            output: "A tool result that can wrap at a narrower width. ".repeat(3),
                            state: ToolState::Completed,
                            panel: None,
                            display: None,
                            images: Vec::new(),
                        })
                        .unwrap(),
                    )
                }));
                source.push(row("assistant", "The tools have finished."));
                source.push(row("assistant", "Here is the final answer."));
                view.patch(0, source, cx);
                view.work_folds.insert(1, true);
                view.phases.insert(2, true);
                view.expanded.insert(3);
                view.rebuild(0, cx);
                view.list.pause_following_tail();
                view.list.scroll_to(ListOffset {
                    item_ix: 0,
                    offset_in_item: px(0.),
                });
            })
            .unwrap();
        cx.refresh().unwrap();
        let header = window
            .update(cx, |view, _, cx| {
                assert_eq!(view.viewport.size.width, px(width));
                let ix = view
                    .rows
                    .iter()
                    .position(|r| matches!(r.body, Body::Phase { group: 2, .. }))
                    .unwrap();
                let header = view.list.bounds_for_item(ix).unwrap();
                view.visible = true;
                view.toggle_phase(2, false, cx);
                view.visible = false;
                header
            })
            .unwrap();
        cx.refresh().unwrap();
        let mut previous = window
            .update(cx, |view, _, _| {
                let ix = view
                    .rows
                    .iter()
                    .position(|r| matches!(r.body, Body::Phase { group: 2, .. }))
                    .unwrap();
                assert_eq!(
                    view.list.bounds_for_item(ix).unwrap(),
                    header,
                    "closing the body must not move or resize its header"
                );
                (0..view.rows.len())
                    .map(|ix| view.list.bounds_for_item(ix).unwrap())
                    .collect::<Vec<_>>()
            })
            .unwrap();
        for elapsed_ms in [40, 160, 220, 280, 310, 319, 320] {
            window
                .update(cx, |view, _, cx| {
                    let start = Instant::now() - std::time::Duration::from_millis(elapsed_ms);
                    for motion in view.motions.values_mut() {
                        motion.start = start;
                    }
                    view.list.remeasure_items(0..view.rows.len() + 1);
                    cx.notify();
                })
                .unwrap();
            cx.refresh().unwrap();
            previous = window.update(cx, |view, _, _| {
                let current: Vec<_> = (0..view.rows.len())
                    .map(|ix| view.list.bounds_for_item(ix).unwrap()).collect();
                for (before, after) in previous.iter().zip(&current) {
                    assert!(after.top() <= before.top() + px(0.1),
                        "closing must not shift rows down at {elapsed_ms}ms: {before:?} vs {after:?}");
                }
                current
            }).unwrap();
        }
        let finishing = window
            .update(cx, |view, _, cx| {
                let finishing: Vec<_> = view
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| !r.in_phase())
                    .map(|(ix, r)| (r.key, view.list.bounds_for_item(ix).unwrap()))
                    .collect();
                // Hidden transcripts intentionally ignore queued frames.
                // Drive this final visible frame explicitly for the test.
                view.visible = true;
                view.animate_frame(cx);
                view.visible = false;
                assert!(!view.rows.iter().any(|r| r.in_phase()));
                finishing
            })
            .unwrap();
        cx.refresh().unwrap();
        window
            .update(cx, |view, _, _| {
                for (key, before) in finishing {
                    let ix = view.rows.iter().position(|r| r.key == key).unwrap();
                    assert_eq!(
                        view.list.bounds_for_item(ix).unwrap(),
                        before,
                        "removing the closed rows must not nudge the transcript"
                    );
                }
            })
            .unwrap();
    }
}

#[gpui::test]
fn work_and_phase_disclosures_finish_at_their_natural_height_after_resizing(
    cx: &mut TestAppContext,
) {
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|_| TranscriptView::new())
        })
        .unwrap()
    });
    window
        .update(cx, |view, _, cx| {
            let tool = |id: &str| {
                row(
                    "tool",
                    &serde_json::to_string(&ToolCall {
                        id: id.into(),
                        name: "Read".into(),
                        input: format!(r#"{{"file_path":"{id}.rs"}}"#),
                        output: String::new(),
                        state: ToolState::Completed,
                        panel: None,
                        display: None,
                        images: Vec::new(),
                    })
                    .unwrap(),
                )
            };
            view.patch(
                0,
                vec![
                    row("user", "Check the layout"),
                    row(
                        "assistant",
                        &"Earlier commentary that wraps onto several lines. ".repeat(8),
                    ),
                    tool("first"),
                    tool("second"),
                    row(
                        "assistant",
                        "The final answer stays below the work.\n\nIts spacing stays fixed.",
                    ),
                ],
                cx,
            );
            view.list.pause_following_tail();
        })
        .unwrap();
    // Exercise first opening, nested tools and reopening at a new width,
    // without relying on any previously painted height.
    for width in [900., 320., 640.] {
        window
            .update(cx, |view, w, cx| {
                w.resize(size(px(width), px(1200.)));
                view.visible = true;
                view.work_folds.insert(1, true);
                view.phases.insert(2, true);
                view.rebuild(1, cx);
                view.motions.clear();
                for row in view.rows.iter().filter(|row| row.work) {
                    view.motions.insert(
                        row.key,
                        Motion::new(
                            true,
                            false,
                            Instant::now() - std::time::Duration::from_secs(1),
                        ),
                    );
                }
                // Retain the finishing frame until its geometry is inspected.
                view.visible = false;
                view.list.scroll_to(ListOffset {
                    item_ix: 0,
                    offset_in_item: px(0.),
                });
                cx.notify();
            })
            .unwrap();
        cx.refresh().unwrap();
        cx.run_until_parked();
        let finishing = window
            .update(cx, |view, _, cx| {
                let bounds: Vec<_> = (0..view.rows.len())
                    .map(|ix| view.list.bounds_for_item(ix).unwrap())
                    .collect();
                view.motions.clear();
                view.list.remeasure_items(0..view.rows.len() + 1);
                cx.notify();
                bounds
            })
            .unwrap();
        cx.refresh().unwrap();
        cx.run_until_parked();
        window.update(cx, |view, _, cx| {
            for (ix, before) in finishing.iter().enumerate() {
                let after = view.list.bounds_for_item(ix).unwrap();
                assert!((before.top() - after.top()).abs() < px(0.1));
                assert!((before.size.height - after.size.height).abs() < px(0.1),
                    "finishing a disclosure must not move the content below it: {before:?} vs {after:?}");
            }
            assert!(view.action_anchors.contains_key(&(1, "copy")));
            assert!(view.action_anchors.contains_key(&(1, "note")));
            assert!(!view.action_anchors.contains_key(&(1, "activity")));
            view.work_folds.insert(1, false);
            view.rebuild(1, cx);
        }).unwrap();
    }
}
