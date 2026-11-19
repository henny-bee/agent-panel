use super::*;

#[gpui::test]
fn pane_surfaces_follow_the_edge_at_a_constant_width(cx: &mut TestAppContext) {
    use std::time::Duration;
    let (window, _, id) = window(cx);
    window
        .update(cx, |shell, w, cx| {
            shell.open(id, w, cx);
            shell.toggle_right_panel(w, cx);
        })
        .unwrap();
    for ms in [30, 65, 110, 170] {
        window
            .update(cx, |shell, _, cx| {
                shell
                    .right_slide
                    .as_ref()
                    .unwrap()
                    .freeze_elapsed(Duration::from_millis(ms));
                cx.notify();
            })
            .unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        })
        .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        let clip = visual.debug_bounds("right-panel-frame").unwrap();
        let content = visual.debug_bounds("right-panel-content").unwrap();
        assert!((content.left() - clip.left()).abs() < px(2.));
        let full = window
            .update(visual, |shell, _, _| shell.workspace.right_panel_width)
            .unwrap();
        assert!((content.size.width - px(full - 1.)).abs() < px(1.));
        assert!(content.size.width > clip.size.width);
    }
    window
        .update(cx, |shell, w, cx| {
            shell
                .right_slide
                .as_ref()
                .unwrap()
                .elapsed(Duration::from_secs(1));
            shell.open_review(
                crate::git::review::ReviewTab::Changes {
                    cwd: "/work/monocode".into(),
                    path: None,
                    staged: None,
                },
                w,
                cx,
            );
            shell
                .review_slide
                .as_ref()
                .unwrap()
                .1
                .freeze_elapsed(Duration::ZERO);
        })
        .unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    })
    .unwrap();
    let mut settled_width = None;
    for ms in [30, 65, 110, 170] {
        window
            .update(cx, |shell, _, cx| {
                shell
                    .review_slide
                    .as_ref()
                    .unwrap()
                    .1
                    .freeze_elapsed(Duration::from_millis(ms));
                cx.notify();
            })
            .unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        })
        .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        let clip = visual.debug_bounds("review-frame").unwrap();
        let content = visual.debug_bounds("review-content").unwrap();
        assert!((content.left() - clip.left()).abs() < px(2.));
        let expected = *settled_width.get_or_insert(content.size.width);
        assert!((content.size.width - expected).abs() < px(1.));
        assert!(content.size.width > clip.size.width);
    }
    cx.update(|cx| cx.set_reduce_motion(true));
    cx.update_window(window.into(), |_, window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    })
    .unwrap();
    window
        .update(cx, |shell, _, _| {
            assert!(shell.right_slide.is_none());
            assert!(shell.review_slide.is_none());
        })
        .unwrap();
}

