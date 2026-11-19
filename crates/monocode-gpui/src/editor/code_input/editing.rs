use super::*;

impl Input {
    pub(crate) fn editor(cx: &mut Context<Self>) -> Self {
        let mut input = Self::new(cx);
        input.code = Some(Code::new());
        input.code.as_mut().unwrap().wrap = cx
            .try_global::<crate::ui::preferences::Preferences>()
            .is_none_or(|p| p.file_word_wrap);
        input.label = "File editor".into();
        input.placeholder = "".into();
        input.font_size = 12.5;
        input.line_height = 20.;
        input.secondary_submit = true;
        input
    }

    pub(crate) fn set_editor_diff(&mut self, diff: bool, can_stage: bool) {
        if let Some(code) = &mut self.code {
            code.can_stage = can_stage;
        }
        if let Some(code) = &mut self.code
            && code.diff != diff
        {
            code.diff = diff;
            code.layout_width = None;
        }
    }

    pub(crate) fn set_editor_path(&mut self, path: String) {
        if let Some(code) = &mut self.code {
            code.path = path;
        }
    }

    pub(crate) fn selection_range(&self) -> Range<usize> {
        self.selection.clone()
    }

    pub(crate) fn replace_bytes(
        &mut self,
        range: Range<usize>,
        text: &str,
        cursor: usize,
        cx: &mut Context<Self>,
    ) {
        if self.read_only || self.is_composing() || !self.accept_code_edit(&range, text, cx) {
            return;
        }
        self.record_undo();
        self.map_code_folds(&range, text.len());
        self.content = format!(
            "{}{}{}",
            &self.content[..range.start],
            text,
            &self.content[range.end..]
        )
        .into();
        let at = range.start + cursor.min(text.len());
        self.selection = at..at;
        self.reversed = false;
        self.code_changed();
        if let Some(code) = &mut self.code {
            code.suppress_completion = true;
        }
        cx.emit(Changed);
        cx.notify();
    }

    pub(crate) fn set_search_matches(
        &mut self,
        matches: Vec<Range<usize>>,
        cx: &mut Context<Self>,
    ) {
        if let Some(code) = &mut self.code {
            code.search_matches = matches;
        }
        cx.notify();
    }

    pub(crate) fn word_wrap(&self) -> bool {
        self.code.as_ref().is_some_and(|c| c.wrap)
    }

    pub(crate) fn code_rail_hovered(&self) -> bool {
        self.code
            .as_ref()
            .is_some_and(|code| code.hovered || code.drag_offset.is_some())
    }

    pub(crate) fn editor_analysis(&self) -> &Analysis {
        &self.code.as_ref().unwrap().analysis
    }

    pub(crate) fn set_word_wrap(&mut self, wrap: bool, cx: &mut Context<Self>) {
        if let Some(code) = &mut self.code
            && code.wrap != wrap
        {
            code.wrap = wrap;
            code.layout_width = None;
            code.reveal = true;
            if wrap {
                self.scroll_x = px(0.);
            }
            cx.notify();
        }
    }

    pub(crate) fn set_editor_analysis(&mut self, analysis: Analysis, cx: &mut Context<Self>) {
        if let Some(code) = &mut self.code {
            code.folded.retain(|fold| analysis.folds.contains(fold));
            code.analysis = Arc::new(analysis);
            code.layout_width = None;
            cx.notify();
        }
    }

    pub(crate) fn cursor_location(&self) -> (usize, usize) {
        let at = self.cursor();
        let row = self.code.as_ref().map_or(0, |c| c.row(at));
        let start = self.code.as_ref().map_or(0, |c| c.offsets[row]);
        (row + 1, self.content[start..at].chars().count() + 1)
    }

    pub(crate) fn editor_line_count(&self) -> usize {
        self.code.as_ref().map_or(1, |code| code.offsets.len())
    }

    pub(crate) fn reload_file(
        &mut self,
        text: String,
        preserve_viewport: bool,
        cx: &mut Context<Self>,
    ) {
        let selection = self.selection.clone();
        let scroll = point(self.scroll_x, self.scroll_y);
        self.set_text(text, cx);
        self.reset_editor_history();
        if preserve_viewport {
            let clamp = |at: usize| {
                let mut at = at.min(self.content.len());
                while !self.content.is_char_boundary(at) {
                    at -= 1;
                }
                at
            };
            self.selection = clamp(selection.start)..clamp(selection.end);
            self.scroll_x = scroll.x;
            self.scroll_y = scroll.y;
            if let Some(code) = &mut self.code {
                code.reveal = false;
            }
        } else {
            self.select_range(0..0, cx);
        }
    }

    pub(crate) fn reset_editor_history(&mut self) {
        if let Some(code) = &mut self.code {
            code.undo.clear();
            code.redo.clear();
        }
    }

    pub(crate) fn accept_code_edit(
        &self,
        range: &Range<usize>,
        text: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        use monocode_core::files::document::{MAX_LINE_BYTES, MAX_TEXT_BYTES};
        if self.content.len() - range.len() + text.len() > MAX_TEXT_BYTES {
            cx.emit(Rejected("Edit exceeds the 2 MB file limit."));
            return false;
        }
        let start = self.content[..range.start].rfind('\n').map_or(0, |n| n + 1);
        let end = self.content[range.end..]
            .find('\n')
            .map_or(self.content.len(), |n| range.end + n);
        let changed = format!(
            "{}{}{}",
            &self.content[start..range.start],
            text,
            &self.content[range.end..end]
        );
        if changed.split('\n').any(|line| line.len() > MAX_LINE_BYTES) {
            cx.emit(Rejected("Edit exceeds the 16 KB line limit."));
            return false;
        }
        true
    }

