//! Port of TerminalView.tsx: a login shell on a PTY, emulated by
//! alacritty_terminal and painted as a grid of cell-snapped text runs.
mod shell;
use shell::default_shell;

use crate::ui::theme::*;
use alacritty_terminal::{
    event::{Event as TermEvent, EventListener, WindowSize},
    event_loop::{EventLoop, EventLoopSender, Msg},
    grid::{Dimensions, Scroll},
    index::{Boundary, Column, Direction, Point as GridPoint, Side},
    selection::{Selection, SelectionType},
    sync::FairMutex,
    term::{
        Config, Term, TermMode,
        cell::Flags,
        color::Colors,
        point_to_viewport,
        search::{RegexIter, RegexSearch},
        viewport_to_point,
    },
    tty,
    vte::ansi::{Color as AnsiColor, CursorShape, CursorStyle as TermCursorStyle, NamedColor, Rgb},
};
use gpui::{
    App, Bounds, ClipboardItem, Context, Element, ElementId, Entity, EventEmitter, FocusHandle,
    Focusable, FontStyle, FontWeight, GlobalElementId, Hsla, InspectorElementId, KeyBinding,
    KeyDownEvent, Keystroke, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, ScrollDelta,
    ScrollWheelEvent, ShapedLine, SharedString, Size, StrikethroughStyle, Style, Task, TextAlign,
    TextRun, UnderlineStyle, Window, actions, div, point, prelude::*, px, rgb, size,
};
use std::{
    borrow::Cow, cell::Cell, collections::HashMap, ops::RangeInclusive, rc::Rc, sync::Arc,
    time::Duration,
};

mod context_menu;
mod grid;
pub(crate) mod input;
use grid::Grid;
#[cfg(test)]
use input::key_bytes;

actions!(
    terminal,
    [SendTab, SendBackTab, SendEscape, Copy, Paste, SelectAll]
);

pub(crate) fn bind(cx: &mut App) {
    let m = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl-shift"
    };
    let c = Some("Terminal");
    // The shell owns these keys while a terminal has focus.
    cx.bind_keys([
        KeyBinding::new("tab", SendTab, c),
        KeyBinding::new("shift-tab", SendBackTab, c),
        KeyBinding::new("escape", SendEscape, c),
        KeyBinding::new(&format!("{m}-c"), Copy, c),
        KeyBinding::new(&format!("{m}-v"), Paste, c),
        KeyBinding::new(&format!("{m}-a"), SelectAll, c),
    ]);
    // Windows Terminal convention: plain Ctrl+V pastes too.
    #[cfg(windows)]
    cx.bind_keys([KeyBinding::new("ctrl-v", Paste, c)]);
}

const FONT_SIZE: f32 = 13.;
/// TerminalView's `scrollback`, retained for every open terminal.
const SCROLLBACK: usize = 2000;
const PADDING: f32 = 8.;
/// The web build's WebLinksAddon URL pattern.
const URL_PATTERN: &str = r#"(https?|HTTPS?)://[^\s"'!*(){}|\\^<>`]*[^\s"':,.!?{}|\\^~\[\]`()<>]"#;

/// Wakeups, titles and replies from the PTY thread.
#[derive(Clone)]
struct Listener(async_channel::Sender<TermEvent>);
impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        let _ = self.0.try_send(event);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct GridSize {
    columns: usize,
    lines: usize,
}
impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.lines
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn columns(&self) -> usize {
        self.columns
    }
}

pub(crate) enum TerminalEvent {
    /// Title, foreground process or running state changed.
    Changed,
}

/// Where the grid was painted, for mapping the pointer to cells.
#[derive(Clone, Copy, Default)]
struct Geometry {
    origin: gpui::Point<gpui::Pixels>,
    cell: Size<gpui::Pixels>,
}

/// An HTTP(S) link in the grid: an OSC 8 hyperlink or a plain URL.
#[derive(Clone, PartialEq, Debug)]
struct Link {
    range: RangeInclusive<GridPoint>,
    uri: String,
}

