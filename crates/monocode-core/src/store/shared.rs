//! A view adapter over the desktop schema: no imported sessions, replacement IDs,
//! persistent native_* tables, or history conversion on open.
mod accounts;
mod agent_settings;
pub(super) mod blocks;
mod edit;
mod follow_ups;
mod reminders;
mod sessions;
mod settings;
#[cfg(test)]
mod tests;
pub(crate) mod turns;
mod window;
use turns::project_turns;
pub use window::{HistoryPage, PAGE_TURNS};

use super::{Reminder, Session, Store};
use crate::session::{
    agent::Settings,
    context::Usage,
    transcript::{Row, Transcript},
};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
};

pub(super) struct Shared {
    columns: HashSet<String>,
    ids: RefCell<HashMap<i64, String>>,
    extras: RefCell<BTreeMap<String, String>>,
    extras_db: Option<Connection>,
    snapshots: RefCell<HashMap<i64, HistoryRevision>>,
    /// The first block sequence of each indexed Mono's row numbering.
    windows: RefCell<HashMap<i64, i64>>,
    queues: RefCell<HashMap<i64, String>>,
    account_source: RefCell<Option<crate::provider::accounts::Book>>,
    preferences: std::sync::Arc<crate::store::desktop_preferences::DesktopPreferences>,
    read_only: bool,
}

/// A revision guard, rather than another owned copy of every rich source block.
/// Keep this small guard even when a view releases its history: pending writes
/// must still refuse a source edit made by another window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HistoryRevision([u8; 32]);
impl HistoryRevision {
    fn of(blocks: &[Value]) -> Self {
        let mut hash = Self::start(blocks.len());
        for block in blocks {
            Self::push(&mut hash, block);
        }
        Self(hash.finalize().into())
    }
    fn start(len: usize) -> Sha256 {
        let mut hash = Sha256::new();
        hash.update((len as u64).to_le_bytes());
        hash
    }
    fn push(hash: &mut Sha256, block: &Value) {
        fn size(hash: &mut Sha256, len: usize) {
            hash.update((len as u64).to_le_bytes());
        }
        fn value(hash: &mut Sha256, item: &Value) {
            match item {
                Value::Null => hash.update([0]),
                Value::Bool(boolean) => hash.update([1, u8::from(*boolean)]),
                Value::Number(number) => {
                    hash.update([2]);
                    // serde_json treats floating negative zero as equal to
                    // positive zero, while retaining its distinction from an integer.
                    let number = if number.is_f64() && number.as_f64() == Some(0.0) {
                        "0.0".into()
                    } else {
                        number.to_string()
                    };
                    size(hash, number.len());
                    hash.update(number.as_bytes());
                }
                Value::String(text) => {
                    hash.update([3]);
                    size(hash, text.len());
                    hash.update(text.as_bytes());
                }
                Value::Array(values) => {
                    hash.update([4]);
                    size(hash, values.len());
                    for item in values {
                        value(hash, item);
                    }
                }
                Value::Object(fields) => {
                    hash.update([5]);
                    size(hash, fields.len());
                    // Be canonical even if another crate enables preserve_order.
                    let mut fields: Vec<_> = fields.iter().collect();
                    fields.sort_unstable_by(|a, b| a.0.cmp(b.0));
                    for (key, item) in fields {
                        size(hash, key.len());
                        hash.update(key.as_bytes());
                        value(hash, item);
                    }
                }
            }
        }
        value(hash, block);
    }
    fn matches(&self, blocks: &[Value]) -> bool {
        *self == Self::of(blocks)
    }
}

