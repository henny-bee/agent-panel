use super::*;

impl TranscriptView {
    /// Shows queued messages as bubbles at the end of the chat.
    pub(crate) fn set_pending(&mut self, pending: Vec<PendingMessage>, cx: &mut Context<Self>) {
        if self.pending == pending {
            return;
        }
        let now = Instant::now();
        let animate = self.visible && !cx.reduce_motion();
        for old in &self.pending {
            if !pending.iter().any(|p| p.id == old.id) {
                self.taken.push(TakenMessage {
                    text: old.text.clone(),
                    removed_at: now,
                    entrance: self.pending_entrances.get(&old.id).copied(),
                });
            }
        }
        self.taken
            .retain(|message| now.duration_since(message.removed_at).as_secs_f32() < 2.);
        let added: Vec<_> = pending
            .iter()
            .filter(|p| !self.pending.iter().any(|o| o.id == p.id))
            .map(|p| p.id.clone())
            .collect();
        self.pending = pending;
        if !self.pending.iter().any(|message| {
            message.error.is_none() && Some(&message.id) == self.submitted_mono_message.as_ref()
        }) {
            self.submitted_mono_message = None;
        }
        if animate {
            for id in added {
                if let Some(ix) = self.pending.iter().position(|p| p.id == id) {
                    self.pending_entrances
                        .insert(self.pending[ix].id.clone(), now);
                }
            }
            self.follow.until = Some(now + std::time::Duration::from_millis(1200));
        }
        let from = self.source.iter().rposition(|r| r.is_prompt()).unwrap_or(0);
        if self.source.is_empty() {
            self.rows.clear();
            self.messages.clear();
            self.push_pending(0);
            self.list.reset(self.rows.len() + 1);
            if self.list.is_following_tail() || self.rows.len() <= self.pending.len() {
                self.list.scroll_to_end();
            }
        } else {
            self.rebuild(from, cx);
        }
        cx.notify();
    }

    pub(super) fn push_pending(&mut self, at: usize) {
        for i in 0..self.pending.len() {
            if Some(&self.pending[i].id) == self.submitted_mono_message.as_ref() {
                continue;
            }
            self.rows.push(UiRow {
                key: (at, 100_000 + i),
                body: Body::Pending(i),
                message: usize::MAX,
                first: false,
                last: false,
                work: false,
                work_tail: false,
                thought_phase: None,
            });
        }
    }

