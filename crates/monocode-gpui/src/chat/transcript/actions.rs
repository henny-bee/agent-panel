use super::*;

impl TranscriptView {
    /// The sent message's actions, under its bubble while it is hovered.
    pub(super) fn prompt_actions(
        &mut self,
        start: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let end = (start + 1..self.source.len())
            .find(|&i| self.source[i].first)
            .unwrap_or(self.source.len());
        let text: String = self.source[start..end]
            .iter()
            .map(|r| r.text.as_str())
            .collect();
        let copied = self.copied.is_some_and(|id| id == start as u64);
        let sent_at = self.source[start].sent_at;
        let base = self.base;
        let stamp_anchor = self
            .action_anchors
            .entry((start, "sent-at"))
            .or_default()
            .clone();
        div()
            .id(("prompt-actions", start))
            .absolute()
            .top_full()
            .right_0()
            .pt_1()
            .px_3()
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.hovered_prompt_actions = Some(start);
                } else if this.hovered_prompt_actions == Some(start) {
                    this.hovered_prompt_actions = None;
                }
                cx.notify();
            }))
            .flex()
            .items_center()
            .gap_1()
            .child(
                response_actions::button(
                    start,
                    "mono-copy-message",
                    if copied { "Copied" } else { "Copy message" },
                    if copied { "check" } else { "response-copy" },
                    false,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    this.copied = Some(start as u64);
                    cx.notify();
                })),
            )
            .when(
                !self.mono
                    && !self.side_question
                    && !self.streaming
                    && monocode_core::provider::supports_rewind(
                        monocode_core::provider::Kind::from_id(&self.provider),
                    )
                    && start >= base
                    && self
                        .source
                        .iter()
                        .rposition(|row| row.first && row.role == "user")
                        == Some(start),
                |d| {
                    d.child(
                        response_actions::button(
                            start,
                            "edit-last-message",
                            "Edit last message",
                            "pencil",
                            false,
                        )
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(ResponseRequest::EditLastTurn { turn: start - base })
                        })),
                    )
                },
            )
            .when_some(sent_at, |d, at| {
                let label = dates::full_stamp(at);
                d.child(
                    div()
                        .id(("message-sent-at", start))
                        .debug_selector(|| "message-sent-at".into())
                        .ml_1()
                        .text_size(px(12.))
                        .line_height(px(16.))
                        .text_color(content(0.4))
                        .aria_label(label.clone())
                        .child(stamp_anchor.probe())
                        .child(monocode_core::session::response::local_clock(at))
                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            this.action_hover(start, "sent-at", *hovered, cx)
                        })),
                )
                .when(self.hovered_action == Some((start, "sent-at")), |d| {
                    d.child(
                        popover(
                            ("message-time-tooltip", start),
                            Anchor::Slot(stamp_anchor.clone()),
                        )
                        .side(Side::Top)
                        .padding(8.)
                        .child(div().text_size(px(12.)).child(label)),
                    )
                })
            })
            .into_any_element()
    }

    /// The first row of the prompt block that opens `turn`.
    pub(super) fn prompt_start(&self, turn: usize) -> usize {
        prompt_start(&self.source, turn)
    }

    pub(super) fn action_hover(
        &mut self,
        turn: usize,
        action: &'static str,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        if hovered {
            self.hovered_action = Some((turn, action));
        } else if self.hovered_action == Some((turn, action)) {
            self.hovered_action = None;
        }
        cx.notify();
    }

    pub(super) fn response_actions(
        &mut self,
        turn: usize,
        complete: bool,
        before_user: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if !complete {
            // Reserve the action gap while the ticker works in the header.
            return div()
                .h(px(if self.mono {
                    if before_user { 24. } else { 28. }
                } else {
                    38.
                }))
                .into_any_element();
        }
        let (rows, _) = self.response_rows(turn);
        let output = monocode_core::session::response::prose(&rows);
        if self.mono {
            return self.mono_actions(turn, output, before_user, cx);
        }
        let copied = self.copied_response.is_some_and(|(id, _)| id == turn);
        let saved = self.note_feedback.get(&turn).is_some_and(|r| r.is_ok());
        let pending = self.note_pending.contains(&turn);
        let info = self.turns.get(&turn).cloned().unwrap_or_default();
        let metrics = info.metrics.filter(|m| m.reported());
        // Reserve the full icon strip before the sibling clock is laid out.
        // Intrinsic flex measurement otherwise reports only its widest button.
        let strip_width = if output.is_empty() { 14. } else { 44. }
            + if self.side_question { 0. } else { 52. }
            + if metrics.is_some() { 29. } else { 0. };
        let mut actions = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .flex_shrink_0()
            .w(px(strip_width));
        if !output.is_empty() {
            let copy_label = if copied { "Copied" } else { "Copy response" };
            let anchor = self
                .action_anchors
                .entry((turn, "copy"))
                .or_default()
                .clone();
            let text = output.clone();
            actions = actions.child(
                response_actions::button(
                    turn,
                    "response-copy",
                    copy_label,
                    if copied { "check" } else { "response-copy" },
                    false,
                )
                .ml(px(-4.))
                .child(anchor.probe())
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    this.action_hover(turn, "copy", *hovered, cx)
                }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    let born = Instant::now();
                    this.copied_response = Some((turn, born));
                    cx.notify();
                    cx.spawn(async move |this, cx| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_secs(2))
                            .await;
                        let _ = this.update(cx, |this, cx| {
                            if this.copied_response == Some((turn, born)) {
                                this.copied_response = None;
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                })),
            );
            let anchor = self
                .action_anchors
                .entry((turn, "note"))
                .or_default()
                .clone();
            actions = actions.child(
                response_actions::button(
                    turn,
                    "response-note",
                    if saved {
                        "Saved to Notes"
                    } else if pending {
                        "Saving note…"
                    } else {
                        "Save as note"
                    },
                    if saved { "check" } else { "response-note" },
                    false,
                )
                .when(pending, |d| d.opacity(0.4))
                .child(anchor.probe())
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    this.action_hover(turn, "note", *hovered, cx)
                }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.note_pending.insert(turn) {
                        this.note_feedback.remove(&turn);
                        cx.emit(ResponseRequest::SaveNote {
                            turn,
                            text: output.clone(),
                            selection: false,
                        });
                        cx.notify();
                    }
                })),
            );
        } else {
            actions = actions.child(icon("check", 14.).text_color(content(0.4)));
        }
        for (action, label, glyph, kind) in [
            (
                "handoff",
                "Handoff",
                "response-handoff",
                monocode_core::session::response::TransferKind::Handoff,
            ),
            (
                "opinion",
                "Second opinion",
                "response-opinion",
                monocode_core::session::response::TransferKind::SecondOpinion,
            ),
        ]
        .into_iter()
        .filter(|_| !self.side_question)
        {
            let anchor = self
                .action_anchors
                .entry((turn, action))
                .or_default()
                .clone();
            let click_anchor = anchor.clone();
            let active = self.target_action == Some((turn, kind));
            actions = actions.child(
                response_actions::button(turn, glyph, label, glyph, active)
                    .aria_expanded(active)
                    .child(anchor.probe())
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        this.action_hover(turn, action, *hovered, cx)
                    }))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(ResponseRequest::Target {
                            turn,
                            kind,
                            anchor: click_anchor.clone(),
                        })
                    })),
            );
        }
        if metrics.is_some() {
            let anchor = self
                .action_anchors
                .entry((turn, "metrics"))
                .or_default()
                .clone();
            actions = actions.child(
                response_actions::button(
                    turn,
                    "response-metrics",
                    "Turn metrics",
                    "response-metrics",
                    false,
                )
                .role(Role::Image)
                .ml(px(3.))
                .child(anchor.probe())
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    this.action_hover(turn, "metrics", *hovered, cx)
                })),
            );
        }
        let dot = || {
            div()
                .size(px(3.))
                .flex_shrink_0()
                .rounded_full()
                .bg(content(0.25))
        };
        let mut row = div()
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(10.))
            .pt(px(4.))
            .pb(px(12.))
            .text_size(px(12.))
            .line_height(px(16.))
            .text_color(content(0.4))
            .child(actions)
            .when_some(info.completed_at, |d, time| {
                d.child(dot()).child(
                    div()
                        .flex_shrink_0()
                        .text_color(content(0.35))
                        .child(monocode_core::session::response::local_clock(time)),
                )
            });
        if let Some(Err(error)) = self.note_feedback.get(&turn) {
            row = row.child(
                div()
                    .id(("note-save-error", turn))
                    .role(Role::Alert)
                    .text_size(px(11.))
                    .child(format!("Could not save note. {error}")),
            );
        }
        if let Some((hover_turn, action)) = self.hovered_action
            && hover_turn == turn
        {
            let (headline, detail) = match action {
                "copy" => (if copied { "Copied" } else { "Copy response" }.into(), None),
                "note" => (
                    if saved {
                        "Saved to Notes"
                    } else {
                        "Save as note"
                    }
                    .into(),
                    None,
                ),
                "handoff" => ("Handoff".into(), None),
                "opinion" => ("Second opinion".into(), None),
                _ => {
                    let metrics = metrics.unwrap_or_default();
                    let mut summary = Vec::new();
                    if let Some(hit) = metrics.cache_hit_percent {
                        summary.push(format!("Cache hit {hit:.0}%"));
                    }
                    if info.duration_ms > 0 && metrics.output_tokens > 0 {
                        summary.push(format!(
                            "Output {:.0} tok/s",
                            metrics.output_tokens as f64 / (info.duration_ms as f64 / 1000.)
                        ));
                    }
                    let headline = if summary.is_empty() {
                        "Turn tokens".into()
                    } else {
                        summary.join(" · ")
                    };
                    let detail = [
                        (metrics.input_tokens, "input"),
                        (metrics.output_tokens, "output"),
                        (metrics.cache_read_tokens, "cached"),
                    ]
                    .into_iter()
                    .filter(|(n, _)| *n > 0)
                    .map(|(n, label)| {
                        format!(
                            "{} {label}",
                            monocode_core::session::context::format_tokens(n)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" · ");
                    (headline, Some(detail))
                }
            };
            if let Some(anchor) = self.action_anchors.get(&(turn, action)) {
                row = row.child(
                    popover(("response-tooltip", turn), Anchor::Slot(anchor.clone()))
                        .side(Side::Top)
                        .max_width(if action == "metrics" { 300. } else { 240. })
                        .padding(8.)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .text_size(px(12.))
                                .line_height(px(16.))
                                .text_color(content(1.))
                                .child(headline)
                                .when_some(detail, |d, detail| {
                                    d.child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(content(0.5))
                                            .child(detail),
                                    )
                                }),
                        ),
                );
            }
        }
        row.into_any_element()
    }

    /// "Show more" opens a message in place, without a transition.
    pub(super) fn toggle_prompt(&mut self, start: usize, cx: &mut Context<Self>) {
        if !self.expanded_prompts.remove(&start) {
            self.expanded_prompts.insert(start);
        }
        self.list.pause_following_tail();
        self.rebuild(start, cx);
    }

    /// Wrapped lines of a user message's text at the bubble's text width.
    pub(super) fn prompt_lines(&self, text: &str, window: &Window) -> usize {
        let column = self.viewport.size.width.min(px(COLUMN));
        // Row padding (`pl-14` and `COLUMN_PAD` in the chat layout,
        // `COLUMN_PAD` both sides otherwise), the bubble's 36rem cap and its
        // `px-3`.
        let bubble = if self.chat_layout {
            (column - px(56. + COLUMN_PAD)).min(PROMPT_BUBBLE_MAX)
        } else {
            column - px(2. * COLUMN_PAD)
        };
        let text = SharedString::from(text.trim_end().to_owned());
        measurements::shape_prompt(text, Some(bubble - PROMPT_PAD_X), window)
            .map(|line| line.wrap_boundaries.len() + 1)
            .sum()
    }
}
