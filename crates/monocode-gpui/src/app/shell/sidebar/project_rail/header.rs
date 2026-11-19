use super::*;

impl Shell {
    pub(super) fn project_rail_heading(
        &self,
        label: &'static str,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .debug_selector(move || format!("rail-heading-{}", label.to_lowercase()))
            .w_full()
            .h(px(30.))
            .pl(px(10.))
            .pr_3()
            .pt_1()
            .pb(px(6.))
            .flex()
            .items_center()
            .gap_1()
            .when(compact, |d| d.px(px(14.)))
            .when(!compact, |d| {
                d.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .px_1()
                        .text_size(px(12.))
                        .line_height(px(20.))
                        .text_color(content(0.5))
                        .child(label),
                )
            })
            .when(label == "Groups", |d| {
                d.child(
                    icon_button("rail-new-project-group", "New project group", "folder-tree")
                        .size(px(20.))
                        .on_click(cx.listener(|this, e: &gpui::ClickEvent, w, cx| {
                            let id = this.create_project_rail_group(None, cx);
                            this.open_menu(
                                MenuTarget::ProjectRailGroup(id),
                                Anchor::Point(e.position()),
                                w,
                                cx,
                            );
                        })),
                )
            })
            .when(label == "Projects", |d| {
                let anchor = AnchorSlot::default();
                d.child(
                    icon_button("rail-open-project", "Open project", "plus")
                        .debug_selector(|| "rail-open-project".into())
                        .child(anchor.probe())
                        .aria_expanded(self.menu_open(&MenuTarget::ProjectRailAdd))
                        .size(px(20.))
                        .when(self.menu_open(&MenuTarget::ProjectRailAdd), |d| {
                            d.bg(content(0.08)).text_color(content(1.))
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if this.menu_open(&MenuTarget::ProjectRailAdd) {
                                this.close_menu(cx);
                            } else {
                                this.open_menu(
                                    MenuTarget::ProjectRailAdd,
                                    Anchor::Slot(anchor.clone()),
                                    window,
                                    cx,
                                );
                            }
                        })),
                )
            })
            .into_any()
    }

    pub(super) fn project_rail_footer(&self, compact: bool, cx: &mut Context<Self>) -> AnyElement {
        div()
            .py_2()
            .pl(px(if compact { 8. } else { 6. }))
            .pr_2()
            .flex_shrink_0()
            .when(!compact, |d| {
                d.children(
                    self.github_star_prompt(cx)
                        .map(|prompt| div().mb_1().child(prompt)),
                )
            })
            .child(
                div()
                    .id("rail-settings")
                    .debug_selector(|| "rail-settings".into())
                    .role(Role::Button)
                    .aria_label("Settings")
                    .cursor_pointer()
                    .h(px(32.))
                    .px_2()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(content(0.5))
                    .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
                    .child(icon("settings", 16.).text_color(content(0.35)))
                    .when(!compact, |d| {
                        d.child(div().flex_1().child("Settings")).child(
                            div()
                                .font_weight(FontWeight::NORMAL)
                                .text_size(px(11.))
                                .text_color(content(0.4))
                                .child(if cfg!(target_os = "macos") {
                                    "⌘,"
                                } else {
                                    "Ctrl ,"
                                }),
                        )
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_settings(crate::settings::Section::General, window, cx)
                    })),
            )
            .into_any()
    }

    pub(super) fn original_rail_navigation(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .pl(px(6.))
            .pr_2()
            .pt(px(2.))
            .pb_2()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_px()
            .when(self.workspace.sidebar_search_visible, |d| {
                d.child(
                    div()
                        .id("rail-search")
                        .debug_selector(|| "sidebar-search-field".into())
                        .group("rail-search")
                        .role(Role::Button)
                        .aria_label("Search")
                        .cursor_pointer()
                        .h(px(32.))
                        .mb(px(2.))
                        .px(px(6.))
                        .rounded_md()
                        .border_1()
                        .border_color(content(0.08))
                        .shadow(vec![
                            BoxShadow::new(px(0.), px(1.), rgba(0x0000001a).into())
                                .blur_radius(px(3.)),
                            BoxShadow::new(px(0.), px(1.), rgba(0x0000001a).into())
                                .blur_radius(px(2.))
                                .spread_radius(px(-1.)),
                        ])
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(content(0.5))
                        .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
                        .child(icon("search", 16.).text_color(content(0.35)))
                        .child(div().flex_1().min_w_0().child("Search"))
                        .child(
                            div()
                                .text_size(px(11.))
                                .font_weight(FontWeight::NORMAL)
                                .text_color(content(0.4))
                                .child(if cfg!(target_os = "macos") {
                                    "⌘K"
                                } else {
                                    "Ctrl K"
                                }),
                        )
                        .on_click(cx.listener(|this, _, w, cx| this.focus_search(w, cx))),
                )
            })
            .children(
                [
                    ("Inbox", "inbox"),
                    ("Notes", "file"),
                    ("Automations", "zap"),
                ]
                .into_iter()
                .filter(|(label, _)| match *label {
                    "Inbox" => self.workspace.sidebar_inbox_visible,
                    "Notes" => {
                        self.workspace.preferences.notes && self.workspace.sidebar_notes_visible
                    }
                    "Automations" => self.workspace.sidebar_automations_visible,
                    _ => true,
                })
                .map(|(label, glyph)| {
                    let selected = self.app_view_label() == Some(label);
                    let unread = label == "Inbox"
                        && self
                            .inbox
                            .as_ref()
                            .is_some_and(|inbox| inbox.read(cx).unread_count() > 0);
                    div()
                        .id(label)
                        .debug_selector(move || format!("sidebar-navigation-{label}"))
                        .role(Role::Button)
                        .aria_label(if unread { "Inbox, new items" } else { label })
                        .aria_selected(selected)
                        .cursor_pointer()
                        .h(px(32.))
                        .px_2()
                        .rounded_md()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(content(if selected { 1. } else { 0.5 }))
                        .when(selected, |d| d.bg(content(0.1)))
                        .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
                        .child(icon(glyph, 16.).text_color(content(if selected {
                            0.7
                        } else {
                            0.35
                        })))
                        .child(div().flex_1().child(label))
                        .when(unread, |d| {
                            d.child(div().size(px(8.)).rounded_full().bg(accent_color()))
                        })
                        .on_click(cx.listener(move |this, _, w, cx| match label {
                            "Inbox" => this.open_inbox(w, cx),
                            "Notes" => this.open_notes(w, cx),
                            "Automations" => this.open_automations(w, cx),
                            _ => (),
                        }))
                }),
            )
            .into_any()
    }

    pub(super) fn project_rail_compact_actions(&mut self, cx: &mut Context<Self>) -> AnyElement {
        use sidebar::classic::ClassicSidebarTab;
        let workspace_active = !self.app_view_open() && !self.mono_active(cx);
        let inbox_unread = self
            .inbox
            .as_ref()
            .is_some_and(|v| v.read(cx).unread_count() > 0);
        let mut tabs = Vec::new();
        for id in self
            .workspace
            .classic_sidebar_tab_order
            .iter()
            .map(String::as_str)
            .chain(["sessions", "files", "changes"])
        {
            let tab = ClassicSidebarTab::from_id(id);
            if !tabs.contains(&tab) {
                tabs.push(tab);
            }
        }
        let mono_rail =
            if self.workspace.project_rail_monos_pinned && self.workspace.preferences.monos {
                self.monos
                    .clone()
                    .filter(|m| !m.read(cx).roster.monos.is_empty())
                    .map(|m| m.update(cx, |m, cx| m.project_navigation_rail(true, cx)))
            } else {
                None
            };
        let active_mono = self
            .monos
            .as_ref()
            .filter(|_| self.workspace.preferences.monos)
            .and_then(|manager| {
                let manager = manager.read(cx);
                manager.current().map(|mono| {
                    let status = manager
                        .chat()
                        .map_or("idle", |chat| chat.read(cx).mono_status());
                    (
                        mono.label(),
                        crate::ui::avatar::avatar(&mono.mascot, mono.color, status, 16., true),
                    )
                })
            });
        let (picker_label, picker_mark) = active_mono.map_or_else(
            || {
                (
                    format!(
                        "Switch project, {}",
                        self.project_label(&self.workspace.project)
                    ),
                    self.project_mark(&self.workspace.project, 16.),
                )
            },
            |(name, mark)| (format!("Switch project, current mono {name}"), mark),
        );
        div()
            .w_full()
            .py(px(6.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(6.))
            .when_some(mono_rail, |d, monos| {
                d.child(monos)
                    .child(div().w(px(24.)).h(px(1.)).bg(content(0.07)))
            })
            .child(
                div()
                    .id("rail-current-project")
                    .debug_selector(|| "rail-current-project".into())
                    .role(Role::Button)
                    .aria_label(picker_label)
                    .aria_expanded(self.menu_open(&MenuTarget::CompactProjectPicker))
                    .size(px(32.))
                    .rounded_md()
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|s| s.bg(content(0.1)))
                    .child(picker_mark)
                    .on_click(cx.listener(|this, e: &gpui::ClickEvent, w, cx| {
                        if this.menu_open(&MenuTarget::CompactProjectPicker) {
                            this.close_menu(cx);
                        } else {
                            this.open_menu(
                                MenuTarget::CompactProjectPicker,
                                Anchor::Point(e.position()),
                                w,
                                cx,
                            )
                        }
                    })),
            )
            .children(tabs.into_iter().map(|tab| {
                let (label, glyph) = match tab {
                    ClassicSidebarTab::Sessions => ("Sessions", "chatting"),
                    ClassicSidebarTab::Files => ("Explorer", "file-script"),
                    ClassicSidebarTab::Changes => ("Changes", "git-branch"),
                };
                let selected = workspace_active
                    && self.workspace.sidebar_visible
                    && self.classic_sidebar_tab == tab;
                let changed = tab == ClassicSidebarTab::Changes
                    && self
                        .git
                        .read(cx)
                        .stats
                        .as_ref()
                        .is_some_and(|s| s.files > 0 || s.additions > 0 || s.deletions > 0);
                icon_button(
                    match tab {
                        ClassicSidebarTab::Sessions => "rail-workspace-sessions",
                        ClassicSidebarTab::Files => "rail-workspace-files",
                        ClassicSidebarTab::Changes => "rail-workspace-changes",
                    },
                    label,
                    glyph,
                )
                .debug_selector(move || format!("rail-workspace-{}", tab.id()))
                .role(Role::Tab)
                .aria_selected(selected)
                .size(px(32.))
                .relative()
                .when(selected, |d| d.bg(content(0.1)))
                .when(changed, |d| {
                    d.child(
                        div()
                            .absolute()
                            .right(px(6.))
                            .top(px(6.))
                            .size(px(6.))
                            .rounded_full()
                            .bg(accent_color()),
                    )
                })
                .on_click(cx.listener(move |this, _, w, cx| {
                    this.leave_app_views(cx);
                    this.leave_app_search(cx);
                    if let Some(m) = &this.monos {
                        m.update(cx, |m, cx| m.close(cx));
                    }
                    this.select_classic_sidebar_tab(tab, w, cx);
                    this.projects_frame.update(cx, |_, cx| cx.notify());
                }))
            }))
            .child(
                icon_button("rail-global-search", "Search", "search")
                    .size(px(32.))
                    .when(self.app_search.is_some(), |d| d.bg(content(0.1)))
                    .on_click(cx.listener(|this, _, w, cx| {
                        if this.app_search.is_some() {
                            this.close_app_search(w, cx)
                        } else {
                            this.focus_search(w, cx)
                        }
                    })),
            )
            .children(
                [
                    (
                        workspace::AppView::Inbox,
                        self.workspace.sidebar_inbox_visible,
                    ),
                    (
                        workspace::AppView::Notes,
                        self.workspace.preferences.notes && self.workspace.sidebar_notes_visible,
                    ),
                    (
                        workspace::AppView::Automations,
                        self.workspace.sidebar_automations_visible,
                    ),
                ]
                .into_iter()
                .filter(|(_, v)| *v)
                .map(|(view, _)| {
                    let selected = self.app_view_label() == Some(view.label());
                    let glyph = if view == workspace::AppView::Notes {
                        "sticky-note"
                    } else {
                        view.icon()
                    };
                    icon_button(
                        match view {
                            workspace::AppView::Inbox => "rail-inbox",
                            workspace::AppView::Notes => "rail-notes",
                            workspace::AppView::Automations => "rail-automations",
                        },
                        view.label(),
                        glyph,
                    )
                    .size(px(32.))
                    .relative()
                    .when(selected, |d| d.bg(content(0.1)))
                    .when(view == workspace::AppView::Inbox && inbox_unread, |d| {
                        d.child(
                            div()
                                .absolute()
                                .right(px(6.))
                                .top(px(6.))
                                .size(px(6.))
                                .rounded_full()
                                .bg(accent_color()),
                        )
                    })
                    .on_click(cx.listener(move |this, _, w, cx| {
                        if this.app_view_label() == Some(view.label()) {
                            this.close_app_view(w, cx);
                        } else {
                            match view {
                                workspace::AppView::Inbox => this.open_inbox(w, cx),
                                workspace::AppView::Notes => this.open_notes(w, cx),
                                workspace::AppView::Automations => this.open_automations(w, cx),
                            }
                        }
                    }))
                }),
            )
            .into_any()
    }
}
