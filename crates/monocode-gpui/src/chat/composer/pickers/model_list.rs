use super::*;

impl ModelList {
    pub(crate) fn new_for(
        provider: Kind,
        current: Option<String>,
        project: &str,
        profile: monocode_core::session::profile::Profile,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut profiles = crate::accounts::get(cx)
            .map(|center| {
                Kind::ALL
                    .into_iter()
                    .map(|kind| (kind, center.read(cx).selected(kind, project)))
                    .collect::<std::collections::HashMap<_, _>>()
            })
            .unwrap_or_default();
        profiles.insert(provider, profile);
        let search = search_field("Search models", "Search models", 0.4, cx);
        let changed = cx.subscribe(&search, |this, _, _: &input::Changed, cx| {
            this.highlighted = this.current_index(cx);
            cx.notify();
        });
        let mut list = Self {
            provider,
            remote_project: monocode_core::files::remote::parse_remote_path(project)
                .map(|_| project.to_owned()),
            remote_providers: None,
            current,
            profiles,
            catalogs: Vec::new(),
            errors: Vec::new(),
            loading: Vec::new(),
            missing: Vec::new(),
            switch_allowed: true,
            hidden: Vec::new(),
            quick: false,
            tab: cx
                .try_global::<crate::ui::preferences::Preferences>()
                .and_then(|prefs| prefs.model_picker_tab.as_deref())
                .map(|tab| {
                    if tab == "favorites" {
                        ModelTab::Favorites
                    } else {
                        ModelTab::Harness(Kind::parse(tab).unwrap_or(provider))
                    }
                })
                .unwrap_or(ModelTab::Harness(provider)),
            highlighted: 0,
            search,
            _changed: changed,
        };
        list.highlighted = list.current_index(cx);
        list.probe_installed(cx);
        list.discover(
            match list.tab {
                ModelTab::Harness(kind) => kind,
                ModelTab::Favorites => provider,
            },
            cx,
        );
        list
    }

    pub(crate) fn hide_providers(mut self, hidden: Vec<String>) -> Self {
        if self.remote_project.is_some() {
            return self;
        }
        if let ModelTab::Harness(kind) = self.tab
            && kind != self.provider
            && hidden.iter().any(|id| id == kind.id())
        {
            self.tab = ModelTab::Favorites;
        }
        self.hidden = hidden;
        self
    }

    /// The quick composer's inline selector: providers in a row of tabs,
    /// a favorite star on every model, and no hover-to-switch.
    pub(crate) fn quick(mut self, cx: &mut Context<Self>) -> Self {
        self.quick = true;
        self.search.update(cx, |search, cx| {
            search.set_placeholder("Search models…", cx)
        });
        self
    }

    /// Follow a pick made through this list, keeping it open.
    pub(crate) fn set_current(
        &mut self,
        kind: Kind,
        model: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.provider = kind;
        self.current = model;
        cx.notify();
    }

    /// The live catalog entry behind a choice, once discovery has run.
    pub(crate) fn catalog_model(&self, kind: Kind, id: Option<&str>) -> Option<&Model> {
        let models = &self.catalogs.iter().find(|(k, _)| *k == kind)?.1;
        match id {
            Some(id) => models.iter().find(|m| m.id == id),
            None => models.iter().find(|m| m.default).or(models.first()),
        }
    }

    pub(super) fn favorite(&self, kind: Kind, id: Option<&str>, cx: &gpui::App) -> bool {
        cx.try_global::<crate::ui::preferences::Preferences>()
            .is_some_and(|prefs| {
                prefs
                    .favorite_models
                    .iter()
                    .any(|c| c.provider == kind.id() && c.model.as_deref() == id)
            })
    }

