//! Session storage. Desktop launches use Monocode's existing schema directly.
mod app_access;
mod contributions;
mod managed_sessions;
mod monocode_import;
mod monos;
mod notes;
mod remote;
mod remote_legacy;
pub use remote::{RemoteProject, RemoteSession, RemoteUpload};
pub(crate) use shared::handle as desktop_session_handle;
mod session_metadata;
pub use session_metadata::{MobileSessionMetadata, SessionSidebarMetadata};
mod session_preview;
pub use session_preview::SessionPreviewData;
pub mod desktop_preferences;
mod edit;
mod reminders;
pub mod search;
mod sessions;
mod shared;
mod side_threads;
mod thread_files;
use crate::session::transcript::{Row, Transcript};
pub use edit::EditSnapshot;
pub use monocode_import::native_model;
pub use monocode_import::{monocode_database, ImportReport};
use rusqlite::{params, Connection};
pub use shared::{HistoryPage, PAGE_TURNS};
pub use side_threads::DesktopSideThread;
use std::path::Path;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Session {
    pub id: i64,
    pub title: String,
    pub provider: String,
    pub cwd: String,
    pub provider_id: Option<String>,
    pub has_messages: bool,
    /// Unix milliseconds; zero means an older session has no recorded time.
    pub updated_at: u64,
    pub archived: bool,
    pub pinned: bool,
    pub work_item: Option<crate::session::groups::WorkItem>,
}

/// A session's pending or delivered reminder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reminder {
    pub session_id: i64,
    /// Unix milliseconds.
    pub due_at: u64,
    pub fired_at: Option<u64>,
}

