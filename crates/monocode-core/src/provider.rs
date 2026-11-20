//! Native provider transports, ported from Monocode's TS adapters.
//! Blocking work belongs to a worker; never call `run` from a render/input handler.
use crate::session::agent::{AccessMode, Approval, Control, Decision, Settings, TurnMetrics};
use crate::session::tool::{ToolCall, ToolState};
use crate::{session::transcript::snapshot_remainder, sys::process};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};

mod acp;
mod commands;
pub use commands::{
    compact, discover_commands, native_invocation, native_prompt, prepare_prompt, supports_compact,
    NativeCommand, NativeSubcommand,
};
pub mod accounts;
mod attachments;
pub mod catalog;
pub mod cli;
mod compaction;
pub(crate) mod devin;
mod discovery;
mod follow_up;
mod lifecycle;
mod limits;
pub mod mcp;
mod opencode;
pub mod operation;
mod parked;
mod pi;
mod protocol;
mod protocol_tools;
mod questions;
mod rewind;
pub mod skills;
mod tasks;
pub mod usage;
pub mod visual_bridge;
pub fn configure_idle_agents(limit: usize) {
    parked::configure(limit);
    opencode::configure(limit);
}
pub fn keep_idle_agents(ids: Vec<i64>) {
    parked::keep(ids);
}
pub fn discard_worktree(path: &Path) {
    parked::discard_worktree(path);
    opencode::discard_worktree(path);
}
pub fn forget_session(kind: Kind, provider_id: &str, account_id: &str, cwd: &Path) {
    parked::forget_session(kind, provider_id, account_id, cwd);
    if kind == Kind::OpenCode {
        opencode::forget_session(provider_id, account_id, cwd);
    }
}
pub(crate) use rewind::claude_resume;
pub use rewind::{
    claude_first_prompt, claude_live_session, claude_transcript, rewind_to, supports_fork,
    supports_rewind, RewindError,
};
/// An id for a Claude session that has not been started yet.
pub fn claude_session_id() -> String {
    lifecycle::claude_session_id()
}
pub use crate::session::provider_kind::Kind;

const BATCH_BYTES: usize = 64 * 1024;
const TICK: Duration = Duration::from_millis(16);

#[derive(Clone, Debug)]
pub struct Request {
    pub kind: Kind,
    pub binary: PathBuf,
    pub cwd: PathBuf,
    pub prompt: String,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub settings: Settings,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Ready,
    Commands(Vec<NativeCommand>),
    FollowUp(crate::session::follow_up::Message),
    FollowUpFailed {
        id: String,
        error: String,
    },
    SessionId(String),
    TurnId(String),
    Model(String),
    Context {
        used: Option<u64>,
        window: Option<u64>,
    },
    Metrics(TurnMetrics),
    Approval(Approval),
    ApprovalResolved(String),
    Question(crate::session::agent::Question),
    QuestionResolved(String),
    BeginAssistant,
    Text(String),
    Reasoning(String),
    Tool(ToolCall),
    Status(String),
    /// What the turn still waits on after the provider yielded; empty clears it.
    Background(Vec<BackgroundTask>),
    UsageLimited(crate::session::agent::Limit),
    Interjection {
        text: String,
        custom_type: String,
        severity: Option<String>,
    },
    ConfigChanged {
        model: Option<String>,
        model_settings: HashMap<String, String>,
    },
    Done,
}

/// A command or subagent left running after the provider yielded its answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackgroundTask {
    pub id: String,
    pub description: String,
    pub agent: bool,
    /// The shell command behind a backgrounded Bash call, when known.
    pub command: Option<String>,
    /// Unix milliseconds.
    pub started_at: u64,
}

fn codex_error(error: &Value, fallback: &str) -> String {
    let message = error["message"].as_str().unwrap_or(fallback);
    if error["codexErrorInfo"] == "usageLimitExceeded" {
        format!("Usage limit reached: {message}")
    } else {
        message.into()
    }
}

/// GUI apps may not inherit the login shell PATH. Try the same common user and
/// system install locations as harness.rs, plus Claude's versioned native install.
/// An explicit override wins and errors at launch rather than silently falling back.
pub fn resolve(kind: Kind) -> Option<PathBuf> {
    if let Some(path) = crate::provider::cli::configured(kind) {
        return Some(path);
    }
    resolve_auto(kind)
}
pub fn resolve_auto(kind: Kind) -> Option<PathBuf> {
    let key = format!("MONOCODE_{}_BINARY", kind.id().to_uppercase());
    if let Some(path) = std::env::var_os(key).filter(|s| !s.is_empty()) {
        return Some(path.into());
    }
    let mut candidates: Vec<PathBuf> = Some(std::ffi::OsString::from(
        crate::vcs::git::scm::launch_env::gui_search_path(),
    ))
    .into_iter()
    .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
    .filter(|p| !p.as_os_str().is_empty())
    .map(|p| p.join(kind.binary_name()))
    .collect();
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let home = PathBuf::from(home);
        for dir in kind.install_dirs().iter().copied().chain([
            ".local/bin",
            ".bun/bin",
            ".npm-global/bin",
            ".cargo/bin",
            "n/bin",
        ]) {
            candidates.push(home.join(dir).join(kind.binary_name()));
        }
        if kind == Kind::Claude {
            candidates.push(home.join(".claude/local/claude"));
            let dir = home.join(".local/share/claude/versions");
            let mut versions: Vec<_> = std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .collect();
            versions.sort_by_key(|p| {
                p.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .split('.')
                    .map(|s| s.parse::<u32>().unwrap_or(0))
                    .collect::<Vec<_>>()
            });
            candidates.extend(versions.into_iter().rev());
        }
        #[cfg(windows)]
        candidates.push(home.join("AppData/Roaming/npm").join(kind.binary_name()));
    }
    for dir in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/snap/bin",
    ] {
        candidates.push(Path::new(dir).join(kind.binary_name()));
    }
    if matches!(kind, Kind::Cursor | Kind::Pi) {
        let alias = if kind == Kind::Cursor {
            "agent"
        } else {
            "pi-coding-agent"
        };
        let paths: Vec<_> = candidates
            .iter()
            .filter_map(|p| p.parent().map(|dir| dir.join(alias)))
            .collect();
        candidates.extend(paths);
    }
    candidates.into_iter().find_map(|p| {
        #[cfg(windows)]
        for ext in ["exe", "cmd", "bat", "com"] {
            let candidate = p.with_extension(ext);
            if executable(&candidate) && discovery::matches(kind, &candidate) {
                return Some(candidate);
            }
        }
        (executable(&p) && discovery::matches(kind, &p)).then_some(p)
    })
}
fn executable(path: &Path) -> bool {
    let Ok(meta) = path.metadata() else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    meta.is_file()
}

