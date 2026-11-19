use super::*;

impl Monos {
    pub(crate) fn overview_chrome(&self) -> OverviewChrome {
        let counts = self.current().map_or((0, 0, 0, 0), |mono| {
            (
                mono.launches.len(),
                mono.documents.len(),
                self.habits.len(),
                self.project_changes.count(&mono.id),
            )
        });
        OverviewChrome {
            active: self.active.clone(),
            tab: self.overview_tab,
            routed: self.route.is_some(),
            counts,
        }
    }

    /// TitleBar.tsx's MonoTitle, used by the original full-width Mono view.
    pub(crate) fn original_title(&self, inset: f32, cx: &App) -> AnyElement {
        let Some(mono) = self.current() else {
            return div().flex_1().min_w_0().into_any_element();
        };
        let state = self
            .chats
            .get(&mono.id)
            .map(|chat| chat.read(cx).mono_status())
            .unwrap_or("idle");
        let status = match state {
            "working" => "Working",
            "needs-you" => "Needs you",
            _ => "Idle",
        };
        div()
            .id("original-mono-title")
            .debug_selector(|| "original-mono-title".into())
            .flex_1()
            .min_w_0()
            .h_full()
            .pl(px(inset))
            .pr_4()
            .flex()
            .items_center()
            .gap_2()
            .child(crate::ui::avatar::avatar(
                &mono.mascot,
                mono.color,
                state,
                16.,
                state == "idle",
            ))
            .child(
                div()
                    .debug_selector(|| "original-mono-name".into())
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(content(1.))
                    .child(mono.label()),
            )
            .child(
                div()
                    .debug_selector(|| "original-mono-status".into())
                    .flex_shrink_0()
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::NORMAL)
                    .text_color(if state == "needs-you" {
                        accent_color()
                    } else {
                        content(0.45)
                    })
                    .child(status),
            )
            .into_any_element()
    }

    pub(crate) fn rail(&mut self, project: Option<String>, cx: &mut Context<Self>) -> AnyElement {
        self.rail_impl(project, false, false, cx)
    }

    /// Original navigation lists every Mono, before the projects; the inset
    /// layout continues to list only the current project's Monos.
    pub(crate) fn project_navigation_rail(
        &mut self,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.rail_impl(None, true, compact, cx)
    }

    pub(super) fn rail_impl(
        &mut self,
        project: Option<String>,
        original: bool,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.prefs.monos {
            return div().into_any_element();
        }
        let roster: Vec<_> = self
            .roster
            .monos
            .iter()
            .filter(|m| project.as_ref().is_none_or(|p| m.works_on(p)))
            .cloned()
            .collect();
        self.rail_ids = roster.iter().map(|m| m.id.clone()).collect();
        for mono in &roster {
            self.rail_bounds.entry(mono.id.clone()).or_default();
        }
        let count = roster.len();
        let has = count > 0;
        if original && !has && compact {
            return div().into_any_element();
        }
        let add_project = project.clone();
        let heading = if original {
            div()
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
                            .child("Monos"),
                    )
                })
        } else {
            section_label("Monos")
        }
        .when(has, |d| {
            d.child(
                icon_button("new-mono", "New mono", "plus")
                    .debug_selector(|| "sidebar-mono-button".into())
                    .size(px(20.))
                    // Centred on the sidebar's trailing column, with the
                    // session filter and the navigation hints and counts.
                    .when(!original, |d| d.mr(px(-3.)))
                    .on_click(cx.listener(move |this, _, w, cx| {
                        this.create(add_project.clone().into_iter().collect(), w, cx)
                    })),
            )
        });
        div()
            .when(!original, |d| d.px_2().pb_3())
            .when(original && !compact, |d| d.pb_2())
            .when(compact, |d| {
                d.w_full().flex().flex_col().items_center().gap(px(6.))
            })
            .when(!compact, |d| d.child(heading))
            .when(!has, |d| {
                d.child(
                    div()
                        .id("new-mono-row")
                        .debug_selector(|| "new-mono".into())
                        .role(Role::Button)
                        .aria_label("New mono")
                        .h(px(32.))
                        .when(original, |d| {
                            d.ml(px(6.)).mr_2().font_weight(gpui::FontWeight::MEDIUM)
                        })
                        .px_2()
                        .rounded_md()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(14.))
                        .text_color(content(0.5))
                        .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
                        .child(icon("plus", 16.))
                        .child("New mono")
                        .on_click(cx.listener(move |this, _, w, cx| {
                            this.create(project.clone().into_iter().collect(), w, cx)
                        })),
                )
            })
            .children(roster.into_iter().enumerate().map(|(index, mono)| {
                let active = self.active.as_deref() == Some(&mono.id);
                let state = self
                    .chats
                    .get(&mono.id)
                    .map(|v| v.read(cx).mono_status())
                    .unwrap_or("idle");
                let unread = self
                    .chats
                    .get(&mono.id)
                    .map(|v| {
                        v.read(cx)
                            .mono_rows(cx)
                            .iter()
                            .skip(mono.seen_rows)
                            .filter(|r| {
                                r.role == "assistant" && r.first && !r.text.trim().is_empty()
                            })
                            .count()
                    })
                    .unwrap_or(0);
                // Like MonoRailSection, an open Mono has already been seen.
                // Activity belongs on the mascot, rather than an idle dot at
                // the end of every row.
                let unread = if active { 0 } else { unread };
                let label = if unread == 0 {
                    format!("{}, {state}", mono.label())
                } else {
                    format!(
                        "{}, {state}, {unread} new {}",
                        mono.label(),
                        if unread == 1 { "reply" } else { "replies" }
                    )
                };
                let id = mono.id.clone();
                let options = id.clone();
                let context = id.clone();
                let drag = id.clone();
                let slot = self.rail_bounds[&id].clone();
                div()
                    .id(SharedString::from(format!("mono-rail-{id}")))
                    .debug_selector({
                        let id = id.clone();
                        move || format!("mono-rail-{id}")
                    })
                    .group("mono-rail-row")
                    .role(Role::Button)
                    .aria_label(label)
                    .h(px(32.))
                    .when(original && !compact, |d| d.ml(px(6.)).mr_2())
                    .when(original && !compact && index + 1 < count, |d| d.mb_px())
                    .when(compact, |d| d.w(px(32.)))
                    .px_2()
                    .rounded_md()
                    .relative()
                    .flex()
                    .items_center()
                    .gap_2()
                    .opacity(if active { 1. } else { 0.65 })
                    .when(active, |d| d.bg(content(0.1)))
                    .hover(|s| s.bg(content(0.08)).opacity(1.))
                    .child(
                        div()
                            .size(px(16.))
                            .relative()
                            .flex_shrink_0()
                            .child(crate::ui::avatar::avatar(
                                &mono.mascot,
                                mono.color,
                                state,
                                16.,
                                true,
                            ))
                            .when(original, |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .right(px(-2.))
                                        .top(px(-2.))
                                        .size(px(6.))
                                        .rounded_full()
                                        .border_1()
                                        .border_color(rgb(crate::ui::theme::BACKGROUND_BASE))
                                        .bg(if state == "working" {
                                            rgb(mono.color).into()
                                        } else if state == "needs-you" {
                                            accent_color()
                                        } else {
                                            content(0.3)
                                        }),
                                )
                            }),
                    )
                    .when(!compact, |d| {
                        d.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .when(unread > 0, |d| d.pr(px(24.)))
                                .group_hover("mono-rail-row", |s| s.pr(px(24.)))
                                .text_size(px(14.))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child(mono.label()),
                        )
                    })
                    .when(unread > 0 && !compact, |d| {
                        d.child(
                            div()
                                .absolute()
                                .right_1()
                                .top(px(4.))
                                .size(px(24.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .group_hover("mono-rail-row", |s| s.invisible())
                                .child(
                                    div()
                                        .debug_selector({
                                            let id = mono.id.clone();
                                            move || format!("mono-rail-unread-{id}")
                                        })
                                        .when(!original, |d| d.size(px(6.)))
                                        .when(original, |d| {
                                            d.h(px(16.))
                                                .min_w(px(16.))
                                                .px_1()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .text_size(px(10.))
                                                .font_weight(gpui::FontWeight::MEDIUM)
                                                .text_color(rgb(0xffffff))
                                                .child(if unread > 99 {
                                                    "99+".into()
                                                } else {
                                                    unread.to_string()
                                                })
                                        })
                                        .rounded_full()
                                        .bg(accent_color()),
                                ),
                        )
                    })
                    .when(!compact, |d| {
                        d.child(
                            div()
                                .absolute()
                                .right_1()
                                .top(px(4.))
                                .invisible()
                                .group_hover("mono-rail-row", |s| s.visible())
                                .child(
                                    icon_button("mono-options", "Mono options", "more-horizontal")
                                        .debug_selector({
                                            let id = mono.id.clone();
                                            move || format!("mono-rail-options-{id}")
                                        })
                                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                            cx.stop_propagation()
                                        })
                                        .on_click(cx.listener(move |this, _, w, cx| {
                                            cx.stop_propagation();
                                            this.open_options(&options, None, w, cx);
                                        })),
                                ),
                        )
                    })
                    .child(slot.probe())
                    .on_click(cx.listener(move |this, _, w, cx| {
                        if std::mem::take(&mut this.skip_click) {
                            return;
                        }
                        this.open(&id, w, cx);
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &gpui::MouseDownEvent, w, cx| {
                            this.open_options(&context, Some(event.position), w, cx)
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &gpui::MouseDownEvent, _, _| {
                            this.skip_click = false;
                            this.drag = Some((drag.clone(), event.position, None));
                        }),
                    )
            }))
            // MonoRailSection owns its menu and background dialog; the Mono
            // view paints them instead while it is open.
            .when(self.current().is_none(), |d| {
                d.children(self.options_surface(cx))
                    .children(self.background_modal())
            })
            .into_any_element()
    }

    pub(crate) fn title_tab(&self, id: &str, width: f32, cx: &Context<Self>) -> Option<AnyElement> {
        self.roster.get(id).map(|mono| {
            let id = mono.id.clone();
            let close = id.clone();
            let active = self.active.as_deref() == Some(&id);
            let selector = format!("mono-title-tab-{id}");
            let group: SharedString = selector.clone().into();
            div()
                .id(SharedString::from(format!("mono-title-{id}")))
                .debug_selector(move || selector.clone())
                .role(Role::Tab)
                .aria_label(mono.label())
                .aria_selected(active)
                .h(px(TITLE_BAR_HEIGHT))
                .w(px(width))
                .flex_shrink_0()
                .relative()
                .flex()
                .items_center()
                .group(group.clone())
                .child(
                    title_tab_plate(active, false)
                        .w_full()
                        .min_w_0()
                        .pr_7()
                        .gap(px(crate::ui::tabs::tile::TAB_TILE_GAP))
                        .group_hover(group.clone(), move |s| {
                            s.opacity(1.).bg(title_tab_fill(active, true))
                        })
                        .child(crate::ui::tabs::tile::mark_tile(
                            &crate::ui::tabs::tile::TAB_TILE,
                            crate::ui::tabs::tile::Face::Neutral,
                            crate::ui::avatar::avatar(&mono.mascot, mono.color, "idle", 14., true),
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .h(px(20.))
                                .text_size(px(TITLE_TAB_TEXT_SIZE))
                                .text_color(content(if active { 1. } else { 0.5 }))
                                .group_hover(group.clone(), |s| s.text_color(content(1.)))
                                .child(crate::ui::tabs::label::tab_label(mono.label())),
                        ),
                )
                .child(
                    div()
                        .id("close-mono-tab")
                        .debug_selector(|| "close-mono-tab-button".into())
                        .role(Role::Button)
                        .aria_label("Close Mono tab")
                        .absolute()
                        .right_1()
                        .top(px(10.))
                        .size(px(20.))
                        .rounded_sm()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(0.)
                        .group_hover(group, |s| s.opacity(1.))
                        .hover(|s| s.bg(content(0.1)))
                        .child(icon("close", 12.))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            let owner = this.owner.clone();
                            let close = close.clone();
                            window.defer(cx, move |window, cx| {
                                let _ = owner.update(cx, |shell, cx| {
                                    shell.close_mono_tab(&close, window, cx)
                                });
                            });
                        })),
                )
                .on_mouse_up(
                    MouseButton::Middle,
                    cx.listener({
                        let id = id.clone();
                        move |this, _, window, cx| {
                            cx.stop_propagation();
                            let owner = this.owner.clone();
                            let id = id.clone();
                            window.defer(cx, move |window, cx| {
                                let _ = owner
                                    .update(cx, |shell, cx| shell.close_mono_tab(&id, window, cx));
                            });
                        }
                    }),
                )
                .on_click(cx.listener(move |this, _, w, cx| {
                    let owner = this.owner.clone();
                    let id = id.clone();
                    w.defer(cx, move |w, cx| {
                        let _ = owner.update(cx, |shell, cx| shell.click_mono_tab(&id, w, cx));
                    });
                }))
                .into_any_element()
        })
    }
}