#[cfg(target_os = "macos")]
#[gpui::test]
#[ignore = "Manual CoreText CPU frame-cost measurement; opens no native window"]
fn pane_motion_frame_cost_with_long_history(cx: &mut TestAppContext) {
    use std::time::{Duration, Instant};
    let cx = &mut TestAppContext::build_with_text_system(
        cx.dispatcher.clone(),
        Some("pane_motion_frame_cost"),
        Arc::new(gpui_apple::AppleTextSystem::new()),
    );
    let (window, _, id) = window(cx);
    let transcript = window.update(cx, |shell, w, cx| {
        shell.open(id, w, cx);
        let session = shell.sessions[&id].clone();
        session.update(cx, |session, cx| {
            session.summary.has_messages = true;
            let mut history = monocode_core::session::transcript::Transcript::default();
            for _ in 0..10_000 {
                history.begin("assistant");
                history.append("A conversation with enough text to wrap while its pane changes width.\nA second line of native text.");
            }
            session.transcript.update(cx, |view, cx| view.patch(0, history.rows, cx));
            cx.notify();
            session.transcript.clone()
        })
    }).unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    })
    .unwrap();
    let mut frames = Vec::new();
    let mut settling_passes = 0;
    for opening in [true, false] {
        window
            .update(cx, |shell, w, cx| {
                shell.toggle_right_panel(w, cx);
                if let Some(slide) = &shell.right_slide {
                    slide.freeze_elapsed(Duration::ZERO);
                }
            })
            .unwrap();
        for frame in 1..=40 {
            let root = window.root(cx).unwrap();
            let (before, sliding) = root.read_with(cx, |shell, _| {
                (shell.render_passes, shell.right_slide.is_some())
            });
            let start = Instant::now();
            window
                .update(cx, |shell, _, cx| {
                    if let Some(slide) = &shell.right_slide {
                        slide.freeze_elapsed(Duration::from_secs_f32(frame as f32 / 120.));
                    }
                    cx.notify();
                })
                .unwrap();
            if root.read_with(cx, |shell, _| shell.render_passes) == before {
                cx.update_window(window.into(), |_, window, cx| {
                    window.simulate_next_frame(cx);
                    window.draw(cx).clear(cx);
                })
                .unwrap();
            }
            let (after, settled) = root.read_with(cx, |shell, _| {
                (shell.render_passes, shell.right_slide.is_none())
            });
            let passes = after - before;
            // Finishing the slide installs the settled panel layout and
            // can notify the shell once more. Include that work in this
            // sample; every other frame must still render exactly once.
            assert!(
                passes == 1 || (passes == 2 && sliding && settled),
                "expected one render, or two at settling; got {passes}"
            );
            settling_passes += passes - 1;
            frames.push(start.elapsed().as_secs_f64() * 1000.);
            let rendered = transcript.read_with(cx, |view, _| view.rendered);
            assert!(
                rendered > 0 && rendered < 100,
                "expected visible history work, got {rendered}"
            );
        }
        window
            .update(cx, |shell, _, _| {
                assert_eq!(shell.workspace.right_panel_open, opening)
            })
            .unwrap();
    }
    frames.sort_by(f64::total_cmp);
    eprintln!(
        "CoreText, 10,000 messages, {} simulated 120Hz frames: CPU p50={:.2}ms p95={:.2}ms max={:.2}ms ({settling_passes} extra settling passes included; excludes presentation/GPU)",
        frames.len(),
        frames[frames.len() / 2],
        frames[frames.len() * 95 / 100],
        frames.last().unwrap()
    );
}

#[gpui::test]
fn explorer_files_share_the_review_pane_and_dirty_close_can_cancel_or_save(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| cx.set_reduce_motion(true));
    use gpui::EntityInputHandler;
    let path = std::env::temp_dir().join(format!("monocode-file-pane-{}.rs", std::process::id()));
    std::fs::write(&path, "fn main() {}\n").unwrap();
    let (window, _, id) = window(cx);
    let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    window
        .update(cx, |shell, w, cx| {
            shell.open(id, w, cx);
            shell.open_review(
                crate::git::review::ReviewTab::File {
                    cwd: "/tmp".into(),
                    path: path.to_string_lossy().into_owned(),
                },
                w,
                cx,
            );
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, w, cx| {
            let pane = shell.reviews[&id].clone();
            pane.update(cx, |pane, cx| {
                pane.navigate_file(1, 4, cx);
                let editor = pane.active_editor().unwrap();
                let input = editor.read(cx).input.clone();
                input.update(cx, |i, cx| i.replace_text_in_range(None, " edited", w, cx));
            });
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            assert!(shell.unsaved_files(cx));
            shell.reviews[&id].update(cx, |pane, cx| pane.close_active(cx));
        })
        .unwrap();
    cx.refresh().unwrap();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(shell.reviews[&id].read(cx).state.tabs.len(), 1);
            assert!(shell.unsaved_files(cx));
            shell.reviews[&id].update(cx, |pane, cx| pane.close_active(cx));
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.simulate_prompt_answer("Save");
    cx.run_until_parked();
    assert!(std::fs::read_to_string(&path).unwrap().contains(" edited"));
    window
        .update(cx, |shell, _, cx| {
            assert!(!shell.reviews.contains_key(&id));
            assert!(!shell.unsaved_files(cx));
        })
        .unwrap();
    std::fs::remove_file(path).unwrap();
}

