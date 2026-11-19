use super::*;

pub(super) struct Grid {
    pub(super) terminal: Entity<Terminal>,
    pub(super) focused: bool,
}

impl IntoElement for Grid {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for Grid {
    type RequestLayoutState = ();
    type PrepaintState = Painted;
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
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = gpui::relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<gpui::Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Painted {
        let font_size = px(FONT_SIZE);
        let base = crate::ui::fonts::terminal(cx);
        let text_system = window.text_system().clone();
        let font_id = text_system.resolve_font(&base);
        let cell_width = text_system
            .advance(font_id, font_size, 'm')
            .map_or(px(7.8), |size| size.width);
        let line_height = (text_system.ascent(font_id, font_size)
            + text_system.descent(font_id, font_size).abs())
        .ceil()
        .max(px(14.));
        let origin = point(bounds.left() + px(PADDING), bounds.top() + px(PADDING / 2.));
        let columns =
            (((bounds.size.width - px(PADDING * 2.)) / cell_width).floor() as usize).max(2);
        let lines = (((bounds.size.height - px(PADDING)) / line_height).floor() as usize).max(1);
        let cell = size(cell_width, line_height);
        let here = window.window_handle().window_id();
        let (term, link) = self.terminal.update(cx, |t, _| {
            if t.sized_by.is_none_or(|by| by == here) || window.is_window_active() {
                t.sized_by = Some(here);
                t.resize(columns, lines, cell);
                t.geometry.set(Some(Geometry { origin, cell }));
            }
            let link = t.hover.and_then(|position| t.link_at(position));
            (t.term.clone(), link)
        });

        let term = term.lock();
        let content = term.renderable_content();
        let colors = content.colors;
        let offset = content.display_offset;
        let selection = content.selection;
        let mut rows: Vec<Vec<Painting>> = vec![Vec::new(); lines];
        for indexed in content.display_iter {
            let cell = indexed.cell;
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let Some(at) = point_to_viewport(offset, indexed.point) else {
                continue;
            };
            if at.line >= lines {
                continue;
            }
            let mut fg = color(cell.fg, colors, true);
            let mut bg = match cell.bg {
                AnsiColor::Named(NamedColor::Background) => None,
                other => Some(color(other, colors, false)),
            };
            if cell.flags.contains(Flags::INVERSE) {
                let back = bg.unwrap_or(rgb(CHAT).into());
                bg = Some(fg);
                fg = back;
            }
            if cell.flags.intersects(Flags::DIM) {
                fg.a *= 0.66;
            }
            if selection.is_some_and(|s| s.contains(indexed.point)) {
                bg = Some(selection_fill());
            }
            let c = if cell.flags.contains(Flags::HIDDEN) {
                ' '
            } else {
                cell.c
            };
            rows[at.line].push((
                at.column.0,
                c,
                Pen {
                    fg,
                    bold: cell.flags.contains(Flags::BOLD),
                    italic: cell.flags.contains(Flags::ITALIC),
                    underline: cell.flags.intersects(Flags::ALL_UNDERLINES)
                        || link
                            .as_ref()
                            .is_some_and(|link| link.range.contains(&indexed.point)),
                    strike: cell.flags.contains(Flags::STRIKEOUT),
                },
                bg,
                cell.flags.contains(Flags::WIDE_CHAR),
            ));
        }
        let cursor_point = content.cursor.point;
        let cursor_shape = content.cursor.shape;
        let show_cursor = content.mode.contains(TermMode::SHOW_CURSOR);
        drop(term);

        let mut painted = Painted {
            lines: Vec::new(),
            rects: Vec::new(),
            cursor: None,
            line_height,
            link_hitbox: link.map(|_| window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal)),
        };
        for (line, cells) in rows.iter().enumerate() {
            let y = origin.y + line_height * line as f32;
            // Backgrounds merge across equal neighbours.
            let mut span: Option<(usize, usize, Hsla)> = None;
            for (column, _, _, bg, wide) in cells {
                let width = if *wide { 2 } else { 1 };
                match (span, bg) {
                    (Some((start, end, fill)), Some(bg)) if end == *column && fill == *bg => {
                        span = Some((start, end + width, fill));
                    }
                    (_, bg) => {
                        if let Some((start, end, fill)) = span.take() {
                            painted
                                .rects
                                .push((cell_rect(origin, cell, line, start, end), fill));
                        }
                        span = bg.map(|bg| (*column, column + width, bg));
                    }
                }
            }
            if let Some((start, end, fill)) = span {
                painted
                    .rects
                    .push((cell_rect(origin, cell, line, start, end), fill));
            }
            // Text runs break where the pen changes, and around wide glyphs
            // so each one spans its two cells.
            let mut ix = 0;
            while ix < cells.len() {
                let (column, _, pen, _, wide) = &cells[ix];
                let mut text = String::new();
                let mut end = ix;
                while end < cells.len()
                    && cells[end].2 == *pen
                    && !cells[end].4
                    && (end == ix || !wide)
                    && cells[end].0 == column + (end - ix)
                {
                    text.push(cells[end].1);
                    end += 1;
                }
                if end == ix {
                    text.push(cells[ix].1);
                    end = ix + 1;
                }
                if !text.trim().is_empty() {
                    let mut font = base.clone();
                    if pen.bold {
                        font.weight = FontWeight::BOLD;
                    }
                    if pen.italic {
                        font.style = FontStyle::Italic;
                    }
                    let run = TextRun {
                        len: text.len(),
                        font,
                        color: pen.fg,
                        background_color: None,
                        underline: pen.underline.then_some(UnderlineStyle {
                            thickness: px(1.),
                            color: Some(pen.fg),
                            wavy: false,
                        }),
                        strikethrough: pen.strike.then_some(StrikethroughStyle {
                            thickness: px(1.),
                            color: Some(pen.fg),
                        }),
                    };
                    let width = if *wide { cell_width * 2. } else { cell_width };
                    let shaped = text_system.shape_line(
                        SharedString::from(text),
                        font_size,
                        &[run],
                        Some(width),
                    );
                    painted
                        .lines
                        .push((point(origin.x + cell_width * *column as f32, y), shaped));
                }
                ix = end;
            }
        }
        if show_cursor
            && cursor_shape != CursorShape::Hidden
            && let Some(at) = point_to_viewport(offset, cursor_point)
            && at.line < lines
        {
            let x = origin.x + cell_width * at.column.0 as f32;
            let y = origin.y + line_height * at.line as f32;
            let accent: Hsla = accent_color();
            let (bounds, hollow) = match (cursor_shape, self.focused) {
                (CursorShape::Beam, _) => {
                    (Bounds::new(point(x, y), size(px(2.), line_height)), false)
                }
                (CursorShape::Underline, _) => (
                    Bounds::new(point(x, y + line_height - px(2.)), size(cell_width, px(2.))),
                    false,
                ),
                (CursorShape::Block, true) => (Bounds::new(point(x, y), cell), false),
                _ => (Bounds::new(point(x, y), cell), true),
            };
            let mut fill = accent;
            if !self.focused {
                fill.a = 0.5;
            } else if cursor_shape == CursorShape::Block {
                fill.a = 0.6;
            }
            painted.cursor = Some((bounds, fill, hollow));
        }
        painted
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<gpui::Pixels>,
        _: &mut (),
        painted: &mut Painted,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Element mouse handlers only receive hovered events. An active drag
        // must keep tracking movement and release anywhere in the window.
        let terminal = self.terminal.downgrade();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
            if phase == gpui::DispatchPhase::Capture {
                let _ = terminal.update(cx, |terminal, cx| terminal.mouse_move(event, cx));
            }
        });
        let terminal = self.terminal.downgrade();
        window.on_mouse_event(move |event: &gpui::MouseUpEvent, phase, _, cx| {
            if phase == gpui::DispatchPhase::Capture && event.button == MouseButton::Left {
                let _ = terminal.update(cx, |terminal, cx| terminal.mouse_up(event, cx));
            }
        });
        if let Some(hitbox) = &painted.link_hitbox {
            window.set_cursor_style(gpui::CursorStyle::PointingHand, hitbox);
        }
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for (rect, fill) in &painted.rects {
                window.paint_quad(gpui::fill(*rect, *fill));
            }
            for (origin, line) in &painted.lines {
                let _ = line.paint(
                    *origin,
                    painted.line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
            if let Some((rect, fill, hollow)) = painted.cursor {
                if hollow {
                    window.paint_quad(gpui::outline(rect, fill, gpui::BorderStyle::Solid));
                } else {
                    window.paint_quad(gpui::fill(rect, fill));
                }
            }
        });
    }
}

