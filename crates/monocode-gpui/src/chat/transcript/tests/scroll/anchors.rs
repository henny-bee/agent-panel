use super::*;

#[gpui::test]
fn prompt_rise_moves_the_painted_bubble_without_moving_its_scroll_anchor(cx: &mut TestAppContext) {
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(900.), px(600.)),
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| TranscriptView::new()),
        )
        .unwrap()
    });
    window
        .update(cx, |view, _, cx| {
            view.streaming(true, cx);
            view.patch(
                0,
                vec![row("user", "A single line"), row("assistant", "")],
                cx,
            );
            view.streaming(false, cx);
            view.prompt_rise_pending = false;
            view.prompt_rise = Some((0, Instant::now() - std::time::Duration::from_millis(40)));
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    let (rising_y, scroll_anchor) = window
        .update(cx, |view, w, cx| {
            let bubble = w
                .painted_quads()
                .into_iter()
                .find(|q| {
                    q.corner_radii.top_left.0 > 0.
                        && q.corner_radii.top_left.0 * 2. == q.bounds.size.height.0
                })
                .expect("single-line bubble is fully rounded");
            assert_eq!(
                bubble.bounds.size.height.0 / w.scale_factor(),
                36.,
                "React's 20px line height plus 8px padding above and below"
            );
            let position = bubble.bounds.top().0 / w.scale_factor();
            let anchor = view.list.logical_scroll_top();
            view.prompt_rise = Some((0, Instant::now() - std::time::Duration::from_millis(900)));
            cx.notify();
            (position, anchor)
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, w, _| {
            let bubble = w
                .painted_quads()
                .into_iter()
                .find(|q| {
                    q.corner_radii.top_left.0 > 0.
                        && q.corner_radii.top_left.0 * 2. == q.bounds.size.height.0
                })
                .unwrap();
            let settled_y = bubble.bounds.top().0 / w.scale_factor();
            assert!(
                rising_y > settled_y + 30.,
                "the painted prompt must visibly rise, rather than only fade"
            );
            assert_eq!(
                view.list.logical_scroll_top().item_ix,
                scroll_anchor.item_ix
            );
            assert_eq!(
                view.list.logical_scroll_top().offset_in_item,
                scroll_anchor.offset_in_item
            );
        })
        .unwrap();
}

#[gpui::test]
fn steered_prompts_take_the_top_anchor_without_restarting_the_provider_turn(
    cx: &mut TestAppContext,
) {
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(900.), px(600.)),
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| TranscriptView::new()),
        )
        .unwrap()
    });
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
            view.streaming(true, cx);
            view.patch(
                2,
                vec![row("user", "Start working"), row("assistant", "Working")],
                cx,
            );
            settle(view, cx);
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();

    for start in [4, 7] {
        window
            .update(cx, |view, _, cx| {
                view.list.scroll_by(px(-100.));
                view.update_jump(cx);
                assert!(!view.list.is_following_tail());
                let mut continuation = row("user", " and its continuation");
                continuation.first = false;
                view.patch(
                    start,
                    vec![
                        row("user", "Change direction"),
                        continuation,
                        row("assistant", "Following the steer"),
                    ],
                    cx,
                );
                assert_eq!(
                    view.live_start, 2,
                    "steering keeps the provider turn running"
                );
                assert_eq!(view.anchor_turn, Some(start));
                assert_eq!(view.prompt_rise.map(|(at, _)| at), Some(start));
                assert!(view.list.is_following_tail());
                assert!(!view.show_jump);
                settle(view, cx);
            })
            .unwrap();
        cx.refresh().unwrap();
        cx.run_until_parked();
        window
            .update(cx, |view, window, cx| {
                let prompt = view.messages.iter().find(|m| m.start == start).unwrap();
                let bounds = view.list.bounds_for_item(prompt.display_start).unwrap();
                assert!(
                    (bounds.top() - view.viewport.top()).abs() < px(1.),
                    "each accepted steer must replace the previous prompt's top anchor"
                );
                window.resize(size(px(700.), px(760.)));
                cx.notify();
            })
            .unwrap();
        cx.refresh().unwrap();
        cx.run_until_parked();
    }

    window
        .update(cx, |view, _, cx| {
            let prompt = view.messages.iter().find(|m| m.start == 7).unwrap();
            let bounds = view.list.bounds_for_item(prompt.display_start).unwrap();
            assert!((bounds.top() - view.viewport.top()).abs() < px(1.));
            view.list.scroll_by(px(-100.));
            let offset = view.list.logical_scroll_top();
            view.patch(0, view.source.clone(), cx);
            assert!(
                !view.list.is_following_tail(),
                "a replay must respect the reader's scroll"
            );
            assert_eq!(view.list.logical_scroll_top().item_ix, offset.item_ix);
            assert_eq!(
                view.list.logical_scroll_top().offset_in_item,
                offset.offset_in_item
            );
            assert!(
                view.prompt_rise.is_none(),
                "a replay must not repeat the entrance"
            );
        })
        .unwrap();
}

