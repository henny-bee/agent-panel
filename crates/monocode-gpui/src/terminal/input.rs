use super::*;

impl Terminal {
    pub(super) fn key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let mode = *self.term.lock().mode();
        if let Some(bytes) = key_bytes(&event.keystroke, mode) {
            cx.stop_propagation();
            self.input(bytes, cx);
        }
    }

    pub(super) fn copy(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self
            .term
            .lock()
            .selection_to_string()
            .filter(|t| !t.is_empty())
        {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub(super) fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        self.input(paste_bytes(&text, bracketed), cx);
    }

    /// Types `text` as one paste, then presses Enter. As a paste, its line
    /// breaks don't send it early; Enter waits a moment, as a CLI that gathers
    /// a paste can take an Enter arriving with it as one more line break.
    pub(crate) fn submit(&mut self, text: &str, cx: &mut Context<Self>) {
        let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        self.input(paste_bytes(text, bracketed), cx);
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(150))
                .await;
            let _ = this.update(cx, |this, cx| this.input(b"\r".to_vec(), cx));
        })
        .detach();
    }

    pub(super) fn select_all(&mut self, cx: &mut Context<Self>) {
        let mut term = self.term.lock();
        let mut selection = Selection::new(
            SelectionType::Simple,
            GridPoint::new(term.topmost_line(), Column(0)),
            Side::Left,
        );
        selection.update(
            GridPoint::new(term.bottommost_line(), term.last_column()),
            Side::Right,
        );
        term.selection = Some(selection);
        drop(term);
        cx.notify();
    }

    /// The cell under a window position, and which half of it.
    /// The zero-based viewport line and column under a pointer.
    pub(super) fn viewport_cell(&self, position: gpui::Point<gpui::Pixels>) -> (usize, usize) {
        let Some(g) = self.geometry.get() else {
            return (0, 0);
        };
        let x = ((position.x - g.origin.x) / g.cell.width).max(0.);
        let y = ((position.y - g.origin.y) / g.cell.height).max(0.);
        (
            (y as usize).min(self.size.lines.saturating_sub(1)),
            (x as usize).min(self.size.columns.saturating_sub(1)),
        )
    }

    pub(super) fn grid_point(
        &self,
        position: gpui::Point<gpui::Pixels>,
    ) -> Option<(GridPoint, Side)> {
        let g = self.geometry.get()?;
        let x = ((position.x - g.origin.x) / g.cell.width).max(0.);
        let y = ((position.y - g.origin.y) / g.cell.height).max(0.);
        let column = (x as usize).min(self.size.columns.saturating_sub(1));
        let line = (y as usize).min(self.size.lines.saturating_sub(1));
        let side = if x.fract() < 0.5 {
            Side::Left
        } else {
            Side::Right
        };
        let offset = self.term.lock().grid().display_offset();
        Some((
            viewport_to_point(offset, GridPoint::new(line, Column(column))),
            side,
        ))
    }

    /// The web link or existing path under a window position, if the
    /// pointer is on a cell.
    pub(super) fn link_at(&mut self, position: gpui::Point<gpui::Pixels>) -> Option<Link> {
        let g = self.geometry.get()?;
        let x = (position.x - g.origin.x) / g.cell.width;
        let y = (position.y - g.origin.y) / g.cell.height;
        if x < 0. || y < 0. || x >= self.size.columns as f32 || y >= self.size.lines as f32 {
            return None;
        }
        let term = self.term.lock();
        let at = viewport_to_point(
            term.grid().display_offset(),
            GridPoint::new(y as usize, Column(x as usize)),
        );
        link_at(&term, self.url_regex.as_mut()?, &self.cwd, at)
    }

    pub(super) fn mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        // Ctrl+click (Cmd on macOS) takes a link even from a program
        // tracking the mouse, which never sees that press.
        self.pressed_link = (event.click_count == 1 && event.modifiers.secondary())
            .then(|| self.link_at(event.position))
            .flatten();
        if self.pressed_link.is_some() {
            return;
        }
        let mode = *self.term.lock().mode();
        // A program tracking the mouse gets the click itself, as it gets
        // the wheel; Shift keeps the terminal's own selection.
        if mode.intersects(TermMode::MOUSE_MODE) && !event.modifiers.shift {
            let (line, column) = self.viewport_cell(event.position);
            let button = mouse_modifiers(&event.modifiers);
            self.write(mouse_report(button, false, line, column, mode));
            self.reported = Some((line, column));
            return;
        }
        let Some((at, side)) = self.grid_point(event.position) else {
            return;
        };
        let mut term = self.term.lock();
        match term.selection.as_mut() {
            Some(selection) if event.modifiers.shift => selection.update(at, side),
            _ => {
                let ty = match event.click_count {
                    2 => SelectionType::Semantic,
                    3.. => SelectionType::Lines,
                    _ => SelectionType::Simple,
                };
                term.selection = Some(Selection::new(ty, at, side));
            }
        }
        drop(term);
        self.selecting = true;
        self.selection_pointer = None;
        self.selection_scroll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                if !this
                    .update_in(cx, |this, window, cx| {
                        if !window.is_window_active() || !this.focus.is_focused(window) {
                            this.selecting = false;
                            this.selection_pointer = None;
                            return false;
                        }
                        this.scroll_selection(cx)
                    })
                    .unwrap_or(false)
                {
                    break;
                }
            }
        }));
        cx.notify();
    }

    /// A Ctrl+click that pressed and released on the same link opens it: a
    /// web link in the browser, a file in MCode, a folder in the file manager.
    pub(super) fn mouse_up(
        &mut self,
        event: &gpui::MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(link) = self.pressed_link.take() {
            if self.link_at(event.position).as_ref() == Some(&link) {
                match link.target {
                    Target::Web => cx.open_url(&link.uri),
                    Target::File => {
                        window.dispatch_action(Box::new(crate::ui::files::OpenPath(link.uri)), cx)
                    }
                    Target::Folder => cx.open_with_system(std::path::Path::new(&link.uri)),
                }
            }
            return;
        }
        if self.reported.is_some() {
            self.report_release(event.position, &event.modifiers);
            return;
        }
        self.stop_selection();
    }

    /// Ends a reported press, unless the program stopped tracking meanwhile.
    pub(super) fn report_release(
        &mut self,
        position: gpui::Point<gpui::Pixels>,
        modifiers: &gpui::Modifiers,
    ) {
        self.reported = None;
        let mode = *self.term.lock().mode();
        if mode.intersects(TermMode::MOUSE_MODE) {
            let (line, column) = self.viewport_cell(position);
            self.write(mouse_report(
                mouse_modifiers(modifiers),
                true,
                line,
                column,
                mode,
            ));
        }
    }

    pub(super) fn stop_selection(&mut self) {
        self.selecting = false;
        self.selection_pointer = None;
        self.selection_scroll = None;
    }

    /// Keep extending the same selection through scrollback at either edge,
    /// even when the pointer is held still outside the terminal.
    pub(super) fn scroll_selection(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.selecting || !self.visible {
            self.selecting = false;
            self.selection_pointer = None;
            return false;
        }
        let Some(position) = self.selection_pointer else {
            // A press without a drag must not scroll a first/last-row click.
            return true;
        };
        let Some(geometry) = self.geometry.get() else {
            return true;
        };
        let y = (position.y - geometry.origin.y) / geometry.cell.height;
        let delta = if y < 0.5 {
            (0.5 - y).ceil().clamp(1., 10.) as i32
        } else if y > self.size.lines as f32 - 0.5 {
            -((y - self.size.lines as f32 + 0.5).ceil().clamp(1., 10.) as i32)
        } else {
            return true;
        };
        let changed = {
            let mut term = self.term.lock();
            if term.selection.is_none() {
                return false;
            }
            let before = term.grid().display_offset();
            term.scroll_display(Scroll::Delta(delta));
            term.grid().display_offset() != before
        };
        if changed {
            if let Some((at, side)) = self.grid_point(position)
                && let Some(selection) = self.term.lock().selection.as_mut()
            {
                selection.update(at, side);
            }
            cx.notify();
        }
        true
    }

    pub(super) fn mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        // shortcut: only drags are reported, not hover with no button held;
        // add it if a program's hover effects (mode 1003) are missed.
        if let Some(last) = self.reported {
            if event.pressed_button != Some(MouseButton::Left) {
                // The release happened outside the window.
                self.report_release(event.position, &event.modifiers);
                return;
            }
            let mode = *self.term.lock().mode();
            let cell = self.viewport_cell(event.position);
            if cell != last && mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) {
                let button = 32 + mouse_modifiers(&event.modifiers);
                self.write(mouse_report(button, false, cell.0, cell.1, mode));
                self.reported = Some(cell);
            }
            return;
        }
        if !self.selecting {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.stop_selection();
            return;
        }
        self.selection_pointer = Some(event.position);
        if let Some((at, side)) = self.grid_point(event.position) {
            if let Some(selection) = self.term.lock().selection.as_mut() {
                selection.update(at, side);
            }
            cx.notify();
        }
    }

    pub(super) fn scroll(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let line = self.geometry.get().map_or(16., |g| g.cell.height.as_f32());
        let delta = match event.delta {
            ScrollDelta::Lines(lines) => lines.y,
            ScrollDelta::Pixels(pixels) => pixels.y.as_f32() / line,
        } + self.scroll_remainder;
        let lines = delta.trunc() as i32;
        self.scroll_remainder = delta.fract();
        if lines == 0 {
            return;
        }
        let mode = *self.term.lock().mode();
        // A program tracking the mouse gets the wheel itself.
        if mode.intersects(TermMode::MOUSE_MODE) {
            let (line, column) = self.viewport_cell(event.position);
            let button = if lines > 0 { 64 } else { 65 };
            let report = mouse_report(button, false, line, column, mode);
            self.write(report.repeat(lines.unsigned_abs() as usize));
        // A full-screen program scrolls itself; the shell scrolls history.
        } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            let key: &[u8] = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            self.write(key.repeat(lines.unsigned_abs() as usize));
        } else {
            self.term.lock().scroll_display(Scroll::Delta(lines));
            cx.notify();
        }
    }
}

