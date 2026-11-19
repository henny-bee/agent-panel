use super::*;

pub(super) fn paint_rail(
    i: &Input,
    rail: Bounds<Pixels>,
    scroll: Pixels,
    w: &mut Window,
    cx: &App,
) -> Vec<MarkerHit> {
    let code = i.code.as_ref().unwrap();
    let height = rail.size.height.as_f32();
    let total = code.total().max(1.);
    let metrics = Metrics::new(scroll.as_f32(), total, height, height);
    let mut markers = Vec::new();
    w.with_content_mask(Some(ContentMask { bounds: rail }), |w| {
        w.paint_quad(fill(
            Bounds::new(rail.origin, size(px(1.), rail.size.height)),
            crate::ui::theme::content(0.07),
        ));
        if metrics.scrollable {
            w.paint_quad(
                fill(
                    Bounds::new(
                        rail.origin + point(px(3.), px(metrics.thumb_top)),
                        size(px(WIDTH - 6.), px(metrics.thumb_height)),
                    ),
                    crate::ui::theme::content(if code.hovered || code.drag_offset.is_some() {
                        0.38 * 0.88
                    } else {
                        0.30 * 0.72
                    }),
                )
                .corner_radii(px(2.)),
            );
        }
        // Exactly one diagnostic per logical line, strongest severity wins.
        let mut diagnostics = std::collections::BTreeMap::new();
        for diagnostic in &code.analysis.diagnostics {
            let row = code.row(diagnostic.from);
            let current = diagnostics.entry(row).or_insert(diagnostic);
            if diagnostic.priority() > current.priority() {
                *current = diagnostic;
            }
        }
        for (row, diagnostic) in diagnostics {
            let bounds = Bounds::new(
                rail.origin
                    + point(
                        px(3.),
                        px(row as f32 / code.offsets.len().max(1) as f32 * height),
                    ),
                size(px(WIDTH - 5.), px(3.)),
            );
            let color = match diagnostic.severity.as_str() {
                "error" => 0xf87171,
                "warning" => 0xfbbf24,
                "info" => 0x60a5fa,
                _ => 0x969696,
            };
            let ink = if diagnostic.severity == "hint" {
                crate::ui::theme::content(0.48)
            } else {
                rgb(color).into()
            };
            w.paint_quad(fill(bounds, ink).corner_radii(px(1.)));
            markers.push(MarkerHit {
                bounds,
                pos: diagnostic.from,
                diagnostic: true,
                message: diagnostic.message.clone(),
            });
        }
        let cursor_y = code.row(i.cursor()) as f32 / code.offsets.len().max(1) as f32 * height;
        let mut cursor_color = crate::ui::theme::accent_color();
        cursor_color.a *= if i.focus.is_focused(w) { 1. } else { 0.6 };
        w.paint_quad(fill(
            Bounds::new(
                rail.origin + point(px(0.), px(cursor_y)),
                size(rail.size.width, px(2.)),
            ),
            cursor_color,
        ));
        let (add, del) = crate::editor::scrollbar::diff_colors(cx);
        // Git's layer is above the cursor/diagnostic layer in editorGit.ts.
        for change in &code.analysis.changes {
            let row = code.row(change.from);
            let top = code.top(row, i.line_height);
            let bottom = if change.kind == "del" {
                top + change.deleted.max(1) as f32 * i.line_height
            } else {
                code.top(code.row(change.to.saturating_sub(1)) + 1, i.line_height)
            };
            let bounds = Bounds::new(
                rail.origin + point(px(3.), px(top / total * height)),
                size(
                    px(WIDTH - 5.),
                    px(((bottom - top).max(0.) / total * height).max(3.)),
                ),
            );
            if change.kind == "mod" {
                let half = bounds.size.width / 2.;
                w.paint_quad(
                    fill(
                        Bounds::new(bounds.origin, size(half, bounds.size.height)),
                        rgb(del),
                    )
                    .corner_radii(px(1.)),
                );
                w.paint_quad(
                    fill(
                        Bounds::new(
                            bounds.origin + point(half, px(0.)),
                            size(half, bounds.size.height),
                        ),
                        rgb(add),
                    )
                    .corner_radii(px(1.)),
                );
            } else {
                w.paint_quad(
                    fill(bounds, rgb(if change.kind == "del" { del } else { add }))
                        .corner_radii(px(1.)),
                );
            }
            markers.push(MarkerHit {
                bounds,
                pos: change.from,
                diagnostic: false,
                message: String::new(),
            });
        }
    });
    markers
}