pub(crate) struct Terminal {
    context_menu: Option<context_menu::EditingMenu>,
    term: Arc<FairMutex<Term<Listener>>>,
    sender: Option<EventLoopSender>,
    focus: FocusHandle,
    pub cwd: String,
    /// The tab label: the foreground command, else the directory.
    pub title: String,
    /// A command other than the shell holds the terminal.
    pub foreground: Option<String>,
    pub exited: bool,
    size: GridSize,
    geometry: Rc<Cell<Option<Geometry>>>,
    /// The window whose painted area sizes the PTY. One terminal shown in
    /// two windows follows the one in use, not whichever painted last.
    sized_by: Option<gpui::WindowId>,
    master_fd: i32,
    shell_pid: u32,
    selecting: bool,
    selection_pointer: Option<gpui::Point<gpui::Pixels>>,
    selection_scroll: Option<Task<()>>,
    /// The cell last reported to a mouse-tracking program while the left
    /// button is held, so its drag and release go to the program too.
    reported: Option<(usize, usize)>,
    scroll_remainder: f32,
    url_regex: Option<RegexSearch>,
    /// The pointer over the grid, for underlining the link beneath it.
    hover: Option<gpui::Point<gpui::Pixels>>,
    /// The link a plain click pressed; releasing on it opens it.
    pressed_link: Option<Link>,
    pub(crate) visible: bool,
    _events: Task<()>,
    _poll: Task<()>,
}
impl EventEmitter<TerminalEvent> for Terminal {}
impl Focusable for Terminal {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

struct Spawned {
    term: Arc<FairMutex<Term<Listener>>>,
    sender: Option<EventLoopSender>,
    master_fd: i32,
    shell_pid: u32,
    events: async_channel::Receiver<TermEvent>,
}
/// A program that holds the terminal in place of the login shell.
pub(crate) struct Program {
    pub path: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}
/// Start the login shell, or `program`, on a PTY with TerminalView's
/// environment. A failure is written into the grid, as TerminalView prints it.
fn spawn(cwd: &str, size: GridSize, id: u64, program: Option<Program>) -> Spawned {
    let (tx, rx) = async_channel::unbounded();
    let listener = Listener(tx);
    let config = Config {
        scrolling_history: SCROLLBACK,
        default_cursor_style: TermCursorStyle {
            shape: CursorShape::Beam,
            blinking: true,
        },
        ..Config::default()
    };
    let term = Arc::new(FairMutex::new(Term::new(config, &size, listener.clone())));
    let workdir = working_dir(cwd);
    let search_path = monocode_core::vcs::git::scm::terminal_search_path();
    let (program, args, extra) = match program {
        Some(program) => (program.path, program.args, program.env),
        None => {
            let (program, args) = default_shell(&search_path);
            (program, args, Vec::new())
        }
    };
    let mut env = HashMap::new();
    for (key, value) in [
        ("TERM", "xterm-256color".to_owned()),
        ("COLORTERM", "truecolor".to_owned()),
        ("COLORFGBG", "15;0".to_owned()),
        ("TERM_PROGRAM", "MonoCode".to_owned()),
        ("PATH", search_path),
        ("PWD", workdir.to_string_lossy().into_owned()),
    ] {
        env.insert(key.to_owned(), value);
    }
    env.extend(extra);
    let options = tty::Options {
        shell: Some(tty::Shell::new(shell::pty_program(&program), args)),
        working_directory: Some(workdir),
        drain_on_exit: false,
        env,
        #[cfg(target_os = "windows")]
        escape_args: true,
    };
    let window = WindowSize {
        num_lines: size.lines as u16,
        num_cols: size.columns as u16,
        cell_width: 8,
        cell_height: 16,
    };
    #[cfg_attr(windows, allow(unused_mut))]
    let mut master_fd = -1;
    let mut shell_pid = 0;
    let sender = match tty::new(&options, window, id) {
        Ok(pty) => {
            #[cfg(unix)]
            {
                use std::os::fd::AsRawFd;
                master_fd = pty.file().as_raw_fd();
                shell_pid = pty.child().id();
            }
            // The Claude CLI view reads `~/.claude/sessions/<pid>.json` to follow
            // `/resume` and `/clear`, so Windows needs the child's pid too.
            #[cfg(windows)]
            {
                shell_pid = pty.child_watcher().pid().map_or(0, |pid| pid.get());
            }
            match EventLoop::new(term.clone(), listener, pty, false, false) {
                Ok(event_loop) => {
                    let sender = event_loop.channel();
                    event_loop.spawn();
                    Some(sender)
                }
                Err(error) => {
                    write_note(&term, &format!("\x1b[31m{error}\x1b[0m"));
                    None
                }
            }
        }
        Err(error) => {
            write_note(
                &term,
                &format!("\x1b[31mCould not start the shell ({program}): {error}\x1b[0m"),
            );
            None
        }
    };
    Spawned {
        term,
        sender,
        master_fd,
        shell_pid,
        events: rx,
    }
}

fn detached(size: GridSize) -> Spawned {
    let (tx, rx) = async_channel::unbounded();
    let term = Term::new(Config::default(), &size, Listener(tx));
    Spawned {
        term: Arc::new(FairMutex::new(term)),
        sender: None,
        master_fd: -1,
        shell_pid: 0,
        events: rx,
    }
}

impl Terminal {
    pub(crate) fn new(cwd: String, cx: &mut Context<Self>) -> Self {
        Self::start(cwd, None, cx)
    }
    /// A terminal that runs `program` and ends with it.
    pub(crate) fn program(cwd: String, program: Program, cx: &mut Context<Self>) -> Self {
        Self::start(cwd, Some(program), cx)
    }
    fn start(cwd: String, program: Option<Program>, cx: &mut Context<Self>) -> Self {
        let size = GridSize {
            columns: 80,
            lines: 24,
        };
        let Spawned {
            term,
            sender,
            master_fd,
            shell_pid,
            events: rx,
        } = if cfg!(test) {
            // GPUI's deterministic test scheduler rejects wakeups from the
            // PTY thread; UI tests run the dock without a shell behind it.
            detached(size)
        } else {
            spawn(&cwd, size, cx.entity_id().as_u64(), program)
        };
        let events = cx.spawn(async move |this, cx| {
            while let Ok(event) = rx.recv().await {
                // Output arrives in bursts; one repaint covers the burst.
                let mut batch = vec![event];
                while let Ok(more) = rx.try_recv() {
                    batch.push(more);
                }
                if this
                    .update(cx, |this, cx| {
                        for event in batch {
                            this.on_event(event, cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        // TerminalView's 1s foreground poll; a hidden dock has no title to paint.
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let Ok(Some((fd, pid))) = this.update(cx, |this, _| {
                    (this.visible && !this.exited).then_some((this.master_fd, this.shell_pid))
                }) else {
                    if this.update(cx, |_, _| ()).is_err() {
                        break;
                    }
                    continue;
                };
                let label = cx
                    .background_executor()
                    .spawn(async move { foreground_label(fd, pid) })
                    .await;
                if this
                    .update(cx, |this, cx| this.set_foreground(label, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            context_menu: None,
            term,
            sender,
            focus: cx.focus_handle(),
            title: default_title(&cwd),
            cwd,
            foreground: None,
            exited: false,
            size,
            geometry: Rc::default(),
            sized_by: None,
            master_fd,
            shell_pid,
            selecting: false,
            selection_pointer: None,
            selection_scroll: None,
            reported: None,
            scroll_remainder: 0.,
            url_regex: RegexSearch::new(URL_PATTERN).ok(),
            hover: None,
            pressed_link: None,
            visible: true,
            _events: events,
            _poll: poll,
        }
    }

    fn on_event(&mut self, event: TermEvent, cx: &mut Context<Self>) {
        match event {
            TermEvent::Wakeup | TermEvent::MouseCursorDirty | TermEvent::CursorBlinkingChange => {
                cx.notify()
            }
            TermEvent::PtyWrite(text) => self.write(text.into_bytes()),
            TermEvent::ColorRequest(index, format) => {
                let colors = *self.term.lock().colors();
                self.write(format(color_rgb(index, &colors)).into_bytes());
            }
            TermEvent::TextAreaSizeRequest(format) => {
                let cell = self
                    .geometry
                    .get()
                    .map_or(size(px(8.), px(16.)), |g| g.cell);
                let reply = format(WindowSize {
                    num_lines: self.size.lines as u16,
                    num_cols: self.size.columns as u16,
                    cell_width: cell.width.as_f32() as u16,
                    cell_height: cell.height.as_f32() as u16,
                });
                self.write(reply.into_bytes());
            }
            TermEvent::ClipboardStore(_, text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text))
            }
            TermEvent::ClipboardLoad(_, format) => {
                let text = cx
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .unwrap_or_default();
                self.write(format(&text).into_bytes());
            }
            TermEvent::ChildExit(status) => {
                let code = status.code().map(|c| format!(" ({c})")).unwrap_or_default();
                write_note(&self.term, &format!("\r\n[process exited{code}]"));
                self.finish(cx);
            }
            TermEvent::Exit => self.finish(cx),
            TermEvent::Title(_) | TermEvent::ResetTitle | TermEvent::Bell => {}
        }
    }
    fn finish(&mut self, cx: &mut Context<Self>) {
        if !self.exited {
            self.exited = true;
            self.sender = None;
            self.set_foreground(None, cx);
            cx.emit(TerminalEvent::Changed);
        }
        cx.notify();
    }
    /// The process the terminal started: the shell or the given program. Zero
    /// where it is not known.
    pub(crate) fn pid(&self) -> u32 {
        self.shell_pid
    }
    fn set_foreground(&mut self, label: Option<String>, cx: &mut Context<Self>) {
        if label == self.foreground {
            return;
        }
        self.title = label.clone().unwrap_or_else(|| default_title(&self.cwd));
        self.foreground = label;
        cx.emit(TerminalEvent::Changed);
    }
    /// Hangs up the job holding the terminal, leaving the shell at its prompt.
    pub(crate) fn stop_foreground(&mut self, cx: &mut Context<Self>) {
        #[cfg(unix)]
        if self.master_fd >= 0 && !self.exited {
            let pgrp = unsafe { libc::tcgetpgrp(self.master_fd) };
            if pgrp > 1 && pgrp != self.shell_pid as i32 {
                unsafe {
                    libc::kill(-pgrp, libc::SIGHUP);
                }
            }
        }
        self.set_foreground(None, cx);
    }
    fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(Msg::Input(bytes.into()));
        }
    }
    /// Typing returns a scrolled-back view to the prompt.
    fn input(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        self.stop_selection();
        {
            let mut term = self.term.lock();
            term.scroll_display(Scroll::Bottom);
            term.selection = None;
        }
        self.write(bytes);
        cx.notify();
    }
    /// Types `command` at the prompt and runs it. The footer names it at
    /// once; the foreground poll corrects the label within a second.
    pub(crate) fn run(&mut self, command: &str, cx: &mut Context<Self>) {
        let mut line = command.trim().replace("\r\n", "\r").replace('\n', "\r");
        line.push('\r');
        self.input(line.into_bytes(), cx);
        if let Some(label) = command_label(command.trim()) {
            self.set_foreground(Some(label), cx);
        }
    }
    /// Fit the grid to the painted area; the PTY learns its new size.
    fn resize(&mut self, columns: usize, lines: usize, cell: Size<gpui::Pixels>) {
        let next = GridSize { columns, lines };
        if next == self.size {
            return;
        }
        self.size = next;
        self.term.lock().resize(next);
        if let Some(sender) = &self.sender {
            let _ = sender.send(Msg::Resize(WindowSize {
                num_lines: lines as u16,
                num_cols: columns as u16,
                cell_width: cell.width.as_f32() as u16,
                cell_height: cell.height.as_f32() as u16,
            }));
        }
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(Msg::Shutdown);
        }
        // The hangup a closing shell expects, for it and its job.
        #[cfg(unix)]
        if self.shell_pid > 1 && !self.exited {
            let pid = self.shell_pid as i32;
            unsafe {
                libc::kill(pid, libc::SIGHUP);
                libc::kill(-pid, libc::SIGHUP);
            }
        }
    }
}

impl Render for Terminal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        div()
            .id("terminal")
            .key_context("Terminal")
            .track_focus(&self.focus)
            .size_full()
            .cursor(gpui::CursorStyle::IBeam)
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if this.hover != Some(event.position) {
                    this.hover = Some(event.position);
                    cx.notify();
                }
            }))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !hovered && this.hover.take().is_some() {
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(|this, event, _, cx| this.key_down(event, cx)))
            .on_action(cx.listener(|this, _: &SendTab, _, cx| this.input(b"\t".to_vec(), cx)))
            .on_action(
                cx.listener(|this, _: &SendBackTab, _, cx| this.input(b"\x1b[Z".to_vec(), cx)),
            )
            .on_action(cx.listener(|this, _: &SendEscape, _, cx| this.input(b"\x1b".to_vec(), cx)))
            .on_action(cx.listener(|this, _: &Copy, _, cx| this.copy(cx)))
            .on_action(cx.listener(|this, _: &Paste, _, cx| this.paste(cx)))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.select_all(cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    if cfg!(target_os = "macos") && event.modifiers.control {
                        this.open_context_menu(event, window, cx);
                    } else {
                        this.mouse_down(event, window, cx);
                    }
                }),
            )
            .on_mouse_down(MouseButton::Right, cx.listener(Self::open_context_menu))
            .on_scroll_wheel(cx.listener(|this, event, _, cx| this.scroll(event, cx)))
            .child(Grid {
                terminal: cx.entity(),
                focused,
            })
            .children(self.editing_menu(cx))
    }
}

