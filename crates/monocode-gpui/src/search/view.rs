use super::*;

pub(super) fn empty_state() -> AnyElement {
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .px_6()
                .pb(px(96.))
                .child(
                    div()
                        .relative()
                        .w(px(288.))
                        .h(px(192.))
                        .mb_2()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            canvas(
                                |_, _, _| (),
                                |bounds, _, window, _| {
                                    for row in 0..19 {
                                        for col in 0..27 {
                                            let x = (col as f32 - 13.) * 10.;
                                            let y = (row as f32 - 9.) * 10.;
                                            let radius =
                                                ((x / 144.).powi(2) + (y / 96.).powi(2)).sqrt();
                                            let alpha =
                                                0.14 * ((0.76 - radius) / 0.58).clamp(0., 1.);
                                            window.paint_quad(
                                                fill(
                                                    Bounds::new(
                                                        bounds.center()
                                                            + point(px(x - 1.5), px(y - 1.5)),
                                                        size(px(3.), px(3.)),
                                                    ),
                                                    content(alpha),
                                                )
                                                .corner_radii(px(1.5)),
                                            );
                                        }
                                    }
                                },
                            )
                            .absolute()
                            .size_full(),
                        )
                        .child(
                            div()
                                .size(px(56.))
                                .rounded(px(16.))
                                .relative()
                                .child(
                                    canvas(
                                        |_, _, _| (),
                                        |bounds, _, window, _| {
                                            let quad = fill(bounds, content(0.06));
                                            #[cfg(any(target_os = "macos", target_os = "windows"))]
                                            let quad =
                                                quad.border_color(gpui::backdrop_blur_marker(
                                                    4. * window.scale_factor(),
                                                ));
                                            window.paint_quad(quad.corner_radii(px(16.)));
                                        },
                                    )
                                    .absolute()
                                    .size_full(),
                                )
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon("search", 24.).text_color(content(0.5))),
                        ),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .line_height(px(19.5))
                        .max_w(px(320.))
                        .text_center()
                        .text_color(content(0.45))
                        .child("Find files, conversations, messages, and projects."),
                ),
        )
        .into_any_element()
}

pub(super) fn empty_message(text: String) -> AnyElement {
    div()
        .py(px(36.))
        .flex()
        .flex_col()
        .items_center()
        .gap_2()
        .child(
            div()
                .size(px(40.))
                .rounded(px(12.))
                .bg(content(0.05))
                .flex()
                .items_center()
                .justify_center()
                .child(icon("search", 18.).text_color(content(0.4))),
        )
        .child(
            div()
                .text_size(px(13.))
                .text_color(content(0.5))
                .child(text),
        )
        .into_any_element()
}

