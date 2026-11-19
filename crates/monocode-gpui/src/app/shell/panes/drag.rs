use super::super::*;

pub(in crate::app::shell) struct TabDrag {
    pub(in crate::app::shell) id: TitleTab,
    pub(in crate::app::shell) from: usize,
    pub(in crate::app::shell) to: usize,
    pub(in crate::app::shell) origin: Pixels,
    pub(in crate::app::shell) offset: Pixels,
    pub(in crate::app::shell) active: bool,
    pub(in crate::app::shell) bounds: Vec<Bounds<Pixels>>,
    /// Pulled far enough below the strip to drop on a pane.
    pub(in crate::app::shell) armed: bool,
}

/// PaneTree.tsx's pane drag and the external drops from TitleBar/Sidebar.
pub(in crate::app::shell) struct PaneDrag {
    pub(in crate::app::shell) source: DragSource,
    pub(in crate::app::shell) start: Point<Pixels>,
    pub(in crate::app::shell) position: Point<Pixels>,
    pub(in crate::app::shell) active: bool,
    pub(in crate::app::shell) over: Option<(i64, Edge)>,
    /// A pane pulled onto the title strip becomes a tab at this index.
    pub(in crate::app::shell) insert: Option<usize>,
}

pub(in crate::app::shell) struct SashDrag {
    pub(in crate::app::shell) tab: i64,
    pub(in crate::app::shell) path: Vec<usize>,
    pub(in crate::app::shell) index: usize,
    pub(in crate::app::shell) dir: SplitDir,
    pub(in crate::app::shell) group: Rect,
    /// Resting on a snap spot: the sash lights up in the accent.
    pub(in crate::app::shell) snapped: bool,
}

/// Something the user resizes by dragging its edge.
#[derive(Clone, PartialEq)]
pub(in crate::app::shell) enum ResizeTarget {
    Sidebar,
    Projects,
    RightPanel,
    Dock(String),
    /// A pane sash, with its group's length in pixels.
    Sash {
        tab: i64,
        path: Vec<usize>,
        index: usize,
        span: f32,
    },
}

/// A pane split into the tree pushes in from its edge while its neighbours
/// follow; a closed pane's neighbours grow back over its room.
pub(in crate::app::shell) struct PaneMotion {
    pub(in crate::app::shell) tab: i64,
    pub(in crate::app::shell) from: HashMap<i64, Rect>,
    pub(in crate::app::shell) entering: HashMap<i64, Edge>,
    /// Closed panes riding their edge out: the old leaf, its edge and the
    /// collapsed rect it ends at.
    pub(in crate::app::shell) leaving: Vec<(LayoutLeaf, Edge, Rect)>,
    /// The split header grows as a lone pane gains a neighbour, and shrinks
    /// as the split closes back to one pane.
    pub(in crate::app::shell) header: Option<bool>,
    pub(in crate::app::shell) slide: PaneSlide,
}