pub(crate) struct CodeElement {
    pub input: Entity<Input>,
}

impl IntoElement for CodeElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

pub(super) fn plain_line(
    text: &str,
    width: Option<Pixels>,
    font_size: f32,
    w: &mut Window,
    color: Hsla,
) -> WrappedLine {
    w.text_system()
        .shape_text(
            text.to_owned().into(),
            px(font_size),
            &[TextRun {
                len: text.len(),
                font: gpui::font(crate::ui::fonts::mono()),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            width,
            None,
        )
        .unwrap()
        .remove(0)
}

pub(super) fn token_line(
    text: &str,
    tokens: &[crate::editor::analysis::Token],
    width: Option<Pixels>,
    font_size: f32,
    w: &mut Window,
) -> WrappedLine {
    let base = TextRun {
        len: 0,
        font: gpui::font(crate::ui::fonts::mono()),
        color: crate::ui::theme::content(1.),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let mut runs = Vec::new();
    let mut at = 0;
    for token in tokens {
        if token.from > at {
            runs.push(TextRun {
                len: token.from - at,
                ..base.clone()
            });
        }
        runs.push(TextRun {
            len: token.to - token.from,
            color: crate::ui::theme::syntax_color(&token.kind),
            ..base.clone()
        });
        at = token.to;
    }
    if at < text.len() || runs.is_empty() {
        runs.push(TextRun {
            len: text.len() - at,
            ..base
        });
    }
    w.text_system()
        .shape_text(text.to_owned().into(), px(font_size), &runs, width, None)
        .unwrap()
        .remove(0)
}

pub(super) fn shape(i: &Input, row: usize, width: Option<Pixels>, w: &mut Window) -> WrappedLine {
    let code = i.code.as_ref().unwrap();
    let start = code.offsets[row];
    let end = code.source_end(row, i.content.len());
    let text: SharedString = if let Some(fold) = code.fold_at(row) {
        format!(
            "{}…{}",
            &i.content[start..fold.from],
            &i.content[fold.to..end]
        )
        .into()
    } else {
        i.content[start..end].to_string().into()
    };
    let base = TextRun {
        len: text.len(),
        font: gpui::font(crate::ui::fonts::mono()),
        color: crate::ui::theme::content(1.),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let mut runs = Vec::new();
    let tokens = &code.analysis.tokens;
    let first = tokens.partition_point(|t| t.to <= start);
    let mut spans = Vec::new();
    for token in tokens.iter().skip(first).take_while(|t| t.from < end) {
        let from = token.from.max(start);
        let to = token.to.min(end);
        let color = crate::ui::theme::syntax_color(&token.kind);
        if let Some(fold) = code.fold_at(row) {
            if from < fold.from {
                spans.push((from - start, to.min(fold.from) - start, color));
            }
            if to > fold.to {
                spans.push((
                    code.display_offset(row, from.max(fold.to)),
                    code.display_offset(row, to),
                    color,
                ));
            }
        } else {
            spans.push((from - start, to - start, color));
        }
    }
    if let Some(fold) = code.fold_at(row) {
        spans.push((
            fold.from - start,
            fold.from - start + "…".len(),
            crate::ui::theme::content(0.4),
        ));
    }
    spans.sort_by_key(|s| s.0);
    let mut at = 0;
    for (from, to, color) in spans {
        if from > at {
            runs.push(TextRun {
                len: from - at,
                ..base.clone()
            });
        }
        if to > from {
            runs.push(TextRun {
                len: to - from,
                color,
                ..base.clone()
            });
        }
        at = to;
    }
    if at < text.len() || runs.is_empty() {
        runs.push(TextRun {
            len: text.len() - at,
            ..base
        });
    }
    if let Some(marked) = &i.marked {
        let marked = marked.start.saturating_sub(start).min(text.len())
            ..marked.end.saturating_sub(start).min(text.len());
        if !marked.is_empty() {
            let mut at = 0;
            let mut decorated = Vec::new();
            for run in runs {
                let end = at + run.len;
                let a = marked.start.clamp(at, end);
                let b = marked.end.clamp(at, end);
                if at < a {
                    decorated.push(TextRun {
                        len: a - at,
                        ..run.clone()
                    });
                }
                if a < b {
                    decorated.push(TextRun {
                        len: b - a,
                        underline: Some(UnderlineStyle {
                            color: Some(run.color),
                            thickness: px(1.),
                            wavy: false,
                        }),
                        ..run.clone()
                    });
                }
                if b < end {
                    decorated.push(TextRun {
                        len: end - b,
                        ..run
                    });
                }
                at = end;
            }
            runs = decorated;
        }
    }
    for diagnostic in code
        .analysis
        .diagnostics
        .iter()
        .filter(|_| code.fold_at(row).is_none())
    {
        let range = diagnostic.range();
        if range.end <= start || range.start >= end {
            continue;
        }
        let from = range.start.saturating_sub(start).min(text.len());
        let to = range.end.saturating_sub(start).min(text.len());
        let mut at = 0;
        let mut decorated = Vec::new();
        for run in runs {
            let end = at + run.len;
            let a = from.clamp(at, end);
            let b = to.clamp(at, end);
            if at < a {
                decorated.push(TextRun {
                    len: a - at,
                    ..run.clone()
                });
            }
            if a < b {
                decorated.push(TextRun {
                    len: b - a,
                    underline: Some(UnderlineStyle {
                        color: Some(rgb(0xf87171).into()),
                        thickness: px(1.),
                        wavy: true,
                    }),
                    ..run.clone()
                });
            }
            if b < end {
                decorated.push(TextRun {
                    len: end - b,
                    ..run
                });
            }
            at = end;
        }
        runs = decorated;
    }
    w.text_system()
        .shape_text(text, px(i.font_size), &runs, width, None)
        .unwrap()
        .remove(0)
}

impl Element for CodeElement {
    type RequestLayoutState = ();
    type PrepaintState = CodePaint;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        w: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (w.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        w: &mut Window,
        cx: &mut App,
    ) -> CodePaint {
        // Measure wrapped block heights only when the document or pane width
        // changes. Short lines use a conservative UTF-8 byte-width bound;
        // offscreen lines do not retain glyph runs.
        self.input.update(cx, |i, _| {
            let code = i.code.as_ref().unwrap();
            let gutter = px(code.gutter());
            let width = (bounds.size.width - gutter - px(WIDTH + 12.)).max(px(1.));
            if code.layout_width != Some(width) {
                let mut tops = Vec::with_capacity(code.offsets.len() + 1);
                let mut y = TOP_PADDING;
                let mut deleted_rows = Vec::new();
                let mut deletion_changes = code
                    .analysis
                    .changes
                    .iter()
                    .filter(|c| c.kind != "add")
                    .peekable();
                let sample = w.text_system().shape_line(
                    "M".into(),
                    px(i.font_size),
                    &[TextRun {
                        len: 1,
                        font: gpui::font(crate::ui::fonts::mono()),
                        color: crate::ui::theme::content(1.),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    None,
                );
                for row in 0..code.offsets.len() {
                    while deletion_changes
                        .peek()
                        .is_some_and(|c| code.row(c.from) < row)
                    {
                        deletion_changes.next();
                    }
                    if !code.hidden(row) && code.diff {
                        while deletion_changes
                            .peek()
                            .is_some_and(|c| code.row(c.from) == row)
                        {
                            let change = deletion_changes.next().unwrap();
                            let mut offset = 0;
                            for (n, text) in change
                                .original
                                .strip_suffix('\n')
                                .unwrap_or(&change.original)
                                .split('\n')
                                .enumerate()
                            {
                                let tokens: Vec<_> = change
                                    .tokens
                                    .iter()
                                    .filter(|t| t.to > offset && t.from < offset + text.len())
                                    .map(|t| crate::editor::analysis::Token {
                                        from: t.from.saturating_sub(offset),
                                        to: t.to.min(offset + text.len()) - offset,
                                        kind: t.kind.clone(),
                                    })
                                    .collect();
                                offset += text.len() + 1;
                                let line = token_line(
                                    text,
                                    &tokens,
                                    code.wrap.then_some(width),
                                    i.font_size,
                                    w,
                                );
                                let height = line.size(i.line_height()).height.as_f32();
                                deleted_rows.push((
                                    y,
                                    height,
                                    text.to_owned(),
                                    change.old_line + n,
                                    tokens,
                                ));
                                y += height;
                            }
                        }
                    }
                    tops.push(y);
                    if code.hidden(row) {
                        continue;
                    }
                    let start = code.offsets[row];
                    let end = code.offsets.get(row + 1).map_or(i.content.len(), |n| n - 1);
                    let text = &i.content[start..end];
                    let short = !text.contains('\t') && sample.width * text.len() <= width;
                    y += if !code.wrap || short {
                        i.line_height
                    } else {
                        shape(i, row, Some(width), w)
                            .size(i.line_height())
                            .height
                            .as_f32()
                    };
                }
                tops.push(y);
                let code = i.code.as_mut().unwrap();
                code.tops = tops;
                code.deleted_rows = deleted_rows;
                code.layout_width = Some(width);
            }
        });
        let i = self.input.read(cx);
        let code = i.code.as_ref().unwrap();
        let gutter = px(code.gutter());
        let text_bounds = Bounds::new(
            bounds.origin + point(gutter, px(0.)),
            size(
                (bounds.size.width - gutter - px(WIDTH + 12.)).max(px(1.)),
                bounds.size.height,
            ),
        );
        let wrap = code.wrap.then_some(text_bounds.size.width);
        let mut scroll = point(i.scroll_x, i.scroll_y);
        if code.reveal {
            let row = code.display_row(i.cursor());
            let caret_line = shape(i, row, wrap, w);
            let p = line_position(
                &caret_line,
                code.display_offset(row, i.cursor()),
                i.line_height(),
                code.affinity.get() == Some((i.cursor(), true)),
            );
            let y = px(code.top(row, i.line_height)) + p.y;
            if y < scroll.y {
                scroll.y = y;
            }
            if y + i.line_height() > scroll.y + bounds.size.height {
                scroll.y = y + i.line_height() - bounds.size.height;
            }
            if p.x < scroll.x {
                scroll.x = p.x;
            }
            if p.x + px(8.) > scroll.x + text_bounds.size.width {
                scroll.x = p.x + px(8.) - text_bounds.size.width;
            }
        }
        if code.wrap {
            scroll.x = px(0.);
        }
        scroll.y = scroll
            .y
            .max(px(0.))
            .min((px(code.total()) - bounds.size.height).max(px(0.)));
        let start = code.row_at_y(scroll.y.as_f32());
        let end =
            (code.row_at_y((scroll.y + bounds.size.height).as_f32()) + 2).min(code.offsets.len());
        let lines = (start..end)
            .filter(|row| !code.hidden(*row))
            .map(|row| (row, shape(i, row, wrap, w)))
            .collect();
        let deleted = code
            .deleted_rows
            .iter()
            .filter(|(top, height, _, _, _)| {
                *top + *height >= scroll.y.as_f32()
                    && *top < (scroll.y + bounds.size.height).as_f32()
            })
            .map(|(top, _, text, number, tokens)| {
                (
                    *top,
                    token_line(text, tokens, wrap, i.font_size, w),
                    *number,
                )
            })
            .collect();
        CodePaint {
            lines,
            deleted,
            scroll,
            text_bounds,
        }
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        state: &mut CodePaint,
        w: &mut Window,
        cx: &mut App,
    ) {
        paint_drag_listener(&self.input, w, cx);
        let i = self.input.read(cx);
        let code = i.code.as_ref().unwrap();
        let lh = i.line_height();
        let cursor = i.cursor();
        let selected = i.selection.clone();
        let focused = i.focus.is_focused(w);
        let line_height = i.line_height;
        let code_affinity = code.affinity.get();
        let starts: Vec<_> = state
            .lines
            .iter()
            .map(|(row, _)| code.offsets[*row])
            .collect();
        let tops: Vec<_> = state
            .lines
            .iter()
            .map(|(row, _)| code.top(*row, line_height))
            .collect();
        let rail = Bounds::new(
            point(bounds.right() - px(WIDTH), bounds.top()),
            size(px(WIDTH), bounds.size.height),
        );
        let markers = paint_rail(i, rail, state.scroll.y, w, cx);
        let content_len = i.content.len();
        let mut snapshot = Code::new();
        snapshot.offsets = code.offsets.clone();
        snapshot.tops = code.tops.clone();
        snapshot.folded = code.folded.clone();
        snapshot.analysis = code.analysis.clone();
        snapshot.search_matches = code.search_matches.clone();
        snapshot.diff = code.diff;
        let code = &snapshot;
        w.handle_input(
            &i.focus,
            ElementInputHandler::new(state.text_bounds, self.input.clone()),
            cx,
        );
        w.with_content_mask(Some(ContentMask { bounds }), |w| {
            if code.diff {
                for (top, line, number) in &state.deleted {
                    let y = bounds.top() + px(*top) - state.scroll.y;
                    w.paint_quad(fill(
                        Bounds::new(
                            point(bounds.left(), y),
                            size(bounds.size.width - px(WIDTH), line.size(lh).height),
                        ),
                        crate::ui::theme::diff_background(false, false),
                    ));
                    w.paint_quad(fill(
                        Bounds::new(
                            point(bounds.left(), y),
                            size(px(code.gutter() - 22.), line.size(lh).height),
                        ),
                        crate::ui::theme::diff_background(false, true),
                    ));
                    let number = plain_line(
                        &number.to_string(),
                        None,
                        11.,
                        w,
                        crate::ui::theme::diff_foreground(false),
                    );
                    let _ = number.paint(
                        point(state.text_bounds.left() - px(26.) - number.width(), y),
                        lh,
                        TextAlign::Left,
                        None,
                        w,
                        cx,
                    );
                    let minus =
                        plain_line("−", None, 12., w, crate::ui::theme::diff_foreground(false));
                    let _ = minus.paint(
                        point(state.text_bounds.left() - px(12.), y),
                        lh,
                        TextAlign::Left,
                        None,
                        w,
                        cx,
                    );
                }
                for ((row, line), top) in state.lines.iter().zip(&tops) {
                    let start = code.offsets[*row];
                    if code
                        .analysis
                        .changes
                        .iter()
                        .any(|c| c.kind != "del" && start >= c.from && start <= c.to)
                    {
                        let y = bounds.top() + px(*top) - state.scroll.y;
                        w.paint_quad(fill(
                            Bounds::new(
                                point(bounds.left(), y),
                                size(bounds.size.width - px(WIDTH), line.size(lh).height),
                            ),
                            crate::ui::theme::diff_background(true, false),
                        ));
                        let plus =
                            plain_line("+", None, 12., w, crate::ui::theme::diff_foreground(true));
                        let _ = plus.paint(
                            point(state.text_bounds.left() - px(12.), y),
                            lh,
                            TextAlign::Left,
                            None,
                            w,
                            cx,
                        );
                    }
                }
            }
            for ((row, shaped), top) in state.lines.iter().zip(&tops) {
                if code.analysis.folds.iter().any(|f| code.row(f.from) == *row) {
                    let arrow: SharedString = if code.fold_at(*row).is_some() {
                        "›"
                    } else {
                        "⌄"
                    }
                    .into();
                    let arrow_line = w.text_system().shape_line(
                        arrow,
                        px(12.),
                        &[TextRun {
                            len: if code.fold_at(*row).is_some() {
                                "›".len()
                            } else {
                                "⌄".len()
                            },
                            font: gpui::font(crate::ui::fonts::mono()),
                            color: crate::ui::theme::content(0.45),
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        }],
                        None,
                    );
                    let _ = arrow_line.paint(
                        point(
                            state.text_bounds.left() - px(12.),
                            bounds.top() + px(*top) - state.scroll.y,
                        ),
                        lh,
                        TextAlign::Left,
                        None,
                        w,
                        cx,
                    );
                }
                let _ = shaped;
                let number: SharedString = (row + 1).to_string().into();
                let len = number.len();
                let line = w.text_system().shape_line(
                    number,
                    px(11.),
                    &[TextRun {
                        len,
                        font: gpui::font(crate::ui::fonts::mono()),
                        color: crate::ui::theme::content(0.3),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    None,
                );
                let _ = line.paint(
                    point(
                        state.text_bounds.left() - px(26.) - line.width,
                        bounds.top() + px(*top) - state.scroll.y,
                    ),
                    lh,
                    TextAlign::Left,
                    None,
                    w,
                    cx,
                );
            }
        });
        w.with_content_mask(
            Some(ContentMask {
                bounds: state.text_bounds,
            }),
            |w| {
                for (top, line, _) in &state.deleted {
                    let _ = line.paint(
                        point(
                            state.text_bounds.left() - state.scroll.x,
                            bounds.top() + px(*top) - state.scroll.y,
                        ),
                        lh,
                        TextAlign::Left,
                        None,
                        w,
                        cx,
                    );
                }
                for (((row, line), &start), top) in state.lines.iter().zip(&starts).zip(&tops) {
                    let y = bounds.top() + px(*top) - state.scroll.y;
                    let x = state.text_bounds.left() - state.scroll.x;
                    if cursor >= start && cursor <= code.source_end(*row, content_len) {
                        w.paint_quad(fill(
                            Bounds::new(
                                point(state.text_bounds.left(), y),
                                size(state.text_bounds.size.width, line.size(lh).height),
                            ),
                            crate::ui::theme::content(0.1),
                        ));
                    }
                    for range in &code.search_matches {
                        let a = code.display_offset(*row, range.start).min(line.len());
                        let b = code.display_offset(*row, range.end).min(line.len());
                        if a < b {
                            let a = line_position(line, a, lh, false);
                            let b = line.position_for_index(b, lh).unwrap_or_default();
                            if a.y == b.y {
                                w.paint_quad(fill(
                                    Bounds::new(
                                        point(x + a.x, y + a.y),
                                        size((b.x - a.x).max(px(2.)), lh),
                                    ),
                                    rgb(0xe2c08d).opacity(0.46),
                                ));
                            }
                        }
                    }
                    let a = code.display_offset(*row, selected.start).min(line.len());
                    let b = code.display_offset(*row, selected.end).min(line.len());
                    if a < b
                        || (selected.start <= start + line.len()
                            && selected.end > start + line.len())
                    {
                        let a = line_position(line, a, lh, false);
                        let b = line.position_for_index(b, lh).unwrap_or_default();
                        let mut sy = a.y;
                        while sy <= b.y {
                            let left = if sy == a.y { a.x } else { px(0.) };
                            let right = if sy == b.y && selected.end <= start + line.len() {
                                b.x
                            } else {
                                state.text_bounds.size.width
                            };
                            w.paint_quad(fill(
                                Bounds::new(
                                    point(x + left, y + sy),
                                    size((right - left).max(px(2.)), lh),
                                ),
                                crate::ui::theme::text_selection(),
                            ));
                            sy += lh;
                        }
                    }
                    let _ = line.paint(point(x, y), lh, TextAlign::Left, None, w, cx);
                    if focused
                        && selected.is_empty()
                        && cursor >= start
                        && cursor <= code.source_end(*row, content_len)
                    {
                        let p = line_position(
                            line,
                            code.display_offset(*row, cursor),
                            lh,
                            code_affinity == Some((cursor, true)),
                        );
                        w.paint_quad(fill(
                            Bounds::new(point(x + p.x, y + p.y), size(px(1.5), lh)),
                            crate::ui::theme::content(1.),
                        ));
                    }
                }
            },
        );
        self.input.update(cx, |i, _| {
            i.bounds = Some(state.text_bounds);
            i.scroll_x = state.scroll.x;
            i.scroll_y = state.scroll.y;
            let code = i.code.as_mut().unwrap();
            code.reveal = false;
            code.rail = Some(rail);
            code.markers = markers;
            for (_, line) in &state.lines {
                code.max_width = code
                    .max_width
                    .max(line.width() - state.text_bounds.size.width + px(20.));
            }
            code.visible = std::mem::take(&mut state.lines);
        });
    }
}
