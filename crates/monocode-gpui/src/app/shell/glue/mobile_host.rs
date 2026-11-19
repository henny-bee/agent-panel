//! A paired phone browses and drives this workspace. The core host pairs and
//! serves devices; the shell answers their requests from its own state, through
//! the same paths as the sidebar, composer and Changes panel.
mod catalog;
mod requests;
mod setup;
mod watches;
use super::super::*;
use crate::{
    accounts::Tone,
    chat::composer::pickers,
    mobile::{catalog as mobile_catalog, project, transcript as turns},
    notifications::delivery,
};
use catalog::catalog_model;
use gpui::{EntityId, Task};
use monocode_core::{
    integrations::mobile::{
        Event, Invite, MobileHost, Push, Request, Status, WatchStart, notify, watch,
    },
    session::{
        agent::{AccessMode, Settings, TurnInfo},
        transcript::Row,
    },
};
use requests::saved_snapshot;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

/// How often the workspace a phone polls is refreshed while access is on.
const PUBLISH_INTERVAL: Duration = Duration::from_secs(3);
/// At most this often while a reply streams: about 20 frames a second.
const PUSH_INTERVAL: Duration = Duration::from_millis(50);
/// Turns a conversation opens on when the phone does not say.
const DEFAULT_TAIL: usize = 3;
const MAX_PAGE: usize = 100;
const READ_TURNS: usize = 30;
/// Under the host's 25 second wait; the phone asks again with the same ID.
const GIT_ACTION_WAIT: Duration = Duration::from_secs(20);
const MAX_RECEIPTS: usize = 1000;
const MAX_GIT_JOBS: usize = 100;
const MAX_ATTACHMENTS: usize = 10;
const MAX_MESSAGE: usize = 100_000;

#[derive(Default)]
pub(crate) struct Mobile {
    host: Option<MobileHost>,
    /// The visible pairing code, until it expires, is used or is closed.
    pub(crate) invite: Option<Invite>,
    /// Commands by device-scoped ID, so a phone's retry never runs one twice.
    receipts: HashMap<String, Receipt>,
    receipt_order: VecDeque<String>,
    /// Git actions can outlast a request; the phone asks again until done.
    git_jobs: HashMap<String, GitJob>,
    git_order: VecDeque<String>,
    watches: HashMap<String, Watch>,
    push_scheduled: bool,
    publishing: bool,
    /// Persisted session IDs, which stay stable for paired phones.
    ids: HashMap<i64, String>,
    /// Sessions whose current turn a phone started: their approvals,
    /// questions and finish are sent to it.
    started: HashSet<i64>,
    /// The last message of each Mono conversation not open here, by its
    /// session, with the update time it was read at: read again only once
    /// the conversation has changed.
    mono_previews: HashMap<i64, MonoPreview>,
}

#[derive(Clone)]
struct MonoPreview {
    updated_at: u64,
    text: Value,
}

struct CatalogEntry {
    id: i64,
    project: String,
    summary: Session,
    live: Option<mobile_catalog::LiveSession>,
}

enum Receipt {
    Pending(String),
    Done(String, Result<Value, String>),
}
struct GitJob {
    signature: String,
    result: Arc<Mutex<Option<Result<Value, String>>>>,
}

#[derive(Clone, PartialEq)]
enum Target {
    Session(i64),
    Mono(String),
}

/// A conversation read from disk, reused until the session opens on desktop.
struct Saved {
    id: i64,
    settings: Settings,
    rows: Vec<Row>,
    harness: String,
    cwd: String,
    /// The store cursor for turns above `rows`: a long Mono loads only its
    /// newest page.
    history: Option<i64>,
    /// The Mono's name, so its saved turns are drawn as Mono turns, as they
    /// are once its chat opens, and keep their heights when it does.
    mono: Option<String>,
    /// The cards of the sessions the Mono started, as they stood on reading.
    launches: HashMap<i64, Value>,
}

/// How an older page's turns are drawn, as the conversation's own are.
struct PageLook {
    mono: Option<String>,
    launches: HashMap<i64, Value>,
    cwd: String,
    harness: String,
    model: Option<String>,
}

/// A read's answer, or the store page it still needs.
enum Read {
    Done(Value),
    /// The turns before the loaded rows, from the page at this cursor.
    Page(i64),
}

struct Watch {
    target: Target,
    tail: usize,
    /// The first turn the watch carries; the window grows from here.
    from: Option<String>,
    incremental: bool,
    baseline: Option<watch::Baseline>,
    saved: Option<Arc<Saved>>,
    loading: bool,
    observed: Option<EntityId>,
    _observers: Vec<Subscription>,
}

