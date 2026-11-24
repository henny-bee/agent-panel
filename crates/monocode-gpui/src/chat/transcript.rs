pub(crate) mod attachments;
pub(crate) mod burst;
pub(crate) mod dates;
mod drafts;
pub(crate) mod git_turns;
mod history;
pub(crate) use history::{HistoryPager, PreparedHistory};
mod actions;
pub(crate) mod activity;
mod follow;
mod images;
mod links;
mod live_phase;
mod mascot;
mod measurements;
pub(crate) mod mono;
mod mono_divider;
mod outline;
mod pending;
mod perf_scroll;
pub(crate) mod phase_tile;
pub(crate) mod phases;
mod provider_switch;
mod remote_plans;
mod row_body;
mod row_parts;
mod rows;
pub(crate) mod selection;
mod state;
mod streaming;
pub(crate) mod tasks;
pub(crate) mod tool_detail;
mod tools;
mod user_links;
mod view;
mod visuals;
#[cfg(test)]
pub(crate) use visuals::placements;

use crate::chat::markdown::model::MarkdownParser;
use crate::chat::response_actions::{self, Request as ResponseRequest};
use crate::ui::popover::{Anchor, AnchorSlot, Side, popover};
use crate::ui::syntax_palette::{HighlightRequest, HighlightedDocument};
use crate::{
    chat::markdown::{self, Fence, MarkdownRow, Part},
    ui::{
        motion::reveal::{Disclosure, Motion, PacedText},
        theme::*,
    },
};
use gpui::{
    AvailableSpace, Bounds, ClipboardItem, Context, Entity, FollowMode, ListAlignment, ListOffset,
    ListState, Pixels, Role, SharedString, StyleRefinement, Window, canvas, div, list, prelude::*,
    px, rgb, size,
};
use monocode_core::session::tool::{ToolCall, ToolState, tasks::Panel};
use monocode_core::session::transcript::Row;
use monocode_core::session::user_links::{UserMessageLink, parse_user_message_link};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::Instant,
};
/// Animates the transcript on the first display frame at or after `due`.
fn motion_frame(view: gpui::WeakEntity<TranscriptView>, due: Option<Instant>, window: &Window) {
    window.on_next_frame(move |window, cx| {
        if due.is_some_and(|due| Instant::now() < due) {
            motion_frame(view, due, window);
        } else {
            let _ = view.update(cx, |this, cx| this.animate_frame(cx));
        }
    });
}

/// Where a launched session stands, for its card in the chat.
#[derive(Clone, PartialEq)]
pub(crate) struct LaunchView {
    pub title: String,
    /// working, input, draft, failed, stopped or done.
    pub state: &'static str,
    pub harness: String,
    pub cwd: String,
}
/// A message the Mono has not taken yet, shown where it will land.
#[derive(Clone, PartialEq)]
pub(crate) struct PendingMessage {
    pub id: String,
    pub text: SharedString,
    pub error: Option<String>,
    pub sent_at: Option<u64>,
    pub attachments: Vec<std::path::PathBuf>,
}
struct TakenMessage {
    text: SharedString,
    removed_at: Instant,
    entrance: Option<Instant>,
}
pub(crate) enum MonoResponseAction {
    Quote {
        text: String,
        window: gpui::AnyWindowHandle,
    },
    Activity(usize),
    /// Retry or discard a message that was not sent.
    Queue {
        id: String,
        remove: bool,
    },
}
impl gpui::EventEmitter<MonoResponseAction> for TranscriptView {}

fn work_title(model: Option<&str>, elapsed_ms: Option<u64>, live: bool) -> String {
    let model = model.map(str::trim).filter(|model| !model.is_empty());
    let verb = match (live, model.is_some()) {
        (true, true) => "working",
        (true, false) => "Working",
        (false, true) => "worked",
        (false, false) => "Worked",
    };
    let title = model.map_or_else(|| verb.to_owned(), |model| format!("{model} {verb}"));
    let Some(ms) = elapsed_ms else {
        return if live { format!("{title}…") } else { title };
    };
    let seconds = ms.saturating_add(500).saturating_div(1000).max(1);
    let duration = if seconds < 60 {
        format!("{seconds}s")
    } else if seconds % 60 == 0 {
        format!("{}m", seconds / 60)
    } else {
        format!("{}m {}s", seconds / 60, seconds % 60)
    };
    format!("{title} for {duration}")
}

