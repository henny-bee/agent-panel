//! UnifiedDiffView.tsx and unifiedDiffDocument.ts: every file's hunks in one
//! virtualized review, with sticky file headers, a fixed gutter beside code
//! lanes that scroll horizontally together, hunk navigation, per-file and
//! per-hunk staging, and line comments for the chat. Highlighting runs off
//! the UI thread, one file at a time, only for files that are on screen.
mod view;
use crate::ui::{
    file_icons::file_type_icon,
    input::{self, Input},
    panel::{format_integer, stroke, tabular},
    popover::{AnchorSlot, Side, popover},
    syntax_palette::{HighlightRequest, HighlightSpan},
    theme::{DIFF_ADD, DIFF_DEL, *},
};
use gpui::{
    AnyElement, App, Bounds, Context, EventEmitter, FocusHandle, Focusable, FontWeight,
    HighlightStyle, Hsla, KeyBinding, ListAlignment, ListOffset, ListState, MouseButton,
    MouseDownEvent, MouseMoveEvent, Pixels, Role, ScrollWheelEvent, SharedString, StyledText,
    Subscription, Task, Window, actions, canvas, div, list, prelude::*, px, rgb,
};
use monocode_core::vcs::diff::{self, Block, Line, LineKind};
use std::{
    borrow::Cow,
    cell::Cell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

pub(crate) const FILE_HEADER: f32 = 40.;
const LINE: f32 = 20.;
const HUNK: f32 = 28.;
const NOTICE: f32 = 44.;
/// `--code-font-size`.
const CODE_SIZE: f32 = 12.5;
const MAX_HIGHLIGHT_CHARS: usize = 250_000;

actions!(diff_review, [PreviousHunk, NextHunk]);

pub(crate) fn bind(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("alt-up", PreviousHunk, Some("DiffReview")),
        KeyBinding::new("alt-down", NextHunk, Some("DiffReview")),
    ]);
}

/// UnifiedDiffFileModel.
#[derive(Clone)]
pub(crate) struct DiffFile {
    pub id: SharedString,
    pub path: String,
    pub label: String,
    pub binary: bool,
    pub too_large: bool,
    pub empty_message: Option<String>,
    pub additions: u32,
    pub deletions: u32,
    pub blocks: Arc<Vec<Block>>,
    pub can_stage: bool,
    pub can_discard: bool,
    pub can_stage_hunk: bool,
}
impl DiffFile {
    fn message(&self) -> Option<String> {
        if self.binary {
            Some("Binary file changed".into())
        } else if self.too_large {
            Some("Diff is too large to display".into())
        } else if let Some(message) = &self.empty_message {
            Some(message.clone())
        } else if self.blocks.is_empty() {
            Some("No textual diff".into())
        } else {
            None
        }
    }
    fn same_content(&self, other: &DiffFile) -> bool {
        Arc::ptr_eq(&self.blocks, &other.blocks)
            && self.label == other.label
            && self.binary == other.binary
            && self.too_large == other.too_large
            && self.empty_message == other.empty_message
            && self.additions == other.additions
            && self.deletions == other.deletions
            && (self.can_stage, self.can_discard, self.can_stage_hunk)
                == (other.can_stage, other.can_discard, other.can_stage_hunk)
    }
}

pub(crate) enum DiffEvent {
    StageFile(SharedString),
    DiscardFile(SharedString),
    StageHunk(SharedString, usize),
    /// Text for the active chat's composer.
    Comment(String),
}

#[derive(Clone, PartialEq)]
enum RowKind {
    File,
    Line {
        block: usize,
        line: usize,
        stage: Option<usize>,
    },
    Hunk(SharedString),
    Notice(SharedString),
}
#[derive(Clone)]
struct Row {
    file: usize,
    kind: RowKind,
    height: f32,
}
#[derive(Clone, PartialEq, Eq, Hash)]
struct RowKey(SharedString, u8, usize, usize);

