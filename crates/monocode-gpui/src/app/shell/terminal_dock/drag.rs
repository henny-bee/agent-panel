use super::*;

impl Shell {
    /// SurfaceTabs selects a tab as it is pressed, then lets it be dragged.
    pub(super) fn start_dock_drag(
        &mut self,
        ix: usize,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dock_skip_click = false;
        self.select_terminal(ix, window, cx);
        let count = self.live_terminals().len();
        if count == 0 {
            return;
        }
        let bounds: Vec<_> = (0..count)
            .filter_map(|ix| self.dock_tab_scroll.bounds_for_item(ix))
            .collect();
        if bounds.len() != count {
            return;
        }
        self.dock_drag = Some(DockTabDrag {
            start: event.position,
            from: ix,
            to: ix,
            origin: event.position.x,
            offset: px(0.),
            active: false,
            bounds,
        });
    }

    pub(in crate::app::shell) fn move_dock_drag(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.dock_drag.as_mut() else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.dock_drag = None;
            cx.notify();
            return;
        }
        if (event.position.y - drag.start.y).abs() > DOCK_PULL {
            let (source, start) = (DragSource::DockTerminal(drag.from), drag.start);
            self.dock_drag = None;
            self.dock_skip_click = true;
            self.tab_drag = None;
            self.pane_drag = Some(PaneDrag {
                source,
                start,
                position: event.position,
                active: false,
                over: None,
                insert: None,
            });
            cx.notify();
            return;
        }
        let delta = event.position.x - drag.origin;
        if delta.abs() < px(4.) && !drag.active {
            return;
        }
        drag.active = true;
        self.dock_skip_click = true;
        let start = drag.bounds[drag.from].left();
        drag.offset = delta.clamp(
            drag.bounds[0].left() - start,
            drag.bounds.last().unwrap().left() - start,
        );
        let at = start + drag.offset;
        drag.to = drag
            .bounds
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                (a.left() - at)
                    .abs()
                    .partial_cmp(&(b.left() - at).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map_or(drag.from, |(ix, _)| ix);
        cx.notify();
    }

    pub(in crate::app::shell) fn finish_dock_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.dock_drag.take() else {
            return;
        };
        if !drag.active || drag.from == drag.to {
            cx.notify();
            return;
        }
        let project = self.workspace.project.clone();
        let now = Instant::now();
        let live = self.live_terminals();
        // Each tab glides from where it is drawn into its new slot.
        let drawn: Vec<(EntityId, f32)> = live
            .iter()
            .enumerate()
            .map(|(ix, t)| {
                let id = t.entity_id();
                let shift = self.dock_shifts.get(&id).map_or(0., |s| s.at(now).0);
                let shift = if ix == drag.from {
                    drag.offset.as_f32()
                } else {
                    shift
                };
                (
                    id,
                    drag.bounds.get(ix).map_or(0., |b| b.left().as_f32()) + shift,
                )
            })
            .collect();
        if let Some(live) = self.terminals.get_mut(&project) {
            let moved = live.remove(drag.from);
            live.insert(drag.to, moved);
        }
        if let Some(dock) = self.workspace.docks.get_mut(&project) {
            dock.reorder(drag.from, drag.to);
        }
        self.dock_shifts.clear();
        if !cx.reduce_motion() {
            for (ix, terminal) in self.live_terminals().iter().enumerate() {
                let id = terminal.entity_id();
                let (Some((_, left)), Some(slot)) =
                    (drawn.iter().find(|(t, _)| *t == id), drag.bounds.get(ix))
                else {
                    continue;
                };
                let from = left - slot.left().as_f32();
                if from.abs() > 0.5 {
                    self.dock_shifts.insert(
                        id,
                        TabShift {
                            from,
                            to: 0.,
                            start: now,
                        },
                    );
                }
            }
        }
        self.persist(cx);
        cx.notify();
    }

    pub(super) fn dock_tab_shift(&self, ix: usize) -> Pixels {
        let Some(d) = self.dock_drag.as_ref().filter(|d| d.active) else {
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
