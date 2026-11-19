use super::*;

impl ReviewPane {
    /// openChangesTab / openCommitTab: an existing tab takes the new focus.
    pub(crate) fn open(&mut self, tab: ReviewTab, cx: &mut Context<Self>) {
        self.remember_body_focus(cx);
        self.tab_drag = None;
        if matches!(&tab, ReviewTab::File { path, .. } if self.preview_path.as_ref() == Some(path))
        {
            self.preview_path = None;
        }
        if let Some(ix) = self.sources.iter().position(|s| s.tab.same_tab(&tab)) {
            let reload = !matches!(
                (&self.sources[ix].tab, &tab),
                (
                    ReviewTab::Changes { staged: a, .. },
                    ReviewTab::Changes { staged: b, .. }
                ) if a == b
            ) && matches!(tab, ReviewTab::Changes { .. } | ReviewTab::Session { .. });
            self.sources[ix].tab = tab;
            if reload {
                self.sources[ix].cancel();
                self.sources[ix].files = None;
                self.sources[ix].loaded.clear();
            }
            self.state.active = ix;
        } else {
            let source = self.source(tab, cx);
            self.sources.push(source);
            self.state.active = self.sources.len() - 1;
        }
        self.state.chat_hidden = self.state.chat_hidden && !self.sources.is_empty();
        self.sync_state();
        self.load_active(cx);
        cx.emit(ReviewEvent::Changed);
        cx.notify();
    }

    /// Single-clicked search matches reuse a clean preview; explicit opens pin it.
    pub(crate) fn open_file(&mut self, tab: ReviewTab, pin: bool, cx: &mut Context<Self>) {
        let ReviewTab::File { path, .. } = &tab else {
            self.open(tab, cx);
            return;
        };
        if pin {
            self.open(tab, cx);
            return;
        }
        self.remember_body_focus(cx);
        let path = path.clone();
        let existing = self.sources.iter().any(|source| source.tab.same_tab(&tab));
        let keep_preview = self.preview_path.as_ref() == Some(&path)
            && !self.sources.iter().any(|source| {
                source.tab.same_tab(&tab)
                    && source
                        .editor
                        .as_ref()
                        .is_some_and(|editor| editor.read(cx).dirty)
            });
        if !existing
            && let Some(preview) = self.preview_path.take()
            && let Some(ix) = self.sources.iter().position(
                |source| matches!(&source.tab, ReviewTab::File { path, .. } if path == &preview),
            )
            && self.pending_close.is_none()
            && !self.sources[ix]
                .editor
                .as_ref()
                .is_some_and(|editor| editor.read(cx).dirty)
        {
            let mut removed = self.sources.remove(ix);
            removed.cancel();
            if self.state.active >= ix && self.state.active > 0 {
                self.state.active -= 1;
            }
            self.hovered_tab = None;
        }
        self.open(tab, cx);
        // Selecting a permanent tab must not make it replaceable again.
        if !existing || keep_preview {
            self.preview_path = Some(path);
        }
        cx.notify();
    }