struct Body {
    blocks: Arc<Vec<Block>>,
    message: Option<String>,
    stageable: bool,
    rows: Vec<Row>,
    columns: usize,
    digits: usize,
}
fn build_body(file: &DiffFile, message: Option<String>, stageable: bool) -> Body {
    let mut rows = Vec::new();
    let (mut columns, mut digits) = (40, 3);
    if let Some(text) = &message {
        rows.push(Row {
            file: 0,
            kind: RowKind::Notice(text.clone().into()),
            height: NOTICE,
        });
    } else {
        for (b, block) in file.blocks.iter().enumerate() {
            let Block::Hunk { lines, chunk } = block else {
                continue;
            };
            let changes = lines
                .iter()
                .any(|l| matches!(l.kind, LineKind::Add | LineKind::Del));
            if changes && !lines.iter().any(|l| l.kind == LineKind::Hunk) {
                rows.push(Row {
                    file: 0,
                    kind: RowKind::Hunk(hunk_range(lines).into()),
                    height: HUNK,
                });
            }
            for (i, line) in lines.iter().enumerate() {
                if line.kind == LineKind::Hunk {
                    rows.push(Row {
                        file: 0,
                        kind: RowKind::Hunk(line.text.clone().into()),
                        height: HUNK,
                    });
                    continue;
                }
                let stage = (stageable && matches!(line.kind, LineKind::Add | LineKind::Del))
                    .then(|| line.chunk.or(*chunk))
                    .flatten();
                rows.push(Row {
                    file: 0,
                    kind: RowKind::Line {
                        block: b,
                        line: i,
                        stage,
                    },
                    height: LINE,
                });
                columns = columns.max(diff::visual_columns(&line.text));
                digits = digits
                    .max(line.old_number.unwrap_or(0).to_string().len())
                    .max(line.new_number.unwrap_or(0).to_string().len());
            }
        }
    }
    Body {
        blocks: file.blocks.clone(),
        message,
        stageable,
        rows,
        columns,
        digits,
    }
}
fn hunk_range(lines: &[Line]) -> String {
    let old: Vec<u32> = lines.iter().filter_map(|l| l.old_number).collect();
    let new: Vec<u32> = lines.iter().filter_map(|l| l.new_number).collect();
    format!(
        "@@ -{},{} +{},{} @@",
        old.first().unwrap_or(&0),
        old.len(),
        new.first().unwrap_or(&0),
        new.len()
    )
}
fn expand_tabs(text: &str) -> Cow<'_, str> {
    if !text.contains('\t') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 8);
    let mut columns = 0;
    for ch in text.chars() {
        if ch == '\t' {
            let n = 4 - columns % 4;
            out.extend(std::iter::repeat_n(' ', n));
            columns += n;
        } else {
            out.push(ch);
            columns += 1;
        }
    }
    Cow::Owned(out)
}

type Tokens = Arc<Vec<Vec<Vec<HighlightSpan>>>>;

