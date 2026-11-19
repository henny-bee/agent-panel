use super::*;

impl Monos {
    /// MonoMenu, beside the row's options button or at a right-click.
    pub(in crate::monos) fn open_options(
        &mut self,
        id: &str,
        at: Option<gpui::Point<gpui::Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mono) = self.roster.get(id).cloned() else {
            return;
        };
        let editor = cx.new(|cx| LookEditor::new(mono, cx));
        let focus = editor.read(cx).name.focus_handle(cx);
        let changed = cx.subscribe(&editor, |this, _, event: &LookChanged, cx| {
            if let Some(mono) = this.roster.get_mut(&event.mono.id) {
                mono.name = event.mono.name.clone();
                mono.mascot = event.mono.mascot.clone();
                mono.color = event.mono.color;
            }
            this.configure_chat(&event.mono.id, cx);
            this.save(cx);
        });
        let submitted = cx.subscribe(&editor, |this, _, _: &LookSubmitted, cx| {
            this.menu = None;
            cx.notify();
        });
        let menu = cx.new(|cx| {
            Menu::new(
                "Mono options",
                vec![
                    MenuItem::Custom(AnyView::from(editor), focus.clone()),
                    MenuItem::Separator,
                    MenuAction::new("background", "Background image")
                        .icon("image-add")
                        .into(),
                    MenuAction::new("delete", "Delete mono…")
                        .icon("trash")
                        .danger()
                        .into(),
                ],
                window,
                cx,
            )
            .width(280.)
            .style(MenuStyle::Group)
        });
        let target = id.to_string();
        let picked = cx.subscribe_in(&menu, window, move |this, _, event: &MenuEvent, w, cx| {
            this.menu = None;
            if let MenuEvent::Picked(action, _) = event {
                match action.as_ref() {
                    "background" => this.open_background(&target, w, cx),
                    "delete" => this.delete_mono(&target, w, cx),
                    _ => {}
                }
            }
            cx.notify();
        });
        self.menu_anchor = match at {
            Some(at) => Anchor::Point(at),
            None => self
                .rail_bounds
                .get(id)
                .cloned()
                .map(Anchor::Slot)
                .unwrap_or_else(|| Anchor::Slot(self.settings_anchor.clone())),
        };
        self.menu = Some((id.into(), menu, vec![changed, submitted, picked]));
        window.focus(&focus, cx);
        cx.notify();
    }

    pub(in crate::monos) fn options_surface(&self, cx: &App) -> Option<AnyElement> {
        self.menu.as_ref().map(|(_, menu, _)| {
            menu_popover(menu, self.menu_anchor.clone(), cx)
                .without_animation()
                .side(Side::Right)
                .priority(90)
                .gap(0.)
                .into_any_element()
        })
    }

    pub(in crate::monos) fn picker_surface(&self, cx: &App) -> Option<AnyElement> {
        self.picker.as_ref().map(|p| {
            menu_popover(&p.menu, p.anchor.clone(), cx)
                .align(p.align)
                .without_animation()
                .priority(90)
                .width(p.width)
                .into_any_element()
        })
    }

    pub(super) fn open_model_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(chat) = self.chat() else {
            return;
        };
        let view = chat.read(cx);
        let provider = Kind::from_id(&view.summary.provider);
        let model = view.mono_model();
        let project = view.summary.cwd.clone();
        let account = view.settings.account.clone();
        let list = cx.new(|cx| ModelList::new_for(provider, model, &project, account, cx));
        let focus = list.read(cx).search_focus(cx);
        let picked = cx.subscribe(&list, |this, _, picked: &ModelPicked, cx| {
            if let Some(view) = this.chat()
                && let Err(e) = view.update(cx, |v, cx| {
                    v.mono_config(picked.0, picked.1.clone(), HashMap::new(), cx)
                })
            {
                this.error = Some(e);
            }
            this.picker = None;
            this.changed(cx);
        });
        let menu = cx.new(|cx| {
            Menu::new(
                "Model",
                vec![MenuItem::Custom(list.into(), focus)],
                window,
                cx,
            )
            .width(400.)
        });
        let closed = cx.subscribe(&menu, |this, _, _: &MenuEvent, cx| {
            this.picker = None;
            cx.notify();
        });
        self.picker = Some(OpenPicker {
            menu,
            anchor: Anchor::Slot(self.model_anchor.clone()),
            width: 400.,
            align: crate::ui::popover::Align::Start,
            _events: vec![picked, closed],
        });
        cx.notify();
    }

    /// What sessions it starts run on, as its settings show it.
    pub(super) fn subsession_choice(&self, mono: &Mono, cx: &App) -> SubsessionChoice {
        let set = mono.subsessions.clone().unwrap_or_default();
        let chat = self.chat();
        let own = chat
            .as_ref()
            .map(|v| v.read(cx).summary.provider.clone())
            .unwrap_or_else(|| "codex".into());
        let kind = Kind::from_id(set.harness.as_deref().unwrap_or(&own));
        let profile = chat
            .as_ref()
            .filter(|v| v.read(cx).summary.provider == kind.id())
            .map(|v| v.read(cx).settings.account.clone())
            .or_else(|| {
                let project = chat.as_ref().map(|v| v.read(cx).summary.cwd.clone())?;
                crate::accounts::get(cx).map(|c| c.read(cx).selected(kind, &project))
            })
            .unwrap_or_default();
        let models =
            monocode_core::provider::catalog::cached_models_for(kind, &profile).unwrap_or_default();
        // Inherited, its effort levels are those of the Mono's own model.
        let model = if set.harness.is_some() {
            set.model.clone()
        } else {
            chat.as_ref().and_then(|v| v.read(cx).mono_model())
        };
        let entry = model
            .as_deref()
            .and_then(|id| models.iter().find(|m| m.id == id));
        let entry = entry.or_else(|| {
            set.harness
                .is_some()
                .then(|| models.iter().find(|m| m.default).or(models.first()))
                .flatten()
        });
        let model_label = match &set.harness {
            None => format!("Same as {}", mono.label()),
            Some(_) => entry
                .map(|m| m.name.clone())
                .or(set.model.clone())
                .unwrap_or_else(|| kind.label().into()),
        };
        let efforts = entry
            .map(|m| monocode_core::project::app_access::effort_choices(kind, m))
            .unwrap_or_default();
        let effort_label = match &set.effort {
            Some(effort) => efforts
                .iter()
                .find(|(value, _)| value == effort)
                .map(|(_, name)| name.clone())
                .unwrap_or_else(|| effort.clone()),
            None if set.harness.is_none() => format!("Same as {}", mono.label()),
            None => "Model default".into(),
        };
        SubsessionChoice {
            provider: set.harness.is_some().then_some(kind),
            kind,
            profile,
            current_model: entry.map(|m| m.id.clone()),
            model_label: model_label.into(),
            efforts,
            effort_label: effort_label.into(),
        }
    }

    pub(super) fn set_subsessions(
        &mut self,
        change: impl FnOnce(&mut monocode_core::session::mono_model::Subsessions),
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.active.clone()
            && let Some(mono) = self.roster.get_mut(&id)
        {
            let mut next = mono.subsessions.clone().unwrap_or_default();
            change(&mut next);
            mono.subsessions = next.normalized();
            self.save(cx);
            self.changed(cx);
        }
    }

    pub(super) fn open_subsession_model_picker(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mono) = self.current().cloned() else {
            return;
        };
        let inherited = mono
            .subsessions
            .as_ref()
            .is_none_or(|s| s.harness.is_none());
        let choice = self.subsession_choice(&mono, cx);
        let project = self
            .chat()
            .map(|v| v.read(cx).summary.cwd.clone())
            .unwrap_or_default();
        let current = (!inherited).then_some(choice.current_model).flatten();
        let list =
            cx.new(|cx| ModelList::new_for(choice.kind, current, &project, choice.profile, cx));
        let focus = list.read(cx).search_focus(cx);
        let picked = cx.subscribe(&list, |this, _, picked: &ModelPicked, cx| {
            let harness = picked.0.id().to_owned();
            let model = picked.1.clone();
            this.set_subsessions(
                |s| {
                    // An effort set for another model may not fit this one.
                    if s.harness.as_deref() != Some(harness.as_str()) || s.model != model {
                        s.effort = None;
                    }
                    s.harness = Some(harness);
                    s.model = model;
                },
                cx,
            );
            this.picker = None;
            cx.notify();
        });
        let items = vec![
            MenuAction::new("subsessions:inherit", format!("Same as {}", mono.label()))
                .checked(inherited)
                .into(),
            MenuItem::Separator,
            MenuItem::Custom(list.into(), focus),
        ];
        let menu = cx.new(|cx| Menu::new("Model", items, window, cx).width(400.));
        let closed = cx.subscribe(&menu, |this, _, event: &MenuEvent, cx| {
            if let MenuEvent::Picked(id, _) = event
                && id == "subsessions:inherit"
            {
                this.set_subsessions(
                    |s| {
                        s.harness = None;
                        s.model = None;
                        s.effort = None;
                    },
                    cx,
                );
            }
            this.picker = None;
            cx.notify();
        });
        self.picker = Some(OpenPicker {
            menu,
            anchor: Anchor::Slot(self.subsession_model_anchor.clone()),
            width: 400.,
            align: crate::ui::popover::Align::Start,
            _events: vec![picked, closed],
        });
        cx.notify();
    }

    pub(super) fn open_subsession_effort_picker(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mono) = self.current().cloned() else {
            return;
        };
        let choice = self.subsession_choice(&mono, cx);
        let effort = mono.subsessions.as_ref().and_then(|s| s.effort.clone());
        let unset = if mono
            .subsessions
            .as_ref()
            .is_none_or(|s| s.harness.is_none())
        {
            format!("Same as {}", mono.label())
        } else {
            "Model default".into()
        };
        let mut items: Vec<MenuItem> = vec![
            MenuAction::new("effort:", unset)
                .checked(effort.is_none())
                .into(),
        ];
        items.extend(choice.efforts.iter().map(|(value, name)| {
            MenuAction::new(format!("effort:{value}"), name.clone())
                .checked(effort.as_deref() == Some(value.as_str()))
                .into()
        }));
        let menu = cx.new(|cx| Menu::new("Effort", items, window, cx));
        let picked = cx.subscribe(&menu, |this, _, event: &MenuEvent, cx| {
            if let MenuEvent::Picked(id, _) = event
                && let Some(value) = id.strip_prefix("effort:")
            {
                let value = (!value.is_empty()).then(|| value.to_owned());
                this.set_subsessions(|s| s.effort = value, cx);
            }
            this.picker = None;
            cx.notify();
        });
        self.picker = Some(OpenPicker {
            menu,
            anchor: Anchor::Slot(self.subsession_effort_anchor.clone()),
            width: 228.,
            align: crate::ui::popover::Align::Start,
            _events: vec![picked],
        });
        cx.notify();
    }

    pub(super) fn open_model_option(
        &mut self,
        label: SharedString,
        choices: Vec<MenuAction>,
        anchor: AnchorSlot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let menu = cx.new(|cx| {
            Menu::new(
                label,
                choices.into_iter().map(Into::into).collect(),
                window,
                cx,
            )
        });
        let picked = cx.subscribe(&menu, |this, _, event: &MenuEvent, cx| {
            if let MenuEvent::Picked(id, _) = event
                && let Some(chat) = this.chat()
                && let Err(e) = chat.update(cx, |v, cx| v.mono_pick_model_option(id, cx))
            {
                this.error = Some(e);
            }
            this.picker = None;
            this.changed(cx);
        });
        self.picker = Some(OpenPicker {
            menu,
            anchor: Anchor::Slot(anchor),
            width: 228.,
            align: crate::ui::popover::Align::Start,
            _events: vec![picked],
        });
        cx.notify();
    }

    pub(super) fn open_access_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let options = self
            .chat()
            .map(|chat| chat.read(cx).mono_access_options())
            .unwrap_or_default();
        let menu = cx.new(|cx| {
            Menu::new("Permissions", options, window, cx)
                .style(MenuStyle::Access)
                .width(288.)
        });
        let picked = cx.subscribe(&menu, |this, _, event: &MenuEvent, cx| {
            if let MenuEvent::Picked(id, _) = event
                && let Some(view) = this.chat()
            {
                view.update(cx, |v, cx| {
                    v.mono_access(
                        AccessMode::from_id(id.strip_prefix("access:").unwrap_or(id)),
                        cx,
                    )
                });
            }
            this.picker = None;
            cx.notify();
        });
        self.picker = Some(OpenPicker {
            menu,
            anchor: Anchor::Slot(self.access_anchor.clone()),
            width: 288.,
            align: crate::ui::popover::Align::Start,
            _events: vec![picked],
        });
        cx.notify();
    }

    pub(super) fn pick_project(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let projects = self
            .owner
            .upgrade()
            .map(|owner| owner.read(cx).workspace.projects.clone())
            .unwrap_or_default();
        let mono = id.to_string();
        let paths = projects.clone();
        let mut items: Vec<MenuItem> = projects
            .into_iter()
            .enumerate()
            .map(|(ix, path)| MenuAction::new(format!("project:{ix}"), path).into())
            .collect();
        items.push(MenuItem::Separator);
        items.push(MenuAction::new("browse", "Open project…").into());
        let menu = cx.new(|cx| Menu::new("Add project", items, window, cx).width(300.));
        let picked = cx.subscribe(&menu, move |this, _, event: &MenuEvent, cx| {
            this.picker = None;
            if let MenuEvent::Picked(action, _) = event {
                if action == "browse" {
                    this.browse_project(&mono, cx);
                } else if let Some(path) = action
                    .strip_prefix("project:")
                    .and_then(|n| n.parse::<usize>().ok())
                    .and_then(|ix| paths.get(ix))
                {
                    this.add_project(&mono, path.clone(), cx);
                }
            }
            cx.notify();
        });
        self.picker = Some(OpenPicker {
            menu,
            anchor: Anchor::Slot(self.projects_anchor.clone()),
            width: 300.,
            align: crate::ui::popover::Align::Start,
            _events: vec![picked],
        });
        cx.notify();
    }

    pub(super) fn add_project(&mut self, id: &str, path: String, cx: &mut Context<Self>) {
        if let Some(m) = self.roster.get_mut(id) {
            m.projects.push(path);
            m.normalize();
        }
        self.configure_chat(id, cx);
        self.save(cx);
    }

    pub(super) fn browse_project(&mut self, id: &str, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add project".into()),
        });
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |this, cx| {
                    this.add_project(&id, path.to_string_lossy().into(), cx)
                });
            }
        })
        .detach();
    }

    pub(crate) fn finish_reorder(&mut self, cx: &mut Context<Self>) {
        if let Some((id, _, Some(over))) = self.drag.take()
            && self.skip_click
        {
            let mut ids = self.rail_ids.clone();
            if let Some((from, to)) = ids
                .iter()
                .position(|s| s == &id)
                .zip(ids.iter().position(|s| s == &over))
            {
                let id = ids.remove(from);
                ids.insert(to, id);
                self.roster.reorder(&ids);
                self.save(cx);
            }
        }
    }
}
