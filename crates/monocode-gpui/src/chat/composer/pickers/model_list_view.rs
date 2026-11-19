use super::*;

impl Render for ModelList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.quick {
            return self.render_quick(cx);
        }
        let entries = self.entries(cx);
        let kind = match self.tab {
            ModelTab::Harness(kind) => Some(kind),
            _ => None,
        };
        let loading = kind.is_some_and(|kind| self.loading.contains(&kind));
        let error = kind.and_then(|kind| {
            self.errors
                .iter()
                .find(|(k, _)| *k == kind)
                .map(|(_, error)| error.clone())
        });
        let empty = match (self.tab, self.query(cx).is_empty()) {
            (ModelTab::Favorites, true) if entries.is_empty() => Some("No favorite models"),
            _ if entries.is_empty() => Some("No matching models"),
            _ => None,
        };
        let rows: Vec<AnyElement> = entries
            .into_iter()
            .enumerate()
            .map(|(ix, entry)| {
                let highlighted = self.highlighted == ix && entry.enabled;
                let current = entry.kind == self.provider && entry.id == self.current;
                let starred = self.favorite(entry.kind, entry.id.as_deref(), cx);
                let (star_kind, star_id) = (entry.kind, entry.id.clone());
                let row_group: SharedString = format!("model-row-{ix}").into();
                div()
                    .id(("model-row", ix))
                    .group(row_group.clone())
                    .flex_shrink_0()
                    .h(px(32.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .text_size(px(13.))
                    .text_color(content(if entry.enabled { 1. } else { 0.3 }))
                    .when(highlighted, |d| d.bg(content(0.1)))
                    .when(!highlighted && entry.enabled, |d| {
                        d.hover(|s| s.bg(content(0.05)))
                    })
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered {
                            this.highlighted = ix;
                            cx.notify();
                        }
                    }))
                    .child(
                        div()
                            .id(("model-option", ix))
                            .role(Role::ListBoxOption)
                            .aria_label(entry.name.clone())
                            .aria_selected(current)
                            .when(!entry.enabled, |d| {
                                d.aria_description(if self.missing.contains(&entry.kind) {
                                    if self.remote_project.is_some() { "This provider is not installed on the connected machine. Install it there, then retry model discovery" } else { "This provider is not installed. Install it, then retry model discovery" }
                                } else {
                                    "Wait for the current turn to finish before switching providers"
                                })
                            })
                            .cursor_pointer()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .px_1()
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .px(px(6.))
                                    .truncate()
                                    .child(entry.name.clone()),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| this.pick(ix, cx))),
                    )
                    .child(
                        div()
                            .id(("model-star", ix))
                            .debug_selector(move || format!("model-star-{ix}"))
                            .role(Role::Button)
                            .aria_label(favorite_label(starred, &entry.name))
                            .cursor_pointer()
                            .size(px(24.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            // Revealed by hovering the row until it is a favorite.
                            .when(!starred, |d| {
                                d.opacity(0.)
                                    .group_hover(row_group.clone(), |s| s.opacity(1.))
                            })
                            .child(
                                icon(if starred { "star-filled" } else { "star" }, 14.)
                                    .text_color(content(if starred { 0.6 } else { 0.35 })),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_favorite(star_kind, star_id.clone(), cx)
                            })),
                    )
                    .when(current, |d| {
                        d.child(
                            div()
                                .size(px(24.))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon("check-2", 14.).text_color(content(0.55))),
                        )
                    })
                    .into_any()
            })
            .collect();
        div()
            .id("model-flyout")
            .role(Role::Dialog)
            .aria_label("Models")
            .size_full()
            .min_h_0()
            .flex()
            .capture_action(cx.listener(|this, _: &input::Up, _, cx| {
                this.step(-1, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &input::Down, _, cx| {
                this.step(1, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &input::Submit, _, cx| {
                this.pick(this.highlighted, cx);
                cx.stop_propagation();
            }))
            .child(
                div()
                    .id("model-providers")
                    .role(Role::TabList)
                    .aria_label("Providers")
                    .w(px(44.))
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .items_center()
                    .gap_1()
                    .p(px(6.))
                    .border_r_1()
                    .border_color(content(0.07))
                    .child(self.tab(ModelTab::Favorites, cx))
                    .children(
                        Kind::ALL
                            .into_iter()
                            .filter(|kind| {
                                if self.remote_project.is_some() {
                                    *kind == self.provider
                                        || self
                                            .remote_providers
                                            .as_ref()
                                            .is_some_and(|providers| providers.contains(kind))
                                } else {
                                    *kind == self.provider
                                        || (!self.hidden.iter().any(|id| id == kind.id())
                                            && !self.missing.contains(kind))
                                }
                            })
                            .map(|kind| self.tab(ModelTab::Harness(kind), cx)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_3()
                            .py(px(10.))
                            .border_b_1()
                            .border_color(content(0.07))
                            .text_color(content(1.))
                            .child(icon("search", 14.).text_color(content(0.5)))
                            .child(div().flex_1().min_w_0().child(self.search.clone())),
                    )
                    .child(
                        div()
                            .id("model-list")
                            .role(Role::ListBox)
                            .aria_label("Models")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .p_1()
                            .flex()
                            .flex_col()
                            .when_some(empty, |d, text| {
                                d.child(
                                    div()
                                        .px_2()
                                        .py_3()
                                        .text_size(px(12.))
                                        .text_color(content(0.5))
                                        .child(text),
                                )
                            })
                            .children(rows)
                            .when(loading, |d| {
                                d.child(
                                    div()
                                        .px_2()
                                        .py_2()
                                        .text_size(px(12.))
                                        .text_color(content(0.5))
                                        .child("Loading models…"),
                                )
                            })
                            .when_some(error, |d, error| {
                                d.child(
                                    div()
                                        .px_2()
                                        .py_2()
                                        .text_size(px(12.))
                                        .text_color(content(0.5))
                                        .child(error)
                                        .child(
                                            div()
                                                .id("retry-models")
                                                .role(Role::Button)
                                                .aria_label("Retry model discovery")
                                                .py_2()
                                                .child("Retry")
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    if let Some(kind) = kind {
                                                        this.discover(kind, cx);
                                                        cx.notify();
                                                    }
                                                })),
                                        ),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }
}

impl ModelList {
    pub(super) fn render_quick(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let entries = self.entries(cx);
        let kind = match self.tab {
            ModelTab::Harness(kind) => Some(kind),
            _ => None,
        };
        let loading = kind.is_some_and(|kind| self.loading.contains(&kind));
        let query = !self.query(cx).is_empty();
        let empty = entries.is_empty().then(|| {
            if query {
                "No matching models"
            } else if kind.is_none() {
                "No favorite models"
            } else if loading {
                "Loading models…"
            } else {
                "No models"
            }
        });
        let favorites = kind.is_none();
        let rows: Vec<AnyElement> = entries
            .into_iter()
            .enumerate()
            .map(|(ix, entry)| {
                let highlighted = self.highlighted == ix;
                let current = entry.kind == self.provider && entry.id == self.current;
                let starred = self.favorite(entry.kind, entry.id.as_deref(), cx);
                let (star_kind, star_id) = (entry.kind, entry.id.clone());
                div()
                    .id(("quick-model", ix))
                    .group("quick-model-row")
                    .flex_shrink_0()
                    .h(px(32.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .when(highlighted, |d| d.bg(content(0.1)))
                    .when(!highlighted, |d| d.hover(|s| s.bg(content(0.06))))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered {
                            this.highlighted = ix;
                            cx.notify();
                        }
                    }))
                    .child(
                        div()
                            .id(("quick-model-option", ix))
                            .role(Role::ListBoxOption)
                            .aria_label(entry.name.clone())
                            .aria_selected(current)
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .px_2()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(13.))
                            .text_color(content(1.))
                            .when(!entry.enabled, |d| d.opacity(0.35))
                            .when(entry.enabled, |d| d.cursor_pointer())
                            .when(favorites, |d| d.child(provider_icon(entry.kind.id(), 14.)))
                            .child(div().min_w_0().truncate().child(entry.name.clone()))
                            .when(current, |d| {
                                d.child(icon("check-2", 14.).ml_auto().text_color(accent_color()))
                            })
                            .on_click(cx.listener(move |this, _, _, cx| this.pick(ix, cx))),
                    )
                    .child(
                        div()
                            .id(("quick-model-star", ix))
                            .role(Role::Button)
                            .aria_label(favorite_label(starred, &entry.name))
                            .group("quick-model-star")
                            .cursor_pointer()
                            .size(px(32.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                icon(if starred { "star-filled" } else { "star" }, 14.)
                                    .text_color(content(0.35))
                                    .group_hover("quick-model-star", |s| s.text_color(content(1.))),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_favorite(star_kind, star_id.clone(), cx)
                            })),
                    )
                    .into_any_element()
            })
            .collect();
        let tabs: Vec<ModelTab> = std::iter::once(ModelTab::Favorites)
            .chain(
                Kind::ALL
                    .into_iter()
                    .filter(|kind| {
                        *kind == self.provider
                            || (!self.hidden.iter().any(|id| id == kind.id())
                                && !self.missing.contains(kind))
                    })
                    .map(ModelTab::Harness),
            )
            .collect();
        div()
            .id("quick-model-selector")
            .role(Role::Group)
            .aria_label("Model selector")
            .w_full()
            .flex()
            .flex_col()
            .capture_action(cx.listener(|this, _: &input::Up, _, cx| {
                this.step(-1, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &input::Down, _, cx| {
                this.step(1, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &input::Submit, _, cx| {
                this.pick(this.highlighted, cx);
                cx.stop_propagation();
            }))
            .child(
                div()
                    .id("quick-model-providers")
                    .role(Role::TabList)
                    .aria_label("Providers")
                    .h(px(44.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .border_b_1()
                    .border_color(content(0.08))
                    .children(tabs.into_iter().map(|tab| {
                        let selected = self.tab == tab;
                        let id: SharedString = match tab {
                            ModelTab::Favorites => "quick-tab-favorites".into(),
                            ModelTab::Harness(kind) => format!("quick-tab-{}", kind.id()).into(),
                        };
                        let label = match tab {
                            ModelTab::Favorites => "Favorites",
                            ModelTab::Harness(kind) => kind.label(),
                        };
                        div()
                            .id(id.clone())
                            .role(Role::Tab)
                            .aria_label(label)
                            .aria_selected(selected)
                            .group(id.clone())
                            .cursor_pointer()
                            .flex_1()
                            .min_w_0()
                            .h(px(32.))
                            .rounded(px(8.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(selected, |d| d.bg(content(0.1)))
                            .when(!selected, |d| {
                                d.opacity(0.4).hover(|s| s.opacity(1.).bg(content(0.06)))
                            })
                            .child(match tab {
                                ModelTab::Favorites => {
                                    icon(if selected { "star-filled" } else { "star" }, 16.)
                                        .text_color(content(1.))
                                        .into_any_element()
                                }
                                ModelTab::Harness(kind) => {
                                    provider_icon(kind.id(), 16.).into_any_element()
                                }
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_tab(tab, window, cx)
                            }))
                    })),
            )
            .child(
                div()
                    .h(px(40.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .border_b_1()
                    .border_color(content(0.08))
                    .text_size(px(13.))
                    .text_color(content(1.))
                    .child(icon("search", 14.).text_color(content(0.4)))
                    .child(div().flex_1().min_w_0().child(self.search.clone())),
            )
            .child(
                div()
                    .id("quick-model-list")
                    .role(Role::ListBox)
                    .aria_label("Models")
                    .h(px(240.))
                    .flex_shrink_0()
                    .overflow_y_scroll()
                    .p_2()
                    .flex()
                    .flex_col()
                    .when_some(empty, |d, text| {
                        d.child(
                            div()
                                .px_2()
                                .py_6()
                                .flex()
                                .justify_center()
                                .text_size(px(12.))
                                .text_color(content(0.45))
                                .child(text),
                        )
                    })
                    .children(rows),
            )
            .into_any_element()
    }
}