/// What a session's turns are built from, live or saved.
struct Snapshot<'a> {
    usage_limit: Value,
    rows: &'a [Row],
    turns: &'a HashMap<usize, TurnInfo>,
    cwd: &'a str,
    harness: &'a str,
    model: Option<String>,
    busy: bool,
    waiting: Option<&'static str>,
    mono: Option<String>,
    pending: Vec<turns::Pending>,
    /// A Mono's launched sessions, as the phone's cards show them.
    launches: &'a HashMap<i64, Value>,
    /// The question the agent waits on, as the phone's card shows it; null
    /// for none, as always for a saved conversation.
    question: Value,
    /// The store cursor for turns above `rows`, which the phone pages in
    /// through `conversation.older`.
    history: Option<i64>,
}
impl Snapshot<'_> {
    fn source(&self) -> turns::Source<'_> {
        turns::Source {
            rows: self.rows,
            turns: self.turns,
            cwd: self.cwd,
            harness: self.harness,
            model_name: self.model.as_deref().map(|model| {
                pickers::cached_model_name(Kind::from_id(self.harness), Some(model)).to_string()
            }),
            busy: self.busy,
            waiting: self.waiting,
            mono: self.mono.as_deref(),
            pending: &self.pending,
            launches: self.launches,
        }
    }
}

/// `encodeURIComponent`, as paired phones already know projects by it.
/// A visual document's component tree, as its first render draws it.
fn visual_tree(body: &str) -> Result<Value, String> {
    use monocode_core::session::visual::{VisualDocument, VisualRuntime};
    let visual: VisualDocument = serde_json::from_str(body).map_err(|e| e.to_string())?;
    visual.validate()?;
    let tree = VisualRuntime::new(&visual.source)?.render()?;
    serde_json::to_value(tree).map_err(|e| e.to_string())
}

fn project_id(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
fn hex(color: u32) -> String {
    format!("#{:06x}", color & 0xff_ffff)
}
fn string(params: &Value, key: &str) -> Option<String> {
    params[key].as_str().map(str::to_owned)
}

impl Shell {
    // MARK: catalog

    // MARK: requests

    // MARK: watches

    // MARK: commands

    // MARK: notifications

    // MARK: models and accounts
}

fn tone_id(tone: Tone) -> &'static str {
    match tone {
        Tone::Ready => "ready",
        Tone::Low => "low",
        Tone::Exhausted => "exhausted",
        Tone::Checking => "checking",
        _ => "unknown",
    }
}

/// An older page's turn ID, `p{cursor}t{row}`: its page's store cursor and
/// the row its turn starts at.
fn page_turn(id: &str) -> Option<(i64, usize)> {
    let (cursor, row) = id.strip_prefix('p')?.split_once('t')?;
    Some((cursor.parse().ok()?, row.parse().ok()?))
}

fn message_text(params: &Value) -> Result<String, String> {
    let text = params["text"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_owned();
    let attached = params["attachments"]
        .as_array()
        .is_some_and(|a| !a.is_empty());
    if (text.is_empty() && !attached) || text.len() > MAX_MESSAGE {
        return Err("Enter a message under 100,000 characters".into());
    }
    Ok(text)
}

/// A model sheet's settings, keeping only well-formed ones.
fn model_settings(params: &Value) -> HashMap<String, String> {
    params["modelSettings"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| {
            let value = value.as_str()?;
            (key.len() <= 64 && value.len() <= 64).then(|| (key.clone(), value.to_owned()))
        })
        .collect()
}

fn mobile_settings_of(settings: &Settings) -> HashMap<String, String> {
    let mut values = settings.model_settings.clone();
    if let Some(effort) = &settings.effort {
        values.insert("effort".into(), effort.clone());
    }
    values.insert("fast".into(), settings.fast.to_string());
    if let Some(thinking) = settings.thinking {
        values.insert("thinking".into(), thinking.to_string());
    }
    values
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// The model a provider picked by the phone starts with: project, then Settings.
fn explicit_model(
    prefs: &crate::ui::preferences::Preferences,
    cwd: &str,
    kind: Kind,
) -> Option<String> {
    glue::new_chat_choice::NewChatChoice::explicit(prefs, cwd, kind)
        .model
        .and_then(|model| monocode_core::store::native_model(&model, kind.id()))
}

/// The model a phone-started session gets: the phone's own pick, else the
/// project then Settings model of an explicit provider, else the new-chat default.
fn start_model(
    prefs: &crate::ui::preferences::Preferences,
    cwd: &str,
    chosen: Option<Kind>,
    requested: Option<String>,
    default_model: Option<String>,
) -> Option<String> {
    match (chosen, requested) {
        (Some(kind), Some(model)) if model.len() <= 256 => {
            monocode_core::store::native_model(&model, kind.id())
        }
        (Some(kind), _) => explicit_model(prefs, cwd, kind),
        _ => default_model,
    }
}

/// Providers the phone's model sheet leaves out. Remote projects list them all,
/// as the desktop model picker does.
fn hidden_providers(prefs: &crate::ui::preferences::Preferences, cwd: &str) -> Vec<String> {
    if monocode_core::files::remote::parse_remote_path(cwd).is_some() {
        return Vec::new();
    }
    prefs.hidden_for(cwd)
}

#[cfg(test)]
mod tests;