    /// A queued message's bubble, as UserMessageBlock draws one, with its
    /// delivery line when it could not be sent.
    pub(super) fn pending_row(&mut self, ix: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(message) = self.pending.get(ix).cloned() else {
            return div().into_any_element();
        };
        let stacked = self
            .pending
            .get(ix + 1)
            .is_some_and(|next| !dates::opens_stretch(next.sent_at, message.sent_at));
        let thinking = ix + 1 == self.pending.len()
            && !self.mono_pending_joins_live_turn()
            && self
                .pending
                .first()
                .is_some_and(|message| message.error.is_none());
        let (mut opacity, mut rise) = (1., 0.);
        if let Some(born) = self.pending_entrances.get(&message.id)
            && !cx.reduce_motion()
        {
            let t = crate::ui::motion::reveal::bezier(
                (born.elapsed().as_secs_f32() / 0.36).min(1.),
                0.22,
                1.,
                0.36,
                1.,
            );
            opacity = t;
            rise = 16. * (1. - t);
        }
        let retry = message.id.clone();
        let discard = message.id.clone();
        let attachments = attachments::Attachment::from_paths(&message.attachments);
        let attachment_body = (!attachments.is_empty())
            .then(|| self.attachment_row(&attachments, &format!("pending-{}", message.id), cx));
        div()
            .id(SharedString::from(format!("mono-pending-{}", message.id)))
            .debug_selector(|| "mono-pending-message".into())
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .opacity(opacity)
            .children(self.pending_separator(ix))
            .child(
                div()
                    .relative()
                    .top(px(rise))
                    .w_full()
                    .max_w(px(COLUMN))
                    .min_w_0()
                    .pl(px(56.))
                    .pr(px(COLUMN_PAD))
                    .pt_1()
                    .pb(px(if stacked { 0. } else { 20. }))
                    .flex()
                    .flex_col()
                    .items_end()
                    .text_size(px(14.))
                    .line_height(px(20.))
                    .child(
                        div()
                            .relative()
                            .max_w(px(576.))
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .px_3()
                            .py_2()
                            .child(
                                canvas(
                                    |_, _, _| {},
                                    move |bounds, _, window, _| {
                                        let radius = if bounds.size.height <= px(37.) {
                                            f32::from(bounds.size.height) * 0.5
                                        } else {
                                            12.
                                        };
                                        window.paint_quad(
                                            gpui::fill(bounds, user_bubble_fill())
                                                .corner_radii(px(radius)),
                                        );
                                    },
                                )
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full(),
                            )
                            .children(attachment_body.map(|body| {
                                div()
                                    .when(!message.text.is_empty(), |d| d.mb_2())
                                    .child(body)
                            }))
                            .when(!message.text.is_empty(), |d| d.child(message.text.clone())),
                    )
                    .when_some(message.error.clone(), |d, error| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_3()
                                .pt_1()
                                .text_size(px(12.))
                                .line_height(px(16.))
                                .text_color(content(0.5))
                                .child(
                                    div()
                                        .id(SharedString::from(format!(
                                            "mono-pending-error-{}",
                                            message.id
                                        )))
                                        .role(Role::Alert)
                                        .child(format!("Not sent · {error}")),
                                )
                                .child(
                                    div()
                                        .id(SharedString::from(format!(
                                            "mono-pending-retry-{retry}"
                                        )))
                                        .role(Role::Button)
                                        .rounded(px(4.))
                                        .px_1()
                                        .py(px(2.))
                                        .text_color(content(0.7))
                                        .hover(|s| s.bg(content(0.08)))
                                        .child("Retry")
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            cx.emit(MonoResponseAction::Queue {
                                                id: retry.clone(),
                                                remove: false,
                                            })
                                        })),
                                )
                                .child(
                                    div()
                                        .id(SharedString::from(format!(
                                            "mono-pending-discard-{discard}"
                                        )))
                                        .role(Role::Button)
                                        .rounded(px(4.))
                                        .px_1()
                                        .py(px(2.))
                                        .text_color(content(0.7))
                                        .hover(|s| s.bg(content(0.08)))
                                        .child("Discard")
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            cx.emit(MonoResponseAction::Queue {
                                                id: discard.clone(),
                                                remove: true,
                                            })
                                        })),
                                ),
                        )
                    }),
            )
            .when(thinking, |d| {
                d.child(
                    div()
                        .w_full()
                        .max_w(px(COLUMN))
                        .min_w_0()
                        .px(px(COLUMN_PAD))
                        .child(self.mono_header(self.source.len(), Some("Thinking…".into()), cx)),
                )
            })
            .into_any_element()
    }

    /// isCompactFollowUp: the next message is the user's too, directly or
    /// past a reply that was only a reaction, so it stacks onto this one.
    pub(super) fn stacks_on_next(&self, start: usize) -> bool {
        let mut at = start + 1;
        while at < self.source.len() && !self.source[at].first {
            at += 1;
        }
        if let Some(next) =
            (at..self.source.len()).find(|&i| self.source[i].first && self.source[i].is_prompt())
            && self.source[next].role == "user"
            && self.mono_next_run_continues(at, next)
        {
            return !self.mono_opens_stretch(next);
        }
        if let Some(message) = self
            .pending
            .iter()
            .find(|message| Some(&message.id) != self.submitted_mono_message.as_ref())
            && self.mono_pending_joins_live_turn()
            && self
                .source
                .iter()
                .rposition(Row::is_prompt)
                .is_some_and(|prompt| prompt < at)
            && !dates::opens_stretch(message.sent_at, self.source[start].sent_at)
        {
            return true;
        }
        while let Some(row) = self.source.get(at) {
            if row.is_prompt() {
                return !self.mono_opens_stretch(at);
            }
            let end = (at + 1..self.source.len())
                .find(|&i| self.source[i].first)
                .unwrap_or(self.source.len());
            let text: String = self.source[at..end]
                .iter()
                .map(|r| r.text.as_str())
                .collect();
            if !(row.role == "assistant" && text.trim().starts_with("<reaction>")) {
                return false;
            }
            at = end;
        }
        false
    }

    /// Motions a person started: these run at the display's full rate.
    pub(super) fn chat_animating(&self) -> bool {
        !self.entrances.is_empty()
            || !self.pending_entrances.is_empty()
            || self.reactions.values().any(|(_, born)| born.is_some())
            || self
                .hovered_turn
                .is_some_and(|(_, since)| since.elapsed().as_secs_f32() < HOVER_FADE)
    }

    /// Motion the stream drives: words revealing and fading in and the tail
    /// easing after growth. Only while one is under way, never for as long as
    /// the provider is busy: a stalled stream must not keep redrawing.
    pub(super) fn stream_animating(&self, now: Instant) -> bool {
        self.reveals.values().any(|r| r.active(now)) || self.follow.offset.abs() >= 0.25
    }
}