fn sql<T>(result: rusqlite::Result<T>) -> Result<T, String> {
    result.map_err(|e| e.to_string())
}
fn parse<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T, String> {
    serde_json::from_str(raw).map_err(|e| e.to_string())
}
/// Desktop app CLI requests name children `app-<caller>-<request>`.
/// Both parts can contain hyphens, so the longest existing caller wins.
fn started_caller<'a>(id: &'a str, ids: &HashSet<String>) -> Option<&'a str> {
    let rest = id.strip_prefix("app-")?;
    rest.match_indices('-')
        .rev()
        .map(|(index, _)| &rest[..index])
        .find(|candidate| ids.contains(*candidate))
}
pub(super) fn encode<T: serde::Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| e.to_string())
}
pub(crate) fn handle(id: &str) -> i64 {
    let bytes = Sha256::digest(id.as_bytes());
    (i64::from_le_bytes(bytes[..8].try_into().unwrap()) & ((1_i64 << 53) - 1)).max(1)
}
pub(super) fn has_table(conn: &Connection, table: &str) -> Result<bool, String> {
    sql(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |r| r.get(0),
    ))
}
/// Clock slack between a prompt's send time and its run's start.
const START_SLACK_MS: u64 = 1_000;
/// A prompt's run start. No run starts before its prompt was sent: an earlier
/// start was copied from another turn and marks a steer, not a new turn.
fn turn_start(block: &Value) -> Option<u64> {
    block["startedAt"].as_u64().filter(|at| {
        block["sentAt"]
            .as_u64()
            .is_none_or(|sent| at.saturating_add(START_SLACK_MS) >= sent)
    })
}
impl Store {
    /// Open the same sessions, notes and reminders as the desktop app. Only a
    /// brand-new file gets a schema; existing schema/data is not migrated.
    pub fn open_monocode(path: &Path) -> Result<Self, String> {
        Self::open_shared(path, false, None)
    }
    pub fn read_monocode(path: &Path) -> Result<Self, String> {
        Self::open_shared(path, true, None)
    }
    #[cfg(test)]
    pub(crate) fn read_monocode_with_preferences_for_test(
        path: &Path,
        preferences: std::sync::Arc<crate::store::desktop_preferences::DesktopPreferences>,
    ) -> Result<Self, String> {
        Self::open_shared(path, true, Some(preferences))
    }
    #[cfg(test)]
    pub(crate) fn open_monocode_with_preferences_for_test(
        path: &Path,
        preferences: std::sync::Arc<crate::store::desktop_preferences::DesktopPreferences>,
    ) -> Result<Self, String> {
        Self::open_shared(path, false, Some(preferences))
    }
    fn open_shared(
        path: &Path,
        read_only: bool,
        preferences: Option<std::sync::Arc<crate::store::desktop_preferences::DesktopPreferences>>,
    ) -> Result<Self, String> {
        let memory = path == Path::new(":memory:");
        let exists = !memory && path.exists();
        if !read_only && !exists {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        let flags = if read_only {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        } else {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
        };
        let conn = sql(Connection::open_with_flags(path, flags))?;
        sql(conn.busy_timeout(std::time::Duration::from_secs(5)))?;
        sql(conn.execute_batch("PRAGMA foreign_keys=ON;"))?;
        if !exists && !read_only {
            sql(conn.execute_batch("PRAGMA journal_mode=WAL;"))?;
            let tx = sql(conn.unchecked_transaction())?;
            sql(tx.execute_batch(include_str!("shared_schema.sql")))?;
            sql(tx.commit())?;
        }
        let columns = sql(sql(conn.prepare("PRAGMA table_info(sessions)"))?
            .query_map([], |r| r.get::<_, String>(1)))?
        .collect::<rusqlite::Result<HashSet<_>>>()
        .map_err(|e| e.to_string())?;
        for required in [
            "id",
            "cwd",
            "harness",
            "model",
            "model_settings",
            "runtime_mode",
            "title",
            "provider_session_id",
            "blocks_json",
            "created_at",
            "updated_at",
        ] {
            if !columns.contains(required) {
                return Err(format!(
                    "Unsupported Monocode database: missing sessions.{required}. No schema changes were made."
                ));
            }
        }
        let (extras, extras_db) = settings::open(path, read_only)?;
        let store = Self {
            conn,
            path: path.into(),
            shared: Some(Shared {
                columns,
                ids: RefCell::new(HashMap::new()),
                extras: RefCell::new(extras),
                extras_db,
                snapshots: RefCell::new(HashMap::new()),
                windows: RefCell::new(HashMap::new()),
                queues: RefCell::new(HashMap::new()),
                account_source: RefCell::new(None),
                preferences: preferences.unwrap_or_else(|| {
                    std::sync::Arc::new(
                        path.parent()
                            .filter(|_| !memory)
                            .map(crate::store::desktop_preferences::DesktopPreferences::load)
                            .unwrap_or_default(),
                    )
                }),
                read_only,
            }),
        };
        // Register only identities here. No blocks_json is read at startup.
        let mut query = sql(store.conn.prepare("SELECT id FROM sessions"))?;
        for id in sql(query.query_map([], |r| r.get::<_, String>(0)))? {
            store.shared_register(&sql(id)?)?;
        }
        drop(query);
        Ok(store)
    }
    pub fn uses_monocode_schema(&self) -> bool {
        self.shared.is_some()
    }
    /// Where native-only tables live: the sidecar beside a desktop database,
    /// which the desktop schema never sees, or a prototype database itself.
    pub(super) fn native_db(&self) -> Option<&Connection> {
        match &self.shared {
            Some(shared) => shared.extras_db.as_ref(),
            None => Some(&self.conn),
        }
    }
    /// A read-only snapshot of desktop preferences, shared by this store's
    /// consumers. Reopening the store refreshes it without per-message WebKit I/O.
    pub fn desktop_preferences(
        &self,
    ) -> Option<&crate::store::desktop_preferences::DesktopPreferences> {
        self.shared
            .as_ref()
            .map(|shared| shared.preferences.as_ref())
    }

    pub(super) fn shared_search_reader(&self) -> Result<Self, String> {
        Self::open_shared(
            &self.path,
            true,
            self.shared
                .as_ref()
                .map(|shared| shared.preferences.clone()),
        )
    }

    /// Resolve only a persisted identity; unlike CLI handle parsing this never
    /// treats a stale numeric-looking desktop ID as an existing session.
    pub fn resolve_existing_session_id(&self, id: &str) -> Option<i64> {
        if self.shared.is_some() {
            let exists = self
                .conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",
                    [id],
                    |r| r.get::<_, bool>(0),
                )
                .ok()?;
            return exists.then(|| self.shared_register(id).ok()).flatten();
        }
        let id = id.parse::<i64>().ok()?;
        self.sessions()
            .ok()?
            .iter()
            .any(|s| s.id == id)
            .then_some(id)
    }
    /// UI handles are stable hashes; the database always retains its original IDs.
    pub fn persisted_session_id(&self, id: i64) -> Result<String, String> {
        if id < 0 {
            if let Some(remote) = self.remote_session(id)? {
                return Ok(format!(
                    "remote:{}:{}",
                    remote.project.environment_id,
                    remote.session_id.unwrap_or_else(|| id.to_string())
                ));
            }
        }
        if let Some(shared) = &self.shared {
            shared
                .ids
                .borrow()
                .get(&id)
                .cloned()
                .ok_or_else(|| "Session no longer exists".into())
        } else {
            Ok(id.to_string())
        }
    }
    pub fn resolve_session_id(&self, id: &str) -> Result<i64, String> {
        if let Some(shared) = &self.shared {
            if let Some((key, _)) = shared
                .ids
                .borrow()
                .iter()
                .find(|(_, value)| value.as_str() == id)
            {
                return Ok(*key);
            }
            if sql(self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",
                [id],
                |r| r.get::<_, bool>(0),
            ))? {
                return self.shared_register(id);
            }
        }
        id.parse::<i64>().map_err(|_| "Unknown session ID".into())
    }
    pub(super) fn shared_register(&self, id: &str) -> Result<i64, String> {
        let key = handle(id);
        let mut ids = self.shared.as_ref().unwrap().ids.borrow_mut();
        if ids.get(&key).is_some_and(|existing| existing != id) {
            return Err("Session identity collision; the database has not been modified".into());
        }
        ids.insert(key, id.into());
        Ok(key)
    }
    pub(super) fn shared_writable(&self) -> Result<(), String> {
        if self.shared.as_ref().unwrap().read_only {
            Err("This Monocode store is read-only".into())
        } else {
            Ok(())
        }
    }
    pub(super) fn shared_field(&self, name: &str, fallback: &str) -> String {
        if self.shared.as_ref().unwrap().columns.contains(name) {
            name.into()
        } else {
            fallback.into()
        }
    }
    pub(super) fn shared_patch(&self, id: i64, fields: &[(&str, Value)]) -> Result<(), String> {
        self.shared_writable()?;
        let id = self.persisted_session_id(id)?;
        let fields: Vec<_> = fields
            .iter()
            .filter(|(name, _)| self.shared.as_ref().unwrap().columns.contains(*name))
            .collect();
        if fields.is_empty() {
            return Ok(());
        }
        let updates = fields
            .iter()
            .enumerate()
            .map(|(ix, (name, _))| format!("{name}=?{}", ix + 2))
            .collect::<Vec<_>>()
            .join(",");
        let mut values = vec![rusqlite::types::Value::Text(id)];
        values.extend(fields.iter().map(|(_, v)| match v {
            Value::Null => rusqlite::types::Value::Null,
            Value::Bool(v) => rusqlite::types::Value::Integer(i64::from(*v)),
            Value::Number(v) => rusqlite::types::Value::Integer(v.as_i64().unwrap_or(i64::MAX)),
            Value::String(v) => rusqlite::types::Value::Text(v.clone()),
            v => rusqlite::types::Value::Text(v.to_string()),
        }));
        sql(self.conn.execute(
            &format!("UPDATE sessions SET {updates} WHERE id=?1"),
            rusqlite::params_from_iter(values),
        ))?;
        Ok(())
    }
    pub(super) fn shared_setting(&self, key: &str) -> Result<Option<String>, String> {
        if let Some((id, message)) = key
            .strip_prefix("mono-delivered:")
            .and_then(|v| v.split_once(':'))
        {
            if let Ok(id) = id.parse::<i64>() {
                if self.shared_delivered(id, message)? {
                    return Ok(Some("1".into()));
                }
            }
        }
        Ok(self
            .shared
            .as_ref()
            .unwrap()
            .extras
            .borrow()
            .get(key)
            .cloned())
    }
    pub(super) fn shared_save_setting(&self, key: &str, value: &str) -> Result<(), String> {
        self.shared_writable()?;
        let shared = self.shared.as_ref().unwrap();
        if shared
            .extras
            .borrow()
            .get(key)
            .is_some_and(|old| old == value)
        {
            return Ok(());
        }
        if let Some(conn) = &shared.extras_db {
            sql(conn.execute(
                "INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, value],
            ))?;
        }
        // Publish only committed values. A failed save must remain retryable.
        shared.extras.borrow_mut().insert(key.into(), value.into());
        Ok(())
    }
    pub(super) fn shared_settings_with_prefix(
        &self,
        prefix: &str,
    ) -> Result<Vec<(i64, String)>, String> {
        Ok(self
            .shared
            .as_ref()
            .unwrap()
            .extras
            .borrow()
            .iter()
            .filter_map(|(key, value)| {
                Some((key.strip_prefix(prefix)?.parse().ok()?, value.clone()))
            })
            .collect())
    }
    pub(super) fn shared_ui_state(&self) -> Result<Option<String>, String> {
        let saved = self.shared_setting("workspace")?;
        let mut workspace = saved.as_deref().map(parse::<Value>).transpose()?;
        if has_table(&self.conn, "workspace_snapshot")? {
            let raw: Option<String> = sql(self
                .conn
                .query_row(
                    "SELECT snapshot_json FROM workspace_snapshot WHERE id=1",
                    [],
                    |r| r.get(0),
                )
                .optional())?;
            if let Some(raw) = raw {
                let snapshot: Value = parse(&raw)?;
                if workspace.is_none() {
                    workspace = Some(
                        crate::store::desktop_preferences::DesktopPreferences::workspace_snapshot(
                            &snapshot,
                            |id| self.resolve_existing_session_id(id),
                        ),
                    );
                } else if let Some(saved) = workspace.as_mut().and_then(Value::as_object_mut) {
                    // Preserve the source snapshot alongside native layouts;
                    // unsupported panes can be recovered without rewriting it.
                    saved
                        .entry("desktop_workspace_snapshot")
                        .or_insert(snapshot);
                }
            }
        }
        let mut merged = workspace.clone().unwrap_or_else(|| json!({}));
        if let Some(preferences) = self.desktop_preferences() {
            preferences.apply_workspace(&mut merged);
        }
        if merged.as_object().is_some_and(|object| !object.is_empty()) {
            Ok(Some(merged.to_string()))
        } else {
            Ok(workspace.map(|value| value.to_string()))
        }
    }
}

