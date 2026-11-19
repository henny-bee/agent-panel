use super::*;

impl Notes {
    pub(super) fn project_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = self.project_query.read(cx).content.to_lowercase();
        let mut projects = self.projects.clone();
        if notes::is_project(&self.cwd) {
            projects.insert(0, self.cwd.clone());
        }
        for note in &self.notes {
            if let Some(path) = note
                .source_cwd
                .as_ref()
                .filter(|path| notes::is_project(path))
            {
                projects.push(path.clone());
            }
        }
        let mut seen = std::collections::HashSet::new();
        projects.retain(|path| seen.insert(path.clone()) && path.to_lowercase().contains(&query));
        let weak = cx.weak_entity();
        popover(
            "note-project-picker",
            Anchor::Slot(self.project_anchor.clone()),
        )
        .width(300.)
        .max_height(340.)
        .on_dismiss(move |_, cx| {
            let _ = weak.update(cx, |this, cx| {
                this.project_open = false;
                cx.notify();
            });
        })
        .child(
            div()
                .p_2()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().px_2().py_1().child(self.project_query.clone()))
                .child(
                    div()
                        .id("note-no-project")
                        .role(Role::Button)
                        .aria_label("No project")
                        .px_2()
                        .py_1p5()
                        .rounded_md()
                        .text_size(px(12.))
                        .hover(|style| style.bg(content(0.08)))
                        .child("No project")
                        .on_click(cx.listener(|this, _, _, cx| this.move_project("~".into(), cx))),
                )
                .child(
                    div()
                        .id("note-project-results")
                        .min_h_0()
                        .overflow_y_scroll()
                        .children(projects.into_iter().enumerate().map(|(ix, path)| {
                            let name = project_name(&path);
                            div()
                                .id(("note-project", ix))
                                .role(Role::Button)
                                .aria_label(path.clone())
                                .px_2()
                                .py_1p5()
                                .rounded_md()
                                .text_size(px(12.))
                                .hover(|style| style.bg(content(0.08)))
                                .child(name)
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(10.))
                                        .text_color(content(0.4))
                                        .child(path.clone()),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.move_project(path.clone(), cx)
                                }))
                        })),
                ),
        )
        .into_any_element()
    }

    pub(super) fn detail(&mut self, note: Note, cx: &mut Context<Self>) -> AnyElement {
        let body = note.body.clone();
        let source = note.source_session_id.clone();
        let can_add = !body.trim().is_empty();
        let project = div()
            .relative()
            .id("note-move-project")
            .role(Role::Button)
            .aria_label("Move note to project")
            .rounded_md()
            .px_1p5()
            .py_1()
            .hover(|style| style.bg(content(0.08)))
            .child(self.project_anchor.probe())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        note.source_cwd
                            .as_deref()
                            .map(|path| self.project_mark(path, 12.))
                            .unwrap_or_else(|| icon("folder", 12.).into_any_element()),
                    )
                    .child(note.project_name().unwrap_or_else(|| "No project".into()))
                    .child(icon("chevron-down", 11.)),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                this.project_open = !this.project_open;
                if this.project_open {
                    this.project_query
                        .update(cx, |input, cx| input.set_text(String::new(), cx));
                    window.focus(&this.project_query.focus_handle(cx), cx);
                }
                cx.notify();
            }));
        let tags = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1p5()
            .text_size(px(11.))
            .child(div().mr(px(2.)).text_color(content(0.45)).child("Tags"))
            .children(note.tags.iter().enumerate().map(|(ix, tag)| {
                let tag = tag.clone();
                div()
                    .h(px(24.))
                    .max_w(px(192.))
                    .flex()
                    .items_center()
                    .gap_1()
                    .rounded_md()
                    .bg(content(0.08))
                    .pl_2()
                    .pr_1()
                    .text_color(content(0.7))
                    .child(div().truncate().child(format!("#{tag}")))
                    .child(
                        div()
                            .id(("remove-note-tag", ix))
                            .role(Role::Button)
                            .aria_label(format!("Remove #{tag}"))
                            .size(px(16.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_sm()
                            .hover(|style| style.bg(content(0.1)))
                            .child(icon("close", 10.))
                            .on_click(cx.listener(move |this, _, _, cx| this.remove_tag(&tag, cx))),
                    )
            }))
            .when(note.tags.len() < notes::MAX_TAGS, |d| {
                d.child(div().min_w(px(80.)).flex_1().child(self.tag.clone()))
            });
        // Solid ink on the inverted fill: dark text on a light button in dark
        // mode, light text on a dark one in light mode.
        let mut ink = shell_fill();
        ink.a = 1.;
        let danger: gpui::Hsla = gpui::rgb(0xf87171).into();
        let actions = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap_2()
            .child(
                div()
                    .id("note-add-to-chat")
                    .role(Role::Button)
                    .aria_label("Add to chat")
                    .h(px(28.))
                    .px_3()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .bg(content(1.))
                    .text_color(ink)
                    .map(|d| {
                        if can_add {
                            d.cursor_pointer()
                                .hover(move |s| s.bg(content(0.85)).text_color(ink))
                        } else {
                            d.opacity(0.4)
                        }
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if can_add {
                            this.flush(cx);
                            if !this.dirty
                                && let Some(note) = this.draft(cx)
                            {
                                cx.emit(Event::AddToChat(note));
                            }
                        }
                    }))
                    .child("Add to chat"),
            )
            .child(
                div()
                    .id("delete-note")
                    .role(Role::Button)
                    .aria_label("Delete note")
                    .h(px(28.))
                    .px_3()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(danger)
                    .cursor_pointer()
                    .hover(|style| style.bg(content(0.08)))
                    .child(icon("trash", 14.).text_color(danger))
                    .child("Delete")
                    .on_click(cx.listener(|this, _, _, cx| this.delete(cx))),
            )
            .when_some(source, |d, source| {
                d.child(
                    div()
                        .id("note-source-session")
                        .role(Role::Button)
                        .aria_label("Open source conversation")
                        .rounded_md()
                        .px_2()
                        .py_1()
                        .text_size(px(12.))
                        .text_color(content(0.5))
                        .hover(|style| style.bg(content(0.08)))
                        .child("Source conversation")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.flush(cx);
                            if this.dirty {
                                return;
                            }
                            if let Some(id) = locked(&this.store)
                                .ok()
                                .and_then(|store| store.resolve_existing_session_id(&source))
                            {
                                cx.emit(Event::OpenSource(id));
                            } else {
                                this.error = Some("Source conversation no longer exists.".into());
                                cx.notify();
                            }
                        })),
                )
            })
            .child(
                icon_button("copy-note", "Copy note", "response-copy").on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(body.clone()))
                }),
            );
        let tabs = div()
            .id("note-sections")
            .role(Role::TabList)
            .aria_label("Note sections")
            .h(px(36.))
            .flex()
            .items_stretch()
            .gap_4()
            .border_b_1()
            .border_color(content(0.07))
            .children(
                [(false, "Preview"), (true, "Source")].map(|(source, label)| {
                    div()
                        .id(if source {
                            "note-source"
                        } else {
                            "note-preview"
                        })
                        .role(Role::Tab)
                        .aria_label(label)
                        .aria_selected(self.source == source)
                        .flex()
                        .items_center()
                        .text_size(px(12.))
                        .text_color(content(if self.source == source { 1. } else { 0.5 }))
                        .when(self.source == source, |d| {
                            d.border_b_2().border_color(content(1.))
                        })
                        .hover(|style| style.text_color(content(1.)))
                        .child(label)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.source = source;
                            if let Some(id) = &this.selected {
                                this.modes.insert(id.clone(), source);
                            }
                            if source {
                                window.focus(&this.body.focus_handle(cx), cx);
                            } else {
                                this.flush(cx);
                                this.update_preview(cx);
                            }
                            cx.notify();
                        }))
                }),
            );
        let editor = div()
            .id("note-image-drop-zone")
            .relative()
            .min_h(px(448.))
            .rounded_lg()
            .border_1()
            .border_color(if self.image_drag {
                accent_color()
            } else {
                content(0.)
            })
            .on_drag_move::<ExternalPaths>(cx.listener(
                |this, event: &gpui::DragMoveEvent<ExternalPaths>, _, cx| {
                    this.image_drag = event.bounds.contains(&event.event.position);
                    cx.notify();
                },
            ))
            .on_drag_move::<DraggedFile>(cx.listener(
                |this, event: &gpui::DragMoveEvent<DraggedFile>, _, cx| {
                    this.image_drag = event.bounds.contains(&event.event.position);
                    cx.notify();
                },
            ))
            .on_file_drop_exit(cx.listener(|this, _, _, cx| {
                this.image_drag = false;
                cx.notify();
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.add_images(paths.paths().to_vec(), cx)
            }))
            .on_drop(cx.listener(|this, file: &DraggedFile, _, cx| {
                this.add_images(vec![file.0.clone()], cx)
            }))
            .when(self.source, |d| {
                d.h(px((self.body.read(cx).editor_line_count() as f32 * 20.
                    + 16.)
                    .clamp(448., 2000.)))
                    .child(self.body.clone())
            })
            .when(!self.source, |d| {
                d.when_some(self.document.clone(), |d, preview| d.child(preview))
            })
            .when(self.image_drag || self.image_busy, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded_lg()
                        .bg(pane_fill().opacity(0.9))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(12.))
                        .text_color(content(0.7))
                        .child(if self.image_busy {
                            "Adding images…"
                        } else {
                            "Drop images here"
                        }),
                )
            });
        div()
            .id("note-detail")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .gap(px(20.))
                    .w_full()
                    .max_w(px(1024.))
                    .mx_auto()
                    .p(px(32.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .text_size(px(12.))
                                    .text_color(content(0.5))
                                    .child(icon("note", 14.))
                                    .child("Note")
                                    .child(div().min_w_0().truncate().child(note.slug.clone()))
                                    .child(project),
                            )
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(self.title.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(content(0.5))
                                    .child(format!("Updated {}", relative_time(note.updated_at))),
                            )
                            .child(tags)
                            .child(actions),
                    )
                    .child(tabs)
                    .child(editor),
            )
            .into_any_element()
    }
}

