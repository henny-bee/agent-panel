use monocode_core::session::agent::{Approval, Control, Settings};
use monocode_core::{
    provider::{self, Event, Kind, Request},
    session::transcript::{Row, Transcript},
    store::{Session, Store},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
pub(crate) mod contributions;
pub(crate) mod fs_watch;
pub(crate) mod git_cache;
pub(crate) mod git_status;
mod handoff;
pub(crate) mod pane_layout;
mod persistence;
pub(crate) mod remote;
mod turn;
mod visual_discovery;
use persistence::PendingSave;
pub(crate) type Database = Arc<Mutex<Store>>;
struct TurnInput {
    summary: Session,
    prompt: String,
    settings: Settings,
}
pub(crate) struct Update {
    pub(crate) worktree_cwd: Option<String>,
    pub(crate) commands: Option<Vec<provider::NativeCommand>>,
    pub(crate) provider_turn_id: Option<String>,
    pub(crate) turn_ready: bool,
    pub(crate) follow_up_delivered: Vec<String>,
    pub(crate) follow_up_errors: Vec<(String, String)>,
    pub(crate) delivered: Option<String>,
    pub(crate) start: usize,
    pub(crate) rows: Vec<Row>,
    pub(crate) provider_id: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) config_model: Option<String>,
    pub(crate) config_settings: Option<std::collections::HashMap<String, String>>,
    /// A title the host changed since the last update; never written back.
    pub(crate) title: Option<String>,
    pub(crate) context: Option<monocode_core::session::context::Usage>,
    pub(crate) metrics: Option<monocode_core::session::agent::TurnMetrics>,
    pub(crate) usage_limit: Option<monocode_core::session::agent::Limit>,
    /// The tasks a yielded turn still waits on, when that changed.
    pub(crate) background: Option<Vec<provider::BackgroundTask>>,
    pub(crate) approvals: Vec<Approval>,
    pub(crate) approvals_resolved: Vec<String>,
    pub(crate) questions: Vec<monocode_core::session::agent::Question>,
    pub(crate) questions_resolved: Vec<String>,
    pub(crate) handoff: Option<Option<monocode_core::session::agent::Handoff>>,
    pub(crate) status: String,
    pub(crate) finished: bool,
}
fn publish(
    tx: &async_channel::Sender<Update>,
    cancel: &AtomicBool,
    mut update: Update,
) -> Result<(), String> {
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Stopped".into());
        }
        match tx.try_send(update) {
            Ok(()) => return Ok(()),
            Err(async_channel::TrySendError::Closed(_)) => return Err("Window closed".into()),
            Err(async_channel::TrySendError::Full(value)) => update = value,
        }
        std::thread::sleep(Duration::from_millis(4));
    }
}
pub(crate) fn locked(store: &Database) -> Result<std::sync::MutexGuard<'_, Store>, String> {
    store
        .lock()
        .map_err(|_| "Native session store unavailable".into())
}

pub(crate) fn run_turn(
    summary: Session,
    prompt: String,
    settings: Settings,
    controls: std::sync::mpsc::Receiver<Control>,
    store: Database,
    cancel: Arc<AtomicBool>,
    tx: async_channel::Sender<Update>,
) {
    if summary.cwd.starts_with("remote://") {
        remote::run(summary, prompt, settings, controls, store, cancel, tx);
        return;
    }
    run_turn_using(
        TurnInput {
            summary,
            prompt,
            settings,
        },
        controls,
        store,
        cancel,
        tx,
        provider::resolve,
    );
}

/// checkpointEdits.ts's sessionEditPaths: files named by successful edit tools.
pub(crate) fn edited_paths(rows: &[Row]) -> Vec<String> {
    let mut paths = Vec::new();
    for row in rows.iter().filter(|r| r.role == "tool") {
        let Ok(tool) = serde_json::from_str::<monocode_core::session::tool::ToolCall>(&row.text)
        else {
            continue;
        };
        if tool.state != monocode_core::session::tool::ToolState::Completed {
            continue;
        }
        let shown = tool.presentation();
        if matches!(
            shown.action.as_str(),
            "Edit" | "Write" | "MultiEdit" | "NotebookEdit"
        ) {
            let input = serde_json::from_str::<serde_json::Value>(&tool.input).unwrap_or_default();
            let many = input["paths"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|path| path.as_str());
            let desktop = tool.display.as_ref().unwrap_or(&input["__desktop"]);
            let desktop_paths = desktop["paths"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|path| path.as_str());
            for path in shown
                .file_path
                .as_deref()
                .into_iter()
                .chain(many)
                .chain(desktop_paths)
            {
                if !path.is_empty() && !paths.iter().any(|p| p == path) {
                    paths.push(path.to_owned());
                }
            }
        }
    }
    paths
}

fn shlex_quote(text: &str) -> String {
    format!(
        "\"{}\"",
        text.replace('\\', "\\\\")
            .replace('\"', "\\\"")
            .replace('$', "\\$")
            .replace('`', "\\`")
    )
}

use turn::run_turn_using;
#[cfg(test)]
mod tests;