    pub(crate) fn close_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.tab_drag = None;
        self.close_prompted = false;
        self.pending_close = Some((ix, false));
        if !self.closing_editors(cx).is_empty() {
            cx.notify();
            return;
        }
        self.pending_close = None;
        self.remove_tab(ix, cx);
    }

    pub(super) fn remove_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.sources.len() {
            return;
        }
        // The last tab's files stay on screen while the pane slides away;
        // the shell drops the pane once it has gone.
        if self.sources.len() == 1 {
            self.preview_path = None;
            self.hovered_tab = None;
            cx.emit(ReviewEvent::Empty);
            return;
        }
        if ix == self.state.active {
            self.remember_body_focus(cx);
        }
        let mut removed = self.sources.remove(ix);
        removed.cancel();
        self.git
            .update(cx, |git, _| git.review_cache.remember(&removed));
        if self.state.active >= ix && self.state.active > 0 {
            self.state.active -= 1;
        }
        self.hovered_tab = None;
        self.sync_state();
        self.load_active(cx);
        cx.emit(ReviewEvent::Changed);
        cx.notify();
    }

    pub(crate) fn close_active(&mut self, cx: &mut Context<Self>) {
        self.close_tab(self.state.active, cx);
    }

    pub(super) fn close_others(&mut self, keep: usize, cx: &mut Context<Self>) {
        self.tab_drag = None;
        self.close_prompted = false;
        self.pending_close = Some((keep, true));
        if !self.closing_editors(cx).is_empty() {
            cx.notify();
            return;
        }
        self.pending_close = None;
        self.remove_others(keep, cx);
    }

    pub(super) fn remove_others(&mut self, keep: usize, cx: &mut Context<Self>) {
        if keep != self.state.active {
            self.remember_body_focus(cx);
        }
        let source = self.sources.remove(keep);
        for mut removed in self.sources.drain(..) {
            removed.cancel();
            self.git
                .update(cx, |git, _| git.review_cache.remember(&removed));
        }
        self.sources.push(source);
        self.state.active = 0;
        self.sync_state();
        cx.emit(ReviewEvent::Changed);
        cx.notify();
    }

    pub(super) fn closing_editors(&self, cx: &gpui::App) -> Vec<Entity<FileEditor>> {
        let Some((target, others)) = self.pending_close else {
            return Vec::new();
        };
        self.sources
            .iter()
            .enumerate()
            .filter(|(ix, _)| if others { *ix != target } else { *ix == target })
            .filter_map(|(_, s)| s.editor.clone())
            .filter(|e| e.read(cx).dirty || e.read(cx).saving)
            .collect()
    }

    pub(super) fn approve_close(&mut self, cx: &mut Context<Self>) {
        self.close_prompted = false;
        if let Some((ix, others)) = self.pending_close.take() {
            if others {
                self.remove_others(ix, cx);
            } else {
                self.remove_tab(ix, cx);
            }
        }
    }

    pub(super) fn prompt_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_close.is_none() || self.close_prompted {
            return;
        }
        self.close_prompted = true;
        let editors = self.closing_editors(cx);
        let names = editors
            .iter()
            .map(|e| {
                std::path::Path::new(&e.read(cx).path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>()
            .join(", ");
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            &format!("Save changes to {names}?"),
            Some("Your unsaved edits will be lost if you discard them."),
            &["Save", "Discard", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let answer = answer.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match answer {
                    Ok(0) => {
                        let editors = this.closing_editors(cx);
                        if editors.is_empty() {
                            this.approve_close(cx);
                        } else {
                            for editor in editors {
                                editor.update(cx, |e, cx| e.save(cx));
                            }
                        }
                    }
                    Ok(1) if !this.closing_editors(cx).iter().any(|e| e.read(cx).saving) => {
                        this.approve_close(cx)
                    }
                    _ => {
                        this.pending_close = None;
                        this.close_prompted = false;
                        this.focus_active(window, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn start_tab_drag(
        &mut self,
        ix: usize,
        event: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) {
        if self.pending_close.is_some() {
            return;
        }
        self.skip_tab_click = false;
        self.select(ix, cx);
        let bounds: Vec<_> = (0..self.sources.len())
            .filter_map(|ix| self.tab_scroll.bounds_for_item(ix))
            .collect();
        if bounds.len() != self.sources.len() {
            return;
        }
        self.tab_drag = Some(FileTabDrag {
            from: ix,
            to: ix,
            origin: event.position.x,
            offset: px(0.),
            bounds,
            active: false,
        });
    }

    pub(super) fn move_tab_drag(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(drag) = &mut self.tab_drag else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.tab_drag = None;
            cx.notify();
            return;
        }
        let delta = event.position.x - drag.origin;
        if !drag.active && delta.abs() < px(4.) {
            return;
        }
        drag.active = true;
        let start = drag.bounds[drag.from].left();
        drag.offset = delta.clamp(
            drag.bounds[0].left() - start,
            drag.bounds.last().unwrap().left() - start,
        );
        drag.to = drag
            .bounds
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                (a.left() - start - drag.offset)
                    .abs()
                    .partial_cmp(&(b.left() - start - drag.offset).abs())
                    .unwrap()
            })
            .map_or(drag.from, |(ix, _)| ix);
        cx.notify();
    }

    pub(super) fn finish_tab_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.tab_drag.take() else {
            return;
        };
        self.skip_tab_click = drag.active;
        if drag.active
            && drag.from != drag.to
            && drag.from < self.sources.len()
            && drag.to < self.sources.len()
        {
            let source = self.sources.remove(drag.from);
            self.sources.insert(drag.to, source);
            self.state.active = drag.to;
            self.hovered_tab = None;
            self.sync_state();
            cx.emit(ReviewEvent::Changed);
        }
        cx.notify();
    }

    pub(super) fn tab_shift(&self, ix: usize) -> Pixels {
        let Some(drag) = self.tab_drag.as_ref().filter(|d| d.active) else {
            return px(0.);
        };
        if ix == drag.from {
            drag.offset
        } else if drag.from < ix && ix <= drag.to {
            drag.bounds[ix - 1].left() - drag.bounds[ix].left()
        } else if drag.to <= ix && ix < drag.from {
            drag.bounds[ix + 1].left() - drag.bounds[ix].left()
        } else {
            px(0.)
        }
    }

    pub(super) fn open_tab_menu(
        &mut self,
        ix: usize,
        at: Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = vec![
            MenuAction::new("close", "Close").into(),
            MenuAction::new("close-others", "Close Others")
                .disabled(self.sources.len() < 2)
                .into(),
        ];
        let menu = cx.new(|cx| Menu::new("File tab actions", items, window, cx));
        let events = cx.subscribe(&menu, move |this, _, event: &MenuEvent, cx| {
            this.menu = None;
            if let MenuEvent::Picked(id, _) = event {
                match id.as_ref() {
                    "close" => this.close_tab(ix, cx),
                    "close-others" => this.close_others(ix, cx),
                    _ => {}
                }
            }
            cx.notify();
        });
        self.menu = Some((ix, at, menu, events));
        cx.notify();
    }

    /// SurfaceTabs: `h-9` strip of `w-56` tabs with a close button each.
    pub(super) fn tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let animate = cx
            .try_global::<crate::ui::preferences::Preferences>()
            .is_some_and(|p| p.tab_animations)
            && !cx.reduce_motion()
            && !self.tab_drag.as_ref().is_some_and(|d| d.active);
        let keyed = self
            .sources
            .iter()
            .map(|s| (format!("{:?}", s.tab), s.tab.clone()))
            .collect();
        let (slots, moving) = self
            .tab_motion
            .sync(keyed, animate, std::time::Instant::now());
        if moving {
            window.request_animation_frame();
        }
        let mut live: Vec<Option<AnyElement>> =
            self.tab_elements(cx).into_iter().map(Some).collect();
        let keys: Vec<String> = self
            .sources
            .iter()
            .map(|s| format!("{:?}", s.tab))
            .collect();
        // TabWidthMotion: `w-56 min-w-28` eases in from nothing, or out to it.
        let slot = |open: f32| {
            div()
                .h_full()
                .w(px(224. * open))
                .min_w_0()
                .flex_shrink(1.)
                .mr(px(-crate::ui::tabs::motion::GAP * (1. - open)))
                .overflow_hidden()
                .flex()
        };
        let elements: Vec<AnyElement> = slots
            .into_iter()
            .filter_map(|s| match s {
                crate::ui::tabs::motion::Slot::Live { key, open } => {
                    let ix = keys.iter().position(|k| *k == key)?;
                    let element = live.get_mut(ix)?.take()?;
                    Some(if open >= 1. {
                        element
                    } else {
                        slot(open).child(element).into_any_element()
                    })
                }
                crate::ui::tabs::motion::Slot::Closing { ghost, open, .. } => {
                    let label: SharedString = ghost.label().into();
                    Some(
                        slot(open)
                            .child(
                                div()
                                    .w(px(224.))
                                    .h_full()
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .child(
                                        div()
                                            .h(px(TAB_PLATE_HEIGHT))
                                            .w_full()
                                            .flex()
                                            .items_center()
                                            .gap(px(crate::ui::tabs::tile::TAB_TILE_GAP))
                                            .pl(px(crate::ui::theme::tab_tile_inset()))
                                            .pr_2()
                                            .rounded_md()
                                            .text_size(px(13.))
                                            .text_color(content(0.5))
                                            .child(crate::ui::tabs::tile::file_tile(
                                                matches!(ghost, ReviewTab::File { .. })
                                                    .then_some(label.as_ref()),
                                            ))
                                            .child(div().min_w_0().truncate().child(label)),
                                    ),
                            )
                            .into_any_element(),
                    )
                }
            })
            .collect();
        div()
            .h(px(PANE_HEADER_HEIGHT))
            .debug_selector(|| "review-tab-strip".into())
            .flex_shrink_0()
            .min_w_0()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(stroke())
            .when(self.pane_draggable, |d| {
                d.child(
                    div()
                        .id("drag-file-pane")
                        .debug_selector(|| "drag-file-pane".into())
                        .role(Role::Button)
                        .aria_label("Drag to reorder pane")
                        .w(px(20.))
                        .h(px(TAB_PLATE_HEIGHT))
                        .ml(px(6.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_md()
                        .cursor(CursorStyle::OpenHand)
                        .text_color(content(0.35))
                        .hover(|s| s.bg(content(0.05)).text_color(content(0.7)))
                        .child(icon("grip-vertical", 14.))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|_, event: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                cx.emit(ReviewEvent::DragPane(event.position));
                            }),
                        ),
                )
            })
            .child(
                div()
                    .id("review-tabs")
                    .role(Role::TabList)
                    .aria_label("Files")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .pl(px(if self.pane_draggable { 2. } else { 6. }))
                    .pr(px(10.))
                    // Every tab fits, sharing the strip however many there are.
                    .overflow_hidden()
                    .track_scroll(&self.tab_scroll)
                    .children(elements),
            )
    }

    pub(super) fn tab_elements(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        self.sources
            .iter()
            .enumerate()
            .map(|(ix, source)| {
                let active = ix == self.state.active;
                let hovered = self.hovered_tab == Some(ix);
                let preview = matches!(&source.tab, ReviewTab::File { path, .. } if self.preview_path.as_ref() == Some(path));
                let label: SharedString = source.tab.label().into();
                div()
                    .id(("review-tab", ix))
                    .relative()
                    .left(self.tab_shift(ix))
                    .debug_selector(move || format!("file-tab-{ix}"))
                    .h_full()
                    .w(px(224.))
                    .min_w_0()
                    .flex_shrink(1.)
                    .flex()
                    .items_center()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event, window, cx| {
                            this.start_tab_drag(ix, event, cx);
                            this.focus_active(window, cx);
                        }),
                    )
                    .on_hover(cx.listener(move |this, hovering: &bool, _, cx| {
                        let next = if *hovering {
                            Some(ix)
                        } else if this.hovered_tab == Some(ix) {
                            None
                        } else {
                            this.hovered_tab
                        };
                        if next != this.hovered_tab {
                            this.hovered_tab = next;
                            cx.notify();
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _, _, cx| this.close_tab(ix, cx)),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            this.select(ix, cx);
                            this.open_tab_menu(ix, event.position, window, cx);
                        }),
                    )
                    .child(
                        div()
                            .id(("review-tab-button", ix))
                            .role(Role::Tab)
                            .aria_label(label.clone())
                            .aria_selected(active)
                            .relative()
                            .h(px(TAB_PLATE_HEIGHT))
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .items_center()
                            .gap(px(crate::ui::tabs::tile::TAB_TILE_GAP))
                            .pl(px(crate::ui::theme::tab_tile_inset()))
                            .pr(px(28.))
                            .rounded_md()
                            .text_size(px(13.))
                            .map(|d| {
                                if active {
                                    d.bg(selection()).text_color(content(1.))
                                } else if hovered {
                                    d.bg(content(0.05)).text_color(content(1.))
                                } else {
                                    d.text_color(content(0.5))
                                }
                            })
                            .child(crate::ui::tabs::tile::file_tile(
                                matches!(source.tab, ReviewTab::File { .. })
                                    .then_some(label.as_ref()),
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .when(preview, |d| d.italic())
                                    .child(label.clone()),
                            )
                            .when(
                                source.editor.as_ref().is_some_and(|e| e.read(cx).dirty),
                                |d| d.child(div().size(px(6.)).rounded_full().bg(content(0.7))),
                            )
                            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                                if std::mem::take(&mut this.skip_tab_click) {
                                    return;
                                }
                                if preview && event.click_count() == 2 {
                                    this.preview_path = None;
                                    cx.notify();
                                }
                                this.select(ix, cx);
                                this.focus_active(window, cx);
                            })),
                    )
                    .child(
                        div()
                            .id(("review-tab-close", ix))
                            .role(Role::Button)
                            .aria_label(SharedString::from(format!("Close {label}")))
                            .absolute()
                            .right_1()
                            .top(gpui::relative(0.5))
                            .mt(px(-10.))
                            .size(px(20.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .when(!(active || hovered), |d| d.opacity(0.))
                            .hover(|s| s.bg(content(0.1)))
                            .child(icon("x", 12.))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.close_tab(ix, cx)
                            })),
                    )
                    .into_any_element()
            })
            .collect()
    }
}