struct Message {
    start: usize,
    display_start: usize,
    role: String,
    parser: MarkdownParser,
    markdown_blocks: Vec<RenderedBlock>,
    under_line: bool,
    /// The turn this message belongs to, as `Body::Actions` numbers it.
    turn: usize,
    attachments: Arc<[attachments::Attachment]>,
    /// A user message's displayed chunks and attachments, which its cached
    /// bubble width depends on.
    prompt_fingerprint: u64,
    /// parseUserMessageLink: the prompt's first web link, with the index of
    /// the text chunk that holds it.
    user_link: Option<(usize, Arc<UserMessageLink>)>,
}
struct RenderedBlock {
    source: Arc<crate::chat::markdown::model::TopBlock>,
    rows: Vec<Arc<MarkdownRow>>,
    words: usize,
}
#[derive(Clone)]
enum Body {
    RemotePlan(crate::chat::conversation::remote_plans::Plan),
    SavedDraft,
    Markdown(Arc<MarkdownRow>),
    /// A dithered break between text replies in one Mono turn.
    MonoDivider,
    Visual(Arc<monocode_core::session::visual::VisualDocument>),
    Plain(SharedString),
    Interjection(monocode_core::session::transcript::Interjection),
    MonoCard(serde_json::Value),
    /// A queued message's bubble, by its index in `pending`.
    Pending(usize),
    /// LaunchedSessionCard for a session the run started.
    Launch(i64),
    /// A commit pill turn's status line, then its card.
    GitAction(Arc<git_turns::GitCard>),
    Tool {
        call: Box<ToolCall>,
        group: Option<usize>,
        tail: bool,
        /// Its phase is still the live end of the turn.
        live: bool,
    },
    Phase {
        title: String,
        group: usize,
        icon: &'static str,
        active: bool,
        /// Lines the group's edits added and removed.
        counts: (usize, usize),
        /// The images its steps showed, as thumbnails at the line's end.
        images: Arc<[images::StepImage]>,
    },
    WorkLine {
        turn: usize,
        expandable: bool,
    },
    Reasoning,
    Actions {
        turn: usize,
        complete: bool,
    },
}
#[derive(Clone)]
struct UiRow {
    key: (usize, usize),
    body: Body,
    message: usize,
    first: bool,
    last: bool,
    work: bool,
    work_tail: bool,
    /// Reasoning headers and their expanded Markdown share the tool phase.
    thought_phase: Option<(usize, bool)>,
}
impl UiRow {
    fn phase_group(&self) -> Option<usize> {
        self.thought_phase
            .map(|(group, _)| group)
            .or(match self.body {
                Body::Tool { group, .. } => group,
                _ => None,
            })
    }
    fn in_phase(&self) -> bool {
        self.phase_group().is_some()
    }
}
const TOP_INSET: f32 = 20.;
// Saved drafts are presentation rows, outside the provider transcript.
const SAVED_DRAFT_START: usize = usize::MAX - 1;
const BOTTOM_INSET: f32 = 40.;
#[cfg(target_os = "macos")]
const TOP_FADE: f32 = 24.;
/// AgentTranscript.tsx's user bubble: `text-sm`, `max-w-[36rem]` and `px-3`.
/// Measurement, line counting and render share these.
const PROMPT_TEXT_SIZE: Pixels = px(14.);
const PROMPT_BUBBLE_MAX: Pixels = px(576.);
/// Horizontal padding on both sides together.
const PROMPT_PAD_X: Pixels = px(24.);
/// Text width that leaves room for "Show less" on a blank last chunk.
const PROMPT_DISCLOSURE_MIN: Pixels = px(72.);
/// `.turn-actions-hover`'s 120ms opacity transition, in seconds.
const HOVER_FADE: f32 = 0.12;

