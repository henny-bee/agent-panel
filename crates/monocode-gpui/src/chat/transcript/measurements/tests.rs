use super::*;
use gpui::{Entity, Render, TestAppContext, WindowBounds, WindowOptions, point};
use std::{cell::RefCell, rc::Rc, time::Duration};

type Probe = Box<dyn FnOnce(&mut TranscriptView, &mut Window, &mut Context<TranscriptView>)>;

struct Fixture {
    transcript: Entity<TranscriptView>,
    probe: Option<Probe>,
}
impl Render for Fixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let transcript = self.transcript.clone();
        let probe = self.probe.take();
        div()
            .relative()
            .size_full()
            .child(self.transcript.clone())
            .child(
                canvas(
                    move |_, window, cx| {
                        if let Some(probe) = probe {
                            transcript.update(cx, |view, cx| probe(view, window, cx));
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
    }
}

fn row(role: &str, text: &str) -> Row {
    Row {
        role: role.into(),
        text: text.into(),
        first: true,
        sent_at: None,
    }
}

fn chat(cx: &mut TestAppContext) -> gpui::WindowHandle<Fixture> {
    fixture(cx, false)
}

fn fixture(cx: &mut TestAppContext, mono: bool) -> gpui::WindowHandle<Fixture> {
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
                let transcript = cx.new(|cx| {
                    let mut view = TranscriptView::new();
                    if mono {
                        view.mono_mode("cat", 0xe86b5f, "MonoCat");
                    }
                    view.visible = false;
                    view.streaming = true;
                    view.smooth_reveal = false;
                    view.patch(
                        0,
                        vec![row("user", "Question"), row("assistant", "A short answer.")],
                        cx,
                    );
                    view.visible = true;
                    view
                });
                cx.new(|_| Fixture {
                    transcript,
                    probe: None,
                })
            },
        )
        .unwrap()
    });
    render(window, cx);
    render(window, cx);
    window
}

fn render(window: gpui::WindowHandle<Fixture>, cx: &mut TestAppContext) {
    window.update(cx, |_, _, cx| cx.notify()).unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
}

// GPUI layout must run in a real draw phase, rather than an update handler.
fn probe<R: 'static>(
    window: gpui::WindowHandle<Fixture>,
    cx: &mut TestAppContext,
    inspect: impl FnOnce(&mut TranscriptView, &mut Window, &mut Context<TranscriptView>) -> R + 'static,
) -> R {
    let result = Rc::new(RefCell::new(None));
    let output = result.clone();
    window
        .update(cx, |fixture, _, cx| {
            fixture.probe = Some(Box::new(move |view, window, cx| {
                *output.borrow_mut() = Some(inspect(view, window, cx));
            }));
            cx.notify();
        })
        .unwrap();
    render(window, cx);

    result.borrow_mut().take().expect("the layout probe ran")
}

fn answer(view: &TranscriptView) -> usize {
    view.rows
        .iter()
        .position(|row| matches!(row.body, Body::Markdown(_)))
        .unwrap()
}

#[test]
fn prompt_padding_does_not_shorten_fractional_text_widths() {
    // These advances straddle an f32 precision boundary after padding is
    // added. The old calculation makes the available text width smaller.
    for base in [104_f32, 232., 488.] {
        let text = px(f32::from_bits(base.to_bits() + 1));
        assert!((text + px(24.)) - px(24.) < text);
        let bubble = prompt_bubble_width([px(40.), text, px(60.)].into_iter());
        assert!(bubble - px(24.) >= text, "the widest line must still fit");
    }
}

#[test]
fn prompt_width_stops_measuring_at_the_bubble_cap() {
    let past_cap = std::iter::from_fn(|| -> Option<Pixels> { panic!("measured past the cap") });
    let lines = [px(100.), PROMPT_BUBBLE_MAX - PROMPT_PAD_X].into_iter();
    assert_eq!(
        prompt_bubble_width(lines.chain(past_cap)),
        PROMPT_BUBBLE_MAX
    );
    assert_eq!(
        prompt_bubble_width([px(551.5)].into_iter()),
        PROMPT_BUBBLE_MAX
    );
    assert_eq!(prompt_bubble_width([px(400.2)].into_iter()), px(425.));
    assert_eq!(
        prompt_bubble_width([px(10.)].into_iter()),
        PROMPT_DISCLOSURE_MIN + PROMPT_PAD_X
    );
}