/// highlightDiffFile: the old and new sides as two documents, mapped back
/// to their lines. Indexed by block, then line.
fn highlight_file(path: &str, blocks: &[Block]) -> Option<Vec<Vec<Vec<HighlightSpan>>>> {
    let mut old: Vec<(usize, usize, String)> = Vec::new();
    let mut new: Vec<(usize, usize, String)> = Vec::new();
    let (mut old_chars, mut new_chars) = (0, 0);
    for (b, block) in blocks.iter().enumerate() {
        for (i, line) in block.lines().iter().enumerate() {
            if line.kind == LineKind::Hunk {
                continue;
            }
            let text = expand_tabs(&line.text).into_owned();
            if line.kind != LineKind::Add {
                old_chars += text.len() + 1;
                old.push((b, i, text.clone()));
            }
            if line.kind != LineKind::Del {
                new_chars += text.len() + 1;
                new.push((b, i, text));
            }
            if old_chars > MAX_HIGHLIGHT_CHARS || new_chars > MAX_HIGHLIGHT_CHARS {
                return None;
            }
        }
    }
    let mut out: Vec<Vec<Vec<HighlightSpan>>> = blocks
        .iter()
        .map(|b| (0..b.lines().len()).map(|_| Vec::new()).collect())
        .collect();
    let ext = path.rsplit('.').next().unwrap_or("");
    for (side, take_del) in [(old, true), (new, false)] {
        let source = side
            .iter()
            .map(|(_, _, t)| t.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let Ok(doc) = crate::ui::syntax_palette::highlight(HighlightRequest {
            source: &source,
            path: Some(ext),
            fence_tag: None,
        }) else {
            continue;
        };
        for ((b, i, _), spans) in side.into_iter().zip(doc.lines) {
            let is_del = blocks[b].lines()[i].kind == LineKind::Del;
            if is_del == take_del {
                out[b][i] = spans;
            }
        }
    }
    Some(out)
}

struct Comment {
    path: String,
    line: Line,
    anchor: AnchorSlot,
    input: gpui::Entity<Input>,
    _events: Vec<Subscription>,
}

pub(crate) struct DiffView {
    files: Vec<DiffFile>,
    by_id: HashMap<SharedString, usize>,
    open: HashSet<SharedString>,
    known: HashSet<SharedString>,
    bodies: HashMap<SharedString, Body>,
    rows: Vec<Row>,
    offsets: Vec<f32>,
    total: f32,
    /// Hunk rows in order: (file, row index when expanded).
    hunks: Vec<(usize, Option<usize>)>,
    file_rows: Vec<usize>,
    max_columns: usize,
    number_width: f32,
    list: ListState,
    scroll_x: f32,
    viewport: Rc<Cell<Option<Bounds<Pixels>>>>,
    char_width: Cell<Option<f32>>,
    hovered: Option<usize>,
    pending: Option<usize>,
    focus_id: Option<SharedString>,
    pub(crate) busy_id: Option<SharedString>,
    pub(crate) file_count: Option<usize>,
    pub(crate) totals: Option<(u32, u32)>,
    highlights: HashMap<SharedString, (Arc<Vec<Block>>, Option<Tokens>)>,
    highlighting: Option<Task<()>>,
    thumb_drag: Option<(Pixels, f32)>,
    overview_drag: Option<f32>,
    overview_bounds: AnchorSlot,
    overview_hovered: bool,
    /// The axis this wheel gesture scrolls, and when it last moved.
    wheel_axis: Option<(bool, std::time::Instant)>,
    comment: Option<Comment>,
    action_slots: HashMap<usize, AnchorSlot>,
    focus: FocusHandle,
}
impl EventEmitter<DiffEvent> for DiffView {}
impl Focusable for DiffView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl DiffView {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let list = ListState::new(0, ListAlignment::Top, px(600.));
        let weak = cx.weak_entity();
        list.set_scroll_handler(move |_, _, cx| {
            let _ = weak.update(cx, |this, cx| {
                // A wheel or drag ends a pending jump, as React's capture does.
                this.pending = None;
                cx.notify();
            });
        });
        Self {
            files: Vec::new(),
            by_id: HashMap::new(),
            open: HashSet::new(),
            known: HashSet::new(),
            bodies: HashMap::new(),
            rows: Vec::new(),
            offsets: vec![0.],
            total: 0.,
            hunks: Vec::new(),
            file_rows: Vec::new(),
            max_columns: 40,
            number_width: 36.,
            list,
            scroll_x: 0.,
            viewport: Rc::default(),
            char_width: Cell::new(None),
            hovered: None,
            pending: None,
            focus_id: None,
            busy_id: None,
            file_count: None,
            totals: None,
            highlights: HashMap::new(),
            highlighting: None,
            thumb_drag: None,
            overview_drag: None,
            overview_bounds: AnchorSlot::default(),
            overview_hovered: false,
            wheel_axis: None,
            comment: None,
            action_slots: HashMap::new(),
            focus: cx.focus_handle(),
        }
    }

    /// New file models. Folds a user made survive; files that appear open.
    pub(crate) fn set_files(&mut self, files: Vec<DiffFile>, cx: &mut Context<Self>) {
        let unchanged = files.len() == self.files.len()
            && files
                .iter()
                .zip(&self.files)
                .all(|(a, b)| a.id == b.id && a.same_content(b));
        if unchanged {
            return;
        }
        let ids: HashSet<SharedString> = files.iter().map(|f| f.id.clone()).collect();
        self.open.retain(|id| ids.contains(id));
        for id in &ids {
            if !self.known.contains(id) {
                self.open.insert(id.clone());
            }
        }
        self.known = ids;
        self.files = files;
        self.rebuild(cx);
    }
    /// The file a review was opened on: expanded and scrolled to the top.
    pub(crate) fn focus_file(&mut self, id: Option<SharedString>, cx: &mut Context<Self>) {
        if id == self.focus_id {
            return;
        }
        self.focus_id = id.clone();
        if let Some(id) = id {
            if self.open.insert(id.clone()) {
                self.rebuild(cx);
            }
            if let Some(&ix) = self.by_id.get(&id) {
                self.pending = Some(ix);
                self.apply_pending(cx);
            }
        }
    }

    fn row_key(&self, ix: usize) -> Option<RowKey> {
        let row = self.rows.get(ix)?;
        let id = self.files.get(row.file)?.id.clone();
        Some(match &row.kind {
            RowKind::File => RowKey(id, 0, 0, 0),
            RowKind::Line { block, line, .. } => RowKey(id, 1, *block, *line),
            RowKind::Hunk(_) => RowKey(id, 2, ix, 0),
            RowKind::Notice(_) => RowKey(id, 3, 0, 0),
        })
    }
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        // Data changes above the reader stay anchored to the row they read.
        let anchor = {
            let top = self.list.logical_scroll_top();
            self.row_key(top.item_ix)
                .map(|key| (key, top.offset_in_item))
        };
        self.by_id = self
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| (f.id.clone(), i))
            .collect();
        let mut rows = Vec::new();
        let mut hunks = Vec::new();
        let mut file_rows = Vec::with_capacity(self.files.len());
        let (mut columns, mut digits) = (40, 3);
        let keep: HashSet<_> = self.files.iter().map(|f| f.id.clone()).collect();
        self.bodies.retain(|id, _| keep.contains(id));
        for (index, file) in self.files.iter().enumerate() {
            let message = file.message();
            let stageable = file.can_stage_hunk;
            let stale = self.bodies.get(&file.id).is_none_or(|b| {
                !Arc::ptr_eq(&b.blocks, &file.blocks)
                    || b.message != message
                    || b.stageable != stageable
            });
            if stale {
                self.bodies
                    .insert(file.id.clone(), build_body(file, message, stageable));
            }
            let body = &self.bodies[&file.id];
            let expanded = self.open.contains(&file.id);
            file_rows.push(rows.len());
            rows.push(Row {
                file: index,
                kind: RowKind::File,
                height: FILE_HEADER,
            });
            let start = rows.len();
            for (i, row) in body.rows.iter().enumerate() {
                if matches!(row.kind, RowKind::Hunk(_)) {
                    hunks.push((index, expanded.then_some(start + i)));
                }
            }
            if expanded {
                rows.extend(body.rows.iter().map(|row| Row {
                    file: index,
                    ..row.clone()
                }));
                columns = columns.max(body.columns);
                digits = digits.max(body.digits);
            }
        }
        let mut offsets = Vec::with_capacity(rows.len() + 1);
        let mut at = 0.;
        for row in &rows {
            offsets.push(at);
            at += row.height;
        }
        offsets.push(at);
        self.rows = rows;
        self.offsets = offsets;
        self.total = at;
        self.hunks = hunks;
        self.file_rows = file_rows;
        self.max_columns = columns;
        self.number_width = (digits as f32 * 8. + 12.).max(36.);
        self.hovered = None;
        self.list.reset(self.rows.len());
        if self.pending.is_some() {
            self.apply_pending(cx);
        } else if let Some((key, offset)) = anchor {
            let target = (0..self.rows.len()).find(|&i| self.row_key(i).as_ref() == Some(&key));
            let target = target.or_else(|| self.by_id.get(&key.0).map(|&f| self.file_rows[f]));
            if let Some(item_ix) = target {
                let offset_in_item = if self.row_key(item_ix).as_ref() == Some(&key) {
                    offset
                } else {
                    px(0.)
                };
                self.list.scroll_to(ListOffset {
                    item_ix,
                    offset_in_item,
                });
            }
        }
        self.clamp_scroll_x();
        cx.notify();
    }
    /// A jump holds until its file has loaded, then lets go.
    fn apply_pending(&mut self, cx: &mut Context<Self>) {
        let Some(file) = self.pending else {
            return;
        };
        let Some(&row) = self.file_rows.get(file) else {
            return;
        };
        self.list.scroll_to(ListOffset {
            item_ix: row,
            offset_in_item: px(0.),
        });
        if self.files[file].empty_message.as_deref() != Some("Loading…") {
            self.pending = None;
        }
        cx.notify();
    }
    fn scroll_to_px(&self, y: f32) {
        let y = y.clamp(0., self.total.max(0.));
        let ix = self.offsets.partition_point(|o| *o <= y).saturating_sub(1);
        let ix = ix.min(self.rows.len().saturating_sub(1));
        self.list.scroll_to(ListOffset {
            item_ix: ix,
            offset_in_item: px(y - self.offsets.get(ix).copied().unwrap_or(0.)),
        });
    }
    fn top_px(&self) -> f32 {
        let top = self.list.logical_scroll_top();
        self.offsets.get(top.item_ix).copied().unwrap_or(0.) + top.offset_in_item.as_f32()
    }
    fn overview_metrics(&self, height: f32) -> crate::editor::scrollbar::Metrics {
        crate::editor::scrollbar::Metrics::new(self.top_px(), self.total, height, height)
    }
    fn overview_scroll(&mut self, y: Pixels, cx: &mut Context<Self>) {
        let Some(bounds) = self.overview_bounds.get() else {
            return;
        };
        let Some(offset) = self.overview_drag else {
            return;
        };
        let m = self.overview_metrics(bounds.size.height.as_f32());
        self.pending = None;
        self.scroll_to_px(m.scroll_from_pointer((y - bounds.top()).as_f32(), offset));
        cx.notify();
    }
    /// currentHunkIndex.
    fn current_hunk(&self) -> Option<usize> {
        let top = self.top_px() + FILE_HEADER + 8.;
        let mut found = None;
        let mut first = None;
        for (index, (_, row)) in self.hunks.iter().enumerate() {
            let Some(row) = row else { continue };
            first.get_or_insert(index);
            if self.offsets[*row] > top + 0.5 {
                break;
            }
            found = Some(index);
        }
        found.or(first)
    }
    fn navigate(&mut self, direction: isize, cx: &mut Context<Self>) {
        if self.hunks.is_empty() {
            return;
        }
        let current = self.current_hunk().map_or(-1, |i| i as isize);
        let index = (current + direction).clamp(0, self.hunks.len() as isize - 1) as usize;
        let (file, _) = self.hunks[index];
        let id = self.files[file].id.clone();
        if self.open.insert(id) {
            self.rebuild(cx);
        }
        if let Some((_, Some(row))) = self.hunks.get(index) {
            self.pending = None;
            self.scroll_to_px(self.offsets[*row] - FILE_HEADER - 8.);
        }
        cx.notify();
    }
    fn toggle(&mut self, file: usize, cx: &mut Context<Self>) {
        let id = self.files[file].id.clone();
        if !self.open.remove(&id) {
            self.open.insert(id);
        }
        self.rebuild(cx);
    }

    fn gutter_width(&self) -> f32 {
        self.number_width * 2. + 24. + 3.
    }
    fn char_width(&self, window: &mut Window) -> f32 {
        if let Some(width) = self.char_width.get() {
            return width;
        }
        let mut font = window.text_style().font();
        font.family = crate::ui::fonts::mono().into();
        let run = gpui::TextRun {
            len: 10,
            font,
            color: Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let width = window
            .text_system()
            .shape_line("0000000000".into(), px(CODE_SIZE), &[run], None)
            .width
            .as_f32()
            / 10.;
        self.char_width.set(Some(width));
        width
    }
    /// `--diff-code-min-width`: `calc((maxColumns + 2)ch + 24px)`.
    fn code_width(&self) -> f32 {
        (self.max_columns as f32 + 2.) * self.char_width.get().unwrap_or(7.5) + 24.
    }
    fn lane_width(&self) -> f32 {
        self.viewport
            .get()
            .map_or(0., |b| b.size.width.as_f32() - self.gutter_width())
    }
    fn max_scroll_x(&self) -> f32 {
        (self.code_width() - self.lane_width()).max(0.)
    }
    fn clamp_scroll_x(&mut self) {
        self.scroll_x = self.scroll_x.clamp(0., self.max_scroll_x());
    }
    /// Runs before the list sees the wheel. Any wheel ends a pending jump,
    /// as React's `stopJump`. Each gesture keeps to the axis it started on:
    /// a sideways one scrolls the code lanes and never the list, a vertical
    /// one never drifts the lanes. Returns whether it took the event.
    fn wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) -> bool {
        use gpui::TouchPhase;
        self.pending = None;
        let delta = event.delta.pixel_delta(px(LINE));
        let (dx, dy) = if event.modifiers.shift && delta.x.abs() < px(0.5) {
            (delta.y, px(0.))
        } else {
            (delta.x, delta.y)
        };
        let now = std::time::Instant::now();
        let fresh = matches!(event.touch_phase, TouchPhase::Started)
            || self.wheel_axis.is_none_or(|(_, at)| {
                now.duration_since(at) > std::time::Duration::from_millis(150)
            });
        let horizontal = if fresh {
            if dx.abs() < px(0.01) && dy.abs() < px(0.01) {
                self.wheel_axis = None;
                return false;
            }
            dx.abs() > dy.abs()
        } else {
            self.wheel_axis.is_some_and(|(h, _)| h)
        };
        self.wheel_axis =
            (!matches!(event.touch_phase, TouchPhase::Ended)).then_some((horizontal, now));
        if !horizontal {
            return false;
        }
        let next = (self.scroll_x - dx.as_f32()).clamp(0., self.max_scroll_x());
        if next != self.scroll_x {
            self.scroll_x = next;
            cx.notify();
        }
        true
    }

    /// Highlights files on screen that have no tokens yet, one at a time.
    fn request_highlights(&mut self, visible: &[usize], cx: &mut Context<Self>) {
        if self.highlighting.is_some() {
            return;
        }
        let Some(&file) = visible.iter().find(|&&f| {
            let file = &self.files[f];
            !file.blocks.is_empty()
                && self
                    .highlights
                    .get(&file.id)
                    .is_none_or(|(blocks, _)| !Arc::ptr_eq(blocks, &file.blocks))
        }) else {
            return;
        };
        let (id, path, blocks) = {
            let f = &self.files[file];
            (f.id.clone(), f.path.clone(), f.blocks.clone())
        };
        let work_blocks = blocks.clone();
        let work = cx
            .background_executor()
            .spawn(async move { highlight_file(&path, &work_blocks).map(Arc::new) });
        self.highlighting = Some(cx.spawn(async move |this, cx| {
            let tokens = work.await;
            let _ = this.update(cx, |this, cx| {
                this.highlights.insert(id, (blocks, tokens));
                this.highlighting = None;
                let keep: HashSet<_> = this.files.iter().map(|f| f.id.clone()).collect();
                this.highlights.retain(|id, _| keep.contains(id));
                cx.notify();
            });
        }));
    }
    fn tokens(&self, file: &DiffFile, block: usize, line: usize) -> Option<&[HighlightSpan]> {
        let (blocks, tokens) = self.highlights.get(&file.id)?;
        if !Arc::ptr_eq(blocks, &file.blocks) {
            return None;
        }
        tokens.as_ref()?.get(block)?.get(line).map(Vec::as_slice)
    }

    fn open_comment(
        &mut self,
        file: usize,
        line: Line,
        row: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = self.files[file].path.clone();
        let anchor = self.action_slots.get(&row).cloned().unwrap_or_default();
        let input = cx.new(|cx| {
            let mut input = Input::commit_message(cx);
            input.set_placeholder("Leave a comment…", cx);
            input
        });
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        let events = vec![
            cx.subscribe(&input, |_, _, _: &input::Changed, cx| cx.notify()),
            cx.subscribe_in(
                &input,
                window,
                |this, _, _: &input::Submitted, window, cx| this.add_comment(window, cx),
            ),
        ];
        self.comment = Some(Comment {
            path,
            line,
            anchor,
            input,
            _events: events,
        });
        cx.notify();
    }
    fn add_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(comment) = &self.comment else {
            return;
        };
        let text = diff::format_comment(
            &comment.path,
            &comment.line,
            &comment.input.read(cx).content,
        );
        if text.is_empty() {
            return;
        }
        self.comment = None;
        window.focus(&self.focus, cx);
        cx.emit(DiffEvent::Comment(text));
        cx.notify();
    }
    fn dismiss_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.comment.take().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }
}

fn blur() -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            // Keep the tint on the blur quad itself. GPUI treats a fully
            // transparent fill as border-only and strips its interior.
            let quad = gpui::fill(bounds, content(0.02))
                .border_color(gpui::backdrop_blur_marker(24. * window.scale_factor()));
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            let quad = gpui::fill(bounds, rgb(CHAT));
            window.paint_quad(quad);
        },
    )
    .absolute()
    .size_full()
}

/// DiffCounts: `+a −d` at 11px medium, nothing when unchanged.
pub(crate) fn diff_counts(additions: u32, deletions: u32) -> AnyElement {
    if additions == 0 && deletions == 0 {
        return div().into_any_element();
    }
    div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .font_features(tabular())
        .when(additions > 0, |d| {
            d.child(
                div()
                    .text_color(rgb(DIFF_ADD))
                    .child(format!("+{}", format_integer(additions as i64))),
            )
        })
        .when(deletions > 0, |d| {
            d.child(
                div()
                    .text_color(rgb(DIFF_DEL))
                    .child(format!("−{}", format_integer(deletions as i64))),
            )
        })
        .into_any_element()
}

#[cfg(test)]
mod tests;