#[gpui::test]
fn panes_split_resize_and_take_drops_from_panes_tabs_and_the_sidebar(cx: &mut TestAppContext) {
    use gpui::{Modifiers, point};
    let (window, a, b) = window(cx);
    cx.update(|cx| {
        bind_keys(cx);
        cx.set_reduce_motion(true);
    });
    window
        .update(cx, |shell, w, cx| {
            shell.sessions[&b].update(cx, |chat, _| chat.summary.has_messages = true);
            shell.open(b, w, cx);
            shell.focus_active(w, cx);
        })
        .unwrap();
    let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    cx.run_until_parked();
    let count_selector: &'static str =
        Box::leak(format!("title-tab-pane-count-{b}").into_boxed_str());
    let close_selector: &'static str = Box::leak(format!("close-title-tab-{b}").into_boxed_str());
    assert!(
        cx.debug_bounds(close_selector).is_some(),
        "a single pane can still be closed"
    );
    assert!(
        cx.debug_bounds(count_selector).is_none(),
        "a single pane has no count badge"
    );
    cx.simulate_keystrokes("secondary-d");
    cx.run_until_parked();
    let new = window
        .update(cx, |shell, _, _| {
            let new = shell.workspace.active.unwrap();
            assert_eq!(shell.workspace.tabs, [a, b]);
            assert_eq!(shell.workspace.layout(b).leaf_ids(), [b, new]);
            new
        })
        .unwrap();
    window
        .update(cx, |shell, _, cx| {
            // Two sessions, one line each; the new pane has focus.
            let copy = shell.tab_copy(b, cx);
            let keys: Vec<_> = copy.panes.iter().map(|p| p.key).collect();
            assert_eq!(keys, [(b, false), (new, false)]);
            assert_eq!(copy.focus, 1);
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    assert!(
        cx.debug_bounds(count_selector).is_some(),
        "split tabs show their pane count"
    );
    let left = cx
        .debug_bounds(Box::leak(format!("pane-{b}").into_boxed_str()))
        .unwrap();
    let right = cx
        .debug_bounds(Box::leak(format!("pane-{new}").into_boxed_str()))
        .unwrap();
    assert!((left.right() - right.left()).abs() < px(1.));
    assert!((left.size.width - right.size.width).abs() < px(2.));

    // The sash between them drags the split.
    let sash = point(left.right(), left.center().y);
    cx.simulate_mouse_down(sash, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        sash - point(px(100.), px(0.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        sash - point(px(100.), px(0.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| {
            let leaves = shell.workspace.layout(b).leaves();
            assert!(leaves[0].rect.w < 0.45, "{leaves:?}");
        })
        .unwrap();

    // A sidebar session dropped on a pane's lower half stacks under it,
    // and the tab that held it closes.
    window
        .update(cx, |shell, _, cx| {
            shell.sessions[&a].update(cx, |s, _| {
                s.summary.cwd = "/work/monocode".into();
                s.summary.has_messages = true;
            });
            shell.invalidate_chrome(cx);
        })
        .unwrap();
    cx.run_until_parked();
    let row = cx
        .debug_bounds(Box::leak(format!("sidebar-session-{a}").into_boxed_str()))
        .unwrap()
        .center();
    let below = point(right.center().x, right.bottom() - px(20.));
    cx.simulate_mouse_down(row, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        row + point(px(20.), px(0.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_move(below, MouseButton::Left, Modifiers::default());
    window
        .update(cx, |shell, _, _| {
            let drag = shell.pane_drag.as_ref().unwrap();
            assert_eq!(drag.over, Some((new, Edge::Bottom)));
        })
        .unwrap();
    cx.simulate_mouse_up(below, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| {
            assert_eq!(shell.workspace.tabs, [b]);
            assert_eq!(shell.workspace.layout(b).leaf_ids(), [b, new, a]);
            assert_eq!(shell.workspace.active, Some(a));
            assert!(shell.pane_drag.is_none());
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();

    // Its header drags it up onto the title strip: a tab of its own.
    let grip = cx
        .debug_bounds(Box::leak(format!("drag-session-pane-{a}").into_boxed_str()))
        .unwrap()
        .center();
    let strip = cx
        .debug_bounds(Box::leak(format!("title-tab-{b}").into_boxed_str()))
        .unwrap();
    let before_b = point(strip.left() + px(10.), strip.center().y);
    cx.simulate_mouse_down(grip, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        grip + point(px(0.), px(-20.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_move(before_b, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(before_b, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| {
            assert_eq!(shell.workspace.tabs, [a, b]);
            assert_eq!(shell.workspace.layout(b).leaf_ids(), [b, new]);
            assert_eq!(shell.workspace.active, Some(a));
        })
        .unwrap();

    // A tab press whose release the title bar swallowed stays inert:
    // dragging a pane below never drags that tab.
    let pressed = cx
        .debug_bounds(Box::leak(format!("title-tab-{b}").into_boxed_str()))
        .unwrap()
        .center();
    window
        .update(cx, |shell, _, cx| {
            let event = MouseDownEvent {
                position: pressed,
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
                click_count: 1,
                first_mouse: false,
            };
            shell.start_tab_drag(TitleTab::Session(b), &event, cx);
            assert!(shell.tab_drag.is_some());
        })
        .unwrap();
    cx.simulate_mouse_move(
        pressed + point(px(0.), px(300.)),
        None,
        Modifiers::default(),
    );
    window
        .update(cx, |shell, _, _| assert!(shell.tab_drag.is_none()))
        .unwrap();
    // A title tab pulled down onto a pane's left half moves in beside it.
    window
        .update(cx, |shell, w, cx| shell.open(b, w, cx))
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    let tab = cx
        .debug_bounds(Box::leak(format!("title-tab-{a}").into_boxed_str()))
        .unwrap()
        .center();
    let pane = cx
        .debug_bounds(Box::leak(format!("pane-{b}").into_boxed_str()))
        .unwrap();
    let left_half = point(pane.left() + px(20.), pane.center().y);
    cx.simulate_mouse_down(tab, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        point(tab.x, px(TITLE_BAR_HEIGHT + PANE_DROP_PULL + 10.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    // Pulled out of the strip, sideways motion no longer slides the tab.
    cx.simulate_mouse_move(
        point(
            tab.x + px(300.),
            px(TITLE_BAR_HEIGHT + PANE_DROP_PULL + 10.),
        ),
        MouseButton::Left,
        Modifiers::default(),
    );
    window
        .update(cx, |shell, _, _| {
            let drag = shell.tab_drag.as_ref().unwrap();
            assert!(drag.armed);
            assert_eq!((drag.offset, drag.to), (px(0.), drag.from));
            assert!(shell.pane_drag.is_some());
        })
        .unwrap();
    cx.simulate_mouse_move(left_half, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(left_half, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| {
            assert_eq!(shell.workspace.tabs, [b]);
            assert_eq!(shell.workspace.layout(b).leaf_ids(), [a, b, new]);
            assert_eq!(shell.workspace.active, Some(a));
        })
        .unwrap();

    // ⌘W closes the focused pane only; its neighbour takes focus.
    cx.simulate_keystrokes("secondary-w");
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| {
            assert_eq!(shell.workspace.tabs, [b]);
            assert_eq!(shell.workspace.layout(b).leaf_ids(), [b, new]);
            assert_eq!(shell.workspace.active, Some(b));
        })
        .unwrap();
}