/// One shaped run at a column, or a filled cell span.
struct Painted {
    lines: Vec<(gpui::Point<gpui::Pixels>, ShapedLine)>,
    rects: Vec<(Bounds<gpui::Pixels>, Hsla)>,
    cursor: Option<(Bounds<gpui::Pixels>, Hsla, bool)>,
    line_height: gpui::Pixels,
    /// Set while the pointer is over a link, for the pointing hand.
    link_hitbox: Option<gpui::Hitbox>,
}

/// A cell to paint: column, glyph, pen, fill and whether it is wide.
type Painting = (usize, char, Pen, Option<Hsla>, bool);
#[derive(Clone, PartialEq)]
struct Pen {
    fg: Hsla,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
}
fn cell_rect(
    origin: gpui::Point<gpui::Pixels>,
    cell: Size<gpui::Pixels>,
    line: usize,
    start: usize,
    end: usize,
) -> Bounds<gpui::Pixels> {
    Bounds::new(
        point(
            origin.x + cell.width * start as f32,
            origin.y + cell.height * line as f32,
        ),
        size(cell.width * (end - start) as f32, cell.height),
    )
}

/// Only HTTP(S) opens, as TerminalView's `openTerminalLink` allows.
fn is_web_link(uri: &str) -> bool {
    let lower = uri.get(..8).unwrap_or(uri).to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}
