use super::*;

impl Render for Completion {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.token.is_some() && self.input.focus_handle(cx).is_focused(window);
        let mut root = div()
            .debug_selector(|| "composer-text-field".into())
            .relative()
            .w_full()
            .child(self.anchor.layout_probe())
            .key_context(if open { "Completion" } else { "" })
            .capture_action(cx.listener(|this, _: &input::Up, _, cx| {
                if this.token.is_some() && !this.input.read(cx).is_composing() {
                    this.step(-1, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &input::Down, _, cx| {
                if this.token.is_some() && !this.input.read(cx).is_composing() {
                    this.step(1, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &input::Submit, window, cx| {
                if this.token.is_some() && !this.input.read(cx).is_composing() {
                    if !this.choices.is_empty() {
                        this.pick(this.active, window, cx);
                    } else {
                        this.dismiss(cx);
                    }
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &Accept, window, cx| {
                this.pick(this.active, window, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| {
                this.dismiss(cx);
                cx.stop_propagation();
            }))
            .child(self.input.clone());
        if open {
            let rows: Vec<_> = self
                .choices
                .iter()
                .enumerate()
                .map(|(ix, choice)| {
                    let (icon_path, icon_color) = choice.icon();
                    div()
                        .id(("completion-row", ix))
                        .role(Role::ListItem)
                        .aria_label(choice.label.clone())
                        .px_3()
                        .py_1p5()
                        .rounded_md()
                        .flex()
                        .items_center()
                        .gap_2()
                        .bg(if ix == self.active {
                            content(0.1)
                        } else {
                            content(0.)
                        })
                        .hover(|s| s.bg(content(0.08)))
                        .child(
                            div()
                                .size(px(30.))
                                .flex_shrink_0()
                                .rounded(px(6.))
                                .bg(content(0.05))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(if choice.file {
                                    crate::ui::file_icons::file_type_icon(
                                        &choice.label,
                                        false,
                                        false,
                                        16.,
                                    )
                                } else {
                                    icon(icon_path, 16.)
                                        .text_color(icon_color)
                                        .into_any_element()
                                }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .truncate()
                                        .child(choice.label.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(content(0.45))
                                        .truncate()
                                        .child(choice.detail.clone()),
                                ),
                        )
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, window, cx| this.pick(ix, window, cx)))
                        .into_any_element()
                })
                .collect();
            let weak = cx.weak_entity();
            let title = if self.mcp_open {
                "MCP SERVERS"
            } else if self
                .token
                .as_ref()
                .is_some_and(|t| t.trigger == Trigger::Mention)
            {
                "FILES & NOTES"
            } else {
                "SKILLS & COMMANDS"
            };
            let anchor = if self.composer_anchor.get().is_some() {
                self.composer_anchor.clone()
            } else {
                self.anchor.clone()
            };
            let bounds = anchor.get().unwrap_or_default();
            let width = bounds
                .size
                .width
                .as_f32()
                .min((window.viewport_size().width.as_f32() - 16.).max(0.));
            let gap = 6.;
            // Measure from the conversation's top, rather than the window's,
            // so a centered composer's menu cannot cover the session tabs.
            let top = self.container_anchor.get().map_or(0., |b| b.top().as_f32());
            let max_height = (bounds.top().as_f32() - top - gap - 8.).clamp(0., 400.);
            let (anchor, side, width, max_height) = if self.inline_field {
                let caret = self.input.read(cx).cursor_anchor().unwrap_or(bounds);
                let above = (caret.top().as_f32() - gap - 8.).max(0.);
                let below =
                    (window.viewport_size().height.as_f32() - caret.bottom().as_f32() - gap - 8.)
                        .max(0.);
                let (y, side, room) = if below >= above {
                    (caret.bottom(), Side::Bottom, below)
                } else {
                    (caret.top(), Side::Top, above)
                };
                (
                    Anchor::Point(gpui::point(caret.left(), y)),
                    side,
                    width.min(400.),
                    room.min(400.),
                )
            } else {
                (Anchor::Slot(anchor), Side::Top, width, max_height)
            };
            root = root.child(
                popover("composer-completions", anchor)
                    .side(side)
                    .gap(gap)
                    .width(width)
                    .max_height(max_height)
                    .on_dismiss(move |_, cx| {
                        let _ = weak.update(cx, |this, cx| this.dismiss(cx));
                    })
                    .child(
                        div()
                            .id("composer-completion-list")
                            .debug_selector(|| "composer-completion-list".into())
                            .role(Role::ListBox)
                            .aria_label(title)
                            .p_2()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .px_2()
                                    .py_1()
                                    .text_size(px(10.))
                                    .text_color(content(0.4))
                                    .child(title),
                            )
                            .child(
                                div()
                                    .id("composer-completion-rows")
                                    .min_h_0()
                                    .max_h(px(320.))
                                    .pr_1()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.scroll)
                                    .children(rows),
                            )
                            .when(self.choices.is_empty(), |d| {
                                d.child(
                                    div()
                                        .px_2()
                                        .py_3()
                                        .text_size(px(12.))
                                        .text_color(content(0.45))
                                        .child(SharedString::from(if self.loading {
                                            "Loading…".to_owned()
                                        } else {
                                            self.error
                                                .clone()
                                                .unwrap_or_else(|| "No matches".into())
                                        })),
                                )
                            })
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .mt_1()
                                    .px_2()
                                    .pt_1p5()
                                    .pb_1()
                                    .border_t_1()
                                    .border_color(content(0.07))
                                    .text_size(px(10.))
                                    .text_color(content(0.35))
                                    .child("↑ ↓ navigate    ↵ / Tab select    Esc dismiss"),
                            ),
                    ),
            );
        }
        root
    }
}
