// Native text-input plumbing adapted from GPUI's Apache-2.0 examples/input.rs.
// Copyright Zed Industries. See NOTICE for source and license.
use gpui::{
    App, AvailableSpace, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, Element,
    ElementId, ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle,
    Focusable, GlobalElementId, Hsla, ImgResourceLoader, InspectorElementId, KeyBinding, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Resource, Role,
    SharedString, Style, TextAlign, TextRun, UTF16Selection, UnderlineStyle, Window, WrappedLine,
    actions, div, fill, point, prelude::*, px, relative, rgb, size,
};
use std::{borrow::Cow, ops::Range};
use unicode_segmentation::UnicodeSegmentation;

mod command;
mod context_menu;
mod editing;
mod element;
mod mention;
mod spelling;
mod text_input;
pub(crate) use command::{Command, CommandRemoved};
use element::InputElement;
use mention::{DisplayLayout, MentionIcon};

const MAX_BYTES: usize = 64 * 1024;
const LINE_HEIGHT: f32 = 22.;
// The composer field starts two lines tall and grows to 140px before
// scrolling. The enclosing composer supplies its own vertical padding.
const MIN_HEIGHT: f32 = LINE_HEIGHT * 2.;
const MAX_HEIGHT: f32 = 140.;
const FONT_SIZE: f32 = 14.;
actions!(
    composer,
    [
        FoldCode,
        UnfoldCode,
        FoldAll,
        UnfoldAll,
        Complete,
        DismissCompletion,
        Undo,
        Redo,
        Indent,
        Outdent,
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectAll,
        Home,
        End,
        SelectHome,
        SelectEnd,
        DocumentStart,
        DocumentEnd,
        SelectDocumentStart,
        SelectDocumentEnd,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        DeleteWordLeft,
        DeleteWordRight,
        DeleteToHome,
        DeleteToEnd,
        Paste,
        Cut,
        Copy,
        Newline,
        Submit,
        SubmitSecondary,
        Up,
        Down
    ]
);
pub struct Submitted;
/// Cmd/Ctrl+Enter in a composer that sends either way, asking for the
/// opposite of its usual follow-up behavior.
pub struct AlternateSubmitted;
pub struct Changed;
pub(crate) enum CodeAction {
    Stage(usize),
    Revert(usize),
}
pub struct Rejected(pub &'static str);

pub fn bind(cx: &mut App) {
    let selection_modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    cx.bind_keys([KeyBinding::new(
        &format!("{selection_modifier}-c"),
        Copy,
        Some("TranscriptSelection"),
    )]);
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, Some("Composer")),
        KeyBinding::new("delete", Delete, Some("Composer")),
        KeyBinding::new("left", Left, Some("Composer")),
        KeyBinding::new("right", Right, Some("Composer")),
        KeyBinding::new("shift-left", SelectLeft, Some("Composer")),
        KeyBinding::new("shift-right", SelectRight, Some("Composer")),
        KeyBinding::new("up", Up, Some("Composer")),
        KeyBinding::new("down", Down, Some("Composer")),
        KeyBinding::new("shift-up", SelectUp, Some("Composer")),
        KeyBinding::new("shift-down", SelectDown, Some("Composer")),
        KeyBinding::new("home", Home, Some("Composer")),
        KeyBinding::new("end", End, Some("Composer")),
        KeyBinding::new("shift-home", SelectHome, Some("Composer")),
        KeyBinding::new("shift-end", SelectEnd, Some("Composer")),
        KeyBinding::new("shift-enter", Newline, Some("Composer")),
        KeyBinding::new("enter", Submit, Some("Composer")),
    ]);
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    cx.bind_keys([
        KeyBinding::new(&format!("{modifier}-z"), Undo, Some("CodeEditor")),
        KeyBinding::new(&format!("{modifier}-shift-z"), Redo, Some("CodeEditor")),
        KeyBinding::new("ctrl-space", Complete, Some("CodeEditor")),
        KeyBinding::new("cmd-alt-[", FoldCode, Some("CodeEditor")),
        KeyBinding::new("cmd-alt-]", UnfoldCode, Some("CodeEditor")),
        KeyBinding::new("ctrl-shift-[", FoldCode, Some("CodeEditor")),
        KeyBinding::new("ctrl-shift-]", UnfoldCode, Some("CodeEditor")),
        KeyBinding::new("cmd-alt-shift-[", FoldAll, Some("CodeEditor")),
        KeyBinding::new("cmd-alt-shift-]", UnfoldAll, Some("CodeEditor")),
        KeyBinding::new("escape", DismissCompletion, Some("CodeEditor")),
        KeyBinding::new("tab", Indent, Some("CodeEditor")),
        KeyBinding::new("shift-tab", Outdent, Some("CodeEditor")),
    ]);
    cx.bind_keys([
        KeyBinding::new(&format!("{modifier}-a"), SelectAll, Some("Composer")),
        KeyBinding::new(&format!("{modifier}-c"), Copy, Some("Composer")),
        KeyBinding::new(&format!("{modifier}-v"), Paste, Some("Composer")),
        KeyBinding::new(&format!("{modifier}-x"), Cut, Some("Composer")),
        KeyBinding::new(
            &format!("{modifier}-enter"),
            SubmitSecondary,
            Some("Composer"),
        ),
    ]);
    if cfg!(target_os = "macos") {
        cx.bind_keys([
            KeyBinding::new("cmd-left", Home, Some("Composer")),
            KeyBinding::new("cmd-right", End, Some("Composer")),
            KeyBinding::new("cmd-shift-left", SelectHome, Some("Composer")),
            KeyBinding::new("cmd-shift-right", SelectEnd, Some("Composer")),
            KeyBinding::new("cmd-up", DocumentStart, Some("Composer")),
            KeyBinding::new("cmd-down", DocumentEnd, Some("Composer")),
            KeyBinding::new("cmd-shift-up", SelectDocumentStart, Some("Composer")),
            KeyBinding::new("cmd-shift-down", SelectDocumentEnd, Some("Composer")),
            KeyBinding::new("cmd-backspace", DeleteToHome, Some("Composer")),
            KeyBinding::new("cmd-delete", DeleteToEnd, Some("Composer")),
        ]);
    } else {
        cx.bind_keys([
            KeyBinding::new("ctrl-home", DocumentStart, Some("Composer")),
            KeyBinding::new("ctrl-end", DocumentEnd, Some("Composer")),
            KeyBinding::new("ctrl-shift-home", SelectDocumentStart, Some("Composer")),
            KeyBinding::new("ctrl-shift-end", SelectDocumentEnd, Some("Composer")),
        ]);
    }
    let word_modifier = if cfg!(target_os = "macos") {
        "alt"
    } else {
        "ctrl"
    };
    cx.bind_keys([
        KeyBinding::new(&format!("{word_modifier}-left"), WordLeft, Some("Composer")),
        KeyBinding::new(
            &format!("{word_modifier}-right"),
            WordRight,
            Some("Composer"),
        ),
        KeyBinding::new(
            &format!("{word_modifier}-shift-left"),
            SelectWordLeft,
            Some("Composer"),
        ),
        KeyBinding::new(
            &format!("{word_modifier}-shift-right"),
            SelectWordRight,
            Some("Composer"),
        ),
        KeyBinding::new(
            &format!("{word_modifier}-backspace"),
            DeleteWordLeft,
            Some("Composer"),
        ),
        KeyBinding::new(
            &format!("{word_modifier}-delete"),
            DeleteWordRight,
            Some("Composer"),
        ),
    ]);
}

