use super::*;

impl Shell {
    pub(crate) fn init_mobile_host(&mut self, cx: &mut Context<Self>) {
        // GPUI's deterministic executor cannot be woken by the link's threads.
        if cx.is_test() {
            return;
        }
        let Some(path) = locked(&self.store)
            .ok()
            .and_then(|store| store.data_dir().map(|dir| dir.join("mobile-access.json")))
        else {
            return;
        };
        let (host, events) = match MobileHost::open(path) {
            Ok(opened) => opened,
            Err(error) => {
                self.error = Some(format!("Mobile access: {error}"));
                return;
            }
        };
        self.mobile.host = Some(host);
        // Close the link and release the router mapping with the app.
        cx.on_app_quit(|this, _| {
            this.shutdown_mobile_host();
            async {}
        })
        .detach();
        cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                if this
                    .update_in(cx, |this, window, cx| {
                        this.handle_mobile_event(event, window, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            loop {
                if this
                    .update(cx, |this, cx| this.publish_mobile_catalog(cx))
                    .is_err()
                {
                    break;
                }
                cx.background_executor().timer(PUBLISH_INTERVAL).await;
            }
        })
        .detach();
    }

    pub(crate) fn shutdown_mobile_host(&mut self) {
        if let Some(host) = &self.mobile.host {
            host.shutdown();
        }
    }

    pub(crate) fn mobile_status(&self) -> Option<Status> {
        self.mobile.host.as_ref().map(MobileHost::status)
    }

    pub(crate) fn mobile_remember_firewall(&mut self, open: bool, cx: &mut Context<Self>) {
        if let Some(host) = &self.mobile.host
            && let Err(error) = host.remember_firewall(open)
        {
            self.error = Some(format!("Mobile access: {error}"));
        }
        self.sync_settings(cx);
    }

    pub(crate) fn mobile_configure(
        &mut self,
        enabled: bool,
        open_router: bool,
        remote_address: String,
        cx: &mut Context<Self>,
    ) {
        let Some(host) = self.mobile.host.clone() else {
            return;
        };
        if !enabled {
            self.mobile.invite = None;
            self.mobile.watches.clear();
        }
        // Binding the port and reading the keyring can block.
        let work = cx
            .background_executor()
            .spawn(async move { host.configure(enabled, open_router, &remote_address) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.error = Some(format!("Mobile access: {error}"));
                }
                this.publish_mobile_catalog(cx);
                this.sync_settings(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn mobile_invite(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self.mobile.host.clone() else {
            return;
        };
        let work = cx.background_executor().spawn(async move { host.invite() });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(invite) => this.mobile.invite = Some(invite),
                    Err(error) => this.error = Some(format!("Mobile access: {error}")),
                }
                this.sync_settings(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn mobile_cancel_invite(&mut self, cx: &mut Context<Self>) {
        if let Some(host) = &self.mobile.host {
            host.cancel_invite();
        }
        self.mobile.invite = None;
        self.sync_settings(cx);
    }

    pub(crate) fn mobile_revoke(&mut self, device_id: &str, cx: &mut Context<Self>) {
        if let Some(host) = &self.mobile.host
            && let Err(error) = host.revoke(device_id)
        {
            self.error = Some(format!("Mobile access: {error}"));
        }
        self.sync_settings(cx);
    }

    pub(super) fn handle_mobile_event(
        &mut self,
        event: Event,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            Event::Request(request) => self.handle_mobile_request(request, window, cx),
            Event::Watch(start) => self.start_mobile_watch(start, cx),
            Event::WatchStop(id) => {
                self.mobile.watches.remove(&id);
            }
        }
    }

    /// What a new session in `cwd` starts with, as a new desktop tab would.
    pub(super) fn mobile_defaults(
        &self,
        cwd: &str,
        cx: &App,
    ) -> (Kind, Option<String>, AccessMode) {
        let choice = self.new_chat_choice(cwd);
        let model = choice
            .model
            .and_then(|model| monocode_core::store::native_model(&model, choice.kind.id()));
        (
            choice.kind,
            model,
            crate::chat::conversation::default_access(cx),
        )
    }

    /// The account a new session uses: the phone's pick, checked against the
    /// final provider, else the project's selection.
    pub(super) fn mobile_account(
        &self,
        kind: Kind,
        cwd: &str,
        params: &Value,
        cx: &App,
    ) -> Result<monocode_core::session::profile::Profile, String> {
        let Some(center) = crate::accounts::get(cx) else {
            return Ok(Default::default());
        };
        let center = center.read(cx);
        let Some(id) = params.get("providerAccountId").filter(|v| !v.is_null()) else {
            return Ok(center.selected(kind, cwd));
        };
        if !monocode_core::provider::accounts::supports_profiles(kind) {
            return Err("This provider does not support account switching".into());
        }
        match id.as_str() {
            Some(id) if center.book.exists(kind, id) => Ok(center.profile(id)),
            _ => Err(
                "Account is no longer available. Choose another account in the model picker."
                    .into(),
            ),
        }
    }

    /// What the phone's model sheet offers: each provider the desktop picker
    /// shows, their models and settings, the permission modes, the accounts,
    /// and what the conversation or a new session in the project runs with.
    pub(super) fn mobile_models(&mut self, request: Request, cx: &mut Context<Self>) {
        let params = request.params.clone();
        let current = match string(&params, "conversationId") {
            Some(conversation) => self.mobile_target(&conversation, cx).and_then(|target| {
                let view = match &target {
                    Target::Session(id) => self.sessions.get(id).cloned(),
                    Target::Mono(mono) => {
                        self.monos.as_ref().and_then(|m| m.read(cx).open_chat(mono))
                    }
                };
                match view {
                    Some(view) => {
                        let v = view.read(cx);
                        Ok((
                            v.summary.cwd.clone(),
                            Kind::from_id(&v.summary.provider),
                            v.mobile_model().map(str::to_owned),
                            v.mobile_model_settings(),
                            v.settings.access,
                            Some(v.settings.account.id.clone()),
                        ))
                    }
                    None => match target {
                        Target::Session(id) => {
                            let summary = self
                                .session_summaries
                                .get(&id)
                                .ok_or("Session is no longer available")?;
                            let settings =
                                locked(&self.store)?.agent_settings(id)?.unwrap_or_default();
                            Ok((
                                summary.cwd.clone(),
                                Kind::from_id(&summary.provider),
                                settings.model.clone(),
                                mobile_settings_of(&settings),
                                settings.access,
                                Some(settings.account.id.clone()),
                            ))
                        }
                        // Not open here: what its saved conversation runs with.
                        Target::Mono(mono) => {
                            let id = self
                                .monos
                                .as_ref()
                                .and_then(|m| m.read(cx).roster.get(&mono)?.session_id)
                                .ok_or(
                                    "This conversation is still opening on desktop; try again",
                                )?;
                            let store = locked(&self.store)?;
                            let (cwd, provider) = match self.session_summaries.get(&id) {
                                Some(summary) => (summary.cwd.clone(), summary.provider.clone()),
                                None => store
                                    .sessions()?
                                    .into_iter()
                                    .find(|s| s.id == id)
                                    .map(|s| (s.cwd, s.provider))
                                    .ok_or("Session is no longer available")?,
                            };
                            let settings = store.agent_settings(id)?.unwrap_or_default();
                            Ok((
                                cwd,
                                Kind::from_id(&provider),
                                settings.model.clone(),
                                mobile_settings_of(&settings),
                                settings.access,
                                Some(settings.account.id.clone()),
                            ))
                        }
                    },
                }
            }),
            None => self
                .mobile_project_path(&string(&params, "projectId").unwrap_or_default(), cx)
                .map(|cwd| {
                    let (kind, model, access) = self.mobile_defaults(&cwd, cx);
                    let mut settings = HashMap::new();
                    if let Some(effort) = crate::chat::conversation::preferred_effort(kind.id(), cx)
                    {
                        settings.insert("effort".into(), effort);
                    }
                    (cwd, kind, model, settings, access, None)
                }),
        };
        let (cwd, kind, model, settings, access, account) = match current {
            Ok(current) => current,
            Err(error) => return request.reply(Err(error)),
        };
        let hidden = hidden_providers(&self.workspace.preferences, &cwd);
        let center = crate::accounts::get(cx).map(|c| c.read(cx));
        let account_kinds = [Kind::Claude, Kind::Codex];
        let accounts: Vec<Value> = center
            .map(|center| {
                account_kinds
                    .iter()
                    .flat_map(|kind| center.book.list(*kind))
                    .map(|a| {
                        let mut value =
                            json!({"id": a.id, "provider": a.provider, "label": a.label});
                        if a.id == monocode_core::session::profile::DEFAULT {
                            value["isDefault"] = json!(true);
                        }
                        value
                    })
                    .collect()
            })
            .unwrap_or_default();
        let selected: serde_json::Map<String, Value> = account_kinds
            .iter()
            .map(|k| {
                (
                    k.id().to_owned(),
                    json!(
                        center
                            .map(|c| c.book.selected(*k, &cwd))
                            .unwrap_or_default()
                    ),
                )
            })
            .collect();
        let profiles: Vec<(Kind, monocode_core::session::profile::Profile)> = Kind::ALL
            .into_iter()
            .map(|k| {
                let profile = center.map(|c| c.selected(k, &cwd)).unwrap_or_default();
                (k, profile)
            })
            .collect();
        let account =
            account.filter(|_| monocode_core::provider::accounts::supports_profiles(kind));
        let work = cx.background_executor().spawn(async move {
            let harnesses: Vec<Kind> = profiles
                .iter()
                .map(|(k, _)| *k)
                .filter(|k| !hidden.iter().any(|h| h == k.id()))
                .filter(|k| monocode_core::provider::resolve(*k).is_some())
                .collect();
            let mut current_model = mobile_catalog::model_id(kind, model.as_deref());
            let models: Vec<Value> = profiles
                .iter()
                .filter(|(k, _)| harnesses.contains(k))
                .flat_map(|(k, profile)| {
                    let rows = monocode_core::provider::catalog::models_for(*k, profile).unwrap_or_default();
                    if *k == kind {
                        current_model = mobile_catalog::current_model_id(kind, model.as_deref(), &rows);
                    }
                    rows
                        .into_iter()
                        .map(move |m| catalog_model(*k, m))
                })
                .collect();
            let mut current = json!({
                "harness": kind.id(),
                "model": current_model,
                "modelSettings": settings,
                "runtimeMode": access.id(),
            });
            if let Some(account) = account {
                current["providerAccountId"] = json!(account);
            }
            json!({
                "harnesses": harnesses.iter().map(|k| json!({"id": k.id(), "label": k.label()})).collect::<Vec<_>>(),
                "models": models,
                "runtimeModes": crate::chat::conversation::mobile_access_modes()
                    .map(|(id, label, hint)| json!({"id": id, "label": label, "hint": hint}))
                    .collect::<Vec<_>>(),
                "accounts": accounts,
                "selectedAccounts": selected,
                "current": current,
            })
        });
        cx.spawn(async move |_, _| request.reply(Ok(work.await)))
            .detach();
    }

    pub(super) fn mobile_usage(&mut self, request: Request, cx: &mut Context<Self>) {
        let kind = match request.params["provider"].as_str() {
            Some("claude") => Kind::Claude,
            Some("codex") => Kind::Codex,
            _ => {
                return request.reply(Err(
                    "Usage is only available for Claude Code and Codex accounts".into(),
                ));
            }
        };
        let Some(center) = crate::accounts::get(cx) else {
            return request.reply(Ok(json!({"accounts": {}})));
        };
        let refresh = request.params["refresh"] == true;
        let accounts: Vec<String> = center
            .read(cx)
            .book
            .list(kind)
            .into_iter()
            .map(|a| a.id)
            .collect();
        center.update(cx, |center, cx| {
            for id in &accounts {
                let key = crate::accounts::Key::new(kind, center.profile(id), None);
                center.refresh(key, refresh, cx);
            }
        });
        let show_remaining = self.workspace.preferences.show_remaining_usage;
        cx.spawn(async move |_, cx| {
            // Wait briefly for probes in flight, as the footer would.
            for _ in 0..50 {
                let fetching = cx
                    .update(|cx| {
                        let center = center.read(cx);
                        accounts.iter().any(|id| {
                            let key = crate::accounts::Key::new(kind, center.profile(id), None);
                            center.usage.get(&key).is_some_and(|state| state.fetching)
                        })
                    });
                if !fetching {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(200)).await;
            }
            let result = cx.update(|cx| {
                let center = center.read(cx);
                let now = monocode_core::provider::usage::now_ms();
                let snapshots: serde_json::Map<String, Value> = accounts
                    .iter()
                    .map(|id| {
                        let key = crate::accounts::Key::new(kind, center.profile(id), None);
                        let state = center.usage.get(&key);
                        let status = center.status(kind, id);
                        let snapshot = state.and_then(|s| s.snapshot.as_ref());
                        let meters: Vec<Value> = snapshot
                            .map(|s| s.windows.as_slice())
                            .unwrap_or_default()
                            .iter()
                            .filter_map(|window| {
                                let title = match window.minutes {
                                    300 => "5h",
                                    10_080 => "Weekly",
                                    43_200 => "Monthly",
                                    _ => return None,
                                };
                                let used = window.used_percent.clamp(0., 100.);
                                let percent = if show_remaining { 100. - used } else { used };
                                let reset = match window.resets_at {
                                    None => window.label(),
                                    Some(at) if at <= now => "reset due".into(),
                                    Some(at) => format!("Resets in {}", monocode_core::provider::usage::duration(at - now)),
                                };
                                Some(json!({
                                    "title": title,
                                    "usedPercent": used,
                                    "percent": percent,
                                    "label": format!("{percent:.0}% {}", if show_remaining { "left" } else { "used" }),
                                    "reset": reset,
                                }))
                            })
                            .collect();
                        (
                            id.clone(),
                            json!({
                                "status": {
                                    "tone": tone_id(status.tone),
                                    "label": status.label,
                                    "detail": status.detail,
                                },
                                "updatedAt": snapshot.map_or(0, |s| s.updated_at),
                                "stale": state.is_none_or(|s| s.fetching || s.error.is_some()),
                                "meters": meters,
                            }),
                        )
                    })
                    .collect();
                json!({"accounts": snapshots})
            });
            request.reply(Ok(result));
        })
        .detach();
    }
}