impl Shell {
    /// Animate the active tab from `before` to its current tree. New panes
    /// enter from the outer edge they sit on, or open from their middle.
    pub(in crate::app::shell) fn begin_pane_motion(
        &mut self,
        before: Vec<LayoutLeaf>,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.workspace.active_tab() else {
            return;
        };
        let after = self.workspace.layout(tab).leaves();
        if cx.reduce_motion() || (after.len() < 2 && before.len() < 2) || before.is_empty() {
            self.pane_motion = None;
            return;
        }
        let (width, height) = self.pane_bounds.get().map_or((800., 600.), |b| {
            (b.size.width.as_f32(), b.size.height.as_f32())
        });
        let mut from = HashMap::new();
        let mut entering = HashMap::new();
        let mut travel: f32 = 0.;
        for leaf in &after {
            let start = match before.iter().find(|b| b.id == leaf.id) {
                Some(old) => old.rect,
                None => {
                    let (edge, start) = enter_from(leaf);
                    entering.insert(leaf.id, edge);
                    start
                }
            };
            travel = travel.max(
                ((start.w - leaf.rect.w).abs() * width)
                    .max((start.h - leaf.rect.h).abs() * height)
                    .max((start.x - leaf.rect.x).abs() * width)
                    .max((start.y - leaf.rect.y).abs() * height),
            );
            from.insert(leaf.id, start);
        }
        // A closed pane slides back out the way a new one comes in. One that
        // moved to another tab is shown there instead.
        let open = self.workspace.open_sessions();
        let leaving: Vec<_> = before
            .iter()
            .filter(|old| {
                !after.iter().any(|leaf| leaf.id == old.id)
                    && !open.contains(&old.id)
                    && self.sessions.contains_key(&old.id)
            })
            .map(|old| {
                let (edge, to) = enter_from(old);
                travel = travel.max((old.rect.w * width).max(old.rect.h * height) * 0.5);
                (*old, edge, to)
            })
            .collect();
        if travel < 1. {
            self.pane_motion = None;
            return;
        }
        let header = match (before.len() > 1, after.len() > 1) {
            (false, true) => Some(true),
            (true, false) => Some(false),
            _ => None,
        };
        self.pane_motion = Some(PaneMotion {
            tab,
            from,
            entering,
            leaving,
            header,
            slide: PaneSlide::new(0., 1., travel),
        });
    }

    /// The pane under a point and the edge it would take, in the active tab.
    pub(in crate::app::shell) fn pane_hit(
        &self,
        position: Point<Pixels>,
        cx: &App,
    ) -> Option<(i64, Edge)> {
        if self.app_view_open() {
            return None;
        }
        // The visible Mono hides the tab's panes and stands in for them.
        if self.mono_active(cx) {
            return self.mono_view_hit(position, cx);
        }
        let bounds = self.pane_bounds.get()?;
        if !bounds.contains(&position) {
            return None;
        }
        let tab = self.workspace.active_tab()?;
        let (w, h) = (bounds.size.width.as_f32(), bounds.size.height.as_f32());
        let (x, y) = (
            (position.x - bounds.left()).as_f32(),
            (position.y - bounds.top()).as_f32(),
        );
        self.workspace
            .layout(tab)
            .leaves()
            .into_iter()
            .find_map(|leaf| {
                let r = Rect {
                    x: leaf.rect.x * w,
                    y: leaf.rect.y * h,
                    w: leaf.rect.w * w,
                    h: leaf.rect.h * h,
                };
                (x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h).then(|| {
                    (
                        leaf.id,
                        crate::runtime::pane_layout::edge_from_point(x, y, r),
                    )
                })
            })
    }

    /// titleTabDropFromPoint: the strip slot nearest the pointer.
    pub(in crate::app::shell) fn tab_insert_at(
        &self,
        position: Point<Pixels>,
        cx: &App,
    ) -> Option<usize> {
        let strip = self.tab_scroll.bounds();
        if !strip.contains(&position) {
            return None;
        }
        let count = self.strip_tabs(cx).len();
        let bounds: Vec<_> = (0..count)
            .filter_map(|ix| self.tab_scroll.bounds_for_item(ix))
            .collect();
        if bounds.len() != count {
            return Some(count);
        }
        Some(
            bounds
                .iter()
                .position(|b| position.x < b.center().x)
                .unwrap_or(count),
        )
    }

