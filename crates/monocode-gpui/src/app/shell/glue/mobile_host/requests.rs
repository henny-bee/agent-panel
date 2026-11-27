use super::*;
use monocode_core::provider;

impl Shell {
    pub(super) fn handle_mobile_request(
        &mut self,
        request: Request,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let method = request.method.clone();
        match method.as_str() {
            "conversation.read" | "conversation.older" => self.mobile_read(request, cx),
            "models.list" => self.mobile_models(request, cx),
            "accounts.usage" => self.mobile_usage(request, cx),
            "mono.document" => self.mobile_document(request, cx),
            "mono.profile" | "mono.update" | "mono.reset" | "mono.delete" | "mono.create" => {
                self.mobile_mono(request, cx)
            }
            "mono.file" | "mono.saveFile" => self.mobile_mono_file(request, cx),
            "project.fileHighlight" => {
                let job = string(&request.params, "job").unwrap_or_default();
                let from = request.params["from"].as_u64().unwrap_or(0) as usize;
                request.reply(Ok(project::file_highlight(&job, from)));
            }
            "project.gitAction" => self.mobile_git_action(request, cx),
            method if method.starts_with("project.") => self.mobile_project(request, cx),
            _ => self.mobile_command(request, window, cx),
        }
    }

    /// Staging, committing, pushing and pull requests, as the Changes panel
    /// runs them. A slow action keeps running; the phone asks again with the
    /// same command ID until it is done, and asking again never reruns it.
    pub(super) fn mobile_git_action(&mut self, request: Request, cx: &mut Context<Self>) {
        let signature = request.params.to_string();
        let result = match self.mobile.git_jobs.get(&request.command_id) {
            Some(job) if job.signature != signature => {
                return request.reply(Err(
                    "Command ID was already used with different input".into()
                ));
            }
            Some(job) => job.result.clone(),
            None => {
                let cwd = match self.mobile_project_path(
                    &string(&request.params, "projectId").unwrap_or_default(),
                    cx,
                ) {
                    Ok(cwd) => cwd,
                    Err(error) => return request.reply(Err(error)),
                };
                let result = Arc::new(Mutex::new(None));
                let slot = result.clone();
                let params = request.params.clone();
                let chosen = cx
                    .try_global::<crate::ui::preferences::Preferences>()
                    .and_then(|p| p.background_text_model());
                cx.background_executor()
                    .spawn(async move {
                        let cwd = PathBuf::from(cwd);
                        let outcome = project::resolve_root(&cwd, params["worktreePath"].as_str())
                            .and_then(|root| project::git_action(&root, &params, chosen));
                        if let Ok(mut slot) = slot.lock() {
                            *slot = Some(outcome);
                        }
                    })
                    .detach();
                if self.mobile.git_jobs.len() >= MAX_GIT_JOBS
                    && let Some(oldest) = self.mobile.git_order.pop_front()
                {
                    self.mobile.git_jobs.remove(&oldest);
                }
                self.mobile.git_order.push_back(request.command_id.clone());
                self.mobile.git_jobs.insert(
                    request.command_id.clone(),
                    GitJob {
                        signature,
                        result: result.clone(),
                    },
                );
                result
            }
        };
        cx.spawn(async move |this, cx| {
            let started = std::time::Instant::now();
            loop {
                let done = result.lock().ok().and_then(|slot| slot.clone());
                if let Some(done) = done {
                    // The Changes panel and badges read the new state too.
                    let _ = this.update(cx, |this, cx| this.sync_git(cx));
                    return request.reply(done);
                }
                if started.elapsed() >= GIT_ACTION_WAIT {
                    return request.reply(Ok(json!({"pending": true})));
                }
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
            }
        })
        .detach();
    }