#[cfg(target_os = "macos")]
#[test]
fn shared_prompt_width_keeps_real_font_lines_unwrapped() {
    let system = gpui::WindowTextSystem::new(Arc::new(gpui::TextSystem::new(Arc::new(
        gpui_apple::AppleTextSystem::new(),
    ))));
    for family in [".AppleSystemUIFont", "Helvetica", "Times New Roman"] {
        let (mut fitted, mut capped) = (0, 0);
        for count in 1..40 {
            let text: SharedString = format!("{} final word", "iW ".repeat(count)).into();
            let run = gpui::TextRun {
                len: text.len(),
                font: gpui::font(family),
                color: Default::default(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let natural = system
                .shape_text(
                    text.clone(),
                    PROMPT_TEXT_SIZE,
                    std::slice::from_ref(&run),
                    None,
                    None,
                )
                .unwrap();
            let widest = natural
                .iter()
                .map(|line| line.width())
                .fold(px(0.), Pixels::max);
            let width = prompt_bubble_width(natural.iter().map(|line| line.width()));
            if widest >= PROMPT_BUBBLE_MAX - PROMPT_PAD_X {
                // A capped bubble wraps its widest line, as render does.
                assert_eq!(width, PROMPT_BUBBLE_MAX, "{family}, {count}");
                capped += 1;
                continue;
            }
            let wrapped = system
                .shape_text(
                    text,
                    PROMPT_TEXT_SIZE,
                    &[run],
                    Some(width - PROMPT_PAD_X),
                    None,
                )
                .unwrap();
            assert!(
                wrapped.iter().all(|line| line.wrap_boundaries.is_empty()),
                "the widest line must not acquire another line: {family}, {count}"
            );
            fitted += 1;
        }
        assert!(fitted > 0 && capped > 0, "both paths ran for {family}");
    }
}

#[gpui::test]
fn prompt_widths_survive_streaming_suffix_updates_and_refresh_after_edits(cx: &mut TestAppContext) {
    let window = chat(cx);
    probe(window, cx, |view, window, cx| {
        view.visible = false;
        let mut transcript = monocode_core::session::transcript::Transcript::default();
        transcript.begin("user");
        transcript.append(&"An earlier prompt\n".repeat(13));
        transcript.begin("assistant");
        transcript.append("An earlier answer");
        let prompt = transcript.begin("user");
        transcript.append(&format!(
            "A wide prompt line for measurement\n{}",
            "x\n".repeat(512)
        ));
        let assistant = transcript.begin("assistant");
        transcript.append("Answer");
        view.expanded_prompts.insert(0);
        view.expanded_prompts.insert(prompt);
        view.patch(0, transcript.rows, cx);
        view.begin_measurements(window, cx);
        let prefix_width = view.split_prompt_width(0, window).unwrap();
        let tail = view
            .rows
            .iter()
            .position(|row| view.messages[row.message].start == prompt && row.last)
            .unwrap();
        let width = view.split_prompt_width(tail, window).unwrap();
        let fresh = view.measurements.fresh_prompt_widths;
        for i in 0..10 {
            // More than one draw may separate streaming source patches.
            for _ in 0..3 {
                view.begin_measurements(window, cx);
            }
            view.patch(
                assistant,
                vec![row("assistant", &format!("Answer {i}"))],
                cx,
            );
            assert_eq!(view.split_prompt_width(tail, window), Some(width));
            assert_eq!(
                view.measurements.fresh_prompt_widths, fresh,
                "an unchanged, expanded prompt must not be measured again"
            );
        }
        // A continuation edit must invalidate the opening message's key,
        // even though the supplied source row is not its first row.
        let mut edited = view.source[prompt + 1].clone();
        assert!(!edited.first);
        edited.text = "A longer replacement line ".repeat(20);
        view.patch(prompt + 1, vec![edited], cx);
        assert!(view.split_prompt_width(tail, window).unwrap() > width);
        assert_eq!(view.split_prompt_width(0, window), Some(prefix_width));
        assert_eq!(view.measurements.fresh_prompt_widths, fresh + 1);

        window.set_rem_size(window.rem_size() + px(1.));
        view.begin_measurements(window, cx);
        assert!(
            view.measurements.prompt_widths.is_empty(),
            "style changes must still invalidate all prompt widths"
        );
        view.split_prompt_width(tail, window).unwrap();
        assert_eq!(view.measurements.fresh_prompt_widths, fresh + 2);
    });
}

fn prompt_tail(view: &TranscriptView, prompt: usize) -> usize {
    view.rows
        .iter()
        .position(|row| view.messages[row.message].start == prompt && row.last)
        .unwrap()
}

#[gpui::test]
fn mono_prompt_widths_survive_streaming_replies_and_refresh_with_attachments(
    cx: &mut TestAppContext,
) {
    let window = fixture(cx, true);
    probe(window, cx, |view, window, cx| {
        assert!(view.mono && view.chat_layout);
        view.visible = false;
        let mut transcript = monocode_core::session::transcript::Transcript::default();
        let prompt = transcript.begin("user");
        transcript.append(&format!(
            "A wide prompt line for measurement\n{}",
            "x\n".repeat(64)
        ));
        let assistant = transcript.begin("assistant");
        transcript.append("Answer");
        view.expanded_prompts.insert(prompt);
        view.patch(0, transcript.rows, cx);
        view.begin_measurements(window, cx);
        let width = view
            .split_prompt_width(prompt_tail(view, prompt), window)
            .unwrap();
        let fresh = view.measurements.fresh_prompt_widths;
        for i in 0..10 {
            for _ in 0..3 {
                view.begin_measurements(window, cx);
            }
            // Mono rebuilds the whole group, from the prompt that opens it.
            view.patch(
                assistant,
                vec![row("assistant", &format!("Answer {i}"))],
                cx,
            );
            let tail = prompt_tail(view, prompt);
            assert_eq!(view.split_prompt_width(tail, window), Some(width));
            assert_eq!(
                view.measurements.fresh_prompt_widths, fresh,
                "a streaming reply must not measure its prompt again"
            );
        }
        // The manifest after a prompt changes its attachments, not its text.
        let manifest = Row {
            role: monocode_core::session::transcript::attachments::ROLE.into(),
            text: monocode_core::session::transcript::attachments::from_paths(&[
                std::path::PathBuf::from("/tmp/notes.txt"),
            ])
            .to_string(),
            first: true,
            sent_at: None,
        };
        view.patch(assistant, vec![manifest, row("assistant", "Answer")], cx);
        assert!(
            !view.messages[view.rows[prompt_tail(view, prompt)].message]
                .attachments
                .is_empty()
        );
        let tail = prompt_tail(view, prompt);
        assert_eq!(
            view.split_prompt_width(tail, window),
            Some(PROMPT_BUBBLE_MAX)
        );
        assert_eq!(view.measurements.fresh_prompt_widths, fresh + 1);
    });
}

#[gpui::test]
fn prompt_widths_survive_viewport_resizes(cx: &mut TestAppContext) {
    let window = chat(cx);
    let (prompt, width, viewport, fresh) = probe(window, cx, |view, window, cx| {
        let mut transcript = monocode_core::session::transcript::Transcript::default();
        let prompt = transcript.begin("user");
        transcript.append(&format!("A wide prompt line\n{}", "x\n".repeat(24)));
        transcript.begin("assistant");
        transcript.append("Answer");
        view.expanded_prompts.insert(prompt);
        view.patch(0, transcript.rows, cx);
        view.begin_measurements(window, cx);
        let width = view
            .split_prompt_width(prompt_tail(view, prompt), window)
            .unwrap();
        (
            prompt,
            width,
            view.viewport.size,
            view.measurements.fresh_prompt_widths,
        )
    });
    for width in [600., 400., 900.] {
        cx.simulate_window_resize(window.into(), size(px(width), px(600.)));
        render(window, cx);
        render(window, cx);
    }
    cx.simulate_window_resize(window.into(), size(px(500.), px(600.)));
    render(window, cx);
    probe(window, cx, move |view, window, cx| {
        view.begin_measurements(window, cx);
        assert_ne!(view.viewport.size, viewport, "the viewport resized");
        assert_eq!(view.measurements.viewport, view.viewport.size);
        let tail = prompt_tail(view, prompt);
        assert_eq!(view.split_prompt_width(tail, window), Some(width));
        assert_eq!(
            view.measurements.fresh_prompt_widths, fresh,
            "a resize must not measure prompt widths again"
        );
    });
}

#[gpui::test]
fn retained_row_measurements_skip_unchanged_layout_and_match_fresh_heights(
    cx: &mut TestAppContext,
) {
    let window = chat(cx);
    probe(window, cx, |view, window, cx| {
        let ix = answer(view);
        let scroll = view.list.logical_scroll_top();
        view.begin_measurements(window, cx);
        let before = view.measurements.fresh;
        let retained = view.measure_row_height(ix, window, cx);
        assert_eq!(view.measurements.fresh, before);
        view.measurements.clear();
        let fresh = view.measure_row_height(ix, window, cx);
        assert!((retained - fresh).abs() < px(0.01));
        assert_eq!(view.measurements.fresh, before + 1);
        assert_eq!(view.list.logical_scroll_top().item_ix, scroll.item_ix);
        assert_eq!(
            view.list.logical_scroll_top().offset_in_item,
            scroll.offset_in_item
        );
        view.begin_measurements(window, cx);
        view.update_follow(window, cx);
        let after_follow = view.measurements.fresh;
        let first_footer = view.footer_height(window, cx);
        assert_eq!(view.footer_height(window, cx), first_footer);
        assert_eq!(view.measurements.fresh, after_follow);
    });
}

#[gpui::test]
fn retained_row_measurements_refresh_after_text_resize_and_font_changes(cx: &mut TestAppContext) {
    let window = chat(cx);
    let short = probe(window, cx, |view, window, cx| {
        let short = view.measure_row_height(answer(view), window, cx);
        view.patch(
            1,
            vec![row("assistant", &"More words in the answer. ".repeat(40))],
            cx,
        );
        view.motions.clear();
        let ix = answer(view);
        view.begin_measurements(window, cx);
        let before = view.measurements.fresh;
        let grown = view.measure_row_height(ix, window, cx);
        assert!(grown > short);
        assert_eq!(view.measurements.fresh, before + 1);
        short
    });
    let wide = probe(window, cx, |view, window, cx| {
        view.measure_row_height(answer(view), window, cx)
    });
    cx.simulate_window_resize(window.into(), size(px(400.), px(600.)));
    render(window, cx);
    render(window, cx);
    probe(window, cx, move |view, window, cx| {
        let ix = answer(view);
        let narrow = view.measure_row_height(ix, window, cx);
        assert!(narrow > wide);
        assert!(wide > short);
        window.set_rem_size(px(22.));
        view.begin_measurements(window, cx);
        let before = view.measurements.fresh;
        let _ = view.measure_row_height(ix, window, cx);
        assert_eq!(view.measurements.fresh, before + 1);
    });
}

#[gpui::test]
fn retained_row_measurements_keep_height_animation_live(cx: &mut TestAppContext) {
    let window = chat(cx);
    probe(window, cx, |view, window, cx| {
        let ix = answer(view);
        let full = view.measure_row_height(ix, window, cx);
        let now = Instant::now();
        view.motions
            .insert(view.rows[ix].key, Motion::new(true, false, now));
        view.frame_now = now + Duration::from_millis(30);
        view.begin_measurements(window, cx);
        let opening = view.measure_row_height(ix, window, cx);
        view.frame_now = now + Duration::from_millis(150);
        view.begin_measurements(window, cx);
        let later = view.measure_row_height(ix, window, cx);
        assert!(opening < later);
        assert!(later < full);
    });
}

#[gpui::test]
fn retained_row_measurements_refresh_when_closing_rows_leave(cx: &mut TestAppContext) {
    let window = chat(cx);
    probe(window, cx, |view, window, cx| {
        view.patch(
            1,
            vec![row(
                "assistant",
                &format!("Short.\n\n{}", "A wrapped second paragraph. ".repeat(25)),
            )],
            cx,
        );
        view.motions.clear();
        view.begin_measurements(window, cx);
        let indices: Vec<_> = view
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row.body, Body::Markdown(_)))
            .map(|(ix, _)| ix)
            .collect();
        assert_eq!(indices.len(), 2);
        let short = view.measure_row_height(indices[0], window, cx);
        let long = view.measure_row_height(indices[1], window, cx);
        assert!(long > short);
        view.motions.insert(
            view.rows[indices[0]].key,
            Motion::new(false, false, Instant::now() - Duration::from_secs(2)),
        );
        view.animate_frame(cx);
        view.begin_measurements(window, cx);
        let before = view.measurements.fresh;
        assert_eq!(view.measure_row_height(indices[0], window, cx), long);
        assert_eq!(view.measurements.fresh, before + 1);
    });
}

