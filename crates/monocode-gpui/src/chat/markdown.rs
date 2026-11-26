//! GPUI port of AgentMarkdown.tsx and its .agent-markdown CSS.
mod cache;
pub(crate) mod mermaid;
pub(crate) mod model;
mod search;
pub(crate) mod selection;
use crate::chat::markdown::model::{Block, InlineRun, TableAlign};
use crate::chat::transcript::selection::TextSelection;
use crate::ui::syntax_palette::HighlightedDocument;
use crate::ui::theme::*;
pub(crate) use cache::{PreparedRow, RenderCache};
use gpui::Window;
use gpui::{
    AnyElement, App, ClickEvent, Div, FontStyle, FontWeight, HighlightStyle, InteractiveText,
    MouseButton, Pixels, Point, Role, SharedString, StrikethroughStyle, StyledText, div,
    prelude::*, px, rgb,
};
pub(crate) use search::{mark_row, mark_styles, row_text};

#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = markdown, no_json)]
pub(crate) struct LinkMenu {
    pub url: String,
    pub position: Point<Pixels>,
}

pub(crate) fn link_menu(url: &str, position: Point<Pixels>, window: &mut Window, cx: &mut App) {
    window.dispatch_action(
        Box::new(LinkMenu {
            url: url.into(),
            position,
        }),
        cx,
    );
}

fn activate_link(event: &ClickEvent, selection: Option<&TextSelection>) -> bool {
    event.click_count() == 1
        && !event.modifiers().shift
        && !selection.is_some_and(TextSelection::has_selection)
}
use std::{
    hash::{DefaultHasher, Hash, Hasher},
    ops::Range,
    sync::Arc,
    time::Instant,
};

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Tone {
    Normal,
    Work,
    Reasoning,
}
impl Tone {
    fn prose(self) -> f32 {
        match self {
            Self::Normal => 0.78,
            Self::Work => 0.65,
            Self::Reasoning => 0.48,
        }
    }
    fn strong(self) -> f32 {
        match self {
            Self::Normal => 1.,
            Self::Work => 0.8,
            Self::Reasoning => 0.62,
        }
    }
}
const PART_BYTES: usize = 2048;
const PART_LINES: usize = 24;

#[derive(Clone, PartialEq)]
pub(crate) struct Fence {
    pub code: SharedString,
    pub language: String,
    pub label: String,
    pub path: Option<String>,
    pub start_line: usize,
    pub numbered: bool,
    pub lines: Vec<Range<usize>>,
    pub key: u64,
}
impl Fence {
    fn new(info: &str, code: &str) -> Self {
        let mut words = info.split_whitespace();
        let first = words.next().unwrap_or("");
        let mut language = first.to_owned();
        let mut label = first.to_owned();
        let mut path = None;
        let mut start_line = 1;
        let citation: Vec<_> = first.splitn(3, ':').collect();
        if citation.len() == 3
            && let (Ok(start), Ok(_)) = (citation[0].parse::<usize>(), citation[1].parse::<usize>())
        {
            start_line = start;
            label = citation[2].into();
            path = Some(label.clone());
            language = citation[2].rsplit('.').next().unwrap_or("").into();
        } else if first.contains('/') || first.contains('\\') {
            language = first.rsplit('.').next().unwrap_or("").into();
            path = Some(first.to_owned());
        }

        // Match parseCodeFence: language, optional path/title and startLine.
        for word in words {
            if let Some(value) = word.strip_prefix("startLine=") {
                start_line = value.parse().unwrap_or(1);
            } else if let Some(value) = word
                .strip_prefix("title=")
                .or_else(|| word.strip_prefix("file="))
            {
                label = value.trim_matches(['\'', '"']).to_owned();
                path = Some(label.clone());
            } else if word.contains('/') || word.contains('.') {
                label = word.trim_matches(['\'', '"']).to_owned();
                path = Some(label.clone());
            }
        }
        if matches!(language.as_str(), "" | "text" | "plaintext" | "txt") {
            language = "text".into();
        }
        let mut hash = DefaultHasher::new();
        language.hash(&mut hash);
        code.hash(&mut hash);
        let mut at = 0;
        let mut lines = Vec::new();
        for line in code.split_inclusive('\n') {
            let end = at + line.trim_end_matches(['\n', '\r']).len();
            lines.push(at..end);
            at += line.len();
        }
        if lines.is_empty() {
            lines.push(0..0);
        }
        Self {
            code: code.to_owned().into(),
            language,
            label,
            path,
            start_line,
            numbered: !info.contains("noLineNumbers"),
            lines,
            key: hash.finish(),
        }
    }
}

