use super::*;

impl Render for Automations {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.filter.read(cx).content.to_lowercase();
        let visible: Vec<_> = self
            .automations
            .iter()
            .filter(|a| {
                format!("{} {} {}", a.name, a.prompt, a.cwd)
                    .to_lowercase()
                    .contains(query.trim())
            })
            .cloned()
            .collect();
        let mut list = div()
            .id("automations-list")
            .role(Role::ListBox)
            .aria_label("Automations")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(6.))
            .flex()
            .flex_col()
            .gap(px(2.));
        if visible.is_empty() {
            list = list.child(
                div()
                    .px(px(12.))
                    .py(px(32.))
                    .text_size(px(12.))
                    .text_color(content(0.45))
                    .child(if self.loading {
                        "Loading automations…"
                    } else if query.trim().is_empty() {
                        "No automations yet"
                    } else {
                        "No matching automations"
                    }),
            );
        }
        for automation in visible {
            let id = automation.id.clone();
            let toggle_id = id.clone();
            let enabled = automation.enabled;
            let active = !self.gallery && self.selected.as_deref() == Some(&id);
            let kind = Kind::from_id(&automation.harness);
            let model = monocode_core::store::native_model(&automation.model, &automation.harness);
            list = list.child(
                div()
                    .id(SharedString::from(format!("automation-{id}")))
                    .relative()
                    .rounded(px(6.))
                    .when(active, |d| d.bg(content(0.1)))
                    .hover(|s| s.bg(content(0.05)))
                    .child(
                        div()
                            .id(SharedString::from(format!("open-{id}")))
                            .role(Role::Button)
                            .aria_label(format!("Open {}", automation.name))
                            .px(px(10.))
                            .py(px(8.))
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .pr(px(36.))
                                    .truncate()
                                    .text_size(px(10.))
                                    .text_color(content(0.45))
                                    .child(automation.trigger_label()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(automation.name),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(5.))
                                    .text_size(px(11.))
                                    .text_color(content(0.45))
                                    .child(self.project_mark(&automation.cwd, 12.))
                                    .child(
                                        div()
                                            .flex_1()
                                            .truncate()
                                            .child(self.project_label(&automation.cwd)),
                                    )
                                    .child(provider_icon(kind.id(), 13.))
                                    .child(
                                        div()
                                            .max_w(px(100.))
                                            .truncate()
                                            .child(cached_model_name(kind, model.as_deref())),
                                    ),
                            )
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.select(id.clone(), cx)),
                            ),
                    )
                    .child(
                        div().absolute().right(px(10.)).top(px(8.)).child(
                            editor::toggle(
                                SharedString::from(format!("toggle-{toggle_id}")),
                                enabled,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.toggle(toggle_id.clone(), !enabled, cx),
                            )),
                        ),
                    ),
            );
        }
        let aside = div()
            .w(px(280.))
            .flex_shrink_0()
            .border_r_1()
            .border_color(content(0.07))
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(36.))
                    .px(px(8.))
                    .border_b_1()
                    .border_color(content(0.07))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(icon("search", 12.))
                    .child(div().flex_1().min_w_0().child(self.filter.clone()))
                    .child(
                        icon_button("new-automation", "New automation", "plus").on_click(
                            cx.listener(|this, _, _, cx| {
                                this.show_gallery(cx);
                            }),
                        ),
                    ),
            )
            .child(list);
        let main_content = if self.gallery || self.editor.is_none() {
            self.render_gallery(cx)
        } else {
            self.render_editor(cx)
        };
        let mut root = div()
            .id("automations-view")
            .role(Role::Region)
            .aria_label("Automations")
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .text_color(content(1.))
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    // Let the trigger menu close its active submenu first.
                    if this.trigger_menu.is_some() {
                        cx.propagate();
                        return;
                    }
                    if this.confirm_delete {
                        this.confirm_delete = false;
                    } else if this.choice.is_some()
                        || this.trigger_menu.is_some()
                        || this.models.is_some()
                        || this.project_picker.is_some()
                    {
                        this.choice = None;
                        this.trigger_menu = None;
                        this.models = None;
                        this.project_picker = None;
                    } else if this.editor.is_some() && !this.gallery {
                        this.show_gallery(cx);
                    } else {
                        cx.emit(Event::Close);
                    }
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(
                div().flex_1().min_h_0().flex().child(aside).child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .when_some(self.error.clone(), |d, error| {
                            d.child(
                                div()
                                    .m(px(16.))
                                    .p(px(10.))
                                    .rounded(px(8.))
                                    .bg(gpui::rgba(0xf8717114))
                                    .text_size(px(12.))
                                    .text_color(rgb(0xfca5a5))
                                    .child(error),
                            )
                        })
                        .child(main_content),
                ),
            );
        if let Some(menu) = &self.trigger_menu {
            root = root.child(
                menu.read(cx)
                    .popover(Anchor::Slot(self.trigger_anchor.clone()), cx),
            );
        }
        if let Some(menu) = &self.choice {
            let weak = cx.weak_entity();
            let field = menu.field.clone();
            root = root.child(
                popover("automation-choice", Anchor::Slot(menu.anchor.clone()))
                    .width(250.)
                    .on_dismiss(move |_, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.choice = None;
                            cx.notify();
                        });
                    })
                    .child(
                        div()
                            .id("automation-choices")
                            .max_h(px(360.))
                            .overflow_y_scroll()
                            .p(px(4.))
                            .children(menu.options.iter().enumerate().map(
                                |(index, (value, label))| {
                                    let field = field.clone();
                                    let value = value.clone();
                                    div()
                                        .id(("automation-choice", index))
                                        .role(Role::MenuItem)
                                        .aria_label(label.clone())
                                        .px(px(8.))
                                        .py(px(8.))
                                        .rounded(px(6.))
                                        .text_size(px(13.))
                                        .hover(|s| s.bg(content(0.07)))
                                        .child(label.clone())
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.apply_choice(field.clone(), value.clone(), cx)
                                        }))
                                },
                            )),
                    ),
            );
        }
        if let Some(picker) = &self.models {
            let weak = cx.weak_entity();
            root = root.child(
                popover(
                    "automation-model-picker",
                    Anchor::Slot(self.model_anchor.clone()),
                )
                .width(340.)
                .on_dismiss(move |_, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.models = None;
                        cx.notify();
                    });
                })
                .child(self.render_model_settings(cx))
                .child(div().h(px(440.)).child(picker.clone())),
            );
        }
        if let Some(picker) = &self.project_picker {
            let weak = cx.weak_entity();
            root = root.child(
                popover(
                    "automation-project-picker",
                    Anchor::Slot(self.project_anchor.clone()),
                )
                .width(320.)
                .on_dismiss(move |_, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.project_picker = None;
                        cx.notify();
                    });
                })
                .child(picker.clone()),
            );
        }
        root
    }
}