    pub(in crate::app::shell) fn move_pane_drag(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.pane_drag.as_ref() else {
            return;
        };
        if matches!(drag.source, DragSource::Tab(_)) {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.cancel_pane_drag(cx);
            return;
        }
        let source = drag.source;
        let start = drag.start;
        let delta = event.position - start;
        if !drag.active && delta.x.as_f32().hypot(delta.y.as_f32()) < 5. {
            return;
        }
        let insert = match source {
            DragSource::Pane(_) | DragSource::Review(_) | DragSource::DockTerminal(_) => {
                self.tab_insert_at(event.position, cx)
            }
            _ => None,
        };
        let over = if insert.is_some() {
            None
        } else {
            self.pane_hit(event.position, cx)
                .and_then(|(id, edge)| {
                    if id != source.session()
                        || matches!(source, DragSource::Review(id) if self.workspace.reviews.get(&id).is_some_and(|r| !r.chat_hidden))
                    {
                        Some((id, edge))
                    } else if matches!(source, DragSource::Pane(_)) {
                        self.session_review_hit(id, event.position).map(|edge| (id, edge))
                    } else {
                        None
                    }
                })
        };
        let drag = self.pane_drag.as_mut().unwrap();
        if !drag.active && matches!(source, DragSource::Session(_)) {
            self.skip_session_click = true;
        }
        drag.active = true;
        drag.position = event.position;
        drag.over = over;
        if drag.insert != insert {
            drag.insert = insert;
            self.tabs_frame.update(cx, |_, cx| cx.notify());
        }
        let drop = matches!(source, DragSource::Session(_))
            .then(|| self.sidebar_drop_at(event.position, source.session(), cx))
            .flatten();
        if self.sidebar_drop != drop {
            self.sidebar_drop = drop;
            self.notify_sidebar(cx);
        }
        cx.notify();
    }

    pub(in crate::app::shell) fn cancel_pane_drag(&mut self, cx: &mut Context<Self>) {
        if self.sidebar_drop.take().is_some() {
            self.notify_sidebar(cx);
        }
        if let Some(drag) = self.pane_drag.take() {
            if drag.insert.is_some() {
                self.tabs_frame.update(cx, |_, cx| cx.notify());
            }
            cx.notify();
        }
    }

    pub(in crate::app::shell) fn finish_pane_drag(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.pane_drag.take() else {
            return;
        };
        self.tabs_frame.update(cx, |_, cx| cx.notify());
        cx.notify();
        if !drag.active {
            return;
        }
        if let Some(target) = self.sidebar_drop.take() {
            self.apply_sidebar_drop(drag.source.session(), target, window, cx);
            return;
        }
        let mono = match drag.source {
            DragSource::Pane(pane) => self.workspace.mono_panes.get(&pane).cloned(),
            _ => None,
        };
        if let DragSource::DockTerminal(ix) = drag.source {
            self.drop_dock_terminal(ix, drag.over, drag.insert, window, cx);
            return;
        }
        let mut over = drag
            .over
            .map(|(target, edge)| (self.resolve_mono_target(target, cx), edge));
        let mut before = self.active_leaves();
        let id = drag.source.session();
        if matches!(drag.source, DragSource::Pane(_))
            && (over.is_some() || drag.insert.is_some())
            && let Some(review) = self.separate_session_review(id, cx)
        {
            // The file keeps the old pane's place while the chat moves.
            for leaf in &mut before {
                if leaf.id == id {
                    leaf.id = review;
                }
            }
            if let Some((target, _)) = &mut over
                && *target == id
            {
                *target = review;
            }
        }
        match (drag.source, over, drag.insert) {
            (DragSource::Review(_), over, insert) if over.is_some() || insert.is_some() => {
                let Some(id) = self.independent_file_pane(id, cx) else {
                    return;
                };
                if let Some(to) = insert {
                    self.workspace.detach(id, self.workspace.tabs.len());
                    self.reorder_title_tab(&TitleTab::Session(id), to, cx);
                    self.pane_motion = None;
                } else if let Some((target, edge)) = over {
                    self.workspace.place(id, target, edge);
                    self.begin_pane_motion(before, cx);
                }
            }
            (DragSource::Pane(_), _, Some(to)) => {
                let tabs = self.strip_tabs(cx);
                let session_index = tabs
                    .get(to)
                    .and_then(TitleTab::session)
                    .and_then(|next| self.workspace.tabs.iter().position(|id| *id == next))
                    .unwrap_or(self.workspace.tabs.len());
                self.workspace.detach(id, session_index);
                self.reorder_title_tab(&TitleTab::Session(id), to, cx);
                self.pane_motion = None;
            }
            (DragSource::Tab(tab), Some((target, edge)), _) => {
                self.workspace.place_tab(tab, target, edge);
                self.begin_pane_motion(before, cx);
            }
            (DragSource::Pane(_) | DragSource::Session(_), Some((target, edge)), _) => {
                self.workspace.place(id, target, edge);
                self.begin_pane_motion(before, cx);
            }
            _ => return,
        }
        self.history
            .prune(&self.workspace.open_sessions(), self.workspace.active);
        self.fit_sidebar(cx);
        // A Mono pulled out of its split onto the strip is its tab again.
        self.settle_mono_panes(window, cx);
        if let (Some(mono), Some(to)) = (mono, drag.insert)
            && self.workspace.title_order.contains(&mono)
        {
            self.reorder_title_tab(&mono, to, cx);
        }
        if self.mono_active(cx) {
            self.persist(cx);
            return;
        }
        if let Some(active) = self.workspace.active {
            self.open(active, window, cx);
        }
    }