pub struct Input {
    context_menu: Option<context_menu::EditingMenu>,
    pub(crate) code: Option<crate::editor::code_input::Code>,
    pub(crate) focus: FocusHandle,
    height: f32,
    single_line: bool,
    pub(crate) read_only: bool,
    secret: bool,
    max_chars: Option<usize>,
    centered: bool,
    pub(crate) placeholder: SharedString,
    placeholder_alpha: Option<f32>,
    pub(crate) label: SharedString,
    pub(crate) font_size: f32,
    pub(crate) line_height: f32,
    min_height: f32,
    max_height: f32,
    /// A textarea that takes Enter as a newline and submits on Cmd/Ctrl+Enter.
    pub(crate) secondary_submit: bool,
    /// Emit [`AlternateSubmitted`] on Cmd/Ctrl+Enter instead of ignoring it.
    alternate_submit: bool,
    pub content: SharedString,
    pub(crate) selection: Range<usize>,
    pub(crate) reversed: bool,
    pub(crate) marked: Option<Range<usize>>,
    composer_tokens: Vec<Range<usize>>,
    inline_commands: bool,
    selected_command: Option<Command>,
    lines: Vec<WrappedLine>,
    pub(crate) bounds: Option<Bounds<Pixels>>,
    pub(crate) scroll_x: Pixels,
    pub(crate) scroll_y: Pixels,
    manual_scroll: bool,
    reveal_in_parent: bool,
    reveal_pending: bool,
    pub(crate) selecting: bool,
    drag_position: Option<Point<Pixels>>,
    pub(crate) autoscroll: Option<gpui::Task<()>>,
}
impl EventEmitter<Submitted> for Input {}
impl EventEmitter<AlternateSubmitted> for Input {}
impl EventEmitter<Changed> for Input {}
impl EventEmitter<Rejected> for Input {}
impl EventEmitter<CodeAction> for Input {}
impl Focusable for Input {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Input {
    /// Keep authentication text out of drawing, accessibility and clipboard.
    pub(crate) fn set_secret(&mut self, secret: bool) {
        self.secret = secret;
    }
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            context_menu: None,
            code: None,
            focus: cx.focus_handle(),
            height: MIN_HEIGHT,
            single_line: false,
            read_only: false,
            secret: false,
            max_chars: None,
            centered: false,
            placeholder: "Ask, build, or describe a change…".into(),
            placeholder_alpha: None,
            label: "Message composer".into(),
            font_size: FONT_SIZE,
            line_height: LINE_HEIGHT,
            min_height: MIN_HEIGHT,
            max_height: MAX_HEIGHT,
            secondary_submit: false,
            alternate_submit: false,
            content: "".into(),
            selection: 0..0,
            reversed: false,
            marked: None,
            composer_tokens: Vec::new(),
            inline_commands: false,
            selected_command: None,
            lines: Vec::new(),
            bounds: None,
            scroll_x: px(0.),
            scroll_y: px(0.),
            manual_scroll: false,
            reveal_in_parent: false,
            reveal_pending: false,
            selecting: false,
            drag_position: None,
            autoscroll: None,
        }
    }
    pub fn search(cx: &mut Context<Self>) -> Self {
        let mut input = Self::new(cx);
        input.height = 20.;
        input.single_line = true;
        input.placeholder = "Search".into();
        input.label = "Search sessions".into();
        input
    }
    /// A one-line field inside a popover, such as a picker's search box.
    /// Only `placeholder_color`'s alpha is kept; its ink follows the theme.
    pub fn field(
        label: impl Into<SharedString>,
        placeholder: impl Into<SharedString>,
        placeholder_color: Hsla,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut input = Self::search(cx);
        input.label = label.into();
        input.placeholder = placeholder.into();
        input.placeholder_alpha = Some(placeholder_color.a);
        // Picker search fields are `text-[13px]`.
        input.font_size = 13.;
        input
    }
    /// The large single-line field heading a command panel.
    pub fn command_field(
        label: impl Into<SharedString>,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut input = Self::search(cx);
        input.label = label.into();
        input.placeholder = placeholder.into();
        input.placeholder_alpha = Some(0.38);
        input.font_size = 17.;
        input.line_height = 24.;
        input.height = 24.;
        input
    }
    /// GitChangesPanel's commit message: a one-row `text-[13px] leading-5`
    /// textarea growing to `max-h-40` less its `py-1`.
    pub fn commit_message(cx: &mut Context<Self>) -> Self {
        let mut input = Self::new(cx);
        input.label = "Commit message".into();
        input.font_size = 13.;
        input.line_height = 20.;
        input.height = 20.;
        input.min_height = 20.;
        input.max_height = 152.;
        input.placeholder_alpha = Some(0.35);
        input.secondary_submit = true;
        input
    }
    pub(crate) fn multiline_field(label: &str, placeholder: &str, cx: &mut Context<Self>) -> Self {
        let mut input = Self::commit_message(cx);
        input.label = label.to_owned().into();
        input.font_size = 12.;
        input.placeholder = placeholder.to_owned().into();
        input.height = 124.;
        input.min_height = 124.;
        input.max_height = 124.;
        input
    }
    /// An action's shell command: three rows growing to six.
    pub(crate) fn action_command(cx: &mut Context<Self>) -> Self {
        let mut input = Self::commit_message(cx);
        input.label = "Command".into();
        input.placeholder = "bun test".into();
        input.height = 60.;
        input.min_height = 60.;
        input.max_height = 120.;
        input
    }
    /// InboxComments' two-row textarea grows within the discussion page.
    pub(crate) fn inbox_comment(cx: &mut Context<Self>) -> Self {
        let mut input = Self::commit_message(cx);
        input.label = "Comment".into();
        input.placeholder = if cfg!(target_os = "macos") {
            "Leave a comment (⌘↩)"
        } else {
            "Leave a comment (Ctrl+Enter)"
        }
        .into();
        input.height = 40.;
        input.min_height = 40.;
        input.max_height = 144.;
        input
    }
    /// AutomationEditor uses a proportional, naturally growing prompt field.
    pub(crate) fn automation_instructions(cx: &mut Context<Self>) -> Self {
        let mut input = Self::commit_message(cx);
        input.label = "Instructions".into();
        input.placeholder = "Tell the agent what to do when this automation runs…".into();
        input.font_size = 14.;
        input.line_height = 22.;
        input.height = 88.;
        input.min_height = 88.;
        input.max_height = f32::MAX;
        input.reveal_in_parent = true;
        input
    }
    pub(crate) fn automation_name(cx: &mut Context<Self>) -> Self {
        let mut input = Self::search(cx);
        input.label = "Automation name".into();
        input.placeholder = "Untitled".into();
        input.font_size = 20.;
        input.line_height = 25.;
        input.height = 25.;
        input
    }
    /// The session composer: one line tall until the text grows.
    pub fn session_composer(cx: &mut Context<Self>) -> Self {
        let mut input = Self::new(cx);
        input.min_height = LINE_HEIGHT;
        input.height = LINE_HEIGHT;
        input
    }
    /// The Mono chat's composer, as tall as the session composer.
    pub(crate) fn mono_composer(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.min_height != LINE_HEIGHT {
            self.height = LINE_HEIGHT;
        }
        self.label = format!("Message {name}").into();
        self.placeholder = self.label.clone();
        self.font_size = 13.;
        self.line_height = LINE_HEIGHT;
        self.min_height = LINE_HEIGHT;
        self.max_height = 144.;
        self.height = self.height.clamp(self.min_height, self.max_height);
        cx.notify();
    }
    /// NewHabitPage's three-row AutoTextarea, capped like React's AutoTextarea.
    pub(crate) fn habit_instructions(cx: &mut Context<Self>) -> Self {
        let mut input = Self::commit_message(cx);
        input.label = "What it does".into();
        input.placeholder =
            "What it should do on each run, and when it's worth telling you about.".into();
        input.font_size = 12.;
        input.line_height = 20.;
        input.height = 60.;
        input.min_height = 60.;
        input.max_height = 320.;
        input.max_chars = Some(4000);
        input
    }
    pub(crate) fn markdown_editor(label: &str, cx: &mut Context<Self>) -> Self {
        let mut input = Self::commit_message(cx);
        input.label = label.to_string().into();
        input.min_height = 120.;
        input.max_height = 2000.;
        input.height = 120.;
        input
    }
    pub(crate) fn set_placeholder(
        &mut self,
        text: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        let text = text.into();
        if self.placeholder != text {
            self.placeholder = text;
            cx.notify();
        }
    }
    pub(crate) fn set_alternate_submit(&mut self, value: bool) {
        self.alternate_submit = value;
    }
    pub(crate) fn set_read_only(&mut self, value: bool) {
        self.read_only = value;
    }
    pub(crate) fn set_max_chars(&mut self, value: usize) {
        self.max_chars = Some(value);
    }
    pub(crate) fn set_font_size(&mut self, size: f32) {
        self.font_size = size;
    }
    pub(crate) fn set_centered(&mut self, centered: bool) {
        self.centered = centered;
    }
    pub(crate) fn set_min_height(&mut self, height: f32) {
        self.min_height = height;
        self.height = self.height.max(height);
    }
    pub(crate) fn set_max_height(&mut self, height: f32) {
        self.max_height = height;
    }
    pub(crate) fn set_line_height(&mut self, height: f32) {
        self.line_height = height;
    }
    pub(crate) fn memory_fact(label: &str, cx: &mut Context<Self>) -> Self {
        let mut input = Self::commit_message(cx);
        input.label = label.to_string().into();
        input.font_size = 12.;
        input.line_height = 18.;
        input.min_height = 18.;
        input.max_height = 320.;
        input.height = 18.;
        input.secondary_submit = false;
        input
    }
    /// MessageQueue.tsx keeps its one-row textarea height while Shift+Enter
    /// inserts newlines; the field scrolls to the caret within that row.
    pub(crate) fn queued_message(cx: &mut Context<Self>) -> Self {
        let mut input = Self::memory_fact("Edit queued message", cx);
        input.max_height = input.min_height;
        input
    }
    pub fn take(&mut self, cx: &mut Context<Self>) -> Option<String> {
        if self.is_composing() || self.content.trim().is_empty() {
            return None;
        }
        let text = self.content.to_string();
        self.content = "".into();
        self.lines.clear();
        self.selection = 0..0;
        self.reversed = false;
        self.scroll_x = px(0.);
        self.scroll_y = px(0.);
        cx.notify();
        Some(text)
    }
    /// An empty composer hands its document and line keys (Cmd+arrows) on to
    /// the workspace's session and project navigation, as React's
    /// `emptyComposer` condition does. Returns whether it did.
    fn yields_navigation(&self, cx: &mut Context<Self>) -> bool {
        let yields = self.code.is_none()
            && !self.single_line
            && self.content.is_empty()
            && !self.is_composing();
        if yields {
            cx.propagate();
        }
        yields
    }
    pub(crate) fn set_text(&mut self, text: String, cx: &mut Context<Self>) {
        self.manual_scroll = false;
        self.reveal_pending = self.reveal_in_parent;
        self.clear_folds();
        self.content = text.into();
        self.lines.clear();
        self.code_changed();
        self.selection = self.content.len()..self.content.len();
        self.reversed = false;
        self.marked = None;
        self.scroll_x = px(0.);
        self.scroll_y = px(0.);
        cx.emit(Changed);
        cx.notify();
    }
    pub fn is_composing(&self) -> bool {
        self.marked.is_some()
    }
    pub(crate) fn selection_dragging(&self) -> bool {
        self.selecting
    }
    pub(crate) fn selection_anchor(&self) -> Option<Bounds<Pixels>> {
        if self.selection.is_empty() {
            return None;
        }
        let bounds = self.bounds?;
        let origin =
            bounds.origin + self.position(self.cursor()) - point(self.scroll_x, self.scroll_y);
        bounds
            .contains(&origin)
            .then(|| Bounds::new(origin, size(px(1.), self.line_height())))
    }
    pub(crate) fn cursor_anchor(&self) -> Option<Bounds<Pixels>> {
        let bounds = self.bounds?;
        let origin =
            bounds.origin + self.position(self.cursor()) - point(self.scroll_x, self.scroll_y);
        Some(Bounds::new(origin, size(px(1.), self.line_height())))
    }
    pub(crate) fn completion_token(&self) -> Option<monocode_core::session::composer::Token> {
        if self.is_composing() || !self.selection.is_empty() {
            return None;
        }
        monocode_core::session::composer::token_at(&self.content, self.cursor())
    }
    pub(crate) fn highlight_composer_tokens(
        &mut self,
        ranges: Vec<Range<usize>>,
        cx: &mut Context<Self>,
    ) {
        if self.composer_tokens != ranges {
            self.composer_tokens = ranges;
            cx.notify();
        }
    }
    pub(crate) fn replace_completion(
        &mut self,
        range: Range<usize>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selection = self.clip_range(range);
        self.replace_text_in_range(None, text, window, cx);
    }
}
/// GPUI only delivers `on_mouse_move` while the field is hovered, so a drag
/// selection listens at the window level until the button is released.
pub(crate) fn paint_drag_listener(input: &Entity<Input>, w: &mut Window, cx: &App) {
    if !input.read(cx).selecting {
        return;
    }
    let input = input.clone();
    w.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
        if phase == gpui::DispatchPhase::Capture {
            input.update(cx, |input, cx| input.drag_move(e, cx));
        }
    });
}
fn from16(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units + ch.len_utf16() > offset {
            return byte;
        }
        units += ch.len_utf16();
    }
    text.len()
}

