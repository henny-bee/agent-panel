use super::*;

impl Shell {
    /// SessionCard's linked issue or PR, opened on GitHub. Hover underlines
    /// it as a link; with no number shown, the line runs under the icon.
    pub(crate) fn work_item_badge(
        &self,
        id: i64,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let item = self.session_metadata(id, cx).summary.work_item.clone()?;
        let noun = if item.kind == "pr" { "PR" } else { "issue" };
        let url = item.url.clone();
        Some(
            div()
                .id(("work-item", id as usize))
                .role(Role::Link)
                .aria_label(format!("Open {noun} #{}", item.number))
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(2.))
                .px(px(2.))
                .rounded_sm()
                .text_size(px(11.))
                .text_color(rgb(ACCENT))
                .child(
                    icon(
                        if item.kind == "pr" {
                            "git-pull-request"
                        } else {
                            "circle-dot"
                        },
                        12.,
                    )
                    .text_color(rgb(ACCENT)),
                )
                .when(!compact, |d| d.child(format!("#{}", item.number)))
                .cursor_pointer()
                .when(compact, |d| {
                    d.border_b_1()
                        .border_color(gpui::transparent_black())
                        .hover(|s| s.border_color(rgb(ACCENT)))
                })
                .when(!compact, |d| d.hover(|s| s.underline()))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
                .on_mouse_up(MouseButton::Middle, {
                    let url = url.clone();
                    move |_, _, cx| {
                        cx.stop_propagation();
                        cx.open_url(&url);
                    }
                })
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    cx.open_url(&url);
                })
                .into_any(),
        )
    }

    /// A virtualized group renders its header separately from its member cards.
    pub(crate) fn sidebar_group_header(&self, entry: &Entry, cx: &mut Context<Self>) -> AnyElement {
        let searching = !self.query.is_empty();
        let folders = self.project_groups();
        let (key, name, glyph, color, collapsed, sessions): (
            String,
            String,
            &'static str,
            Option<gpui::Hsla>,
            bool,
            &Vec<i64>,
        ) = match entry {
            Entry::Reminders {
                collapsed,
                sessions,
            } => (
                "reminders".into(),
                "Reminders".into(),
                "clock",
                Some(rgb(REMINDERS_COLOR).into()),
                *collapsed,
                sessions,
            ),
            Entry::Folder { id, sessions } => {
                let Some(folder) = folders.folder(id) else {
                    return div().into_any();
                };
                (
                    format!("folder:{id}"),
                    folder.name.clone(),
                    "folder",
                    folder
                        .custom_color
                        .as_deref()
                        .and_then(|color| color.strip_prefix('#'))
                        .and_then(|color| u32::from_str_radix(color, 16).ok())
                        .map(|color| rgb(color).into())
                        .or_else(|| folder.color_index.and_then(folder_color)),
                    folder.collapsed,
                    sessions,
                )
            }
            Entry::Pinned { sessions } if self.workspace.classic_layout => (
                "pinned".into(),
                "Pinned".into(),
                "pin",
                None,
                folders.pinned_collapsed,
                sessions,
            ),
            Entry::Pinned { .. } | Entry::Session(_) => return div().into_any(),
        };
        let expanded = searching || !collapsed;
        let folder_id = key.strip_prefix("folder:").map(str::to_string);
        folder_id
            .as_ref()
            .and_then(|f| self.rename_row(&RenameTarget::Folder(f.clone()), cx))
            .unwrap_or_else(|| self.group_row(&key, name, glyph, color, expanded, sessions, cx))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn group_row(
        &self,
        key: &str,
        name: String,
        glyph: &'static str,
        color: Option<gpui::Hsla>,
        expanded: bool,
        sessions: &[i64],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.workspace.classic_layout {
            return self.classic_group_row(key, name, glyph, color, expanded, sessions, cx);
        }
        let approval = sessions
            .iter()
            .any(|id| self.session_metadata(*id, cx).needs_approval());
        let done = sessions.iter().any(|id| self.unseen_finished.contains(id));
        let group = SharedString::from(format!("group-row-{key}"));
        let toggle_key = key.to_string();
        let menu_key = key.to_string();
        let drag_key = key.strip_prefix("folder:").map(str::to_string);
        let bounds = drag_key
            .as_ref()
            .and_then(|id| self.folder_bounds.get(id))
            .cloned();
        let drop_target = drag_key.as_ref().is_some_and(|id| {
            self.sidebar_drop == Some(ListDrop::Folder(id.clone()))
                || self
                    .folder_drag
                    .as_ref()
                    .is_some_and(|drag| drag.active && drag.over.as_ref() == Some(id))
        });
        let mark = |side| {
            let glyph = icon(glyph, side);
            match color {
                Some(color) => glyph.text_color(color),
                None => glyph,
            }
        };
        div()
            .id(SharedString::from(format!("session-group-{key}")))
            .debug_selector({
                let key = key.to_string();
                move || format!("session-group-{key}")
            })
            .group(group.clone())
            .role(Role::Button)
            .aria_label(name.clone())
            .aria_expanded(expanded)
            .cursor_pointer()
            .h(px(32.))
            .px_2()
            .rounded_md()
            .flex()
            .items_center()
            .gap_2()
            .text_color(content(0.5))
            .when(expanded, |d| d.bg(content(0.05)).text_color(content(0.8)))
            .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
            .when(drop_target, |d| d.bg(content(0.14)))
            .when(
                drag_key.as_ref().is_some_and(|id| {
                    self.folder_drag
                        .as_ref()
                        .is_some_and(|drag| drag.active && &drag.id == id)
                }),
                |d| d.opacity(0.4),
            )
            .child(
                div()
                    .size(px(16.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(0.7)
                    .child(
                        div()
                            .when(expanded, |d| d.invisible())
                            .group_hover(group.clone(), |s| s.invisible())
                            .child(mark(16.)),
                    )
                    .child(
                        div()
                            .absolute()
                            .invisible()
                            .when(expanded, |d| d.visible())
                            .group_hover(group.clone(), |s| s.visible())
                            .child(icon(
                                if expanded {
                                    "chevron-down"
                                } else {
                                    "chevron-right"
                                },
                                16.,
                            )),
                    )
                    .relative(),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(name),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_size(px(11.))
                    .text_color(content(0.4))
                    .when(!expanded && approval, |d| {
                        d.child(icon("circle-alert", 12.).text_color(rgb(0xfbbf24)))
                    })
                    .when(!expanded && !approval && done, |d| {
                        d.child(icon("check-circle-2", 12.).text_color(rgb(0x2dd4bf)))
                    })
                    .child(sessions.len().to_string()),
            )
            .children(bounds.map(|slot| slot.probe().into_any_element()))
            .relative()
            .when_some(drag_key, |d, id| {
                d.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, _| {
                        this.skip_folder_click = false;
                        this.folder_drag = Some(FolderDrag {
                            id: id.clone(),
                            start: event.position,
                            active: false,
                            over: None,
                        });
                    }),
                )
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                if std::mem::take(&mut this.skip_folder_click) {
                    return;
                }
                this.sidebar_target = toggle_key
                    .strip_prefix("folder:")
                    .map(|id| RenameTarget::Folder(id.into()));
                window.focus(&this.sidebar_focus, cx);
                if !this.query.is_empty() {
                    return;
                }
                let key = toggle_key.clone();
                this.update_groups(cx, |groups| match key.as_str() {
                    "reminders" => groups.reminders_collapsed = !groups.reminders_collapsed,
                    folder => groups.toggle_folder(folder.trim_start_matches("folder:")),
                });
            }))
            .when(key.starts_with("folder:"), |d| {
                d.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        let folder = menu_key.trim_start_matches("folder:").to_string();
                        let anchor = Anchor::Point(event.position);
                        this.open_menu(MenuTarget::Folder(folder), anchor, window, cx);
                    }),
                )
            })
            .into_any()
    }

    /// Original FolderRow and its enclosing tinted shell. The count and label
    /// retain their original alignment in collapsed and expanded states.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn classic_group_row(
        &self,
        key: &str,
        name: String,
        glyph: &'static str,
        color: Option<gpui::Hsla>,
        expanded: bool,
        sessions: &[i64],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let approval = sessions
            .iter()
            .any(|id| self.session_metadata(*id, cx).needs_approval());
        let busy = sessions
            .iter()
            .any(|id| self.session_metadata(*id, cx).responding());
        let busy_mark = sessions
            .iter()
            .find(|id| self.session_metadata(**id, cx).responding())
            .and_then(|id| self.session_marks.get(id))
            .cloned();
        let done = sessions.iter().any(|id| self.unseen_finished.contains(id));
        let group = SharedString::from(format!("classic-group-row-{key}"));
        let toggle_key = key.to_owned();
        let menu_key = key.to_owned();
        let folder_id = key.strip_prefix("folder:").map(str::to_owned);
        let bounds = folder_id
            .as_ref()
            .and_then(|id| self.folder_bounds.get(id))
            .cloned();
        let drop_target = folder_id.as_ref().is_some_and(|id| {
            self.sidebar_drop == Some(ListDrop::Folder(id.clone()))
                || self
                    .folder_drag
                    .as_ref()
                    .is_some_and(|drag| drag.active && drag.over.as_ref() == Some(id))
        });
        let mut fill = color.unwrap_or_else(|| content(0.05));
        if color.is_some() {
            fill.a = 0.18;
        }
        div()
            .id(SharedString::from(format!("session-group-{key}")))
            .debug_selector({
                let key = key.to_owned();
                move || format!("session-group-{key}")
            })
            .group(group.clone())
            .role(Role::Button)
            .aria_label(name.clone())
            .aria_expanded(expanded)
            .cursor_pointer()
            .relative()
            .h(px(32.))
            .px_2()
            .rounded_t_md()
            .when(!expanded, |d| d.rounded_b_md())
            .bg(fill)
            .flex()
            .items_center()
            .gap_1p5()
            .text_color(content(if expanded { 1. } else { 0.8 }))
            .hover(|s| s.bg(fill.blend(content(0.1))).text_color(content(1.)))
            .when(drop_target, |d| {
                let mut accent = crate::ui::theme::accent_color();
                accent.a = 0.2;
                d.bg(fill.blend(accent))
            })
            .when(
                folder_id.as_ref().is_some_and(|id| {
                    self.folder_drag
                        .as_ref()
                        .is_some_and(|drag| drag.active && &drag.id == id)
                }),
                |d| d.opacity(0.4),
            )
            .child(
                div()
                    .size(px(16.))
                    .flex_shrink_0()
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .when(expanded && glyph == "folder", |d| d.invisible())
                            .group_hover(group.clone(), |s| s.invisible())
                            .child(icon(glyph, 14.).text_color(content(1.))),
                    )
                    .child(
                        div()
                            .absolute()
                            .invisible()
                            .when(expanded && glyph == "folder", |d| d.visible())
                            .group_hover(group.clone(), |s| s.visible())
                            .child(
                                icon(
                                    if expanded {
                                        "chevron-down"
                                    } else {
                                        "chevron-right"
                                    },
                                    14.,
                                )
                                .text_color(content(1.)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .line_height(px(17.875))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(content(1.))
                    .child(name),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_size(px(11.))
                    .line_height(px(16.5))
                    .text_color(content(0.45))
                    .when(!expanded && approval, |d| {
                        d.child(icon("circle-alert", 12.).text_color(rgb(0xfbbf24)))
                    })
                    .when(!expanded && !approval && busy, |d| d.children(busy_mark))
                    .when(!expanded && !approval && !busy && done, |d| {
                        d.child(icon("check", 12.).text_color(rgb(0x34d399)))
                    })
                    .child(sessions.len().to_string()),
            )
            .children(bounds.map(|slot| slot.probe().into_any_element()))
            .when_some(folder_id, |d, id| {
                d.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, _| {
                        this.skip_folder_click = false;
                        this.folder_drag = Some(FolderDrag {
                            id: id.clone(),
                            start: event.position,
                            active: false,
                            over: None,
                        });
                    }),
                )
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                if std::mem::take(&mut this.skip_folder_click) {
                    return;
                }
                this.sidebar_target = toggle_key
                    .strip_prefix("folder:")
                    .map(|id| RenameTarget::Folder(id.into()));
                window.focus(&this.sidebar_focus, cx);
                if !this.query.is_empty() {
                    return;
                }
                this.update_groups(cx, |groups| match toggle_key.as_str() {
                    "pinned" => groups.pinned_collapsed = !groups.pinned_collapsed,
                    "reminders" => groups.reminders_collapsed = !groups.reminders_collapsed,
                    folder => groups.toggle_folder(folder.trim_start_matches("folder:")),
                });
            }))
            .when(key.starts_with("folder:"), |d| {
                d.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        this.open_menu(
                            MenuTarget::Folder(menu_key.trim_start_matches("folder:").to_owned()),
                            Anchor::Point(event.position),
                            window,
                            cx,
                        );
                    }),
                )
            })
            .into_any_element()
    }

    pub(super) fn classic_group_fill(&self, entry: &Entry) -> gpui::Hsla {
        let color = match entry {
            Entry::Reminders { .. } => Some(rgb(REMINDERS_COLOR).into()),
            Entry::Folder { id, .. } => self.project_groups().folder(id).and_then(|folder| {
                folder
                    .custom_color
                    .as_deref()
                    .and_then(|color| color.strip_prefix('#'))
                    .and_then(|color| u32::from_str_radix(color, 16).ok())
                    .map(|color| rgb(color).into())
                    .or_else(|| folder.color_index.and_then(folder_color))
            }),
            _ => None,
        };
        color
            .map(|mut color| {
                color.a = 0.18;
                color
            })
            .unwrap_or_else(|| content(0.05))
    }

    pub(crate) fn classic_group_member(
        &self,
        entry: &Entry,
        id: i64,
        first: bool,
        last: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let folder = matches!(entry, Entry::Folder { .. });
        div()
            .bg(self.classic_group_fill(entry))
            .px_1()
            .when(first, |d| d.pt_1())
            .when(last && !folder, |d| d.pb_1().rounded_b_md())
            .when(!last, |d| d.pb_px())
            .child(self.classic_session_card_at_depth(id, false, true, cx))
            .into_any_element()
    }

    pub(crate) fn classic_folder_new_session(
        &self,
        id: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entry = Entry::Folder {
            id: id.to_owned(),
            sessions: Vec::new(),
        };
        let folder = id.to_owned();
        div()
            .bg(self.classic_group_fill(&entry))
            .pt_1()
            .rounded_b_md()
            .child(
                div().border_t_1().border_color(content(0.08)).p_1().child(
                    div()
                        .id(SharedString::from(format!("new-session-in-folder-{id}")))
                        .role(Role::Button)
                        .aria_label("New session")
                        .cursor_pointer()
                        .border_1()
                        .border_color(content(0.))
                        .rounded_md()
                        .px(px(10.))
                        .py(px(6.))
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_size(px(13.))
                        .line_height(px(17.875))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(content(0.45))
                        .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
                        .child(icon("plus", 12.))
                        .child("New session")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.create_in_folder(folder.clone(), cx);
                        })),
                ),
            )
            .into_any_element()
    }
}