    pub(crate) fn code_changed(&mut self) {
        if let Some(code) = &mut self.code {
            code.completions.clear();
            code.completion_task = None;
            code.completion_source = None;
            code.deleted_rows.clear();
            code.hovered_change = None;
            code.offsets = std::iter::once(0)
                .chain(self.content.match_indices('\n').map(|(at, _)| at + 1))
                .collect();
            code.visible.clear();
            code.reveal = true;
            code.max_width = px(0.);
            code.layout_width = None;
            code.tops = (0..=code.offsets.len())
                .map(|row| TOP_PADDING + row as f32 * self.line_height)
                .collect();
            // Carry the last results through the edit until fresh analysis
            // lands; clearing them would flash the whole buffer uncoloured.
            let previous = std::mem::replace(&mut code.text, self.content.clone());
            if let Some(edit) = crate::editor::analysis::Edit::between(&previous, &self.content) {
                code.analysis = Arc::new(code.analysis.remapped(edit));
            }
            code.markers.clear();
            code.affinity.set(None);
        }
    }

    pub(crate) fn reveal_code(&mut self) {
        if let Some(code) = &mut self.code {
            code.reveal = true;
        }
    }

    pub(crate) fn record_undo(&mut self) {
        if let Some(code) = &mut self.code
            && self.marked.is_none()
        {
            code.undo
                .push((self.content.clone(), self.selection.clone()));
            let mut bytes = 0;
            let mut keep = 0;
            for (text, _) in code.undo.iter().rev().take(100) {
                bytes += text.len();
                if bytes > 8 * 1024 * 1024 {
                    break;
                }
                keep += 1;
            }
            let discard = code.undo.len().saturating_sub(keep.max(1));
            code.undo.drain(..discard);
            code.redo.clear();
        }
    }

    pub(crate) fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.history(false, cx);
    }

    pub(crate) fn history_available(&self, redo: bool) -> bool {
        !self.read_only
            && self.marked.is_none()
            && self
                .code
                .as_ref()
                .is_some_and(|code| !(if redo { &code.redo } else { &code.undo }).is_empty())
    }

    pub(crate) fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        self.history(true, cx);
    }

    pub(super) fn history(&mut self, redo: bool, cx: &mut Context<Self>) {
        let Some(code) = &mut self.code else {
            cx.propagate();
            return;
        };
        if self.marked.is_some() {
            return;
        }
        let source = if redo { &mut code.redo } else { &mut code.undo };
        let Some((text, selection)) = source.pop() else {
            return;
        };
        let target = if redo { &mut code.undo } else { &mut code.redo };
        target.push((self.content.clone(), self.selection.clone()));
        self.clear_folds();
        self.content = text;
        self.selection = selection;
        self.reversed = false;
        self.code_changed();
        cx.emit(Changed);
        cx.notify();
    }

    pub(crate) fn indent(&mut self, _: &Indent, w: &mut Window, cx: &mut Context<Self>) {
        if self.code.is_none() {
            cx.propagate();
            return;
        }
        if self.accept_completion(cx) {
            return;
        }
        if self.selection.is_empty() {
            if self.expand_emmet(cx) {
                return;
            }
            self.replace_text_in_range(None, "  ", w, cx);
        } else {
            self.indent_lines(false, w, cx);
        }
    }

    pub(crate) fn outdent(&mut self, _: &Outdent, w: &mut Window, cx: &mut Context<Self>) {
        if self.code.is_none() {
            cx.propagate();
            return;
        }
        self.indent_lines(true, w, cx);
    }

    pub(super) fn indent_lines(&mut self, outdent: bool, w: &mut Window, cx: &mut Context<Self>) {
        if self.marked.is_some() {
            return;
        }
        let selection = self.selection.clone();
        let start = self.content[..selection.start]
            .rfind('\n')
            .map_or(0, |at| at + 1);
        let last = if selection.end > selection.start
            && self.content.as_bytes().get(selection.end - 1) == Some(&b'\n')
        {
            selection.end - 1
        } else {
            selection.end
        };
        let end = self.content[last..]
            .find('\n')
            .map_or(self.content.len(), |n| last + n);
        let replacement = self.content[start..end]
            .split('\n')
            .map(|line| {
                if outdent {
                    let remove = if line.starts_with('\t') {
                        1
                    } else {
                        line.bytes().take(4).take_while(|c| *c == b' ').count()
                    };
                    line[remove..].to_owned()
                } else {
                    format!("    {line}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let removed = (end - start).saturating_sub(replacement.len());
        let inserted = replacement.len();
        self.replace_text_in_range(Some(self.range16(start..end)), &replacement, w, cx);
        if selection.is_empty() {
            self.move_to(selection.start.saturating_sub(removed).max(start), cx);
        } else {
            self.select_range(start..start + inserted, cx);
        }
    }
}
