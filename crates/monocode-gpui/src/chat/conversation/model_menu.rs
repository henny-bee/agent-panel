use super::*;

impl Conversation {
    pub(crate) fn settings_models(&self) -> Vec<(String, String)> {
        self.catalog
            .iter()
            .map(|model| {
                (
                    model.id.clone(),
                    model_display_name(
                        Kind::from_id(&self.summary.provider),
                        &model.id,
                        &model.name,
                    )
                    .to_string(),
                )
            })
            .collect()
    }
}

impl Conversation {
    pub(super) fn refresh_catalog(&mut self, cx: &mut Context<Self>) {
        if cfg!(test) {
            return;
        }
        let kind = Kind::from_id(&self.summary.provider);
        self.attach_account(cx);
        self.catalog_epoch += 1;
        let epoch = self.catalog_epoch;
        let account = self.settings.account.clone();
        let work = cx.background_executor().spawn({
            let remote_project = self.is_remote().then(|| self.summary.cwd.clone());
            async move {
                if let Some(project) = remote_project {
                    monocode_core::provider::catalog::remote_models(&project, kind.id())
                } else {
                    catalog::models_for(kind, &account)
                }
            }
        });
        cx.spawn(async move |this, cx| {
            if let Ok(models) = work.await {
                let _ = this.update(cx, |this, cx| {
                    if Kind::from_id(&this.summary.provider) == kind && this.catalog_epoch == epoch
                    {
                        this.catalog = models;
                        if this.is_remote() && this.model.is_none() {
                            this.model = this
                                .catalog
                                .iter()
                                .find(|m| m.default)
                                .or_else(|| this.catalog.first())
                                .map(|m| m.id.clone());
                            this.settings.model = this.model.clone();
                            this.save_settings(cx);
                        }
                        cx.emit(SessionChanged);
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    pub(super) fn model_info(&self) -> Option<&Model> {
        match self.model.as_deref().or(self.resolved_model.as_deref()) {
            Some(id) => self.catalog.iter().find(|m| m.id == id),
            None => self
                .catalog
                .iter()
                .find(|m| m.default)
                .or_else(|| self.catalog.first()),
        }
    }

    /// A new tab shows the model and access it was created with from its
    /// first frame instead of flipping when the stored settings load.
    pub(crate) fn seed_new_settings(&mut self, settings: Settings, cx: &mut Context<Self>) {
        if self.loaded || self.settings.revision != 0 {
            return;
        }
        if settings.model.is_some() {
            self.model = settings.model.clone();
        }
        self.settings = settings;
        cx.notify();
    }

    pub(super) fn save_settings(&mut self, cx: &mut Context<Self>) {
        self.settings.revision = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let mut settings = self.settings.clone();
        settings.model = self.model.clone();
        let store = self.store.clone();
        let id = self.summary.id;
        let provider = self.summary.provider.clone();
        let remote = self.is_remote();
        let title = (!self.summary.has_messages).then(|| self.summary.title.clone());
        cx.background_executor()
            .spawn(async move {
                let saved = locked(&store).and_then(|s| {
                    s.configure_agent(id, &provider, &settings)?;
                    if let Some(title) = title
                        && s.agent_settings(id)?
                            .is_some_and(|saved| saved.revision == settings.revision)
                    {
                        s.title(id, &title)?;
                    }
                    Ok(())
                });
                if saved.is_ok() && remote {
                    let _ = crate::runtime::remote::configure_saved(store, id);
                }
            })
            .detach();
    }

    pub(super) fn menu_choice(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.git_action_choice(id, cx) {
            return;
        }
        if let Some(choice) = id.strip_prefix("recent-model:") {
            if let Ok((provider, model)) = serde_json::from_str::<(String, Option<String>)>(choice)
                && let Some(kind) = Kind::parse(&provider)
            {
                let picked = ModelPicked(kind, model);
                if self.apply_model_pick(&picked, cx) {
                    remember_model(kind, picked.1, cx);
                }
            }
            return;
        }
        if let Some(id) = id.strip_prefix("usage-account:") {
            if let Some(center) = crate::accounts::get(cx) {
                let kind = Kind::from_id(&self.summary.provider);
                if center.read(cx).book.exists(kind, id) {
                    let profile = center.read(cx).profile(id);
                    self.switch_account(profile, cx);
                }
            }
            return;
        }
        if std::env::var_os("MONOCODE_NATIVE_TRACE").is_some() {
            eprintln!("native settings session={} action={id}", self.summary.id);
        }
        if id == "mode:plan" {
            self.toggle_mode("plan", cx);
            return;
        }
        if let Some(mode) = id.strip_prefix("access:") {
            self.settings.access = AccessMode::from_id(mode);
            // Picking an access mode leaves a plan turn, chip and all.
            if self.settings.plan {
                self.toggle_mode("plan", cx);
            }
            remember_access(self.settings.access, cx);
        } else if let Some(effort) = id.strip_prefix("effort:") {
            self.set_effort_choice(effort, cx);
        } else if let Some(tier) = id.strip_prefix("tier:") {
            self.settings.service_tier = (!tier.is_empty()).then(|| tier.to_owned());
            self.settings.model_settings.remove("serviceTier");
        } else if let Some(setting) = id.strip_prefix("model-setting:") {
            if let Ok((key, value)) = serde_json::from_str::<(String, String)>(setting) {
                if is_effort_key(&key)
                    && (!self.is_remote() || matches!(key.as_str(), "effort" | "reasoningEffort"))
                {
                    self.set_effort_choice(&value, cx);
                }
                self.settings.model_settings.insert(key, value);
            }
        } else if let Some(key) = id.strip_prefix("model-toggle:") {
            let current = self
                .settings
                .model_settings
                .get(key)
                .map(String::as_str)
                .or_else(|| {
                    self.model_info()
                        .and_then(|model| model.controls.iter().find(|c| c.id == key))
                        .map(|c| c.current.as_str())
                });
            let value = if current == Some("true") {
                "false"
            } else {
                "true"
            };
            self.settings
                .model_settings
                .insert(key.to_owned(), value.to_owned());
        } else {
            match id {
                "workspace:current" => {
                    self.settings.worktree_removed = false;
                    self.settings.new_worktree = false;
                    self.settings.worktree_cwd = None;
                    self.refresh_git(cx);
                }
                "workspace:new" => {
                    self.settings.worktree_removed = false;
                    self.settings.new_worktree = true;
                    self.settings.worktree_cwd = None;
                    self.settings.worktree_base =
                        self.settings.worktree_base.clone().or(self.branch.clone());
                    self.refresh_git(cx);
                }
                "fast" => self.settings.fast = !self.settings.fast,
                "fast:on" => self.settings.fast = true,
                "fast:off" => self.settings.fast = false,
                "thinking" => {
                    self.settings.thinking = Some(!self.settings.thinking.unwrap_or(false))
                }
                "plan" | "operator" | "orchestrator" | "draft" => {
                    self.toggle_mode(id, cx);
                    return;
                }
                _ => return,
            }
        }
        self.save_settings(cx);
        cx.notify();
    }

    pub(super) fn set_effort_choice(&mut self, effort: &str, cx: &mut Context<Self>) {
        self.settings
            .model_settings
            .retain(|key, _| !is_effort_key(key));
        self.settings.effort = (!effort.is_empty()).then(|| effort.to_owned());
        let provider = self.summary.provider.clone();
        if cx.has_global::<crate::ui::preferences::Preferences>() {
            cx.update_global::<crate::ui::preferences::Preferences, _>(|prefs, _| {
                if effort.is_empty() {
                    prefs.provider_efforts.remove(&provider);
                } else {
                    prefs.provider_efforts.insert(provider, effort.to_owned());
                }
            });
        }
    }

    pub(super) fn selected_effort(&self) -> Option<&str> {
        self.settings
            .effort
            .as_deref()
            .or_else(|| {
                [
                    "effort",
                    "reasoning",
                    "reasoningEffort",
                    "reasoning_effort",
                    "variant",
                ]
                .into_iter()
                .find_map(|key| self.settings.model_settings.get(key).map(String::as_str))
            })
            .or_else(|| self.model_info().and_then(|m| m.default_effort.as_deref()))
            .or_else(|| {
                self.model_info()
                    .and_then(|m| m.efforts.first().map(String::as_str))
            })
    }

    pub(super) fn effort_label(&self) -> SharedString {
        effort_name(self.selected_effort().unwrap_or("default")).into()
    }

    pub(super) fn effort_setting_label(&self) -> &'static str {
        if Kind::from_id(&self.summary.provider) == Kind::Codex {
            "Reasoning"
        } else {
            "Effort"
        }
    }

    pub(super) fn selected_tier(&self) -> Option<&str> {
        self.settings
            .service_tier
            .as_deref()
            .or_else(|| {
                self.settings
                    .model_settings
                    .get("serviceTier")
                    .map(String::as_str)
            })
            .filter(|value| !value.is_empty() && *value != "default")
    }

    /// The model trigger's fast-mode bolt: shown while fast mode is on,
    /// unless beside mode already has its own Fast pill.
    pub(super) fn trigger_fast(&self, beside: bool) -> bool {
        self.model_info().is_some_and(|model| {
            model.fast
                && self.settings.fast
                && !(beside && model.efforts.is_empty() && model.tiers.is_empty())
        })
    }

    /// Combined ModelPicker trigger; beside mode renders these as separate pills.
    pub(super) fn trigger_effort_label(&self) -> Option<SharedString> {
        if let Some(model) = self.model_info() {
            if let Some(control) = model.controls.iter().find(|control| {
                !model_control_toggle(control)
                    && matches!(
                        control.id.as_str(),
                        "effort"
                            | "reasoning"
                            | "reasoningEffort"
                            | "reasoning_effort"
                            | "thinking"
                            | "variant"
                    )
            }) {
                let selected = self
                    .settings
                    .model_settings
                    .get(&control.id)
                    .unwrap_or(&control.current);
                return Some(
                    control
                        .choices
                        .iter()
                        .find(|(value, _)| value == selected)
                        .map(|(_, label)| label)
                        .unwrap_or(selected)
                        .clone()
                        .into(),
                );
            }
            return (!model.efforts.is_empty()).then(|| self.effort_label());
        }
        // Keep the saved effort visible while the live catalog is loading.
        self.settings.effort.as_ref().map(|_| self.effort_label())
    }

    pub(super) fn effort_items(&self) -> Vec<MenuAction> {
        self.model_info().map_or_else(Vec::new, |model| {
            model
                .efforts
                .iter()
                .map(|effort| {
                    MenuAction::new(format!("effort:{effort}"), effort_name(effort))
                        .checked(self.selected_effort() == Some(effort.as_str()))
                })
                .collect()
        })
    }

    /// The pill's label, which drops the catalog names' "Claude " prefix.
    pub(super) fn model_label(&self) -> SharedString {
        let name = self
            .model_info()
            .map(|m| model_display_name(Kind::from_id(&self.summary.provider), &m.id, &m.name))
            .unwrap_or_else(|| {
                model_name(
                    Kind::from_id(&self.summary.provider),
                    self.model.as_deref().or(self.resolved_model.as_deref()),
                )
            });
        if Kind::from_id(&self.summary.provider) == Kind::Claude {
            name.strip_prefix("Claude ")
                .map_or(name.clone(), |short| short.to_owned().into())
        } else {
            name
        }
    }

    /// A different provider is armed for the next submission. The running
    /// provider retains its controls, account and immutable turn identity.
    pub(crate) fn apply_model_pick(
        &mut self,
        picked: &ModelPicked,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.editing_last_turn.pending() || self.status == "Preparing handoff…" {
            return false;
        }
        let changed_model = self.summary.provider != picked.0.id() || self.model != picked.1;
        if std::env::var_os("MONOCODE_NATIVE_TRACE").is_some() {
            eprintln!(
                "native model session={} provider={} model={:?}",
                self.summary.id,
                picked.0.id(),
                picked.1
            );
        }
        if self.summary.provider != picked.0.id() {
            let reverting = self
                .settings
                .pending_provider_switch
                .as_ref()
                .is_some_and(|pending| pending.from == picked.0.id());
            let restored = reverting
                .then(|| self.settings.pending_provider_switch.clone())
                .flatten();
            let changed = locked(&self.store).and_then(|store| {
                store.change_provider(self.summary.id, picked.0.id())?;
                if let Some(id) = restored
                    .as_ref()
                    .and_then(|pending| pending.provider_id.as_deref())
                {
                    store.provider_id(self.summary.id, id)?;
                }
                Ok(())
            });
            if let Err(error) = changed {
                self.status = error;
                cx.notify();
                return false;
            }
            if let Some(pending) = restored {
                self.settings.account = pending.account;
                self.summary.provider_id = pending.provider_id;
                self.settings.pending_provider_switch = None;
                self.settings.handoff = None;
            } else {
                let has_history = self.busy
                    || self
                        .transcript
                        .read(cx)
                        .source_rows()
                        .iter()
                        .any(|row| row.is_prompt());
                if has_history && self.settings.pending_provider_switch.is_none() {
                    self.settings.pending_provider_switch =
                        Some(monocode_core::session::agent::ProviderSwitch {
                            from: self.summary.provider.clone(),
                            model_label: self.model_label().to_string(),
                            cwd: self.execution_cwd().to_owned(),
                            model: self.model.clone(),
                            model_settings: self.settings.model_settings.clone(),
                            effort: self.settings.effort.clone(),
                            fast: self.settings.fast,
                            service_tier: self.settings.service_tier.clone(),
                            thinking: self.settings.thinking,
                            skip_outgoing_recap: self.settings.usage_limit.is_some(),
                            account: self.settings.account.clone(),
                            provider_id: self.summary.provider_id.clone(),
                        });
                }
                self.settings.account = crate::accounts::get(cx)
                    .map(|center| center.read(cx).selected(picked.0, &self.summary.cwd))
                    .unwrap_or_default();
                self.summary.provider_id = None;
                if let Some(pending) = &self.settings.pending_provider_switch {
                    self.settings.handoff = Some(monocode_core::session::agent::Handoff {
                        from: pending.from.clone(),
                        brief: monocode_core::session::response::handoff(
                            self.transcript.read(cx).source_rows(),
                        ),
                    });
                }
            }
            self.summary.provider = picked.0.id().into();
            self.completion.update(cx, |completion, cx| {
                completion.set_provider(picked.0.id(), cx)
            });
            self.catalog.clear();
            if !self.summary.has_messages {
                self.summary.title = format!("New {} session", picked.0.label());
            }
        }
        self.model = picked.1.clone();
        if let Some(context) = &mut self.context {
            context.window = None;
        }
        let store = self.store.clone();
        let id = self.summary.id;
        let context = self.context;
        cx.background_executor()
            .spawn(async move {
                if let Some(context) = context {
                    let _ = locked(&store).and_then(|s| s.save_context_usage(id, &context));
                }
            })
            .detach();
        self.resolved_model = None;
        self.settings.effort = preferred_effort(&self.summary.provider, cx);
        self.settings.fast = false;
        self.settings.thinking = None;
        self.settings.service_tier = None;
        self.settings.model_settings.clear();
        if changed_model && self.settings.usage_limit.is_some() && !self.busy {
            self.recover_usage_after_model_change(cx);
        }
        self.refresh_catalog(cx);
        self.save_settings(cx);
        cx.emit(SessionChanged);
        true
    }

    pub(super) fn menu_is(&self, kind: ComposerMenu) -> bool {
        self.menu.as_ref().is_some_and(|open| open.kind == kind)
    }

    pub(super) fn anchor(&self, kind: ComposerMenu) -> &AnchorSlot {
        match kind {
            ComposerMenu::Add => &self.add_anchor,
            ComposerMenu::Model | ComposerMenu::Recent => &self.model_anchor,
            ComposerMenu::Access => &self.access_anchor,
            ComposerMenu::Account => &self.account_anchor,
            ComposerMenu::Workspace => &self.workspace_anchor,
            ComposerMenu::GitActions => &self.git_action_anchor,
            ComposerMenu::Setting(ix) => self
                .mono_ui
                .setting_anchors
                .get(ix)
                .unwrap_or(&self.model_anchor),
        }
    }

    pub(super) fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        window.focus(&self.input.focus_handle(cx), cx);
        cx.notify();
    }

    pub(super) fn toggle_menu(
        &mut self,
        kind: ComposerMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if kind == ComposerMenu::Model
            && cx
                .try_global::<crate::ui::preferences::Preferences>()
                .is_some_and(|p| p.model_controls == "beside")
        {
            self.toggle_mono_models(window, cx);
            return;
        }
        self.mono_ui.models = None;
        if self.menu.as_ref().is_some_and(|open| open.kind == kind) {
            self.close_menu(window, cx);
            return;
        }
        // Another menu gives way at once rather than overlapping this one.
        self.menu = None;
        self.completion
            .update(cx, |completion, cx| completion.dismiss(cx));
        self.git_picker = None;
        self.git_events = None;
        let mut events = Vec::new();
        let (label, items, width, style) = self.menu_items(kind, &mut events, window, cx);
        // The recent models open on the current one.
        let current = (kind == ComposerMenu::Recent)
            .then(|| {
                items.iter().find_map(|item| match item {
                    MenuItem::Action(action) if action.checked => Some(action.id.clone()),
                    _ => None,
                })
            })
            .flatten();
        let menu = cx.new(|cx| {
            let menu = Menu::new(label, items, window, cx)
                .width(width)
                .style(style);
            match current {
                Some(id) => menu.select_value(&id),
                None => menu,
            }
        });
        events.push(
            cx.subscribe_in(&menu, window, |this, menu, event: &MenuEvent, _, cx| {
                if let MenuEvent::Picked(id, _) = event {
                    let toggle = menu.read(cx).toggle_state(id);
                    if id.as_ref() == "upload" {
                        this.choose_attachments(cx);
                    } else {
                        this.menu_choice(id, cx);
                    }
                    if let Some(on) = toggle {
                        menu.update(cx, |menu, cx| menu.set_toggle(id, !on, cx));
                        cx.notify();
                        return;
                    }
                }
                this.menu = None;
                cx.notify();
            }),
        );
        self.menu = Some(OpenMenu {
            kind,
            menu,
            _events: events,
        });
        cx.notify();
    }

    pub(super) fn menu_items(
        &self,
        kind: ComposerMenu,
        events: &mut Vec<Subscription>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (&'static str, Vec<MenuItem>, f32, MenuStyle) {
        let provider = Kind::from_id(&self.summary.provider);
        match kind {
            ComposerMenu::Account => {
                let items = crate::accounts::get(cx)
                    .map(|center| {
                        center
                            .read(cx)
                            .book
                            .list(provider)
                            .into_iter()
                            .map(|account| {
                                MenuAction::new(
                                    format!("usage-account:{}", account.id),
                                    account.label,
                                )
                                .checked(account.id == self.settings.account.id)
                                .into()
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                ("Switch account", items, 250., MenuStyle::Explorer)
            }
            ComposerMenu::Workspace => {
                let picker = cx.new(|cx| {
                    GitPicker::new(
                        self.summary.cwd.clone().into(),
                        GitMode::Worktree,
                        Some(self.execution_cwd().to_owned()),
                        cx,
                    )
                    .existing_worktrees()
                });
                events.push(cx.subscribe_in(
                    &picker,
                    window,
                    |this, _, picked: &GitPicked, window, cx| match picked {
                        GitPicked::Worktree(path) => {
                            if this.use_existing_worktree(path, cx) {
                                this.close_menu(window, cx);
                            }
                        }
                        GitPicked::Dismissed => this.close_menu(window, cx),
                        _ => {}
                    },
                ));
                let focus = picker.read(cx).focus(cx);
                (
                    "Workspace",
                    vec![
                        MenuItem::Header("Workspace".into()),
                        MenuAction::new("workspace:current", "Current checkout")
                            .icon("folder")
                            .checked(
                                !self.settings.new_worktree && self.settings.worktree_cwd.is_none(),
                            )
                            .into(),
                        MenuAction::new("workspace:new", "New worktree")
                            .icon("folder-tree")
                            .checked(self.settings.new_worktree)
                            .into(),
                        MenuItem::Separator,
                        MenuAction::new("workspace:existing", "Existing worktrees")
                            .icon("folder-tree")
                            .flyout(Flyout {
                                view: picker.into(),
                                width: 300.,
                                padding: 0.,
                                height: None,
                                max_height: Some(320.),
                                focus: Some(focus),
                                surface: None,
                            })
                            .into(),
                    ],
                    240.,
                    MenuStyle::Settings,
                )
            }
            // Composer.tsx's plus menu with native mode and attachment actions.
            ComposerMenu::Add => {
                let mut items = vec![MenuItem::Header("Add to message".into())];
                items.extend(
                    [
                        MenuAction::new("upload", "Upload file")
                            .icon("file-add")
                            .description("Attach files or images"),
                        MenuAction::new("plan", "Plan mode")
                            .icon("ai-idea")
                            .icon_color(tint(YELLOW_300, 0.8))
                            .description("Review a plan before building"),
                        MenuAction::new("operator", "Operator")
                            .disabled(self.is_remote())
                            .icon("cursor-magic")
                            .icon_color(tint(SKY_300, 0.8))
                            .description("Give this thread access to MCode"),
                        MenuAction::new("orchestrator", "Orchestrator")
                            .disabled(self.is_remote())
                            .icon("share")
                            .icon_color(tint(FUCHSIA_300, 0.65))
                            .badge("v1")
                            .description("Plan and coordinate agent work"),
                        MenuAction::new("draft", "Draft")
                            .icon("circle-dashed-hi")
                            .description("Save this message without starting the agent")
                            .disabled(self.busy || self.follow_ups.state.draft.is_some()),
                    ]
                    .into_iter()
                    .map(|action| {
                        let checked = match action.id.as_ref() {
                            "plan" => self.settings.plan,
                            "draft" => self.draft_active(cx),
                            "operator" => self.settings.operator,
                            "orchestrator" => self.settings.orchestrator,
                            _ => false,
                        };
                        action.checked(checked).into()
                    }),
                );
                ("Add to message", items, 250., MenuStyle::Composer)
            }
            ComposerMenu::GitActions => (
                "Git actions",
                self.git_action_menu_items(cx),
                250.,
                MenuStyle::Composer,
            ),
            // ModelPicker: a "Model" row whose flyout lists the models.
            ComposerMenu::Model => {
                let list = cx.new(|cx| {
                    let hidden = cx
                        .try_global::<crate::ui::preferences::Preferences>()
                        .map(|p| p.hidden_for(&self.summary.cwd))
                        .unwrap_or_default();
                    ModelList::new_for(
                        provider,
                        self.model.clone(),
                        &self.summary.cwd,
                        self.settings.account.clone(),
                        cx,
                    )
                    .hide_providers(hidden)
                    .allow_provider_switch(true)
                });
                events.push(cx.subscribe_in(
                    &list,
                    window,
                    |this, _, picked: &ModelPicked, window, cx| {
                        if this.apply_model_pick(picked, cx) {
                            this.close_menu(window, cx);
                        }
                    },
                ));
                let focus = list.read(cx).search_focus(cx);
                let mut items = if cx
                    .try_global::<crate::ui::preferences::Preferences>()
                    .is_none_or(|p| p.model_controls != "beside")
                {
                    self.mono_model_options()
                } else {
                    Vec::new()
                };
                items.push(
                    MenuAction::new("model", "Model")
                        .value_image(provider.id())
                        .value(self.model_info().map_or_else(
                            || model_name(provider, self.model.as_deref()),
                            |model| model_display_name(provider, &model.id, &model.name),
                        ))
                        .flyout(Flyout {
                            view: list.into(),
                            width: 310.,
                            padding: 0.,
                            height: Some(442.),
                            max_height: None,
                            focus: Some(focus),
                            surface: None,
                        })
                        .into(),
                );
                ("Model and settings", items, 250., MenuStyle::Settings)
            }
            ComposerMenu::Recent => (
                "Recently used models",
                self.recent_model_items(cx),
                250.,
                MenuStyle::Recent,
            ),
            ComposerMenu::Access => {
                let items = self.mono_access_options();
                ("Access", items, 288., MenuStyle::Access)
            }
            ComposerMenu::Setting(ix) => self.mono_setting_menu(ix),
        }
    }

    /// The model trigger: the composer's model menu.
    #[cfg(test)]
    pub(crate) fn toggle_model_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.mono.is_some() {
            self.toggle_mono_models(window, cx);
            return;
        }
        self.toggle_menu(ComposerMenu::Model, window, cx);
    }

    /// `App: Switch Model` (Cmd+.): toggles the recent models, as ModelPicker's
    /// hotkey does.
    pub(crate) fn toggle_recent_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mono_ui.models = None;
        self.toggle_menu(ComposerMenu::Recent, window, cx);
    }

    /// Right-clicking the model trigger opens the recent models over any menu.
    pub(super) fn open_recent_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.menu_is(ComposerMenu::Recent) {
            self.toggle_recent_models(window, cx);
        }
    }

    /// recentMenuModels: up to six remembered choices, with the current model
    /// added when it is not among them.
    pub(super) fn recent_model_items(&self, cx: &gpui::App) -> Vec<MenuItem> {
        let provider = Kind::from_id(&self.summary.provider);
        let current = (provider, self.model.clone());
        let hidden = cx
            .try_global::<crate::ui::preferences::Preferences>()
            .map(|p| p.hidden_for(&self.summary.cwd))
            .unwrap_or_default();
        let mut choices: Vec<(Kind, Option<String>)> = cx
            .try_global::<crate::ui::preferences::Preferences>()
            .into_iter()
            .flat_map(|p| &p.recent_models)
            .filter_map(|choice| {
                let kind = Kind::parse(&choice.provider)?;
                choice.model.as_ref()?;
                (kind == provider || !hidden.iter().any(|id| id == kind.id()))
                    .then(|| (kind, choice.model.clone()))
            })
            .collect();
        choices.dedup();
        if !choices.contains(&current) {
            choices.push(current.clone());
        }
        choices.truncate(6);
        choices
            .into_iter()
            .map(|(kind, model)| {
                let selected = (kind, model.clone()) == current;
                let name = if selected {
                    self.model_label()
                } else {
                    let name = cached_model_name(kind, model.as_deref());
                    match name.strip_prefix("Claude ") {
                        Some(short) if kind == Kind::Claude => short.to_owned().into(),
                        _ => name,
                    }
                };
                let id = serde_json::to_string(&(kind.id(), &model)).unwrap_or_default();
                MenuAction::new(format!("recent-model:{id}"), name)
                    .image(kind.id())
                    .description(kind.label())
                    .checked(selected)
                    .into()
            })
            .collect()
    }
}