fn follow_up(value: &Value) -> crate::session::follow_up::Message {
    crate::session::follow_up::Message {
        id: value["id"].as_str().unwrap_or("").into(),
        text: value["text"].as_str().unwrap_or("").into(),
        prepared_prompt: value["preparedPrompt"].as_str().map(str::to_owned),
        attachments: value["attachments"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| a["path"].as_str())
            .map(PathBuf::from)
            .collect(),
        mode: value["intent"].as_str().map(str::to_owned),
        error: value["error"].as_str().map(str::to_owned),
        ..Default::default()
    }
}
fn update_follow_up(value: &mut Value, message: &crate::session::follow_up::Message) {
    value["id"] = json!(message.id);
    value["text"] = json!(message.text);
    value["preparedPrompt"] = json!(message.prepared_prompt);
    if let Some(mode) = &message.mode {
        value["intent"] = json!(mode);
    }
    if let Some(error) = &message.error {
        value["error"] = json!(error);
    }
    if value["attachments"].is_null() {
        value["attachments"]=json!(message.attachments.iter().map(|p|json!({"id":p.to_string_lossy(),"path":p,"name":p.file_name().unwrap_or_default().to_string_lossy(),
            "mimeType":"application/octet-stream","kind":"file","size":std::fs::metadata(p).map(|m|m.len()).unwrap_or(0)})).collect::<Vec<_>>());
    }
}
