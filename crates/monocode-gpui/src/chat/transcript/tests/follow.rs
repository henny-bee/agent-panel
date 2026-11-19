mod measure;
mod mono_badges;
mod reading;
mod settling;
mod turns;
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

fn chat(cx: &mut TestAppContext) -> gpui::WindowHandle<TranscriptView> {
    chat_mode(cx, true)
}

fn chat_mode(cx: &mut TestAppContext, mono: bool) -> gpui::WindowHandle<TranscriptView> {
    cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(900.), px(600.)),
                ))),
                ..Default::default()
            },
            move |_, cx| {
                cx.new(|_| {
                    let mut view = TranscriptView::new();
                    if mono {
                        view.mono_mode("cat", 0xe86b5f, "MonoCat");
                    }
                    view
                })
            },
        )
        .unwrap()
    })
}

fn render(window: gpui::WindowHandle<TranscriptView>, cx: &mut TestAppContext) {
    window.update(cx, |_, _, cx| cx.notify()).unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
}

struct SubmitChat {
    transcript: gpui::Entity<TranscriptView>,
    input: gpui::Entity<crate::ui::input::Input>,
}
impl gpui::Render for SubmitChat {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h_0().child(self.transcript.clone()))
            .child(div().w_full().p_2().child(self.input.clone()))
    }
}

fn input_chat(cx: &mut TestAppContext, text: &str) -> gpui::WindowHandle<SubmitChat> {
    cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(500.), px(700.)),
                ))),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|cx| {
                    let input = cx.new(crate::ui::input::Input::new);
                    input.update(cx, |input, cx| {
                        input.mono_composer("MonoCat", cx);
                        input.set_text(text.to_owned(), cx);
                    });
                    let transcript = cx.new(|cx| {
                        let mut view = TranscriptView::new();
                        view.mono_mode("cat", 0xe86b5f, "MonoCat");
                        view.patch(
                            0,
                            vec![row("user", "Earlier"), row("assistant", "Done")],
                            cx,
                        );
                        view
                    });
                    SubmitChat { transcript, input }
                })
            },
        )
        .unwrap()
    })
}

fn bubble_top(window: &mut Window) -> f32 {
    window
        .painted_quads()
        .into_iter()
        .filter(|q| {
            q.corner_radii.top_left.0 > 0.
                && q.corner_radii.top_left.0 * 2. == q.bounds.size.height.0
        })
        .map(|q| q.bounds.top().0 / window.scale_factor())
        .next()
        .expect("the sent bubble is painted")
}

fn tool(ix: usize, done: bool) -> Row {
    row(
        "tool",
        &serde_json::to_string(&ToolCall {
            id: ix.to_string(),
            name: "Bash".into(),
            input: format!(r#"{{"command":"rg pattern-{ix} src"}}"#),
            output: if done {
                "match\n".repeat(6)
            } else {
                String::new()
            },
            state: if done {
                ToolState::Completed
            } else {
                ToolState::Running
            },
            panel: None,
            display: None,
        })
        .unwrap(),
    )
}

fn evicted_turns() -> Vec<Row> {
    (0..12)
        .flat_map(|i| {
            [
                row("user", &format!("Question {i}")),
                tool(i, true),
                row(
                    "assistant",
                    &format!(
                        "Answer {i}.\n\n{}",
                        "A long paragraph of prose. ".repeat(40)
                    ),
                ),
            ]
        })
        .collect()
}

/// Evict an inactive transcript, then install the same history through
/// `install_prepared_history`, as a reload that does not go through the local
/// reopen path does, and report where the reader is afterwards.
fn reinstall_after_eviction(
    cx: &mut TestAppContext,
    reading: bool,
    grown: bool,
) -> ((String, usize, f32), bool, bool) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let window = chat_mode(cx, false);
    window
        .update(cx, |view, _, cx| {
            view.visible = false;
            view.install_prepared_history(PreparedHistory::new(evicted_turns(), false), cx);
            view.visible = true;
        })
        .unwrap();
    render(window, cx);
    let top = |view: &TranscriptView| {
        let at = view.list.logical_scroll_top();
        let row = &view.rows[at.item_ix];
        (
            view.source[row.key.0].text.clone(),
            row.key.1,
            f32::from(at.offset_in_item),
        )
    };
    window
        .update(cx, |view, _, _| {
            if reading {
                let ix = view.rows.iter().position(|r| r.key.0 == 15).unwrap();
                view.list.scroll_to(ListOffset {
                    item_ix: ix,
                    offset_in_item: px(13.),
                });
            }
        })
        .unwrap();
    render(window, cx);
    let expected = window
        .update(cx, |view, _, _| {
            assert_eq!(view.list.is_following_tail(), !reading);
            top(view)
        })
        .unwrap();
    window
        .update(cx, |view, _, cx| {
            view.visible = false;
            assert!(view.suspend_history(cx));
            view.visible = true;
            let mut rows = evicted_turns();
            if grown {
                rows.push(row("assistant", "One more reply."));
            }
            view.install_prepared_history(PreparedHistory::new(rows, false), cx);
        })
        .unwrap();
    render(window, cx);
    window
        .update(cx, |view, _, _| {
            let after = top(view);
            let same = if reading {
                after == expected
            } else {
                view.list.is_following_tail()
            };
            (after, same, view.history_position.is_none())
        })
        .unwrap()
}