pub struct Store {
    conn: Connection,
    path: std::path::PathBuf,
    shared: Option<shared::Shared>,
}
impl Store {
    pub fn provider_accounts(&self) -> Result<crate::provider::accounts::Book, String> {
        if self.shared.is_some() {
            return self.shared_accounts();
        }

        use rusqlite::OptionalExtension;
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM native_settings WHERE key='provider-accounts'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        value
            .map(|v| {
                serde_json::from_str(&v).map_err(|e| format!("Cannot read provider accounts: {e}"))
            })
            .unwrap_or_else(|| Ok(Default::default()))
    }
    pub fn save_provider_accounts(
        &self,
        book: &crate::provider::accounts::Book,
    ) -> Result<(), String> {
        if self.shared.is_some() {
            return self.shared_save_accounts(book);
        }

        let value = serde_json::to_string(book).map_err(|e| e.to_string())?;
        self.conn.execute("INSERT INTO native_settings(key,value) VALUES('provider-accounts',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [value]).map_err(|e| e.to_string())?;
        Ok(())
    }
    /// Legacy prototype fixtures. Desktop launches use open_monocode, including
    /// isolated development launches, so production history needs no conversion.
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
          CREATE TABLE IF NOT EXISTS native_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS native_sessions(id INTEGER PRIMARY KEY, title TEXT NOT NULL,
            provider TEXT NOT NULL, cwd TEXT NOT NULL, provider_id TEXT,
            updated_at INTEGER NOT NULL DEFAULT 0, archived INTEGER NOT NULL DEFAULT 0);
          CREATE TABLE IF NOT EXISTS native_rows(session_id INTEGER NOT NULL REFERENCES native_sessions(id),
            row_ix INTEGER NOT NULL, role TEXT NOT NULL, body TEXT NOT NULL, first INTEGER NOT NULL,
            PRIMARY KEY(session_id,row_ix));
          CREATE TABLE IF NOT EXISTS native_reminders(session_id INTEGER PRIMARY KEY REFERENCES native_sessions(id),
            due_at INTEGER NOT NULL, fired_at INTEGER);").map_err(|e| e.to_string())?;
        let columns = conn
            .prepare("PRAGMA table_info(native_sessions)")
            .map_err(|e| e.to_string())?
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for (column, kind) in [
            ("updated_at", "INTEGER NOT NULL DEFAULT 0"),
            ("archived", "INTEGER NOT NULL DEFAULT 0"),
            ("pinned", "INTEGER NOT NULL DEFAULT 0"),
            ("work_item", "TEXT"),
        ] {
            if !columns.iter().any(|name| name == column) {
                conn.execute(
                    &format!("ALTER TABLE native_sessions ADD COLUMN {column} {kind}"),
                    [],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        let has_sent_at = conn
            .prepare("PRAGMA table_info(native_rows)")
            .map_err(|e| e.to_string())?
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
            .iter()
            .any(|name| name == "sent_at");
        if !has_sent_at {
            conn.execute("ALTER TABLE native_rows ADD COLUMN sent_at INTEGER", [])
                .map_err(|e| e.to_string())?;
        }
        // Older native histories have completion times in their turn receipts.
        // Preserve unknown ages instead of stamping all old conversations now.
        let recorded = conn.prepare("SELECT s.id,a.value FROM native_sessions s JOIN native_settings a ON a.key='agent:'||s.id WHERE s.updated_at=0")
            .map_err(|e| e.to_string())?
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        for (id, settings) in recorded {
            if let Ok(settings) = serde_json::from_str::<crate::session::agent::Settings>(&settings)
            {
                if let Some(time) = settings
                    .turns
                    .values()
                    .filter_map(|turn| turn.completed_at)
                    .max()
                {
                    conn.execute(
                        "UPDATE native_sessions SET updated_at=?2 WHERE id=?1",
                        params![id, time.min(i64::MAX as u64) as i64],
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
        }
        Ok(Self {
            conn,
            path: path.into(),
            shared: None,
        })
    }
    /// The selected desktop or isolated-development database.
    pub fn database_path(&self) -> &Path {
        &self.path
    }
    pub fn data_dir(&self) -> Option<&Path> {
        (self.path != Path::new(":memory:"))
            .then(|| self.path.parent())
            .flatten()
    }
    pub fn setting(&self, key: &str) -> Result<Option<String>, String> {
        if self.shared.is_some() {
            return self.shared_setting(key);
        }

        use rusqlite::OptionalExtension;
        self.conn
            .query_row(
                "SELECT value FROM native_settings WHERE key=?1",
                [key],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())
    }
    pub fn save_setting(&self, key: &str, value: &str) -> Result<(), String> {
        if self.shared.is_some() {
            return self.shared_save_setting(key, value);
        }

        self.conn.execute("INSERT INTO native_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,value])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn ui_state(&self) -> Result<Option<String>, String> {
        if self.shared.is_some() {
            return self.shared_ui_state();
        }

        use rusqlite::OptionalExtension;
        self.conn
            .query_row(
                "SELECT value FROM native_settings WHERE key='workspace'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())
    }
    pub fn agent_settings(
        &self,
        id: i64,
    ) -> Result<Option<crate::session::agent::Settings>, String> {
        let remote = id < 0 || self.remote_session(id)?.is_some();
        if remote {
            if let Some(raw) = self.setting(&format!("remote-agent:{id}"))? {
                return serde_json::from_str(&raw)
                    .map(Some)
                    .map_err(|e| format!("Cannot read remote settings: {e}"));
            }
            if id < 0 || self.shared.is_none() {
                return Ok(None);
            }
        }
        if self.shared.is_some() {
            let mut settings = self.shared_agent(id)?;
            if remote {
                if let Some(settings) = settings.as_mut() {
                    if settings.worktree_cwd.is_none() {
                        settings.worktree_cwd = self.legacy_remote_worktree(id)?;
                    }
                }
            }
            return Ok(settings);
        }

        use rusqlite::OptionalExtension;
        let text: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM native_settings WHERE key=?1",
                [format!("agent:{id}")],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        text.map(|text| serde_json::from_str(&text).map_err(|e| e.to_string()))
            .transpose()
    }
    /// Read workspace membership without loading every session's full agent settings.
    pub fn session_worktrees(
        &self,
    ) -> Result<std::collections::HashMap<i64, Option<String>>, String> {
        if self.shared.is_some() {
            return self.shared_worktrees();
        }

        self.conn
            .prepare("SELECT s.id, CASE WHEN json_valid(a.value) THEN CASE WHEN json_type(a.value, '$.worktree_cwd')='text' THEN json_extract(a.value, '$.worktree_cwd') END END FROM native_sessions s LEFT JOIN native_settings a ON a.key='agent:'||s.id")
            .map_err(|e| e.to_string())?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())
    }
    pub fn context_usage(&self, id: i64) -> Result<Option<crate::session::context::Usage>, String> {
        if self.shared.is_some() {
            return self.shared_context(id);
        }

        use rusqlite::OptionalExtension;
        let text: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM native_settings WHERE key=?1",
                [format!("context:{id}")],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        text.map(|text| serde_json::from_str(&text).map_err(|e| e.to_string()))
            .transpose()
    }
    pub fn save_context_usage(
        &self,
        id: i64,
        usage: &crate::session::context::Usage,
    ) -> Result<(), String> {
        if self.shared.is_some() {
            return self.shared_patch(
                id,
                &[
                    ("context_used", serde_json::json!(usage.used)),
                    ("context_window", serde_json::json!(usage.window)),
                ],
            );
        }

        self.conn.execute("INSERT INTO native_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![format!("context:{id}"), serde_json::to_string(usage).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn save_note(&self, note: &crate::session::response::Note) -> Result<(), String> {
        self.save_response_note(note)
    }
    pub fn notes(&self) -> Result<Vec<crate::session::response::Note>, String> {
        Ok(self
            .list_notes()?
            .into_iter()
            .map(|note| {
                let source_session = self
                    .note_source_session(&note)
                    .or_else(|| {
                        self.shared
                            .is_none()
                            .then(|| note.source_session_id.as_deref()?.parse().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                crate::session::response::Note {
                    id: note
                        .id
                        .strip_prefix("gpui-note-")
                        .and_then(|id| id.parse().ok())
                        .unwrap_or_else(|| shared::handle(&note.id) as u64),
                    title: note.title,
                    body: note.body,
                    tags: note.tags,
                    source_session,
                    source_cwd: note.source_cwd.unwrap_or_default(),
                }
            })
            .collect())
    }
    /// Read-only fallback for early native settings; list_notes owns merging it
    /// with the desktop table, so this must never call the notes facade.
    fn legacy_notes(&self) -> Result<Vec<crate::session::response::Note>, String> {
        let mut query = self
            .conn
            .prepare("SELECT value FROM native_settings WHERE key LIKE 'note:%' ORDER BY key DESC")
            .map_err(|e| e.to_string())?;
        let rows = query
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|row| {
            serde_json::from_str(&row.map_err(|e| e.to_string())?).map_err(|e| e.to_string())
        })
        .collect()
    }
    pub fn save_agent_settings(
        &self,
        id: i64,
        settings: &crate::session::agent::Settings,
    ) -> Result<(), String> {
        if id < 0 || self.remote_session(id)?.is_some() {
            return self.save_setting(
                &format!("remote-agent:{id}"),
                &serde_json::to_string(settings).map_err(|e| e.to_string())?,
            );
        }
        if self.shared.is_some() {
            return self.shared_save_agent(id, settings);
        }

        self.conn.execute("INSERT INTO native_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![format!("agent:{id}"), serde_json::to_string(settings).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn configure_agent(
        &self,
        id: i64,
        provider: &str,
        settings: &crate::session::agent::Settings,
    ) -> Result<(), String> {
        if self
            .agent_settings(id)?
            .is_some_and(|saved| saved.revision > settings.revision)
        {
            return Ok(());
        }
        self.change_provider(id, provider)?;
        self.save_agent_settings(id, settings)
    }
    pub fn change_provider(&self, id: i64, provider: &str) -> Result<(), String> {
        if let Some(mut remote) = self.remote_session(id)? {
            if remote.summary.provider != provider {
                // The host fixes the harness at creation, as React's remote view does.
                if remote.session_id.is_some() {
                    return Err("Start a new remote conversation to change provider".into());
                }
                remote.summary.provider = provider.into();
                self.save_remote_session(&remote)?;
            }
            return Ok(());
        }
        if self.shared.is_some() {
            return self.shared_change_provider(id, provider);
        }

        self.conn.execute("UPDATE native_sessions SET provider=?2,provider_id=NULL WHERE id=?1 AND provider<>?2", params![id,provider]).map_err(|e|e.to_string())?;
        Ok(())
    }
    pub fn save_ui_state(&self, state: &str) -> Result<(), String> {
        if self.shared.is_some() {
            return self.shared_save_setting("workspace", state);
        }

        self.conn.execute("INSERT INTO native_settings(key,value) VALUES('workspace',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[state]).map_err(|e|e.to_string())?;
        Ok(())
    }
    /// Each session another one started through the app CLI, and its caller.
    pub fn started_links(&self) -> Result<std::collections::HashMap<i64, i64>, String> {
        let mut links = if self.shared.is_some() {
            self.shared_started_links()?
        } else {
            std::collections::HashMap::new()
        };
        let saved: std::collections::HashMap<_, _> = self
            .settings_with_prefix("started-by:")?
            .into_iter()
            .map(|(child, value)| Ok((child, value.trim().parse().map_err(|_| "Invalid caller")?)))
            .collect::<Result<_, String>>()?;
        links.extend(saved);
        Ok(links)
    }
    pub fn save_started_link(&self, child: i64, caller: i64) -> Result<(), String> {
        self.save_setting(&format!("started-by:{child}"), &caller.to_string())
    }
    /// Every lead's last orchestration summary.
    pub fn orchestration_summaries(
        &self,
    ) -> Result<std::collections::HashMap<i64, crate::session::orchestration::Summary>, String>
    {
        Ok(self
            .settings_with_prefix("orchestration:")?
            .into_iter()
            .filter_map(|(lead, value)| Some((lead, serde_json::from_str(&value).ok()?)))
            .collect())
    }
    pub fn save_orchestration_summary(
        &self,
        lead: i64,
        summary: &crate::session::orchestration::Summary,
    ) -> Result<(), String> {
        let value = serde_json::to_string(summary).map_err(|e| e.to_string())?;
        self.save_setting(&format!("orchestration:{lead}"), &value)
    }
    /// `prefix<session id>` settings, by session id.
    fn settings_with_prefix(&self, prefix: &str) -> Result<Vec<(i64, String)>, String> {
        if self.shared.is_some() {
            return self.shared_settings_with_prefix(prefix);
        }

        let mut stmt = self
            .conn
            .prepare("SELECT key, value FROM native_settings WHERE key LIKE ?1 || '%'")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([prefix], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            let (key, value) = row.map_err(|e| e.to_string())?;
            if let Some(id) = key.strip_prefix(prefix).and_then(|id| id.parse().ok()) {
                out.push((id, value));
            }
        }
        Ok(out)
    }
    /// A project's folders and folded groups.
    pub fn groups(&self, cwd: &str) -> Result<crate::session::groups::Groups, String> {
        if self.shared.is_some() {
            return self.shared_groups(cwd);
        }

        use rusqlite::OptionalExtension;
        let text: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM native_settings WHERE key=?1",
                [format!("groups:{cwd}")],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(text
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default())
    }
    pub fn save_groups(
        &self,
        cwd: &str,
        groups: &crate::session::groups::Groups,
    ) -> Result<(), String> {
        if self.shared.is_some() {
            return self.shared_save_setting(&format!("groups:{cwd}"), &shared::encode(groups)?);
        }

        self.conn.execute("INSERT INTO native_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![format!("groups:{cwd}"), serde_json::to_string(groups).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