    /// A conversation as the phone shows it: its latest turns, or a page of
    /// older ones before a turn it already has.
    pub(super) fn mobile_read(&mut self, request: Request, cx: &mut Context<Self>) {
        let conversation = string(&request.params, "conversationId").unwrap_or_default();
        let target = match self.mobile_target(&conversation, cx) {
            Ok(target) => target,
            Err(error) => return request.reply(Err(error)),
        };
        let older = request.method == "conversation.older";
        let before = string(&request.params, "before").unwrap_or_default();
        let limit = (request.params["limit"]
            .as_u64()
            .unwrap_or(DEFAULT_TAIL as u64) as usize)
            .clamp(1, MAX_PAGE);
        // Before a turn of an older page the phone already holds: the store
        // has the rest.
        if older && let Some((cursor, at)) = page_turn(&before) {
            return self.mobile_older_page(request, &target, cursor, Some(at), limit, cx);
        }
        let page = move |snapshot: &Snapshot| -> Result<Read, String> {
            let source = snapshot.source();
            let ids = turns::turn_ids(&source);
            if older {
                let (start, end, more) = watch::before(&ids, &before, limit)?;
                // At the top of the loaded rows; a long Mono keeps the rest in
                // the store.
                if start == end
                    && let Some(cursor) = snapshot.history
                {
                    return Ok(Read::Page(cursor));
                }
                return Ok(Read::Done(json!({
                    "turns": turns::build(&source, start..end),
                    "older": more || snapshot.history.is_some(),
                })));
            }
            let start = ids.len().saturating_sub(READ_TURNS);
            Ok(Read::Done(json!({
                "id": conversation,
                "messages": turns::messages(snapshot.rows),
                "turns": turns::build(&source, start..ids.len()),
                "busy": snapshot.busy,
                "usageLimit": snapshot.usage_limit,
                "older": start > 0 || snapshot.history.is_some(),
                "question": snapshot.question,
            })))
        };
        match self.with_live_snapshot(&target, cx, &page) {
            Some(Ok(Read::Done(value))) => request.reply(Ok(value)),
            Some(Ok(Read::Page(cursor))) => {
                self.mobile_older_page(request, &target, cursor, None, limit, cx)
            }
            Some(Err(error)) => request.reply(Err(error)),
            None => {
                let load = self.load_saved(&target, cx);
                cx.spawn(async move |this, cx| {
                    match load.await.and_then(|saved| page(&saved_snapshot(&saved))) {
                        Ok(Read::Done(value)) => request.reply(Ok(value)),
                        Ok(Read::Page(cursor)) => {
                            let _ = this.update(cx, |this, cx| {
                                this.mobile_older_page(request, &target, cursor, None, limit, cx)
                            });
                        }
                        Err(error) => request.reply(Err(error)),
                    }
                })
                .detach();
            }
        }
    }

    /// Turns from the store page at `cursor`, above the loaded rows: before
    /// its turn starting at row `before`, or with none its newest. Their IDs
    /// name the page, so the turns the phone has keep theirs.
    pub(super) fn mobile_older_page(
        &mut self,
        request: Request,
        target: &Target,
        cursor: i64,
        before: Option<usize>,
        limit: usize,
        cx: &mut Context<Self>,
    ) {
        let (session, name) = match target {
            Target::Session(id) => (Some(*id), None),
            Target::Mono(mono) => {
                let found = self.monos.as_ref().and_then(|m| {
                    let mono = m.read(cx).roster.get(mono)?;
                    Some((mono.session_id, mono.label()))
                });
                (found.as_ref().and_then(|f| f.0), found.map(|f| f.1))
            }
        };
        let Some(session) = session else {
            return request.reply(Ok(json!({"turns": [], "older": false})));
        };
        // Drawn as the loaded turns are, from the open chat or the summary.
        let look = self
            .with_live_snapshot(target, cx, |s| PageLook {
                mono: s.mono.clone(),
                launches: s.launches.clone(),
                cwd: s.cwd.to_owned(),
                harness: s.harness.to_owned(),
                model: s.model.clone(),
            })
            .unwrap_or_else(|| {
                let summary = self.session_summaries.get(&session);
                PageLook {
                    mono: name,
                    launches: self.mobile_launches(target, cx),
                    cwd: summary.map(|s| s.cwd.clone()).unwrap_or_default(),
                    harness: summary.map(|s| s.provider.clone()).unwrap_or_default(),
                    model: None,
                }
            });
        let store = self.store.clone();
        let work = cx.background_executor().spawn(async move {
            let store = locked(&store)?;
            older_page(&store, session, cursor, before, limit, look)
        });
        cx.spawn(async move |_, _| request.reply(work.await))
            .detach();
    }