fn selection_fill() -> Hsla {
    crate::ui::theme::text_selection()
}
fn write_note(term: &Arc<FairMutex<Term<Listener>>>, text: &str) {
    let mut parser = alacritty_terminal::vte::ansi::Processor::<
        alacritty_terminal::vte::ansi::StdSyncHandler,
    >::new();
    parser.advance(&mut *term.lock(), text.as_bytes());
}

/// TerminalView's ANSI_DARK palette.
const ANSI: [u32; 16] = [
    0x1d2428, 0xf87171, 0x4ade80, 0xfbbf24, 0x60a5fa, 0xc084fc, 0x22d3ee, 0xe8eef2, 0x64748b,
    0xfca5a5, 0x86efac, 0xfde68a, 0x93c5fd, 0xd8b4fe, 0x67e8f9, 0xf8fafc,
];
/// oscColors(): what a CLI asking for colours is told.
const OSC_FOREGROUND: u32 = TEXT;
const OSC_BACKGROUND: u32 = 0x171717;

fn indexed_rgb(index: u8) -> u32 {
    match index {
        0..=15 => ANSI[index as usize],
        16..=231 => {
            let i = index - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v as u32 * 40 };
            (level(i / 36) << 16) | (level((i / 6) % 6) << 8) | level(i % 6)
        }
        _ => {
            let v = 8 + (index as u32 - 232) * 10;
            (v << 16) | (v << 8) | v
        }
    }
}
fn hex(rgb: Rgb) -> u32 {
    ((rgb.r as u32) << 16) | ((rgb.g as u32) << 8) | rgb.b as u32
}
fn to_rgb(value: u32) -> Rgb {
    Rgb {
        r: (value >> 16) as u8,
        g: (value >> 8) as u8,
        b: value as u8,
    }
}
/// Reply colour for an OSC 4/10/11/12 query: an app-set colour wins.
fn color_rgb(index: usize, colors: &Colors) -> Rgb {
    if let Some(rgb) = colors[index] {
        return rgb;
    }
    to_rgb(match index {
        0..=255 => indexed_rgb(index as u8),
        256 => OSC_FOREGROUND,
        257 => OSC_BACKGROUND,
        258 => ACCENT,
        _ => OSC_FOREGROUND,
    })
}
fn default_title(cwd: &str) -> String {
    let name = std::path::Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.is_empty() || name == "/" {
        "Terminal".into()
    } else {
        name
    }
}
fn home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from)
}
fn working_dir(cwd: &str) -> std::path::PathBuf {
    let path = match cwd.strip_prefix("~") {
        Some(rest) => home().map_or_else(|| cwd.into(), |h| h.join(rest.trim_start_matches('/'))),
        None => std::path::PathBuf::from(cwd),
    };
    if path.is_dir() {
        path
    } else {
        home().unwrap_or(path)
    }
}
/// pty.rs's foreground_label: the command holding the terminal, unless it
/// is the shell itself.
#[cfg(unix)]
fn foreground_label(master_fd: i32, shell_pid: u32) -> Option<String> {
    if master_fd < 0 {
        return None;
    }
    let pgrp = unsafe { libc::tcgetpgrp(master_fd) };
    if pgrp <= 0 || pgrp == shell_pid as i32 {
        return None;
    }
    let output = std::process::Command::new("ps")
        .args(["-p", &pgrp.to_string(), "-o", "args="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let label = command_label(String::from_utf8_lossy(&output.stdout).trim())?;
    (!is_shell_name(&label)).then_some(label)
}
#[cfg(not(unix))]
fn foreground_label(_: i32, _: u32) -> Option<String> {
    None
}
/// Interpreters name the script they run: `node vite.js` reads `vite.js`.
pub(crate) fn command_label(args: &str) -> Option<String> {
    let mut parts = args.split_whitespace();
    let exe = parts.next()?;
    let base = std::path::Path::new(exe).file_name()?.to_str()?.to_owned();
    let base = base.trim_start_matches('-').to_owned();
    if matches!(
        base.as_str(),
        "node" | "nodejs" | "python" | "python3" | "ruby" | "deno" | "bun"
    ) {
        for part in parts {
            if part.starts_with('-') {
                continue;
            }
            let name = std::path::Path::new(part).file_name()?.to_str()?;
            if !name.starts_with('-') {
                return Some(name.to_owned());
            }
        }
    }
    Some(base)
}
/// The supported agent CLI a foreground label names, as a provider id, so
/// its tab can show the provider's mark instead of the terminal icon.
pub(crate) fn agent_cli(label: &str) -> Option<&'static str> {
    // Script launchers report the script: `codex.js` is Codex.
    let name = label.split('.').next()?;
    match name {
        "cursor-agent" | "agent" => return Some("cursor"),
        "pi-coding-agent" => return Some("pi"),
        _ => {}
    }
    monocode_core::provider::Kind::parse(name).map(|kind| kind.id())
}
impl Terminal {
    /// The agent CLI holding this terminal, if any.
    pub(crate) fn agent(&self) -> Option<&'static str> {
        agent_cli(self.foreground.as_deref()?)
    }
}
/// Only Unix reads the foreground job.
#[cfg(any(unix, test))]
fn is_shell_name(name: &str) -> bool {
    matches!(
        name,
        "zsh" | "bash" | "sh" | "fish" | "nu" | "dash" | "ksh" | "tcsh" | "zsh5"
    )
}

/// The xterm button-code bits for the modifiers held during a mouse report.
fn mouse_modifiers(modifiers: &gpui::Modifiers) -> u8 {
    u8::from(modifiers.alt) * 8 + u8::from(modifiers.control) * 16
}

/// One mouse report for an xterm button code (0 left, +32 while dragging,
/// 64/65 a wheel notch), SGR when asked for, else the legacy X10 encoding
/// (which cannot address past column or line 223).
fn mouse_report(button: u8, release: bool, line: usize, column: usize, mode: TermMode) -> Vec<u8> {
    if mode.contains(TermMode::SGR_MOUSE) {
        let end = if release { 'm' } else { 'M' };
        format!("\x1b[<{button};{};{}{end}", column + 1, line + 1).into_bytes()
    } else {
        // X10 has no release per button: every release is button 3.
        let button = if release { button | 3 } else { button };
        let cell = |n: usize| (n.min(222) + 33) as u8;
        vec![0x1b, b'[', b'M', button + 32, cell(column), cell(line)]
    }
}

#[cfg(test)]
mod tests;
