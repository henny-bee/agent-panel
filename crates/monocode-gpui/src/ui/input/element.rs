use super::*;

pub(super) struct InputElement {
    pub(super) input: Entity<Input>,
}

impl IntoElement for InputElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for InputElement {
    type RequestLayoutState = ();
    type PrepaintState = PaintState;
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
        let i = self.input.read(cx);
        if i.single_line || i.content.is_empty() {
            style.size.height = px(i.height).into();
            return (w.request_layout(style, [], cx), ());
        }
        // Grow with the wrapped draft, like the auto-resizing textarea.
        let (text, _) = i.display_text();
        let font_size = px(i.font_size);
        let (line_height, min_height, max_height) = (i.line_height, i.min_height, i.max_height);
        let font = w.text_style().font();
        let layout = w.request_measured_layout(style, move |known, available, w, _| {
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(width) => Some(width),
                _ => None,
            });
            let run = TextRun {
                len: text.len(),
                font: font.clone(),
                color: Hsla::default(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let height = w
                .text_system()
                .shape_text(text.clone(), font_size, &[run], width, None)
                .map(|lines| {
                    lines
                        .iter()
                        .map(|line| line.size(px(line_height)).height)
                        .fold(px(0.), |a, b| a + b)
                })
                .unwrap_or_default();
            size(
                width.unwrap_or_default(),
                height.max(px(min_height)).min(px(max_height)),
            )
        });
        (layout, ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        w: &mut Window,
        cx: &mut App,
    ) -> PaintState {
        let i = self.input.read(cx);
        let style = w.text_style();
        let placeholder = i.content.is_empty();
        let (text, layout) = i.display_text();
        let command = layout.command;
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color: match (placeholder, i.placeholder_alpha) {
                // Re-ink every paint so a theme switch never strands the
                // placeholder in the previous theme's color.
                (true, Some(alpha)) => crate::ui::theme::content(alpha),
                // `.composer-field::placeholder`: content ink at 40%.
                (true, None) if !i.single_line => crate::ui::theme::content(0.4),
                _ => style.color,
            },
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let to_display = |at| layout.to_display(at);
        // Mentions read as plain ink beside their icon; other tokens take the accent.
        let highlights: Vec<_> = if placeholder {
            Vec::new()
        } else {
            i.composer_tokens
                .iter()
                .filter(|range| {
                    range.end <= i.content.len()
                        && i.content.is_char_boundary(range.start)
                        && i.content.is_char_boundary(range.end)
                })
                .map(|range| {
                    let mention = layout.mentions.iter().any(|(at, _)| *at == range.start);
                    (to_display(range.start)..to_display(range.end), mention)
                })
                .collect()
        };
        let marked = i
            .marked
            .as_ref()
            .map(|range| to_display(range.start)..to_display(range.end));
        let mut boundaries = vec![0, text.len()];
        for range in highlights
            .iter()
            .map(|(range, _)| range)
            .chain(marked.iter())
        {
            boundaries.extend([range.start, range.end]);
        }
        if let Some(command) = command {
            boundaries.extend([command.icon_range().end, command.name_range().end]);
        }
        boundaries.sort_unstable();
        boundaries.dedup();
        let runs: Vec<_> = boundaries
            .windows(2)
            .filter(|pair| pair[0] < pair[1])
            .map(|pair| {
                let at = pair[0];
                let color = if command.is_some_and(|c| c.icon_range().contains(&at)) {
                    crate::ui::theme::content(0.)
                } else if let Some(command) = command.filter(|c| c.name_range().contains(&at)) {
                    command.mode.color()
                } else if let Some((_, mention)) =
                    highlights.iter().find(|(range, _)| range.contains(&at))
                {
                    if *mention {
                        crate::ui::theme::content(1.)
                    } else {
                        crate::ui::theme::accent_color()
                    }
                } else {
                    run.color
                };
                TextRun {
                    len: pair[1] - at,
                    color,
                    underline: marked
                        .as_ref()
                        .filter(|range| range.contains(&at))
                        .map(|_| UnderlineStyle {
                            color: Some(style.color),
                            thickness: px(1.),
                            wavy: false,
                        }),
                    ..run.clone()
                }
            })
            .collect();
        let lines: Vec<_> = w
            .text_system()
            .shape_text(
                text,
                px(i.font_size),
                &runs,
                (!i.single_line).then_some(bounds.size.width),
                None,
            )
            .unwrap_or_default()
            .into_iter()
            .collect();
        // Compute caret position against the new layout before choosing scroll.
        let cursor = to_display(i.cursor());
        let mut base = 0;
        let mut y = px(0.);
        let line_height = i.line_height();
        let mut caret = point(px(0.), px(0.));
        for line in &lines {
            if cursor <= base + line.len() {
                caret = line
                    .position_for_index(cursor - base, line_height)
                    .unwrap_or_default()
                    + point(px(0.), y);
                break;
            }
            base += line.len() + 1;
            y += line.size(line_height).height;
        }
        // Single-line fields never wrap or scroll vertically. In particular,
        // a line taller than its viewport used to alternate between 0 and 2 px
        // on every repaint, making even the empty placeholder visibly jump.
        let scroll_y = if i.manual_scroll {
            i.scroll_y
        } else if i.single_line || caret.y < i.scroll_y {
            caret.y
        } else if caret.y + line_height > i.scroll_y + bounds.size.height {
            caret.y + line_height - bounds.size.height
        } else {
            i.scroll_y
        };
        let scroll_x = if !i.single_line || placeholder {
            px(0.)
        } else if caret.x < i.scroll_x {
            caret.x
        } else if caret.x + px(2.) > i.scroll_x + bounds.size.width {
            (caret.x + px(2.) - bounds.size.width).max(px(0.))
        } else {
            i.scroll_x
        };
        let pending = i.reveal_pending;
        let reveal = pending && i.focus.is_focused(w);
        if reveal {
            w.request_autoscroll(Bounds::new(
                bounds.origin + caret - point(scroll_x, scroll_y + px(8.)),
                size(px(2.), line_height + px(16.)),
            ));
        }
        // Consume the request once. Repainting or manually scrolling a focused
        // document must not keep snapping back to its last caret position.
        if pending {
            self.input
                .update(cx, |input, _| input.reveal_pending = false);
        }
        PaintState {
            lines,
            scroll: point(scroll_x, scroll_y),
            layout,
        }
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        state: &mut PaintState,
        w: &mut Window,
        cx: &mut App,
    ) {
        paint_drag_listener(&self.input, w, cx);
        let i = self.input.read(cx);
        let focus = i.focus.clone();
        let to_display = |at| state.layout.to_display(at);
        let selection = to_display(i.selection.start)..to_display(i.selection.end);
        let cursor = to_display(i.cursor());
        let line_height = i.line_height();
        let center_offset = if i.centered && i.single_line {
            (bounds.size.width
                - state
                    .lines
                    .first()
                    .map(|line| line.size(line_height).width)
                    .unwrap_or_default())
            .max(px(0.))
                / 2.
        } else {
            px(0.)
        };
        w.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        w.with_content_mask(Some(ContentMask { bounds }), |w| {
            let mut base = 0;
            let x = bounds.left() - state.scroll.x + center_offset;
            let mut y = bounds.top() - state.scroll.y;
            for line in &state.lines {
                let height = line.size(line_height).height;
                if y + height >= bounds.top() && y < bounds.bottom() {
                    let start = selection.start.saturating_sub(base).min(line.len());
                    let end = selection.end.saturating_sub(base).min(line.len());
                    if start < end {
                        let a = line
                            .position_for_index(start, line_height)
                            .unwrap_or_default();
                        let b = line
                            .position_for_index(end, line_height)
                            .unwrap_or_default();
                        let mut sy = a.y;
                        while sy <= b.y {
                            let left = if sy == a.y { a.x } else { px(0.) };
                            let right = if sy == b.y { b.x } else { bounds.size.width };
                            w.paint_quad(fill(
                                Bounds::new(
                                    point(x + left, y + sy),
                                    size((right - left).max(px(1.)), line_height),
                                ),
                                crate::ui::theme::text_selection(),
                            ));
                            sy += line_height;
                        }
                    }
                    let _ = line.paint(point(x, y), line_height, TextAlign::Left, None, w, cx);
                    for (source, icon) in &state.layout.mentions {
                        let at = to_display(*source);
                        if at < base || at >= base + line.len() {
                            continue;
                        }
                        // Centered in the em blank; the thin blank after it is the gap.
                        let (Some(a), Some(b)) = (
                            line.position_for_index(at - base, line_height),
                            line.position_for_index(at + mention::ICON_LEN - base, line_height),
                        ) else {
                            continue;
                        };
                        let side = px(mention::ICON_SIDE);
                        let icon_bounds = Bounds::new(
                            point(
                                x + (a.x + b.x.max(a.x)) / 2. - side / 2.,
                                y + a.y + (line_height - side) / 2.,
                            ),
                            size(side, side),
                        );
                        match icon {
                            MentionIcon::File(asset) => {
                                if let Some(Ok(image)) = w.use_asset::<ImgResourceLoader>(
                                    &Resource::Embedded(asset.clone()),
                                    cx,
                                ) {
                                    let _ = w.paint_image(
                                        bounds,
                                        icon_bounds,
                                        Default::default(),
                                        image,
                                        0,
                                        false,
                                    );
                                }
                            }
                            MentionIcon::Note => {
                                let _ = w.paint_svg(
                                    icon_bounds,
                                    "note".into(),
                                    None,
                                    Default::default(),
                                    crate::ui::theme::accent_color(),
                                    cx,
                                );
                            }
                        }
                    }
                    if focus.is_focused(w)
                        && selection.is_empty()
                        && cursor >= base
                        && cursor <= base + line.len()
                    {
                        let p = line
                            .position_for_index(cursor - base, line_height)
                            .unwrap_or_default();
                        w.paint_quad(fill(
                            Bounds::new(point(x + p.x, y + p.y), size(px(1.5), line_height)),
                            crate::ui::theme::content(1.),
                        ));
                    }
                }
                base += line.len() + 1;
                y += height;
            }
            if let Some(command) = state.layout.command {
                let icon_bounds = Bounds::new(
                    bounds.origin
                        + point(
                            -state.scroll.x,
                            (line_height - px(14.)) / 2. - state.scroll.y,
                        ),
                    size(px(14.), px(14.)),
                );
                let _ = w.paint_svg(
                    icon_bounds,
                    command.mode.icon().into(),
                    None,
                    Default::default(),
                    command.mode.color(),
                    cx,
                );
            }
        });
        self.input.update(cx, |i, _| {
            i.lines = std::mem::take(&mut state.lines);
            i.bounds = Some(Bounds::new(
                bounds.origin + point(center_offset, px(0.)),
                bounds.size,
            ));
            i.scroll_x = state.scroll.x;
            i.scroll_y = state.scroll.y;
        });
    }
}