#[gpui::test]
fn live_prompt_anchors_respect_history_visibility_and_the_anchor_preference(
    cx: &mut TestAppContext,
) {
    let entity = cx.new(|_| TranscriptView::new());
    entity.update(cx, |view, cx| {
        view.patch(0, vec![row("user", "Restored history")], cx);
        assert!(view.anchor_turn.is_none());
        assert!(view.prompt_rise.is_none());
        view.streaming(true, cx);
        view.visible(false, cx);
        view.patch(1, vec![row("user", "A steer while hidden")], cx);
        assert_eq!(view.anchor_turn, Some(1));
        assert!(view.prompt_rise.is_none());
        view.visible(true, cx);
        view.patch(1, vec![row("user", "A steer while hidden")], cx);
        assert!(
            view.prompt_rise.is_none(),
            "showing a tab must not replay a steer"
        );

        view.apply_preferences(
            &crate::ui::preferences::Preferences {
                anchor_prompts: false,
                ..Default::default()
            },
            cx,
        );
        view.list.pause_following_tail();
        view.patch(2, vec![row("user", "A steer with anchoring disabled")], cx);
        assert!(view.anchor_turn.is_none());
        assert!(view.prompt_rise.is_none());
        assert!(
            view.list.is_following_tail(),
            "a newly sent message still resumes following"
        );
    });
}

#[gpui::test]
fn sent_prompt_stays_at_the_top_then_long_replies_follow_until_scrolled_away(
    cx: &mut TestAppContext,
) {
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(900.), px(600.)),
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| TranscriptView::new()),
        )
        .unwrap()
    });
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
            view.streaming(true, cx);
            view.patch(
                2,
                vec![
                    row("user", "The newly sent prompt"),
                    row("assistant", "A short reply"),
                ],
                cx,
            );
            view.streaming(false, cx);
            settle(view, cx);
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, w, cx| {
            #[cfg(target_os = "macos")]
            {
                let markers: Vec<_> = w
                    .painted_quads()
                    .into_iter()
                    .filter(|q| {
                        q.border_color
                            == gpui_apple::metal_renderer::edge_fade_marker(
                                true,
                                TOP_FADE * w.scale_factor(),
                                BOTTOM_INSET * w.scale_factor(),
                            )
                            || q.border_color
                                == gpui_apple::metal_renderer::edge_fade_marker(
                                    false,
                                    TOP_FADE * w.scale_factor(),
                                    BOTTOM_INSET * w.scale_factor(),
                                )
                    })
                    .collect();
                assert_eq!(
                    markers.len(),
                    2,
                    "both mask boundaries must be painted inside the viewport"
                );
                assert_eq!(markers[0].bounds, markers[1].bounds);
            }
            let prompt = view
                .rows
                .iter()
                .position(|r| view.messages[r.message].start == 2)
                .unwrap();
            let bounds = view.list.bounds_for_item(prompt).unwrap();
            assert!(
                (bounds.top() - view.viewport.top()).abs() < px(1.),
                "prompt row must stay at the top, below the list's fade padding"
            );
            assert!(view.list.is_following_tail());
            w.resize(size(px(700.), px(760.)));
            cx.notify();
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, _, cx| {
            let prompt = view
                .rows
                .iter()
                .position(|r| view.messages[r.message].start == 2)
                .unwrap();
            assert!(
                (view.list.bounds_for_item(prompt).unwrap().top() - view.viewport.top()).abs()
                    < px(1.),
                "resizing must retain the anchored prompt"
            );
            view.patch(
                3,
                vec![row(
                    "assistant",
                    &"A paragraph of streamed reply text.\n\n".repeat(80),
                )],
                cx,
            );
            settle(view, cx);
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, _, cx| {
            assert!(view.list.is_following_tail());
            view.list.scroll_by(px(-150.));
            view.update_jump(cx);
            assert!(view.show_jump);
            let offset = view.list.logical_scroll_top();
            view.patch(
                3,
                vec![row(
                    "assistant",
                    &"A paragraph of streamed reply text.\n\n".repeat(90),
                )],
                cx,
            );
            settle(view, cx);
            assert_eq!(view.list.logical_scroll_top().item_ix, offset.item_ix);
            assert_eq!(
                view.list.logical_scroll_top().offset_in_item,
                offset.offset_in_item
            );
            view.jump_to_bottom(cx);
            assert!(!view.show_jump && view.list.is_following_tail());
        })
        .unwrap();
}