/// The link covering `at`: the HTTP(S) OSC 8 hyperlink on its cells, else
/// a plain URL on its logical line, else a path there that exists.
pub(super) fn link_at<T>(
    term: &Term<T>,
    regex: &mut RegexSearch,
    cwd: &str,
    at: GridPoint,
) -> Option<Link> {
    let start = term.line_search_left(at);
    let end = term.line_search_right(at);
    if let Some(hyperlink) = term.grid()[at].hyperlink() {
        let same = |point: GridPoint| term.grid()[point].hyperlink().as_ref() == Some(&hyperlink);
        let mut first = at;
        while first > start && same(first.sub(term, Boundary::None, 1)) {
            first = first.sub(term, Boundary::None, 1);
        }
        let mut last = at;
        while last < end && same(last.add(term, Boundary::None, 1)) {
            last = last.add(term, Boundary::None, 1);
        }
        let uri = hyperlink.uri();
        return is_web_link(uri).then(|| Link {
            range: first..=last,
            uri: uri.to_owned(),
            target: Target::Web,
        });
    }
    RegexIter::new(start, end, Direction::Right, term, regex)
        .find(|found| found.contains(&at))
        .map(|range| {
            let mut uri = String::new();
            let mut point = *range.start();
            loop {
                let cell = &term.grid()[point];
                if !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    uri.push(cell.c);
                }
                if point >= *range.end() {
                    break;
                }
                point = point.add(term, Boundary::None, 1);
            }
            Link {
                range,
                uri,
                target: Target::Web,
            }
        })
        .filter(|link| is_web_link(&link.uri))
        .or_else(|| path_at(term, cwd, at, start..=end))
}