impl SearchView {
    /// What the panel offers before anything is typed: where you were last.
    pub(super) fn suggestions(&self) -> Vec<Hit> {
        let mut hits = vec![];
        let (conversations, files, projects) = match self.scope {
            Scope::All => (5, 4, 0),
            Scope::Conversations => (10, 0, 0),
            Scope::Files => (0, 10, 0),
            Scope::Projects => (0, 0, 12),
        };
        let mut recent: Vec<_> = self
            .summaries
            .iter()
            .filter(|s| !s.archived && s.has_messages)
            .collect();
        recent.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(b.id.cmp(&a.id)));
        hits.extend(recent.into_iter().take(conversations).map(|s| Hit {
            id: format!("conversation:{}", s.id),
            target: Target::Conversation(s.id),
            title: monocode_core::session::title::display_title(&s.title, &s.provider).into(),
            meta: search::project_name(&s.cwd),
            provider: s.provider.clone(),
            highlights: vec![],
            score: 0,
            updated_at: s.updated_at,
        }));
        hits.extend(
            self.recent
                .iter()
                .filter_map(|path| self.files.iter().find(|f| &f.path == path))
                .take(files)
                .map(|f| Hit {
                    id: format!("file:{}", f.path),
                    target: Target::File {
                        path: f.path.clone(),
                        location: None,
                    },
                    title: f.name.clone(),
                    meta: f.relative.clone(),
                    provider: String::new(),
                    highlights: vec![],
                    score: 0,
                    updated_at: 0,
                }),
        );
        hits.extend(self.projects.iter().take(projects).map(|path| Hit {
            id: format!("project:{path}"),
            target: Target::Project(path.clone()),
            title: search::project_name(path),
            meta: path.clone(),
            provider: String::new(),
            highlights: vec![],
            score: 0,
            updated_at: 0,
        }));
        hits
    }

    pub(super) fn row(&self, ix: usize, hit: &Hit, cx: &mut Context<Self>) -> AnyElement {
        let active = self.active == ix;
        let mark = match &hit.target {
            Target::Conversation(_) => provider_icon(&hit.provider, 14.).into_any_element(),
            Target::Message { .. } => icon("message-square", 14.)
                .text_color(content(0.6))
                .into_any_element(),
            Target::File { path, .. } => crate::ui::file_icons::file_type_icon(
                &search::project_name(path),
                false,
                false,
                15.,
            ),
            Target::Project(path) => self
                .logos
                .get(path)
                .map(|logo| {
                    crate::ui::image_memory::thumbnail(std::path::PathBuf::from(logo))
                        .size(px(16.))
                        .rounded_sm()
                        .into_any_element()
                })
                .unwrap_or_else(|| {
                    icon("folder", 14.)
                        .text_color(content(0.6))
                        .into_any_element()
                }),
        };
        let text = StyledText::new(hit.title.clone()).with_highlights(
            hit.highlights.iter().cloned().map(|range| {
                (
                    range,
                    HighlightStyle {
                        font_weight: Some(gpui::FontWeight::SEMIBOLD),
                        color: Some(accent_color()),
                        ..Default::default()
                    },
                )
            }),
        );
        let selection = if is_light() { 0.07 } else { 0.09 };
        div()
            .id(("app-search-row", ix))
            .debug_selector(move || format!("app-search-row-{ix}"))
            .role(Role::ListItem)
            .aria_selected(active)
            .aria_label(format!("{} {}", hit.title, hit.meta))
            .h(px(38.))
            .w_full()
            .flex_shrink_0()
            .relative()
            .rounded(px(8.))
            .pl(px(8.))
            .pr(px(10.))
            .flex()
            .gap(px(10.))
            .items_center()
            .text_size(px(13.5))
            .cursor_pointer()
            .when(active, |d| d.bg(content(selection)))
            .when(active, |d| {
                d.child(
                    div()
                        .absolute()
                        .left(px(0.))
                        .top(px(10.))
                        .w(px(2.5))
                        .h(px(18.))
                        .rounded_full()
                        .bg(accent_color()),
                )
            })
            .child(
                div()
                    .size(px(26.))
                    .flex_shrink_0()
                    .rounded(px(7.))
                    .bg(content(if active { 0.08 } else { 0.045 }))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(mark),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_color(content(if active { 1. } else { 0.86 }))
                    .child(text),
            )
            .child(
                div()
                    .min_w_0()
                    .max_w(relative(0.45))
                    .truncate()
                    .text_size(px(12.))
                    .text_color(content(if active { 0.5 } else { 0.38 }))
                    .child(hit.meta.clone()),
            )
            .when(active, |d| d.child(keycap("↵")))
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                if this.pointer.is_some_and(|old| old != event.position) && this.active != ix {
                    this.active = ix;
                    cx.notify();
                }
                this.pointer = Some(event.position);
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| this.pick(ix, cx)))
            .into_any_element()
    }
}

