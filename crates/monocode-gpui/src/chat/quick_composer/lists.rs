use super::*;

impl QuickComposer {
    pub(super) fn permissions(&mut self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("quick-permissions-list")
            .role(Role::ListBox)
            .aria_label("Permissions")
            .track_focus(&self.permissions_focus)
            .flex_shrink_0()
            .border_t_1()
            .border_color(content(0.08))
            .p_2()
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "down" => this.step(1, cx),
                    "up" => this.step(-1, cx),
                    "enter" | "space" => this.choose_permission(this.highlight, window, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .children(
                PERMISSIONS
                    .into_iter()
                    .enumerate()
                    .map(|(ix, (mode, glyph, hint))| {
                        let selected = mode == self.access;
                        self.option_row(("quick-permission", ix), ix == self.highlight, cx)
                            .aria_label(mode.label())
                            .aria_selected(selected)
                            .px_3()
                            .py_2()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                icon(glyph, 16.).text_color(if mode == AccessMode::FullAccess {
                                    amber(0.9)
                                } else {
                                    content(0.75)
                                }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .child(mode.label()),
                                    )
                                    .child(
                                        div()
                                            .mt(px(2.))
                                            .text_size(px(11.))
                                            .text_color(content(0.45))
                                            .child(hint),
                                    ),
                            )
                            .when(selected, |d| {
                                d.child(icon("check-2", 14.).text_color(accent_color()))
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.choose_permission(ix, window, cx)
                            }))
                    }),
            )
            .into_any_element()
    }

    pub(super) fn project_list(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let options = self.project_options(cx);
        let focused = self.query.focus_handle(cx).is_focused(window);
        div()
            .id("quick-projects")
            .flex_shrink_0()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(content(0.08))
            .capture_action(cx.listener(|this, _: &input::Up, _, cx| {
                this.step(-1, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &input::Down, _, cx| {
                this.step(1, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &input::Submit, window, cx| {
                this.choose_project(this.highlight, window, cx);
                cx.stop_propagation();
            }))
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_2()
                    .text_color(content(1.))
                    .child(icon("search", 14.).text_color(content(if focused {
                        0.6
                    } else {
                        0.45
                    })))
                    .child(div().flex_1().min_w_0().child(self.query.clone())),
            )
            .child(
                div()
                    .id("quick-project-list")
                    .role(Role::ListBox)
                    .aria_label("Projects")
                    .max_h(px(256.))
                    .overflow_y_scroll()
                    .px_2()
                    .pb_2()
                    .when(options.is_empty(), |d| {
                        d.child(
                            div()
                                .px_2()
                                .py_2()
                                .text_size(px(12.))
                                .text_color(content(0.45))
                                .child("No matches"),
                        )
                    })
                    .children(options.into_iter().enumerate().map(|(ix, path)| {
                        self.option_row(("quick-project-option", ix), ix == self.highlight, cx)
                            .aria_label(self.project_label(&path))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(crate::ui::project_badge::project_icon(
                                &self.appearance.get(&path).cloned().unwrap_or_default(),
                                &path,
                                12.,
                            ))
                            .child(div().min_w_0().truncate().child(self.project_label(&path)))
                            .child(
                                div()
                                    .ml_auto()
                                    .min_w_0()
                                    .pl_3()
                                    .truncate()
                                    .text_size(px(11.))
                                    .text_color(content(0.4))
                                    .child(crate::chat::composer::pickers::pretty_parent(&path)),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.choose_project(ix, window, cx)
                            }))
                    })),
            )
            .into_any_element()
    }

    pub(super) fn workspace_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let new = self.workspace.new_worktree;
        let current = !new && self.workspace.tree.is_none();
        let row = |id: &'static str, glyph: &'static str, label: &'static str, checked: bool| {
            div()
                .id(id)
                .role(Role::MenuItemRadio)
                .aria_label(label)
                .aria_selected(checked)
                .cursor_pointer()
                .h(px(32.))
                .px_2()
                .rounded(px(8.))
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(13.))
                .text_color(content(0.85))
                .hover(|s| s.bg(content(0.08)).text_color(content(1.)))
                .child(icon(glyph, 14.).text_color(content(0.6)))
                .child(div().flex_1().child(label))
                .when(checked, |d| {
                    d.child(icon("check-2", 14.).text_color(accent_color()))
                })
        };
        div()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(content(0.08))
            .child(
                div()
                    .p_2()
                    .child(
                        row(
                            "quick-workspace-current",
                            "folder",
                            "Current checkout",
                            current,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.choose_workspace(false, window, cx)
                        })),
                    )
                    .child(
                        row("quick-workspace-new", "folder-tree", "New worktree", new).on_click(
                            cx.listener(|this, _, window, cx| {
                                this.choose_workspace(true, window, cx)
                            }),
                        ),
                    ),
            )
            .child(
                div()
                    .px_4()
                    .pt_1()
                    .pb_1()
                    .text_size(px(11.))
                    .text_color(content(0.45))
                    .child("Existing worktrees"),
            )
            .children(
                self.git_picker
                    .clone()
                    .map(|picker| div().h(px(200.)).flex().flex_col().child(picker)),
            )
            .into_any_element()
    }

    /// QuickModelSelector.tsx's footer: fast mode, the effort, and a reset.
    pub(super) fn effort(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let model = self
            .models
            .as_ref()?
            .read(cx)
            .catalog_model(self.kind, self.model.as_deref())?
            .clone();
        if model.efforts.is_empty() {
            return None;
        }
        let count = model.efforts.len();
        let selected = self
            .effort
            .clone()
            .filter(|e| model.efforts.contains(e))
            .or(model.default_effort.clone())
            .unwrap_or_else(|| model.efforts[0].clone());
        let index = model
            .efforts
            .iter()
            .position(|e| *e == selected)
            .unwrap_or(0);
        let fraction = index as f32 / (count.max(2) - 1) as f32;
        let fast = self.fast;
        let efforts = model.efforts.clone();
        let track = self.effort_track.clone();
        let pick = move |this: &mut Self, x: gpui::Pixels, cx: &mut Context<Self>| {
            let Some(bounds) = track.get() else {
                return;
            };
            let inset = px(10.);
            let width = (bounds.size.width - inset * 2.).max(px(1.));
            let ratio = ((x - bounds.origin.x - inset) / width).clamp(0., 1.);
            let ix = (ratio * (efforts.len() - 1) as f32).round() as usize;
            if let Some(effort) = efforts.get(ix) {
                this.effort = Some(effort.clone());
                cx.notify();
            }
        };
        let pick_down = pick.clone();
        let effort_label: SharedString = effort_name(&selected).into();
        Some(
            div()
                .flex_shrink_0()
                .border_t_1()
                .border_color(content(0.08))
                .px_4()
                .pt_2()
                .pb_3()
                .child(
                    div()
                        .mb_1()
                        .flex()
                        .items_center()
                        .child(div().size(px(28.)).map(|d| {
                            if !model.fast {
                                return d;
                            }
                            d.child(
                                div()
                                    .id("quick-fast")
                                    .role(Role::Button)
                                    .aria_label("Fast mode")
                                    .aria_toggled(if fast {
                                        gpui::Toggled::True
                                    } else {
                                        gpui::Toggled::False
                                    })
                                    .cursor_pointer()
                                    .size(px(28.))
                                    .rounded(px(6.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .when(fast, |d| d.bg(amber(0.2)).hover(|s| s.bg(amber(0.3))))
                                    .when(!fast, |d| d.hover(|s| s.bg(content(0.08))))
                                    .child(icon("zap", 16.).text_color(if fast {
                                        amber(1.)
                                    } else {
                                        content(0.4)
                                    }))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.fast = !this.fast;
                                        cx.notify();
                                    })),
                            )
                        }))
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .justify_center()
                                .text_size(px(13.))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(accent_color())
                                .child(effort_label.clone()),
                        )
                        .child(
                            div()
                                .id("quick-effort-reset")
                                .role(Role::Button)
                                .aria_label("Reset to saved defaults")
                                .group("quick-effort-reset")
                                .cursor_pointer()
                                .size(px(28.))
                                .rounded(px(6.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(|s| s.bg(content(0.08)))
                                .child(
                                    icon("rotate-ccw", 16.)
                                        .text_color(content(0.4))
                                        .group_hover("quick-effort-reset", |s| {
                                            s.text_color(content(1.))
                                        }),
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.effort = crate::chat::conversation::preferred_effort(
                                        this.kind.id(),
                                        cx,
                                    );
                                    this.fast = false;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .id("quick-effort")
                        .role(Role::Slider)
                        .aria_label(if self.kind == Kind::Codex {
                            "Reasoning"
                        } else {
                            "Effort"
                        })
                        .aria_value(effort_label)
                        .relative()
                        .h(px(24.))
                        .cursor_pointer()
                        .child(self.effort_track.probe())
                        .child(
                            div()
                                .absolute()
                                .left_0()
                                .right_0()
                                .top(px(6.))
                                .h(px(12.))
                                .rounded_full()
                                .overflow_hidden()
                                .bg(content(0.15))
                                .child(
                                    div()
                                        .h_full()
                                        .w(gpui::relative(fraction))
                                        .bg(accent_color()),
                                ),
                        )
                        .child(
                            div()
                                .absolute()
                                .left(px(10.))
                                .right(px(10.))
                                .top(px(12.))
                                .children((0..count).map(|ix| {
                                    div()
                                        .absolute()
                                        .left(gpui::relative(ix as f32 / (count.max(2) - 1) as f32))
                                        .ml(px(-2.))
                                        .top(px(-2.))
                                        .size(px(4.))
                                        .rounded_full()
                                        .bg(if ix <= index {
                                            white(0.45)
                                        } else {
                                            content(0.25)
                                        })
                                })),
                        )
                        .child(
                            div()
                                .absolute()
                                .left(px(10.))
                                .right(px(10.))
                                .top(px(2.))
                                .child(
                                    div()
                                        .absolute()
                                        .left(gpui::relative(fraction))
                                        .ml(px(-10.))
                                        .size(px(20.))
                                        .rounded_full()
                                        .border_1()
                                        .border_color(black(0.05))
                                        .bg(white(1.))
                                        .shadow(vec![gpui::BoxShadow {
                                            color: black(0.2),
                                            offset: gpui::point(px(0.), px(1.)),
                                            blur_radius: px(4.),
                                            spread_radius: px(0.),
                                            inset: false,
                                        }]),
                                ),
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                                this.effort_drag = true;
                                pick_down(this, event.position.x, cx);
                            }),
                        ),
                )
                .on_mouse_move(
                    cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                        if this.effort_drag {
                            if event.pressed_button == Some(MouseButton::Left) {
                                pick(this, event.position.x, cx);
                            } else {
                                this.effort_drag = false;
                            }
                        }
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| this.effort_drag = false),
                )
                .into_any_element(),
        )
    }
}
