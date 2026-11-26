mod accounts;
mod resume;
pub(crate) use resume::Resumed;
mod approvals;
mod attachment_drop;
mod background_tasks;
mod btw;
mod changes_shelf;
mod claude_terminal;
mod commands;
pub(crate) mod composer_features;
mod composer_shelf;
mod edit_last_turn;
mod follow_up;
pub(crate) mod git_actions;
mod history;
mod lifecycle;
mod mobile;
mod model_menu;
mod mono;
mod notifications;
mod remote;
pub(crate) mod remote_plans;
mod response_menu;
mod send;
mod title;
mod view;
mod worktree_naming;
use crate::{
    chat::{
        composer::{
            motion::ComposerMotion,
            pickers::{
                ModelList, ModelPicked, cached_model_name, model_display_name, model_name,
                remember_model,
            },
        },
        response_actions::{Request as ResponseRequest, Transfer},
        transcript::TranscriptView,
    },
    git::picker::{GitPicker, Mode as GitMode, Picked as GitPicked},
    runtime::{Database, locked, run_turn},
    ui::{
        input,
        popover::{
            Align, Anchor, AnchorSlot, Flyout, Menu, MenuAction, MenuEvent, MenuItem, MenuStyle,
            Side, menu_popover, popover,
        },
        theme::*,
    },
};
pub(crate) use accounts::UsageResumed;
use gpui::{
    AnyElement, Context, CursorStyle, Entity, EventEmitter, Focusable, MouseButton, Role,
    SharedString, StyleRefinement, Subscription, Window, canvas, div, prelude::*, px,
};
pub(crate) use mobile::{
    Waiting as MobileWaiting, mobile_access_modes, usage_state as mobile_usage_state,
};
pub(crate) use mono::QueueChanged as MonoQueueChanged;
pub(crate) use mono::Submitted as MonoSubmitted;
use monocode_core::{provider::Kind, store::Session};
use monocode_core::{
    provider::catalog::{self, Model},
    session::agent::{AccessMode, Approval, Control, Decision, Settings},
};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use view::{composer_surface, send_button};
// Keep empty and docked composers aligned with the transcript column. Below
// this width the empty session drops its title.
const COMPOSER_COLUMN: f32 = COLUMN;
const COMPOSER_RADIUS: f32 = 12.;
// RUNTIME_MODE_LABEL and RUNTIME_MODE_HINT from the React session model,
// with AccessPicker's icons; Full access is amber.
const ACCESS_MODES: &[(&str, &str, &str, &str)] = &[
    (
        "supervised",
        "Manual",
        "Always ask before making changes.",
        "square-lock",
    ),
    (
        "auto-accept-edits",
        "Accept edits",
        "Automatically accept all file edits.",
        "pencil-edit",
    ),
    (
        "auto",
        "Auto",
        "An AI reviewer can approve or deny actions.",
        "sparkles",
    ),
    (
        "full-access",
        "Bypass permissions",
        "Allow commands, edits, and supported MCP confirmations in non-plan turns without prompts.",
        "shield-alert",
    ),
];
// Tailwind v4 accents the composer menus tint their icons with.
const YELLOW_300: u32 = 0xffdf20;
const SKY_300: u32 = 0x74d4ff;
const FUCHSIA_300: u32 = 0xf4a8ff;
const AMBER_400: u32 = 0xffb900;
fn tint(color: u32, alpha: f32) -> gpui::Hsla {
    let mut c: gpui::Hsla = gpui::rgb(color).into();
    c.a = alpha;
    c
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ComposerMenu {
    Add,
    Model,
    /// ModelPicker's recent models, on right-click or `App: Switch Model`.
    Recent,
    Access,
    Account,
    Workspace,
    /// A Mono's model setting pill: 0 is effort, then each model control.
    Setting(usize),
    /// The commit pill's chevron above the composer.
    GitActions,
}
struct OpenMenu {
    kind: ComposerMenu,
    menu: Entity<Menu>,
    _events: Vec<Subscription>,
}
use response_menu::ResponseMenu;
pub(crate) struct SessionChanged;
/// A Claude CLI moved its tab to another conversation; this is the entry that
/// now lists the one it left.
pub(crate) struct SessionLeft(pub(crate) Session);
pub(crate) struct ChooseFolder(pub(crate) AnchorSlot);
/// The review card asked to open the session's recorded changes.
pub(crate) struct OpenSessionChanges(pub(crate) crate::git::review::ReviewTab);
/// A fork of this conversation to open in a tab, with the message it was cut
/// at (and that message's attachments) waiting in its composer.
pub(crate) struct Forked(
    pub(crate) Session,
    pub(crate) String,
    pub(crate) Vec<std::path::PathBuf>,
);
pub(crate) struct Conversation {
    editing_last_turn: edit_last_turn::State,
    /// Which earlier message ↑ brought back into the empty composer, counting
    /// back from the newest.
    recall: Option<usize>,
    btw: Option<Entity<btw::Btw>>,
    /// The composer box the /btw sheet morphs out of, and how visible the
    /// composer is under the sheet.
    btw_home: (AnchorSlot, f32),
    _btw_observer: Option<Subscription>,
    pub(crate) summary: Session,
    remote_state: remote::State,
    remote_plans: remote_plans::State,
    store: Database,
    pub(crate) transcript: Entity<TranscriptView>,
    pub(crate) review: Entity<crate::git::session_review::SessionReview>,
    _review_events: Subscription,
    _review_observation: Subscription,
    pub(crate) input: Entity<input::Input>,
    completion: Entity<crate::chat::composer::completion::Completion>,
    _completion_event: Subscription,
    _command_removal: Subscription,
    _draft_observation: Subscription,
    _input_focus: Option<[Subscription; 2]>,
    draft_has_value: bool,
    folder_selected: bool,
    follow_ups: follow_up::FollowUps,
    background: background_tasks::BackgroundTasks,
    changes: changes_shelf::ChangesShelf,
    attachments: Vec<std::path::PathBuf>,
    pending_attachments: Vec<std::path::PathBuf>,
    attachment_photos: std::collections::HashMap<
        std::path::PathBuf,
        Entity<crate::chat::transcript::attachments::Photo>,
    >,
    attaching: bool,
    file_drag: bool,
    attachment_error: Option<String>,
    app_environment: std::collections::HashMap<String, String>,
    /// The running turn's policy, independent of the next composer's mode.
    pub(crate) app_read_only: bool,
    planning_orchestration: bool,
    pub(crate) orchestration_plan: Option<monocode_core::session::orchestration::Plan>,
    orchestration_error: Option<String>,
    pub(crate) orchestration_progress: Option<String>,
    status: String,
    loaded: bool,
    history_unloaded: bool,
    history_actions: std::collections::VecDeque<history::HistoryAction>,
    pending_search: Option<(usize, String)>,
    loading: bool,
    pub(crate) busy: bool,
    /// The UI is idle; serialize the next launch behind the worker's last save.
    stopping: bool,
    notification_cancelled: bool,
    /// The store's saved-draft flag, until the conversation loads its own.
    pub(crate) draft_hint: bool,
    /// The provider model for the next turn; `None` uses its configured default.
    model: Option<String>,
    resolved_model: Option<String>,
    pub(crate) settings: Settings,
    pub(crate) mono_turn: u64,
    pub(crate) submission_generation: u64,
    mono_ui: mono::ChatState,
    project_appearance: monocode_core::project::appearance::ProjectAppearance,
    classic_layout: bool,
    catalog: Vec<Model>,
    catalog_epoch: u64,
    account_switching: bool,
    _usage_clock: gpui::Task<()>,
    _account_usage: Option<Subscription>,
    approvals: std::collections::VecDeque<Approval>,
    approval_ui: approvals::State,
    questions: std::collections::VecDeque<(
        Entity<crate::chat::questions::QuestionCard>,
        Vec<Subscription>,
    )>,
    question_overlay: crate::chat::questions::Overlay,
    controls: Option<std::sync::mpsc::Sender<Control>>,
    branch: Option<String>,
    git: Option<monocode_core::vcs::git::Snapshot>,
    git_picker: Option<Entity<GitPicker>>,
    git_events: Option<Subscription>,
    preparing_worktree: bool,
    context: Option<monocode_core::session::context::Usage>,
    context_hovered: bool,
    /// The submit button's morph between square and labelled pill.
    send_morph: std::cell::RefCell<crate::chat::composer::send_morph::Morph>,
    /// ⌘ (Ctrl elsewhere) as last seen, which flips Queue and Steer.
    send_alternate: bool,
    menu: Option<OpenMenu>,
    add_anchor: AnchorSlot,
    model_anchor: AnchorSlot,
    access_anchor: AnchorSlot,
    account_anchor: AnchorSlot,
    workspace_anchor: AnchorSlot,
    git_action_anchor: AnchorSlot,
    /// Files this session's edit tools changed, across its whole history.
    session_edits: Vec<String>,
    /// Where `session_edits` stand in Git: uncommitted ones show the pill.
    session_git: monocode_core::vcs::git::SessionChanges,
    /// The session's latest action commit is on no remote yet: the pill offers Push.
    session_unpushed: bool,
    session_edits_generation: u64,
    branch_anchor: AnchorSlot,
    context_anchor: AnchorSlot,
    composer_anchor: AnchorSlot,
    composer_motion: ComposerMotion,
    narrow: bool,
    // A split or review pane sits beside this chat; only then does a narrow
    // pane rest the composer where it docks.
    beside_pane: bool,
    arcade: Option<Entity<crate::chat::arcade::Arcade>>,
    ink: Option<Entity<crate::chat::ink_field::InkField>>,
    runner: Option<Entity<crate::chat::composer::runner::Runner>>,
    cancel: Arc<AtomicBool>,
    title_cancel: Option<Arc<AtomicBool>>,
    worktree_name_cancel: Option<Arc<AtomicBool>>,
    /// The Claude Code CLI shown in place of the chat.
    claude_terminal: Option<claude_terminal::State>,
    /// A CLI start still looking things up (Ok), or why the last one failed.
    claude_start: Option<Result<gpui::Task<()>, String>>,
    /// A Claude session is moving between the CLI and the chat.
    switching_mode: bool,
    /// `/resume`'s session list while the composer holds the command.
    resume: Option<resume::Picker>,
    /// What this Claude session did, for the agents and in-this-chat panels.
    claude_activity: Option<Entity<crate::chat::claude::panels::ClaudeActivity>>,
    _submission: Subscription,
    _alternate_submission: Subscription,
    response_events: Option<Subscription>,
    response_menu: Option<ResponseMenu>,
    response_request: Option<(
        usize,
        monocode_core::session::response::TransferKind,
        Instant,
    )>,
}
impl Drop for Conversation {
    fn drop(&mut self) {
        if let Some(cancel) = self.worktree_name_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        if !self.summary.cwd.starts_with("remote://") {
            self.cancel.store(true, Ordering::Relaxed);
        }
        self.cancel_title_generation();
    }
}
impl Conversation {}

impl EventEmitter<SessionChanged> for Conversation {}
impl EventEmitter<SessionLeft> for Conversation {}
impl EventEmitter<ChooseFolder> for Conversation {}
impl EventEmitter<OpenSessionChanges> for Conversation {}
impl EventEmitter<Forked> for Conversation {}
impl EventEmitter<Resumed> for Conversation {}
impl EventEmitter<Transfer> for Conversation {}
impl EventEmitter<composer_features::StartOrchestration> for Conversation {}
impl EventEmitter<composer_features::StopOrchestration> for Conversation {}

impl Conversation {}

/// For profiling without clicking: with `MONOCODE_PERF_PROMPT=<text>`, the
/// first chat created switches to Claude and sends that text once, a few
/// seconds after launch. scripts/perf/frame-run.sh sets it. It only runs
/// while `MONOCODE_CLAUDE_BINARY` replaces `claude`, so it never reaches a
/// real provider.
fn perf_prompt(cx: &mut Context<Conversation>) {
    static SENT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let Some(prompt) = std::env::var("MONOCODE_PERF_PROMPT")
        .ok()
        .filter(|prompt| !prompt.is_empty())
    else {
        return;
    };
    if std::env::var_os("MONOCODE_CLAUDE_BINARY").is_none()
        || SENT.swap(true, std::sync::atomic::Ordering::Relaxed)
    {
        return;
    }
    cx.spawn(async move |this, cx| {
        cx.background_executor()
            .timer(std::time::Duration::from_secs(3))
            .await;
        this.update(cx, |this, cx| {
            if this.summary.has_messages {
                return;
            }
            let claude = monocode_core::provider::Kind::Claude.id();
            if this.summary.provider != claude {
                this.summary.provider = claude.into();
                this.summary.provider_id = None;
                this.model = None;
                this.completion
                    .update(cx, |completion, cx| completion.set_provider(claude, cx));
                this.catalog.clear();
            }
            this.launch_prompt(prompt, cx);
        })
        .ok();
    })
    .detach();
}

/// The access mode the last composer pick left as the default for new chats.
pub(crate) fn default_access(cx: &gpui::App) -> AccessMode {
    cx.try_global::<crate::ui::preferences::Preferences>()
        .and_then(|prefs| prefs.default_access.as_deref())
        .map(AccessMode::from_id)
        .unwrap_or_default()
}
pub(crate) fn remember_access(mode: AccessMode, cx: &mut gpui::App) {
    if cx.has_global::<crate::ui::preferences::Preferences>() {
        cx.update_global::<crate::ui::preferences::Preferences, _>(|prefs, _| {
            prefs.default_access = Some(mode.id().into());
        });
    }
}
fn is_effort_key(key: &str) -> bool {
    matches!(
        key,
        "effort" | "reasoning" | "reasoningEffort" | "reasoning_effort"
    )
}
pub(crate) fn preferred_effort(provider: &str, cx: &gpui::App) -> Option<String> {
    cx.try_global::<crate::ui::preferences::Preferences>()
        .and_then(|prefs| prefs.provider_efforts.get(provider).cloned())
}
pub(crate) fn effort_name(value: &str) -> String {
    match value {
        "default" => "Default",
        "none" => "None",
        "minimal" => "Minimal",
        "low" => "Low",
        "medium" => "Medium",
        "high" => "High",
        "xhigh" => "Extra High",
        "max" => "Max",
        "ultra" => "Ultra",
        "ultracode" => "Ultracode",
        "ultrathink" => "Ultrathink",
        other => other,
    }
    .into()
}

fn model_control_toggle(control: &catalog::ModelControl) -> bool {
    control.choices.len() == 2
        && control.choices.iter().any(|(value, _)| value == "true")
        && control.choices.iter().any(|(value, _)| value == "false")
}

// The composer's `h-6.5` toolbar pills share `bg-selection` and its hover.
fn tool_pill(id: &'static str, label: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .group(id)
        .cursor_pointer()
        .relative()
        .flex_shrink_0()
        .h(px(26.))
        .px(px(6.))
        .rounded_md()
        .flex()
        .items_center()
        .gap_1()
        .bg(content(0.1))
        .text_color(content(1.))
        .hover(|s| s.bg(content(0.15)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}
fn pill_label(label: impl Into<SharedString>) -> impl IntoElement {
    div()
        .min_w_0()
        .truncate()
        .text_size(px(11.))
        .child(label.into())
}

/// The checked-out branch (or short detached commit) for `cwd`, read from the
/// repository's HEAD, including linked worktrees whose `.git` is a file.
pub(crate) fn git_branch(cwd: &Path) -> Option<String> {
    let root = cwd.ancestors().find(|d| d.join(".git").exists())?;
    let dot = root.join(".git");
    let git = if dot.is_dir() {
        dot
    } else {
        let link = std::fs::read_to_string(&dot).ok()?;
        root.join(link.strip_prefix("gitdir:")?.trim())
    };
    let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
    let head = head.trim();
    Some(match head.strip_prefix("ref: refs/heads/") {
        Some(branch) => branch.to_owned(),
        None => head.chars().take(7).collect(),
    })
}

#[cfg(test)]
mod tests;

/// The amber bolt the model trigger carries while fast mode is on.
pub(crate) fn fast_bolt() -> gpui::Div {
    div()
        .flex_shrink_0()
        .child(icon("zap", 12.).text_color(gpui::rgb(0xfbbf24)))
}