    /// Runs `f` on the open chat's transcript when its history is in memory.
    pub(super) fn with_live_snapshot<T>(
        &self,
        target: &Target,
        cx: &App,
        f: impl FnOnce(&Snapshot) -> T,
    ) -> Option<T> {
        let (view, mono) = match target {
            Target::Session(id) => (self.sessions.get(id)?.clone(), None),
            Target::Mono(id) => {
                let manager = self.monos.as_ref()?.read(cx);
                let name = manager.roster.get(id).map(|m| m.label());
                (manager.open_chat(id)?, name)
            }
        };
        let chat = view.read(cx);
        // A tab showing the Claude CLI has no chat rows: its conversation is
        // read from the CLI's log, as a saved one is.
        if !chat.history_loaded() || chat.claude_mode(cx) == Some(true) {
            return None;
        }
        // Built for every streamed frame, so only once there's history to show.
        let launches = self.mobile_launches(target, cx);
        let waiting = match chat.mobile_waiting(cx) {
            Some(crate::chat::conversation::MobileWaiting::Approval) => {
                Some("Waiting for approval")
            }
            Some(crate::chat::conversation::MobileWaiting::Question) => Some("Waiting for answers"),
            None => None,
        };
        let snapshot = Snapshot {
            usage_limit: chat.mobile_usage_limit(cx),
            rows: chat.transcript.read(cx).source_rows(),
            turns: &chat.settings.turns,
            cwd: &chat.summary.cwd,
            harness: &chat.summary.provider,
            model: chat.mobile_model().map(str::to_owned),
            busy: chat.busy,
            waiting,
            mono,
            pending: chat.mobile_pending(cx),
            launches: &launches,
            question: chat.mobile_question(cx),
            history: chat.transcript.read(cx).older_history_cursor(),
        };
        Some(f(&snapshot))
    }

    pub(super) fn load_saved(
        &self,
        target: &Target,
        cx: &mut Context<Self>,
    ) -> Task<Result<Arc<Saved>, String>> {
        let (id, mono) = match target {
            Target::Session(id) => (Some(*id), None),
            Target::Mono(mono) => {
                let manager = self.monos.as_ref().map(|m| m.read(cx));
                let found = manager.and_then(|m| m.roster.get(mono));
                (found.and_then(|m| m.session_id), found.map(|m| m.label()))
            }
        };
        let summary = id.and_then(|id| self.session_summaries.get(&id).cloned());
        // An open tab knows the session its CLI moved to with `/clear` or
        // `/resume`; a closed one has its saved binding.
        let binding = id.and_then(|id| match self.sessions.get(&id) {
            Some(view) => view.read(cx).summary.provider_id.clone(),
            None => summary.as_ref().and_then(|s| s.provider_id.clone()),
        });
        let launches = self.mobile_launches(target, cx);
        let store = self.store.clone();
        cx.background_executor().spawn(async move {
            let Some(id) = id else {
                // A Mono that has not talked yet.
                return Ok(Arc::new(Saved {
                    id: 0,
                    settings: Settings::default(),
                    rows: Vec::new(),
                    harness: String::new(),
                    cwd: String::new(),
                    history: None,
                    mono,
                    launches,
                }));
            };
            let locked_store = locked(&store)?;
            let mut settings = locked_store.agent_settings(id)?.unwrap_or_default();
            let (harness, cwd) = match summary {
                Some(summary) => (summary.provider, summary.cwd),
                None => locked_store
                    .sessions()?
                    .into_iter()
                    .find(|s| s.id == id)
                    .map(|s| (s.provider, s.cwd))
                    .ok_or("Session is no longer available")?,
            };
            let log = cli_binding(&settings, &harness, binding.as_deref()).and_then(|binding| {
                provider::claude_transcript(&settings.account, provider::claude_session(binding))
                    .map(|path| (path, binding.to_owned()))
            });
            let (rows, history) = match log {
                Some((path, binding)) => {
                    drop(locked_store);
                    // Turn timings are numbered by the chat's rows, not these.
                    settings.turns.clear();
                    (provider::claude_history(&path, &binding)?, None)
                }
                None => (
                    locked_store.load(id)?.rows,
                    locked_store.history_before(id)?,
                ),
            };
            Ok(Arc::new(Saved {
                id,
                settings,
                rows,
                harness,
                cwd,
                history,
                mono,
                launches,
            }))
        })
    }