fn limit_utf16(text: &str, units: usize) -> String {
    let mut used = 0;
    text.chars()
        .take_while(|c| {
            used += c.len_utf16();
            used <= units
        })
        .collect()
}
fn bounded(text: &str, max: usize) -> &str {
    let mut n = max.min(text.len());
    while !text.is_char_boundary(n) {
        n -= 1;
    }
    &text[..n]
}

struct PaintState {
    lines: Vec<WrappedLine>,
    scroll: Point<Pixels>,
    layout: DisplayLayout,
}
impl Render for Input {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("native-composer")
            .relative()
            .role(if self.single_line {
                Role::TextInput
            } else {
                Role::MultilineTextInput
            })
            .aria_label(self.label.clone())
            .aria_value(if self.secret {
                "*".repeat(self.content.chars().count()).into()
            } else {
                self.content.clone()
            })
            .tab_index(0)
            .w_full()
            .min_w_0()
            .key_context(if self.code.is_some() {
                "Composer CodeEditor"
            } else {
                "Composer"
            })
            .when(self.code.is_some(), |d| {
                d.h_full().font_family(crate::ui::fonts::mono())
            })
            .track_focus(&self.focus)
            .cursor(if self.code_rail_hovered() {
                CursorStyle::Arrow
            } else {
                CursorStyle::IBeam
            })
            .on_action(cx.listener(Self::start_completion))
            .on_action(cx.listener(Self::dismiss_completion))
            .on_action(cx.listener(Self::fold))
            .on_action(cx.listener(Self::unfold))
            .on_action(cx.listener(Self::fold_all))
            .on_action(cx.listener(Self::unfold_all))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::indent))
            .on_action(cx.listener(Self::outdent))
            .on_scroll_wheel(cx.listener(Self::scroll))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::document_start))
            .on_action(cx.listener(Self::document_end))
            .on_action(cx.listener(Self::select_document_start))
            .on_action(cx.listener(Self::select_document_end))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete_word_right))
            .on_action(cx.listener(Self::delete_to_home))
            .on_action(cx.listener(Self::delete_to_end))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::submit_secondary))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::open_context_menu))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .child(if self.code.is_some() {
                crate::editor::code_input::paint::CodeElement { input: cx.entity() }
                    .into_any_element()
            } else {
                InputElement { input: cx.entity() }.into_any_element()
            })
            .children(self.code_tooltip())
            .children(self.completion_popup(cx))
            .children(self.hunk_actions(cx))
            .children(self.editing_menu(cx))
    }
}

#[cfg(test)]
mod tests;
