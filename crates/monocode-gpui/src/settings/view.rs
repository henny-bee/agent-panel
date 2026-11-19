use super::*;

impl SettingsView {
    /// The breadcrumb and Restore defaults: in the inset title bar, or the
    /// page's own 40px header row in the classic layout. Search joins them
    /// only while the settings nav, its usual home, is collapsed.
    pub(crate) fn header(
        &mut self,
        in_title_bar: bool,
        with_search: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let search = with_search.then(|| self.search_field(false, cx));
        div()
            .id("settings-header")
            .debug_selector(|| "settings-header".into())
            .h_full()
            .min_w_0()
            .flex_1()
            .flex()
            .items_center()
            .child(
                crate::ui::chrome::titlebar_drag_area()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(13.))
                    .map(|d| {
                        if in_title_bar {
                            d.pr_3().pl_1()
                        } else {
                            d.px_3()
                        }
                    })
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(content(0.45))
                            .child("Settings"),
                    )
                    .child(div().flex_shrink_0().text_color(content(0.25)).child("/"))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(content(1.))
                            .child(self.section.label()),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .map(|d| if in_title_bar { d.pr_4() } else { d.pr_2() })
                    .when(self.section == Section::Appearance, |d| {
                        d.child(
                            div()
                                .id("restore-defaults")
                                .role(Role::Button)
                                .aria_label("Restore defaults")
                                .flex()
                                .flex_shrink_0()
                                .items_center()
                                .gap(px(6.))
                                .rounded_md()
                                .px_2()
                                .py_1()
                                .text_size(px(12.))
                                .line_height(px(16.))
                                .text_color(content(0.5))
                                .cursor_pointer()
                                .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
                                .child(
                                    icon("rotate-ccw", 14.)
                                        .text_color(gpui::transparent_black())
                                        .map(|i| i.text_color(content(0.5))),
                                )
                                .child("Restore defaults")
                                .on_click(cx.listener(|this, _, _, cx| this.restore_defaults(cx))),
                        )
                    })
                    .children(search),
            )
            .into_any_element()
    }

    /// The settings search: atop the settings nav, or in the header while
    /// the nav is collapsed.
    pub(crate) fn search_field(&mut self, in_nav: bool, cx: &mut Context<Self>) -> AnyElement {
        let results = model::search(&self.search_query, cx.compositor_name());
        let open = !self.search_query.trim().is_empty();
        let weak = cx.weak_entity();
        if self.search_in_nav != Some(in_nav) {
            self.search_in_nav = Some(in_nav);
            self.search.update(cx, |i, cx| {
                i.set_font_size(if in_nav { 14. } else { 12. });
                cx.notify();
            });
        }
        div()
            .id("settings-search")
            .debug_selector(|| "settings-search".into())
            .relative()
            .flex_shrink_0()
            .when(in_nav, |d| d.w_full())
            .child(self.search_anchor.probe())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .border_1()
                    .border_color(content(0.1))
                    .px_2()
                    // In the nav it lines up with its rows; the
                    // border takes one pixel of their inset.
                    .map(|d| {
                        if in_nav {
                            d.w_full().h(px(30.)).pl(px(7.))
                        } else {
                            d.w(px(192.)).h(px(28.))
                        }
                    })
                    .child(
                        icon("search-01", if in_nav { 16. } else { 14. })
                            .flex_shrink_0()
                            .text_color(content(0.45)),
                    )
                    .child(div().min_w_0().flex_1().child(self.search.clone()))
                    .when(!self.search_query.is_empty(), |d| {
                        d.child(
                            div()
                                .id("clear-settings-search")
                                .role(Role::Button)
                                .aria_label("Clear settings search")
                                .size_4()
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_sm()
                                .cursor_pointer()
                                .child(icon("x", 12.).text_color(content(0.45)))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.search
                                        .update(cx, |i, cx| i.set_text(String::new(), cx));
                                    this.search_query.clear();
                                    window.focus(&this.search.focus_handle(cx), cx);
                                    cx.notify();
                                })),
                        )
                    }),
            )
            // The field binds up/down to its own cursor
            // actions, which run before key listeners, so
            // the results take them first.
            .capture_action(cx.listener(|this, _: &input::Up, _, cx| {
                if this.step_search(-1, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &input::Down, _, cx| {
                if this.step_search(1, cx) {
                    cx.stop_propagation();
                }
            }))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if this.search_query.trim().is_empty() {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "down" => {
                        this.step_search(1, cx);
                    }
                    "up" => {
                        this.step_search(-1, cx);
                    }
                    "escape" => {
                        this.search_query.clear();
                        this.search
                            .update(cx, |i, cx| i.set_text(String::new(), cx));
                    }
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .when(open, |d| {
                let active = self.search_active;
                d.child(
                    popover(
                        "settings-search-results",
                        Anchor::Slot(self.search_anchor.clone()),
                    )
                    .side(Side::Bottom)
                    .align(if in_nav { Align::Start } else { Align::End })
                    .width(300.)
                    .max_height(320.)
                    .padding(4.)
                    .on_dismiss(move |_, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.search_query.clear();
                            this.search
                                .update(cx, |i, cx| i.set_text(String::new(), cx));
                            cx.notify();
                        });
                    })
                    .child(
                        div()
                            .id("settings-search-list")
                            .role(Role::ListBox)
                            .aria_label("Settings search results")
                            .flex()
                            .flex_col()
                            .when(results.is_empty(), |d| {
                                d.child(
                                    div()
                                        .px_2()
                                        .py(px(6.))
                                        .text_size(px(12.))
                                        .line_height(px(16.))
                                        .text_color(content(0.45))
                                        .child("No matching settings"),
                                )
                            })
                            .children(results.into_iter().enumerate().map(|(ix, result)| {
                                let tag = if result.setting.is_some() {
                                    result.section.label()
                                } else {
                                    "Page"
                                };
                                let label = result.label;
                                div()
                                    .id(("settings-result", ix))
                                    .debug_selector(move || format!("settings-result-{ix}"))
                                    .role(Role::ListBoxOption)
                                    .aria_label(label)
                                    .aria_selected(ix == active)
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .rounded_lg()
                                    .px_2()
                                    .py(px(6.))
                                    .text_size(px(12.))
                                    .line_height(px(16.))
                                    .text_color(content(1.))
                                    .cursor_pointer()
                                    .map(|d| {
                                        if ix == active {
                                            d.bg(content(0.1))
                                        } else {
                                            d.hover(|s| s.bg(content(0.05)))
                                        }
                                    })
                                    .child(div().min_w_0().flex_1().truncate().child(label))
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .text_size(px(11.))
                                            .text_color(content(0.4))
                                            .child(tag),
                                    )
                                    .on_mouse_move(cx.listener(move |this, _, _, cx| {
                                        if this.search_active != ix {
                                            this.search_active = ix;
                                            cx.notify();
                                        }
                                    }))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.go(result.clone(), cx)
                                    }))
                            })),
                    ),
                )
            })
            .into_any_element()
    }
}