    pub(in crate::app::shell) fn drag_sash(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(sash) = self.pane_sash.as_ref() else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.release_resizes(cx);
            return;
        }
        let Some(target) = self.sash_target() else {
            return;
        };
        let Some(bounds) = self.pane_bounds.get() else {
            return;
        };
        let g = sash.group;
        let (at, origin) = if sash.dir == SplitDir::Right {
            let origin = bounds.left().as_f32() + g.x * bounds.size.width.as_f32();
            (event.position.x.as_f32(), origin)
        } else {
            let origin = bounds.top().as_f32() + g.y * bounds.size.height.as_f32();
            (event.position.y.as_f32(), origin)
        };
        let ResizeTarget::Sash {
            tab,
            ref path,
            index,
            span,
        } = target
        else {
            return;
        };
        let span = span.max(1.);
        let mut value = (at - origin) / span;
        // Near an even split the sash settles on it.
        let snap = self
            .workspace
            .sash_snaps(tab, path, index)
            .into_iter()
            .map(|spot| (spot, (spot - value).abs() * span))
            .filter(|(_, px)| *px <= SASH_SNAP_PX)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(spot, _)| spot);
        if let Some(spot) = snap {
            value = spot;
        }
        if let Some(sash) = self.pane_sash.as_mut() {
            sash.snapped = snap.is_some();
        }
        self.drag_resize(target, value, cx);
    }

    pub(in crate::app::shell) fn sash_target(&self) -> Option<ResizeTarget> {
        let sash = self.pane_sash.as_ref()?;
        let bounds = self.pane_bounds.get()?;
        let span = if sash.dir == SplitDir::Right {
            sash.group.w * bounds.size.width.as_f32()
        } else {
            sash.group.h * bounds.size.height.as_f32()
        };
        Some(ResizeTarget::Sash {
            tab: sash.tab,
            path: sash.path.clone(),
            index: sash.index,
            span,
        })
    }

    /// The target's current size, the range it rests in, and pixels per unit.
    pub(in crate::app::shell) fn resize_extent(
        &self,
        target: &ResizeTarget,
    ) -> Option<(f32, f32, f32, f32)> {
        Some(match target {
            ResizeTarget::Projects => (self.workspace.project_rail_width, 176., 360., 1.),
            ResizeTarget::Sidebar => {
                let max = (self.viewport_width * 0.5).clamp(200., 560.);
                (self.workspace.sidebar_width, 200., max, 1.)
            }
            ResizeTarget::RightPanel => {
                let (min, max) = (self.right_panel_min, crate::ui::panel::WIDTH_MAX);
                let max = max.min((self.viewport_width * 0.5).floor()).max(min);
                (self.right_panel_width(), min, max, 1.)
            }
            ResizeTarget::Dock(project) => {
                let dock = self.workspace.docks.get(project)?;
                let (min, max) = dock.side.limits(self.viewport());
                (dock.size, min, max, 1.)
            }
            ResizeTarget::Sash {
                tab,
                path,
                index,
                span,
            } => {
                let (at, min, max) = self.workspace.sash_range(*tab, path, *index)?;
                (at, min, max, span.max(1.))
            }
        })
    }

    pub(in crate::app::shell) fn set_resize(&mut self, target: &ResizeTarget, value: f32) {
        match target {
            ResizeTarget::Projects => self.workspace.project_rail_width = value,
            ResizeTarget::Sidebar => self.workspace.sidebar_width = value,
            ResizeTarget::RightPanel => self.workspace.right_panel_width = value,
            ResizeTarget::Dock(project) => {
                if let Some(dock) = self.workspace.docks.get_mut(project) {
                    dock.size = value;
                }
            }
            ResizeTarget::Sash {
                tab, path, index, ..
            } => self.workspace.place_sash(*tab, path, *index, value),
        }
    }

    /// Follows the pointer to `value`, stopping at the target's limits.
    pub(in crate::app::shell) fn drag_resize(
        &mut self,
        target: ResizeTarget,
        value: f32,
        cx: &mut Context<Self>,
    ) {
        let Some((_, min, max, _)) = self.resize_extent(&target) else {
            return;
        };
        let clamped = value.clamp(min, max);
        let to = if matches!(target, ResizeTarget::Sash { .. }) {
            clamped
        } else {
            clamped.round()
        };
        self.set_resize(&target, to);
        cx.notify();
    }

    /// Ends every resize in progress.
    pub(in crate::app::shell) fn release_resizes(&mut self, cx: &mut Context<Self>) -> bool {
        let any = [
            self.pane_sash.take().is_some(),
            self.pane_panel_resize.take().is_some(),
            self.resize.take().is_some(),
            self.project_resize.take().is_some(),
            self.right_resize.take().is_some(),
            self.dock_resize.take().is_some(),
        ]
        .contains(&true);
        if any {
            self.persist(cx);
            cx.notify();
        }
        any
    }

    /// The dragged session's title, following the pointer.
    pub(in crate::app::shell) fn drag_ghost(&self, cx: &App) -> Option<AnyElement> {
        let drag = self.pane_drag.as_ref().filter(|d| d.active)?;
        if let Some(title) = self.dragged_terminal_title(drag.source, cx) {
            return Some(terminal_drag_ghost(drag.position, title));
        }
        let id = drag.source.session();
        let review = (matches!(drag.source, DragSource::Review(_))
            || self.workspace.review_owners.contains_key(&id))
        .then(|| self.workspace.reviews.get(&id))
        .flatten()
        .and_then(|r| r.tabs.get(r.active));
        let owner = self.workspace.session_for_pane(id);
        self.session_summaries.get(&owner)?;
        let session = self.session_metadata(owner, cx);
        let title = if let Some(tab) = review {
            tab.label()
        } else if self.workspace.review_owners.contains_key(&id) {
            self.tab_copy(id, cx).headline
        } else {
            session.display_title()
        };
        Some(
            div()
                .absolute()
                .left(drag.position.x + px(14.))
                .top(drag.position.y + px(10.))
                .max_w(px(240.))
                .h(px(28.))
                .px_2()
                .flex()
                .items_center()
                .gap(px(6.))
                .rounded_md()
                .border_1()
                .border_color(content(0.1))
                .bg(pane_fill())
                .shadow_md()
                .text_size(px(12.))
                .text_color(content(1.))
                .child(if let Some(tab) = review {
                    if matches!(tab, crate::git::review::ReviewTab::File { .. }) {
                        crate::ui::file_icons::file_type_icon(&tab.label(), false, false, 14.)
                    } else {
                        icon("git-compare", 14.).into_any_element()
                    }
                } else {
                    provider_icon(&session.summary.provider, 14.).into_any_element()
                })
                .child(div().min_w_0().truncate().child(title))
                .into_any_element(),
        )
    }

    pub(in crate::app::shell) fn start_tab_drag(
        &mut self,
        id: TitleTab,
        event: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) {
        self.skip_tab_click = false;
        self.close_menu(cx);
        let tabs = self.strip_tabs(cx);
        let Some(from) = tabs.iter().position(|tab| *tab == id) else {
            return;
        };
        if tabs.len() < 2 {
            return;
        }
        let bounds: Vec<_> = (0..tabs.len())
            .filter_map(|ix| self.tab_scroll.bounds_for_item(ix))
            .collect();
        if bounds.len() != tabs.len() {
            return;
        }
        self.tab_drag = Some(TabDrag {
            id,
            from,
            to: from,
            origin: event.position.x,
            offset: px(0.),
            active: false,
            bounds,
            armed: false,
        });
        self.tabs_frame.update(cx, |_, cx| cx.notify());
    }

    pub(in crate::app::shell) fn move_tab_drag(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.tab_drag.as_mut() else {
            return;
        };
        // A press whose release never reached the shell (the title bar can
        // swallow it) must not wake up on the next drag elsewhere.
        if event.pressed_button != Some(MouseButton::Left) {
            self.tab_drag = None;
            self.cancel_pane_drag(cx);
            self.tabs_frame.update(cx, |_, cx| cx.notify());
            return;
        }
        let delta = event.position.x - drag.origin;
        let pulled = event.position.y.as_f32() - TITLE_BAR_HEIGHT;
        if delta.abs() < px(4.) && !drag.active && pulled < 4. {
            return;
        }
        drag.active = true;
        self.skip_tab_click = true;
        // Once armed it stays armed until the pointer is back over the tabs.
        if pulled <= 0. {
            drag.armed = false;
        } else if pulled >= PANE_DROP_PULL {
            drag.armed = true;
        }
        if drag.armed {
            // Pulled out of the strip: the tab settles back into its slot
            // and the strip holds still while the drag targets a pane.
            drag.offset = px(0.);
            drag.to = drag.from;
        } else {
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
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(ix, _)| ix)
                .unwrap_or(drag.from);
        }
        let (id, armed) = (drag.id.clone(), drag.armed);
        // The active tab has no other pane to land on, but its title still
        // follows the pointer.
        let own = self.active_title_tab(cx).as_ref() == Some(&id);
        let source = match &id {
            TitleTab::Session(tab) => Some(DragSource::Tab(*tab)),
            TitleTab::Mono { .. } => Some(DragSource::MonoTab),
            TitleTab::Search | TitleTab::App(_) => None,
        };
        let was = self.pane_drag.is_some();
        self.pane_drag = source.filter(|_| armed).map(|source| PaneDrag {
            source,
            start: event.position,
            position: event.position,
            active: true,
            over: if own {
                None
            } else {
                self.pane_hit(event.position, cx)
            },
            insert: None,
        });
        if armed || was {
            cx.notify();
        }
        self.tabs_frame.update(cx, |_, cx| cx.notify());
    }

    pub(in crate::app::shell) fn finish_tab_drag(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(drag) = self.tab_drag.take() {
            let onto_pane = self.pane_drag.as_ref().is_some_and(|d| {
                d.over.is_some()
                    && match d.source {
                        DragSource::Tab(tab) => drag.id.session() == Some(tab),
                        DragSource::MonoTab => matches!(drag.id, TitleTab::Mono { .. }),
                        _ => false,
                    }
            });
            if onto_pane {
                self.skip_tab_click = true;
                if matches!(drag.id, TitleTab::Mono { .. }) {
                    self.drop_mono_tab(drag.id, window, cx);
                } else {
                    self.finish_pane_drag(window, cx);
                }
            } else if drag.active {
                self.cancel_pane_drag(cx);
                // Each tab glides from where it is drawn into its new slot.
                let now = std::time::Instant::now();
                let visible = self.strip_tabs(cx);
                let drawn: Vec<(TitleTab, f32)> = visible
                    .iter()
                    .enumerate()
                    .map(|(ix, id)| {
                        let shift = self.tab_shifts.get(id).map_or(0., |s| s.at(now).0);
                        (
                            id.clone(),
                            drag.bounds.get(ix).map_or(0., |b| b.left().as_f32()) + shift,
                        )
                    })
                    .collect();
                self.reorder_title_tab(&drag.id, drag.to, cx);
                self.tab_shifts.clear();
                if !cx.reduce_motion() {
                    for (ix, id) in self.strip_tabs(cx).iter().enumerate() {
                        let (Some((_, left)), Some(slot)) =
                            (drawn.iter().find(|(t, _)| t == id), drag.bounds.get(ix))
                        else {
                            continue;
                        };
                        let from = left - slot.left().as_f32();
                        if from.abs() > 0.5 {
                            self.tab_shifts.insert(
                                id.clone(),
                                TabShift {
                                    from,
                                    to: 0.,
                                    start: now,
                                },
                            );
                        }
                    }
                }
                self.skip_tab_click = true;
                self.persist(cx);
            }
            self.tabs_frame.update(cx, |_, cx| cx.notify());
        }
    }

    /// Where tab `id` is drawn now, easing toward `target`.
    pub(in crate::app::shell) fn eased_shift(
        &mut self,
        id: TitleTab,
        target: f32,
        follow: bool,
        reduced: bool,
    ) -> (f32, bool) {
        let now = std::time::Instant::now();
        // The dragged tab rides the pointer; it eases only into a slot.
        if follow || reduced {
            self.tab_shifts.remove(&id);
            if follow {
                self.tab_shifts.insert(
                    id.clone(),
                    TabShift {
                        from: target,
                        to: target,
                        start: now - std::time::Duration::from_secs(1),
                    },
                );
            }
            return (target, false);
        }
        let current = self.tab_shifts.get(&id).map(|s| s.at(now));
        if current.is_none_or(|_| self.tab_shifts[&id].to != target) {
            let from = current.map_or(0., |(value, _)| value);
            if from == target {
                self.tab_shifts.remove(&id);
                return (target, false);
            }
            self.tab_shifts.insert(
                id.clone(),
                TabShift {
                    from,
                    to: target,
                    start: now,
                },
            );
        }
        let (value, done) = self.tab_shifts[&id].at(now);
        if done && target == 0. {
            self.tab_shifts.remove(&id);
        }
        (value, !done)
    }

    pub(in crate::app::shell) fn tab_shift(&self, ix: usize) -> Pixels {
        let Some(d) = self.tab_drag.as_ref().filter(|d| d.active) else {
            return px(0.);
        };
        if ix == d.from {
            return d.offset;
        }
        if d.from < d.to && ix > d.from && ix <= d.to {
            return d.bounds[ix - 1].left() - d.bounds[ix].left();
        }
        if d.from > d.to && ix >= d.to && ix < d.from {
            return d.bounds[ix + 1].left() - d.bounds[ix].left();
        }
        px(0.)
    }
}