impl gpui::Render for Monos {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.hide_html(cx);
        let Some(mono) = self.current().cloned() else {
            return div().into_any_element();
        };
        self.ensure_changes_summary(&mono, cx);
        let bg = self
            .backgrounds
            .entry(mono.id.clone())
            .or_insert_with(|| cx.new(|_| crate::project::background::ProjectBackground::default()))
            .clone();
        let empty = self.chat().is_none_or(|v| !v.read(cx).summary.has_messages);
        bg.update(cx, |bg, cx| {
            bg.configure(mono.background.clone(), empty, None, cx)
        });
        div()
            .size_full()
            .min_w_0()
            .flex()
            .relative()
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape"
                    && this.settings.is_some()
                    && this.picker.is_none()
                    && this.menu.is_none()
                {
                    this.save_open_file(cx);
                    this.settings = None;
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .capture_any_mouse_down(|_, window, _| {
                #[cfg(target_os = "macos")]
                crate::ui::traffic_lights::focus_content(window);
                #[cfg(not(target_os = "macos"))]
                let _ = window;
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .relative()
                    .child(bg)
                    .children(
                        self.attention_views(&mono.id, cx)
                            .into_iter()
                            .map(|view| view.update(cx, |v, cx| v.mono_attention(window, cx))),
                    )
                    .map(|d| {
                        if self.floating_shows(&mono.id, cx) {
                            d.child(self.floating_placeholder(&mono, cx))
                        } else if let Some(chat) = self.chat() {
                            d.child(div().flex_1().min_h_0().child(chat))
                        } else {
                            d
                        }
                    })
                    .when(!self.original_chrome, |d| d.child(self.topbar(&mono, cx))),
            )
            .children(self.settings_modal(&mono, window, cx))
            .children(self.picker_surface(cx))
            .children(self.options_surface(cx))
            .children(self.background_modal())
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, _| {
                if let Some((_, start, over)) = &mut this.drag
                    && (event.position - *start).magnitude() > 5.
                {
                    this.skip_click = true;
                    *over = this.rail_bounds.iter().find_map(|(id, s)| {
                        s.get()
                            .filter(|b| b.contains(&event.position))
                            .map(|_| id.clone())
                    });
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.finish_reorder(cx);
                }),
            )
            .into_any_element()
    }
}
