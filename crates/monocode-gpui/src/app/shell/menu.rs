use super::*;

impl Shell {
    pub(super) fn menu_open(&self, target: &MenuTarget) -> bool {
        self.menu
            .as_ref()
            .is_some_and(|open| open.target == *target)
    }

    pub(super) fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            self.clear_context_selection();
            self.invalidate_chrome(cx);
            cx.notify();
        }
    }

    pub(crate) fn open_menu(
        &mut self,
        target: MenuTarget,
        anchor: Anchor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_preview(cx);
        // Close the previous menu first, so the new one remembers the focus
        // that was there before either opened.
        self.menu = None;
        let mut events = Vec::new();
        let (label, items, width, style) = self.menu_items(&target, &mut events, window, cx);
        let menu = cx.new(|cx| {
            Menu::new(label, items, window, cx)
                .width(width)
                .style(style)
        });
        events.push(
            cx.subscribe_in(&menu, window, |this, _, event: &MenuEvent, window, cx| {
                if let MenuEvent::Picked(id, _) = event
                    && let Some(open) = this
                        .menu
                        .as_ref()
                        .filter(|m| m.target == MenuTarget::SidebarOptions)
                {
                    let menu = open.menu.clone();
                    this.change_sidebar_option(id, cx);
                    let items = this.sidebar_options_menu_items();
                    menu.update(cx, |menu, cx| menu.replace_items(items, cx));
                    return;
                }
                if let MenuEvent::Picked(id, _) = event
                    && id != "archived"
                    && let Some(open) = this
                        .menu
                        .as_ref()
                        .filter(|m| m.target == MenuTarget::SessionFilter)
                {
                    let menu = open.menu.clone();
                    this.change_session_filter(id, cx);
                    let items = this.filter_menu_items(cx);
                    menu.update(cx, |menu, cx| menu.replace_items(items, cx));
                    return;
                }
                let Some(open) = this.menu.take() else {
                    return;
                };
                let target = open.target.clone();
                if let MenuEvent::Picked(id, row) = event {
                    this.run_menu(target, id, *row, window, cx);
                }
                this.clear_context_selection();
                this.invalidate_chrome(cx);
                cx.notify();
            }),
        );
        self.menu = Some(ShellMenu {
            target,
            anchor,
            menu,
            _events: events,
        });
        self.invalidate_chrome(cx);
        cx.notify();
    }

    pub(super) fn menu_items(
        &self,
        target: &MenuTarget,
        events: &mut Vec<Subscription>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (SharedString, Vec<MenuItem>, f32, MenuStyle) {
        if let Some(menu) = self.terminal_menu_items(target, cx) {
            return menu;
        }
        let title = |id: &i64| {
            self.session_summaries
                .get(id)
                .map(|_| self.session_metadata(*id, cx).display_title())
                .unwrap_or_default()
        };
        match target {
            // Answered by terminal_menu_items above.
            MenuTarget::TerminalTab(_) | MenuTarget::RunningTerminals => {
                ("".into(), Vec::new(), 160., MenuStyle::Explorer)
            }
            MenuTarget::Actions => (
                "Actions".into(),
                self.actions_menu(),
                200.,
                MenuStyle::Explorer,
            ),
            MenuTarget::NewTab => (
                "New tab".into(),
                vec![
                    MenuAction::new("session", "New Session")
                        .icon("plus")
                        .shortcut(format!("{MOD}N"))
                        .into(),
                    MenuAction::new("terminal", "New Terminal")
                        .icon("terminal")
                        .shortcut(format!("{MOD}J"))
                        .into(),
                ],
                200.,
                MenuStyle::Explorer,
            ),
            // The model picker a conversation shows, starting a session instead.
            MenuTarget::NewTabModel => {
                let choice = self.default_choice();
                let project = self.workspace.project.clone();
                let profile = crate::accounts::get(cx)
                    .map(|accounts| accounts.read(cx).selected(choice.kind, &project))
                    .unwrap_or_default();
                let hidden = self.workspace.preferences.hidden_for(&project);
                let list = cx.new(|cx| {
                    crate::chat::composer::pickers::ModelList::new_for(
                        choice.kind,
                        choice.model,
                        &project,
                        profile,
                        cx,
                    )
                    .hide_providers(hidden)
                    .allow_provider_switch(true)
                });
                events.push(cx.subscribe(
                    &list,
                    |this, _, picked: &crate::chat::composer::pickers::ModelPicked, cx| {
                        this.close_menu(cx);
                        let choice = glue::new_chat_choice::NewChatChoice {
                            kind: picked.0,
                            model: picked.1.clone(),
                        };
                        this.create_session(choice, false, cx);
                    },
                ));
                let focus = list.read(cx).search_focus(cx);
                (
                    "New session model".into(),
                    vec![MenuItem::Custom(list.into(), focus)],
                    310.,
                    MenuStyle::Explorer,
                )
            }
            MenuTarget::Tab(_)
            | MenuTarget::MonoTab(_)
            | MenuTarget::SearchTab
            | MenuTarget::AppTab(_) => {
                let tab = match target {
                    MenuTarget::Tab(id) => TitleTab::Session(*id),
                    MenuTarget::MonoTab(id) => self.mono_tab(id.clone()),
                    MenuTarget::SearchTab => TitleTab::Search,
                    MenuTarget::AppTab(view) => TitleTab::App(*view),
                    _ => unreachable!(),
                };
                let tabs = self.visible_tabs(cx);
                let ix = tabs.iter().position(|entry| *entry == tab).unwrap_or(0);
                let count = tabs.len();
                let closable = tab.session().is_none_or(|id| self.tab_can_close(id, cx));
                let pinned = self.tab_pinned(&tab);
                let mut items = vec![
                    MenuAction::new(
                        if pinned { "unpin" } else { "pin" },
                        if pinned { "Unpin Tab" } else { "Pin Tab" },
                    )
                    .into(),
                    MenuItem::Separator,
                    MenuAction::new("close", "Close Tab")
                        .shortcut(format!("{MOD}W"))
                        .disabled(!closable)
                        .into(),
                    MenuItem::Separator,
                    MenuAction::new("others", "Close Other Tabs")
                        .disabled(count < 2)
                        .into(),
                    MenuAction::new("right", "Close Tabs to the Right")
                        .disabled(ix + 1 >= count)
                        .into(),
                    MenuAction::new("left", "Close Tabs to the Left")
                        .disabled(ix == 0)
                        .into(),
                    MenuAction::new("all", "Close All Tabs")
                        .shortcut(if cfg!(target_os = "macos") {
                            "⌘⇧W"
                        } else {
                            "Ctrl+Shift+W"
                        })
                        .disabled(count < 2 && !closable)
                        .into(),
                ];
                // Closing a tab keeps its session; archive also hides it from
                // the active history while preserving everything on disk.
                if let Some(id) = tab.session().filter(|id| {
                    self.session_summaries.contains_key(id)
                        && self.session_metadata(*id, cx).summary.has_messages
                }) {
                    items.extend([
                        MenuItem::Separator,
                        MenuAction::new(
                            "archive",
                            if self.session_metadata(id, cx).summary.archived {
                                "Unarchive"
                            } else {
                                "Archive"
                            },
                        )
                        .into(),
                        MenuAction::new("delete", "Delete").danger().into(),
                    ]);
                }
                (
                    format!(
                        "Tab actions for {}",
                        match &tab {
                            TitleTab::Session(id) => title(id),
                            TitleTab::Search => "Search".into(),
                            TitleTab::App(view) => view.label().into(),
                            TitleTab::Mono { id, .. } => self
                                .monos
                                .as_ref()
                                .and_then(|m| m.read(cx).roster.get(id))
                                .map(|m| m.label())
                                .unwrap_or_default(),
                        }
                    )
                    .into(),
                    items,
                    244.,
                    MenuStyle::Explorer,
                )
            }
            MenuTarget::Session(id) => (
                format!("Session actions for {}", title(id)).into(),
                self.session_menu_items(*id, cx),
                228.,
                MenuStyle::Explorer,
            ),
            MenuTarget::Folder(folder) => (
                "Folder actions".into(),
                self.folder_menu_items(folder, events, cx),
                228.,
                MenuStyle::Explorer,
            ),
            MenuTarget::TerminalDock => {
                let side = self
                    .workspace
                    .docks
                    .get(&self.workspace.project)
                    .map_or(DockSide::Bottom, |d| d.side);
                (
                    "Move terminal".into(),
                    [
                        ("bottom", "Dock Bottom", DockSide::Bottom),
                        ("top", "Dock Top", DockSide::Top),
                        ("left", "Dock Left", DockSide::Left),
                        ("right", "Dock Right", DockSide::Right),
                    ]
                    .into_iter()
                    .map(|(id, label, s)| MenuAction::new(id, label).checked(s == side).into())
                    .collect(),
                    160.,
                    MenuStyle::Explorer,
                )
            }
            MenuTarget::SessionFilter => (
                "Filter sessions".into(),
                self.filter_menu_items(cx),
                228.,
                MenuStyle::Filter,
            ),
            MenuTarget::SidebarOptions => (
                "Sidebar options".into(),
                self.sidebar_options_menu_items(),
                184.,
                MenuStyle::Filter,
            ),
            MenuTarget::ProjectRailAdd => (
                "Open project".into(),
                vec![
                    MenuAction::new("open-project", "Open folder…")
                        .icon("folder-add")
                        .into(),
                    MenuAction::new("open-remote-project", "Open folder on a machine…")
                        .icon("globe")
                        .into(),
                ],
                230.,
                MenuStyle::Explorer,
            ),
            MenuTarget::CompactProjectPicker => {
                let (monos, active) = self
                    .monos
                    .as_ref()
                    .filter(|_| self.workspace.classic_layout && self.workspace.preferences.monos)
                    .map(|manager| {
                        let manager = manager.read(cx);
                        let monos = manager
                            .roster
                            .monos
                            .iter()
                            .map(|mono| crate::chat::composer::pickers::PickerMono {
                                id: mono.id.clone(),
                                name: mono.label(),
                                mascot: mono.mascot.clone(),
                                color: mono.color,
                            })
                            .collect();
                        (monos, manager.active.clone())
                    })
                    .unwrap_or_default();
                let picker = cx.new(|cx| {
                    ProjectPicker::new(&self.workspace.projects, &self.workspace.project, cx)
                        .appearance(self.workspace.project_appearance.clone())
                        .allow_remote()
                        .monos(monos, active, cx)
                });
                events.push(cx.subscribe_in(
                    &picker,
                    window,
                    |this, _, picked: &ProjectPicked, window, cx| {
                        this.close_menu(cx);
                        match picked {
                            ProjectPicked::Project(path) => {
                                this.select_project(path.clone(), window, cx)
                            }
                            ProjectPicked::Mono(id) => {
                                if let Some(monos) = this.monos.clone() {
                                    monos.update(cx, |m, cx| m.open(id, window, cx));
                                }
                            }
                            ProjectPicked::New => this.open_project(cx),
                            ProjectPicked::Remote => {
                                this.open_remote_project(window, cx);
                                return;
                            }
                        }
                        this.focus_active(window, cx);
                        this.invalidate_chrome(cx);
                        cx.notify();
                    },
                ));
                let focus = picker.read(cx).search_focus(cx);
                (
                    "Switch project".into(),
                    vec![MenuItem::Custom(picker.into(), focus)],
                    286.,
                    MenuStyle::Switcher,
                )
            }
            // SidebarWorktreeSwitcher's menu under the project header.
            MenuTarget::Header => {
                let picker = cx.new(|cx| {
                    ProjectPicker::new(&self.workspace.projects, &self.workspace.project, cx)
                        .appearance(self.workspace.project_appearance.clone())
                        .allow_remote()
                });
                events.push(cx.subscribe_in(
                    &picker,
                    window,
                    |this, _, picked: &ProjectPicked, window, cx| {
                        this.close_menu(cx);
                        match picked {
                            ProjectPicked::Project(path) if *path != this.workspace.project => {
                                this.select_project(path.clone(), window, cx)
                            }
                            ProjectPicked::Project(_) | ProjectPicked::Mono(_) => {}
                            ProjectPicked::New => this.open_project(cx),
                            ProjectPicked::Remote => {
                                this.open_remote_project(window, cx);
                                return;
                            }
                        }
                        this.focus_active(window, cx);
                        this.invalidate_chrome(cx);
                        cx.notify();
                    },
                ));
                let focus = picker.read(cx).search_focus(cx);
                let branch = self
                    .worktree_focus
                    .get(&self.workspace.project)
                    .and_then(|focus| focus.branch.clone())
                    .or_else(|| {
                        crate::chat::conversation::git_branch(std::path::Path::new(
                            &self.workspace.project,
                        ))
                    });
                let worktree = MenuAction::new("switch-worktree", "Switch worktree")
                    .icon("folder-tree")
                    .flyout(self.worktree_flyout(events, window, cx));
                (
                    format!("{} menu", self.project_label(&self.workspace.project)).into(),
                    vec![
                        match branch {
                            Some(branch) => worktree.value(branch),
                            None => worktree,
                        }
                        .into(),
                        MenuItem::Separator,
                        MenuAction::new("switch-project", "Switch project")
                            .icon("folder")
                            .flyout(Flyout {
                                view: picker.into(),
                                width: 286.,
                                padding: 0.,
                                height: None,
                                max_height: Some(380.),
                                focus: Some(focus),
                                surface: None,
                            })
                            .into(),
                        MenuAction::new("project-settings", "Project settings")
                            .icon("sliders")
                            .flyout(self.project_settings_flyout(events, window, cx))
                            .into(),
                        MenuAction::new("open-remote-project", "Open folder on a machine…")
                            .icon("globe")
                            .into(),
                        MenuAction::new("settings", "Settings")
                            .icon("settings")
                            .shortcut(format!("{MOD},"))
                            .into(),
                    ],
                    240.,
                    MenuStyle::Switcher,
                )
            }
            MenuTarget::ProjectRailGroup(id) => {
                let controls = self.project_rail_group_controls(id, events, window, cx);
                (
                    "Project group actions".into(),
                    vec![
                        MenuItem::Custom(controls.clone().into(), controls.read(cx).focus(cx)),
                        MenuItem::Separator,
                        MenuAction::new("delete-project-group", "Delete group")
                            .icon("trash")
                            .description("Projects will become ungrouped")
                            .danger()
                            .into(),
                    ],
                    260.,
                    MenuStyle::Group,
                )
            }
            // TabGroupMenu's appearance controls and project actions.
            MenuTarget::Project { path, rail } => {
                let controls = self.project_appearance_controls(path, events, window, cx);
                let mut items: Vec<MenuItem> = vec![
                    MenuItem::Custom(controls.clone().into(), controls.read(cx).focus(cx)),
                    MenuItem::Separator,
                    MenuAction::new("project-theme", "Theme colors")
                        .icon("palette")
                        .into(),
                    MenuAction::new("background", "Background image")
                        .icon("image-add")
                        .into(),
                ];
                if *rail {
                    let mut groups = vec![MenuAction::new("project-group:new", "New group…")];
                    if self.workspace.project_rail_assignments.contains_key(path) {
                        groups.push(MenuAction::new("project-group:", "No group"));
                    }
                    groups.extend(self.workspace.project_rail_groups.iter().map(|group| {
                        MenuAction::new(format!("project-group:{}", group.id), group.name.clone())
                            .checked(
                                self.workspace.project_rail_assignments.get(path)
                                    == Some(&group.id),
                            )
                    }));
                    let pinned = self.workspace.project_rail_pinned.contains(path);
                    items.extend([
                        MenuAction::new("project-group", "Move to group")
                            .icon("folder-tree")
                            .submenu(groups)
                            .into(),
                        MenuAction::new(
                            "pin",
                            if pinned {
                                "Unpin project"
                            } else {
                                "Pin project"
                            },
                        )
                        .icon(if pinned { "pin-off" } else { "pin" })
                        .into(),
                    ]);
                }
                let peers = self.workspace.project_peers(path);
                if peers.len() > 1 {
                    let ix = peers.iter().position(|p| p == path).unwrap_or(0);
                    let (first, last) = (ix == 0, ix + 1 == peers.len());
                    let (back, forward) = if self.workspace.classic_layout {
                        ("Move up", "Move down")
                    } else {
                        ("Move left", "Move right")
                    };
                    let shortcut = |command| {
                        crate::settings::command_shortcut(&self.workspace.preferences, command)
                    };
                    let step = |id, label, command, disabled| {
                        let action = MenuAction::new(id, label).disabled(disabled);
                        match shortcut(command) {
                            Some(keys) => action.shortcut(keys),
                            None => action,
                        }
                    };
                    items.push(
                        MenuAction::new("move", "Move")
                            .icon("chevrons-up-down")
                            .submenu(vec![
                                step("move:back", back, "Project: Move Back", first),
                                step("move:forward", forward, "Project: Move Forward", last),
                                MenuAction::new("move:start", "Move to start").disabled(first),
                                MenuAction::new("move:end", "Move to end").disabled(last),
                            ])
                            .into(),
                    );
                }
                items.extend([
                    MenuAction::new("reveal", REVEAL_LABEL)
                        .icon("folder-open")
                        .into(),
                    self.external_editor_menu(path).into(),
                    MenuItem::Separator,
                    self.project_notification_menu(path).into(),
                    MenuItem::Separator,
                    MenuAction::new("archive", "Archive").icon("archive").into(),
                    MenuAction::new("delete", "Delete")
                        .icon("trash")
                        .danger()
                        .into(),
                ]);
                (
                    format!("Project actions for {}", project_name(path)).into(),
                    items,
                    260.,
                    MenuStyle::Group,
                )
            }
        }
    }

    pub(super) fn run_menu(
        &mut self,
        target: MenuTarget,
        action: &str,
        row: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let MenuTarget::Tab(id) = target {
            match action {
                "archive" => return self.toggle_archive(id, cx),
                "delete" => return self.confirm_delete_session(id, window, cx),
                _ => {}
            }
        }
        match target {
            MenuTarget::TerminalTab(ix) => self.run_terminal_tab_menu(ix, action, window, cx),
            MenuTarget::RunningTerminals => self.run_running_terminals_menu(action, window, cx),
            MenuTarget::Actions => self.run_actions_menu(action, window, cx),
            MenuTarget::NewTab => match action {
                "terminal" => self.new_title_terminal(window, cx),
                _ => self.new_chat(cx),
            },
            // Its list reports the pick itself.
            MenuTarget::NewTabModel => {}
            MenuTarget::TerminalDock => {
                let side = match action {
                    "top" => DockSide::Top,
                    "left" => DockSide::Left,
                    "right" => DockSide::Right,
                    _ => DockSide::Bottom,
                };
                self.set_dock_side(side, cx);
            }
            MenuTarget::SessionFilter => self.change_session_filter(action, cx),
            MenuTarget::SidebarOptions => self.change_sidebar_option(action, cx),
            MenuTarget::Tab(id) => {
                self.run_title_tab_menu(TitleTab::Session(id), action, window, cx)
            }
            MenuTarget::MonoTab(id) => {
                self.run_title_tab_menu(self.mono_tab(id), action, window, cx)
            }
            MenuTarget::SearchTab => self.run_title_tab_menu(TitleTab::Search, action, window, cx),
            MenuTarget::AppTab(view) => {
                self.run_title_tab_menu(TitleTab::App(view), action, window, cx)
            }
            MenuTarget::Session(id) => self.run_session_menu(id, action, window, cx),
            MenuTarget::Folder(folder) => self.run_folder_menu(folder, action, window, cx),
            MenuTarget::ProjectRailAdd => match action {
                "open-project" => self.open_project(cx),
                "open-remote-project" => self.open_remote_project(window, cx),
                _ => {}
            },
            MenuTarget::CompactProjectPicker => {}
            MenuTarget::ProjectRailGroup(id) => {
                if action == "delete-project-group" {
                    self.workspace.project_rail_groups.retain(|g| g.id != id);
                    self.workspace
                        .project_rail_assignments
                        .retain(|_, group| group != &id);
                    self.persist(cx);
                    self.projects_frame.update(cx, |_, cx| cx.notify());
                }
            }
            MenuTarget::Header => {
                // "Project settings" opens the project's menu beside its row.
                if action == "settings" {
                    self.open_settings(crate::settings::Section::General, window, cx);
                } else if action == "open-remote-project" {
                    self.open_remote_project(window, cx);
                } else if action == "project-settings" {
                    let at = row
                        .map_or_else(Point::default, |r| Point::new(r.right(), r.top() - px(8.)));
                    let target = MenuTarget::Project {
                        path: self.workspace.project.clone(),
                        rail: false,
                    };
                    self.open_menu(target, Anchor::Point(at), window, cx);
                }
            }
            MenuTarget::Project { path, .. } => {
                if action == "pin" {
                    self.toggle_project_rail_pin(&path, cx);
                } else if action == "project-group:new" {
                    let id = self.create_project_rail_group(Some(&path), cx);
                    let at = row.map_or_else(Point::default, |r| Point::new(r.left(), r.top()));
                    self.open_menu(
                        MenuTarget::ProjectRailGroup(id),
                        Anchor::Point(at),
                        window,
                        cx,
                    );
                } else if let Some(group) = action.strip_prefix("project-group:") {
                    self.assign_project_rail_group(&path, group, cx);
                } else if let Some(direction) = action.strip_prefix("move:") {
                    let moved = match direction {
                        "back" => self.workspace.step_project_order(&path, -1),
                        "forward" => self.workspace.step_project_order(&path, 1),
                        "start" => self.workspace.move_project_to_end(&path, -1),
                        _ => self.workspace.move_project_to_end(&path, 1),
                    };
                    if moved {
                        self.project_order_changed(&path, cx);
                    }
                } else if action == "reveal" {
                    if path.starts_with("remote://") {
                        self.error = Some("Remote folders stay on their host machine.".into());
                        cx.notify();
                    } else {
                        cx.reveal_path(std::path::Path::new(&path));
                    }
                } else if action == "project-theme" {
                    self.open_project_theme(path, window, cx);
                } else if action == "background" {
                    self.open_background_dialog(path, window, cx);
                } else if action == "archive" {
                    self.archive_project(&path, window, cx);
                } else if action == "delete" {
                    self.confirm_delete_project(path, window, cx);
                } else if let Some(editor) = action
                    .strip_prefix("external-editor:")
                    .filter(|editor| *editor != "none")
                {
                    self.open_in_external_editor(editor, &path, cx);
                } else if action.starts_with("mute:") {
                    self.run_project_notification_menu(&path, action, window, cx);
                }
            }
        }
    }
}
