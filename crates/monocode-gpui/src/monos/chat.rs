use super::*;

impl Monos {
    pub(crate) fn chat(&self) -> Option<Entity<Conversation>> {
        self.active
            .as_ref()
            .and_then(|id| self.chats.get(id))
            .cloned()
    }

    /// A Mono's chat when it is open, without opening it.
    pub(crate) fn open_chat(&self, id: &str) -> Option<Entity<Conversation>> {
        self.chats.get(id).cloned()
    }

    /// A Mono's chat for a paired phone, opening it in the background when
    /// it is not open yet.
    pub(crate) fn mobile_chat(
        &mut self,
        id: &str,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Conversation>> {
        self.pane_chat(id, cx)
    }

    /// A Mono's chat for a split pane, loading it when it is not open yet.
    pub(crate) fn pane_chat(
        &mut self,
        id: &str,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Conversation>> {
        self.ensure_chat(id, cx);
        self.chats.get(id).cloned()
    }

    /// A phone's message to a Mono, queued as one typed in its chat.
    pub(crate) fn mobile_send(
        &mut self,
        id: &str,
        text: String,
        attachments: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.roster.get(id).is_none() {
            return Err("Mono is no longer available".into());
        }
        self.ensure_chat(id, cx);
        self.enqueue(id, text, attachments, false, cx);
        Ok(())
    }

    pub(super) fn ensure_chat(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.chats.contains_key(id) || !self.loading_chats.insert(id.into()) {
            return;
        }
        let Some(mono) = self.roster.get(id).cloned() else {
            return;
        };
        let store = self.store.clone();
        let prefs = self.prefs.clone();
        let id = id.to_string();
        let work = cx.background_executor().spawn(async move {
            let db = locked(&store)?;
            if let Some(missing) = &mono.unavailable_session {
                return Err(format!("This Mono's saved conversation ({missing}) is unavailable. Its identity and history have been preserved."));
            }
            if let Some(session) = mono.session_id {
                let summary = db.mono_session(session)?
                    .ok_or("This Mono's saved conversation is unavailable. Reset its conversation explicitly to start a new one.")?;
                let delivered = mono
                    .outbox
                    .iter()
                    .filter(|q| {
                        db.setting(&format!("mono-delivered:{}:{}", summary.id, q.id))
                            .ok()
                            .flatten()
                            .is_some()
                    })
                    .map(|q| q.id.clone())
                    .collect::<Vec<_>>();
                return Ok::<_, String>((summary, None, delivered));
            }
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".into());
            let provider = prefs
                .default_provider
                .as_deref()
                .and_then(Kind::parse)
                .unwrap_or(Kind::Codex);
            let mut summary = db.create(provider.id(), &home)?;
            summary.title = mono.label();
            db.title(summary.id, &summary.title)?;
            // A Mono's chat is conversation: it answers quickly at low effort
            // and hands heavy work to background sessions. Only new chats get
            // this; existing Monos keep whatever effort they already have.
            let settings = Settings {
                access: AccessMode::Auto,
                model: prefs.default_models.get(provider.id()).cloned(),
                effort: Some("low".into()),
                ..Default::default()
            };
            db.configure_agent(summary.id, provider.id(), &settings)?;
            Ok((summary, Some(settings), Vec::new()))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.loading_chats.remove(&id);
                let (summary, settings, delivered) = match result {
                    Ok(value) => value,
                    Err(e) => {
                        this.error = Some(e);
                        this.changed(cx);
                        return;
                    }
                };
                if !this.roster.hidden_sessions.contains(&summary.id) {
                    this.roster.hidden_sessions.push(summary.id);
                }
                let Some(mono) = this.roster.get_mut(&id) else {
                    return;
                };
                mono.session_id = Some(summary.id);
                mono.outbox.retain(|q| !delivered.contains(&q.id));
                let mono = mono.clone();
                let view = cx.new(|cx| Conversation::new(summary, this.store.clone(), cx));
                let root = this.root.clone();
                let env = this
                    .bridge
                    .as_ref()
                    .map(|b| b.environment(view.read(cx).summary.id))
                    .unwrap_or_default();
                view.update(cx, |view, cx| {
                    view.configure_mono(mono, root, env, cx);
                    if let Some(settings) = settings {
                        let config = view.settings.mono.take();
                        let env = std::mem::take(&mut view.settings.environment);
                        view.install_transfer(settings, None, cx);
                        view.settings.mono = config;
                        view.settings.environment = env;
                    }
                    view.load(cx);
                });
                let source = id.clone();
                this.observations.push(cx.subscribe(
                    &view,
                    move |this, _, event: &MonoSubmitted, cx| {
                        this.enqueue(
                            &source,
                            event.text.clone(),
                            event
                                .attachments
                                .iter()
                                .map(|p| p.to_string_lossy().to_string())
                                .collect(),
                            false,
                            cx,
                        )
                    },
                ));
                let source = id.clone();
                this.observations.push(cx.subscribe(
                    &view,
                    move |this, _, _: &SessionChanged, cx| {
                        this.chat_changed(&source, cx);
                    },
                ));
                let source = id.clone();
                this.observations.push(cx.subscribe(
                    &view,
                    move |this, _, _: &crate::chat::conversation::UsageResumed, cx| {
                        let empty = this.roster.get(&source).is_none_or(|m| m.outbox.is_empty());
                        if empty {
                            this.enqueue(
                                &source,
                                "Continue from where you left off.".into(),
                                Vec::new(),
                                false,
                                cx,
                            );
                        } else {
                            if let Some(mono) = this.roster.get_mut(&source) {
                                for message in &mut mono.outbox {
                                    message.error = None;
                                }
                            }
                            this.save(cx);
                            this.configure_chat(&source, cx);
                        }
                    },
                ));
                let source = id.clone();
                this.observations
                    .push(cx.observe(&view, move |this, _, cx| {
                        if this.active.as_ref() == Some(&source) {
                            cx.notify();
                        }
                        this.dispatch(&source, cx);
                    }));
                let source = id.clone();
                this.observations.push(cx.subscribe(
                    &view,
                    move |this, _, event: &crate::chat::conversation::MonoQueueChanged, cx| {
                        this.queue_changed(&source, &event.id, event.remove, cx)
                    },
                ));
                let transcript = view.read(cx).transcript.clone();
                let target = view.clone();
                let source = id.clone();
                this.observations.push(cx.subscribe(
                    &transcript,
                    move |this, _, event: &crate::chat::transcript::MonoResponseAction, cx| {
                        match event {
                            crate::chat::transcript::MonoResponseAction::Queue { id, remove } => {
                                this.queue_changed(&source, id, *remove, cx)
                            }
                            crate::chat::transcript::MonoResponseAction::Activity(turn) => {
                                this.activity_turn = Some(*turn);
                                this.show_overview(cx);
                                this.route = None;
                                this.overview_tab = OverviewTab::Activity;
                                cx.notify();
                            }
                            crate::chat::transcript::MonoResponseAction::Quote { text, window } => {
                                let target = target.clone();
                                let text = text.clone();
                                let handle = *window;
                                let app: &mut App = cx;
                                app.defer(move |cx| {
                                    let _ = handle.update(cx, |_, window, cx| {
                                        target.update(cx, |v, cx| v.mono_quote(&text, window, cx))
                                    });
                                });
                            }
                        }
                    },
                ));
                let source = id.clone();
                this.observations.push(cx.subscribe(
                    &transcript,
                    move |this, _, event: &cards::CardAction, cx| {
                        this.card_action(&source, &event.0, cx)
                    },
                ));
                this.chats.insert(id.clone(), view);
                if let Some(handle) = this.parent_window {
                    let owner = cx.weak_entity();
                    let id = id.clone();
                    cx.defer(move |cx| {
                        let _ = handle.update(cx, |_, window, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                if this.active.as_deref() == Some(&id)
                                    && this.settings_focus.is_focused(window)
                                {
                                    this.focus_active(window, cx);
                                }
                            });
                        });
                    });
                }
                this.save(cx);
                this.dispatch(&id, cx);
            });
        })
        .detach();
    }

    /// Retry clears a queued message's error; discard drops it.
    pub(super) fn queue_changed(
        &mut self,
        source: &str,
        id: &str,
        remove: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(mono) = self.roster.get_mut(source) {
            if remove {
                mono.outbox.retain(|m| m.id != id);
            } else if let Some(message) = mono.outbox.iter_mut().find(|m| m.id == id) {
                message.error = None;
            }
        }
        self.save(cx);
        self.configure_chat(source, cx);
    }

    pub(super) fn configure_chat(&mut self, id: &str, cx: &mut Context<Self>) {
        self.refresh_work(id, cx);
        if let Some(mono) = self.roster.get(id) {
            // The shell may be mid-update when this runs; reach it afterwards.
            let launches = mono.launches.iter().map(|l| l.session_id).collect();
            let owner = self.owner.clone();
            cx.defer(move |cx| {
                let _ = owner.update(cx, |shell, cx| shell.mono_track_sessions(launches, cx));
            });
        }
        if let (Some(mono), Some(view)) =
            (self.roster.get(id).cloned(), self.chats.get(id).cloned())
        {
            let env = self
                .bridge
                .as_ref()
                .map(|b| b.environment(view.read(cx).summary.id))
                .unwrap_or_default();
            let root = self.root.clone();
            view.update(cx, |view, cx| view.configure_mono(mono, root, env, cx));
        }
    }

    pub(super) fn enqueue(
        &mut self,
        id: &str,
        text: String,
        attachments: Vec<String>,
        internal: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(mono) = self.roster.get_mut(id) {
            mono.outbox.push(Message {
                id: mono::new_id(),
                text,
                sent_at: Some(monocode_core::sys::time::now_ms()),
                internal,
                error: None,
                attachments,
            });
            self.save(cx);
            self.configure_chat(id, cx);
            self.dispatch(id, cx);
        }
    }

    pub(super) fn dispatch(&mut self, id: &str, cx: &mut Context<Self>) {
        let delivered = self
            .chats
            .get(id)
            .map(|v| v.read(cx).mono_delivered().clone())
            .unwrap_or_default();
        let failures = self
            .chats
            .get(id)
            .cloned()
            .map(|view| view.update(cx, |view, _| view.mono_take_failures()))
            .unwrap_or_default();
        let mut changed = false;
        if let Some(mono) = self.roster.get_mut(id) {
            let before = mono.outbox.len();
            mono.outbox
                .retain(|message| !delivered.contains(&message.id));
            changed = before != mono.outbox.len();
            for (id, error) in failures {
                if let Some(message) = mono.outbox.iter_mut().find(|m| m.id == id) {
                    message.error = Some(error);
                    changed = true;
                }
            }
        }
        if changed {
            self.save(cx);
            self.configure_chat(id, cx);
        }

        if let Some(view) = self
            .chats
            .get(id)
            .cloned()
            .filter(|v| !self.failed_reports.contains(id) && v.read(cx).ready_for_external(cx))
        {
            let reports = self
                .roster
                .get_mut(id)
                .map(|m| std::mem::take(&mut m.pending_reports))
                .unwrap_or_default();
            if !reports.is_empty() {
                let mut failed = Vec::new();
                for report in reports {
                    if self.failed_reports.contains(id) {
                        failed.push(report);
                        continue;
                    }
                    let (role, text) = report
                        .strip_prefix("mono-card:")
                        .map(|s| ("mono-card", s.to_string()))
                        .unwrap_or_else(|| ("assistant", report.clone()));
                    let body = (role == "assistant").then(|| {
                        text.strip_prefix("**Habit report**")
                            .unwrap_or(&text)
                            .trim()
                            .to_owned()
                    });
                    if let Err(error) = view.update(cx, |v, cx| v.mono_post(role, text, cx)) {
                        self.error = Some(error);
                        self.failed_reports.insert(id.into());
                        failed.push(report);
                    } else if let Some(body) = body {
                        let session = view.read(cx).summary.id;
                        cx.emit(NotificationPosted { session, body });
                    }
                }
                if let Some(mono) = self.roster.get_mut(id) {
                    mono.pending_reports.extend(failed);
                }
                self.save(cx);
            }
        }
        let Some(view) = self.chats.get(id).cloned() else {
            return;
        };
        let busy = view.read(cx).busy;
        let Some(message) = self
            .roster
            .get(id)
            .and_then(|m| m.outbox.iter().find(|message| !busy || !message.internal))
            .cloned()
            .filter(|m| m.error.is_none() && self.persisted_messages.contains(&m.id))
        else {
            return;
        };
        if view.update(cx, |v, cx| v.mono_dispatch(&message, cx)) && !busy {
            self.inflight.insert(id.into(), message);
        }
    }

    pub(super) fn chat_changed(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(view) = self.chats.get(id) else {
            return;
        };
        let view = view.read(cx);
        let row_count = view.mono_rows(cx).len();
        let finished = !view.busy;
        let completion =
            (finished && self.inflight.contains_key(id)).then(|| NotificationFinished {
                summary: view.summary.clone(),
                body: view.notification_reply_body(cx),
                habit: false,
                was_cancelled: view.notification_was_cancelled(),
            });
        cx.emit(NotificationChanged);
        if let Some(completion) = completion {
            cx.emit(completion);
        }
        if self.active.as_deref() == Some(id)
            && let Some(mono) = self.roster.get_mut(id)
        {
            mono.seen_rows = row_count;
        }
        if finished {
            if let Some(mut sent) = self.inflight.remove(id) {
                let (ok, status) = self.chats[id].read(cx).external_outcome(cx);
                if !ok && !status.starts_with("Stopped") {
                    if let Some(mono) = self.roster.get_mut(id) {
                        if let Some(queued) = mono.outbox.iter_mut().find(|m| m.id == sent.id) {
                            queued.error = Some(status);
                        } else {
                            sent.id = mono::new_id();
                            sent.error = Some(status);
                            mono.outbox.insert(0, sent);
                        }
                    }
                    self.save(cx);
                    self.configure_chat(id, cx);
                }
            }
            self.check_completions(cx);
            self.reload_files(id, cx);
        }
        self.dispatch(id, cx);
        self.changed(cx);
    }

    pub(crate) fn open(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.prefs.monos || self.roster.get(id).is_none() {
            return;
        }
        self.make_active(id, cx);
        self.ensure_chat(id, cx);
        cx.emit(TabOpened(id.into()));
        self.focus_active(window, cx);
        if let Some(view) = self.chats.get(id) {
            let count = view.read(cx).mono_rows(cx).len();
            if let Some(mono) = self.roster.get_mut(id) {
                mono.seen_rows = count;
            }
        }
        self.save(cx);
    }

    /// Makes `id` the active Mono: on a switch, saves the previous Mono's
    /// state, restores `id`'s panel view, and drops the previous Mono's files
    /// and habits so nothing acts on them while `id`'s reload.
    pub(crate) fn make_active(&mut self, id: &str, cx: &mut Context<Self>) {
        let switched = self.active.as_deref() != Some(id);
        if switched {
            self.save_open_file(cx);
            self.pause_changes(cx);
            self.remember_panel_view();
            self.restore_panel_view(id, cx);
            self.details.update(cx, |details, cx| details.reset(cx));
            self.fields.retain(|k, _| k.starts_with("file:"));
            self.files = None;
            self.habits.clear();
            self.activity_turn = None;
            self.memory_adding = false;
            self.memory_edit = None;
            self.settings = None;
        }
        self.active = Some(id.into());
        // A refresh may have evicted a restored document's body meanwhile.
        if switched
            && let Some(Route::Document(document)) = self.route.clone()
            && !self.work.documents.contains_key(&document)
        {
            self.open_document(&document, cx);
        }
        self.reload_files(id, cx);
    }

    pub(crate) fn focus_active(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.is_none()
            && let Some(chat) = self.chat()
        {
            window.focus(&chat.read(cx).input.focus_handle(cx), cx);
        } else {
            window.focus(&self.settings_focus, cx);
        }
    }

    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        self.save_open_file(cx);
        self.pause_changes(cx);
        self.hide_html(cx);
        self.remember_panel_view();
        if self.active.take().is_some() {
            self.details.update(cx, |details, cx| details.reset(cx));
            self.settings = None;
            self.picker = None;
            self.save(cx);
        }
    }
}