#[derive(Clone, PartialEq)]
pub(crate) enum Part {
    Text {
        runs: Vec<InlineRun>,
        heading: u8,
    },
    Code {
        fence: Arc<Fence>,
        lines: Range<usize>,
    },
    Mermaid {
        fence: Arc<Fence>,
    },
    Table {
        cells: Vec<Vec<Vec<InlineRun>>>,
        columns: Arc<[f32]>,
        align: Vec<TableAlign>,
        header: bool,
        last: bool,
    },
    Rule,
    Math(String),
    Visual {
        id: String,
        caption: String,
    },
}
#[derive(Clone, PartialEq)]
pub(crate) struct MarkdownRow {
    pub part: Part,
    pub indent: usize,
    pub quote: usize,
    pub marker: Option<String>,
    pub gap: f32,
    pub word_start: usize,
}

pub(crate) fn rows(block: &Block) -> Vec<MarkdownRow> {
    let mut rows = Vec::new();
    flatten(block, 0, 0, None, &mut rows);
    rows
}
fn flatten(
    block: &Block,
    indent: usize,
    quote: usize,
    marker: Option<String>,
    out: &mut Vec<MarkdownRow>,
) {
    let first = out.len();
    let mut add = |part, gap| {
        out.push(MarkdownRow {
            part,
            indent,
            quote,
            marker: None,
            gap,
            word_start: 0,
        })
    };
    match block {
        Block::Paragraph { runs } | Block::Heading { runs, .. } => {
            let heading = match block {
                Block::Heading { level, .. } => *level,
                _ => 0,
            };
            for (ix, runs) in split_runs(runs).into_iter().enumerate() {
                add(
                    Part::Text { runs, heading },
                    if ix == 0 {
                        if heading > 0 { 24. } else { 16. }
                    } else {
                        0.
                    },
                );
            }
        }
        Block::CodeBlock { language, code } => {
            let fence = Arc::new(Fence::new(language.as_deref().unwrap_or(""), code));
            if fence.language.eq_ignore_ascii_case("mermaid") {
                add(Part::Mermaid { fence }, 16.);
            } else {
                for at in (0..fence.lines.len()).step_by(PART_LINES) {
                    add(
                        Part::Code {
                            fence: fence.clone(),
                            lines: at..(at + PART_LINES).min(fence.lines.len()),
                        },
                        if at == 0 { 16. } else { 0. },
                    );
                }
            }
        }
        Block::List {
            ordered_start,
            items,
        } => {
            for (ix, children) in items.iter().enumerate() {
                let start = out.len();
                let bullet =
                    ordered_start.map_or_else(|| "•".into(), |n| format!("{}.", n + ix as u64));
                for child in children {
                    flatten(child, indent + 1, quote, None, out);
                }
                if let Some(row) = out.get_mut(start) {
                    row.marker = Some(bullet);
                    row.gap = if ix == 0 { 8. } else { 4. };
                }
            }
        }
        Block::BlockQuote { children } => {
            for child in children {
                flatten(child, indent, quote + 1, None, out);
            }
        }
        Block::Table {
            header,
            rows,
            align,
        } => {
            let all: Vec<_> = std::iter::once(header.clone())
                .chain(rows.iter().cloned())
                .collect();
            let columns = table_column_widths(&all);
            for (ix, cells) in all.chunks(PART_LINES).enumerate() {
                add(
                    Part::Table {
                        cells: cells.to_vec(),
                        columns: columns.clone(),
                        align: align.clone(),
                        header: ix == 0,
                        last: (ix + 1) * PART_LINES >= all.len(),
                    },
                    if ix == 0 { 16. } else { 0. },
                );
            }
        }
        Block::Rule => add(Part::Rule, 24.),
        Block::Math { tex } => add(Part::Math(tex.clone()), 16.),
        Block::Visual { id, caption } => add(
            Part::Visual {
                id: id.clone(),
                caption: caption.clone(),
            },
            16.,
        ),
    }
    if let Some(row) = out.get_mut(first)
        && marker.is_some()
    {
        row.marker = marker;
    }
}

