use super::*;

impl Monos {
    pub(in crate::monos) fn topbar(&self, mono: &Mono, cx: &mut Context<Self>) -> AnyElement {
        let id = mono.id.clone();
        div()
            .absolute()
            .top_0()
            .right(px(10.))
            .h(px(40.))
            .flex()
            .items_center()
            .child(
                icon_button("mono-settings", "Mono settings", "settings")
                    .debug_selector(|| "mono-settings".into())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.settings = Some(SettingsSection::General);
                        window.focus(&this.settings_focus, cx);
                        this.reload_files(&id, cx);
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    pub(in crate::monos) fn select_overview(&mut self, tab: OverviewTab, cx: &mut Context<Self>) {
        if tab != OverviewTab::Changes {
            self.pause_changes(cx);
        }
        if self.route.is_some() {
            self.save_open_file(cx);
            self.route = None;
            cx.notify();
        }
        if self.overview_tab != tab {
            self.overview_reveal =
                Some(crate::project::tabs::TabReveal::new(tab, self.overview_tab));
            if tab != OverviewTab::Activity {
                self.overview_previous = tab;
            }
            self.overview_tab = tab;
            cx.notify();
        }
    }

    /// MonoSidebarHeader lives in the base page, so routed pages replace its
    /// tabs and close control together with the overview body.
    pub(in crate::monos) fn original_overview_header(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let owner = self.owner.clone();
        div()
            .id("original-mono-overview-header")
            .debug_selector(|| "original-mono-overview-header".into())
            .h(px(40.))
            .flex_shrink_0()
            // MonoSidebarHeader's pl-4 and OverviewTabs' -ml-2 combine
            // into an 8px inset, keeping the tab icons aligned with actions.
            .pl(px(8.))
            // The window's top-right corner: clear of client-drawn controls.
            .pr(px(12. + crate::ui::chrome::corner_controls_width(window)))
            .border_b_1()
            .border_color(content(0.07))
            .flex()
            .items_center()
            .gap(px(2.))
            .child(self.overview_controls(true, window, cx))
            .child(
                div()
                    .id("original-hide-mono-overview")
                    .debug_selector(|| "original-hide-mono-overview".into())
                    .role(Role::Button)
                    .aria_label("Hide overview")
                    .aria_toggled(gpui::Toggled::True)
                    .size(px(24.))
                    .flex_shrink_0()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(content(1.))
                    .hover(|s| s.bg(content(0.1)))
                    .child(icon("sidebar-right", 14.).text_color(content(1.)))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |_, _, _, cx| {
                        let owner = owner.clone();
                        cx.spawn(async move |_, cx| {
                            let _ = owner.update_in(cx, |shell, window, cx| {
                                shell.close_mono_overview(window, cx)
                            });
                        })
                        .detach();
                    })),
            )
            .into_any_element()
    }

