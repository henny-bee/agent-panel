use super::*;

impl Input {
    pub(crate) fn code_position(&self, at: usize) -> Point<Pixels> {
        let code = self.code.as_ref().unwrap();
        let row = code.display_row(at);
        let p = code
            .visible
            .iter()
            .find(|(ix, _)| *ix == row)
            .map(|(_, line)| {
                line_position(
                    line,
                    code.display_offset(row, at),
                    self.line_height(),
                    code.affinity.get() == Some((at, true)),
                )
            })
            .unwrap_or_default();
        p + point(px(0.), px(code.top(row, self.line_height)))
    }

    pub(crate) fn code_index(&self, p: Point<Pixels>) -> usize {
        let code = self.code.as_ref().unwrap();
        let row = code.row_at_y(p.y.as_f32());
        let start = code.offsets[row];
        let at = code
            .visible
            .iter()
            .find(|(ix, _)| *ix == row)
            .map_or(start, |(_, line)| {
                code.source_offset(
                    row,
                    line.closest_index_for_position(
                        point(p.x, p.y - px(code.top(row, self.line_height))),
                        self.line_height(),
                    )
                    .unwrap_or_else(|ix| ix),
                )
            });
        if let Some((_, line)) = code.visible.iter().find(|(ix, _)| *ix == row) {
            let local_y = (p.y.as_f32() - code.top(row, self.line_height)).max(0.);
            let downstream = line_position(
                line,
                code.display_offset(row, at),
                self.line_height(),
                false,
            );
            code.affinity.set(Some((
                at,
                (local_y / self.line_height).floor()
                    < (downstream.y.as_f32() / self.line_height).floor(),
            )));
        }
        at
    }

    pub(crate) fn code_vertical(&self, down: bool) -> usize {
        let code = self.code.as_ref().unwrap();
        if code.wrap {
            let p = self.code_position(self.cursor());
            return self.code_index(point(
                p.x,
                (p.y + self.line_height() * if down { 1. } else { -1. }).max(px(0.)),
            ));
        }
        let row = code.row(self.cursor());
        let col = self.content[code.offsets[row]..self.cursor()]
            .graphemes(true)
            .count();
        let mut row = if down {
            (row + 1).min(code.offsets.len() - 1)
        } else {
            row.saturating_sub(1)
        };
        while code.hidden(row) && row > 0 && row + 1 < code.offsets.len() {
            if down {
                row += 1;
            } else {
                row -= 1;
            }
        }
        let start = code.offsets[row];
        let end = code
            .offsets
            .get(row + 1)
            .map_or(self.content.len(), |s| s - 1);
        start
            + self.content[start..end]
                .grapheme_indices(true)
                .nth(col)
                .map_or(end - start, |(at, _)| at)
    }

    pub(crate) fn code_scroll(
        &mut self,
        e: &gpui::ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(code) = &mut self.code {
            let delta = e.delta.pixel_delta(px(self.line_height));
            let height = self.bounds.map_or(px(0.), |b| b.size.height);
            self.scroll_y = (self.scroll_y - delta.y)
                .max(px(0.))
                .min((px(code.total()) - height).max(px(0.)));
            self.scroll_x = (self.scroll_x - delta.x).max(px(0.)).min(code.max_width);
            code.reveal = false;
            cx.stop_propagation();
            cx.notify();
        }
    }

    pub(crate) fn code_scrollbar_down(
        &mut self,
        e: &MouseDownEvent,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(code) = &mut self.code else {
            return false;
        };
        let Some(rail) = code.rail.filter(|b| b.contains(&e.position)) else {
            return false;
        };
        cx.stop_propagation();
        if let Some(marker) = code
            .markers
            .iter()
            .rev()
            .find(|m| m.bounds.contains(&e.position))
            .cloned()
        {
            if marker.diagnostic {
                self.move_to(marker.pos, cx);
            }
            let code = self.code.as_mut().unwrap();
            let row = code.row(marker.pos);
            self.scroll_y =
                px((code.top(row, self.line_height) - rail.size.height.as_f32() / 2.).max(0.));
            code.reveal = false;
            w.focus(&self.focus, cx);
        } else {
            let metrics = Metrics::new(
                self.scroll_y.as_f32(),
                code.total(),
                rail.size.height.as_f32(),
                rail.size.height.as_f32(),
            );
            if !metrics.scrollable {
                return true;
            }
            let y = (e.position.y - rail.top()).as_f32();
            let offset = if e.position.x >= rail.left() + px(3.)
                && e.position.x <= rail.right() - px(3.)
                && y >= metrics.thumb_top
                && y <= metrics.thumb_top + metrics.thumb_height
            {
                y - metrics.thumb_top
            } else {
                metrics.thumb_height / 2.
            };
            code.drag_offset = Some(offset);
            self.scroll_y = px(metrics.scroll_from_pointer(y, offset));
            code.reveal = false;
        }
        cx.notify();
        true
    }