/// useBottomChatMotion's follow: the list keeps its real position at the end
/// while the painted chat eases into it, so a sent message or a streamed reply
/// glides up instead of jumping.
#[derive(Default)]
struct Follow {
    offset: f32,
    velocity: f32,
    growth: f32,
    /// Growth since the spring last stepped, the feed-forward's sample.
    grown: f32,
    measured: Option<FollowMeasure>,
    last_frame: Option<Instant>,
    #[cfg(test)]
    frame_elapsed: Option<f32>,
    last_step: Option<Instant>,
    settled_at: Option<Instant>,
    /// Growth keeps easing this long after a reply ends.
    until: Option<Instant>,
}
/// Units are px per 60fps frame, as in useBottomChatMotion.ts.
const SPRING_FRAME: f32 = 1000. / 60.;
/// Row motion redraws the whole transcript, so it runs at no more than 60fps
/// even on 120Hz displays; the slack keeps a 60Hz display at its full rate.
const MOTION_FRAME: std::time::Duration = std::time::Duration::from_millis(15);
/// Mono replies can appear together; keep their glide below 1080px/second.
const CHAT_FOLLOW_MAX_SPEED: f32 = 18.;
/// Steady frames only measure the tail. A large reveal measures its full
/// new distance once before handing over to this bounded window.
const FOLLOW_ROWS: usize = 80;

