//! Pane tree, tabs and split layout of the shell.

pub(crate) mod drag;
pub(crate) mod file_panes;
pub(crate) mod layout;
pub(crate) mod mono_panes;
pub(crate) mod pane_panels;
mod tabs;
pub(crate) mod terminal_panes;
pub(crate) mod title_tabs;
use super::*;

impl Shell {
    /// The session pane's split header (SessionPane.tsx): grip, focus dot,
    /// title and a close button. The header moves the chat independently of
    /// its review; closing a lone chat leaves the review in the tab.
    /// `shown` is the review split's progress: the header grows and fades
    /// with the slide instead of shifting the chat at once.
    pub(super) fn split_header(
        &self,
        id: i64,
        shown: f32,
        in_split: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title = self
            .sessions
            .get(&id)
            .map(|s| s.read(cx).display_title())
            .or_else(|| self.mono_pane_title(id, cx))
            .or_else(|| self.terminal_pane_title(id, cx))
            .unwrap_or_default();
        let focused = self.workspace.active == Some(id) && !self.review_focused;
        div()
            .id(("split-chat-header", id as usize))
            .debug_selector(move || format!("split-header-{id}"))
            .h(px(36. * shown))
            .opacity(shown)
            .overflow_hidden()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(6.))
            .px_2()
            .border_b_1()
            .border_color(crate::ui::panel::stroke())
            .cursor(CursorStyle::OpenHand)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.review_focused = false;
                    this.focus_pane(id, window, cx);
                    this.tab_drag = None;
                    this.pane_drag = Some(PaneDrag {
                        source: DragSource::Pane(id),
                        start: event.position,
                        position: event.position,
                        active: false,
                        over: None,
                        insert: None,
                    });
                    cx.notify();
                }),
            )
            .child(
                div()
                    .id(("drag-session-pane", id as usize))
                    .debug_selector(move || format!("drag-session-pane-{id}"))
                    .role(Role::Button)
                    .aria_label("Drag to reorder pane")
                    .w(px(20.))
                    .h(px(30.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .text_color(content(0.35))
                    .hover(|s| s.bg(content(0.05)).text_color(content(0.7)))
                    .child(icon("grip-vertical", 14.)),
            )
            .child(
                div()
                    .size(px(8.))
                    .flex_shrink_0()
                    .rounded_full()
                    .when(focused, |d| d.bg(accent_color())),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_size(px(12.))
                    .text_color(content(1.))
                    .child(title),
            )
            .children(self.pane_header_controls(id, cx))
            .children(
                [
                    (SplitDir::Right, "split-right", "Split right"),
                    (SplitDir::Down, "split-down", "Split down"),
                ]
                .map(|(dir, glyph, label)| {
                    let key = format!("split-pane-{glyph}-{id}");
                    div()
                        .id(SharedString::from(key.clone()))
                        .debug_selector(move || key.clone())
                        .role(Role::Button)
                        .aria_label(label)
                        .group(glyph)
                        .size(px(20.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(|s| s.bg(content(0.1)))
                        .child(
                            icon(glyph, 13.)
                                .text_color(content(0.5))
                                .group_hover(glyph, |s| s.text_color(content(1.))),
                        )
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.review_focused = false;
                            this.focus_pane(id, window, cx);
                            // A terminal splits off a fresh shell; a chat or a
                            // Mono, a new session with its provider.
                            if this.workspace.terminal_panes.contains_key(&id) {
                                this.split_title_terminal_toward(dir, window, cx);
                            } else {
                                this.split_session(dir, cx);
                            }
                        }))
                }),
            )
            .child(
                div()
                    .id(("close-chat-pane", id as usize))
                    .role(Role::Button)
                    .aria_label("Close pane")
                    .group("close-chat-pane")
                    .size(px(20.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| s.bg(content(0.1)))
                    .child(
                        icon("x", 12.)
                            .text_color(content(0.5))
                            .group_hover("close-chat-pane", |s| s.text_color(content(1.))),
                    )
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        if in_split {
                            this.close_pane(id, window, cx);
                        } else if let Some(review) = this.reviews.get(&id).cloned() {
                            this.review_focused = true;
                            review.update(cx, |r, cx| {
                                r.state.chat_hidden = true;
                                r.focus_active(window, cx);
                                cx.emit(crate::git::review::ReviewEvent::Changed);
                            });
                        }
                    })),
            )
            .into_any_element()
    }

    pub(super) fn split_width(&self) -> f32 {
        self.workspace
            .active
            .and_then(|id| self.split_bounds.get(&id))
            .and_then(AnchorSlot::get)
            .map_or(800., |b| b.size.width.as_f32())
    }

    /// The card's workspace: the active tab's pane tree, or its one session.
    pub(super) fn workspace_body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(view) = self.app_view_body() {
            return view;
        }
        let Some(id) = self.workspace.active else {
            return div().size_full().into_any_element();
        };
        let tab = self.workspace.tab_of(id).unwrap_or(id);
        let layout = self.workspace.layout(tab);
        if layout.is_split() || self.pane_motion.as_ref().is_some_and(|m| m.tab == tab) {
            return self.pane_tree(tab, layout.leaves(), window, cx);
        }
        let hint = self.drop_hint(id);
        div()
            .size_full()
            .relative()
            .child(self.pane_bounds.layout_probe())
            .child(self.session_surface(id, None, window, cx))
            .children(hint)
            .into_any_element()
    }

    /// PaneTree.tsx: absolutely placed panes, sashes on shared edges and a
    /// drop hint on the pane under a drag.
    pub(super) fn pane_tree(
        &mut self,
        tab: i64,
        leaves: Vec<LayoutLeaf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let size = self.pane_bounds.get().map_or((800., 600.), |b| {
            (b.size.width.as_f32(), b.size.height.as_f32())
        });
        let motion = match self.pane_motion.as_ref().filter(|m| m.tab == tab) {
            Some(motion) => {
                let (t, done) = motion.slide.at(std::time::Instant::now());
                if done {
                    self.pane_motion = None;
                    None
                } else {
                    window.request_animation_frame();
                    Some(t)
                }
            }
            None => {
                self.pane_motion = None;
                None
            }
        };
        let dragging = self
            .pane_drag
            .as_ref()
            .filter(|d| d.active && !matches!(d.source, DragSource::Review(_)))
            .map(|d| d.source.session());
        let (header, leaving) = match (motion, self.pane_motion.as_ref()) {
            (Some(_), Some(m)) => (m.header, m.leaving.clone()),
            _ => (None, Vec::new()),
        };
        let t = motion.unwrap_or(1.);
        // A sliding pane holds its full size against the edge it rides; its
        // neighbours rewrap as they give way.
        let pinned = |leaf: &LayoutLeaf, edge: Edge, surface: AnyElement| {
            div()
                .absolute()
                .w(px(leaf.rect.w * size.0))
                .h(px(leaf.rect.h * size.1))
                .map(|d| match edge {
                    Edge::Left => d.top_0().right_0(),
                    Edge::Top => d.left_0().bottom_0(),
                    Edge::Right | Edge::Bottom => d.top_0().left_0(),
                })
                .flex()
                .flex_col()
                .child(surface)
        };
        let frame = |id: i64, rect: Rect| {
            div()
                .id(("pane", id as usize))
                .debug_selector(move || format!("pane-{id}"))
                .absolute()
                .left(gpui::relative(rect.x))
                .top(gpui::relative(rect.y))
                .w(gpui::relative(rect.w))
                .h(gpui::relative(rect.h))
                .overflow_hidden()
                .flex()
                .flex_col()
                .border_color(crate::ui::panel::stroke())
                .when(rect.x > 0.001, |d| d.border_l_1())
                .when(rect.y > 0.001, |d| d.border_t_1())
        };
        let mut panes = Vec::with_capacity(leaves.len() + leaving.len());
        for leaf in &leaves {
            let id = leaf.id;
            let (rect, entering) = match (motion, self.pane_motion.as_ref()) {
                (Some(t), Some(m)) => (
                    m.from
                        .get(&id)
                        .copied()
                        .unwrap_or(leaf.rect)
                        .lerp(leaf.rect, t),
                    m.entering.get(&id).copied(),
                ),
                _ => (leaf.rect, None),
            };
            // A pane with its review keeps its header either way.
            let shown = if entering.is_some() || self.review_for(id, cx).is_some() {
                1.
            } else {
                match header {
                    Some(true) => t,
                    Some(false) => 1. - t,
                    None => 1.,
                }
            };
            // Once the last neighbour has left, render the lone-pane header
            // state on this frame too, rather than flashing a full header.
            let split = (leaves.len() > 1 || motion.is_some()).then_some(shown);
            let surface = self.session_surface(id, split, window, cx);
            let content = match entering {
                Some(edge) => pinned(leaf, edge, surface),
                None => div().size_full().flex().flex_col().child(surface),
            };
            let content = match self.pane_panel_frame(id, window, cx) {
                Some(panel) => div()
                    .size_full()
                    .flex()
                    .child(
                        div()
                            .h_full()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(content),
                    )
                    .child(panel),
                None => content,
            };
            panes.push(
                frame(id, rect)
                    .when(
                        dragging == Some(id)
                            && self
                                .workspace
                                .reviews
                                .get(&id)
                                .is_none_or(|r| r.chat_hidden),
                        |d| d.opacity(0.4),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| this.focus_pane(id, window, cx)),
                    )
                    .child(content)
                    .children(self.drop_hint(id))
                    .into_any_element(),
            );
        }
        // Leaving panes paint over the neighbours growing into their room.
        for (leaf, edge, to) in &leaving {
            let surface = self.session_surface(leaf.id, Some(1.), window, cx);
            panes.push(
                frame(leaf.id, leaf.rect.lerp(*to, t))
                    .opacity(1. - 0.5 * t)
                    .child(pinned(leaf, *edge, surface))
                    .into_any_element(),
            );
        }
        let sashes = if motion.is_some() {
            Vec::new()
        } else {
            self.workspace.layout(tab).sashes()
        };
        div()
            .id("pane-tree")
            .size_full()
            .relative()
            .child(self.pane_bounds.layout_probe())
            .children(panes)
            .children(sashes.into_iter().map(|sash| {
                let row = sash.dir == SplitDir::Right;
                let g = sash.group;
                let key = sash.path.iter().fold(sash.index, |k, ix| k * 31 + ix + 1);
                div()
                    .id(("pane-sash", key))
                    .debug_selector(move || format!("pane-sash-{key}"))
                    .group("resize-handle")
                    .role(Role::Splitter)
                    .aria_label("Resize panes")
                    .absolute()
                    .map(|d| {
                        if row {
                            d.left(gpui::relative(g.x + sash.boundary * g.w))
                                .top(gpui::relative(g.y))
                                .h(gpui::relative(g.h))
                                .ml(px(-5.5))
                                .w(px(12.))
                                .cursor(CursorStyle::ResizeLeftRight)
                        } else {
                            d.top(gpui::relative(g.y + sash.boundary * g.h))
                                .left(gpui::relative(g.x))
                                .w(gpui::relative(g.w))
                                .mt(px(-5.5))
                                .h(px(12.))
                                .cursor(CursorStyle::ResizeUpDown)
                        }
                    })
                    .map(|d| {
                        let drag = self
                            .pane_sash
                            .as_ref()
                            .filter(|drag| drag.path == sash.path && drag.index == sash.index);
                        let snapped = drag.is_some_and(|drag| drag.snapped);
                        d.child(resize_border(row, drag.is_some()))
                            .when(snapped, |d| d.child(sash_snap_line(row)))
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            // A double-click evens out the two panes.
                            if event.click_count == 2 {
                                this.pane_sash = None;
                                if let Some(middle) = this
                                    .workspace
                                    .sash_snaps(tab, &sash.path, sash.index)
                                    .first()
                                {
                                    this.workspace
                                        .place_sash(tab, &sash.path, sash.index, *middle);
                                    this.persist(cx);
                                    cx.notify();
                                }
                                return;
                            }
                            this.pane_sash = Some(SashDrag {
                                tab,
                                path: sash.path.clone(),
                                index: sash.index,
                                dir: sash.dir,
                                group: g,
                                snapped: false,
                            });
                        }),
                    )
            }))
            .into_any_element()
    }

    /// PaneDropHint: half the pane washed toward the edge it will take.
    pub(super) fn drop_hint(&self, id: i64) -> Option<AnyElement> {
        let drag = self.pane_drag.as_ref().filter(|d| d.active)?;
        let (target, edge) = drag.over?;
        if target != id {
            return None;
        }
        let wash = div().absolute().bg(accent_color()).opacity(0.15);
        let line = div().absolute().bg(accent_color());
        let (wash, line) = match edge {
            Edge::Left => (
                wash.top_0().bottom_0().left_0().w_1_2(),
                line.top_0().bottom_0().left_0().w(px(2.)),
            ),
            Edge::Right => (
                wash.top_0().bottom_0().right_0().w_1_2(),
                line.top_0().bottom_0().right_0().w(px(2.)),
            ),
            Edge::Top => (
                wash.left_0().right_0().top_0().h_1_2(),
                line.left_0().right_0().top_0().h(px(2.)),
            ),
            Edge::Bottom => (
                wash.left_0().right_0().bottom_0().h_1_2(),
                line.left_0().right_0().bottom_0().h(px(2.)),
            ),
        };
        let review_ratio = (drag.source == DragSource::Pane(id))
            .then(|| self.workspace.reviews.get(&id))
            .flatten()
            .filter(|state| !state.chat_hidden)
            .map(|state| state.ratio);
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .when_some(review_ratio, |d, ratio| {
                    d.left(gpui::relative(1. - ratio)).w(gpui::relative(ratio))
                })
                .child(wash)
                .child(line)
                .into_any_element(),
        )
    }

    /// One session's surface: its chat, with its review pane beside it.
    /// `split` is the split header's progress while this is a pane of a
    /// split tab; None for a tab's lone session.
    pub(super) fn session_surface(
        &mut self,
        id: i64,
        split: Option<f32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.workspace.mono_panes.contains_key(&id) {
            return self.mono_pane_surface(id, split, cx);
        }
        if self.workspace.terminal_panes.contains_key(&id) {
            return self.terminal_pane_surface(id, split, cx);
        }
        let in_split = split.is_some();
        let chat = self.sessions.get(&id).cloned();
        let now = std::time::Instant::now();
        let sliding = self
            .review_slide
            .as_ref()
            .filter(|(sid, _, _)| *sid == id)
            .map(|(_, slide, review)| {
                let (width, done) = slide.at(now);
                (width, done, slide.opening(), review.clone())
            });
        // A completed close must use the same surface as the next frame.
        // Keeping the removed review here would retain its collapsing header
        // and change the layout again on the following redraw.
        let sliding = match sliding {
            Some((_, true, _, _)) => {
                self.review_slide = None;
                None
            }
            sliding => sliding,
        };
        let review = sliding
            .as_ref()
            .map(|(_, _, _, review)| review.clone())
            .or_else(|| self.review_for(id, cx));
        if let Some(chat) = &chat
            && self.lays_out(id, cx)
        {
            let beside = in_split || review.is_some();
            chat.update(cx, |session, cx| session.set_beside_pane(beside, cx));
        }
        let Some(review) = review else {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .when_some(split.filter(|p| *p > 0.), |d, p| {
                    d.child(self.split_header(id, p, true, cx))
                })
                .child(self.chat_with_background(id, chat, cx))
                .into_any_element();
        };
        let state = review.read(cx).state.clone();
        let (ratio, deferred) = match sliding {
            Some((width, _, opening, _)) => {
                window.request_animation_frame();
                (width, opening)
            }
            None => (state.ratio, false),
        };
        // Mount the body once the pane has arrived; cold file reads and diff
        // construction cannot compete with its opening frames.
        review.update(cx, |r, cx| {
            r.set_visible(true, cx);
            r.set_deferred(deferred, cx);
        });
        let chat_hidden = state.chat_hidden;
        let header = if in_split {
            1.
        } else if state.ratio > 0. {
            (ratio / state.ratio).clamp(0., 1.)
        } else {
            1.
        };
        let header_shown = split.unwrap_or(header);
        let ratio = if chat_hidden { 1. } else { ratio.clamp(0., 1.) };
        let bounds = self.split_bounds.entry(id).or_default().clone();
        let full_width = bounds
            .get()
            .map_or(600., |b| b.size.width.as_f32() * state.ratio);
        let dragging = self.review_drag && self.workspace.active == Some(id);
        let moving = self
            .pane_drag
            .as_ref()
            .is_some_and(|d| d.active && d.source == DragSource::Review(id));
        let moving_chat = self
            .pane_drag
            .as_ref()
            .is_some_and(|d| d.active && d.source == DragSource::Pane(id));
        let sliding_now = self
            .review_slide
            .as_ref()
            .is_some_and(|(sid, _, _)| *sid == id);
        div()
            .size_full()
            .relative()
            .flex()
            .child(bounds.layout_probe())
            .when(!chat_hidden, |d| {
                d.child(
                    div()
                        .h_full()
                        .min_w_0()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .when(moving_chat, |d| d.opacity(0.4))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                if this.review_focused {
                                    this.review_focused = false;
                                    cx.notify();
                                }
                            }),
                        )
                        .child(self.split_header(id, header_shown, in_split, cx))
                        .child(self.chat_with_background(id, chat, cx)),
                )
            })
            .child(
                div()
                    .id(("review-frame", id as usize))
                    .debug_selector(|| "review-frame".into())
                    .relative()
                    .h_full()
                    .flex_shrink_0()
                    .w(gpui::relative(ratio))
                    .overflow_hidden()
                    .when(moving, |d| d.opacity(0.4))
                    .when(!chat_hidden, |d| {
                        d.border_l_1().border_color(crate::ui::panel::stroke())
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            if !this.review_focused {
                                this.review_focused = true;
                                cx.notify();
                            }
                        }),
                    )
                    // Ride the moving leading edge at a fixed layout width.
                    // Pinning to the outer right edge would only uncover the
                    // contents, losing the slide React gives the whole surface.
                    .map(|d| {
                        // The same progress drives both directions, so a
                        // reversal cannot flash from a faded exit to opaque.
                        let d = d.opacity(if moving { header * 0.4 } else { header });
                        let review = review.cached(StyleRefinement::default().size_full());
                        if sliding_now && !chat_hidden {
                            d.child(
                                div()
                                    .id("review-content")
                                    .debug_selector(|| "review-content".into())
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .h_full()
                                    .w(px(full_width))
                                    .child(review),
                            )
                        } else {
                            d.child(div().id("review-content").size_full().child(review))
                        }
                    }),
            )
            .when(!chat_hidden && !sliding_now, |d| {
                d.child(
                    div()
                        .id(("review-sash", id as usize))
                        .group("resize-handle")
                        .role(Role::Splitter)
                        .aria_label("Resize panes")
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(gpui::relative(1. - ratio))
                        .ml(px(-5.5))
                        .w(px(12.))
                        .cursor(CursorStyle::ResizeLeftRight)
                        .child(resize_border(true, dragging))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                this.focus_pane(id, window, cx);
                                this.review_drag = true;
                                cx.stop_propagation();
                                cx.notify();
                            }),
                        ),
                )
            })
            .into_any_element()
    }

    /// A click anywhere in a pane focuses it (PaneTree's onFocus).
    pub(super) fn focus_pane(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace.review_owners.contains_key(&id) {
            self.open_file_pane(id, window, cx);
            return;
        }
        if self.workspace.mono_panes.contains_key(&id) {
            if self.workspace.active != Some(id) {
                self.open_mono_pane(id, window, cx);
            }
            return;
        }
        if self.workspace.terminal_panes.contains_key(&id) {
            self.open_terminal_pane(id, window, cx);
            return;
        }
        if self.workspace.active == Some(id) {
            return;
        }
        self.navigation_revision += 1;
        self.history.record(id);
        self.workspace.open(id);
        if let Some(view) = self.sessions.get(&id) {
            self.workspace.project = view.read(cx).summary.cwd.clone();
            self.workspace
                .project_focus
                .insert(self.workspace.project.clone(), id);
            view.update(cx, |v, cx| v.load(cx));
            self.record_worktree_session(id, cx);
        }
        if !self.review_focused {
            self.focus_active(window, cx);
        }
        self.persist(cx);
        cx.notify();
    }

    /// Close one pane of a split tab; the tab's only pane closes the tab.
    pub(super) fn close_pane(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if !self
            .workspace
            .tab_of(id)
            .is_some_and(|tab| self.workspace.layout(tab).is_split())
        {
            self.close(id, window, cx);
            return;
        }
        self.confirm_close_terminal_panes(&[id], window, cx, move |this, window, cx| {
            this.finish_close_pane(id, window, cx);
        });
    }

    pub(super) fn finish_close_pane(
        &mut self,
        id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = self.workspace.active;
        let before = self.active_leaves();
        if !self.workspace.close_pane(id) {
            self.close(id, window, cx);
            return;
        }
        self.history
            .prune(&self.workspace.open_sessions(), self.workspace.active);
        self.navigation_revision += 1;
        self.begin_pane_motion(before, cx);
        self.fit_sidebar(cx);
        // A Mono left alone becomes its tab again, and shows when focused.
        self.settle_mono_panes(window, cx);
        if self.mono_active(cx) {
            return;
        }
        if self.workspace.active != active {
            if let Some(active) = self.workspace.active {
                self.open(active, window, cx);
            }
        } else {
            self.persist(cx);
            cx.notify();
        }
    }

    /// cmd-d / shift-cmd-d: a new session beside the focused pane, with the
    /// focused session's provider.
    pub(super) fn split_session(&mut self, dir: SplitDir, cx: &mut Context<Self>) {
        if self.app_view_open() || self.workspace.active.is_none() {
            return;
        }
        let kind = self
            .workspace
            .active
            .and_then(|id| self.sessions.get(&id))
            .map_or(Kind::Codex, |s| Kind::from_id(&s.read(cx).summary.provider));
        let defaults = sessions::loading::NewSessionDefaults::read(
            self.explicit_choice(kind),
            &self.workspace.project,
            cx,
        );
        let navigation_revision = self.navigation_revision;
        let target = self.workspace.active;
        let store = self.store.clone();
        let cwd = self.workspace.project.clone();
        let worktree = target.and_then(|id| {
            let path = self.session_working_copy(id, cx);
            (monocode_core::files::path_key(path) != monocode_core::files::path_key(&cwd)).then(
                || crate::git::worktree_switcher::WorktreeFocus {
                    path: path.into(),
                    branch: self.worktree_focus.get(&cwd).and_then(|f| f.branch.clone()),
                },
            )
        });
        let worktree_cwd = worktree.as_ref().map(|f| f.path.clone());
        let work = cx
            .background_executor()
            .spawn(async move { defaults.create(&*locked(&store)?, &cwd, worktree_cwd) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok((summary, settings)) => {
                    let id = summary.id;
                    this.add(summary, cx);
                    if let Some(view) = this.sessions.get(&id) {
                        view.update(cx, |view, cx| view.seed_new_settings(settings, cx));
                    }
                    this.seed_worktree(id, worktree.as_ref(), cx);
                    this.order.retain(|s| *s != id);
                    this.order.insert(0, id);
                    if this.navigation_revision != navigation_revision
                        || this.workspace.active != target
                    {
                        this.invalidate_chrome(cx);
                        cx.notify();
                        return;
                    }
                    let before = this.active_leaves();
                    this.workspace.split(id, dir);
                    this.begin_pane_motion(before, cx);
                    this.fit_sidebar(cx);
                    this.activate_session(id, window, cx);
                }
                Err(error) => {
                    this.error = Some(error);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn focus_neighbor(
        &mut self,
        dir: Edge,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.workspace.neighbor(dir) {
            self.review_focused = false;
            self.open(id, window, cx);
        }
    }

    pub(super) fn active_leaves(&self) -> Vec<LayoutLeaf> {
        self.workspace
            .active_tab()
            .map(|tab| self.workspace.layout(tab).leaves())
            .unwrap_or_default()
    }
}

/// paneEnterFrom/collapsedRect: a pane on an outer edge slides in from it;
/// one in the middle opens from its centre line.
pub(super) fn enter_from(leaf: &LayoutLeaf) -> (Edge, Rect) {
    let r = leaf.rect;
    let edge = 0.001;
    if leaf.axis == SplitDir::Right {
        if r.x + r.w >= 1. - edge {
            return (
                Edge::Right,
                Rect {
                    x: r.x + r.w,
                    w: 0.,
                    ..r
                },
            );
        }
        if r.x <= edge {
            return (Edge::Left, Rect { w: 0., ..r });
        }
        (
            Edge::Right,
            Rect {
                x: r.x + r.w / 2.,
                w: 0.,
                ..r
            },
        )
    } else {
        if r.y + r.h >= 1. - edge {
            return (
                Edge::Bottom,
                Rect {
                    y: r.y + r.h,
                    h: 0.,
                    ..r
                },
            );
        }
        if r.y <= edge {
            return (Edge::Top, Rect { h: 0., ..r });
        }
        (
            Edge::Bottom,
            Rect {
                y: r.y + r.h / 2.,
                h: 0.,
                ..r
            },
        )
    }
}