impl SearchView {
    /// The original page: a title-strip field, scope tabs and full-height rows.
    pub(super) fn render_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let rows: Vec<_> = self
            .hits
            .iter()
            .enumerate()
            .map(|(ix, hit)| {
                let mark = match &hit.target {
                    Target::Conversation(_) => provider_icon(&hit.provider, 14.).into_any_element(),
                    Target::Message { .. } => icon("message-square", 14.)
                        .text_color(content(0.55))
                        .into_any_element(),
                    Target::File { path, .. } => crate::ui::file_icons::file_type_icon(
                        &search::project_name(path),
                        false,
                        false,
                        16.,
                    ),
                    Target::Project(path) => self
                        .logos
                        .get(path)
                        .map(|logo| {
                            crate::ui::image_memory::thumbnail(std::path::PathBuf::from(logo))
                                .size(px(14.))
                                .rounded_sm()
                                .into_any_element()
                        })
                        .unwrap_or_else(|| icon("folder", 14.).into_any_element()),
                };
                let text = StyledText::new(hit.title.clone()).with_highlights(
                    hit.highlights.iter().cloned().map(|range| {
                        (
                            range,
                            HighlightStyle {
                                font_weight: None,
                                color: Some(accent_color()),
                                ..Default::default()
                            },
                        )
                    }),
                );
                div()
                    .id(("app-search-row", ix))
                    .debug_selector(move || format!("app-search-row-{ix}"))
                    .role(Role::ListItem)
                    .aria_selected(self.active == ix)
                    .aria_label(format!("{} {}", hit.title, hit.meta))
                    .h(px(32.))
                    .w_full()
                    .flex_shrink_0()
                    .rounded_md()
                    .px_2()
                    .flex()
                    .gap_2()
                    .items_center()
                    .text_size(px(13.))
                    .when(self.active == ix, |d| {
                        d.bg(content(if content(1.).l < 0.5 { 0.06 } else { 0.10 }))
                    })
                    .child(
                        div()
                            .size(px(16.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(mark),
                    )
                    .child(div().min_w_0().flex_1().truncate().child(text))
                    .child(
                        div()
                            .min_w_0()
                            .max_w(relative(0.45))
                            .truncate()
                            .font_family("monospace")
                            .text_size(px(11.))
                            .text_color(content(0.4))
                            .child(hit.meta.clone()),
                    )
                    .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                        if this.pointer.is_some_and(|old| old != event.position)
                            && this.active != ix
                        {
                            this.active = ix;
                            cx.notify();
                        }
                        this.pointer = Some(event.position);
                    }))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, _, cx| this.pick(ix, cx)))
                    .into_any_element()
            })
            .collect();
        let header = if self.original_header.is_some() {
            crate::ui::chrome::titlebar_drag_area()
        } else {
            div()
        };
        let loading = self.sessions_loading || self.files_loading;
        let stroke = content(if self.classic { 0.07 } else { 0.04 });
        let selection = if content(1.).l < 0.5 { 0.06 } else { 0.10 };
        div()
            .id("app-search")
            .key_context("Workspace")
            .debug_selector(|| "app-search".into())
            .aria_label("Search")
            .size_full()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .text_color(content(1.))
            .capture_action(cx.listener(|this, _: &input::Down, _, cx| {
                this.step(1, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &input::Up, _, cx| {
                this.step(-1, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &input::Submit, _, cx| {
                this.pick(this.active, cx);
                cx.stop_propagation();
            }))
            .on_action(
                cx.listener(|this, _: &crate::ui::popover::DismissMenu, _, cx| {
                    if !this.input.read(cx).is_composing() {
                        cx.emit(Event::Close);
                    }
                    cx.stop_propagation();
                }),
            )
            .child(
                header
                    .id("app-search-header")
                    .debug_selector(|| "app-search-header".into())
                    .h(px(40.))
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(stroke)
                    .pl(px(self.original_header.unwrap_or(12.)))
                    .pr(px(12.
                        + if self.original_header.is_some() {
                            crate::ui::chrome::client_controls_width(window)
                        } else {
                            0.
                        }))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(icon("search", 14.).text_color(content(0.5)))
                    .child(
                        div()
                            .debug_selector(|| "app-search-field".into())
                            .flex_1()
                            .min_w_0()
                            .when(self.original_header.is_some(), |d| {
                                d.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            })
                            .child(self.input.clone()),
                    )
                    .when(loading, |d| {
                        d.child(crate::ui::panel::spinner(
                            "app-search-loading",
                            "loader",
                            14.,
                        ))
                    }),
            )
            .child(
                div()
                    .id("app-search-scopes")
                    .debug_selector(|| "app-search-scopes".into())
                    .h(px(36.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(1.))
                    .px_3()
                    .border_b_1()
                    .border_color(stroke)
                    .children(
                        Scope::ALL
                            .into_iter()
                            .enumerate()
                            .map(|(ix, (scope, label))| {
                                div()
                                    .id(("app-search-scope", ix))
                                    .debug_selector(move || format!("app-search-scope-{ix}"))
                                    .role(Role::Button)
                                    .aria_label(label)
                                    .child(label)
                                    .aria_selected(self.scope == scope)
                                    .rounded_md()
                                    .px_2()
                                    .py_1()
                                    .text_size(px(12.))
                                    .line_height(px(18.))
                                    .bg(content(if self.scope == scope { selection } else { 0. }))
                                    .text_color(content(if self.scope == scope { 1. } else { 0.5 }))
                                    .hover(|s| s.bg(content(0.05)).text_color(content(1.)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.select_scope(scope, cx);
                                    }))
                            }),
                    ),
            )
            .child(if self.query.is_empty() {
                empty_state()
            } else {
                div()
                    .id("app-search-results")
                    .role(Role::ListBox)
                    .aria_label("Search results")
                    .flex_1()
                    .min_h_0()
                    .p_1p5()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .when(self.truncated.0 || self.truncated.1, |d| {
                        d.child(
                            div()
                                .px_2p5()
                                .py_1()
                                .text_size(px(11.))
                                .text_color(content(0.45))
                                .child("Results limited to the first matches"),
                        )
                    })
                    .children(rows)
                    .when(self.hits.is_empty() && !loading, |d| {
                        d.child(
                            div()
                                .px_2()
                                .py_1p5()
                                .text_size(px(12.))
                                .text_color(content(0.5))
                                .child(self.error.clone().unwrap_or_else(|| "No results".into())),
                        )
                    })
                    .into_any_element()
            })
    }

    pub(super) fn render_panel(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let suggesting = self.query.is_empty();
        let mut list: Vec<AnyElement> = vec![];
        if self.truncated.0 || self.truncated.1 {
            list.push(
                div()
                    .px(px(10.))
                    .pt_1()
                    .text_size(px(11.))
                    .text_color(content(0.4))
                    .child("Showing the first matches")
                    .into_any_element(),
            );
        }
        let mut last = None;
        let hits = std::mem::take(&mut self.hits);
        for (ix, hit) in hits.iter().enumerate() {
            let heading = section(&hit.target, suggesting);
            if last != Some(heading) {
                last = Some(heading);
                list.push(
                    div()
                        .flex_shrink_0()
                        .px(px(10.))
                        .pt(px(if ix == 0 { 6. } else { 12. }))
                        .pb(px(5.))
                        .text_size(px(11.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(content(0.42))
                        .child(heading)
                        .into_any_element(),
                );
            }
            list.push(self.row(ix, hit, cx));
        }
        self.hits = hits;
        let loading = self.sessions_loading || self.files_loading;
        let count = self.hits.len();
        div()
            .id("app-search")
            .key_context("Workspace")
            .debug_selector(|| "app-search".into())
            .role(Role::Dialog)
            .aria_label("Search")
            .w_full()
            .flex()
            .flex_col()
            .text_color(content(1.))
            .capture_action(cx.listener(|this, _: &input::Down, _, cx| {
                this.step(1, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &input::Up, _, cx| {
                this.step(-1, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &input::Submit, _, cx| {
                this.pick(this.active, cx);
                cx.stop_propagation();
            }))
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                let k = &event.keystroke;
                if k.key == "tab" && !k.modifiers.platform && !k.modifiers.control {
                    this.cycle_scope(if k.modifiers.shift { -1 } else { 1 }, cx);
                    cx.stop_propagation();
                }
            }))
            .on_action(
                cx.listener(|this, _: &crate::ui::popover::DismissMenu, _, cx| {
                    if !this.input.read(cx).is_composing() {
                        cx.emit(Event::Close);
                    }
                    cx.stop_propagation();
                }),
            )
            .child(
                div()
                    .id("app-search-header")
                    .debug_selector(|| "app-search-header".into())
                    .h(px(56.))
                    .flex_shrink_0()
                    .px(px(18.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(icon("search", 18.).text_color(content(0.45)))
                    .child(
                        div()
                            .debug_selector(|| "app-search-field".into())
                            .flex_1()
                            .min_w_0()
                            .child(self.input.clone()),
                    )
                    .when(loading, |d| {
                        d.child(crate::ui::panel::spinner(
                            "app-search-loading",
                            "loader",
                            14.,
                        ))
                    })
                    .child(keycap("esc")),
            )
            .child(
                div()
                    .id("app-search-scopes")
                    .debug_selector(|| "app-search-scopes".into())
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .px(px(12.))
                    .pb(px(10.))
                    .border_b_1()
                    .border_color(content(0.07))
                    .children(
                        Scope::ALL
                            .into_iter()
                            .enumerate()
                            .map(|(ix, (scope, label))| {
                                let selected = self.scope == scope;
                                let glyph = match scope {
                                    Scope::All => "sparkles",
                                    Scope::Conversations => "message-square",
                                    Scope::Files => "file",
                                    Scope::Projects => "folder",
                                };
                                div()
                                    .id(("app-search-scope", ix))
                                    .debug_selector(move || format!("app-search-scope-{ix}"))
                                    .role(Role::Button)
                                    .aria_label(label)
                                    .aria_selected(selected)
                                    .cursor_pointer()
                                    .h(px(26.))
                                    .px(px(10.))
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .rounded_full()
                                    .border_1()
                                    .border_color(if selected {
                                        accent_color().opacity(0.45)
                                    } else {
                                        content(0.08)
                                    })
                                    .bg(if selected {
                                        accent_color().opacity(0.14)
                                    } else {
                                        content(0.)
                                    })
                                    .text_size(px(12.))
                                    .text_color(content(if selected { 1. } else { 0.55 }))
                                    .hover(|s| s.bg(content(0.05)).text_color(content(0.95)))
                                    .child(icon(glyph, 12.).text_color(if selected {
                                        accent_color()
                                    } else {
                                        content(0.45)
                                    }))
                                    .child(label)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.select_scope(scope, cx);
                                    }))
                            }),
                    ),
            )
            .child(
                div()
                    .id("app-search-results")
                    .role(Role::ListBox)
                    .aria_label("Search results")
                    .h(px(400.))
                    .px(px(8.))
                    .py(px(6.))
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(list)
                    .when(self.hits.is_empty() && !(loading && !suggesting), |d| {
                        d.child(empty_message(if suggesting {
                            match self.scope {
                                Scope::Projects => "No projects yet".into(),
                                _ => "Type to search conversations, files and projects".into(),
                            }
                        } else {
                            self.error
                                .clone()
                                .unwrap_or_else(|| format!("No results for “{}”", self.query))
                        }))
                    }),
            )
            .child(
                div()
                    .h(px(36.))
                    .flex_shrink_0()
                    .px(px(14.))
                    .border_t_1()
                    .border_color(content(0.07))
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .text_size(px(11.))
                    .text_color(content(0.45))
                    .child(hint(&["↑", "↓"], "Navigate"))
                    .child(hint(&["↵"], "Open"))
                    .child(hint(&["⇥"], "Switch scope"))
                    .child(div().flex_1())
                    .when(!suggesting && count > 0, |d| {
                        d.child(format!(
                            "{count} result{}",
                            if count == 1 { "" } else { "s" }
                        ))
                    }),
            )
    }
}
