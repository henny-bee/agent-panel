use super::*;

impl Shell {
    pub(super) fn start_rename(
        &mut self,
        target: RenameTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_preview(cx);
        let text = match &target {
            RenameTarget::Session(id) => self.session_metadata(*id, cx).display_title(),
            RenameTarget::Folder(folder) => self
                .project_groups()
                .folder(folder)
                .map(|f| f.name.clone())
                .unwrap_or_default(),
        };
        let input = cx.new(|cx| {
            let mut input = input::Input::field(
                "Rename. Press Enter to confirm or Escape to cancel.",
                "",
                content(0.35),
                cx,
            );
            input.set_font_size(if self.workspace.classic_layout {
                13.
            } else {
                14.
            });
            input
        });
        let len = text.len();
        input.update(cx, |i, cx| {
            i.set_text(text, cx);
            i.select_range(0..len, cx);
        });
        let focus = input.read(cx).focus_handle(cx);
        let events = vec![
            cx.subscribe_in(
                &input,
                window,
                |this, _, _: &input::Submitted, window, cx| this.finish_rename(true, window, cx),
            ),
            cx.on_blur(&focus, window, |this, window, cx| {
                this.finish_rename(true, window, cx)
            }),
        ];
        window.focus(&focus, cx);
        self.renaming = Some(Renaming {
            target,
            input,
            _events: events,
        });
        self.invalidate_chrome(cx);
        cx.notify();
    }

    pub(super) fn finish_rename(
        &mut self,
        commit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(renaming) = self.renaming.take() else {
            return;
        };
        let name = renaming.input.read(cx).content.trim().to_string();
        if commit && !name.is_empty() {
            match renaming.target {
                RenameTarget::Session(id) => {
                    if self.session_summaries.contains_key(&id)
                        && self.session_metadata(id, cx).summary.title != name
                    {
                        if let Some(summary) = self.session_summaries.get_mut(&id) {
                            summary.title = name.clone();
                        }
                        if let Some(session) = self.sessions.get(&id) {
                            session.update(cx, |s, cx| {
                                s.rename(name.clone(), cx);
                            });
                        }
                        self.store_write("rename the session", cx, move |s| s.title(id, &name));
                    }
                }
                RenameTarget::Folder(folder) => {
                    self.update_groups(cx, |groups| groups.rename_folder(&folder, &name))
                }
            }
        }
        if renaming.input.read(cx).focus_handle(cx).is_focused(window) {
            self.focus_active(window, cx);
        }
        self.invalidate_chrome(cx);
        cx.notify();
    }

