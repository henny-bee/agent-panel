use super::*;

impl Input {
    pub(crate) fn select_range(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        self.manual_scroll = false;
        self.reveal_pending = self.reveal_in_parent;
        let range = self.clip_range(range);
        self.unfold_range(&range);
        self.reveal_code();
        self.selection = range;
        self.reversed = false;
        cx.notify();
    }

    /// Layouts and native text callbacks can still refer to the previous value.
    /// Every cursor and edit range must index the current UTF-8 buffer.
    pub(super) fn clip_offset(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.content.len());
        while !self.content.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    pub(super) fn clip_range(&self, range: Range<usize>) -> Range<usize> {
        let start = self.clip_offset(range.start);
        let end = self.clip_offset(range.end);
        start.min(end)..start.max(end)
    }

    pub(super) fn replacement_range(&self, range: Option<Range<usize>>) -> Range<usize> {
        self.clip_range(
            range
                .map(|range| self.range8(range))
                .or_else(|| self.marked.clone())
                .unwrap_or_else(|| self.selection.clone()),
        )
    }

    pub(crate) fn cursor(&self) -> usize {
        self.clip_offset(if self.reversed {
            self.selection.start
        } else {
            self.selection.end
        })
    }

    pub(super) fn scroll(
        &mut self,
        event: &gpui::ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.code.is_some() {
            self.code_scroll(event, window, cx);
            return;
        }
        if self.single_line {
            return;
        }
        let line_height = px(self.line_height);
        let total = self
            .lines
            .iter()
            .map(|line| line.size(line_height).height)
            .fold(px(0.), |a, b| a + b);
        let viewport = self.bounds.map_or(px(0.), |b| b.size.height);
        let max_scroll = (total - viewport).max(px(0.));
        if max_scroll <= px(0.) {
            return;
        }
        let next =
            (self.scroll_y - event.delta.pixel_delta(line_height).y).clamp(px(0.), max_scroll);
        if next == self.scroll_y {
            // Let the surrounding page scroll once the textarea reaches an edge.
            return;
        }
        self.scroll_y = next;
        self.manual_scroll = true;
        cx.stop_propagation();
        cx.notify();
    }