/// The path covering `at`, read as the word around it (wrapped rows
/// included), when it names a file or folder that exists. `Read(src/a.rs)`
/// gives `src/a.rs`, and `src/a.rs:12:3,` that file at line 12, column 3;
/// relative paths start at `cwd`.
/// shortcut: paths with spaces are not found, and the shell's later `cd`s are
/// unknown; read OSC 7 if the bottom Terminal needs them.
fn path_at<T>(
    term: &Term<T>,
    cwd: &str,
    at: GridPoint,
    line: RangeInclusive<GridPoint>,
) -> Option<Link> {
    let grid = term.grid();
    let spacer = |point: GridPoint| {
        grid[point]
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
    };
    let breaks = |point: GridPoint| {
        let c = grid[point].c;
        !spacer(point)
            && (c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '<' | '>' | '|' | '\0'))
    };
    if breaks(at) {
        return None;
    }
    let mut first = at;
    while first > *line.start() && !breaks(first.sub(term, Boundary::None, 1)) {
        first = first.sub(term, Boundary::None, 1);
    }
    let mut cells = Vec::new();
    let mut point = first;
    loop {
        if !spacer(point) {
            cells.push((point, grid[point].c));
        }
        if point >= *line.end() || breaks(point.add(term, Boundary::None, 1)) {
            break;
        }
        point = point.add(term, Boundary::None, 1);
    }
    while cells
        .last()
        .is_some_and(|(_, c)| matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}'))
    {
        cells.pop();
    }
    // The whole word, else what follows its first bracket: `Read(src/a.rs`.
    let skip = cells
        .iter()
        .take_while(|(_, c)| matches!(c, '(' | '[' | '{'))
        .count();
    let open = cells.iter().position(|(_, c)| *c == '(').map(|ix| ix + 1);
    [Some(skip), open].into_iter().flatten().find_map(|from| {
        let cells = cells.get(from..).filter(|cells| !cells.is_empty())?;
        let range = cells[0].0..=cells[cells.len() - 1].0;
        if !range.contains(&at) {
            return None;
        }
        let text: String = cells.iter().map(|(_, c)| c).collect();
        let named = text.split(':').next().unwrap_or_default();
        // A word without `/` or `.` stays text, even when `cwd/src` exists.
        if !(named.contains('/') || named.contains('.')) || text.contains("://") {
            return None;
        }
        let (path, location) =
            crate::ui::files::file_location(&text, &working_dir(cwd).to_string_lossy());
        let target = match std::fs::metadata(&path) {
            Ok(meta) if meta.is_dir() => Target::Folder,
            Ok(_) => Target::File,
            Err(_) => return None,
        };
        let uri = match location {
            Some((line, column)) if target == Target::File => format!("{path}:{line}:{column}"),
            _ => path,
        };
        Some(Link { range, uri, target })
    })
}

