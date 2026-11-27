use super::*;

impl Shell {
    pub(super) fn start_mobile_watch(&mut self, start: WatchStart, cx: &mut Context<Self>) {
        let target = match self.mobile_target(&start.conversation_id, cx) {
            Ok(target) => target,
            Err(error) => return start.accept(Err(error)),
        };
        let id = start.id.clone();
        self.mobile.watches.insert(
            id,
            Watch {
                target,
                tail: (start.tail.unwrap_or(DEFAULT_TAIL as u64) as usize).clamp(1, MAX_PAGE),
                from: start.from.clone(),
                incremental: start.incremental,
                baseline: None,
                saved: None,
                loading: false,
                observed: None,
                _observers: Vec::new(),
                _follow: None,
            },
        );
        start.accept(Ok(()));
        // The phone waits for its first frame; send it straight away.
        self.push_mobile_watches(cx);
    }

    pub(super) fn schedule_mobile_push(&mut self, cx: &mut Context<Self>) {
        if self.mobile.push_scheduled || self.mobile.watches.is_empty() {
            return;
        }
        self.mobile.push_scheduled = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PUSH_INTERVAL).await;
            let _ = this.update(cx, |this, cx| {
                this.mobile.push_scheduled = false;
                this.push_mobile_watches(cx);
            });
        })
        .detach();
    }

    /// Follows the chat a watch shows, so each change becomes a frame.
    pub(super) fn observe_mobile_watch(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(watch) = self.mobile.watches.get(id) else {
            return;
        };
        let view = match &watch.target {
            Target::Session(session) => self.sessions.get(session).cloned(),
            Target::Mono(mono) => self.monos.as_ref().and_then(|m| m.read(cx).open_chat(mono)),
        };
        let Some(view) = view else {
            return;
        };
        let cli = view.read(cx).claude_mode(cx) == Some(true);
        if watch.observed == Some((view.entity_id(), cli)) {
            return;
        }
        let transcript = view.read(cx).transcript.clone();
        let observers = vec![
            cx.observe(&view, |this, _, cx| this.schedule_mobile_push(cx)),
            cx.observe(&transcript, |this, _, cx| this.schedule_mobile_push(cx)),
        ];
        let follow = cli.then(|| self.follow_cli_log(id, view.downgrade(), cx));
        if let Some(watch) = self.mobile.watches.get_mut(id) {
            // Read before the mode changed, it holds the other side's rows.
            if watch.observed.is_some_and(|(_, was)| was != cli) {
                watch.saved = None;
            }
            watch.observed = Some((view.entity_id(), cli));
            watch._observers = observers;
            watch._follow = follow;
        }
    }

    /// The CLI writes its conversation to a log, not to the chat: each time
    /// the log changes, the watch reads it again and sends the new turns.
    fn follow_cli_log(
        &self,
        id: &str,
        view: gpui::WeakEntity<crate::chat::conversation::Conversation>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let id = id.to_owned();
        cx.spawn(async move |this, cx| {
            let mut seen = None;
            // ponytail: stats the log every half second while a phone shows
            // it; a file watcher if many watched CLI tabs make it show.
            loop {
                cx.background_executor().timer(CLI_LOG_POLL).await;
                let Ok((account, binding)) = view.read_with(cx, |chat, _| {
                    (
                        chat.settings.account.clone(),
                        chat.summary.provider_id.clone(),
                    )
                }) else {
                    break;
                };
                let stamp =
                    cx.background_executor()
                        .spawn(async move {
                            binding.and_then(|binding| cli_log_stamp(&account, &binding))
                        })
                        .await;
                if stamp == seen {
                    continue;
                }
                let first = seen.is_none();
                seen = stamp;
                if first {
                    continue;
                }
                let alive = this.update(cx, |this, cx| {
                    let Some(watch) = this.mobile.watches.get_mut(&id) else {
                        return false;
                    };
                    watch.saved = None;
                    this.push_mobile_watches(cx);
                    true
                });
                if !matches!(alive, Ok(true)) {
                    break;
                }
            }
        })
    }

    pub(super) fn push_mobile_watches(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self.mobile.host.clone() else {
            return;
        };
        let ids: Vec<String> = self.mobile.watches.keys().cloned().collect();
        let mut again = false;
        for id in ids {
            self.observe_mobile_watch(&id, cx);
            let Some(watch) = self.mobile.watches.get(&id) else {
                continue;
            };
            let target = watch.target.clone();
            let (tail, from, incremental) = (watch.tail, watch.from.clone(), watch.incremental);
            let frame = |snapshot: &Snapshot| {
                let source = snapshot.source();
                let ids = turns::turn_ids(&source);
                let (start, from, older) = watch::window(&ids, from.as_deref(), tail);
                // Turns above the loaded rows are older history too.
                let older = older || snapshot.history.is_some();
                let built = turns::build(&source, start..ids.len());
                (
                    from,
                    older,
                    built,
                    snapshot.busy,
                    snapshot.question.clone(),
                    snapshot.usage_limit.clone(),
                )
            };
            let built = match self.with_live_snapshot(&target, cx, frame) {
                Some(built) => built,
                None => match watch.saved.clone() {
                    Some(saved) => frame(&saved_snapshot(&saved)),
                    None => {
                        self.load_mobile_watch(&id, cx);
                        continue;
                    }
                },
            };
            let Some(watch) = self.mobile.watches.get_mut(&id) else {
                continue;
            };
            let (from, older, built, busy, question, usage_limit) = built;
            watch.from = from;
            let Some((update, baseline)) = watch::update(
                watch.baseline.as_ref(),
                busy,
                older,
                &question,
                &built,
                incremental,
                &usage_limit,
            ) else {
                continue;
            };
            match host.push(&id, update) {
                Ok(()) => watch.baseline = Some(baseline),
                // The phone is behind; the next push sends from the old baseline.
                Err(Push::Busy) => again = true,
                Err(Push::Gone) => {
                    self.mobile.watches.remove(&id);
                }
            }
        }
        if again {
            self.schedule_mobile_push(cx);
        }
    }

    /// A watched session that is not open is read from disk once.
    pub(super) fn load_mobile_watch(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(watch) = self.mobile.watches.get_mut(id) else {
            return;
        };
        if watch.loading {
            return;
        }
        watch.loading = true;
        let target = watch.target.clone();
        let load = self.load_saved(&target, cx);
        let id = id.to_owned();
        cx.spawn(async move |this, cx| {
            let result = load.await;
            let _ = this.update(cx, |this, cx| {
                let Some(watch) = this.mobile.watches.get_mut(&id) else {
                    return;
                };
                watch.loading = false;
                match result {
                    Ok(saved) => {
                        let limited = saved.settings.usage_limit.is_some();
                        let target = watch.target.clone();
                        watch.saved = Some(saved);
                        // A limited conversation needs the desktop clock even when
                        // first opened from a phone: reset labels and eligibility
                        // must advance without another transcript row arriving.
                        if limited {
                            let view = match target {
                                Target::Session(session) => this.ensure_session(session, cx),
                                Target::Mono(mono) => this
                                    .monos
                                    .clone()
                                    .and_then(|m| m.update(cx, |m, cx| m.mobile_chat(&mono, cx))),
                            };
                            if let Some(view) = view {
                                view.update(cx, |v, cx| v.load(cx));
                            }
                        }
                        this.push_mobile_watches(cx);
                    }
                    Err(error) => {
                        if let Some(host) = &this.mobile.host {
                            let _ = host.push(&id, json!({"error": error}));
                        }
                        this.mobile.watches.remove(&id);
                    }
                }
            });
        })
        .detach();
    }

    /// Sends a session's notice to paired phones: always a Mono's, and
    /// another's while a phone started its current turn, which its finish
    /// ends. Not while the desktop shows that session in front of the user.
    pub(crate) fn notify_mobile(
        &mut self,
        summary: &Session,
        category: &str,
        body: &str,
        cx: &mut Context<Self>,
    ) {
        let id = summary.id;
        let from_phone = if category == "agentFinished" {
            self.mobile.started.remove(&id)
        } else {
            self.mobile.started.contains(&id)
        };
        // A Mono's own chat, or one of its habit runs.
        let mono = self.monos.as_ref().and_then(|monos| {
            let mono = monos.read(cx).notification_mono(id)?;
            Some((mono.id.clone(), mono.label(), mono.session_id != Some(id)))
        });
        if self.mobile.host.is_none()
            || (mono.is_none() && !from_phone)
            || (self.window_active && self.notice_session_visible(id, cx))
        {
            return;
        }
        let body = delivery::clip_body(body);
        let notice = match mono {
            Some((mono, name, habit)) => notify::Notice {
                title: name,
                subtitle: None,
                body: if body.is_empty() {
                    "Sent you a message".into()
                } else {
                    body
                },
                // A habit run's question is in its own session, not the
                // Mono's chat: the phone opens it as it does a Mono's
                // sessions, in any project.
                data: if habit && self.session_summaries.contains_key(&id) {
                    let mut data = self.mobile_notice_session(summary, cx);
                    data["monoId"] = json!(mono);
                    data
                } else {
                    json!({"monoId": mono})
                },
            },
            None => {
                let (project, _) = self.mobile_notice_project(summary, cx);
                notify::Notice {
                    title: monocode_core::session::title::display_title(
                        &summary.title,
                        &summary.provider,
                    )
                    .to_owned(),
                    subtitle: Some(project),
                    body: if body.is_empty() {
                        "Finished".into()
                    } else {
                        body
                    },
                    data: self.mobile_notice_session(summary, cx),
                }
            }
        };
        // A Mono's own messages are their own category; its questions and
        // approvals are like any agent's.
        let category = if self
            .monos
            .as_ref()
            .is_some_and(|monos| monos.read(cx).notification_mono(id).is_some())
            && category != "agentInput"
        {
            "monoMessages"
        } else {
            category
        };
        self.send_mobile_notice(category, notice, cx);
    }

    /// The project a session's notice is filed under, by name and path.
    pub(super) fn mobile_notice_project(&self, summary: &Session, cx: &App) -> (String, String) {
        let key = monocode_core::files::path_key(&summary.cwd);
        let path = self
            .mobile_projects(cx)
            .into_iter()
            .find(|path| monocode_core::files::path_key(path) == key)
            .unwrap_or_else(|| summary.cwd.clone());
        (self.appearance(&path).label(&path), path)
    }

    /// Where a tap on a session's notice opens it on the phone.
    pub(super) fn mobile_notice_session(&mut self, summary: &Session, cx: &App) -> Value {
        let (_, path) = self.mobile_notice_project(summary, cx);
        json!({
            "projectId": project_id(&path),
            "sessionId": self.mobile_session_id(summary.id),
        })
    }

    /// A turn the phone started ended with no finish to tell it of, as when
    /// it was stopped: the session's next turns are the desktop's again.
    pub(crate) fn mobile_turn_ended(&mut self, id: i64) {
        self.mobile.started.remove(&id);
    }

    /// A Mono's report, posted to its chat on its own.
    pub(crate) fn notify_mobile_post(&mut self, session: i64, body: &str, cx: &mut Context<Self>) {
        if let Some(view) = self.notification_chat(session, cx) {
            let summary = view.read(cx).summary.clone();
            self.notify_mobile(&summary, "monoMessages", body, cx);
        }
    }

    pub(super) fn send_mobile_notice(&self, category: &str, notice: notify::Notice, cx: &App) {
        let Some(host) = self.mobile.host.clone() else {
            return;
        };
        let tokens = host.push_tokens(category);
        if tokens.is_empty() {
            return;
        }
        cx.background_executor()
            .spawn(async move {
                match notify::send(&tokens, &notice) {
                    Ok(gone) if !gone.is_empty() => {
                        let _ = host.forget_push_tokens(&gone);
                    }
                    Ok(_) => {}
                    Err(error) => eprintln!("Mobile notification not sent: {error}"),
                }
            })
            .detach();
    }
}

/// The size and write time of a Claude session's CLI log, which change with
/// each line the CLI writes.
fn cli_log_stamp(
    account: &monocode_core::session::profile::Profile,
    binding: &str,
) -> Option<(u64, std::time::SystemTime)> {
    use monocode_core::provider;
    let path = provider::claude_transcript(account, provider::claude_session(binding))?;
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}