    pub(super) fn mobile_command(
        &mut self,
        request: Request,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let signature = json!([request.method, request.params]).to_string();
        match self.mobile.receipts.get(&request.command_id) {
            Some(Receipt::Pending(seen) | Receipt::Done(seen, _)) if *seen != signature => {
                return request.reply(Err(
                    "Command ID was already used with different input".into()
                ));
            }
            Some(Receipt::Done(_, result)) => return request.reply(result.clone()),
            Some(Receipt::Pending(_)) => {
                return request.reply(Err("Desktop is still working on this; try again".into()));
            }
            None => {}
        }
        if self.mobile.receipts.len() >= MAX_RECEIPTS
            && let Some(oldest) = self.mobile.receipt_order.pop_front()
        {
            self.mobile.receipts.remove(&oldest);
        }
        self.mobile
            .receipt_order
            .push_back(request.command_id.clone());
        self.mobile.receipts.insert(
            request.command_id.clone(),
            Receipt::Pending(signature.clone()),
        );
        let command = request.command_id.clone();
        let finish = move |this: &mut Shell, request: Request, result: Result<Value, String>| {
            // A failed command may be retried with the same ID.
            match &result {
                Ok(_) => {
                    this.mobile.receipts.insert(
                        command.clone(),
                        Receipt::Done(signature.clone(), result.clone()),
                    );
                }
                Err(_) => {
                    this.mobile.receipts.remove(&command);
                }
            }
            request.reply(result);
        };
        match request.method.as_str() {
            "session.start" => self.mobile_start(request, finish, cx),
            "conversation.send" => self.mobile_send(request, finish, cx),
            _ => {
                let result =
                    self.mobile_session_command(&request.method, &request.params, window, cx);
                finish(self, request, result);
            }
        }
    }