/// What pasting `text` sends: wrapped in paste markers when the program asks
/// for them, else with line breaks as Enter presses.
pub(crate) fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        // A pasted end marker must not end the paste early.
        let text = text.replace("\x1b[201~", "");
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

/// Bytes a keystroke sends, or None to leave it to the app's bindings.
/// Includes terminalKeys.ts's macOS editing shortcuts.
pub(crate) fn key_bytes(keystroke: &Keystroke, mode: TermMode) -> Option<Vec<u8>> {
    let m = &keystroke.modifiers;
    let key = keystroke.key.as_str();
    if m.platform {
        if m.control || m.shift || m.alt {
            return None;
        }
        return match key {
            "left" => Some(b"\x01".to_vec()),
            "right" => Some(b"\x05".to_vec()),
            "backspace" => Some(b"\x15".to_vec()),
            _ => None,
        };
    }
    if m.alt && !m.control && !m.shift {
        match key {
            "left" => return Some(b"\x1bb".to_vec()),
            "right" => return Some(b"\x1bf".to_vec()),
            _ => {}
        }
    }
    let modifier = 1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.control as u8;
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let cursor = |final_byte: char| {
        Some(if modifier > 1 {
            format!("\x1b[1;{modifier}{final_byte}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{final_byte}").into_bytes()
        } else {
            format!("\x1b[{final_byte}").into_bytes()
        })
    };
    let tilde = |code: u8| {
        Some(if modifier > 1 {
            format!("\x1b[{code};{modifier}~").into_bytes()
        } else {
            format!("\x1b[{code}~").into_bytes()
        })
    };
    let function = |final_byte: char| {
        Some(if modifier > 1 {
            format!("\x1b[1;{modifier}{final_byte}").into_bytes()
        } else {
            format!("\x1bO{final_byte}").into_bytes()
        })
    };
    let meta = |bytes: &[u8]| {
        let mut out = Vec::with_capacity(bytes.len() + 1);
        if m.alt {
            out.push(0x1b);
        }
        out.extend_from_slice(bytes);
        Some(out)
    };
    match key {
        "up" => return cursor('A'),
        "down" => return cursor('B'),
        "right" => return cursor('C'),
        "left" => return cursor('D'),
        "home" => return cursor('H'),
        "end" => return cursor('F'),
        "insert" => return tilde(2),
        "delete" => return tilde(3),
        "pageup" => return tilde(5),
        "pagedown" => return tilde(6),
        "f1" => return function('P'),
        "f2" => return function('Q'),
        "f3" => return function('R'),
        "f4" => return function('S'),
        "f5" => return tilde(15),
        "f6" => return tilde(17),
        "f7" => return tilde(18),
        "f8" => return tilde(19),
        "f9" => return tilde(20),
        "f10" => return tilde(21),
        "f11" => return tilde(23),
        "f12" => return tilde(24),
        "enter" => return meta(b"\r"),
        "escape" => return meta(b"\x1b"),
        "tab" if m.shift => return Some(b"\x1b[Z".to_vec()),
        "tab" => return meta(b"\t"),
        "backspace" if m.control => return meta(b"\x08"),
        "backspace" => return meta(b"\x7f"),
        _ => {}
    }
    if m.control {
        let ch = if key == "space" {
            ' '
        } else {
            let mut chars = key.chars();
            let ch = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            ch
        };
        let code = match ch.to_ascii_lowercase() {
            c @ 'a'..='z' => c as u8 - b'a' + 1,
            ' ' | '@' | '2' => 0,
            '[' | '3' => 27,
            '\\' | '4' => 28,
            ']' | '5' => 29,
            '^' | '6' => 30,
            '_' | '-' | '/' | '7' => 31,
            '8' => 127,
            _ => return None,
        };
        return meta(&[code]);
    }
    // Option is Meta (`macOptionIsMeta`): the key itself, after ESC.
    if m.alt {
        let text = if key == "space" {
            " ".to_owned()
        } else if m.shift {
            key.to_uppercase()
        } else {
            key.to_owned()
        };
        if text.chars().count() == 1 {
            return meta(text.as_bytes());
        }
    }
    let text = keystroke.key_char.as_deref().or(match key {
        "space" => Some(" "),
        _ => None,
    })?;
    Some(text.as_bytes().to_vec())
}