impl gpui::Render for Notes {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.blurs_bound {
            self.blurs_bound = true;
            self._subscriptions.push(cx.on_blur(
                &self.title.focus_handle(cx),
                window,
                |this, _, cx| this.flush(cx),
            ));
            self._subscriptions.push(cx.on_blur(
                &self.tag.focus_handle(cx),
                window,
                |this, _, cx| this.add_tag(cx),
            ));
        }
        if !cx.has_active_drag() {
            self.image_drag = false;
        }
        let query = self.query.read(cx).content.to_string();
        let visible: Vec<_> = self
            .notes
            .iter()
            .filter(|note| note.matches(&query))
            .cloned()
            .collect();
        let list = div().relative().w(px(self.list_width)).h_full().flex_shrink_0().min_h_0().flex().flex_col().border_r_1().border_color(content(0.07))
            .child(div().h(px(36.)).flex_shrink_0().px_2().flex().items_center().gap_1().border_b_1().border_color(content(0.07)).child(icon("search", 12.))
                .child(div().flex_1().min_w_0().child(self.query.clone()))
                .child(icon_button("create-note", "New note", "plus").on_click(cx.listener(|this, _, window, cx| this.create(window, cx)))))
            .child(div().id("notes-list").role(Role::ListBox).aria_label("Saved notes").flex_1().min_h_0().overflow_y_scroll().p(px(6.))
                .when(visible.is_empty(), |d| d.child(div().px_1p5().py_2().text_size(px(12.)).text_color(content(0.5)).child(if self.loading { "Loading notes…" } else if !query.trim().is_empty() { "No matching notes" } else { "No notes yet. Save a turn from the transcript, or create one here." })))
                .children(visible.iter().enumerate().map(|(ix, note)| {
                    let id = note.id.clone();
                    let active = self.selected.as_ref() == Some(&id);
                    let preview = note.preview();
                    div().id(("saved-note", ix)).role(Role::Button).aria_label(note.title.clone()).aria_selected(active).mb(px(2.)).px(px(10.)).py_2().rounded_md().text_color(content(0.8)).hover(|style| style.bg(content(0.05))).when(active, |d| d.bg(content(0.1)))
                        .child(div().flex().items_center().gap_2().text_size(px(11.)).text_color(content(0.45))
                            .child(div().flex_1().truncate().when_some(note.project_name(), |d, name| d.child(div().flex().items_center().gap_1p5().child(self.project_mark(note.source_cwd.as_deref().unwrap_or(""), 12.)).child(name))))
                            .child(relative_time(note.updated_at)))
                        .child(div().mt_1().truncate().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(content(1.)).child(note.title.clone()))
                        .when(!preview.is_empty(), |d| d.child(div().mt_1().truncate().text_size(px(12.)).text_color(content(0.45)).child(preview)))
                        .when(!note.tags.is_empty(), |d| d.child(div().mt_1p5().flex().items_center().gap_1().overflow_hidden()
                            .children(note.tags.iter().take(3).map(|tag| div().max_w(px(96.)).truncate().rounded_sm().bg(content(0.08)).px_1p5().py(px(2.)).text_size(px(10.)).text_color(content(0.55)).child(format!("#{tag}"))))
                            .when(note.tags.len() > 3, |d| d.child(div().text_size(px(10.)).text_color(content(0.4)).child(format!("+{}", note.tags.len() - 3))))))
                        .on_click(cx.listener(move |this, _, _, cx| this.select(id.clone(), cx)))
                })))
            .child(div().id("resize-notes-list").role(Role::Splitter).aria_label("Resize notes list").absolute().top_0().bottom_0().right(px(-3.)).w(px(6.)).cursor(CursorStyle::ResizeLeftRight).hover(|style| style.bg(content(0.1)))
                .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| { if event.click_count == 2 { this.list_width = 280.; } else { this.resize = Some((event.position.x.as_f32(), this.list_width)); } cx.notify(); })));
        let detail = if let Some(note) = self.draft(cx) {
            self.detail(note, cx)
        } else {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .text_size(px(13.))
                .text_color(content(0.45))
                .child(icon("note", 24.))
                .child("Select a note")
                .into_any_element()
        };
        div()
            .id("notes-view")
            .role(Role::Region)
            .aria_label("Notes")
            .size_full()
            .flex()
            .flex_col()
            .text_color(content(1.))
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                if key == "enter"
                    && this.title.focus_handle(cx).is_focused(window)
                    && !this.title.read(cx).is_composing()
                {
                    this.flush(cx);
                    window.focus(&this.query.focus_handle(cx), cx);
                    cx.stop_propagation();
                } else if key == "escape" {
                    if this.project_open {
                        this.project_open = false;
                    } else {
                        this.flush(cx);
                        if !this.dirty {
                            cx.emit(Event::Close);
                        }
                    }
                    cx.stop_propagation();
                    cx.notify();
                } else if (event.keystroke.modifiers.platform || event.keystroke.modifiers.control)
                    && key == "s"
                {
                    this.flush(cx);
                    cx.stop_propagation();
                } else if (event.keystroke.modifiers.platform || event.keystroke.modifiers.control)
                    && key == "n"
                {
                    this.create(window, cx);
                    cx.stop_propagation();
                } else if key == "backspace"
                    && this.tag.focus_handle(cx).is_focused(window)
                    && this.tag.read(cx).content.is_empty()
                    && let Some(tag) = this.draft(cx).and_then(|note| note.tags.last().cloned())
                {
                    this.remove_tag(&tag, cx);
                }
            }))
            .on_mouse_move(
                cx.listener(|this, event: &gpui::MouseMoveEvent, window, cx| {
                    if let Some((start, width)) = this.resize {
                        this.list_width = (width + event.position.x.as_f32() - start).clamp(
                            240.,
                            420_f32
                                .min(window.viewport_size().width.as_f32() * 0.5)
                                .max(240.),
                        );
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.resize = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.resize = None),
            )
            .child(div().flex_1().min_h_0().flex().child(list).child(detail))
            .when_some(self.error.clone(), |d, error| {
                d.child(
                    div()
                        .id("note-error")
                        .role(Role::Alert)
                        .flex_shrink_0()
                        .px_4()
                        .py_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.))
                        .text_color(gpui::rgb(0xf87171))
                        .child(div().flex_1().child(error))
                        .child(
                            div()
                                .id("retry-note-save")
                                .role(Role::Button)
                                .aria_label("Retry saving note")
                                .underline()
                                .child("Retry")
                                .on_click(cx.listener(|this, _, _, cx| this.flush(cx))),
                        ),
                )
            })
            .when(self.project_open, |d| d.child(self.project_picker(cx)))
    }
}
