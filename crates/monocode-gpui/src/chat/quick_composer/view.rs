use super::*;

/// The toolbar's and header's pill buttons.
pub(super) fn pill(
    id: &'static str,
    label: impl Into<SharedString>,
    open: bool,
    ink: f32,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.into())
        .aria_expanded(open)
        .cursor_pointer()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded(px(6.))
        .text_size(px(12.))
        .when(open, |d| d.bg(content(0.12)).text_color(content(1.)))
        .when(!open, |d| {
            d.text_color(content(ink))
                .hover(|s| s.bg(content(0.08)).text_color(content(1.)))
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

impl QuickComposer {
    pub(super) fn header(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let new = self.workspace.new_worktree;
        let linked = self.workspace.tree.is_some();
        let enabled = !self.busy
            && !matches!(self.picker, Some(p) if !matches!(p, Picker::Workspace | Picker::Branch))
            && self.git.as_ref().is_some_and(|g| g.current.is_some());
        let label = if new {
            "New worktree"
        } else {
            "Current checkout"
        };
        let current = self.git.as_ref().and_then(|g| g.current.clone());
        let base = self
            .workspace
            .base
            .clone()
            .or(current.clone())
            .unwrap_or_else(|| "HEAD".into());
        let branch = if new {
            format!("From {base}")
        } else {
            current.unwrap_or_else(|| {
                if self.git_settled {
                    "No repo".into()
                } else {
                    "Loading…".into()
                }
            })
        };
        let loading = !self.git_settled && self.git.is_none();
        let tree = linked
            && self.git.as_ref().is_some_and(|g| {
                g.worktrees
                    .iter()
                    .any(|w| !w.main && Some(w.path.as_str()) == self.workspace.tree.as_deref())
            });
        let project = if self.cwd.is_empty() {
            "No project".to_owned()
        } else {
            self.project_label(&self.cwd)
        };
        div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_2()
            .px(px(20.))
            .pt_3()
            .pr_8()
            .child(
                pill(
                    "quick-workspace",
                    format!("Workspace {label}"),
                    self.picker == Some(Picker::Workspace),
                    0.55,
                )
                .h(px(24.))
                .ml(px(-6.))
                .px(px(6.))
                .max_w(px(192.))
                .when(!enabled, |d| d.opacity(0.4))
                .child(
                    icon(
                        if new || linked {
                            "folder-tree"
                        } else {
                            "folder"
                        },
                        14.,
                    )
                    .text_color(content(0.55)),
                )
                .child(div().truncate().child(label))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if enabled {
                        this.open(Picker::Workspace, window, cx)
                    }
                })),
            )
            .child(
                pill(
                    "quick-branch",
                    if new {
                        format!("Create worktree from {base}")
                    } else {
                        "Choose branch".into()
                    },
                    self.picker == Some(Picker::Branch),
                    0.55,
                )
                .h(px(24.))
                .ml(px(-6.))
                .px(px(6.))
                .max_w(px(256.))
                .when(!enabled, |d| d.opacity(0.4))
                .child(
                    icon(if tree { "folder-tree" } else { "git-branch" }, 14.)
                        .text_color(content(0.55)),
                )
                .child(div().relative().min_w_0().truncate().map(|d| {
                    if loading {
                        // Reserve the line box while the current branch loads.
                        d.child(div().invisible().child("main")).child(
                            div()
                                .absolute()
                                .left_0()
                                .right_0()
                                .top(px(5.))
                                .h(px(6.))
                                .rounded_full()
                                .bg(content(0.3)),
                        )
                    } else {
                        d.child(branch)
                    }
                }))
                .when(tree, |d| {
                    d.child(
                        div()
                            .flex_shrink_0()
                            .px_1()
                            .rounded(px(4.))
                            .bg(content(0.08))
                            .text_size(px(10.))
                            .text_color(content(0.45))
                            .child("Worktree"),
                    )
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    if enabled {
                        this.open(Picker::Branch, window, cx)
                    }
                })),
            )
            .child(
                pill(
                    "quick-project",
                    "Project (⌘P)",
                    self.picker == Some(Picker::Project),
                    0.55,
                )
                .ml_auto()
                .max_w(px(WIDTH * 0.4))
                .h(px(24.))
                .px(px(6.))
                .when(self.projects.is_empty(), |d| d.opacity(0.5))
                .when(!self.cwd.is_empty(), |d| {
                    d.child(crate::ui::project_badge::project_icon(
                        &self.appearance.get(&self.cwd).cloned().unwrap_or_default(),
                        &self.cwd,
                        12.,
                    ))
                })
                .child(div().truncate().child(project))
                .child(icon("chevron-down", 12.).text_color(content(0.4)))
                .on_click(cx.listener(|this, _, window, cx| {
                    if !this.projects.is_empty() {
                        this.open(Picker::Project, window, cx)
                    }
                })),
            )
    }

    pub(super) fn attachment_chips(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.attachments.is_empty() {
            return None;
        }
        let attachments = &self.attachments;
        self.photos.retain(|path, _| attachments.contains(path));
        for path in &self.attachments {
            if let Some(mime) = monocode_core::session::composer::image_mime(path)
                && !self.photos.contains_key(path)
            {
                let origin = crate::chat::transcript::attachments::Origin::Path(path.clone());
                let photo = cx.new(|cx| {
                    crate::chat::transcript::attachments::Photo::new(origin, mime.into(), cx)
                });
                self.photos.insert(path.clone(), photo);
            }
        }
        let busy = self.busy;
        Some(
            div()
                .id("quick-attachments")
                .role(Role::Group)
                .aria_label("Attachments")
                .flex_shrink_0()
                .max_h(px(112.))
                .overflow_y_scroll()
                .flex()
                .flex_wrap()
                .gap(px(6.))
                .px(px(20.))
                .pt_4()
                .pb_1()
                .children(self.attachments.iter().enumerate().map(|(ix, path)| {
                    let label = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let photo = self.photos.get(path).cloned();
                    let remove = div()
                        .id(("quick-remove-attachment", ix))
                        .role(Role::Button)
                        .aria_label(format!("Remove {label}"))
                        .cursor_pointer()
                        .flex_shrink_0()
                        .size(px(16.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(content(0.6))
                        .hover(|s| s.bg(content(0.15)).text_color(content(1.)))
                        .child(icon("close", 10.))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if ix < this.attachments.len() {
                                let path = this.attachments.remove(ix);
                                this.staged_attachments.retain(|staged| staged != &path);
                                cx.notify();
                            }
                        }));
                    div()
                        .id(("quick-attachment", ix))
                        .h(px(28.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .pl_1()
                        .pr_1()
                        .rounded(px(6.))
                        .bg(content(0.06))
                        .text_size(px(11.))
                        .child(match photo {
                            Some(photo) => div()
                                .size(px(20.))
                                .rounded(px(4.))
                                .overflow_hidden()
                                .child(photo)
                                .into_any_element(),
                            None => {
                                crate::ui::file_icons::file_type_icon(&label, false, false, 14.)
                            }
                        })
                        .child(div().max_w(px(190.)).truncate().child(label))
                        .when(!busy, |d| d.child(remove))
                }))
                .into_any_element(),
        )
    }

    pub(super) fn toolbar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let supported = self.attachments_supported();
        let name = model_display_name(
            self.kind,
            self.model.as_deref().unwrap_or(""),
            &model_name(self.kind, self.model.as_deref()),
        );
        let can_submit = self.can_submit(cx);
        let draft = self.leading(cx) == Some(Command::Draft);
        // As the effort footer: only for a model that offers fast mode.
        let fast = self.fast
            && self.models.as_ref().is_some_and(|models| {
                models
                    .read(cx)
                    .catalog_model(self.kind, self.model.as_deref())
                    .is_some_and(|model| model.fast)
            });
        div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(6.))
            .border_t_1()
            .border_color(content(0.08))
            .px_3()
            .py_2()
            .child(
                pill(
                    "quick-attach",
                    "Add attachment",
                    self.picker == Some(Picker::Attachments),
                    0.7,
                )
                .relative()
                .size(px(26.))
                .flex_shrink_0()
                .justify_center()
                .when(!supported || self.attaching || self.busy, |d| {
                    d.opacity(0.4)
                })
                .child(self.plus.probe())
                .child(icon("plus", 14.).text_color(content(0.7)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if supported && !this.attaching && !this.busy {
                        this.open(Picker::Attachments, window, cx)
                    }
                })),
            )
            .child(
                pill(
                    "quick-model",
                    "Model (⌘.)",
                    self.picker == Some(Picker::Model),
                    0.7,
                )
                .max_w(px(WIDTH * 0.4))
                .px_2()
                .py_1()
                .child(provider_icon(self.kind.id(), 14.))
                .child(div().truncate().child(name))
                .when(fast, |d| d.child(crate::chat::conversation::fast_bolt()))
                .child(icon("chevron-down", 12.).text_color(content(0.4)))
                .on_click(cx.listener(|this, _, window, cx| this.open(Picker::Model, window, cx))),
            )
            .when(self.kind != Kind::Fx, |d| {
                let permission = PERMISSIONS
                    .iter()
                    .find(|(mode, ..)| *mode == self.access)
                    .map_or("lock", |(_, icon, _)| *icon);
                d.child(
                    pill(
                        "quick-permissions",
                        "Permissions",
                        self.picker == Some(Picker::Permissions),
                        0.7,
                    )
                    .px_2()
                    .py_1()
                    .child(icon(permission, 14.).text_color(
                        if self.access == AccessMode::FullAccess {
                            amber(0.9)
                        } else {
                            content(0.7)
                        },
                    ))
                    .child(div().truncate().child(self.access.label()))
                    .child(icon("chevron-down", 12.).text_color(content(0.4)))
                    .on_click(
                        cx.listener(|this, _, window, cx| {
                            this.open(Picker::Permissions, window, cx)
                        }),
                    ),
                )
            })
            .child(
                div()
                    .ml_auto()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_3()
                    .text_size(px(11.))
                    .text_color(content(0.45))
                    .map(|d| {
                        if self.attaching {
                            d.child(
                                div()
                                    .id("quick-attaching")
                                    .role(Role::Status)
                                    .child("Adding attachment…"),
                            )
                        } else if let Some(error) = self.error.clone() {
                            d.child(
                                div()
                                    .max_w(px(288.))
                                    .truncate()
                                    .text_color(gpui::rgb(0xf87171))
                                    .child(error),
                            )
                        } else {
                            d.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(kbd("↵"))
                                    .child("start"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(kbd("⌘↵"))
                                    .child("start and open"),
                            )
                        }
                    })
                    .child(
                        div()
                            .id("quick-send")
                            .role(Role::Button)
                            .aria_label(if draft { "Save draft" } else { "Start" })
                            .px(px(10.))
                            .py_1()
                            .rounded(px(6.))
                            .bg(accent_color())
                            .text_size(px(12.))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(white(1.))
                            .when(!can_submit, |d| d.opacity(0.4))
                            .when(can_submit, |d| d.cursor_pointer())
                            .child(if draft { "Save draft" } else { "Start" })
                            .on_click(cx.listener(|this, _, _, cx| this.launch(false, cx))),
                    ),
            )
    }

    pub(super) fn option_row(
        &self,
        id: (&'static str, usize),
        active: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let ix = id.1;
        div()
            .id(id)
            .role(Role::ListBoxOption)
            .aria_selected(active)
            .cursor_pointer()
            .w_full()
            .rounded(px(8.))
            .px_2()
            .py(px(6.))
            .text_size(px(13.))
            .when(active, |d| d.bg(content(0.12)).text_color(content(1.)))
            .when(!active, |d| d.text_color(content(0.75)))
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.highlight = ix;
                    cx.notify();
                }
            }))
    }

    pub(super) fn commands(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let options = self.command_options(cx);
        div()
            .id("quick-composer-commands")
            .role(Role::ListBox)
            .aria_label("Commands")
            .flex_shrink_0()
            .border_t_1()
            .border_color(content(0.08))
            .p_2()
            .when(options.is_empty(), |d| {
                d.child(
                    div()
                        .px_2()
                        .py_2()
                        .text_size(px(12.))
                        .text_color(content(0.45))
                        .child("No matching commands"),
                )
            })
            .children(
                options
                    .into_iter()
                    .enumerate()
                    .map(|(ix, (command, description))| {
                        let name = command.name();
                        let label = format!("{}{}", name[..1].to_uppercase(), &name[1..]);
                        self.option_row(("quick-command", ix), ix == self.highlight, cx)
                            .aria_label(label.clone())
                            .flex()
                            .flex_col()
                            .items_start()
                            .gap(px(2.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(icon(command.icon(), 14.).text_color(command.color()))
                                    .child(label),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .line_height(px(16.))
                                    .text_color(content(0.5))
                                    .child(description),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.choose_command(ix, cx);
                                window.focus(&this.input.focus_handle(cx), cx);
                            }))
                    }),
            )
            .into_any_element()
    }
}

