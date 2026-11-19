use super::*;

impl Input {
    pub(crate) fn unfold_range(&mut self, range: &Range<usize>) {
        if let Some(code) = &mut self.code {
            let before = code.folded.len();
            code.folded.retain(|f| {
                !(range.start > f.from && range.start < f.to
                    || range.end > f.from && range.end < f.to)
            });
            if code.folded.len() != before {
                code.layout_width = None;
            }
        }
    }

    pub(crate) fn schedule_completion(&mut self, explicit: bool, cx: &mut Context<Self>) {
        if self.is_composing() || !self.selection.is_empty() {
            return;
        }
        let at = self.cursor();
        if !explicit
            && !self.content[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.')
        {
            return;
        }
        let Some(code) = &mut self.code else {
            return;
        };
        if !explicit && std::mem::take(&mut code.suppress_completion) {
            return;
        }
        let source = self.content.clone();
        let path = code.path.clone();
        code.completion_source = Some((source.clone(), at));
        code.completion_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(if explicit {
                    0
                } else {
                    100
                }))
                .await;
            let text = source.clone();
            let result = cx
                .background_executor()
                .spawn(async move { crate::editor::analysis::complete(&path, &text, at, explicit) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.content == source
                    && this.cursor() == at
                    && !this.is_composing()
                    && let Some(code) = &mut this.code
                    && code.completion_source.as_ref() == Some(&(source, at))
                {
                    code.completions = result.unwrap_or_default();
                    code.completion_index = 0;
                    cx.notify();
                }
            });
        }));
    }

    pub(crate) fn clear_completion(&mut self) {
        if let Some(code) = &mut self.code {
            code.completions.clear();
            code.completion_task = None;
            code.completion_source = None;
        }
    }

    pub(crate) fn start_completion(
        &mut self,
        _: &Complete,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.schedule_completion(true, cx);
    }

    pub(crate) fn accept_completion(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(code) = &self.code else {
            return false;
        };
        if self.is_composing()
            || code.completion_source.as_ref() != Some(&(self.content.clone(), self.cursor()))
        {
            return false;
        }
        let Some(option) = code.completions.get(code.completion_index).cloned() else {
            return false;
        };
        self.replace_bytes(option.from..option.to, &option.insert, option.cursor, cx);
        true
    }

    pub(crate) fn completion_step(&mut self, backwards: bool, cx: &mut Context<Self>) -> bool {
        let Some(code) = &mut self.code else {
            return false;
        };
        if code.completions.is_empty() {
            return false;
        }
        code.completion_index = (code.completion_index
            + code.completions.len()
            + if backwards {
                code.completions.len() - 1
            } else {
                1
            })
            % code.completions.len();
        cx.notify();
        true
    }

    pub(crate) fn dismiss_completion(
        &mut self,
        _: &DismissCompletion,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(code) = &mut self.code else {
            cx.propagate();
            return;
        };
        if code.completions.is_empty() && code.completion_task.is_none() {
            cx.propagate();
            return;
        }
        code.completions.clear();
        code.completion_task = None;
        code.completion_source = None;
        cx.notify();
    }

    pub(super) fn expand_emmet(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(code) = &self.code else {
            return false;
        };
        let path = code.path.clone();
        // Expansion is requested explicitly with Tab. The same bounded worker
        // service is used here, and stale results never replace newer typing.
        let source = self.content.clone();
        let at = self.cursor();
        if !matches!(
            std::path::Path::new(&path)
                .extension()
                .and_then(|s| s.to_str()),
            Some("html" | "htm" | "jsx" | "tsx" | "md" | "mdx" | "markdown")
        ) {
            return false;
        }
        let token = source[..at].split_whitespace().last().unwrap_or("");
        if token.is_empty() || token.contains(['<', '=', ';', '"', '\'']) {
            return false;
        }
        cx.spawn(async move |this, cx| {
            let text = source.clone();
            let result = cx
                .background_executor()
                .spawn(async move { crate::editor::analysis::emmet(&path, &text, at) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.content == source && this.cursor() == at && !this.is_composing() {
                    if let Ok(Some(result)) = result {
                        this.replace_bytes(0..source.len(), &result.formatted, result.cursor, cx);
                    } else {
                        this.replace_bytes(at..at, "  ", 2, cx);
                    }
                }
            });
        })
        .detach();
        true
    }

    pub(crate) fn completion_popup(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let code = self.code.as_ref()?;
        if code.completions.is_empty()
            || code.completion_source.as_ref() != Some(&(self.content.clone(), self.cursor()))
        {
            return None;
        }
        let p = self.code_position(self.cursor());
        let gutter = code.gutter();
        let height = (code.completions.len().min(10) as f32 * 22.).min(240.);
        let viewport = self.bounds.map_or(400., |b| b.size.height.as_f32());
        let y = (p.y - self.scroll_y).as_f32();
        let top = if y + self.line_height + height > viewport {
            (y - height).max(0.)
        } else {
            y + self.line_height
        };
        let first = code.completion_index.saturating_sub(9);
        let viewport_width = self.bounds.map_or(400., |b| b.size.width.as_f32() + gutter);
        let width = code
            .completions
            .iter()
            .map(|o| (o.label.chars().count() + o.detail.chars().count()) as f32 * 7.2 + 44.)
            .fold(180., f32::max)
            .min(400.)
            .min((viewport_width - 8.).max(1.));
        let left = (gutter + (p.x - self.scroll_x).as_f32())
            .clamp(4., (viewport_width - width - 4.).max(4.));
        Some(
            div()
                .id("editor-completions")
                .role(Role::List)
                .aria_label("Autocomplete")
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(width))
                .rounded(px(6.))
                .border_1()
                .border_color(crate::ui::theme::content(0.12))
                .bg(crate::ui::theme::shell_fill().opacity(1.))
                .shadow_md()
                .overflow_hidden()
                .py_1()
                .children(
                    code.completions
                        .iter()
                        .enumerate()
                        .skip(first)
                        .take(10)
                        .map(|(n, option)| {
                            div()
                                .id(("completion", n))
                                .role(Role::ListItem)
                                .aria_label(option.label.clone())
                                .px_2()
                                .h(px(22.))
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_size(px(12.))
                                .text_color(crate::ui::theme::content(1.))
                                .when(n == code.completion_index, |d| {
                                    d.bg(crate::ui::theme::accent_color().opacity(0.25))
                                })
                                .child(div().text_color(crate::ui::theme::content(0.5)).child(
                                    match option.kind.as_str() {
                                        "function" => "ƒ",
                                        "keyword" => "k",
                                        "variable" => "v",
                                        _ => "·",
                                    },
                                ))
                                .child(option.label.clone())
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .text_color(crate::ui::theme::content(0.5))
                                        .child(option.detail.clone()),
                                )
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(code) = &mut this.code {
                                        code.completion_index = n;
                                    }
                                    this.accept_completion(cx);
                                }))
                        }),
                )
                .into_any_element(),
        )
    }

    pub(super) fn toggle_fold_row(&mut self, row: usize, cx: &mut Context<Self>) {
        let Some(code) = &mut self.code else {
            return;
        };
        if let Some(n) = code.folded.iter().position(|f| code.row(f.from) == row) {
            code.folded.remove(n);
        } else if let Some(fold) = code
            .analysis
            .folds
            .iter()
            .find(|f| code.row(f.from) == row)
            .cloned()
        {
            if self.selection.end > fold.from && self.selection.start < fold.to {
                self.selection = fold.from..fold.from;
            }
            code.folded.push(fold);
        }
        code.layout_width = None;
        cx.notify();
    }

    pub(crate) fn fold(&mut self, _: &FoldCode, _: &mut Window, cx: &mut Context<Self>) {
        let Some(code) = &self.code else {
            cx.propagate();
            return;
        };
        let row = code.row(self.cursor());
        if !code.folded.iter().any(|f| code.row(f.from) == row) {
            self.toggle_fold_row(row, cx);
        }
    }

    pub(crate) fn unfold(&mut self, _: &UnfoldCode, _: &mut Window, cx: &mut Context<Self>) {
        let Some(code) = &self.code else {
            cx.propagate();
            return;
        };
        let row = code.display_row(self.cursor());
        if code.fold_at(row).is_some() {
            self.toggle_fold_row(row, cx);
        }
    }

    pub(crate) fn fold_all(&mut self, _: &FoldAll, _: &mut Window, cx: &mut Context<Self>) {
        let Some(code) = &mut self.code else {
            cx.propagate();
            return;
        };
        code.folded = code.analysis.folds.clone();
        code.folded.sort_by_key(|f| f.from);
        let mut end = 0;
        code.folded.retain(|f| {
            if f.from < end {
                false
            } else {
                end = f.to;
                true
            }
        });
        code.layout_width = None;
        self.selection = 0..0;
        cx.notify();
    }

    pub(crate) fn unfold_all(&mut self, _: &UnfoldAll, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(code) = &mut self.code {
            code.folded.clear();
            code.layout_width = None;
            cx.notify();
        }
    }

    pub(crate) fn code_fold_down(&mut self, e: &MouseDownEvent, cx: &mut Context<Self>) -> bool {
        let (Some(code), Some(bounds)) = (&self.code, self.bounds) else {
            return false;
        };
        if e.position.x < bounds.left() - px(14.)
            || e.position.x >= bounds.left()
            || e.position.y < bounds.top()
            || e.position.y > bounds.bottom()
        {
            return false;
        }
        let row = code.row_at_y((e.position.y - bounds.top() + self.scroll_y).as_f32());
        self.toggle_fold_row(row, cx);
        true
    }

    pub(crate) fn map_code_folds(&mut self, range: &Range<usize>, inserted: usize) {
        if let Some(code) = &mut self.code {
            code.folded.retain_mut(|f| {
                if range.end <= f.from {
                    f.from = f.from + inserted - range.len();
                    f.to = f.to + inserted - range.len();
                    true
                } else {
                    range.start >= f.to
                }
            });
        }
    }

    pub(crate) fn clear_folds(&mut self) {
        if let Some(code) = &mut self.code {
            code.folded.clear();
            code.layout_width = None;
        }
    }

    pub(crate) fn folded_boundary(&self, at: usize, backwards: bool) -> Option<usize> {
        self.code.as_ref()?.folded.iter().find_map(|f| {
            if backwards && at == f.to {
                Some(f.from)
            } else if !backwards && at == f.from {
                Some(f.to)
            } else {
                None
            }
        })
    }
}
