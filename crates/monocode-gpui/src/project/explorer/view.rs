use super::*;

impl Explorer {
    pub(super) fn folder_row(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .px_2()
            .pt_1p5()
            .pb_1()
            .flex_shrink_0()
            .child(
                div()
                    .h(px(30.))
                    .flex()
                    .items_center()
                    .gap_1()
                    .pl_2()
                    .pr_1()
                    .rounded_lg()
                    .bg(content(0.05))
                    .child(div().flex_1().min_w_0().child(self.folder_input.clone()))
                    .when(self.custom_folder, |d| {
                        d.child(
                            icon_action(
                                "explorer-folder-reset",
                                "Back to this chat's folder",
                                "x",
                                true,
                            )
                            .debug_selector(|| "explorer-folder-reset".into())
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(FolderChosen(None)))),
                        )
                    })
                    .child(div().w(px(1.)).h(px(16.)).bg(stroke()))
                    .child(
                        icon_action(
                            "explorer-folder-pick",
                            "Choose a folder…",
                            "folder-open",
                            true,
                        )
                        .debug_selector(|| "explorer-folder-pick".into())
                        .on_click(cx.listener(|this, _, _, cx| this.pick_folder(cx))),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn header_icon(
        id: &'static str,
        label: SharedString,
        path: &'static str,
        enabled: bool,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .debug_selector(move || id.into())
            .role(Role::Button)
            .aria_label(label)
            .group(id)
            .h(px(24.))
            .min_w_0()
            .flex_1()
            .self_center()
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .when(!enabled, |d| d.opacity(0.4))
            .when(enabled, |d| d.hover(|s| s.bg(content(0.05))))
            .child(icon(path, 14.).text_color(content(0.5)).when(enabled, |i| {
                i.group_hover(id, |s| s.text_color(content(1.)))
            }))
    }

    pub(super) fn line(&self, ix: usize, focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let statuses = &self.git.read(cx).statuses;
        let pad = |depth: usize| px(28. + depth as f32 * 12.);
        match self.lines[ix].clone() {
            Line::Note { depth, text } => div()
                .pl(pad(depth))
                .pr_2()
                .truncate()
                .text_size(px(12.))
                .text_color(content(0.5))
                .child(text)
                .into_any_element(),
            Line::Issue { depth, text, error } => div()
                .pl(pad(depth))
                .pr_2()
                .pb_1()
                .text_size(px(12.))
                .line_height(px(16.))
                .text_color(rgb(if error { RED_400 } else { AMBER_400 }))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .children(text.into_iter().map(|(t, bold)| {
                            div()
                                .when(bold, |d| d.font_weight(FontWeight::SEMIBOLD))
                                .child(t)
                        })),
                )
                .into_any_element(),
            Line::Name { depth, is_dir } => {
                let leaf = files::leaf_name(&self.name_input.read(cx).content);
                div()
                    .key_context("ExplorerName")
                    .on_action(cx.listener(|this, _: &CancelName, window, cx| {
                        this.finish_name(false, window, cx)
                    }))
                    .h(px(ROW))
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_1()
                    .bg(content(0.1))
                    .pl(px(8. + depth as f32 * 12.))
                    .pr_2()
                    .child(
                        div()
                            .size_4()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(is_dir, |d| {
                                d.child(icon("chevron-right", 14.).text_color(content(0.5)))
                            }),
                    )
                    .child(file_type_icon(&leaf, is_dir, false, 16.))
                    .child(
                        div()
                            .h(px(20.))
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .items_center()
                            .rounded_sm()
                            .bg(content(0.1))
                            .border_1()
                            .border_color(accent_color())
                            .px_1()
                            .text_size(px(ENTRY_FONT_SIZE))
                            .text_color(content(1.))
                            .when(self.name_busy, |d| d.opacity(0.5))
                            .child(self.name_input.clone()),
                    )
                    .into_any_element()
            }
            Line::Entry { entry, depth, open } => {
                let selected = self.selected.as_deref() == Some(entry.path.as_str());
                let cut = self
                    .clip
                    .as_ref()
                    .is_some_and(|c| c.cut && c.path == entry.path);
                let over = self.drag_over.as_deref() == Some(entry.path.as_str());
                let status = if entry.is_dir {
                    statuses.dirs.get(&entry.path)
                } else {
                    statuses.files.get(&entry.path)
                };
                let git_color: Option<Hsla> = status.and_then(|s| match s.as_str() {
                    "modified" => Some(rgb(AMBER_400).into()),
                    "added" | "untracked" => Some(rgb(DIFF_ADD).into()),
                    "deleted" => Some(rgb(DIFF_DEL).into()),
                    _ => None,
                });
                let path = entry.path.clone();
                let menu_path = entry.path.clone();
                let drop_path = entry.path.clone();
                let over_path = entry.path.clone();
                let is_dir = entry.is_dir;
                div()
                    .id(("explorer-row", ix))
                    .role(Role::TreeItem)
                    .aria_label(SharedString::from(entry.path.clone()))
                    .when(is_dir, |d| d.aria_expanded(open))
                    .aria_selected(selected)
                    .h(px(ROW))
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_1()
                    .pl(px(8. + depth as f32 * 12.))
                    .pr_2()
                    .text_size(px(ENTRY_FONT_SIZE))
                    .line_height(px(ENTRY_FONT_SIZE))
                    .text_color(content(1.))
                    .when(selected || over, |d| d.bg(selection()))
                    .when(!selected && !over, |d| d.hover(|s| s.bg(content(0.05))))
                    .when(selected && focused && self.keyboard, |d| {
                        d.border_1().border_color(accent_color().opacity(0.4))
                    })
                    .when(cut, |d| d.opacity(0.5))
                    .when(!is_dir, |d| {
                        d.on_drag(DraggedFile(PathBuf::from(&entry.path)), |file, _, _, cx| {
                            cx.new(|_| file.clone())
                        })
                    })
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.keyboard = false;
                        window.focus(&this.focus, cx);
                        this.select(path.clone(), cx);
                        if is_dir {
                            this.toggle(&path, cx);
                        } else {
                            cx.emit(OpenFile(path.clone()));
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.open_menu(
                                Target {
                                    path: menu_path.clone(),
                                    is_dir,
                                    root: false,
                                },
                                event.position,
                                window,
                                cx,
                            );
                        }),
                    )
                    .on_drag_move::<ExternalPaths>(cx.listener(
                        move |this, event: &gpui::DragMoveEvent<ExternalPaths>, _, cx| {
                            if event.bounds.contains(&event.event.position) {
                                let target = this.create_parent(Some(&over_path));
                                if this.drag_over.as_ref() != Some(&target) {
                                    this.drag_over = Some(target);
                                    cx.notify();
                                }
                            }
                        },
                    ))
                    .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                        this.drag_over = None;
                        let dest = this.create_parent(Some(&drop_path));
                        this.copy_external(paths.paths().to_vec(), dest, window, cx);
                    }))
                    .child(
                        div()
                            .size_4()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(is_dir, |d| {
                                d.child(
                                    icon(
                                        if open {
                                            "chevron-down"
                                        } else {
                                            "chevron-right"
                                        },
                                        14.,
                                    )
                                    .text_color(content(0.5)),
                                )
                            }),
                    )
                    .child(file_type_icon(&entry.name, is_dir, open, 16.))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .line_height(px(ENTRY_FONT_SIZE * 1.4))
                            .when(entry.ignored, |d| d.italic().text_color(content(0.5)))
                            .when_some(git_color.filter(|_| !entry.ignored), |d, c| d.text_color(c))
                            .child(entry.name.clone()),
                    )
                    .into_any_element()
            }
        }
    }
}