fn table_column_widths(rows: &[Vec<Vec<InlineRun>>]) -> Arc<[f32]> {
    let count = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let mut widths = vec![160_f32; count];
    // These are preferred flex widths, not fixed cell widths. Sample the whole
    // table once so virtualized chunks use the same columns. Short labels keep
    // the stylesheet's 160px minimum; prose receives more of the available room.
    for row in rows {
        for (col, cell) in row.iter().enumerate() {
            let chars = runs_text(cell)
                .lines()
                .map(|line| line.chars().count())
                .max()
                .unwrap_or(0);
            widths[col] = widths[col].max((chars as f32 * 6. + 20.).clamp(160., 480.));
        }
    }
    widths.into()
}

/// Split text while keeping every byte and its inline style; storage boundaries
/// never become paragraph, list, or code-fence boundaries.
fn split_runs(runs: &[InlineRun]) -> Vec<Vec<InlineRun>> {
    let mut parts = Vec::new();
    let mut part = Vec::new();
    let mut bytes = 0;
    let mut lines = 0;
    for run in runs {
        let mut text = run.text.as_str();
        while !text.is_empty() {
            if bytes >= PART_BYTES || lines >= 12 {
                parts.push(std::mem::take(&mut part));
                bytes = 0;
                lines = 0;
            }
            let mut end = text.len().min(PART_BYTES - bytes);
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            if end == 0 {
                parts.push(std::mem::take(&mut part));
                bytes = 0;
                lines = 0;
                continue;
            }
            if let Some((at, _)) = text[..end].match_indices('\n').nth(11 - lines) {
                end = at + 1;
            }
            let slice = &text[..end];
            part.push(InlineRun {
                text: slice.to_owned(),
                style: run.style.clone(),
            });
            bytes += end;
            lines += slice.matches('\n').count();
            text = &text[end..];
        }
    }
    if !part.is_empty() {
        parts.push(part);
    }
    parts
}