impl Render for QuickComposer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (height, fade, held) = self.picker_motion(window, cx);
        self.fit(height, window, cx);
        if !cx.has_active_drag() {
            self.dragging = false;
        }
        let picker = self.picker;
        let warning = !self.attachments.is_empty() && !self.attachments_supported();
        let fill = if is_light() {
            floating_fill().opacity(0.8)
        } else {
            gpui::rgba(0x1717178c).into()
        };
        let mut root = div()
            .id("quick-composer")
            .key_context("QuickComposer")
            .w_full()
            .text_color(content(1.))
            .font_family(crate::ui::fonts::UI)
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                let m = &event.keystroke.modifiers;
                if key == "escape" {
                    if this.picker.is_some() {
                        this.close_picker(window, cx);
                    } else {
                        this.dismiss(window, cx);
                    }
                    cx.stop_propagation();
                } else if key == "enter" && m.platform {
                    this.launch(true, cx);
                    cx.stop_propagation();
                } else if key == "p" && m.platform {
                    this.open(Picker::Project, window, cx);
                    cx.stop_propagation();
                } else if key == "." && m.platform {
                    this.open(Picker::Model, window, cx);
                    cx.stop_propagation();
                } else if key == "tab" && this.picker == Some(Picker::Commands) {
                    this.choose_command(this.highlight, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &input::Paste, _, cx| {
                if this.paste(cx) {
                    cx.stop_propagation();
                }
            }))
            .on_drag_move::<ExternalPaths>(cx.listener(
                |this, event: &gpui::DragMoveEvent<ExternalPaths>, _, cx| {
                    let over = event.bounds.contains(&event.event.position);
                    if this.dragging != over {
                        this.dragging = over;
                        cx.notify();
                    }
                },
            ))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.dragging = false;
                this.add_attachments(paths.paths().to_vec(), cx);
                this.focus(window, cx);
            }))
            .child(
                // The window is exactly this card, so the native shadow and
                // blur follow its rounded outline.
                div()
                    .relative()
                    .w_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded(px(RADIUS))
                    .border_1()
                    .border_color(content(0.1))
                    .bg(fill)
                    // Mid-tween the card clips its full-height contents, so the
                    // lists are revealed rather than squeezed.
                    .when(held, |d| d.h(px(height)))
                    .child(
                div()
                    .relative()
                    .w_full()
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .child(self.card.layout_probe())
                    .when(picker == Some(Picker::Commands), |d| {
                        d.capture_action(cx.listener(|this, _: &input::Up, _, cx| {
                            this.step(-1, cx);
                            cx.stop_propagation();
                        }))
                        .capture_action(cx.listener(|this, _: &input::Down, _, cx| {
                            this.step(1, cx);
                            cx.stop_propagation();
                        }))
                    })
                    .child(
                        div()
                            .id("quick-drag")
                            .group("quick-drag")
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .h(px(12.))
                            .flex()
                            .justify_center()
                            .pt_1()
                            .cursor_grab()
                            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                                cx.stop_propagation();
                                window.start_window_move();
                            })
                            .child(
                                div()
                                    .w(px(24.))
                                    .h(px(2.))
                                    .rounded_full()
                                    .bg(content(0.15))
                                    .group_hover("quick-drag", |s| s.bg(content(0.35))),
                            ),
                    )
                    .child(
                        div()
                            .id("quick-close")
                            .role(Role::Button)
                            .aria_label("Close composer")
                            .group("quick-close")
                            .cursor_pointer()
                            .absolute()
                            .right_2()
                            .top(px(14.))
                            .size(px(20.))
                            .rounded(px(4.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|s| s.bg(content(0.08)))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(
                                icon("close", 12.)
                                    .text_color(content(0.35))
                                    .group_hover("quick-close", |s| s.text_color(content(1.))),
                            )
                            .on_click(cx.listener(|this, _, window, cx| this.dismiss(window, cx))),
                    )
                    .child(self.header(cx))
                    .children(self.attachment_chips(cx))
                    .child(
                        div()
                            .flex_shrink_0()
                            .pl(px(20.))
                            .pr(px(36.))
                            .pt_4()
                            .pb_2()
                            .child(self.input.clone()),
                    )
                    .when(warning, |d| {
                        d.child(
                            div()
                                .id("quick-attachment-warning")
                                .role(Role::Alert)
                                .px(px(20.))
                                .pb_2()
                                .text_size(px(12.))
                                .text_color(amber(1.))
                                .child(
                                    "Choose a provider that supports attachments, or remove the attached files.",
                                ),
                        )
                    })
                    .child(self.toolbar(cx))
                    .child(div().flex().flex_col().flex_shrink_0().opacity(fade).map(|d| match picker {
                        Some(Picker::Commands) => d.child(self.commands(cx)),
                        Some(Picker::Permissions) => d.child(self.permissions(cx)),
                        Some(Picker::Project) => d.child(self.project_list(window, cx)),
                        Some(Picker::Workspace) => d.child(self.workspace_list(cx)),
                        Some(Picker::Branch) => d.children(self.git_picker.clone().map(|p| {
                            div()
                                .flex_shrink_0()
                                .h(px(280.))
                                .flex()
                                .flex_col()
                                .border_t_1()
                                .border_color(content(0.08))
                                .child(p)
                        })),
                        Some(Picker::Model) => {
                            let effort = self.effort(cx);
                            d.children(self.models.clone().map(|list| {
                                div()
                                    .flex_shrink_0()
                                    .border_t_1()
                                    .border_color(content(0.08))
                                    .child(list)
                            }))
                            .children(effort)
                        }
                        _ => d,
                    })),
                    )
                    .when(self.dragging, |d| {
                        d.child(
                            div()
                                .absolute()
                                .inset_0()
                                .rounded(px(RADIUS))
                                .border_1()
                                .border_dashed()
                                .border_color(accent_color().opacity(0.6))
                                .bg(floating_fill().opacity(0.9))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(14.))
                                .text_color(accent_color())
                                .child("Drop to attach"),
                        )
                    }),
            );
        if picker == Some(Picker::Attachments) {
            let weak = cx.weak_entity();
            root = root.child(
                popover("quick-attach-options", Anchor::Slot(self.plus.clone()))
                    .side(Side::Top)
                    .width(220.)
                    .on_dismiss(move |window, cx| {
                        let _ = weak.update(cx, |this, cx| this.close_picker(window, cx));
                    })
                    .child(
                        div()
                            .p_1()
                            .child(
                                div()
                                    .id("quick-choose-files")
                                    .role(Role::MenuItem)
                                    .aria_label("Choose files…")
                                    .cursor_pointer()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .px_2()
                                    .py(px(6.))
                                    .rounded(px(6.))
                                    .text_size(px(12.))
                                    .hover(|s| s.bg(content(0.08)))
                                    .child(icon("image-add", 14.).text_color(content(0.75)))
                                    .child("Choose files…")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.choose_files(window, cx)
                                    })),
                            )
                            .child(
                                div()
                                    .id("quick-screenshot")
                                    .role(Role::MenuItem)
                                    .aria_label("Take screenshot…")
                                    .cursor_pointer()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .px_2()
                                    .py(px(6.))
                                    .rounded(px(6.))
                                    .text_size(px(12.))
                                    .hover(|s| s.bg(content(0.08)))
                                    .child(icon("maximize-2", 14.).text_color(content(0.75)))
                                    .child("Take screenshot…")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.take_screenshot(window, cx)
                                    })),
                            ),
                    ),
            );
        }
        root
    }
}