impl Render for Explorer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(search) = &self.search {
            return search
                .clone()
                .cached(StyleRefinement::default().flex_1().min_h_0().w_full())
                .into_any_element();
        }
        if !self.available() {
            return div()
                .px_3()
                .py_2()
                .text_size(px(12.))
                .text_color(content(0.5))
                .child("No project folder")
                .into_any_element();
        }
        let root_open = self.expanded.contains(&self.cwd);
        let name: SharedString = basename(&self.cwd).to_uppercase().into();
        let root_selected = self.drag_over.as_deref() == Some(self.cwd.as_str());
        let focused = self.focus.contains_focused(window, cx);
        let entity = cx.entity();
        let cwd = self.cwd.clone();
        let menu = self
            .menu
            .as_ref()
            .map(|open| menu_popover(&open.menu, Anchor::Point(open.anchor), cx).gap(0.));
        // B and B2: the toolbar stays; the tree gives way to the open view.
        let claude_view = self.claude_tab.and(self.claude.clone());
        let claude_buttons: Vec<AnyElement> = match self.claude {
            Some(_) => [
                (
                    ClaudeTab::Agents,
                    "explorer-claude-agents",
                    "Agents in this chat",
                    "bot",
                ),
                (
                    ClaudeTab::Chat,
                    "explorer-claude-chat",
                    "What Claude used in this chat",
                    "message-square",
                ),
            ]
            .into_iter()
            .map(|(tab, id, label, path)| {
                let active = self.claude_tab == Some(tab);
                Self::header_icon(id, label.into(), path, true)
                    .aria_toggled(if active {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .when(active, |d| d.bg(content(0.1)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let next = (this.claude_tab != Some(tab)).then_some(tab);
                        this.show_claude(next, cx);
                    }))
                    .into_any_element()
            })
            .collect(),
            None => Vec::new(),
        };
        div()
            .id("explorer")
            .role(Role::TabPanel)
            .aria_label("Explorer")
            .key_context("Explorer")
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .on_key_down(cx.listener(Self::on_key))
            .on_action(cx.listener(|this, _: &Down, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &Up, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &PageDown, window, cx| {
                let n = this.page(window);
                this.step(n, cx)
            }))
            .on_action(cx.listener(|this, _: &PageUp, window, cx| {
                let n = this.page(window);
                this.step(-n, cx)
            }))
            .on_action(cx.listener(|this, _: &First, _, cx| this.step(-isize::MAX / 2, cx)))
            .on_action(cx.listener(|this, _: &Last, _, cx| this.step(isize::MAX / 2, cx)))
            .on_action(cx.listener(|this, _: &Expand, _, cx| {
                let (path, is_dir) = this.current();
                if !is_dir {
                    return;
                }
                if !this.expanded.contains(&path) {
                    this.toggle(&path, cx);
                } else {
                    let rows = this.nav_rows();
                    if let Some(ix) = rows.iter().position(|(p, _)| *p == path)
                        && let Some((next, _)) = rows.get(ix + 1).cloned()
                        && parent_of(&next) == path
                    {
                        this.focus_row(next, cx);
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &Collapse, _, cx| {
                let (path, is_dir) = this.current();
                if is_dir && this.expanded.contains(&path) {
                    this.toggle(&path, cx);
                } else if path != this.cwd {
                    this.focus_row(parent_of(&path), cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Activate, _, cx| this.activate(cx)))
            .on_action(cx.listener(|this, _: &OpenSelected, _, cx| this.activate(cx)))
            .on_action(cx.listener(|this, _: &Rename, window, cx| {
                let (path, _) = this.current();
                this.start_rename(path, window, cx)
            }))
            .on_action(cx.listener(|this, _: &Delete, window, cx| {
                let (path, _) = this.current();
                this.delete(path, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ClearCut, _, cx| {
                if this.clip.as_ref().is_some_and(|c| c.cut) {
                    this.clip = None;
                    this.rebuild(cx);
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, action: &CopyEntry, _, cx| {
                let _ = action;
                let (path, is_dir) = this.current();
                if path != this.cwd {
                    this.clip = Some(Clip {
                        cut: false,
                        path,
                        is_dir,
                    });
                    this.rebuild(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CutEntry, _, cx| {
                let (path, is_dir) = this.current();
                if path != this.cwd {
                    this.clip = Some(Clip {
                        cut: true,
                        path,
                        is_dir,
                    });
                    this.rebuild(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PasteEntry, window, cx| {
                let (path, _) = this.current();
                this.paste(path, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CopyPath, _, cx| {
                let (path, _) = this.current();
                cx.write_to_clipboard(ClipboardItem::new_string(path));
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.open_menu(
                        Target {
                            path: this.cwd.clone(),
                            is_dir: true,
                            root: true,
                        },
                        event.position,
                        window,
                        cx,
                    );
                }),
            )
            .child(
                pane_header("explorer-header", "Explorer")
                    .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                    .child(
                        Self::header_icon(
                            "explorer-collapse",
                            "Collapse All".into(),
                            "fold-vertical",
                            true,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_tree(cx);
                            this.collapse_all(cx)
                        })),
                    )
                    .child(
                        Self::header_icon(
                            "explorer-search",
                            format!("Search in files ({MOD}Shift+F)").into(),
                            "search",
                            true,
                        )
                        .on_click(cx.listener(|this, _, window, cx| this.open_search(window, cx))),
                    )
                    .children(claude_buttons),
            )
            .children(claude_view)
            .when(self.claude_tab.is_none() || self.claude.is_none(), |d| {
                d.child(self.folder_row(cx))
                    .child(
                        div()
                            .h(px(32.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .id("explorer-root")
                                    .role(Role::TreeItem)
                                    .aria_label(SharedString::from(cwd.clone()))
                                    .aria_expanded(root_open)
                                    .h_full()
                                    .min_w_0()
                                    .flex_1()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .pl_2()
                                    .when(root_selected, |d| d.bg(selection()))
                                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                                        window.prevent_default()
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        window.focus(&this.focus, cx);
                                        let cwd = this.cwd.clone();
                                        this.select(cwd.clone(), cx);
                                        this.toggle(&cwd, cx);
                                    }))
                                    .on_drop(cx.listener(
                                        |this, paths: &ExternalPaths, window, cx| {
                                            this.drag_over = None;
                                            let dest = this.cwd.clone();
                                            this.copy_external(
                                                paths.paths().to_vec(),
                                                dest,
                                                window,
                                                cx,
                                            );
                                        },
                                    ))
                                    .child(
                                        div()
                                            .size_4()
                                            .flex_shrink_0()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(
                                                icon(
                                                    if root_open {
                                                        "chevron-down"
                                                    } else {
                                                        "chevron-right"
                                                    },
                                                    14.,
                                                )
                                                .text_color(content(0.5)),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(px(11.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(content(0.5))
                                            .child(name),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .id("explorer-tree")
                            .role(Role::Tree)
                            .flex_1()
                            .min_h_0()
                            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                                this.drag_over = None;
                                let dest = this.cwd.clone();
                                this.copy_external(paths.paths().to_vec(), dest, window, cx);
                            }))
                            .when(root_open, |d| {
                                d.child(
                                    list(self.list.clone(), move |ix, _, cx| {
                                        entity.update(cx, |this, cx| this.line(ix, focused, cx))
                                    })
                                    .size_full(),
                                )
                            }),
                    )
            })
            .children(menu)
            .into_any_element()
    }
}