pub(crate) fn word_count(row: &MarkdownRow) -> usize {
    let count = |runs: &[InlineRun]| {
        runs.iter()
            .filter(|r| !r.style.code && !r.style.math && r.style.link.is_none())
            .map(|r| r.text.split_whitespace().count())
            .sum::<usize>()
    };
    match &row.part {
        Part::Text { runs, .. } => count(runs),
        Part::Table { cells, .. } => cells.iter().flatten().map(|r| count(r)).sum(),
        _ => 0,
    }
}
#[allow(clippy::too_many_arguments)]
fn rich_inline(
    runs: &[InlineRun],
    id: SharedString,
    tone: Tone,
    fade: Option<&crate::ui::motion::reveal::WordFades>,
    _now: Instant,
    mut word: usize,
    font_size: f32,
    selection: Option<TextSelection>,
) -> AnyElement {
    let mut children = Vec::new();
    let mut offset = 0;
    for (run_ix, run) in runs.iter().enumerate() {
        let style = &run.style;
        if style.code {
            let text = run.text.clone();
            let file = is_file_chip(&text);
            let styled = StyledText::new(text.clone());
            let layout = styled.layout().clone();
            let selected = selection.as_ref().map(|s| {
                let selected = s.offset(offset);
                if file {
                    selected.with_cursor(gpui::CursorStyle::PointingHand)
                } else {
                    selected
                }
            });
            let clicked_selection = selected.clone();
            let child = styled.into_any_element();
            let child = if let Some(selected) = selected {
                selected.wrap(text.clone().into(), layout, child)
            } else {
                child
            };
            children.push(
                div()
                    .id(SharedString::from(format!("{id}-code-{run_ix}")))
                    .flex()
                    .items_center()
                    .gap_1()
                    .min_h(px(24.))
                    .max_w_full()
                    .px(px(6.))
                    .rounded(px(6.))
                    .bg(style
                        .search
                        .map(search::search_background)
                        .unwrap_or_else(|| content(0.08)))
                    .font_family(crate::ui::fonts::mono())
                    .text_size(px(font_size * 0.8))
                    .line_height(px(24.))
                    .text_color(content(1.))
                    .when(file, |d| {
                        let path = style.link.clone().unwrap_or_else(|| text.clone());
                        let menu_path = path.clone();
                        d.cursor(gpui::CursorStyle::PointingHand)
                            .hover(|s| s.bg(content(0.13)))
                            .on_click(move |event, window, cx| {
                                if activate_link(event, clicked_selection.as_ref()) {
                                    open_link(&path, window, cx)
                                }
                            })
                            .on_mouse_down(MouseButton::Right, move |event, window, cx| {
                                link_menu(&menu_path, event.position, window, cx);
                                cx.stop_propagation();
                            })
                            .child(crate::ui::file_icons::file_icon(&text, 14.))
                    })
                    // A flex item keeps its unwrapped width unless allowed to
                    // shrink, so a long path would run past the column.
                    .child(
                        div()
                            .min_w_0()
                            .debug_selector(move || format!("rich-code-{run_ix}"))
                            .child(child),
                    )
                    .into_any_element(),
            );
            offset += run.text.len();
            continue;
        }
        for (part_ix, part) in run.text.split_inclusive(char::is_whitespace).enumerate() {
            let text = part.trim_end_matches(char::is_whitespace);
            if !text.is_empty() {
                let alpha = if style.link.is_some() || style.math {
                    1.
                } else {
                    let value = fade.map_or(1., |fade| fade.opacity(word));
                    word += 1;
                    value
                };
                let color = if style.link.is_some() {
                    accent_color()
                } else if style.bold || style.italic {
                    content(tone.strong())
                } else {
                    content(tone.prose())
                };
                let base_opacity = color.a;
                let mut opaque_color = color;
                opaque_color.a = 1.;
                let highlight = HighlightStyle {
                    color: Some(opaque_color),
                    background_color: style.search.map(search::search_background),
                    fade_out: Some(1. - base_opacity),
                    font_weight: style.bold.then_some(FontWeight::SEMIBOLD),
                    font_style: style.italic.then_some(FontStyle::Italic),
                    strikethrough: style.strikethrough.then_some(StrikethroughStyle {
                        thickness: px(1.),
                        color: None,
                    }),
                    ..Default::default()
                };
                let link = style.link.clone();
                let menu_link = link.clone();
                let styled =
                    StyledText::new(text.to_owned()).with_highlights([(0..text.len(), highlight)]);
                let layout = styled.layout().clone();
                let child = styled.into_any_element();
                let selected = selection.as_ref().map(|s| {
                    let selected = s.offset(offset);
                    if link.is_some() {
                        selected.with_cursor(gpui::CursorStyle::PointingHand)
                    } else {
                        selected
                    }
                });
                let clicked_selection = selected.clone();
                let child = if let Some(selected) = selected {
                    selected.wrap(text.to_owned().into(), layout, child)
                } else {
                    child
                };
                children.push(
                    div()
                        .id(SharedString::from(format!("{id}-{run_ix}-{part_ix}")))
                        .debug_selector(move || format!("rich-prose-{run_ix}-{part_ix}"))
                        .max_w_full()
                        .text_size(px(font_size))
                        .line_height(px(24.))
                        .opacity(alpha)
                        .when(link.is_some(), |d| {
                            d.cursor(gpui::CursorStyle::PointingHand)
                        })
                        .child(child)
                        .on_click(move |event, window, cx| {
                            if let Some(url) = &link
                                && activate_link(event, clicked_selection.as_ref())
                            {
                                open_link(url, window, cx);
                            }
                        })
                        .on_mouse_down(MouseButton::Right, move |event, window, cx| {
                            if let Some(url) = &menu_link {
                                link_menu(url, event.position, window, cx);
                                cx.stop_propagation();
                            }
                        })
                        .into_any_element(),
                );
            }
            offset += part.len();
            if part.ends_with('\n') {
                children.push(div().w_full().h(px(0.)).into_any_element());
            } else if part.len() > text.len() {
                children.push(div().w(px(font_size * 0.25)).h(px(1.)).into_any_element());
            }
        }
    }
    div()
        .id(id)
        .role(Role::Label)
        .aria_label(runs.iter().map(|run| run.text.as_str()).collect::<String>())
        .w_full()
        .flex()
        .flex_wrap()
        .items_center()
        .children(children)
        .into_any_element()
}
fn is_file_chip(text: &str) -> bool {
    !text.chars().any(char::is_whitespace)
        && text
            .rsplit('/')
            .next()
            .is_some_and(|name| name.contains('.'))
}
/// Bind preview links to their document, independent of the focused pane.
pub(crate) fn resolve_file_links(row: &mut MarkdownRow, base: &str) {
    fn resolve(runs: &mut [InlineRun], base: &str) {
        for run in runs {
            let target =
                run.style.link.clone().or_else(|| {
                    (run.style.code && is_file_chip(&run.text)).then(|| run.text.clone())
                });
            if let Some(target) = target
                && !target.starts_with('#')
                && !target.starts_with("mailto:")
                && (!target.contains("://") || target.starts_with("file://"))
            {
                let (path, location) = crate::ui::files::file_location(&target, base);
                run.style.link = Some(location.map_or_else(
                    || path.clone(),
                    |(line, col)| format!("{path}:{line}:{col}"),
                ));
            }
        }
    }
    match &mut row.part {
        Part::Text { runs, .. } => resolve(runs, base),
        Part::Table { cells, .. } => {
            for runs in cells.iter_mut().flatten() {
                resolve(runs, base);
            }
        }
        _ => {}
    }
}

