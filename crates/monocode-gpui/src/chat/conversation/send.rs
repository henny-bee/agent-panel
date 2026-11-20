use super::*;

impl Conversation {
    /// The quick composer's files, sent with its prompt.
    pub(crate) fn quick_attach(&mut self, paths: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        if !paths.is_empty() {
            self.add_attachments(paths, cx);
        }
    }

    pub(crate) fn quick_submit(
        &mut self,
        prompt: String,
        settings: Settings,
        cx: &mut Context<Self>,
    ) {
        if self.editing_last_turn.pending() {
            self.defer_edit_transfer(settings, Some(prompt), cx);
            return;
        }
        if self.needs_history_restore() || self.stopping {
            self.defer_history_action(
                move |this, cx| {
                    if this.preserve_new_composer(cx) {
                        this.install_transfer(settings, None, cx);
                        this.enqueue_restored_prompt(prompt, cx);
                    } else {
                        this.quick_submit(prompt, settings, cx);
                    }
                },
                cx,
            );
            return;
        }
        self.loaded = true;
        self.model = settings.model.clone();
        self.settings = settings;
        self.input
            .update(cx, |input, cx| input.set_text(prompt, cx));
        self.send(cx);
    }

    pub(super) fn send(&mut self, cx: &mut Context<Self>) {
        self.send_follow_up(false, cx);
    }

    /// `invert` sends a mid-turn message the opposite way from the follow-up
    /// preference: queued when it steers, steering when it queues.
    pub(super) fn send_follow_up(&mut self, invert: bool, cx: &mut Context<Self>) {
        if self.is_remote() && self.remote_send_guard(cx) {
            return;
        }
        if self.needs_history_restore() {
            self.defer_history_action(move |this, cx| this.send_follow_up(invert, cx), cx);
            return;
        }
        if self.submit_edited(cx) {
            return;
        }
        if self.handle_local_command(cx) {
            return;
        }
        if self.settings.mono.is_some() {
            if self.attaching || self.input.read(cx).is_composing() {
                return;
            }
            let text = self
                .input
                .update(cx, |input, cx| input.take(cx))
                .unwrap_or_default();
            let attachments = std::mem::take(&mut self.attachments);
            if !text.trim().is_empty() || !attachments.is_empty() {
                cx.emit(mono::Submitted { text, attachments });
                cx.notify();
            }
            return;
        }
        if self.settings.worktree_removed {
            self.status = "Select a branch or worktree to continue this conversation.".into();
            cx.notify();
            return;
        }
        if !self.loaded || (self.preparing_worktree && !self.busy && !self.stopping) {
            return;
        }
        if self.draft_active(cx) {
            self.save_message_draft(cx);
            return;
        }
        if !self.folder_selected
            && monocode_core::session::composer::consume_command(
                &self.input.read(cx).content,
                "add-to-folder",
            )
            .is_some()
        {
            self.request_folder(cx);
            return;
        }
        if self.attaching
            || (!self.input.read(cx).has_message_text() && self.attachments.is_empty())
        {
            return;
        }
        if self.input.read(cx).is_composing() {
            return;
        }
        let raw = self
            .input
            .update(cx, |i, cx| i.take(cx))
            .unwrap_or_default();
        let raw = monocode_core::session::composer::consume_command(&raw, "add-to-folder")
            .map(str::to_owned)
            .unwrap_or(raw);
        self.folder_selected = false;
        let prompt = self.consume_mode_command(raw, cx);
        if prompt.trim().is_empty() && self.attachments.is_empty() {
            return;
        }
        self.completion
            .update(cx, |completion, cx| completion.dismiss(cx));
        let mode = if self.settings.orchestrator {
            Some("orchestrator")
        } else if self.settings.plan {
            Some("plan")
        } else if self.settings.operator {
            Some("operator")
        } else {
            None
        };
        if self.settings.pending_provider_switch.is_some() && self.busy {
            // Picking a provider only arms the change. Sending is what stops
            // the outgoing provider and commits the handoff.
            self.stop(cx);
        }
        if self.sends_as_follow_up() {
            self.enqueue_follow_up_inverted(prompt, mode.map(str::to_owned), invert, cx);
            if self.stopping || self.settings.pending_provider_switch.is_some() {
                // A fresh Send continues the conversation, while older queued
                // messages remain paused after Stop.
                if let Some(id) = self
                    .follow_ups
                    .state
                    .messages
                    .last()
                    .map(|message| message.id.clone())
                {
                    self.follow_ups.continuations.push_back(id);
                    if self.follow_ups.requested.is_none() {
                        self.follow_ups.requested = self.follow_ups.continuations.front().cloned();
                    }
                }
            }
            if mode.is_some() {
                self.clear_submitted_mode(cx);
            }
            return;
        }
        self.planning_orchestration = self.settings.orchestrator;
        self.orchestration_plan = None;
        self.orchestration_error = None;
        self.launch_prompt(prompt, cx);
        if self.busy && mode.is_some() {
            self.clear_submitted_mode(cx);
        }
    }