    pub(super) fn toggle_favorite(
        &mut self,
        kind: Kind,
        id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if !cx.has_global::<crate::ui::preferences::Preferences>() {
            return;
        }
        cx.update_global::<crate::ui::preferences::Preferences, _>(|prefs, _| {
            let before = prefs.favorite_models.len();
            prefs
                .favorite_models
                .retain(|c| !(c.provider == kind.id() && c.model == id));
            if prefs.favorite_models.len() == before {
                prefs
                    .favorite_models
                    .push(crate::ui::preferences::ModelChoice {
                        provider: kind.id().into(),
                        model: id,
                    });
            }
        });
        cx.notify();
    }

    pub(crate) fn allow_provider_switch(mut self, allowed: bool) -> Self {
        self.switch_allowed = allowed;
        self
    }

    /// Like the Tauri picker, hide providers whose CLI isn't installed once the
    /// probe settles. The session's own provider always stays.
    pub(super) fn probe_installed(&mut self, cx: &mut Context<Self>) {
        if cfg!(test) || self.remote_project.is_some() {
            return;
        }
        let work = cx.background_executor().spawn(async move {
            Kind::ALL
                .into_iter()
                .filter(|kind| monocode_core::provider::resolve(*kind).is_none())
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let missing = work.await;
            let _ = this.update(cx, |this, cx| {
                for kind in missing {
                    if !this.missing.contains(&kind) {
                        this.missing.push(kind);
                    }
                }
                if let ModelTab::Harness(selected) = this.tab
                    && selected != this.provider
                    && this.missing.contains(&selected)
                {
                    this.tab = ModelTab::Harness(this.provider);
                }
                if this.query(cx).is_empty() {
                    this.highlighted = this.current_index(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn discover(&mut self, kind: Kind, cx: &mut Context<Self>) {
        if cfg!(test) || self.loading.contains(&kind) {
            return;
        }
        if let Some(project) = self.remote_project.clone() {
            self.loading = Kind::ALL.to_vec();
            let work = cx
                .background_executor()
                .spawn(async move { catalog::remote_catalog(&project) });
            cx.spawn(async move |this, cx| {
                let result = work.await;
                let _ = this.update(cx, |this, cx| {
                    this.loading.clear();
                    match result {
                        Ok(catalog) => {
                            this.missing = Kind::ALL
                                .into_iter()
                                .filter(|kind| !catalog.providers.contains(kind))
                                .collect();
                            this.remote_providers = Some(catalog.providers);
                            this.catalogs = catalog.models.into_iter().collect();
                            this.errors = catalog.errors.into_iter().collect();
                            if let ModelTab::Harness(selected) = this.tab
                                && this.missing.contains(&selected)
                                && selected != this.provider
                            {
                                this.tab = ModelTab::Harness(this.provider);
                            }
                        }
                        Err(error) => {
                            this.errors = Kind::ALL
                                .into_iter()
                                .map(|kind| (kind, error.clone()))
                                .collect();
                        }
                    }
                    this.highlighted = this.current_index(cx);
                    cx.notify();
                });
            })
            .detach();
            return;
        }
        self.loading.push(kind);
        self.errors.retain(|(k, _)| *k != kind);
        let work = cx.background_executor().spawn({
            let profile = self.profiles.get(&kind).cloned().unwrap_or_default();
            async move {
                let result = catalog::models_for(kind, &profile);
                // Missing is a fact about the binary, not about the message it
                // produced. Resolve on the worker so the UI never probes paths.
                let missing = result.is_err() && provider::resolve(kind).is_none();
                (result, missing)
            }
        });
        cx.spawn(async move |this, cx| {
            let (result, missing) = work.await;
            let _ = this.update(cx, |this, cx| {
                this.loading.retain(|k| *k != kind);
                match result {
                    Ok(models) => {
                        this.catalogs.retain(|(k, _)| *k != kind);
                        this.catalogs.push((kind, models));
                    }
                    Err(error) => this.errors.push((kind, error)),
                }
                this.missing.retain(|k| *k != kind);
                if missing {
                    this.missing.push(kind);
                }
                if this.query(cx).is_empty() {
                    this.highlighted = this.current_index(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn search_focus(&self, cx: &gpui::App) -> FocusHandle {
        self.search.focus_handle(cx)
    }

    pub(super) fn query(&self, cx: &gpui::App) -> String {
        self.search.read(cx).content.to_lowercase()
    }

    pub(super) fn entries(&self, cx: &gpui::App) -> Vec<ModelEntry> {
        let ModelTab::Harness(kind) = self.tab else {
            let query = self.query(cx);
            return cx
                .try_global::<crate::ui::preferences::Preferences>()
                .into_iter()
                .flat_map(|prefs| &prefs.favorite_models)
                .filter_map(|choice| {
                    let kind = Kind::parse(&choice.provider)?;
                    if kind != self.provider && self.hidden.iter().any(|id| id == kind.id()) {
                        return None;
                    }
                    let name = self.saved_model_name(kind, choice.model.as_deref());
                    let name: SharedString = format!("{} · {}", kind.label(), name).into();
                    if !query.is_empty() && !name.to_lowercase().contains(&query) {
                        return None;
                    }
                    Some(ModelEntry {
                        kind,
                        id: choice.model.clone(),
                        name,
                        enabled: self.enabled(kind),
                    })
                })
                .collect();
        };
        let enabled = self.enabled(kind);
        let mut entries = vec![ModelEntry {
            kind,
            id: None,
            name: "Default".into(),
            enabled,
        }];
        if let Some(prefs) = cx.try_global::<crate::ui::preferences::Preferences>() {
            for choice in &prefs.recent_models {
                if choice.provider == kind.id()
                    && choice.model.is_some()
                    && !entries.iter().any(|entry| entry.id == choice.model)
                {
                    entries.push(ModelEntry {
                        kind,
                        id: choice.model.clone(),
                        name: self.saved_model_name(kind, choice.model.as_deref()),
                        enabled,
                    });
                }
            }
        }
        if let Some((_, catalog)) = self.catalogs.iter().find(|(k, _)| *k == kind) {
            let additional: Vec<_> = catalog
                .iter()
                .filter(|model| {
                    model.id != "default"
                        && !entries
                            .iter()
                            .any(|entry| entry.id.as_deref() == Some(model.id.as_str()))
                })
                .map(|model| ModelEntry {
                    kind,
                    id: Some(model.id.clone()),
                    name: model_display_name(kind, &model.id, &model.name),
                    enabled,
                })
                .collect();
            entries.extend(additional);
        } else if kind == Kind::Claude && self.remote_project.is_none() {
            // Stable CLI aliases remain usable while discovery is loading.
            let additional: Vec<_> = [("sonnet", "Sonnet"), ("opus", "Opus"), ("haiku", "Haiku")]
                .into_iter()
                .filter(|(id, _)| !entries.iter().any(|entry| entry.id.as_deref() == Some(*id)))
                .map(|(id, name)| ModelEntry {
                    kind,
                    id: Some(id.into()),
                    name: name.into(),
                    enabled,
                })
                .collect();
            entries.extend(additional);
        }
        // An override from the environment that the catalog doesn't name.
        if kind == self.provider
            && let Some(id) = &self.current
            && !entries.iter().any(|e| e.id.as_ref() == Some(id))
        {
            entries.push(ModelEntry {
                kind,
                id: Some(id.clone()),
                name: model_name(kind, Some(id)),
                enabled,
            });
        }
        let query = self.query(cx);
        entries
            .into_iter()
            .filter(|e| {
                query.is_empty()
                    || format!("{} {}", e.name, kind.label())
                        .to_lowercase()
                        .contains(&query)
            })
            .collect()
    }

    pub(super) fn enabled(&self, kind: Kind) -> bool {
        (kind == self.provider || self.switch_allowed)
            && !self.missing.contains(&kind)
            && (self.remote_project.is_none()
                || self
                    .remote_providers
                    .as_ref()
                    .is_some_and(|providers| providers.contains(&kind)))
    }

    pub(super) fn saved_model_name(&self, kind: Kind, id: Option<&str>) -> SharedString {
        self.catalogs
            .iter()
            .find(|(provider, _)| *provider == kind)
            .and_then(|(_, models)| models.iter().find(|model| Some(model.id.as_str()) == id))
            .map(|model| model_display_name(kind, &model.id, &model.name))
            .unwrap_or_else(|| model_name(kind, id))
    }

    pub(super) fn current_index(&self, cx: &gpui::App) -> usize {
        self.entries(cx)
            .iter()
            .position(|entry| entry.kind == self.provider && entry.id == self.current)
            .unwrap_or(0)
    }

    pub(super) fn select_tab(
        &mut self,
        tab: ModelTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tab == tab {
            return;
        }
        self.tab = tab;
        if cx.has_global::<crate::ui::preferences::Preferences>() {
            cx.update_global::<crate::ui::preferences::Preferences, _>(|prefs, _| {
                prefs.model_picker_tab = Some(
                    match tab {
                        ModelTab::Favorites => "favorites",
                        ModelTab::Harness(kind) => kind.id(),
                    }
                    .into(),
                );
            });
        }
        if let ModelTab::Harness(kind) = tab
            && !self.catalogs.iter().any(|(k, _)| *k == kind)
            && !self.errors.iter().any(|(k, _)| *k == kind)
        {
            self.discover(kind, cx);
        }
        self.search.update(cx, |search, cx| {
            search.take(cx);
        });
        self.highlighted = self.current_index(cx);
        window.focus(&self.search.focus_handle(cx), cx);
        cx.notify();
    }

    pub(super) fn step(&mut self, direction: isize, cx: &mut Context<Self>) {
        let count = self.entries(cx).len();
        if count > 0 {
            self.highlighted =
                (self.highlighted as isize + direction).clamp(0, count as isize - 1) as usize;
            cx.notify();
        }
    }

    pub(super) fn pick(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(entry) = self.entries(cx).into_iter().nth(ix)
            && entry.enabled
        {
            remember_model(entry.kind, entry.id.clone(), cx);
            cx.emit(ModelPicked(entry.kind, entry.id));
        }
    }

    pub(super) fn tab(&self, tab: ModelTab, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.tab == tab;
        let (id, label): (&'static str, SharedString) = match tab {
            ModelTab::Favorites => ("model-tab-favorites", "Favorites".into()),
            ModelTab::Harness(kind) => (
                if kind == Kind::Claude {
                    "model-tab-claude"
                } else if kind == Kind::Codex {
                    "model-tab-codex"
                } else {
                    kind.id()
                },
                kind.label().into(),
            ),
        };
        div()
            .id(id)
            .role(Role::Tab)
            .aria_label(label)
            .aria_selected(selected)
            .group(id)
            .cursor_pointer()
            .flex_shrink_0()
            .size(px(32.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .justify_center()
            .when(selected, |d| d.bg(content(0.12)))
            .when(!selected, |d| d.hover(|s| s.bg(content(0.08))))
            .child(match tab {
                ModelTab::Favorites => icon(if selected { "star-filled" } else { "star" }, 16.)
                    .text_color(content(if selected { 1. } else { 0.45 }))
                    .group_hover(id, |s| s.text_color(content(1.)))
                    .into_any_element(),
                ModelTab::Harness(kind) => provider_icon(kind.id(), 16.).into_any_element(),
            })
            // Hovering an unselected provider selects it, as in the rail.
            .on_hover(cx.listener(move |this, hovered: &bool, window, cx| {
                if *hovered && !this.quick {
                    this.select_tab(tab, window, cx);
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| this.select_tab(tab, window, cx)))
            .into_any()
    }
}