    pub(super) fn mobile_session_command(
        &mut self,
        method: &str,
        params: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Value, String> {
        let conversation = string(params, "conversationId").unwrap_or_default();
        let target = self.mobile_target(&conversation, cx)?;
        let reply = json!({"sessionId": conversation});
        match (method, &target) {
            ("session.cancel", Target::Session(id)) => {
                if let Some(view) = self.sessions.get(id) {
                    view.update(cx, |v, cx| v.stop_if_busy(cx));
                }
                Ok(reply)
            }
            ("session.cancel", Target::Mono(mono)) => {
                if let Some(chat) = self.monos.as_ref().and_then(|m| m.read(cx).open_chat(mono)) {
                    chat.update(cx, |v, cx| v.stop_if_busy(cx));
                }
                Ok(reply)
            }
            ("session.resumeUsage" | "session.resumeUsageAtReset", target) => {
                let automatic = if method == "session.resumeUsageAtReset" {
                    Some(
                        params["enabled"]
                            .as_bool()
                            .ok_or("Choose whether to resume at reset")?,
                    )
                } else {
                    None
                };
                let view = match target {
                    Target::Session(id) => self.ensure_session(*id, cx),
                    Target::Mono(mono) => self
                        .monos
                        .clone()
                        .and_then(|m| m.update(cx, |m, cx| m.mobile_chat(mono, cx))),
                }
                .ok_or("This conversation is still opening on desktop; try again")?;
                view.update(cx, |v, cx| {
                    v.mobile_resume_usage(&params["token"], automatic, cx)
                })?;
                Ok(reply)
            }
            ("session.configure", target) => {
                let view = match target {
                    Target::Session(id) => self.ensure_session(*id, cx),
                    Target::Mono(mono) => self
                        .monos
                        .clone()
                        .and_then(|m| m.update(cx, |m, cx| m.mobile_chat(mono, cx))),
                }
                .ok_or("This conversation is still opening on desktop; try again")?;
                let model = match (
                    string(params, "harness").as_deref().and_then(Kind::parse),
                    string(params, "model"),
                ) {
                    (Some(kind), Some(model)) if model.len() <= 256 => {
                        Some((kind, monocode_core::store::native_model(&model, kind.id())))
                    }
                    _ => None,
                };
                let settings = model_settings(params);
                let access = params["runtimeMode"]
                    .as_str()
                    .filter(|mode| {
                        crate::chat::conversation::mobile_access_modes().any(|(id, ..)| id == *mode)
                    })
                    .map(AccessMode::from_id);
                view.update(cx, |v, cx| {
                    v.load(cx);
                    v.mobile_configure(model, &settings, access, cx);
                });
                Ok(reply)
            }
            ("session.rename", Target::Session(id)) => {
                let title = params["title"]
                    .as_str()
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                if title.is_empty() || title.chars().count() > 200 {
                    return Err("Enter a title under 200 characters".into());
                }
                let id = *id;
                if let Some(summary) = self.session_summaries.get_mut(&id) {
                    summary.title = title.clone();
                }
                if let Some(view) = self.sessions.get(&id) {
                    view.update(cx, |v, cx| v.rename(title.clone(), cx));
                }
                self.store_write("rename the session", cx, move |s| s.title(id, &title));
                self.invalidate_chrome(cx);
                cx.notify();
                Ok(reply)
            }
            ("session.archive", Target::Session(id)) => {
                if !self.session_metadata(*id, cx).summary.archived {
                    self.toggle_archive(*id, cx);
                }
                Ok(reply)
            }
            ("session.delete", Target::Session(id)) => {
                self.delete_session(*id, window, cx);
                Ok(reply)
            }
            ("session.answerQuestion" | "session.keepQuestionOpen", target) => {
                let view = match target {
                    Target::Session(id) => self.sessions.get(id).cloned(),
                    Target::Mono(mono) => {
                        self.monos.as_ref().and_then(|m| m.read(cx).open_chat(mono))
                    }
                }
                .ok_or("This question was already answered")?;
                let request_id = params["requestId"]
                    .as_u64()
                    .ok_or("This question was already answered")?;
                // Keeping it open sends no reply, only holds a timed question.
                let answer = method == "session.answerQuestion";
                let answered = answer.then(|| &params["reply"]);
                view.update(cx, |v, cx| {
                    v.mobile_answer_question(request_id, answered, cx)
                })?;
                Ok(reply)
            }
            ("session.cut", Target::Session(id)) => {
                let (turn, cut, text) = mobile_cut_params(params)?;
                let view = self
                    .ensure_session(*id, cx)
                    .ok_or("Session is no longer available")?;
                view.update(cx, |v, cx| v.mobile_cut(turn, cut, text, cx))?;
                Ok(reply)
            }
            ("session.rename", Target::Mono(_)) => {
                Err("A Mono's conversation can't be renamed".into())
            }
            ("session.archive" | "session.delete", Target::Mono(_)) => {
                Err("This session can't be changed from the phone".into())
            }
            _ => Err("Unsupported mobile command".into()),
        }
    }

    pub(super) fn mobile_send(
        &mut self,
        request: Request,
        finish: impl FnOnce(&mut Shell, Request, Result<Value, String>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let conversation = string(&request.params, "conversationId").unwrap_or_default();
        let staged = (|| {
            let target = self.mobile_target(&conversation, cx)?;
            let text = message_text(&request.params)?;
            Ok::<_, String>((target, text))
        })();
        let (target, text) = match staged {
            Ok(staged) => staged,
            Err(error) => return finish(self, request, Err(error)),
        };
        let stage = self.stage_mobile_attachments(&request.params, cx);
        cx.spawn(async move |this, cx| {
            let attachments = stage.await;
            let _ = this.update(cx, |this, cx| {
                let result = attachments.and_then(|attachments| match &target {
                    Target::Session(id) => {
                        let view = this
                            .ensure_session(*id, cx)
                            .ok_or("Session is no longer available")?;
                        view.update(cx, |v, cx| v.mobile_submit(text, attachments, cx));
                        this.mobile.started.insert(*id);
                        Ok(())
                    }
                    Target::Mono(mono) => this
                        .monos
                        .clone()
                        .ok_or("Mono is no longer available")?
                        .update(cx, |m, cx| {
                            m.mobile_send(
                                mono,
                                text,
                                attachments
                                    .iter()
                                    .map(|p| p.to_string_lossy().into_owned())
                                    .collect(),
                                cx,
                            )
                        }),
                });
                let result = result.map(|()| json!({"sessionId": conversation}));
                this.schedule_mobile_push(cx);
                finish(this, request, result);
            });
        })
        .detach();
    }

    /// A new session from the phone, set up as a new desktop tab would be and
    /// to the phone's model sheet choice, then sent its first message.
    pub(super) fn mobile_start(
        &mut self,
        request: Request,
        finish: impl FnOnce(&mut Shell, Request, Result<Value, String>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let params = request.params.clone();
        let prepared = (|| {
            let text = message_text(&params)?;
            let cwd =
                self.mobile_project_path(&string(&params, "projectId").unwrap_or_default(), cx)?;
            let (default_kind, default_model, default_access) = self.mobile_defaults(&cwd, cx);
            let chosen = string(&params, "harness").as_deref().and_then(Kind::parse);
            let kind = chosen.unwrap_or(default_kind);
            let model = start_model(
                &self.workspace.preferences,
                &cwd,
                chosen,
                string(&params, "model"),
                default_model,
            );
            let access = params["runtimeMode"]
                .as_str()
                .filter(|mode| {
                    crate::chat::conversation::mobile_access_modes().any(|(id, ..)| id == *mode)
                })
                .map_or(default_access, AccessMode::from_id);
            let account = self.mobile_account(kind, &cwd, &params, cx)?;
            Ok::<_, String>((text, cwd, kind, model, access, account))
        })();
        let (text, cwd, kind, model, access, account) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => return finish(self, request, Err(error)),
        };
        let effort = crate::chat::conversation::preferred_effort(kind.id(), cx);
        let mut chosen_settings = model_settings(&params);
        let stage = self.stage_mobile_attachments(&params, cx);
        let store = self.store.clone();
        let work = cx.background_executor().spawn(async move {
            let attachments = stage.await?;
            let root = PathBuf::from(&cwd);
            let worktree = match params["worktreePath"].as_str() {
                Some(path) => Some(
                    project::resolve_root(&root, Some(path))?
                        .to_string_lossy()
                        .into_owned(),
                ),
                None => None,
            };
            let store = locked(&store)?;
            let summary = store.create(kind.id(), &cwd)?;
            let mut settings = sessions::loading::new_session_settings(
                store
                    .desktop_preferences()
                    .and_then(|preferences| preferences.session_defaults(kind.id(), &cwd)),
                model,
            );
            settings.access = access;
            settings.account = account;
            if let Some(effort) = chosen_settings.remove("effort").or(effort) {
                settings.effort = Some(effort);
            }
            if let Some(fast) = chosen_settings.remove("fast") {
                settings.fast = fast == "true";
            }
            if let Some(thinking) = chosen_settings.remove("thinking") {
                settings.thinking = Some(thinking == "true");
            }
            settings.model_settings.extend(chosen_settings);
            settings.worktree_cwd = worktree;
            // A new worktree off HEAD is made on the first send, as automations do.
            settings.new_worktree = params["workspaceMode"] == "worktree";
            store.save_agent_settings(summary.id, &settings)?;
            Ok::<_, String>((summary, settings, attachments))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                let result = result.map(|(summary, settings, attachments)| {
                    let id = summary.id;
                    this.mobile.started.insert(id);
                    this.add(summary, cx);
                    if let Some(view) = this.sessions.get(&id) {
                        view.update(cx, |v, cx| {
                            v.install_transfer(settings, None, cx);
                            v.mobile_submit(text, attachments, cx);
                        });
                    }
                    this.order.retain(|s| *s != id);
                    this.order.insert(0, id);
                    this.invalidate_chrome(cx);
                    this.persist(cx);
                    cx.notify();
                    json!({"sessionId": this.mobile_session_id(id)})
                });
                this.publish_mobile_catalog(cx);
                finish(this, request, result);
            });
        })
        .detach();
    }