struct FollowMeasure {
    viewport: gpui::Size<Pixels>,
    bottom: Pixels,
    window_size: gpui::Size<Pixels>,
    rows: Vec<((usize, usize), f32)>,
    /// Regular chats fill the space below an anchored prompt before scrolling.
    footer: f32,
    /// A top-aligned chat that fits the viewport has no scroll distance yet.
    slack: f32,
}
/// The first row of the prompt block that opens `turn`.
pub(crate) fn prompt_start(source: &[Row], turn: usize) -> usize {
    let mut at = turn.min(source.len());
    while at > 0 && source[at - 1].is_prompt() {
        at -= 1;
    }
    at
}
/// groupMonoChatTurns: the turn a completion run at `turn` continues, if any.
pub(crate) fn mono_continues(source: &[Row], turn: usize) -> Option<usize> {
    let last = turn.checked_sub(1)?;
    if !source.get(last).is_some_and(|r| r.is_prompt()) {
        return None;
    }
    let prompt = prompt_start(source, turn);
    if prompt == 0
        || source[prompt..turn].iter().any(|r| r.role != "mono-system")
        || source[prompt - 1].is_prompt()
        || dates::mono_opens_stretch(source, prompt)
    {
        return None;
    }
    Some(
        source[..prompt]
            .iter()
            .rposition(|r| r.is_prompt())
            .map_or(0, |i| i + 1),
    )
}
/// A reasoning summary's heading, as Codex writes them (`**Finding the repo**`),
/// when it is short enough for a status line.
fn reasoning_heading(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let heading = line
        .strip_prefix("**")
        .and_then(|l| l.strip_suffix("**"))
        .or_else(|| line.strip_prefix('#').map(|l| l.trim_start_matches('#')))?
        .trim()
        .trim_end_matches(['.', ':', '…']);
    let words = heading.split_whitespace().count();
    ((1..=8).contains(&words) && heading.chars().count() <= 60).then(|| heading.to_owned())
}
/// What a running tool is doing, in the user's words where we can tell.
fn tool_status(tool: &ToolCall) -> String {
    let shown = tool.presentation();
    let app = monocode_core::session::tool::app_tool(&tool.name)
        || shown
            .command
            .as_deref()
            .and_then(monocode_core::session::tool::app_cli_action)
            .is_some();
    if app {
        return format!("{} {}…", shown.action, shown.target);
    }
    let (action, _) = tool.summary();
    match action.as_str() {
        "Run" => {
            // The model's own one-line description of the command, when given.
            if let Some(description) = shown
                .description
                .as_deref()
                .map(|d| d.trim().trim_end_matches(['.', '…']))
                .filter(|d| !d.is_empty() && d.chars().count() <= 60)
            {
                return format!("{description}…");
            }
            command_status(shown.command.as_deref().unwrap_or("")).into()
        }
        "Edit" | "Write" => "Editing file…".into(),
        "Read" => "Reading file…".into(),
        "Search" => "Searching the web…".into(),
        "Find" | "List" | "Glob" | "Grep" => "Exploring project…".into(),
        "Agent" | "Task" => "Running agent…".into(),
        _ => mcp_status(&tool.name).unwrap_or_else(|| "Using tool…".into()),
    }
}
/// A status for a shell command the model did not describe.
fn command_status(command: &str) -> &'static str {
    let mut words = command.split_whitespace();
    match (words.next().unwrap_or(""), words.next().unwrap_or("")) {
        ("gh", "pr") => "Looking at pull requests…",
        ("gh", "issue") => "Looking at issues…",
        ("gh", "run" | "workflow") => "Checking CI…",
        ("gh", "search" | "repo") => "Searching GitHub…",
        ("gh", _) => "Checking GitHub…",
        ("git", _) => "Checking git…",
        ("curl" | "wget", _) => "Fetching from the web…",
        ("npm" | "bun" | "pnpm" | "yarn" | "cargo" | "make", _) => "Running a build…",
        _ => "Running command…",
    }
}
/// `mcp__github__search_repositories` → "Search repositories…".
fn mcp_status(name: &str) -> Option<String> {
    let tool = name
        .strip_prefix("mcp__")
        .and_then(|rest| rest.split_once("__"))
        .map(|(_, tool)| tool)?;
    let words = tool.replace(['_', '-'], " ");
    let mut chars = words.trim().chars();
    let first = chars.next()?;
    Some(format!("{}{}…", first.to_uppercase(), chars.as_str()))
}
/// isEmojiOnlyMessage: up to three emoji, which read as a reaction.
pub(crate) fn is_emoji_only(text: &str) -> bool {
    use unicode_segmentation::UnicodeSegmentation;
    let mut count = 0;
    for grapheme in text.trim().graphemes(true) {
        if grapheme.trim().is_empty() {
            continue;
        }
        let first = grapheme.chars().next().unwrap_or_default() as u32;
        let picture = matches!(first, 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0x2300..=0x23FF | 0x3030 | 0x303D | 0x3297 | 0x3299 | 0x00A9 | 0x00AE | 0x203C | 0x2049 | 0x2122 | 0x2139 | 0x2194..=0x21AA)
            || grapheme.contains('\u{20E3}');
        count += 1;
        if !picture || count > 3 {
            return false;
        }
    }
    count > 0
}

/// A single clipping boundary across virtualized siblings, matching React's
/// grid body. Fully revealed steps keep their height; only the boundary row clips.
struct PhaseLayout {
    width: Pixels,
    height: f32,
    rows: HashMap<(usize, usize), (f32, f32)>,
}
impl PhaseLayout {
    fn row_progress(&self, key: (usize, usize), progress: f32) -> f32 {
        let (top, height) = self.rows[&key];
        if height <= 0. {
            return 0.;
        }
        ((self.height * progress - top) / height).clamp(0., 1.)
    }
}