pub(super) fn color(value: AnsiColor, colors: &Colors, foreground: bool) -> Hsla {
    let dim = |c: u32| {
        let mut h: Hsla = rgb(c).into();
        h.a *= 0.66;
        h
    };
    match value {
        AnsiColor::Spec(spec) => rgb(hex(spec)).into(),
        AnsiColor::Indexed(index) => colors[index as usize]
            .map_or_else(|| rgb(indexed_rgb(index)).into(), |c| rgb(hex(c)).into()),
        AnsiColor::Named(name) => {
            if let Some(c) = colors[name] {
                return rgb(hex(c)).into();
            }
            match name {
                NamedColor::Foreground | NamedColor::BrightForeground => content(1.),
                NamedColor::DimForeground => content(0.66),
                NamedColor::Background => {
                    if foreground {
                        rgb(CHAT).into()
                    } else {
                        gpui::transparent_black()
                    }
                }
                NamedColor::Cursor => accent_color(),
                NamedColor::DimBlack => dim(ANSI[0]),
                NamedColor::DimRed => dim(ANSI[1]),
                NamedColor::DimGreen => dim(ANSI[2]),
                NamedColor::DimYellow => dim(ANSI[3]),
                NamedColor::DimBlue => dim(ANSI[4]),
                NamedColor::DimMagenta => dim(ANSI[5]),
                NamedColor::DimCyan => dim(ANSI[6]),
                NamedColor::DimWhite => dim(ANSI[7]),
                other => rgb(ANSI[(other as usize).min(15)]).into(),
            }
        }
    }
}