fn open_link(url: &str, window: &mut Window, cx: &mut App) {
    if url.starts_with("https://") || url.starts_with("http://") || url.starts_with("mailto:") {
        cx.open_url(url);
    } else if !url.starts_with('#') && (!url.contains("://") || url.starts_with("file://")) {
        window.dispatch_action(Box::new(crate::ui::files::OpenPath(url.into())), cx);
    }
}

pub(crate) fn body(
    row: &MarkdownRow,
    id: SharedString,
    tone: Tone,
    fade: Option<&crate::ui::motion::reveal::WordFades>,
    now: Instant,
) -> AnyElement {
    body_selected(row, id, tone, fade, now, None, None)
}
pub(crate) fn code_lines(
    fence: &Fence,
    range: Range<usize>,
    highlighted: Option<&HighlightedDocument>,
    query: &str,
    current: bool,
) -> Div {
    code_lines_selected(fence, range, highlighted, query, current, None)
}
fn runs_text(runs: &[InlineRun]) -> String {
    let rich = runs.iter().any(|run| run.style.code);
    runs.iter()
        .map(|run| {
            format!(
                "{}{}",
                if rich {
                    String::new()
                } else {
                    run.style.task.as_ref().map_or(String::new(), |task| {
                        if task.checked {
                            "☑ ".into()
                        } else {
                            "☐ ".into()
                        }
                    })
                },
                run.text
            )
        })
        .collect()
}
pub(crate) fn selection_text(row: &MarkdownRow) -> String {
    match &row.part {
        Part::Text { runs, .. } => runs_text(runs),
        Part::Table { cells, .. } => cells
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| runs_text(cell))
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => row_text(row),
    }
}

use selection::{body_selected, code_lines_selected};
#[cfg(test)]
mod tests;