pub(crate) struct TranscriptView {
    selection: selection::Selection,
    selection_focus: Option<gpui::FocusHandle>,
    links: links::State,
    work_item_cards: user_links::Cards,
    source: Vec<Row>,
    source_bytes: usize,
    attachment_images: HashMap<String, gpui::Entity<attachments::Photo>>,
    visuals: HashMap<usize, visuals::CachedVisual>,
    /// Visual ids the answers place in their prose.
    placed: HashSet<String>,
    /// Placed visuals by id: their message's start and document.
    published: HashMap<String, (usize, Arc<monocode_core::session::visual::VisualDocument>)>,
    history_position: Option<history::HistoryPosition>,
    history_start: usize,
    history_loading: bool,
    history_requested: bool,
    /// Rows of older pages above the store's window. Store row `i` is `base + i`.
    base: usize,
    /// The store cursor for the turns above the loaded rows, if any.
    history_before: Option<i64>,
    /// The store cursor for the turns just above the window, which older
    /// pages loaded here don't move: a paired phone pages from the window.
    window_before: Option<i64>,
    history_pager: Option<history::HistoryPager>,
    /// The last page read failed; only a scroll retries it.
    history_failed: bool,
    history_search: Option<(usize, String)>,
    prepared_parsers: HashMap<usize, MarkdownParser>,
    mono: bool,
    mono_groups: std::cell::RefCell<HashMap<usize, usize>>,
    /// The Mono's mascot, colour and name, which sign its turns.
    mono_look: Option<(String, u32, SharedString)>,
    /// The turn whose actions show, and since when, as `.turn-actions-hover`.
    hovered_turn: Option<(usize, Instant)>,
    /// The prompt message whose copy action shows.
    hovered_prompt: Option<usize>,
    /// The prompt whose action strip the pointer is on. The strip hangs
    /// below the row's hitbox, so leaving the row must not hide it.
    hovered_prompt_actions: Option<usize>,
    /// Every hovered row and its turn. Entering the next row can arrive
    /// before leaving the last, so the turn is hovered while any row is.
    hover_rows: HashMap<(usize, usize), usize>,
    /// Messages entering the chat, by source start, as useBottomChatMotion.
    entrances: HashMap<usize, Instant>,
    /// The emoji a Mono answered a prompt with, by the prompt's start.
    reactions: HashMap<usize, (String, Option<Instant>)>,
    follow: Follow,
    /// monoPendingTranscriptBlocks: queued messages, after the transcript.
    pending: Vec<PendingMessage>,
    /// The outbox keeps this message until storage accepts it, but its prompt
    /// and Thinking header are already painted in the source transcript.
    submitted_mono_message: Option<String>,
    saved_draft: Option<monocode_core::session::follow_up::Message>,
    /// Pending messages just taken, including any entrance still in flight.
    taken: Vec<TakenMessage>,
    pending_entrances: HashMap<String, Instant>,
    launches: HashMap<i64, LaunchView>,
    /// A clock for the tool and task shimmers.
    started_at: Instant,
    /// One clock per frame for row motion. The list measures rows and the
    /// footer re-measures them; mixed times leave the spacer a few pixels
    /// short of a fast disclosure, so an anchored prompt bounces.
    frame_now: Instant,
    measurements: measurements::Measurements,
    markdown_cache: markdown::RenderCache,
    messages: Vec<Message>,
    rows: Vec<UiRow>,
    streaming: bool,
    perf_scroll: perf_scroll::PerfScroll,
    chat_layout: bool,
    anchor_prompts: bool,
    smooth_reveal: bool,
    /// Tool groups open only when clicked, even while running.
    collapse_groups: bool,
    /// Finished replies fold their work under "worked for".
    fold_work: bool,
    search_match: Option<(usize, String)>,
    highlights: HashMap<u64, Option<Arc<HighlightedDocument>>>,
    highlighting: HashSet<u64>,
    highlight_queue: VecDeque<Arc<Fence>>,
    highlight_active: bool,
    highlight_epoch: u64,
    highlight_order: VecDeque<(u64, usize)>,
    highlight_bytes: usize,
    copied: Option<u64>,
    copied_response: Option<(usize, Instant)>,
    note_pending: HashSet<usize>,
    note_feedback: HashMap<usize, Result<(), String>>,
    action_anchors: HashMap<(usize, &'static str), AnchorSlot>,
    hovered_action: Option<(usize, &'static str)>,
    /// Step thumbnails' anchors and the one pointed at, by strip and index.
    image_anchors: HashMap<(usize, usize), AnchorSlot>,
    hovered_image: Option<(usize, usize)>,
    /// Each work phase's tile, its own view so a hover redraws only it.
    phase_tiles: HashMap<usize, Entity<crate::chat::transcript::phase_tile::PhaseTile>>,
    /// Each reply's "worked for" tile, keyed by turn.
    work_tiles: HashMap<usize, Entity<crate::chat::transcript::phase_tile::PhaseTile>>,
    pub(crate) target_action: Option<(usize, monocode_core::session::response::TransferKind)>,
    document: bool,
    side_question: bool,
    pub(crate) file_link_base: Option<String>,
    expanded: HashSet<usize>,
    agent_panels: tasks::AgentPanels,
    /// User messages opened past their four-line clamp, by start.
    expanded_prompts: HashSet<usize>,
    /// Collapsed user messages whose later chunks are hidden, by start.
    clamped_prompts: HashSet<usize>,
    phases: HashMap<usize, bool>,
    phase_toggle: Option<usize>,
    phase_layouts: HashMap<usize, PhaseLayout>,
    phase_scrolls: HashMap<usize, live_phase::LiveScroll>,
    windowed_phases: HashSet<usize>,
    phase_max_height: Pixels,
    work_folds: HashMap<usize, bool>,
    reveals: HashMap<usize, PacedText>,
    live_start: usize,
    motions: HashMap<(usize, usize), Motion>,
    reflow: crate::ui::motion::reflow::ReflowMotion,
    frame_scheduled: bool,
    last_motion_frame: Option<Instant>,
    visible: bool,
    viewport: Bounds<Pixels>,
    anchor_turn: Option<usize>,
    prompt_rise: Option<(usize, Instant)>,
    /// The mode or /visualize burst inside a just-sent prompt.
    burst: Option<burst::Burst>,
    prompt_rise_pending: bool,
    introduced_prompt: Option<usize>,
    show_jump: bool,
    commit_files: git_turns::CommitFiles,
    copied_commit: Option<(String, Instant)>,
    /// The conversation's git action pill holds the column's left end, so the
    /// jump button moves from the center to the right end.
    jump_beside_pill: bool,
    jump_anchor: crate::ui::popover::AnchorSlot,
    scroll_bound: bool,
    fold_targets: HashMap<usize, Option<usize>>,
    step_queue: HashMap<usize, Instant>,
    seen_steps: HashSet<usize>,
    pub(crate) turns: HashMap<usize, monocode_core::session::agent::TurnInfo>,
    provider: String,
    model: String,
    active_provider_turn_id: Option<String>,
    started: Option<std::time::Instant>,
    elapsed: HashMap<usize, std::time::Duration>,
    outline: outline::Outline,
    pub(crate) list: ListState,
    pub(crate) rendered: usize,
    pub(crate) render_passes: usize,
    #[cfg(test)]
    rebuild_passes: usize,
}
impl TranscriptView {}
impl gpui::EventEmitter<ResponseRequest> for TranscriptView {}
/// The frosted fill behind the round controls floating over the transcript.
pub(crate) fn floating_surface() -> impl IntoElement {
    canvas(
        |_, _, _| {},
        |bounds, _, window, _| {
            let radius = bounds.size.height / 2.;
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            window.paint_quad(
                gpui::fill(bounds, content(0.04))
                    .corner_radii(radius)
                    .border_color(gpui::backdrop_blur_marker(12. * window.scale_factor())),
            );
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            window.paint_quad(gpui::fill(bounds, rgb(CHAT)).corner_radii(radius));
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

fn paint_edge_fade(bounds: Bounds<Pixels>, begin: bool, window: &mut Window) {
    #[cfg(target_os = "macos")]
    window.paint_quad(gpui::fill(bounds, gpui::black()).border_color(
        gpui_apple::metal_renderer::edge_fade_marker(
            begin,
            TOP_FADE * window.scale_factor(),
            BOTTOM_INSET * window.scale_factor(),
        ),
    ));
    // A color overlay cannot mask text without tinting the pane and creating
    // a seam above the composer. Only the Metal backend supports a real mask.
    #[cfg(not(target_os = "macos"))]
    let _ = (bounds, begin, window);
}

#[cfg(test)]
use mono::mono_work_label;
#[cfg(test)]
mod tests;
