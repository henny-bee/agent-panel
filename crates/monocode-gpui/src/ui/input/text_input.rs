use super::*;

impl EntityInputHandler for Input {
    fn accepts_text_input(&self, _: &mut Window, _: &mut Context<Self>) -> bool {
        !self.read_only
    }
    fn text_input_configuration(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::TextInputConfiguration {
        let enabled = !self.secret
            && !self.single_line
            && self.inline_commands
            && self.code.is_none()
            && cx
                .try_global::<crate::ui::preferences::Preferences>()
                .is_some_and(|p| p.autocorrect);
        gpui::TextInputConfiguration {
            autocorrect: enabled,
            suggestions: enabled,
            ..Default::default()
        }
    }

    fn text_for_range(
        &mut self,
        r: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let r = self.range8(r);
        *actual = Some(self.range16(r.clone()));
        Some(if self.secret {
            "*".repeat(self.content[r].encode_utf16().count())
        } else {
            self.content[r].into()
        })
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range16(self.selection.clone()),
            reversed: self.reversed,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.clone().map(|r| self.range16(r))
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.emit(Changed);
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        r: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        self.manual_scroll = false;
        self.reveal_pending = self.reveal_in_parent;
        let mut r = self.replacement_range(r);
        let mut corrected = None;
        if self.inline_commands
            && !self.single_line
            && self.code.is_none()
            && self.marked.is_none()
            && r.start == r.end
            && text.len() == 1
            && text.chars().all(char::is_whitespace)
            && cx
                .try_global::<crate::ui::preferences::Preferences>()
                .is_some_and(|p| p.autocorrect)
        {
            let start = self.content[..r.start]
                .rfind(|c: char| !c.is_alphabetic())
                .map_or(0, |ix| {
                    ix + self.content[ix..].chars().next().unwrap().len_utf8()
                });
            let word = &self.content[start..r.start];
            if word.len() >= 2
                && (start == 0
                    || self.content[..start]
                        .chars()
                        .next_back()
                        .is_some_and(char::is_whitespace))
                && let Some(value) = spelling::correction(word)
            {
                r.start = start;
                corrected = Some(format!("{value}{text}"));
            }
        }
        let text = corrected.as_deref().unwrap_or(text);
        let previous_command = self
            .inline_commands
            .then(|| Command::leading(&self.content))
            .flatten();
        let normalized = self.normalized(text);
        if self.code.is_some() && !self.accept_code_edit(&r, &normalized, cx) {
            return;
        }
        let mut normalized = normalized.into_owned();
        if let Some(limit) = self.max_chars {
            let remaining = limit.saturating_sub(
                self.content[..r.start].encode_utf16().count()
                    + self.content[r.end..].encode_utf16().count(),
            );
            normalized = limit_utf16(&normalized, remaining);
        }
        self.record_undo();
        let text = bounded(
            &normalized,
            (if self.code.is_some() {
                monocode_core::files::document::MAX_TEXT_BYTES
            } else {
                MAX_BYTES
            })
            .saturating_sub(self.content.len() - (r.end - r.start)),
        );
        self.map_code_folds(&r, text.len());
        self.content = format!(
            "{}{}{}",
            &self.content[..r.start],
            text,
            &self.content[r.end..]
        )
        .into();
        let end = r.start + text.len();
        self.code_changed();
        self.selection = end..end;
        self.reversed = false;
        self.marked = None;
        self.command_removed(previous_command, cx);
        cx.emit(Changed);
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        r: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        self.manual_scroll = false;
        self.reveal_pending = self.reveal_in_parent;
        let r = self.replacement_range(r);
        let normalized = self.normalized(text);
        if self.code.is_some() && !self.accept_code_edit(&r, &normalized, cx) {
            return;
        }
        let mut normalized = normalized.into_owned();
        if let Some(limit) = self.max_chars {
            let remaining = limit.saturating_sub(
                self.content[..r.start].encode_utf16().count()
                    + self.content[r.end..].encode_utf16().count(),
            );
            normalized = limit_utf16(&normalized, remaining);
        }
        self.record_undo();
        let text = bounded(
            &normalized,
            (if self.code.is_some() {
                monocode_core::files::document::MAX_TEXT_BYTES
            } else {
                MAX_BYTES
            })
            .saturating_sub(self.content.len() - (r.end - r.start)),
        );
        self.map_code_folds(&r, text.len());
        self.content = format!(
            "{}{}{}",
            &self.content[..r.start],
            text,
            &self.content[r.end..]
        )
        .into();
        let text = text.to_owned();
        self.code_changed();
        self.marked = (!text.is_empty()).then_some(r.start..r.start + text.len());
        let selection = selection
            .map(|s| r.start + from16(&text, s.start)..r.start + from16(&text, s.end))
            .unwrap_or(r.start + text.len()..r.start + text.len());
        self.selection = self.clip_range(selection);
        self.reversed = false;
        cx.emit(Changed);
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        r: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let r = self.range8(r);
        let origin =
            self.bounds?.origin + self.position(r.start) - point(self.scroll_x, self.scroll_y);
        Some(Bounds::new(origin, size(px(2.), self.line_height())))
    }
    fn character_index_for_point(
        &mut self,
        p: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.to16(self.mouse_index(p)))
    }
}
