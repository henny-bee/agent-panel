use super::super::*;
use gpui::{Bounds, EntityInputHandler, TestAppContext, WindowBounds, WindowOptions, point, size};
use monocode_core::store::Store;
use std::{sync::Mutex, time::Duration};

fn window(cx: &mut TestAppContext, width: f32) -> gpui::WindowHandle<Conversation> {
    let store = Store::open(Path::new(":memory:")).unwrap();
    let summary = store.create("codex", "/work/monocode").unwrap();
    let store = Arc::new(Mutex::new(store));
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(width), px(700.)),
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|cx| Conversation::new(summary, store, cx)),
        )
        .unwrap()
    });
    window
        .update(cx, |c, w, cx| {
            c.loaded = true;
            c.status = "Ready".into();
            w.focus(&c.input.focus_handle(cx), cx);
            c.input.update(cx, |i, cx| {
                i.replace_text_in_range(None, "First message", w, cx)
            });
            cx.notify();
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    window
}
fn begin(
    window: gpui::WindowHandle<Conversation>,
    cx: &mut TestAppContext,
) -> Bounds<gpui::Pixels> {
    let from = window
        .update(cx, |c, _, cx| {
            let from = c.composer_anchor.get().unwrap();
            // Use the first-send presentation path without a provider request.
            c.input.update(cx, |i, cx| {
                i.take(cx);
            });
            c.begin_turn(cx);
            cx.notify();
            from
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    from
}
#[gpui::test]
fn composer_uses_react_surface_and_equal_text_padding_with_attachments(cx: &mut TestAppContext) {
    let preferences = crate::ui::preferences::Preferences::default();
    crate::ui::theme::configure(&preferences, false);
    cx.update(|cx| {
        cx.set_global(preferences);
        cx.set_reduce_motion(true);
    });
    for width in [480., 1000.] {
        let window = window(cx, width);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        for docked in [false, true] {
            for attached in [false, true] {
                window
                    .update(visual, |c, _, cx| {
                        c.summary.has_messages = docked;
                        c.attachments = if attached {
                            vec![std::path::PathBuf::from("/work/notes.txt")]
                        } else {
                            Vec::new()
                        };
                        c.input.update(cx, |input, cx| {
                            input.set_text(
                                "First typed line\nSecond typed line\nThird typed line".into(),
                                cx,
                            );
                        });
                        cx.notify();
                    })
                    .unwrap();
                visual.run_until_parked();
                visual.refresh().unwrap();
                let composer = visual.debug_bounds("composer-box").unwrap();
                let field = visual.debug_bounds("composer-text-field").unwrap();
                let toolbar = visual.debug_bounds("composer-toolbar").unwrap();
                let top = if attached {
                    visual
                        .debug_bounds("composer-attachments")
                        .unwrap()
                        .bottom()
                } else {
                    composer.top() + px(1.)
                };
                assert_eq!(field.top() - top, px(10.));
                assert_eq!(toolbar.top() - field.bottom(), px(14.));
                assert_eq!(field.size.height, px(66.));
                window
                    .update(visual, |_, window, _| {
                        let scale = window.scale_factor();
                        let surface = window
                            .painted_quads()
                            .into_iter()
                            .find(|quad| {
                                (quad.corner_radii.top_left.0 / scale - 11.).abs() < 0.1
                                    && quad.background.as_solid() == Some(content(0.03))
                                    && (quad.bounds.size.width.0 / scale
                                        - f32::from(composer.size.width))
                                    .abs()
                                        <= 2.
                            })
                            .expect("the composer paints React's content-3% surface");
                        #[cfg(any(target_os = "macos", target_os = "windows"))]
                        assert_eq!(
                            surface.border_color,
                            gpui::backdrop_blur_marker(24. * scale)
                        );
                        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                        let _ = surface;
                        assert!(window.painted_quads().iter().any(|quad| {
                            quad.border_color == content(0.2)
                                && (quad.bounds.size.width.0 / scale
                                    - f32::from(composer.size.width))
                                .abs()
                                    < 0.1
                        }));
                    })
                    .unwrap();
            }
        }
    }
}

#[gpui::test]
fn first_submit_moves_the_painted_composer_down_without_resizing_chat(cx: &mut TestAppContext) {
    let window = window(cx, 1000.);
    let from = begin(window, cx);
    let (initial, transcript) = window
        .update(cx, |c, _, cx| {
            (
                c.composer_anchor.get().unwrap(),
                c.transcript.read(cx).list.viewport_bounds(),
            )
        })
        .unwrap();
    assert!(
        (initial.top() - from.top()).abs() < px(1.),
        "the first docked frame must stay at the empty-session slot"
    );
    window
        .update(cx, |c, _, cx| {
            c.composer_motion.elapsed(Duration::from_millis(120));
            cx.notify();
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    let middle = window
        .update(cx, |c, w, cx| {
            let bounds = c.composer_anchor.get().unwrap();
            let painted = w
                .painted_quads()
                .into_iter()
                .find(|q| {
                    (q.corner_radii.top_left.0 / w.scale_factor() - 12.).abs() < 0.1
                        && q.bounds.size.width.0 / w.scale_factor() > 500.
                })
                .expect("the composer box must be painted at its animated position");
            assert!(
                (painted.bounds.top().0 / w.scale_factor() - f32::from(bounds.top())).abs() < 1.
            );
            assert!(c.input.focus_handle(cx).is_focused(w));
            c.composer_motion.elapsed(Duration::from_millis(480));
            cx.notify();
            bounds
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    let docked = window
        .update(cx, |c, _, _| c.composer_anchor.get().unwrap())
        .unwrap();
    assert!(
        middle.top() > from.top() + px(30.),
        "the composer must slide down"
    );
    assert!(
        middle.top() < docked.top() - px(5.),
        "an intermediate frame must not already be docked"
    );
    assert!((docked.bottom() - px(694.)).abs() < px(1.));
    assert_eq!(
        window
            .update(cx, |c, _, cx| c.transcript.read(cx).list.viewport_bounds())
            .unwrap(),
        transcript,
        "composer motion must not relayout the transcript"
    );
}
#[gpui::test]
fn narrow_layout_and_opening_history_do_not_slide(cx: &mut TestAppContext) {
    let narrow = window(cx, 820.);
    narrow
        .update(cx, |c, _, cx| c.set_beside_pane(true, cx))
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    let from = begin(narrow, cx);
    narrow
        .update(cx, |c, _, _| {
            assert!((c.composer_anchor.get().unwrap().top() - from.top()).abs() < px(1.));
        })
        .unwrap();
    let history = window(cx, 1000.);
    let from = history
        .update(cx, |c, _, cx| {
            let from = c.composer_anchor.get().unwrap();
            c.summary.has_messages = true;
            cx.notify();
            from
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    history
        .update(cx, |c, _, _| {
            assert!(c.composer_anchor.get().unwrap().top() > from.top() + px(100.));
        })
        .unwrap();
}
#[gpui::test]
fn reduced_motion_docks_the_first_submit_immediately(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let window = window(cx, 1000.);
    let from = begin(window, cx);
    window
        .update(cx, |c, _, _| {
            assert!(c.composer_anchor.get().unwrap().top() > from.top() + px(100.));
        })
        .unwrap();
}