    pub(crate) fn overview_controls(
        &self,
        expand_label: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(mono) = self.current() else {
            return div().into_any_element();
        };
        let progress = self
            .overview_reveal
            .as_ref()
            .and_then(|r| r.progress(self.overview_tab, window, cx));
        let tabs = [
            (OverviewTab::Overview, "Overview", "dashboard", 0),
            (
                OverviewTab::Sessions,
                "Sub-agents",
                "message-multiple",
                mono.launches.len(),
            ),
            (
                OverviewTab::Documents,
                "Documents",
                "file",
                mono.documents.len(),
            ),
            (OverviewTab::Habits, "Habits", "clock", self.habits.len()),
            (
                OverviewTab::Changes,
                "Changes",
                "file-diff",
                self.project_changes.count(&mono.id),
            ),
        ];
        div()
            .id("mono-overview-tabs")
            .debug_selector(|| "mono-overview-tabs".into())
            .role(Role::TabList)
            .aria_label("Mono details")
            .h(px(TITLE_BAR_HEIGHT))
            .flex_1()
            .min_w_0()
            .mr(px(2.))
            .flex()
            .items_center()
            .gap(px(2.))
            .children(tabs.into_iter().map(|(tab, label, glyph, count)| {
                let selected = self.overview_tab == tab;
                let unfolded = match progress {
                    Some((_, p)) if selected => p,
                    Some((closing, p)) if tab == closing => 1. - p,
                    _ if selected => 1.,
                    _ => 0.,
                };
                let id = match tab {
                    OverviewTab::Overview => "mono-overview-Overview",
                    OverviewTab::Sessions => "mono-overview-Sessions",
                    OverviewTab::Documents => "mono-overview-Documents",
                    OverviewTab::Habits => "mono-overview-Habits",
                    OverviewTab::Changes => "mono-overview-Changes",
                    OverviewTab::Activity => unreachable!(),
                };
                crate::project::tabs::tab_button(
                    id,
                    (label, count),
                    glyph,
                    selected,
                    tab == OverviewTab::Changes && count > 0 && !selected,
                    if expand_label { unfolded } else { 0. },
                    window,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.select_overview(tab, cx)))
            }))
            .child(crate::ui::chrome::titlebar_drag_area().flex_1().min_w_0())
            .when(self.overview_tab == OverviewTab::Habits, |d| {
                d.child(
                    icon_button("mono-new-habit", "New habit", "plus")
                        .debug_selector(|| "mono-new-habit".into())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.route = Some(Route::NewHabit);
                            this.clear_habit_fields();
                            cx.notify();
                        })),
                )
            })
            .child(
                icon_button("mono-activity", "Show activity", "list-bullet")
                    .debug_selector(|| "mono-activity".into())
                    .when(self.overview_tab == OverviewTab::Activity, |d| {
                        d.bg(content(0.1))
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        let tab = if this.overview_tab == OverviewTab::Activity {
                            this.overview_previous
                        } else {
                            OverviewTab::Activity
                        };
                        this.select_overview(tab, cx);
                    })),
            )
            .into_any_element()
    }

    pub(in crate::monos) fn overview_panel(
        &mut self,
        mono: &Mono,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.ensure_changes_summary(mono, cx);
        if self.overview_tab == OverviewTab::Changes {
            self.ensure_changes(mono, window, cx);
        } else {
            self.pause_changes(cx);
        }
        let body = match self.overview_tab {
            OverviewTab::Overview => self.dashboard(mono, window, cx),
            OverviewTab::Sessions => self.session_list(mono, cx),
            OverviewTab::Habits => self.habits_list(mono, cx),
            OverviewTab::Documents => self.document_list(mono, cx),
            OverviewTab::Changes => self.changes_list(mono, cx),
            OverviewTab::Activity => self.activity_view(mono, cx),
        };
        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(body)
            .into_any_element()
    }

    pub(super) fn session_list(&mut self, mono: &Mono, cx: &mut Context<Self>) -> AnyElement {
        let filter = self.field(
            "mono-session-search",
            "Search sub-agents…",
            String::new(),
            false,
            cx,
        );
        let query = filter.read(cx).content.to_lowercase();
        let views = self
            .owner
            .upgrade()
            .map(|owner| owner.read(cx).mono_session_views(cx))
            .unwrap_or_default();
        let mut launches: Vec<_> = mono
            .launches
            .iter()
            .rev()
            .filter(|l| {
                self.session_filters
                    .matches(l, views.iter().find(|v| v.id == l.session_id))
                    && format!(
                        "{} {} {}",
                        views
                            .iter()
                            .find(|v| v.id == l.session_id)
                            .map(|v| v.title.as_str())
                            .unwrap_or(&l.title),
                        views
                            .iter()
                            .find(|v| v.id == l.session_id)
                            .map(|v| v.cwd.as_str())
                            .unwrap_or(&l.cwd),
                        l.harness
                    )
                    .to_lowercase()
                    .contains(&query)
            })
            .cloned()
            .collect();
        // Most recent activity first: running or waiting sessions, then by
        // last update. Launch order alone puts re-attached old sessions on top.
        launches.sort_by_cached_key(|l| {
            std::cmp::Reverse(
                views
                    .iter()
                    .find(|v| v.id == l.session_id)
                    .map(|v| (v.busy || v.needs_input, v.updated_at)),
            )
        });
        let search_bar = search_bar(filter).child(
            icon_button("mono-session-filter", "Filter sub-agents", "filter")
                .relative()
                .child(self.session_filter_anchor.probe())
                .on_click(cx.listener(|this, _, window, cx| this.filter_menu(window, cx))),
        );
        let no_matches = launches.is_empty() && !mono.launches.is_empty();
        div().size_full().flex().flex_col()
            .when(!mono.launches.is_empty(),|d|d.child(search_bar))
            .child(div().id("mono-session-list").debug_selector(|| "mono-session-list".into()).flex_1().min_h_0().overflow_y_scroll().p_2().flex().flex_col().gap_1p5()
                .when(mono.launches.is_empty(),|d|d.child(empty_state("message-multiple",&format!("Ask {} to hand work to a sub-agent on one of your projects, like a bug fix or a review, and it shows up here.",mono.label()))))
                .when(no_matches,|d|d.child(empty_state("search-01",if query.trim().is_empty(){"No sub-agents match these filters."}else{"No sub-agents match this search."})))
                .children(launches.into_iter().map(|launch|{
                    let id = launch.session_id;
                    let state = views.iter().find(|v| v.id == id);
                    let title = state.map(|s| s.title.clone()).unwrap_or(launch.title.clone());
                    let cwd = state.map(|s| &s.cwd).unwrap_or(&launch.cwd);
                    let project = mono.projects.iter().find(|p| monocode_core::files::path_key(p) == monocode_core::files::path_key(cwd)).unwrap_or(cwd);
                    let project = std::path::Path::new(project).file_name().map_or(project.clone(), |n| n.to_string_lossy().to_string());
                    let branch = state.and_then(|s| s.branch.clone());
                    sub_agent_card(SubAgent {
                        id,
                        title,
                        harness: &launch.harness,
                        model: state.map(|s| s.model.clone()).filter(|m| !m.is_empty()),
                        model_id: launch.model.as_deref(),
                        place: match &branch { Some(b) => format!("{project} · {b}"), None => project },
                        branch: branch.is_some(),
                        state: match state {
                            Some(s) if s.needs_input => AgentState::NeedsInput,
                            Some(s) if s.busy => AgentState::Working,
                            Some(s) => AgentState::Idle(s.updated_at),
                            None => AgentState::Unavailable,
                        },
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.route = Some(Route::Session(id));
                        cx.notify();
                    }))
}))).into_any_element()
    }

    pub(super) fn document_list(&mut self, mono: &Mono, cx: &mut Context<Self>) -> AnyElement {
        let filter = self.field(
            "mono-document-search",
            "Search documents…",
            String::new(),
            false,
            cx,
        );
        let query = filter.read(cx).content.to_string();
        let documents: Vec<_> = mono
            .documents
            .iter()
            .rev()
            .filter(|d| matches_query(&query, &format!("{} {}", d.title, d.summary)))
            .collect();
        let list = div()
            .id("mono-document-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_2()
            .flex()
            .flex_col()
            .gap_1p5()
            .when(mono.documents.is_empty(), |d| {
                d.child(empty_state(
                    "file",
                    &format!(
                        "Ask {} to write something up, like a plan or a summary, and it shows up here.",
                        mono.label()
                    ),
                ))
            })
            .when(documents.is_empty() && !mono.documents.is_empty(), |d| {
                d.child(empty_state("search-01", "No documents match this search."))
            })
            .children(documents.into_iter().map(|document| {
                // MonoOverview's DocumentRow: laid out like a habit.
                let id = document.id.clone();
                let visual = matches!(document.kind.as_str(), "html" | "visual");
                let label = if visual { "Visual" } else { "Document" };
                outlined(div().id(SharedString::from(format!("mono-document-{id}"))))
                    .debug_selector({
                        let id = id.clone();
                        move || format!("mono-document-{id}")
                    })
                    .role(Role::Button)
                    .aria_label(format!("Open {} {}", label.to_lowercase(), document.title))
                    .w_full()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .flex_shrink_0()
                    .px(px(10.))
                    .py_2()
                    .child(
                        div()
                            .size(px(28.))
                            .flex_shrink_0()
                            .rounded_md()
                            .bg(content(0.06))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                icon(if visual { "chart-square" } else { "file" }, 14.)
                                .text_color(content(0.6)),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(12.))
                                    .line_height(px(20.))
                                    .text_color(content(0.85))
                                    .child(document.title.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.))
                                    .line_height(px(16.))
                                    .text_color(content(0.4))
                                    .child(if document.summary.trim().is_empty() {
                                        label.to_string()
                                    } else {
                                        document.summary.clone()
                                    }),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_document(&id, cx);
                        this.route = Some(Route::Document(id.clone()));
                        cx.notify();
                    }))
            }));
        div()
            .size_full()
            .flex()
            .flex_col()
            .when(!mono.documents.is_empty(), |d| d.child(search_bar(filter)))
            .child(list)
            .into_any_element()
    }

    pub(in crate::monos) fn page_header(&self, title: &str, cx: &mut Context<Self>) -> Div {
        div()
            .h(px(40.))
            .flex_shrink_0()
            .min_w_0()
            .overflow_hidden()
            .border_b_1()
            .border_color(content(0.1))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .child(
                icon_button("mono-page-back", "Back", "chevron-left")
                    .debug_selector(|| "mono-page-back".into())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.route = None;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(title.to_string()),
            )
    }

    pub(in crate::monos) fn route_page(
        &mut self,
        mono: &Mono,
        route: Route,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let interactive = self.route.as_ref() == Some(&route);
        match route {
            Route::Review => div()
                .size_full()
                .flex()
                .flex_col()
                .child(self.page_header("Changes", cx))
                .when_some(self.review.clone(), |d, review| {
                    d.child(div().flex_1().min_h_0().child(review))
                })
                .into_any_element(),
            Route::Session(id) => {
                let view = self.owner.upgrade().and_then(|owner| {
                    owner.update(cx, |owner, cx| owner.mono_ensure_session(id, cx))
                });
                if let Some(view) = &view {
                    view.update(cx, |v, cx| v.load(cx));
                }
                let title = view
                    .as_ref()
                    .map(|v| v.read(cx).summary.title.clone())
                    .unwrap_or_else(|| "Session unavailable".into());
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(
                        self.page_header(&title, cx).child(
                            icon_button(
                                "mono-open-session",
                                "Open session in workspace",
                                "external-link",
                            )
                            .on_click(
                                cx.listener(move |this, _, w, cx| this.open_workspace(id, w, cx)),
                            ),
                        ),
                    )
                    .when_some(view, |d, view| {
                        d.child(div().flex_1().min_h_0().child(view))
                    })
                    .into_any_element()
            }
            Route::Document(id) => {
                let Some(document) = self
                    .work
                    .documents
                    .get(&id)
                    .or_else(|| {
                        mono.documents.iter().find(|d| {
                            d.id == id
                                && (d.kind == "visual" || !mono.desktop_document_ids.contains(&id))
                        })
                    })
                    .cloned()
                else {
                    let title = mono
                        .documents
                        .iter()
                        .find(|d| d.id == id)
                        .map_or("Document", |d| d.title.as_str());
                    return div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .child(self.page_header(title, cx))
                        .child(empty(if self.work.loading_documents.contains(&id) {
                            "Loading document…"
                        } else {
                            "Document unavailable"
                        }))
                        .into_any_element();
                };
                let view: AnyElement = if document.kind == "html"
                    && (self.settings.is_some()
                        || self.menu.is_some()
                        || self.picker.is_some()
                        || self.background.is_some())
                {
                    div().into_any_element()
                } else if document.kind == "html" {
                    let html = self
                        .html_documents
                        .entry(id.clone())
                        .or_insert_with(|| {
                            cx.new(|_| super::super::html::Html::new(document.body.clone()))
                        })
                        .clone();
                    html.update(cx, |html, _| html.set_interactive(interactive));
                    html.into_any_element()
                } else if document.kind == "visual" {
                    // The same native dither visual the chat shows, on its own page.
                    let visual = match self.visual_documents.get(&id) {
                        Some(view) => Some(view.clone()),
                        None => serde_json::from_str(&document.body).ok().map(|visual| {
                            // The page header already titles it.
                            let view = cx.new(|cx| {
                                let mut view = crate::chat::visuals::VisualView::new(visual, cx);
                                view.set_inline(Some(String::new()), cx);
                                view
                            });
                            self.visual_documents.insert(id.clone(), view.clone());
                            view
                        }),
                    };
                    match visual {
                        Some(view) => div()
                            .id("mono-visual-document")
                            .size_full()
                            .overflow_y_scroll()
                            .px_4()
                            .py_3()
                            .child(view)
                            .into_any_element(),
                        None => empty("Visual unavailable"),
                    }
                } else {
                    self.documents
                        .entry(id.clone())
                        .or_insert_with(|| {
                            cx.new(|cx| {
                                crate::chat::transcript::TranscriptView::document(
                                    document.body.clone(),
                                    cx,
                                )
                            })
                        })
                        .clone()
                        .into_any_element()
                };
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(self.page_header(&document.title, cx).when(
                        document.kind != "visual",
                        |header| {
                            header.child(
                                icon_button("mono-document-copy", "Copy document", "response-copy")
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                            document.body.clone(),
                                        ))
                                    }),
                            )
                        },
                    ))
                    .child(div().flex_1().min_h_0().child(view))
                    .into_any_element()
            }
            Route::Habit(id) => self.habit_page(mono, Some(&id), window, cx),
            Route::NewHabit => self.habit_page(mono, None, window, cx),
        }
    }

    pub(super) fn activity_view(&mut self, mono: &Mono, cx: &mut Context<Self>) -> AnyElement {
        let Some(chat) = self.chat() else {
            return empty("Loading…");
        };
        // Response actions number turns across every loaded page.
        let source = chat.read(cx).transcript.read(cx).history_rows();
        let Some(start) = self.activity_turn.filter(|i| *i < source.len()) else {
            return empty_state(
                "list-bullet",
                &format!(
                    "Select the activity on one of {}’s messages to see each step it took.",
                    mono.label()
                ),
            );
        };
        let end = source
            .iter()
            .enumerate()
            .skip(start + 1)
            .find(|(_, r)| r.is_prompt() && r.first)
            .map(|(i, _)| i)
            .unwrap_or(source.len());
        let rows = source[start..end]
            .iter()
            .filter(|r| matches!(r.role.as_str(), "tool" | "reasoning"))
            .cloned()
            .collect::<Vec<_>>();
        if rows.is_empty() {
            return empty_state("list-bullet", "No activity in this turn yet.");
        }
        let key = mono::files::content_hash(&format!("{rows:?}"));
        if self.activity.as_ref().is_none_or(|(old, _)| old != &key) {
            self.activity = Some((
                key,
                cx.new(|cx| crate::chat::transcript::TranscriptView::activity(rows, cx)),
            ));
        }
        div()
            .size_full()
            .child(self.activity.as_ref().unwrap().1.clone())
            .into_any_element()
    }
}
