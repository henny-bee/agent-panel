//! Mono data types shared by sessions and the mono service.
use crate::project::appearance::Background;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Mono {
    pub id: String,
    pub session_id: Option<i64>,
    /// Keep a missing desktop association visible instead of silently starting
    /// a replacement conversation when its saved UUID cannot be resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_session: Option<String>,
    pub name: String,
    pub mascot: String,
    pub color: u32,
    pub projects: Vec<String>,
    pub show_started_sessions_in_sidebar: bool,
    pub use_sidebar_folders: bool,
    pub background: Option<Background>,
    pub launches: Vec<Launch>,
    pub documents: Vec<Document>,
    /// References last projected from desktop blocks, to reconcile deletions.
    #[serde(default)]
    pub desktop_document_ids: Vec<String>,
    #[serde(default)]
    pub desktop_launch_ids: Vec<i64>,
    pub outbox: Vec<Message>,
    pub pending_reports: Vec<String>,
    pub habit_sessions: Vec<i64>,
    pub seen_rows: usize,
    /// Minutes of inactivity before a heartbeat; zero turns heartbeats off.
    pub heartbeat_minutes: u32,
    /// The model and effort for sessions it starts without naming any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subsessions: Option<Subsessions>,
}

/// What a Mono's sessions start with when it names no provider, model or
/// effort itself. Each part left unset is inherited from the Mono, as before.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Subsessions {
    pub harness: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Launch {
    pub session_id: i64,
    pub cwd: String,
    pub title: String,
    pub harness: String,
    pub model: Option<String>,
    pub origin_turn: u64,
    pub notify_on_complete: bool,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub dismissed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub summary: String,
    pub body: String,
    pub created_at: u64,
    pub updated_at: u64,
}

pub type Work = (Vec<Document>, Vec<Launch>);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub sent_at: Option<u64>,
    #[serde(default)]
    pub internal: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub attachments: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Roster {
    pub monos: Vec<Mono>,
    pub hidden_sessions: Vec<i64>,
    /// Last observed desktop identities, for reconciling later desktop edits
    /// without discarding deliberate native edits or resurrecting deletions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) desktop_source: Option<Vec<Mono>>,
}