/// Highlight only the existing hairline; its parent keeps the wider, transparent
/// grab area centered on that line. The wash brightens the underlying border.
/// A sash resting on a snap spot: a 2px accent line over its border.
pub(in crate::app::shell) fn sash_snap_line(vertical: bool) -> gpui::Div {
    div().absolute().bg(accent_color()).rounded_full().map(|d| {
        if vertical {
            d.top_0()
                .h_full()
                .left(gpui::relative(0.5))
                .ml(px(-1.))
                .w(px(2.))
        } else {
            d.left_0()
                .w_full()
                .top(gpui::relative(0.5))
                .mt(px(-1.))
                .h(px(2.))
        }
    })
}

pub(crate) fn resize_border(vertical: bool, dragging: bool) -> gpui::Div {
    div()
        .absolute()
        .bg(content(0.12))
        .opacity(if dragging { 1. } else { 0. })
        .group_hover("resize-handle", |s| s.opacity(1.))
        .map(|d| {
            if vertical {
                d.top_0()
                    .h_full()
                    .left(gpui::relative(0.5))
                    .ml(px(-0.5))
                    .w(px(1.))
            } else {
                d.left_0()
                    .w_full()
                    .top(gpui::relative(0.5))
                    .mt(px(-0.5))
                    .h(px(1.))
            }
        })
}