    pub(crate) fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.manual_scroll = false;
        self.reveal_pending = self.reveal_in_parent;
        self.reveal_code();
        self.clear_completion();
        let offset = self.clip_offset(offset);
        self.selection = offset..offset;
        self.reversed = false;
        cx.notify();
    }

    pub(super) fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.manual_scroll = false;
        self.reveal_pending = self.reveal_in_parent;
        self.reveal_code();
        let offset = self.clip_offset(offset);
        self.selection = self.clip_range(self.selection.clone());
        if self.reversed {
            self.selection.start = offset;
        } else {
            self.selection.end = offset;
        }
        if self.selection.start > self.selection.end {
            self.reversed = !self.reversed;
            self.selection = self.selection.end..self.selection.start;
        }
        cx.notify();
    }

    pub(super) fn prev(&self, at: usize) -> usize {
        if let Some(at) = self.folded_boundary(at, true) {
            return at;
        }
        if self.code.is_some() {
            return self.content[..at]
                .grapheme_indices(true)
                .next_back()
                .map_or(0, |(at, _)| at);
        }
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(ix, _)| (ix < at).then_some(ix))
            .unwrap_or(0)
    }

    pub(crate) fn next(&self, at: usize) -> usize {
        if let Some(at) = self.folded_boundary(at, false) {
            return at;
        }
        if self.code.is_some() {
            return self.content[at..]
                .grapheme_indices(true)
                .nth(1)
                .map_or(self.content.len(), |(next, _)| at + next);
        }
        self.content
            .grapheme_indices(true)
            .find_map(|(ix, _)| (ix > at).then_some(ix))
            .unwrap_or(self.content.len())
    }

    pub(super) fn from16(&self, offset: usize) -> usize {
        from16(&self.content, offset)
    }

    pub(super) fn to16(&self, offset: usize) -> usize {
        self.content[..self.clip_offset(offset)]
            .encode_utf16()
            .count()
    }

    pub(crate) fn range16(&self, r: Range<usize>) -> Range<usize> {
        let r = self.clip_range(r);
        self.to16(r.start)..self.to16(r.end)
    }

    pub(super) fn range8(&self, r: Range<usize>) -> Range<usize> {
        self.clip_range(self.from16(r.start)..self.from16(r.end))
    }

    pub(crate) fn line_height(&self) -> Pixels {
        px(if self.single_line {
            self.height
        } else {
            self.line_height
        })
    }

    pub(super) fn previous_word(&self) -> usize {
        self.content
            .unicode_word_indices()
            .rev()
            .find_map(|(start, _)| (start < self.cursor()).then_some(start))
            .unwrap_or(0)
    }

    pub(super) fn next_word(&self) -> usize {
        self.content
            .unicode_word_indices()
            .map(|(start, word)| start + word.len())
            .find(|end| *end > self.cursor())
            .unwrap_or(self.content.len())
    }

    pub(super) fn line_edge(&self, end: bool) -> usize {
        if self.code.is_some() {
            if self.word_wrap() {
                let p = self.code_position(self.cursor());
                return self.code_index(point(if end { px(f32::MAX) } else { px(0.) }, p.y));
            }
            let at = self.cursor();
            return if end {
                self.content[at..]
                    .find('\n')
                    .map_or(self.content.len(), |n| at + n)
            } else {
                self.content[..at].rfind('\n').map_or(0, |n| n + 1)
            };
        }
        if self.single_line {
            return if end { self.content.len() } else { 0 };
        }
        let p = self.position(self.cursor());
        self.index(point(if end { px(f32::MAX) } else { px(0.) }, p.y))
    }

    pub(super) fn vertical(&self, down: bool) -> usize {
        if self.code.is_some() {
            return self.code_vertical(down);
        }
        let p = self.position(self.cursor());
        let y = p.y + self.line_height() * if down { 1. } else { -1. };
        if y < px(0.) {
            0
        } else {
            self.index(point(p.x, y))
        }
    }

    pub(super) fn normalized<'a>(&self, text: &'a str) -> Cow<'a, str> {
        if self.single_line && text.contains(['\r', '\n']) {
            Cow::Owned(text.replace("\r\n", " ").replace(['\r', '\n'], " "))
        } else {
            Cow::Borrowed(text)
        }
    }

    pub(super) fn position(&self, at: usize) -> Point<Pixels> {
        if self.code.is_some() {
            return self.code_position(at);
        }
        let at = self.display_layout().to_display(at);
        let mut base = 0;
        let mut y = px(0.);
        for line in &self.lines {
            if at <= base + line.len() {
                return line
                    .position_for_index(at - base, self.line_height())
                    .unwrap_or_default()
                    + point(px(0.), y);
            }
            base += line.len() + 1;
            y += line.size(self.line_height()).height;
        }
        point(px(0.), y)
    }

    pub(super) fn index(&self, p: Point<Pixels>) -> usize {
        // The placeholder is painted as text, but is never editable content.
        if self.content.is_empty() {
            return 0;
        }
        if self.code.is_some() {
            return self.clip_offset(self.code_index(p));
        }
        let mut base = 0;
        let mut y = p.y;
        for line in &self.lines {
            let h = line.size(self.line_height()).height;
            if y < h {
                let at = base
                    + line
                        .closest_index_for_position(point(p.x, y.max(px(0.))), self.line_height())
                        .unwrap_or_else(|ix| ix);
                let at = self.display_layout().to_source(at);
                return self.clip_offset(at);
            }
            y -= h;
            base += line.len() + 1;
        }
        self.content.len()
    }

    pub(super) fn mouse_index(&self, p: Point<Pixels>) -> usize {
        self.bounds
            .map(|b| self.index(p - b.origin + point(self.scroll_x, self.scroll_y)))
            .unwrap_or(0)
            .min(self.content.len())
    }

    pub(super) fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(
            if self.selection.is_empty() {
                self.prev(self.cursor())
            } else {
                self.selection.start
            },
            cx,
        );
    }

    pub(super) fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(
            if self.selection.is_empty() {
                self.next(self.cursor())
            } else {
                self.selection.end
            },
            cx,
        );
    }

    pub(super) fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.prev(self.cursor()), cx);
    }

    pub(super) fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next(self.cursor()), cx);
    }

    pub(super) fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx);
    }

    pub(super) fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.line_edge(false), cx);
    }

    pub(super) fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.line_edge(true), cx);
    }

    pub(super) fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        if self.yields_navigation(cx) {
            return;
        }
        self.select_to(self.line_edge(false), cx);
    }

    pub(super) fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        if self.yields_navigation(cx) {
            return;
        }
        self.select_to(self.line_edge(true), cx);
    }

    pub(super) fn document_start(
        &mut self,
        _: &DocumentStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.yields_navigation(cx) {
            return;
        }
        self.move_to(0, cx);
    }

    pub(super) fn document_end(&mut self, _: &DocumentEnd, _: &mut Window, cx: &mut Context<Self>) {
        if self.yields_navigation(cx) {
            return;
        }
        self.move_to(self.content.len(), cx);
    }

    pub(super) fn select_document_start(
        &mut self,
        _: &SelectDocumentStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.yields_navigation(cx) {
            return;
        }
        self.select_to(0, cx);
    }

    pub(super) fn select_document_end(
        &mut self,
        _: &SelectDocumentEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.yields_navigation(cx) {
            return;
        }
        self.select_to(self.content.len(), cx);
    }

    pub(super) fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.previous_word(), cx);
    }

    pub(super) fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.next_word(), cx);
    }

    pub(super) fn select_word_left(
        &mut self,
        _: &SelectWordLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.yields_navigation(cx) {
            return;
        }
        self.select_to(self.previous_word(), cx);
    }

    pub(super) fn select_word_right(
        &mut self,
        _: &SelectWordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.yields_navigation(cx) {
            return;
        }
        self.select_to(self.next_word(), cx);
    }

    pub(super) fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        if !self.completion_step(true, cx) {
            self.move_to(self.vertical(false), cx);
        }
    }

    pub(super) fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        if !self.completion_step(false, cx) {
            self.move_to(self.vertical(true), cx);
        }
    }

    pub(super) fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.vertical(false), cx);
    }

    pub(super) fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.vertical(true), cx);
    }

    pub(super) fn delete_word_left(
        &mut self,
        _: &DeleteWordLeft,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selection.is_empty() {
            self.select_to(self.previous_word(), cx);
        }
        self.replace_text_in_range(None, "", w, cx);
    }

    pub(super) fn delete_word_right(
        &mut self,
        _: &DeleteWordRight,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selection.is_empty() {
            self.select_to(self.next_word(), cx);
        }
        self.replace_text_in_range(None, "", w, cx);
    }

    pub(super) fn delete_to_home(
        &mut self,
        _: &DeleteToHome,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selection.is_empty() {
            self.select_to(self.line_edge(false), cx);
        }
        self.replace_text_in_range(None, "", w, cx);
    }

    pub(super) fn delete_to_end(
        &mut self,
        _: &DeleteToEnd,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selection.is_empty() {
            self.select_to(self.line_edge(true), cx);
        }
        self.replace_text_in_range(None, "", w, cx);
    }

    pub(super) fn backspace(&mut self, _: &Backspace, w: &mut Window, cx: &mut Context<Self>) {
        if self.selection == (0..0)
            && self.inline_commands
            && Command::leading(&self.content).is_none()
            && let Some(command) = self.selected_command
        {
            cx.emit(CommandRemoved(command.name()));
            return;
        }
        if self.selection.is_empty() {
            self.select_to(self.prev(self.cursor()), cx);
        }
        self.replace_text_in_range(None, "", w, cx);
    }

    pub(super) fn delete(&mut self, _: &Delete, w: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            self.select_to(self.next(self.cursor()), cx);
        }
        self.replace_text_in_range(None, "", w, cx);
    }

    pub(super) fn newline(&mut self, _: &Newline, w: &mut Window, cx: &mut Context<Self>) {
        if !self.single_line {
            self.replace_text_in_range(None, "\n", w, cx);
        }
    }

    pub(super) fn submit(&mut self, _: &Submit, w: &mut Window, cx: &mut Context<Self>) {
        if self.accept_completion(cx) {
            return;
        }
        if self.secondary_submit {
            if self.code.is_some() {
                let at = self.selection.start;
                let start = self.content[..at].rfind('\n').map_or(0, |n| n + 1);
                let indent: String = self.content[start..at]
                    .chars()
                    .take_while(|c| matches!(c, ' ' | '\t'))
                    .collect();
                self.replace_text_in_range(None, &format!("\n{indent}"), w, cx);
            } else {
                self.replace_text_in_range(None, "\n", w, cx);
            }
        } else if self.marked.is_none() {
            cx.emit(Submitted);
        }
    }

    pub(super) fn submit_secondary(
        &mut self,
        _: &SubmitSecondary,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.secondary_submit {
            if self.marked.is_none() {
                cx.emit(Submitted);
            }
        } else if self.alternate_submit {
            if self.marked.is_none() {
                cx.emit(AlternateSubmitted);
            }
        } else {
            cx.propagate();
        }
    }

    pub(super) fn paste(&mut self, _: &Paste, w: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
            self.replace_text_in_range(None, &text.replace("\r\n", "\n"), w, cx);
        }
    }

    pub(super) fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.secret && !self.selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selection.clone()].into(),
            ));
        }
    }

    pub(super) fn cut(&mut self, _: &Cut, w: &mut Window, cx: &mut Context<Self>) {
        self.copy(&Copy, w, cx);
        self.replace_text_in_range(None, "", w, cx);
    }

    pub(super) fn mouse_down(
        &mut self,
        e: &MouseDownEvent,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if cfg!(target_os = "macos") && e.modifiers.control {
            self.open_context_menu(e, w, cx);
            return;
        }
        #[cfg(target_os = "macos")]
        crate::ui::traffic_lights::focus_content(w);
        if self.code_fold_down(e, cx) || self.code_scrollbar_down(e, w, cx) {
            return;
        }
        w.focus(&self.focus, cx);
        self.selecting = true;
        self.drag_position = Some(e.position);
        let ix = self.mouse_index(e.position);
        if e.modifiers.shift {
            self.select_to(ix, cx)
        } else if e.click_count >= 3 {
            let start = self.content[..ix].rfind('\n').map_or(0, |i| i + 1);
            let end = self.content[ix..]
                .find('\n')
                .map_or(self.content.len(), |i| ix + i);
            self.move_to(start, cx);
            self.select_to(end, cx);
        } else if e.click_count == 2 {
            let range = self
                .content
                .split_word_bound_indices()
                .find(|(start, word)| ix >= *start && ix < start + word.len())
                .map(|(start, word)| start..start + word.len())
                .unwrap_or(ix..ix);
            self.move_to(range.start, cx);
            self.select_to(range.end, cx);
        } else {
            self.move_to(ix, cx)
        }
    }

    pub(super) fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.end_drag();
        self.code_scrollbar_up();
    }

    pub(super) fn mouse_move(
        &mut self,
        e: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.code_hover(e, cx);
        self.code_scrollbar_move(e, cx);
    }

    pub(super) fn end_drag(&mut self) {
        self.selecting = false;
        self.drag_position = None;
        self.autoscroll = None;
    }

    /// Extends a drag selection wherever the pointer goes, not only while it
    /// hovers the field, and keeps scrolling while it is held past an edge.
    pub(super) fn drag_move(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        if e.pressed_button != Some(MouseButton::Left) {
            self.end_drag();
            return;
        }
        self.drag_position = Some(e.position);
        self.drag_select(cx);
        if self.autoscroll.is_none() && self.drag_overshoot() != px(0.) {
            self.autoscroll = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(16))
                        .await;
                    if !this
                        .update(cx, |input, cx| input.autoscroll_step(cx))
                        .unwrap_or(false)
                    {
                        break;
                    }
                }
            }));
        }
    }

    /// Selects up to the pointer, pinned inside the viewport so the selection
    /// follows the scroll instead of jumping ahead of it.
    pub(super) fn drag_select(&mut self, cx: &mut Context<Self>) {
        let (Some(p), Some(b)) = (self.drag_position, self.bounds) else {
            return;
        };
        let y = p.y.max(b.top()).min(b.bottom() - px(1.));
        self.select_to(self.mouse_index(point(p.x, y)), cx);
    }

    /// How far the dragging pointer is above (negative) or below the field.
    pub(super) fn drag_overshoot(&self) -> Pixels {
        match (self.drag_position, self.bounds) {
            _ if self.single_line => px(0.),
            (Some(p), Some(b)) if p.y < b.top() => p.y - b.top(),
            (Some(p), Some(b)) if p.y > b.bottom() => p.y - b.bottom(),
            _ => px(0.),
        }
    }

    pub(super) fn max_scroll_y(&self) -> Pixels {
        if self.code.is_some() {
            return self.code_max_scroll_y();
        }
        let total = self
            .lines
            .iter()
            .map(|line| line.size(self.line_height()).height)
            .fold(px(0.), |a, b| a + b);
        (total - self.bounds.map_or(px(0.), |b| b.size.height)).max(px(0.))
    }

    pub(super) fn autoscroll_step(&mut self, cx: &mut Context<Self>) -> bool {
        let overshoot = self.drag_overshoot().as_f32();
        let max = self.max_scroll_y();
        if !self.selecting || overshoot == 0. || max <= px(0.) {
            self.autoscroll = None;
            return false;
        }
        // Speed grows with distance past the edge, like native text views.
        let limit = self.line_height * 3.;
        let step = (overshoot / 4.).clamp(-limit, limit);
        let step = if step.abs() < 1. { step.signum() } else { step };
        self.scroll_y = (self.scroll_y + px(step)).max(px(0.)).min(max);
        self.drag_select(cx);
        true
    }
}