    pub(crate) fn code_scrollbar_move(
        &mut self,
        e: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(code) = &mut self.code else {
            return false;
        };
        let Some(rail) = code.rail else {
            return false;
        };
        let hovered = rail.contains(&e.position);
        if code.hovered != hovered {
            code.hovered = hovered;
            cx.notify();
        }
        let marker = code
            .markers
            .iter()
            .rposition(|m| m.bounds.contains(&e.position) && !m.message.is_empty());
        if code.hovered_marker != marker {
            code.hovered_marker = marker;
            cx.notify();
        }
        if let Some(offset) = code.drag_offset {
            if e.pressed_button != Some(MouseButton::Left) {
                code.drag_offset = None;
                return false;
            }
            let metrics = Metrics::new(
                self.scroll_y.as_f32(),
                code.total(),
                rail.size.height.as_f32(),
                rail.size.height.as_f32(),
            );
            self.scroll_y =
                px(metrics.scroll_from_pointer((e.position.y - rail.top()).as_f32(), offset));
            code.reveal = false;
            cx.notify();
            return true;
        }
        false
    }

    pub(crate) fn code_max_scroll_y(&self) -> Pixels {
        let height = self.bounds.map_or(px(0.), |b| b.size.height);
        self.code
            .as_ref()
            .map_or(px(0.), |code| (px(code.total()) - height).max(px(0.)))
    }

    pub(crate) fn code_scrollbar_up(&mut self) {
        if let Some(code) = &mut self.code {
            code.drag_offset = None;
        }
    }

    pub(crate) fn code_hover(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        let (Some(code), Some(bounds)) = (&mut self.code, self.bounds) else {
            return;
        };
        if !code.diff {
            return;
        }
        let y = (e.position.y - bounds.top() + self.scroll_y).as_f32();
        let row = code.row_at_y(y);
        let hovered = if e.position.x >= bounds.left() - px(code.gutter())
            && e.position.x <= bounds.right()
            && e.position.y >= bounds.top()
            && e.position.y <= bounds.bottom()
        {
            code.analysis
                .changes
                .iter()
                .position(|c| code.row(c.from) <= row && code.row(c.to) >= row)
        } else {
            None
        };
        if hovered != code.hovered_change {
            code.hovered_change = hovered;
            cx.notify();
        }
    }

    pub(crate) fn hunk_actions(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let code = self.code.as_ref()?;
        let index = code.hovered_change?;
        let change = code.analysis.changes.get(index)?;
        let row = code.row(change.from);
        let top = (code.top(row, self.line_height) - self.scroll_y.as_f32()).max(0.);
        Some(
            div()
                .id("editor-hunk-actions")
                .absolute()
                .left(px(code.gutter() - 18.))
                .top(px(top))
                .w(px(18.))
                .flex()
                .flex_col()
                .overflow_hidden()
                .rounded(px(4.))
                .border_1()
                .border_color(crate::ui::theme::content(0.14))
                .bg(crate::ui::theme::shell_fill().opacity(0.92))
                .shadow_md()
                .when(code.can_stage, |d| {
                    d.child(
                        crate::ui::panel::icon_action("stage-change", "Stage change", "plus", true)
                            .size(px(18.))
                            .on_click(
                                cx.listener(move |_, _, _, cx| cx.emit(CodeAction::Stage(index))),
                            ),
                    )
                })
                .child(
                    crate::ui::panel::icon_action(
                        "revert-change",
                        "Revert change",
                        "rotate-ccw",
                        true,
                    )
                    .size(px(18.))
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(CodeAction::Revert(index)))),
                )
                .into_any_element(),
        )
    }

    pub(crate) fn code_tooltip(&self) -> Option<AnyElement> {
        let code = self.code.as_ref()?;
        let marker = code.markers.get(code.hovered_marker?)?;
        let rail = code.rail?;
        Some(
            div()
                .absolute()
                .right(px(WIDTH + 6.))
                .top(
                    (marker.bounds.top() - rail.top())
                        .min((rail.size.height - px(32.)).max(px(0.))),
                )
                .max_w(px(360.))
                .px_2()
                .py_1()
                .rounded_md()
                .bg(crate::ui::theme::pane_fill())
                .border_1()
                .border_color(crate::ui::theme::content(0.14))
                .text_size(px(12.))
                .text_color(crate::ui::theme::content(1.))
                .child(marker.message.clone())
                .into_any_element(),
        )
    }
}