    /// SessionRenameRow and FolderRenameRow: the name in a focused field.
    pub(crate) fn rename_row(
        &self,
        target: &RenameTarget,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let renaming = self.renaming.as_ref().filter(|r| r.target == *target)?;
        if self.workspace.classic_layout {
            let folder = match target {
                RenameTarget::Folder(id) => self.project_groups().folder(id).cloned(),
                _ => None,
            };
            let mut outline = crate::ui::theme::accent_color();
            outline.a = 0.4;
            let input = div()
                .flex_1()
                .min_w_0()
                .h(px(if folder.is_some() { 21.875 } else { 25.875 }))
                .px_2()
                .flex()
                .items_center()
                .rounded_sm()
                .bg(content(0.1))
                .border_1()
                .border_color(outline)
                .text_size(px(13.))
                .line_height(px(17.875))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(content(1.))
                .child(renaming.input.clone());
            let row = div()
                .key_context("SessionRename")
                .on_action(cx.listener(|this, _: &CancelRename, window, cx| {
                    this.finish_rename(false, window, cx)
                }))
                .w_full()
                .flex()
                .items_center()
                .gap_1p5()
                .text_color(content(1.));
            return Some(if let Some(folder) = folder {
                row.px_2()
                    .py(px(6.))
                    .rounded_t_md()
                    .bg(self.classic_group_fill(&Entry::Folder {
                        id: folder.id.clone(),
                        sessions: Vec::new(),
                    }))
                    .child(
                        div()
                            .size(px(16.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon("chevron-down", 14.).text_color(content(0.5))),
                    )
                    .child(input)
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(11.))
                            .line_height(px(16.5))
                            .text_color(content(0.45))
                            .child(folder.session_ids.len().to_string()),
                    )
                    .into_any_element()
            } else {
                let (active, approval) = match target {
                    RenameTarget::Session(id) => (
                        self.workspace.active == Some(*id),
                        self.session_metadata(*id, cx).needs_approval(),
                    ),
                    _ => (false, false),
                };
                row.px(px(10.))
                    .py_2()
                    .rounded_md()
                    .when(active, |d| d.bg(content(0.1)))
                    .when(approval, |d| {
                        d.bg(gpui::Hsla::from(rgb(0xfbbf24)).opacity(0.1))
                    })
                    .child(input)
                    .into_any_element()
            });
        }
        Some(
            div()
                .key_context("SessionRename")
                .on_action(cx.listener(|this, _: &CancelRename, window, cx| {
                    this.finish_rename(false, window, cx)
                }))
                .h(px(32.))
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .size(px(16.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(matches!(target, RenameTarget::Folder(_)), |d| {
                            d.child(icon("chevron-down", 14.))
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h(px(22.))
                        .px_2()
                        .flex()
                        .items_center()
                        .rounded_sm()
                        .bg(content(0.1))
                        .border_1()
                        .border_color(rgba(0x4da3f566))
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(content(1.))
                        .child(renaming.input.clone()),
                )
                .into_any(),
        )
    }

    pub(super) fn open_link_dialog(
        &mut self,
        id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_preview(cx);
        let initial = self.session_metadata(id, cx).summary.work_item.clone();
        let input = cx.new(|cx| {
            input::Input::field(
                "GitHub issue or pull request URL",
                "https://github.com/owner/repo/pull/123",
                content(0.3),
                cx,
            )
        });
        if let Some(item) = &initial {
            let url = item.url.clone();
            input.update(cx, |i, cx| i.set_text(url, cx));
        }
        let events = vec![
            cx.subscribe_in(
                &input,
                window,
                |this, _, _: &input::Submitted, window, cx| this.save_link(true, window, cx),
            ),
            cx.subscribe(&input, |this, _, _: &input::Changed, cx| {
                if let Some(dialog) = &mut this.link_dialog
                    && dialog.error.take().is_some()
                {
                    cx.notify();
                }
            }),
        ];
        window.focus(&input.read(cx).focus_handle(cx), cx);
        self.link_dialog = Some(LinkDialog {
            id,
            input,
            error: None,
            editing: initial.is_some(),
            _events: events,
        });
        cx.notify();
    }

    pub(crate) fn close_link_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.link_dialog.take().is_some() {
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    /// Saves the parsed link, or with `keep` false, removes it.
    pub(super) fn save_link(&mut self, keep: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = &mut self.link_dialog else {
            return;
        };
        let id = dialog.id;
        let item = if keep {
            match parse_github_work_item(dialog.input.read(cx).content.trim()) {
                Some(item) => Some(item),
                None => {
                    dialog.error = Some("Enter a valid GitHub issue or pull request URL.".into());
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        if let Some(summary) = self.session_summaries.get_mut(&id) {
            summary.work_item = item.clone();
        }
        if let Some(session) = self.sessions.get(&id) {
            session.update(cx, |s, _| s.summary.work_item = item.clone());
        }
        self.store_write("link the session", cx, move |s| {
            s.set_work_item(id, item.as_ref())
        });
        self.invalidate_chrome(cx);
        self.close_link_dialog(window, cx);
    }

    /// LinkSessionWorkItemDialog, centred over the window.
    pub(crate) fn link_modal(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dialog = self.link_dialog.as_ref()?;
        let title = self.session_metadata(dialog.id, cx).display_title();
        let viewport = window.viewport_size();
        let width = 384f32.min(viewport.width.as_f32() - 24.);
        let anchor = Anchor::Center;
        let weak = cx.weak_entity();
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .role(Role::Button)
                .cursor_pointer()
                .px_3()
                .py_1p5()
                .rounded_md()
                .text_size(px(12.))
                .child(label)
        };
        let field = div()
            .h(px(36.))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded_md()
            .border_1()
            .border_color(if dialog.error.is_some() {
                rgba(0xf8717199).into()
            } else {
                content(0.1)
            })
            .bg(content(0.05))
            .text_size(px(13.))
            .text_color(content(1.))
            .child(dialog.input.clone());
        Some(
            div()
                .absolute()
                .size_full()
                .child(crate::ui::modal::scrim("link-work-item-scrim"))
                .child(
                    popover("link-work-item-modal", anchor)
                        .modal_surface(self.workspace.boxy_layout())
                        .width(width)
                        .gap(0.)
                        .priority(60)
                        .on_dismiss(move |window, cx| {
                            let _ = weak.update(cx, |this, cx| this.close_link_dialog(window, cx));
                        })
                        .child(
                            div()
                                .id("link-work-item-dialog")
                                .debug_selector(|| "link-work-item-dialog".into())
                                .role(Role::Dialog)
                                .aria_label(if dialog.editing {
                                    "Edit GitHub link"
                                } else {
                                    "Link GitHub issue or PR"
                                })
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .px_4()
                                        .pt_4()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_size(px(14.))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(content(1.))
                                                .child(if dialog.editing {
                                                    "Edit GitHub link"
                                                } else {
                                                    "Link GitHub issue or PR"
                                                }),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(12.))
                                                .text_color(content(0.5))
                                                .truncate()
                                                .child(title),
                                        ),
                                )
                                .child(
                                    div()
                                        .p_4()
                                        .flex()
                                        .flex_col()
                                        .gap_4()
                                        .text_size(px(12.))
                                        .child(
                                            div()
                                                .flex()
                                                .flex_col()
                                                .gap_1p5()
                                                .child(
                                                    div()
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .text_color(content(0.8))
                                                        .child("Issue or pull request URL"),
                                                )
                                                .child(field)
                                                .child(match &dialog.error {
                                                    Some(error) => div()
                                                            .id("link-error")
                                                        .role(Role::Alert)
                                                        .text_size(px(11.))
                                                        .text_color(rgb(0xf87171))
                                                        .child(error.clone()),
                                                    None => div()
                                                            .id("link-help")
                                                        .text_size(px(11.))
                                                        .text_color(content(0.45))
                                                        .child(
                                                            "Paste the full github.com URL. The linked item will appear on the session card.",
                                                        ),
                                                }),
                                        )
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .justify_end()
                                                .gap_2()
                                                .when(dialog.editing, |d| {
                                                    d.child(
                                                        button("link-remove", "Remove link")
                                                            .text_color(rgb(0xf87171))
                                                            .hover(|s| s.bg(rgba(0xf871711a)))
                                                            .on_click(cx.listener(|this, _, window, cx| {
                                                                this.save_link(false, window, cx)
                                                            })),
                                                    )
                                                    .child(div().flex_1())
                                                })
                                                .child(
                                                    crate::ui::modal::ghost_button("link-cancel", "Cancel")
                                                        .cursor_pointer()
                                                        .on_click(cx.listener(|this, _, window, cx| {
                                                            this.close_link_dialog(window, cx)
                                                        })),
                                                )
                                                .child(
                                                    crate::ui::modal::primary_button(
                                                        "link-save",
                                                        if dialog.editing { "Update link" } else { "Link" },
                                                    )
                                                    .debug_selector(|| "link-save".into())
                                                    .cursor_pointer()
                                                    .on_click(cx.listener(|this, _, window, cx| {
                                                        this.save_link(true, window, cx)
                                                    })),
                                                ),
                                        ),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}