    /// Files a phone sent with a message, as `{ name, mimeType, data }` in
    /// base64, written where pasted files go and checked as attachments are.
    pub(super) fn stage_mobile_attachments(
        &self,
        params: &Value,
        cx: &mut Context<Self>,
    ) -> Task<Result<Vec<PathBuf>, String>> {
        let value = params["attachments"].clone();
        let store = self.store.clone();
        cx.background_executor().spawn(async move {
            if value.is_null() {
                return Ok(Vec::new());
            }
            let items = value
                .as_array()
                .filter(|items| items.len() <= MAX_ATTACHMENTS)
                .ok_or(format!("Attach up to {MAX_ATTACHMENTS} files"))?;
            if items.is_empty() {
                return Ok(Vec::new());
            }
            let root = locked(&store)?
                .data_dir()
                .map(Path::to_path_buf)
                .unwrap_or_else(std::env::temp_dir)
                .join("attachments");
            std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
            let mut paths = Vec::new();
            for item in items {
                let name = item["name"]
                    .as_str()
                    .map(str::trim)
                    .filter(|name| !name.is_empty() && name.len() <= 255)
                    .ok_or("An attachment is missing its name")?;
                let data = item["data"]
                    .as_str()
                    .ok_or_else(|| format!("{name} has no contents"))?;
                use base64::Engine;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .map_err(|_| format!("{name} could not be read"))?;
                // Only the file name survives; a phone never picks the folder.
                let safe: String = Path::new(name)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "attachment".into())
                    .chars()
                    .map(|c| {
                        if c.is_control() || c == '/' || c == '\\' {
                            '_'
                        } else {
                            c
                        }
                    })
                    .collect();
                let path = root.join(format!("mobile-{}-{safe}", monocode_core::mono::new_id()));
                std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
                paths.push(path);
            }
            monocode_core::session::composer::validate_attachments(&paths)?;
            Ok(paths)
        })
    }
}

