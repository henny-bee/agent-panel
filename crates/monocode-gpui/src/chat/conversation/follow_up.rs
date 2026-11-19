//! Composer.tsx / MessageQueue.tsx's durable follow-ups and saved user draft.
use super::*;
use monocode_core::session::follow_up::{Message, State};

#[derive(Default)]
pub(super) struct FollowUps {
    pub state: State,
    pub motion: super::composer_shelf::Motion,
    pub persisted: u64,
    pub sending: Option<String>,
    pub requested: Option<String>,
    pub continuations: std::collections::VecDeque<String>,
    pub resuming: bool,
    pub ready: bool,
    pub draft_selected: bool,
    pub edit: Option<QueueEdit>,
}
pub(super) struct QueueEdit {
    id: String,
    input: Entity<input::Input>,
    _submit: Subscription,
}

impl Conversation {
    pub(super) fn draft_active(&self, cx: &gpui::App) -> bool {
        self.follow_ups.draft_selected
            || monocode_core::session::composer::consume_command(
                &self.input.read(cx).content,
                "draft",
            )
            .is_some()
    }
    pub(super) fn message(
        &mut self,
        text: String,
        attachments: Vec<std::path::PathBuf>,
        mode: Option<String>,
        cx: &gpui::App,
    ) -> Message {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let prepared = self.completion.read(cx).prepare_prompt(&text);
        Message {
            prepared_prompt: (prepared != text).then_some(prepared),
            id: format!(
                "composer-{}-{}-{}",
                self.summary.id,
                monocode_core::sys::time::now_ms(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ),
            text,
            attachments,
            mode,
            ..Default::default()
        }
    }
    pub(super) fn save_follow_ups(&mut self, cx: &mut Context<Self>) {
        self.follow_ups.state.revision += 1;
        let mut snapshot = self.follow_ups.state.clone();
        let revision = snapshot.revision;
        let store = self.store.clone();
        let id = self.summary.id;
        let work = cx.background_executor().spawn(async move {
            let directory = locked(&store)?.data_dir().map(Path::to_path_buf);
            snapshot.retain_attachments(id, directory.as_deref())?;
            locked(&store)?.save_follow_ups(id, &snapshot)?;
            Ok::<_, String>(snapshot)
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(snapshot) => {
                        if this.follow_ups.state.revision == revision {
                            this.follow_ups.state = snapshot;
                            let draft = this.follow_ups.state.draft.clone();
                            this.transcript
                                .update(cx, |view, cx| view.set_saved_draft(draft, cx));
                        }
                        this.follow_ups.persisted = this.follow_ups.persisted.max(revision);
                        this.pump_follow_ups(cx);
                    }
                    Err(error) if this.follow_ups.state.revision == revision => {
                        this.follow_ups.state.paused = true;
                        this.status = format!("Could not save messages: {error}");
                        if let Some(head) = this.follow_ups.state.messages.first_mut() {
                            head.error = Some(this.status.clone());
                        }
                    }
                    Err(_) => {}
                }
                cx.emit(SessionChanged);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn enqueue_follow_up(
        &mut self,
        prompt: String,
        mode: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.enqueue_follow_up_inverted(prompt, mode, false, cx);
    }
    /// Whether a plain follow-up sent now steers the turn rather than queuing;
    /// `invert` flips the preference, as Cmd+Enter does.
    pub(super) fn steers_follow_up(&self, invert: bool, cx: &gpui::App) -> bool {
        let prefers_steer = cx
            .try_global::<crate::ui::preferences::Preferences>()
            .is_none_or(|prefs| prefs.follow_up == "steer");
        // Once the agent has answered and only background work is left, which
        // may never end (a dev server), queuing would park the message behind
        // it, so hand it to the agent now.
        (prefers_steer != invert || !self.background.tasks.is_empty())
            && Kind::from_id(&self.summary.provider).can_steer()
    }
    /// `invert` flips the follow-up preference for this message (Cmd+Enter).
    pub(super) fn enqueue_follow_up_inverted(
        &mut self,
        prompt: String,
        mode: Option<String>,
        invert: bool,
        cx: &mut Context<Self>,
    ) {
        let attachments = std::mem::take(&mut self.attachments);
        let mut message = self.message(prompt, attachments, mode, cx);
        message.steer = message.can_steer() && self.steers_follow_up(invert, cx);
        if invert && self.busy && message.steer {
            self.follow_ups.requested = Some(message.id.clone());
        }
        self.follow_ups.motion.insert(
            &message.id,
            self.follow_ups.state.messages.is_empty(),
            !cx.reduce_motion(),
        );
        self.follow_ups.state.messages.push(message);
        self.save_follow_ups(cx);
    }
    pub(super) fn save_message_draft(&mut self, cx: &mut Context<Self>) {
        if self.is_remote() {
            self.remote_save_draft(cx);
            return;
        }
        if self.needs_history_restore() {
            self.defer_history_action(|this, cx| this.save_message_draft(cx), cx);
            return;
        }
        if self.busy
            || self.follow_ups.state.draft.is_some()
            || !self.loaded
            || self.attaching
            || self.input.read(cx).is_composing()
        {
            return;
        }
        let raw = self.input.read(cx).content.to_string();
        let text = monocode_core::session::composer::consume_command(&raw, "draft")
            .unwrap_or(&raw)
            .to_owned();
        if text.trim().is_empty() && self.attachments.is_empty() {
            return;
        }
        self.input.update(cx, |i, cx| i.set_text(String::new(), cx));
        let attachments = std::mem::take(&mut self.attachments);
        let mut message = self.message(text, attachments, None, cx);
        message.at = self.transcript.read(cx).source_rows().len();
        if self.summary.title.starts_with("New ") {
            self.summary.title = message
                .label()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(42)
                .collect();
            let (store, id, title) = (
                self.store.clone(),
                self.summary.id,
                self.summary.title.clone(),
            );
            cx.background_executor()
                .spawn(async move {
                    let _ = locked(&store).and_then(|s| s.title(id, &title));
                })
                .detach();
        }
        if !self.summary.has_messages {
            self.composer_motion.capture(self.composer_anchor.get());
        }
        self.summary.has_messages = true;
        self.follow_ups.draft_selected = false;
        self.settings.plan = false;
        self.settings.operator = false;
        self.settings.orchestrator = false;
        self.save_settings(cx);
        self.follow_ups.state.draft = Some(message.clone());
        self.transcript
            .update(cx, |view, cx| view.introduce_saved_draft(message, cx));
        self.completion
            .update(cx, |completion, cx| completion.dismiss(cx));
        self.save_follow_ups(cx);
        cx.emit(SessionChanged);
    }
    pub(super) fn draft_action(&mut self, send: bool, cx: &mut Context<Self>) {
        if self.is_remote() {
            self.remote_draft_action(send, cx);
            return;
        }
        if self.needs_history_restore() {
            self.defer_history_action(move |this, cx| this.draft_action(send, cx), cx);
            return;
        }
        if send && (!self.loaded || self.preparing_worktree) {
            return;
        }
        let Some(mut message) = self.follow_ups.state.draft.take() else {
            return;
        };
        if send {
            message.steer = cx
                .try_global::<crate::ui::preferences::Preferences>()
                .is_none_or(|prefs| prefs.follow_up == "steer")
                && Kind::from_id(&self.summary.provider).can_steer();
            if !self.busy {
                self.follow_ups.requested = Some(message.id.clone());
            }
            self.follow_ups.motion.insert(
                &message.id,
                self.follow_ups.state.messages.is_empty(),
                !cx.reduce_motion(),
            );
            self.follow_ups.state.messages.push(message);
        }
        self.summary.has_messages = !self.transcript.read(cx).source_rows().is_empty() || send;
        self.transcript
            .update(cx, |view, cx| view.set_saved_draft(None, cx));
        self.save_follow_ups(cx);
        cx.emit(SessionChanged);
    }
    pub(super) fn follow_up_delivered(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.follow_ups.sending.as_deref() == Some(id) {
            self.follow_ups.sending = None;
        }
        if self.follow_ups.requested.as_deref() == Some(id) {
            self.follow_ups.requested = None;
        }
        self.follow_ups
            .continuations
            .retain(|message| message != id);
        if self.follow_ups.requested.is_none() {
            self.follow_ups.requested = self.follow_ups.continuations.front().cloned();
        }
        let before = self.follow_ups.state.messages.len();
        self.follow_ups
            .state
            .messages
            .retain(|message| message.id != id);
        if before != self.follow_ups.state.messages.len() {
            self.save_follow_ups(cx);
        }
    }
    pub(super) fn fail_follow_up(&mut self, id: &str, error: String, cx: &mut Context<Self>) {
        if let Some(message) = self
            .follow_ups
            .state
            .messages
            .iter_mut()
            .find(|m| m.id == id)
        {
            message.error = Some(error);
            self.follow_ups.state.paused = true;
            if self
                .follow_ups
                .continuations
                .iter()
                .any(|message| message == id)
            {
                self.follow_ups.continuations.clear();
            }
            if self.follow_ups.sending.as_deref() == Some(id) {
                self.follow_ups.sending = None;
            }
            if self.follow_ups.requested.as_deref() == Some(id) {
                self.follow_ups.requested = None;
            }
            self.save_follow_ups(cx);
        }
    }
    pub(super) fn pump_follow_ups(&mut self, cx: &mut Context<Self>) {
        if self.editing_last_turn.pending() || self.stopping {
            return;
        }
        if !self.history_ready() {
            // Persistence callbacks can arrive while a hidden history reloads.
            // The load completion pumps the durable queue after restoring rows.
            return;
        }
        if self.settings.usage_limit.is_some()
            || self.settings.worktree_removed
            || self.account_switching
            || self.settings.mono.is_some()
            || !self.loaded
            || self.preparing_worktree
            || (self.follow_ups.state.paused && self.follow_ups.requested.is_none())
            || self.follow_ups.sending.is_some()
            || self.follow_ups.resuming
            || self.follow_ups.persisted < self.follow_ups.state.revision
        {
            return;
        }
        let message = self
            .follow_ups
            .requested
            .as_ref()
            .and_then(|id| self.follow_ups.state.messages.iter().find(|m| &m.id == id))
            .or_else(|| {
                self.busy
                    .then(|| self.follow_ups.state.messages.iter().find(|m| m.steer))
                    .flatten()
            })
            .or_else(|| self.follow_ups.state.messages.first());
        let Some(message) = message.cloned() else {
            return;
        };
        if self
            .follow_ups
            .edit
            .as_ref()
            .is_some_and(|edit| edit.id == message.id)
        {
            return;
        }
        if self.busy {
            let requested = self.follow_ups.requested.as_deref() == Some(message.id.as_str());
            if self.settings.pending_provider_switch.is_some() {
                if requested {
                    self.stop(cx);
                    self.follow_ups.requested = Some(message.id.clone());
                    self.follow_ups.continuations.push_back(message.id);
                }
                return;
            }
            if !message.steer
                || !Kind::from_id(&self.summary.provider).can_steer()
                || !message.can_steer()
                || !self.follow_ups.ready
                || (!requested
                    && self
                        .questions
                        .iter()
                        .any(|(card, _)| card.read(cx).blocks_steering()))
            {
                return;
            }
            if let Some(controls) = &self.controls {
                if controls
                    .send(Control {
                        id: message.id.clone(),
                        decision: Decision::Steer(message.clone()),
                    })
                    .is_ok()
                {
                    self.follow_ups.sending = Some(message.id);
                } else {
                    self.fail_follow_up(
                        &message.id,
                        "The provider disconnected before accepting this message".into(),
                        cx,
                    );
                }
            }
        } else {
            if self.settings.new_worktree && self.settings.worktree_cwd.is_none() {
                self.prepare_worktree(cx);
                return;
            }
            self.follow_ups.sending = Some(message.id.clone());
            let message_id = message.id.clone();
            self.settings.mono_message_id = Some(message.id.clone());
            self.launch_message(message, cx);
            self.settings.mono_message_id = None;
            if !self.busy {
                self.fail_follow_up(&message_id, self.status.clone(), cx);
            }
        }
        cx.notify();
    }
    /// Starts `message` with its own one-shot mode, attachments and prompt.
    /// The user's next composer selection must not change a message already
    /// queued, and the message must not take the composer's.
    pub(super) fn launch_message(&mut self, message: Message, cx: &mut Context<Self>) {
        let previous = (
            self.settings.plan,
            self.settings.operator,
            self.settings.orchestrator,
        );
        self.settings.plan = message.mode.as_deref() == Some("plan");
        self.settings.operator = message.mode.as_deref() == Some("operator");
        self.settings.orchestrator = message.mode.as_deref() == Some("orchestrator");
        self.planning_orchestration = self.settings.orchestrator;
        self.orchestration_plan = None;
        self.orchestration_error = None;
        let composer_attachments = std::mem::take(&mut self.attachments);
        self.settings.attachments = message.attachments;
        let previous_prepared = self.settings.prepared_prompt.take();
        self.settings.prepared_prompt = message.prepared_prompt;
        self.launch_prompt(message.text, cx);
        self.settings.prepared_prompt = previous_prepared;
        self.attachments = composer_attachments;
        self.settings.attachments.clear();
        (
            self.settings.plan,
            self.settings.operator,
            self.settings.orchestrator,
        ) = previous;
    }
    fn resume_queue(&mut self, cx: &mut Context<Self>) {
        if self.stopping {
            self.defer_history_action(|this, cx| this.resume_queue(cx), cx);
            return;
        }
        if self.busy || !self.follow_ups.state.paused || self.follow_ups.state.messages.is_empty() {
            return;
        }
        // Resume the interrupted provider turn before releasing its outbox,
        // matching App.tsx's queueStatus = "resuming" continuation.
        self.follow_ups.resuming = true;
        self.launch_prompt("Continue from where you left off.".into(), cx);
        if !self.busy {
            self.follow_ups.resuming = false;
        }
        cx.notify();
    }
    fn edit_queued(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.follow_ups.sending.as_ref() == Some(&id) {
            return;
        }
        let Some(message) = self
            .follow_ups
            .state
            .messages
            .iter()
            .find(|message| message.id == id)
        else {
            return;
        };
        let text = message.text.clone();
        let input = cx.new(|cx| {
            let mut input = input::Input::queued_message(cx);
            input.set_text(text, cx);
            input
        });
        let submit = cx.subscribe_in(
            &input,
            window,
            |this, _, _: &input::Submitted, window, cx| this.finish_queue_edit(true, window, cx),
        );
        window.focus(&input.focus_handle(cx), cx);
        self.follow_ups.edit = Some(QueueEdit {
            id,
            input,
            _submit: submit,
        });
        cx.notify();
    }
    fn finish_queue_edit(&mut self, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = &self.follow_ups.edit else {
            return;
        };
        if edit.input.read(cx).is_composing() {
            return;
        }
        if save {
            let text = edit.input.read(cx).content.to_string();
            let prepared = self.completion.read(cx).prepare_prompt(&text);
            if let Some(message) = self
                .follow_ups
                .state
                .messages
                .iter_mut()
                .find(|m| m.id == edit.id)
            {
                if text.trim().is_empty() && message.attachments.is_empty() {
                    return;
                }
                message.replace_text(text, prepared);
                message.error = None;
            }
        }
        self.follow_ups.edit = None;
        window.focus(&self.input.focus_handle(cx), cx);
        if save {
            self.save_follow_ups(cx);
        } else {
            self.pump_follow_ups(cx);
        }
        cx.notify();
    }
    pub(super) fn message_queue(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let width = self.composer_anchor.get().map(|bounds| bounds.size.width);
        self.follow_ups.motion.retain(
            self.follow_ups
                .state
                .messages
                .iter()
                .map(|message| message.id.as_str()),
            !cx.reduce_motion() && width.is_some(),
        );
        if self.follow_ups.state.messages.is_empty() {
            return div().into_any_element();
        }
        let card_width = (width.unwrap_or_default() - px(16.)).max(px(0.));
        let row_width = (card_width - px(22.)).max(px(0.));
        let failed = self
            .follow_ups
            .state
            .messages
            .iter()
            .any(|m| m.error.is_some());
        let card = composer_shelf::card("message-queue-card", self.classic_layout)
            .when(self.follow_ups.state.paused && !self.follow_ups.resuming, |d| d.child(
                div().h(px(28.)).flex().items_center().gap_2().border_b_1().border_color(content(0.1))
                    .child(icon("queue-pause", 14.))
                    .child(div().flex_1().min_w_0().truncate().child(if failed { "A message couldn't be sent" } else { "Queue paused because you interrupted" }))
                    .child(queue_button("queue-resume", if failed { "Retry queue" } else { "Resume queue" }, "queue-play").w_auto().px(px(6.)).gap(px(6.)).child(if failed { "Retry" } else { "Resume" }).on_click(cx.listener(|this, _, _, cx| this.resume_queue(cx))))
            ))
            .children(self.follow_ups.state.messages.iter().enumerate().map(|(index, message)| {
                let id = message.id.clone();
                let editing = self.follow_ups.edit.as_ref().filter(|edit| edit.id == id);
                let sending = self.follow_ups.sending.as_ref() == Some(&id);
                div().id(SharedString::from(format!("queued-{id}"))).role(Role::Group).aria_label(format!("Queued message: {}", message.label())).min_h(px(32.)).flex().items_center().gap(px(10.)).when(index > 0, |d| d.border_t_1().border_color(content(0.05)))
                    .child(icon("queue-list", 14.).text_color(content(0.35)))
                    .map(|row| if let Some(edit) = editing {
                        row.child(div().flex_1().min_w_0().px(px(6.)).py(px(2.)).rounded_md().border_1().border_color(content(0.3)).bg(content(0.05)).on_action(cx.listener(|this, _: &crate::ui::popover::DismissMenu, window, cx| { this.finish_queue_edit(false, window, cx); cx.stop_propagation(); })).child(edit.input.clone()))
                            .child(queue_button("queue-save", "Save queued message", "check").on_click(cx.listener(|this, _, window, cx| this.finish_queue_edit(true, window, cx))))
                            .child(queue_button("queue-cancel", "Cancel queued message edit", "close").on_click(cx.listener(|this, _, window, cx| this.finish_queue_edit(false, window, cx))))
                    } else {
                        let (steer, edit, remove) = (id.clone(), id.clone(), id);
                        row.child(div().flex_1().min_w_0().py_1().child(div().id("queued-message-text").role(Role::Label).aria_label(message.label()).truncate().text_color(content(0.75)).child(message.label())).when_some(message.error.clone(), |d, error| d.child(div().id("queue-error").role(Role::Alert).text_color(gpui::rgb(0xf87171)).child(error))))
                            .when(Kind::from_id(&self.summary.provider).can_steer(), |d| d.child(queue_button("queue-steer", "Steer queued message", "queue-steer").w_auto().px_2().gap(px(6.)).opacity(if sending { 0.4 } else { 1. }).child("Steer").on_click(cx.listener(move |this, _, _, cx| {
                                if !sending && let Some(message) = this.follow_ups.state.messages.iter_mut().find(|m| m.id == steer) {
                                    if !message.can_steer() && this.busy { return; }
                                    message.steer = true;
                                    message.error = None;
                                    this.follow_ups.requested = Some(steer.clone());
                                    this.save_follow_ups(cx);
                                }
                            }))))
                            .child(queue_button("queue-edit", "Edit queued message", "pencil-edit").opacity(if sending { 0.4 } else { 1. }).on_click(cx.listener(move |this, _, window, cx| { if !sending { this.edit_queued(edit.clone(), window, cx); } })))
                            .child(queue_button("queue-remove", "Remove queued message", "trash").opacity(if sending { 0.4 } else { 1. }).on_click(cx.listener(move |this, _, _, cx| {
                                if !sending {
                                    this.follow_ups.state.messages.retain(|m| m.id != remove);
                                    this.follow_ups.continuations.retain(|id| id != &remove);
                                    if this.follow_ups.requested.as_ref() == Some(&remove) { this.follow_ups.requested = this.follow_ups.continuations.front().cloned(); }
                                    this.save_follow_ups(cx);
                                }
                            })))
                    })
                    .map(|row| self.follow_ups.motion.row(&message.id, row, row_width, window))
            }));
        composer_shelf::shelf("message-queue")
            .child(self.follow_ups.motion.card(card, card_width, window))
            .into_any_element()
    }
}

fn queue_button(
    id: &'static str,
    label: &'static str,
    glyph: &'static str,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .size(px(24.))
        .flex_shrink_0()
        .rounded_md()
        .flex()
        .items_center()
        .justify_center()
        .text_color(content(0.5))
        .hover(|s| s.bg(content(0.08)).text_color(content(0.9)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(icon(glyph, 14.))
}

#[cfg(test)]
mod tests;