    /// A message sent now joins the follow-ups instead of starting a turn.
    pub(super) fn sends_as_follow_up(&self) -> bool {
        self.stopping
            || self.busy
            || self.settings.pending_provider_switch.is_some()
            || (!self.follow_ups.state.messages.is_empty() && !self.follow_ups.state.paused)
    }

    /// The submit icon for a message that will steer or queue, if it will.
    pub(super) fn follow_up_action(
        &self,
        alternate: bool,
        cx: &gpui::App,
    ) -> Option<(&'static str, &'static str)> {
        // Stop and provider switches send the message as the next turn.
        if self.settings.mono.is_some()
            || self.is_remote()
            || self.stopping
            || self.settings.pending_provider_switch.is_some()
            || !self.sends_as_follow_up()
        {
            return None;
        }
        let mode = self.settings.orchestrator || self.settings.plan || self.settings.operator;
        Some(
            if self.busy && !mode && self.steers_follow_up(alternate, cx) {
                ("Steer", "queue-steer")
            } else {
                ("Queue", "queue-list")
            },
        )
    }

    pub(super) fn launch_prompt(&mut self, prompt: String, cx: &mut Context<Self>) {
        if self.stopping {
            let mode = if self.settings.orchestrator {
                Some("orchestrator")
            } else if self.settings.plan {
                Some("plan")
            } else if self.settings.operator {
                Some("operator")
            } else {
                None
            };
            self.enqueue_follow_up(prompt, mode.map(str::to_owned), cx);
            if let Some(id) = self
                .follow_ups
                .state
                .messages
                .last()
                .map(|message| message.id.clone())
            {
                self.follow_ups.continuations.push_back(id);
                if self.follow_ups.requested.is_none() {
                    self.follow_ups.requested = self.follow_ups.continuations.front().cloned();
                }
            }
            return;
        }
        if !self.history_ready() {
            self.defer_history_action(move |this, cx| this.launch_prompt(prompt, cx), cx);
            return;
        }
        if self.account_switching {
            return;
        }
        let kind = Kind::from_id(&self.summary.provider);
        if self.settings.account.id != monocode_core::session::profile::DEFAULT
            && crate::accounts::get(cx)
                .is_some_and(|center| !center.read(cx).book.exists(kind, &self.settings.account.id))
        {
            self.account_unavailable(cx);
            return;
        }
        self.settings.usage_limit = None;
        self.attach_account(cx);
        let worktree_message = if prompt.trim().is_empty() {
            self.settings
                .attachments
                .iter()
                .chain(&self.attachments)
                .filter_map(|path| path.file_name())
                .map(|name| name.to_string_lossy())
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            prompt.clone()
        };
        if !self.is_remote() {
            if !self.summary.has_messages {
                self.generate_worktree_branch(&worktree_message, cx);
            }
            self.generate_title(&prompt, cx);
        }
        let model_label = self.model_label();
        self.sync_tool_paths(cx);
        self.transcript
            .update(cx, |v, _| v.identity(&self.summary.provider, &model_label));
        self.begin_turn(cx);
        let generation = self.submission_generation;
        let source_provider = self.summary.provider.clone();
        let source_account = self.settings.account.id.clone();
        self.status = if self.settings.pending_provider_switch.is_some() {
            "Preparing handoff…"
        } else {
            "Connecting…"
        }
        .into();
        self.cancel = Arc::new(AtomicBool::new(false));
        let summary = self.summary.clone();
        let store = self.store.clone();
        let cancel = self.cancel.clone();
        // Composer modes reset after Send; the thread grant and running turn's
        // access policy must outlive that reset and edits to the next draft.
        self.settings.operator_enabled |= self.settings.operator;
        self.app_read_only = self.settings.plan
            || self.settings.orchestrator
            || self.settings.access == AccessMode::ReadOnly;
        let mut settings = self.settings.clone();
        if let Some(model) = self.model_info() {
            for control in &model.controls {
                settings
                    .model_settings
                    .entry(control.id.clone())
                    .or_insert_with(|| control.current.clone());
            }
        }
        settings.turn_model = Some(monocode_core::session::agent::TurnInfo {
            provider: self.summary.provider.clone(),
            model: model_label.to_string(),
            model_id: self
                .model_info()
                .map(|model| model.id.clone())
                .or(self.model.clone()),
            started_at: Some(monocode_core::sys::time::now_ms()),
            ..Default::default()
        });
        if !self.is_remote() && settings.prepared_prompt.is_none() {
            let prepared = self.completion.read(cx).prepare_prompt(&prompt);
            if prepared != prompt {
                settings.prepared_prompt = Some(prepared);
            }
        }
        settings.disable_claude_hooks = cx
            .try_global::<crate::ui::preferences::Preferences>()
            .is_some_and(|p| !p.claude_hooks);
        settings.model = self.model.clone();
        settings.environment.extend(self.app_environment.clone());
        if settings.remote_action.as_ref().is_none_or(|action| {
            (action["type"] == "send"
                && action["draftBlockId"].is_null()
                && action["planBlockId"].is_null())
                || action["type"] == "draft"
        }) {
            settings
                .attachments
                .extend(std::mem::take(&mut self.attachments));
        }
        if settings
            .remote_action
            .as_ref()
            .is_some_and(|action| action["planBlockId"].is_string())
        {
            settings.attachments.clear();
        }
        self.preparing_worktree = settings.new_worktree && settings.worktree_cwd.is_none();
        if self.preparing_worktree {
            settings.worktree_base = settings.worktree_base.or(self.branch.clone());
            self.status = "Creating worktree…".into();
        }
        if settings.mono.is_none()
            && settings
                .remote_action
                .as_ref()
                .is_none_or(|action| action["type"] == "send")
        {
            self.transcript
                .update(cx, |view, cx| view.submit_prompt(&prompt, &settings, cx));
        }
        if settings.orchestrator {
            settings.plan = true;
            settings.operator = false;
            self.settings.orchestrator = false;
        }
        if self.settings.handoff.take().is_some() {
            self.save_settings(cx);
        }
        let (controls, control_rx) = std::sync::mpsc::channel();
        self.controls = Some(controls);
        self.approvals.clear();
        self.questions.clear();
        let (tx, rx) = async_channel::bounded(8);
        let mut turn_model_id = settings
            .turn_model
            .as_ref()
            .and_then(|turn| turn.model_id.clone())
            .or_else(|| settings.model.clone());
        let remote_client = self
            .is_remote()
            .then(|| self.remote_state.client.clone())
            .flatten();
        cx.background_executor()
            .spawn(async move {
                if let Some(client) = remote_client {
                    crate::runtime::remote::run_with_client(
                        client, summary, prompt, settings, control_rx, store, cancel, tx,
                    );
                } else {
                    run_turn(summary, prompt, settings, control_rx, store, cancel, tx);
                }
            })
            .detach();
        cx.spawn(async move |this, cx| {
            while let Ok(update) = rx.recv().await {
                // Land each update on the spinners' 30Hz beat: the reply then
                // redraws in their frames instead of adding frames of its own.
                if !update.finished {
                    crate::ui::motion::pulse::beat(cx).await;
                }
                let finished = update.finished;
                if this
                    .update(cx, |this, cx| {
                        if this.submission_generation != generation {
                            return;
                        }
                        let stopping = this.stopping;
                        // The final update settles accepted rows and storage.
                        // Ignore streaming notifications after the user stops.
                        if stopping && !finished {
                            return;
                        }
                        let source_selected = this.summary.provider == source_provider
                            && this.settings.account.id == source_account;
                        if let Some(cwd) = update.worktree_cwd
                            && (this.settings.worktree_cwd.as_ref() != Some(&cwd)
                                || this.settings.new_worktree)
                        {
                            this.preparing_worktree = false;
                            this.settings.worktree_cwd = Some(cwd);
                            this.settings.new_worktree = false;
                            this.save_settings(cx);
                            this.generate_worktree_branch(&worktree_message, cx);
                            this.refresh_git(cx);
                        }
                        // The host renamed the conversation; it is not sent back.
                        if let Some(title) = update.title
                            && this.summary.title != title
                        {
                            this.summary.title = title;
                            cx.emit(SessionChanged);
                        }
                        if let Some(commands) = update.commands.filter(|_| source_selected) {
                            this.completion.update(cx, |completion, cx| {
                                completion.set_native_commands(commands, cx)
                            });
                        }
                        let needed_approval = this.needs_approval();
                        let incoming_notification_input = !stopping
                            && (!update.approvals.is_empty() || !update.questions.is_empty());
                        let mono_queue_changed = this.settings.mono.is_some()
                            && (update.turn_ready
                                || update.delivered.is_some()
                                || !update.follow_up_delivered.is_empty()
                                || !update.follow_up_errors.is_empty());
                        this.transcript.update(cx, |v, cx| {
                            v.patch(update.start, update.rows, cx);
                            if stopping {
                                v.stop_streaming(cx);
                            }
                        });
                        if let Some(id) = update.provider_turn_id {
                            this.transcript
                                .update(cx, |view, _| view.record_provider_turn(id));
                        }
                        if let Some(id) = update.provider_id {
                            if let Some(pending) = this.settings.pending_provider_switch.as_mut()
                                && pending.from == source_provider
                                && pending.account.id == source_account
                            {
                                pending.provider_id = Some(id);
                            } else if source_selected {
                                this.summary.provider_id = Some(id);
                            }
                        }
                        if let Some(handoff) = update.handoff {
                            this.settings.pending_provider_switch = None;
                            this.settings.handoff = handoff;
                            this.save_settings(cx);
                        }
                        if let Some(model) = update.config_model {
                            if let Some(pending) = this.settings.pending_provider_switch.as_mut()
                                && pending.from == source_provider
                                && pending.account.id == source_account
                            {
                                pending.model = Some(model);
                            } else if source_selected {
                                this.model = Some(model.clone());
                                this.settings.model = Some(model);
                            }
                        }
                        if let Some(settings) = update.config_settings {
                            if let Some(pending) = this.settings.pending_provider_switch.as_mut()
                                && pending.from == source_provider
                                && pending.account.id == source_account
                            {
                                pending.model_settings.extend(settings);
                            } else if source_selected {
                                this.settings.model_settings.extend(settings);
                            }
                            this.save_settings(cx);
                        }
                        if let Some(model) = update.model {
                            turn_model_id = Some(model.clone());
                            let name: SharedString = this
                                .catalog
                                .iter()
                                .find(|entry| source_selected && entry.id == model)
                                .map(|entry| {
                                    let name = model_display_name(
                                        Kind::from_id(&source_provider),
                                        &entry.id,
                                        &entry.name,
                                    );
                                    name.strip_prefix("Claude ")
                                        .unwrap_or(&name)
                                        .to_owned()
                                        .into()
                                })
                                .unwrap_or_else(|| {
                                    model_name(Kind::from_id(&source_provider), Some(&model))
                                });
                            if source_selected {
                                this.resolved_model = Some(model);
                            }
                            this.transcript
                                .update(cx, |v, _| v.identity(&source_provider, &name));
                        }
                        if let Some(context) = update.context.filter(|_| source_selected) {
                            this.context = Some(context);
                        }
                        if let Some(metrics) = update.metrics {
                            this.transcript.update(cx, |v, _| v.metrics(metrics));
                        }
                        if update.turn_ready && !stopping {
                            this.follow_ups.ready = true;
                        }
                        for id in update.follow_up_delivered {
                            if this.settings.mono.is_some() {
                                this.mono_message_delivered(id, cx);
                            } else {
                                this.follow_up_delivered(&id, cx);
                            }
                        }
                        for (id, error) in update.follow_up_errors {
                            if this.settings.mono.is_some() {
                                this.mono_message_failed(id, error);
                            } else {
                                this.fail_follow_up(&id, error, cx);
                            }
                        }
                        if let Some(id) = update.delivered {
                            if this.settings.mono.is_some() {
                                this.mono_message_delivered(id, cx);
                            } else {
                                this.follow_up_delivered(&id, cx);
                            }
                        }
                        if !stopping || finished {
                            this.status = update.status;
                        }
                        if let Some(tasks) = update.background.filter(|_| !stopping) {
                            this.set_background(tasks, cx);
                        }
                        if !stopping {
                            this.approvals.extend(update.approvals);
                        }
                        for id in update.approvals_resolved {
                            this.approvals.retain(|approval| approval.id != id);
                        }
                        for id in update.questions_resolved {
                            this.questions.retain(|(card, _)| card.read(cx).id() != id);
                        }
                        for request in update.questions.into_iter().filter(|_| !stopping) {
                            let card =
                                cx.new(|cx| crate::chat::questions::QuestionCard::new(request, cx));
                            let events = cx.subscribe(
                                &card,
                                |this, _, reply: &crate::chat::questions::Answered, cx| {
                                    if this
                                        .questions
                                        .front()
                                        .is_none_or(|(card, _)| card.read(cx).id() != reply.id)
                                    {
                                        return;
                                    }
                                    let before = this.needs_approval();
                                    let decision = reply
                                        .answers
                                        .clone()
                                        .map_or(Decision::Deny, Decision::Answer);
                                    if this.controls.as_ref().is_none_or(|controls| {
                                        controls
                                            .send(Control {
                                                id: reply.id.clone(),
                                                decision,
                                            })
                                            .is_err()
                                    }) {
                                        this.status =
                                            "The provider is no longer waiting for this answer"
                                                .into();
                                        cx.notify();
                                        return;
                                    }
                                    if let Some((card, _)) = this.questions.pop_front() {
                                        this.question_overlay.release(card, cx);
                                    }
                                    this.status = if !this.approvals.is_empty() {
                                        "Waiting for approval"
                                    } else if !this.questions.is_empty() {
                                        "Waiting for your answer"
                                    } else {
                                        "Working…"
                                    }
                                    .into();
                                    if before != this.needs_approval() {
                                        cx.emit(SessionChanged);
                                    }
                                    cx.notify();
                                },
                            );
                            let keep_open = cx.subscribe(
                                &card,
                                |this, _, request: &crate::chat::questions::KeptOpen, cx| {
                                    if let Some(controls) = &this.controls {
                                        let _ = controls.send(Control {
                                            id: request.id.clone(),
                                            decision: Decision::KeepOpen,
                                        });
                                    }
                                    cx.emit(SessionChanged);
                                    cx.notify();
                                },
                            );
                            this.questions.push_back((card, vec![events, keep_open]));
                            this.status = "Waiting for your answer".into();
                        }
                        if !finished && !this.approvals.is_empty() {
                            this.status = "Waiting for approval".into();
                        } else if !finished && !this.questions.is_empty() {
                            this.status = "Waiting for your answer".into();
                        }
                        if finished {
                            if this.is_remote() {
                                this.settings.remote_action = None;
                                this.remote_state.refreshing = false;
                            }
                            this.summary.updated_at = monocode_core::sys::time::now_ms();
                            this.busy = false;
                            this.stopping = false;
                            this.clear_background();
                            if this.is_remote() {
                                this.refresh_remote_snapshot(cx);
                            }
                            this.preparing_worktree = false;
                            if !stopping && let Some(limit) = update.usage_limit {
                                this.settings.usage_limit = Some(limit);
                            }
                            if !this.is_remote() {
                                this.detect_usage_limit(cx);
                            }
                            this.review
                                .update(cx, |review, cx| review.set_busy(false, cx));
                            this.follow_ups.ready = false;
                            if this.follow_ups.resuming {
                                this.follow_ups.resuming = false;
                                if !stopping && this.status == "Ready" {
                                    this.follow_ups.state.paused = false;
                                    for message in &mut this.follow_ups.state.messages {
                                        message.error = None;
                                    }
                                    this.save_follow_ups(cx);
                                }
                            }
                            if let Some(id) = this.follow_ups.sending.clone() {
                                this.fail_follow_up(
                                    &id,
                                    "The turn ended before this message was accepted".into(),
                                    cx,
                                );
                            }
                            if this.status != "Ready" && !this.follow_ups.state.messages.is_empty()
                            {
                                this.follow_ups.state.paused = true;
                                this.save_follow_ups(cx);
                            }
                            this.approvals.clear();
                            this.questions.clear();
                            this.controls = None;
                            this.transcript.update(cx, |v, cx| v.streaming(false, cx));
                            let model_id = turn_model_id.clone();
                            this.transcript.update(cx, |view, _| {
                                if let Some(turn) = view.active_turn()
                                    && let Some(info) = view.turns.get_mut(&turn)
                                {
                                    info.model_id = model_id;
                                }
                            });
                            if stopping {
                                this.planning_orchestration = false;
                                this.orchestration_progress = None;
                            } else {
                                this.finish_orchestration_plan(cx);
                            }
                            this.settings.turns = this.transcript.read(cx).window_turns();
                            this.save_settings(cx);
                            this.refresh_git(cx);
                            this.run_history_actions(cx);
                            this.mono_flush_external(cx);
                            cx.emit(SessionChanged);
                        } else if incoming_notification_input
                            || mono_queue_changed
                            || needed_approval != this.needs_approval()
                        {
                            cx.emit(SessionChanged);
                        }
                        this.pump_follow_ups(cx);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.emit(SessionChanged);
        cx.notify();
    }

    pub(super) fn begin_turn(&mut self, cx: &mut Context<Self>) {
        self.submission_generation = self.submission_generation.wrapping_add(1);
        if !self.summary.has_messages {
            self.composer_motion.capture(self.composer_anchor.get());
        }
        self.busy = true;
        self.stopping = false;
        self.notification_cancelled = false;
        self.review
            .update(cx, |review, cx| review.set_busy(true, cx));
        self.follow_ups.ready = false;
        self.mono_turn = self
            .mono_turn
            .wrapping_add(1)
            .max(monocode_core::sys::time::now_ms() & 0xffff_ffff);
        self.summary.updated_at = monocode_core::sys::time::now_ms();
        self.transcript.update(cx, |v, cx| v.streaming(true, cx));
        self.summary.has_messages = true;
    }
}

impl Conversation {
    pub(crate) fn install_transfer(
        &mut self,
        settings: Settings,
        prompt: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.editing_last_turn.pending() {
            self.defer_edit_transfer(settings, prompt, cx);
            return;
        }
        if self.needs_history_restore() || self.stopping {
            self.defer_history_action(
                move |this, cx| {
                    // Mono callers may configure their live context immediately
                    // after installing a transfer. Keep that context at replay.
                    let mono = this.settings.mono.take();
                    let environment = std::mem::take(&mut this.settings.environment);
                    let mut settings = settings;
                    if settings.mono.is_none() {
                        settings.mono = mono;
                    }
                    settings.environment.extend(environment);
                    match prompt {
                        Some(prompt) if this.preserve_new_composer(cx) => {
                            this.install_transfer(settings, None, cx);
                            this.enqueue_restored_prompt(prompt, cx);
                        }
                        prompt => this.install_transfer(settings, prompt, cx),
                    }
                },
                cx,
            );
            return;
        }
        self.model = settings.model.clone();
        self.settings = settings;
        self.attach_account(cx);
        self.loaded = true;
        self.status = "Ready".into();
        self.refresh_catalog(cx);
        self.refresh_git(cx);
        if let Some(prompt) = prompt {
            self.input.update(cx, |i, cx| i.set_text(prompt, cx));
            self.send(cx);
        } else {
            cx.notify();
        }
    }

    pub(super) fn close_response_menu(&mut self, cx: &mut Context<Self>) {
        self.response_menu = None;
        self.response_request = None;
        self.transcript.update(cx, |view, cx| {
            view.target_action = None;
            cx.notify();
        });
        cx.notify();
    }

    pub(super) fn response_request(
        &mut self,
        request: &ResponseRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match request {
            ResponseRequest::RemotePlan { id, text, build } => {
                self.remote_plan_request(id, text, *build, cx)
            }
            ResponseRequest::QuoteSelection { text } => self.mono_quote(text, window, cx),
            ResponseRequest::Cut { turn, cut } => self.begin_cut(*turn, *cut, window, cx),
            ResponseRequest::Draft { send } => self.draft_action(*send, cx),
            ResponseRequest::SaveNote {
                turn,
                text,
                selection,
            } => {
                let turn = *turn;
                let note = monocode_core::session::response::Note {
                    id: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos() as u64,
                    title: {
                        let title = self.display_title();
                        if *selection || title.trim().is_empty() || title == "New session" {
                            monocode_core::session::notes::title(text)
                        } else {
                            title
                        }
                    },
                    body: text.clone(),
                    tags: Vec::new(),
                    source_session: self.summary.id,
                    source_cwd: self.summary.cwd.clone(),
                };
                let store = self.store.clone();
                let work = cx
                    .background_executor()
                    .spawn(async move { locked(&store)?.save_note(&note) });
                cx.spawn(async move |this, cx| {
                    let result = work.await;
                    let _ = this.update(cx, |this, cx| {
                        this.transcript
                            .update(cx, |view, cx| view.note_result(turn, result, cx))
                    });
                })
                .detach();
            }
            ResponseRequest::Target { turn, kind, anchor } => {
                if self
                    .response_request
                    .is_some_and(|(at, k, _)| at == *turn && k == *kind)
                {
                    self.close_response_menu(cx);
                    return;
                }
                let turn = *turn;
                let kind = *kind;
                let token = Instant::now();
                let anchor = anchor.clone();
                self.menu = None;
                self.git_picker = None;
                self.response_request = Some((turn, kind, token));
                self.transcript.update(cx, |view, cx| {
                    view.target_action = Some((turn, kind));
                    cx.notify();
                });
                self.open_response_menu(turn, kind, token, anchor, window, cx);
                cx.notify();
            }
        }
    }

    pub(super) fn finish_response_target(
        &mut self,
        turn: usize,
        kind: monocode_core::session::response::TransferKind,
        id: &str,
        cx: &mut Context<Self>,
    ) {
        let Ok(target) = serde_json::from_str::<monocode_core::session::response::Target>(id)
        else {
            return;
        };
        let from = self
            .transcript
            .read(cx)
            .turns
            .get(&turn)
            .map(|t| t.provider.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.summary.provider)
            .to_owned();
        let (rows, through) = self.transcript.read(cx).response_rows(turn);
        let transfer = Transfer {
            kind,
            target,
            from,
            turn: rows,
            through,
            cwd: self.summary.cwd.clone(),
            title: self.summary.title.clone(),
            settings: self.settings.clone(),
        };
        self.close_response_menu(cx);
        cx.emit(transfer);
    }

    /// `Session: Stop` on Escape: cancels a running turn.
    pub(crate) fn stop_if_busy(&mut self, cx: &mut Context<Self>) {
        if self.cancel_edit_last_turn(cx) {
            return;
        }
        if self.busy {
            self.stop(cx);
        }
    }

    pub(super) fn stop(&mut self, cx: &mut Context<Self>) {
        if !self.busy || self.stopping {
            return;
        }
        self.stopping = true;
        self.busy = false;
        self.follow_ups.resuming = false;
        self.follow_ups.continuations.clear();
        self.notification_cancelled = true;
        self.cancel.store(true, Ordering::Relaxed);
        if !self.follow_ups.state.messages.is_empty() {
            self.follow_ups.state.paused = true;
            self.follow_ups.requested = None;
            self.save_follow_ups(cx);
        }
        self.follow_ups.ready = false;
        self.clear_background();
        self.approvals.clear();
        self.questions.clear();
        self.controls = None;
        self.status = "Ready".into();
        self.transcript
            .update(cx, |view, cx| view.stop_streaming(cx));
        self.review
            .update(cx, |review, cx| review.set_busy(false, cx));
        cx.emit(SessionChanged);
        cx.notify();
    }

    // Composer.tsx in the inset layout: a frosted, rounded box holding the
    // field and its toolbar, with the workspace bar sitting under it.
    pub(super) fn composer_runner(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<Entity<crate::chat::composer::runner::Runner>> {
        // A Mono already signs its turns with its own mascot.
        if self.settings.mono.is_some()
            || cx
                .try_global::<crate::ui::preferences::Preferences>()
                .is_some_and(|p| !p.composer_mascot)
        {
            return None;
        }
        if self.busy && self.runner.is_none() {
            let transcript = self.transcript.clone();
            self.runner =
                Some(cx.new(|cx| crate::chat::composer::runner::Runner::new(transcript, cx)));
        }
        let runner = self.runner.clone()?;
        let name = monocode_core::project::appearance::MASCOTS
            [self.project_appearance.mascot(&self.summary.cwd)];
        let color = self.project_appearance.color(&self.summary.cwd);
        runner.update(cx, |r, cx| r.configure(self.busy, name, color, cx));
        Some(runner)
    }
}