/// See `Shell::mobile_older_page`. Before a page's first turn it reads the
/// page before that one.
pub(super) fn older_page(
    store: &monocode_core::store::Store,
    session: i64,
    mut cursor: i64,
    before: Option<usize>,
    limit: usize,
    mut look: PageLook,
) -> Result<Value, String> {
    if look.harness.is_empty()
        && let Some(saved) = store.sessions()?.into_iter().find(|s| s.id == session)
    {
        look.harness = saved.provider;
        look.cwd = saved.cwd;
    }
    let mut page = store.load_before(session, cursor, false)?;
    let mut before = before.map(|row| format!("t{row}"));
    loop {
        // The page's turns, or None when the turn asked for is its first.
        let turns = {
            let snapshot = Snapshot {
                rows: &page.rows,
                turns: &page.turns,
                cwd: &look.cwd,
                harness: &look.harness,
                model: look.model.clone(),
                busy: false,
                waiting: None,
                mono: look.mono.clone(),
                pending: Vec::new(),
                launches: &look.launches,
                question: Value::Null,
                history: page.before,
                usage_limit: Value::Null,
            };
            let source = snapshot.source();
            let ids = turns::turn_ids(&source);
            let end = match &before {
                Some(id) => ids
                    .iter()
                    .position(|turn| turn == id)
                    .ok_or("This part of the conversation is no longer available")?,
                None => ids.len(),
            };
            let start = end.saturating_sub(limit);
            (end > 0).then(|| (start, turns::build(&source, start..end)))
        };
        let Some((start, mut built)) = turns else {
            // Before this page's first turn: the page above it, if any.
            let Some(above) = page.before else {
                return Ok(json!({"turns": [], "older": false}));
            };
            cursor = above;
            page = store.load_before(session, cursor, false)?;
            before = None;
            continue;
        };
        for turn in &mut built {
            let id = turn["id"].as_str().map(|id| format!("p{cursor}{id}"));
            if let Some(id) = id {
                turn["id"] = json!(id);
            }
        }
        return Ok(json!({"turns": built, "older": start > 0 || page.before.is_some()}));
    }
}