/// On resume, Claude Code replays a background-task notification from the
/// earlier session and answers it with an empty result before the real turn.
fn claude_turn_result(value: &Value) -> bool {
    value["type"] == "result"
        && !(value["origin"]["kind"] == "task-notification" && value["num_turns"] == 0)
}

fn write_json(out: &mut impl Write, value: Value) -> Result<(), String> {
    let mut frame = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
    frame.push(b'\n');
    out.write_all(&frame)
        .and_then(|_| out.flush())
        .map_err(|e| e.to_string())
}

/// State machine is deliberately testable without spawning a provider.
struct Protocol {
    initialized: bool,
    compact: bool,
    command_probe: bool,
    compact_confirmed: bool,
    active_turn: Option<String>,
    previous_turn: Option<String>,
    interrupt_id: Option<String>,
    claude_prompt_sent: bool,
    claude_result_seen: bool,
    claude_usage_limit: Option<crate::session::agent::Limit>,
    claude_resume_at: Option<Instant>,
    follow_up_ready: bool,
    steering: HashMap<String, (crate::session::follow_up::Message, Instant)>,
    request: Request,
    emitted: HashMap<String, String>,
    claude_message: usize,
    begun: bool,
    done: bool,
    tools: HashMap<String, ToolCall>,
    active_compaction: Option<String>,
    activities: tasks::Activities,
    /// Streaming tool id and its partial input JSON, by content block index.
    claude_tools: HashMap<u64, (String, String)>,
    text_key: Option<String>,
    approvals: HashMap<String, (Value, String, Value)>,
    metrics: TurnMetrics,
    usage_total: u64,
    acp: acp::State,
    pi: pi::State,
    probe: bool,
    catalog_result: Option<Value>,
    questions: HashMap<String, (Value, String, Value, crate::session::agent::Question)>,
    question_order: std::collections::VecDeque<String>,
    visible_question: Option<String>,
    async_question_items: std::collections::HashSet<String>,
    async_answers: HashMap<String, (Value, String, Value, crate::session::agent::Question)>,
}
impl Protocol {}

/// Stream at bounded memory cost. `emit` implements backpressure and must observe
/// cancellation while waiting for the viewport. Errors are returned after reaping.
pub fn run(
    request: Request,
    cancel: Arc<AtomicBool>,
    emit: impl FnMut(Vec<Event>) -> Result<(), String>,
) -> Result<(), String> {
    run_with_controls(request, cancel, None, emit)
}
pub fn run_with_controls(
    request: Request,
    cancel: Arc<AtomicBool>,
    controls: Option<mpsc::Receiver<Control>>,
    emit: impl FnMut(Vec<Event>) -> Result<(), String>,
) -> Result<(), String> {
    run_process(request, cancel, controls, false, emit).map(|_| ())
}

pub(crate) fn catalog_for(
    kind: Kind,
    account: crate::session::profile::Profile,
) -> Result<Value, String> {
    let binary = resolve(kind).ok_or_else(|| kind.unavailable_hint())?;
    let cwd = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir().map_err(|e| e.to_string())?);
    run_process(
        Request {
            kind,
            binary,
            cwd,
            prompt: String::new(),
            provider_id: None,
            model: None,
            settings: Settings {
                account,
                access: AccessMode::ReadOnly,
                ..Default::default()
            },
        },
        Arc::new(AtomicBool::new(false)),
        None,
        true,
        |_| Ok(()),
    )?
    .ok_or_else(|| format!("{} returned no model catalog", kind.label()))
}

fn run_process(
    request: Request,
    cancel: Arc<AtomicBool>,
    controls: Option<mpsc::Receiver<Control>>,
    probe: bool,
    emit: impl FnMut(Vec<Event>) -> Result<(), String>,
) -> Result<Option<Value>, String> {
    run_operation(request, cancel, controls, probe, false, false, emit)
}

#[cfg(test)]
mod tests;
use operation::run_operation;
// Unix-only text tests reach it through `use super::*`.
#[cfg(all(test, unix))]
use operation::run_text;