#[gpui::test]
fn streaming_tail_changes_reuse_prefix_rows_and_measurements(cx: &mut TestAppContext) {
    let window = chat(cx);
    probe(window, cx, |view, window, cx| {
        let mut text =
            "First paragraph.\n\nSecond paragraph.\n\nThird paragraph.\n\nTail".to_owned();
        view.patch(1, vec![row("assistant", &text)], cx);
        view.motions.clear();
        view.begin_measurements(window, cx);
        let first = answer(view);
        let key = view.rows[first].key;
        let Body::Markdown(original) = view.rows[first].body.clone() else {
            unreachable!()
        };
        let height = view.measure_row_height(first, window, cx);
        text.push_str(" grows while the prefix stays unchanged.");
        view.patch(1, vec![row("assistant", &text)], cx);
        view.motions.clear();
        view.begin_measurements(window, cx);
        let first = view.rows.iter().position(|row| row.key == key).unwrap();
        let Body::Markdown(current) = &view.rows[first].body else {
            unreachable!()
        };
        assert!(Arc::ptr_eq(&original, current));
        let before = view.measurements.fresh;
        assert_eq!(view.measure_row_height(first, window, cx), height);
        assert_eq!(
            view.measurements.fresh, before,
            "the stable prefix must not be laid out again"
        );
        assert!(
            view.reveals.is_empty(),
            "unanimated history must not retain reveal text or word timings"
        );
    });
}