pub(super) fn saved_snapshot(saved: &Saved) -> Snapshot<'_> {
    Snapshot {
        usage_limit: crate::chat::conversation::mobile_usage_state(
            saved.id,
            &saved.harness,
            saved.settings.model.as_deref(),
            &saved.settings,
            saved.settings.usage_limit.as_ref(),
            false,
            saved.cwd.starts_with("remote://"),
        ),
        rows: &saved.rows,
        turns: &saved.settings.turns,
        cwd: &saved.cwd,
        harness: &saved.harness,
        model: saved.settings.model.clone(),
        busy: false,
        waiting: None,
        mono: saved.mono.clone(),
        pending: Vec::new(),
        launches: &saved.launches,
        question: Value::Null,
        history: saved.history,
    }
}

/// The Claude session whose CLI log holds this conversation. A session shown
/// in the CLI keeps it there; the store only has what the chat last read.
pub(super) fn cli_binding<'a>(
    settings: &Settings,
    harness: &str,
    binding: Option<&'a str>,
) -> Option<&'a str> {
    (settings.terminal && provider::Kind::from_id(harness) == provider::Kind::Claude)
        .then_some(binding)
        .flatten()
}

/// A phone's cut: the row its message starts on, read from the turn ID
/// (`t12` -> 12), what to do there, and an edit's new text. Turns of older
/// pages name their page and can't be cut from the phone.
pub(super) fn mobile_cut_params(
    params: &Value,
) -> Result<(usize, crate::chat::response_actions::Cut, Option<String>), String> {
    use crate::chat::response_actions::Cut;
    let turn = string(params, "turnId")
        .and_then(|turn| turn.strip_prefix('t')?.parse().ok())
        .ok_or("This message can't be changed from the phone")?;
    let cut = match params["cut"].as_str() {
        Some("fork") => Cut::Fork,
        Some("rewind") => Cut::Rewind,
        Some("edit") => Cut::Edit,
        _ => return Err("Unsupported mobile command".into()),
    };
    let text = string(params, "text").filter(|text| !text.trim().is_empty());
    if cut == Cut::Edit && text.is_none() {
        return Err("Type the new message first".into());
    }
    Ok((turn, cut, text))
}
