//! The editor shares native selection/IME handling with Input and retains glyph
//! runs for visible lines. Scroll position is independent of the caret.
mod completion;
mod editing;
pub(crate) mod paint;
mod pointer;
use crate::editor::{
    analysis::Analysis,
    scrollbar::{Metrics, WIDTH},
};
use crate::ui::input::*;
use gpui::{
    AnyElement, App, Bounds, ContentMask, Context, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, GlobalElementId, Hsla, InspectorElementId, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, Pixels, Point, Role, SharedString, Style, Task, TextAlign,
    TextRun, UnderlineStyle, Window, WrappedLine, div, fill, point, prelude::*, px, relative, rgb,
    size,
};
use std::{cell::Cell, ops::Range, sync::Arc};
use unicode_segmentation::UnicodeSegmentation;

const TOP_PADDING: f32 = 8.;
const BOTTOM_PADDING: f32 = 32.;

#[derive(Clone)]
struct MarkerHit {
    bounds: Bounds<Pixels>,
    pos: usize,
    diagnostic: bool,
    message: String,
}

pub(crate) struct Code {
    hovered_change: Option<usize>,
    can_stage: bool,
    suppress_completion: bool,
    diff: bool,
    deleted_rows: Vec<(f32, f32, String, usize, Vec<crate::editor::analysis::Token>)>,
    path: String,
    completions: Vec<crate::editor::analysis::Completion>,
    completion_index: usize,
    completion_task: Option<Task<()>>,
    completion_source: Option<(SharedString, usize)>,
    folded: Vec<crate::editor::analysis::Fold>,
    search_matches: Vec<Range<usize>>,
    offsets: Vec<usize>,
    visible: Vec<(usize, WrappedLine)>,
    reveal: bool,
    max_width: Pixels,
    undo: Vec<(SharedString, Range<usize>)>,
    redo: Vec<(SharedString, Range<usize>)>,
    wrap: bool,
    layout_width: Option<Pixels>,
    tops: Vec<f32>,
    analysis: Arc<Analysis>,
    /// The buffer as of the last change, which `analysis` offsets follow.
    text: SharedString,
    rail: Option<Bounds<Pixels>>,
    drag_offset: Option<f32>,
    hovered: bool,
    markers: Vec<MarkerHit>,
    hovered_marker: Option<usize>,
    affinity: Cell<Option<(usize, bool)>>,
}
impl Code {
    fn new() -> Self {
        Self {
            hovered_change: None,
            can_stage: false,
            suppress_completion: false,
            diff: false,
            deleted_rows: Vec::new(),
            path: String::new(),
            completions: Vec::new(),
            completion_index: 0,
            completion_task: None,
            completion_source: None,
            folded: Vec::new(),
            search_matches: Vec::new(),
            offsets: vec![0],
            visible: Vec::new(),
            reveal: true,
            max_width: px(0.),
            undo: Vec::new(),
            redo: Vec::new(),
            wrap: true,
            layout_width: None,
            tops: vec![TOP_PADDING, TOP_PADDING + 20.],
            analysis: Arc::new(Analysis::default()),
            text: SharedString::default(),
            rail: None,
            drag_offset: None,
            hovered: false,
            markers: Vec::new(),
            hovered_marker: None,
            affinity: Cell::new(None),
        }
    }
    fn row(&self, offset: usize) -> usize {
        self.offsets
            .partition_point(|start| *start <= offset)
            .saturating_sub(1)
    }
    fn fold_at(&self, row: usize) -> Option<&crate::editor::analysis::Fold> {
        self.folded.iter().find(|f| self.row(f.from) == row)
    }
    fn hidden(&self, row: usize) -> bool {
        self.folded
            .iter()
            .any(|f| row > self.row(f.from) && row <= self.row(f.to))
    }
    fn display_row(&self, at: usize) -> usize {
        let row = self.row(at);
        self.folded
            .iter()
            .find(|f| row > self.row(f.from) && row <= self.row(f.to))
            .map_or(row, |f| self.row(f.from))
    }
    fn display_offset(&self, row: usize, at: usize) -> usize {
        let start = self.offsets[row];
        if let Some(f) = self.fold_at(row) {
            if at >= f.to {
                return f.from - start + "…".len() + at - f.to;
            }
            if at > f.from {
                return f.from - start;
            }
        }
        at.saturating_sub(start)
    }
    fn source_offset(&self, row: usize, at: usize) -> usize {
        let start = self.offsets[row];
        if let Some(f) = self.fold_at(row) {
            let prefix = f.from - start;
            if at > prefix {
                return f.to + at.saturating_sub(prefix + "…".len());
            }
        }
        start + at
    }
    fn source_end(&self, row: usize, len: usize) -> usize {
        let row = self.fold_at(row).map_or(row, |f| self.row(f.to));
        self.offsets.get(row + 1).map_or(len, |s| s - 1)
    }
    fn gutter(&self) -> f32 {
        let digits = self.offsets.len().to_string().len() as f32;
        if self.diff {
            4. + 12. + digits * 7.5 + 5. + 16. + 1. + 6.
        } else {
            4. + (digits * 7.5 + 12.).max(36.) + 12. + 1. + 6.
        }
    }
    fn top(&self, row: usize, line_height: f32) -> f32 {
        self.tops
            .get(row)
            .copied()
            .unwrap_or(TOP_PADDING + row as f32 * line_height)
    }
    fn total(&self) -> f32 {
        self.tops.last().copied().unwrap_or(TOP_PADDING) + BOTTOM_PADDING
    }
    fn row_at_y(&self, y: f32) -> usize {
        self.tops
            .partition_point(|top| *top <= y)
            .saturating_sub(1)
            .min(self.offsets.len() - 1)
    }
}
impl Input {}

// GPUI resolves an index at a soft boundary to the end of the previous row.
// Keep the clicked/navigation side of that boundary for the caret and IME.
fn line_position(line: &WrappedLine, at: usize, lh: Pixels, upstream: bool) -> Point<Pixels> {
    if !upstream {
        for (row, boundary) in line.wrap_boundaries().iter().enumerate() {
            let glyph = &line.runs()[boundary.run_ix].glyphs[boundary.glyph_ix];
            if glyph.index == at {
                return point(px(0.), lh * (row + 1));
            }
        }
    }
    line.position_for_index(at, lh).unwrap_or_default()
}

pub(crate) struct CodePaint {
    lines: Vec<(usize, WrappedLine)>,
    deleted: Vec<(f32, WrappedLine, usize)>,
    scroll: Point<Pixels>,
    text_bounds: Bounds<Pixels>,
}
#[cfg(test)]
mod tests;