/// SettingsNav: the rail body while Settings is open. The search heads it,
/// the pages scroll beneath, and Back to app sits at the bottom.
pub(crate) fn nav(
    section: Section,
    search: AnyElement,
    select: impl Fn(Section, &mut Window, &mut App) + Clone + 'static,
    back: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let row = |id: SharedString, label: &'static str, path: &'static str, active: bool| {
        div()
            .id(id.clone())
            .debug_selector(move || id.to_string())
            .role(Role::Button)
            .aria_label(label)
            .aria_selected(active)
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .rounded_md()
            .px_2()
            .py(px(6.))
            .cursor_pointer()
            .map(|d| {
                if active {
                    d.bg(content(0.1)).text_color(content(1.))
                } else {
                    d.text_color(content(0.5))
                        .hover(|s| s.bg(content(0.05)).text_color(content(1.)))
                }
            })
            .child(
                gpui::svg()
                    .path(path)
                    .size(px(16.))
                    .flex_shrink_0()
                    .opacity(0.7)
                    .text_color(if active { content(1.) } else { content(0.5) }),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_size(px(14.))
                    .line_height(px(17.5))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(label),
            )
    };
    div()
        .id("settings-nav")
        .debug_selector(|| "settings-nav".into())
        .role(Role::Navigation)
        .aria_label("Settings")
        .size_full()
        .flex()
        .flex_col()
        .child(
            div()
                .flex_shrink_0()
                .flex()
                .flex_col()
                .px_2()
                .pt_2()
                .pb_2()
                .child(search),
        )
        .child(
            div()
                .id("settings-nav-scroll")
                .min_h_0()
                .flex_1()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_5()
                .px_2()
                .pt_2()
                .pb_3()
                .children(model::GROUPS.iter().map(|(group, sections)| {
                    div()
                        .flex()
                        .flex_col()
                        .gap_px()
                        .child(
                            div()
                                .px_2()
                                .pb_1()
                                .text_size(px(12.))
                                .line_height(px(16.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(content(0.35))
                                .child(*group),
                        )
                        .children(sections.iter().map(|&item| {
                            let select = select.clone();
                            row(
                                format!("settings-nav-{}", item.id()).into(),
                                item.label(),
                                item.icon(),
                                item == section,
                            )
                            .on_click(move |_, window, cx| select(item, window, cx))
                        }))
                }))
                .into_any_element(),
        )
        .child(
            div()
                .flex_shrink_0()
                .flex()
                .flex_col()
                .items_start()
                .px_2()
                .pt_1()
                .pb_2()
                .child(
                    div()
                        .id("settings-nav-back")
                        .debug_selector(|| "settings-nav-back".into())
                        .group("settings-nav-back")
                        .role(Role::Button)
                        .aria_label("Back to app")
                        .flex()
                        .items_center()
                        // The nav rows' inset, icon column and gap, so the
                        // chevron and label line up with theirs.
                        .gap_2()
                        .h(px(30.))
                        .px_2()
                        .rounded_full()
                        .text_color(content(0.35))
                        .cursor_pointer()
                        .hover(|s| s.bg(content(0.08)).text_color(content(1.)))
                        .child(
                            div()
                                .size(px(16.))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    icon("chevron-left", 14.)
                                        .text_color(content(0.35))
                                        .group_hover("settings-nav-back", |s| {
                                            s.text_color(content(1.))
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .pr_1()
                                .text_size(px(13.))
                                .line_height(px(16.))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child("Back to app"),
                        )
                        .on_click(move |_, window, cx| back(window, cx)),
                ),
        )
        .into_any_element()
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.pending_scroll {
            let weak = cx.weak_entity();
            window.on_next_frame(move |_, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.scroll_to_revealed();
                    cx.notify();
                });
            });
        }
        self.notification_window_state(window, cx);
        self.connections_window_state(window, cx);
        let page = self.page(window, cx);
        let narrow = self
            .scroll
            .bounds()
            .size
            .width
            .as_f32()
            .finite_and(|w| w > 0. && w < 560.);
        div()
            .id("settings-view")
            .debug_selector(|| "settings-view".into())
            .role(Role::Region)
            .aria_label("Settings")
            .track_focus(if self.recording.is_some() {
                &self.recorder_focus
            } else {
                &self.focus
            })
            .key_context("SettingsView")
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .text_color(content(1.))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if let Some(kind) = this.slider_drag {
                    if event.pressed_button == Some(MouseButton::Left) {
                        this.slide(kind, event.position.x, cx);
                    } else {
                        this.slider_drag = None;
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.slider_drag = None),
            )
            .children(if self.section == Section::Skills {
                page
            } else {
                vec![
                    div()
                        .id("settings-scroll")
                        .min_h_0()
                        .flex_1()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .child(
                            div()
                                .mx_auto()
                                .w_full()
                                .max_w(px(1024.))
                                .map(|d| {
                                    if narrow {
                                        d.px_5().pt_6().pb_16()
                                    } else {
                                        d.px_8().pt_8().pb_16()
                                    }
                                })
                                .child(crate::ui::widgets::page_header(
                                    self.section.label(),
                                    self.section.description(),
                                ))
                                .children(page),
                        )
                        .into_any_element(),
                ]
            })
            .children(self.management_overlay(window, cx))
            .children(self.notification_overlay(cx))
    }
}
