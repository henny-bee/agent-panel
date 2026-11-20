//! Two-phase cut at a user message: capture a revision, then trim or copy it
//! only after provider confirmation. Rewind, Edit and Fork all start here.
use super::*;
use crate::provider::Kind;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct EditSnapshot {
    pub start: usize,
    pub prompt: String,
    pub attachments: Vec<PathBuf>,
    pub provider_turn_id: Option<String>,
    pub provider_id: String,
    pub account_id: String,
    pub cwd: String,
    pub rows: Vec<Row>,
    pub(super) blocks: Option<Vec<Value>>,
    pub(super) block_start: usize,
    pub(super) provider: String,
}
impl EditSnapshot {
    pub fn provider(&self) -> &str {
        &self.provider
    }
    pub fn matches(&self, rows: &[Row]) -> bool {
        self.rows == rows
    }
    /// User messages after the cut, which the provider's copy must lose too.
    pub fn later(&self) -> usize {
        self.rows[self.start + 1..]
            .iter()
            .filter(|row| row.first && row.role == "user")
            .count()
    }
    /// Whether no user message precedes the cut, so no provider history survives it.
    pub fn first(&self) -> bool {
        !self.rows[..self.start]
            .iter()
            .any(|row| row.first && row.role == "user")
    }
    /// When the message the cut lands on was sent, if the history recorded it.
    pub fn sent_at(&self) -> Option<u64> {
        self.rows[self.start].sent_at
    }
}

/// `"Plan"` -> `"Plan (fork)"`, or `"Plan (fork 2)"`, `"Plan (fork 3)"`, ...
/// when the earlier titles are taken.
fn fork_title(title: &str, taken: &[&str]) -> String {
    let title = title.trim();
    std::iter::once(format!("{title} (fork)"))
        .chain((2..).map(|n| format!("{title} (fork {n})")))
        .find(|candidate| !taken.contains(&candidate.as_str()))
        .unwrap_or_default()
}

/// The provider turn id recorded on the reply to the prompt at row `at`.
pub(super) fn turn_id_after(
    rows: &[Row],
    settings: &crate::session::agent::Settings,
    at: usize,
) -> Option<String> {
    let end = (at + 1..rows.len())
        .find(|&row| rows[row].first && rows[row].role == "user")
        .unwrap_or(rows.len());
    settings
        .turns
        .iter()
        .filter(|(row, info)| (at + 1..end).contains(*row) && info.provider_turn_id.is_some())
        .min_by_key(|(row, _)| **row)
        .and_then(|(_, info)| info.provider_turn_id.clone())
}

pub(super) fn codex_turn_start(
    rows: &[Row],
    settings: &crate::session::agent::Settings,
    id: &str,
) -> Option<usize> {
    let response = settings
        .turns
        .iter()
        .filter(|(_, info)| info.provider_turn_id.as_deref() == Some(id))
        .map(|(&row, _)| row)
        .min()?;
    rows.iter()
        .enumerate()
        .take(response)
        .rev()
        .find(|(_, row)| row.role == "user" && row.first)
        .map(|(row, _)| row)
}

/// Native history previously rendered files as trailing local Markdown links,
/// without retaining the attachment payload. Re-sending that text alone would
/// silently drop images. Refuse ambiguous local-file tails until a manifest is
/// available; rich desktop attachment lists are restored separately.
pub(super) fn untracked_attachment_tail(prompt: &str) -> bool {
    let Some(tail) = prompt.rsplit("\n\n").next() else {
        return false;
    };
    let Some(path) = tail
        .strip_prefix('[')
        .and_then(|tail| tail.split_once("](<"))
        .and_then(|(_, path)| path.strip_suffix(">)"))
    else {
        return false;
    };
    Path::new(path).is_absolute()
        || path.starts_with('/')
        || path.starts_with("\\\\")
        || path.as_bytes().get(1) == Some(&b':')
}

const EDIT_RECOVERY: &str = "A previous rewind may have changed the provider's context before local history was saved. Choose Cancel edit to continue here anyway, or start a new conversation.";

impl Store {
    /// Durable guard for provider context that diverged after a confirmed edit.
    /// Guards saved with older wording read as today's, which names the way out.
    pub fn edit_recovery_reason(&self, id: i64) -> Result<Option<String>, String> {
        Ok(self
            .setting(&format!("edit-recovery:{id}"))?
            .filter(|reason| !reason.is_empty())
            .map(|_| EDIT_RECOVERY.to_owned()))
    }
    pub fn mark_edit_recovery_required(&self, id: i64) -> Result<(), String> {
        self.save_setting(&format!("edit-recovery:{id}"), EDIT_RECOVERY)
    }
    /// Lifts the guard. The user chose to continue with the provider's context.
    pub fn clear_edit_recovery(&self, id: i64) -> Result<(), String> {
        self.save_setting(&format!("edit-recovery:{id}"), "")
    }
    /// Captures the conversation for a cut just before the user message that
    /// starts at row `at`. `expected` is the visible transcript; never recall an
    /// unloaded revision.
    pub fn prepare_edit(
        &self,
        id: i64,
        expected: &[Row],
        at: usize,
    ) -> Result<EditSnapshot, String> {
        if let Some(reason) = self.edit_recovery_reason(id)? {
            return Err(reason);
        }
        if self.shared.is_some() {
            return self.shared_prepare_edit(id, expected, at);
        }
        let session = self
            .sessions()?
            .into_iter()
            .find(|s| s.id == id)
            .ok_or("Conversation no longer exists")?;
        if !crate::provider::supports_rewind(Kind::from_id(&session.provider)) {
            return Err("This provider does not support rewinding".into());
        }
        let rows = self.load(id)?.rows;
        if rows != expected {
            return Err("Conversation changed; reopen it before editing".into());
        }
        self.edit_queue_empty(id)?;
        if !rows.get(at).is_some_and(|r| r.role == "user" && r.first) {
            return Err("No user message to edit".into());
        }
        let end = (at + 1..rows.len())
            .find(|&i| rows[i].first)
            .unwrap_or(rows.len());
        let settings = self.agent_settings(id)?.unwrap_or_default();
        let provider_turn_id = turn_id_after(&rows, &settings, at);
        let start = if session.provider == "codex" {
            let turn = provider_turn_id
                .as_deref()
                .ok_or("Codex did not expose an exact user turn to edit")?;
            codex_turn_start(&rows, &settings, turn)
                .or_else(|| {
                    (!rows[..at].iter().any(|row| row.role == "user" && row.first)).then_some(at)
                })
                .ok_or("Codex user-turn boundaries are unavailable; history was preserved")?
        } else {
            at
        };
        let prompt: String = rows[at..end].iter().map(|r| r.text.as_str()).collect();
        let attachments = rows
            .get(end)
            .map(crate::session::transcript::attachments::paths)
            .transpose()?
            .unwrap_or_default();
        if attachments.is_empty() && untracked_attachment_tail(&prompt) {
            return Err("This message has attachment links without a saved attachment manifest and cannot be edited safely".into());
        }
        Ok(EditSnapshot {
            start,
            prompt,
            attachments,
            provider_turn_id,
            provider_id: session.provider_id.ok_or("No provider session to edit")?,
            account_id: settings.account.id,
            cwd: settings.worktree_cwd.unwrap_or(session.cwd),
            rows,
            blocks: None,
            block_start: 0,
            provider: session.provider,
        })
    }
    pub(super) fn edit_queue_empty(&self, id: i64) -> Result<(), String> {
        if self.shared.is_some() {
            return self.shared_edit_queue_empty(id);
        }
        let queue = self.follow_ups(id)?;
        if !queue.messages.is_empty() || queue.draft.is_some() {
            return Err(
                "Clear queued messages and saved drafts before editing the last message".into(),
            );
        }
        Ok(())
    }
    /// Checks again immediately before the external RPC, without retaining the
    /// database mutex while waiting on provider I/O.
    pub fn validate_edit(&self, id: i64, edit: &EditSnapshot) -> Result<(), String> {
        let session = self
            .sessions()?
            .into_iter()
            .find(|s| s.id == id)
            .ok_or("Conversation no longer exists")?;
        if session.provider != edit.provider
            || session.provider_id.as_deref() != Some(edit.provider_id.as_str())
        {
            return Err(
                "Provider session changed; cancel the edit and reopen this conversation".into(),
            );
        }
        let settings = self.agent_settings(id)?.unwrap_or_default();
        if settings.account.id != edit.account_id
            || settings.worktree_cwd.as_deref().unwrap_or(&session.cwd) != edit.cwd
        {
            return Err(
                "The provider account or working directory changed; cancel the edit before sending"
                    .into(),
            );
        }
        if let Some(expected) = &edit.blocks {
            self.shared_validate_edit_rows(id, expected, &edit.rows)?;
        } else if self.load(id)?.rows != edit.rows {
            return Err("Conversation changed; its history was preserved".into());
        }
        self.edit_queue_empty(id)?;
        Ok(())
    }
    /// Call only after `rewind_to` confirms, with the binding it returned. This
    /// is a compare-and-swap, preserving an independently changed conversation
    /// instead of deleting it.
    pub fn finish_edit(
        &mut self,
        id: i64,
        edit: &EditSnapshot,
        provider_id: Option<&str>,
    ) -> Result<(), String> {
        // A provider that rewound in place is ahead of local history until this
        // write lands, so a failed write or crash must leave a durable guard. A
        // cut copy leaves the old binding valid and needs none.
        let in_place = provider_id == Some(edit.provider_id.as_str());
        if in_place {
            self.mark_edit_recovery_required(id)?;
        }
        if self.shared.is_some() {
            self.shared_finish_edit(id, edit, provider_id)?;
        } else {
            let start = i64::try_from(edit.start)
                .map_err(|_| "Conversation row index exceeds SQLite's integer range".to_owned())?;
            let updated_at = crate::sys::time::now_ms().min(i64::MAX as u64) as i64;
            let tx = self
                .conn
                .unchecked_transaction()
                .map_err(|e| e.to_string())?;
            self.validate_edit(id, edit)?;
            self.conn
                .execute(
                    "DELETE FROM native_rows WHERE session_id=?1 AND row_ix>=?2",
                    params![id, start],
                )
                .map_err(|e| e.to_string())?;
            self.conn
                .execute(
                    "UPDATE native_sessions SET provider_id=?2,updated_at=?3 WHERE id=?1",
                    params![id, provider_id, updated_at],
                )
                .map_err(|e| e.to_string())?;
            if let Some(mut settings) = self.agent_settings(id)? {
                settings.turns.retain(|&row, _| row < edit.start);
                self.save_agent_settings(id, &settings)?;
            }
            self.save_context_usage(id, &Default::default())?;
            tx.commit().map_err(|e| e.to_string())?;
        }
        if in_place {
            self.clear_edit_recovery(id)?;
        }
        Ok(())
    }
    /// Copies the conversation before `edit` into a new one bound to
    /// `provider_id` and titled after the source, which is left as it was.
    pub fn fork_edit(
        &mut self,
        id: i64,
        edit: &EditSnapshot,
        provider_id: Option<&str>,
    ) -> Result<Session, String> {
        self.validate_edit(id, edit)?;
        let sessions = self.sessions()?;
        let source = sessions
            .iter()
            .find(|s| s.id == id)
            .ok_or("Conversation no longer exists")?;
        let taken: Vec<&str> = sessions.iter().map(|s| s.title.as_str()).collect();
        let title = fork_title(&source.title, &taken);
        let mut settings = self.agent_settings(id)?.unwrap_or_default();
        settings.turns.retain(|&row, _| row < edit.start);
        let mut fork = self.create(&source.provider, &source.cwd)?;
        self.title(fork.id, &title)?;
        if let Some(blocks) = &edit.blocks {
            // Rich turn receipts travel with the copied blocks.
            self.shared_store_blocks(fork.id, &blocks[..edit.block_start])?;
            self.shared_save_agent_preferences(fork.id, &settings)?;
        } else {
            self.write_rows(fork.id, 0, &edit.rows[..edit.start])?;
            self.save_agent_settings(fork.id, &settings)?;
        }
        if let Some(binding) = provider_id {
            self.provider_id(fork.id, binding)?;
        }
        fork.title = title;
        fork.provider_id = provider_id.map(str::to_owned);
        fork.has_messages = edit.start > 0;
        Ok(fork)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Store, i64, Vec<Row>) {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        let session = store.create("codex", "/project").unwrap();
        store.provider_id(session.id, "thread").unwrap();
        let mut transcript = Transcript::default();
        for (role, text) in [
            ("user", "Earlier"),
            ("assistant", "Kept"),
            ("user", "Edit me"),
            ("assistant", "Replaced"),
        ] {
            transcript.begin(role);
            transcript.append(text);
        }
        store.write_rows(session.id, 0, &transcript.rows).unwrap();
        let mut settings = crate::session::agent::Settings::default();
        settings.turns.insert(
            3,
            crate::session::agent::TurnInfo {
                provider_turn_id: Some("turn".into()),
                ..Default::default()
            },
        );
        store.save_agent_settings(session.id, &settings).unwrap();
        (store, session.id, transcript.rows)
    }
    #[test]
    fn preparing_and_cancelling_an_edit_leaves_all_rows_intact() {
        let (store, id, rows) = fixture();
        let edit = store.prepare_edit(id, &rows, 2).unwrap();
        assert_eq!(edit.start, 2);
        assert_eq!(edit.prompt, "Edit me");
        assert_eq!(store.load(id).unwrap().rows, rows);
    }
    #[test]
    fn confirmed_edit_truncates_exactly_from_its_message_and_rebinds_forks() {
        let (mut store, id, rows) = fixture();
        let edit = store.prepare_edit(id, &rows, 2).unwrap();
        store.finish_edit(id, &edit, Some("fork")).unwrap();
        assert_eq!(store.load(id).unwrap().rows, rows[..2]);
        assert_eq!(
            store.sessions().unwrap()[0].provider_id.as_deref(),
            Some("fork")
        );
    }
    #[test]
    fn a_cut_at_the_first_message_empties_the_chat_and_drops_the_binding() {
        let (mut store, id, rows) = fixture();
        let mut settings = store.agent_settings(id).unwrap().unwrap();
        settings.turns.insert(
            1,
            crate::session::agent::TurnInfo {
                provider_turn_id: Some("first".into()),
                ..Default::default()
            },
        );
        store.save_agent_settings(id, &settings).unwrap();
        let edit = store.prepare_edit(id, &rows, 0).unwrap();
        assert_eq!(edit.prompt, "Earlier");
        assert_eq!(edit.provider_turn_id.as_deref(), Some("first"));
        assert_eq!((edit.start, edit.later(), edit.first()), (0, 1, true));
        store.finish_edit(id, &edit, None).unwrap();
        assert!(store.load(id).unwrap().rows.is_empty());
        assert_eq!(store.sessions().unwrap()[0].provider_id, None);
    }
    #[test]
    fn a_fork_copies_the_history_before_the_message_and_leaves_the_source() {
        let (mut store, id, rows) = fixture();
        store.title(id, "Plan").unwrap();
        let edit = store.prepare_edit(id, &rows, 2).unwrap();
        let fork = store.fork_edit(id, &edit, Some("copy")).unwrap();
        assert_eq!(fork.title, "Plan (fork)");
        assert_eq!(fork.provider_id.as_deref(), Some("copy"));
        assert_eq!(store.load(fork.id).unwrap().rows, rows[..2]);
        assert_eq!(store.load(id).unwrap().rows, rows);
        assert_eq!(fork_title("Plan", &["Plan (fork)"]), "Plan (fork 2)");
        assert_eq!(
            fork_title("Plan ", &["Plan (fork)", "Plan (fork 2)"]),
            "Plan (fork 3)"
        );
    }
    #[test]
    fn late_rows_or_provider_binding_changes_are_never_overwritten() {
        let (mut store, id, rows) = fixture();
        let edit = store.prepare_edit(id, &rows, 2).unwrap();
        store.provider_id(id, "another-thread").unwrap();
        assert!(store.finish_edit(id, &edit, Some("fork")).is_err());
        assert_eq!(store.load(id).unwrap().rows, rows);
        store.provider_id(id, "thread").unwrap();
        let mut changed = rows.clone();
        changed[3].text = "Another window".into();
        store.write_rows(id, 3, &changed[3..]).unwrap();
        assert!(store.finish_edit(id, &edit, Some("fork")).is_err());
        assert_eq!(store.load(id).unwrap().rows, changed);
    }
    #[test]
    fn codex_steered_messages_recall_only_latest_prompt_but_replace_the_complete_provider_turn() {
        let (mut store, id, mut rows) = fixture();
        let mut transcript = Transcript { rows };
        transcript.begin("user");
        transcript.append("Latest steer");
        transcript.begin("assistant");
        transcript.append("After steer");
        rows = transcript.rows;
        store.write_rows(id, 4, &rows[4..]).unwrap();
        let mut settings = store.agent_settings(id).unwrap().unwrap();
        settings.turns.insert(
            5,
            crate::session::agent::TurnInfo {
                provider_turn_id: Some("turn".into()),
                ..Default::default()
            },
        );
        store.save_agent_settings(id, &settings).unwrap();
        let edit = store.prepare_edit(id, &rows, 4).unwrap();
        assert_eq!(edit.prompt, "Latest steer");
        assert_eq!(edit.start, 2);
        store.finish_edit(id, &edit, Some("thread")).unwrap();
        assert_eq!(store.load(id).unwrap().rows, rows[..2]);
        assert!(store.agent_settings(id).unwrap().unwrap().turns.is_empty());
    }
    #[test]
    fn native_attachment_links_are_never_recalled_as_text_without_the_original_files() {
        for path in [
            "/tmp/photo.png",
            r"C:\Users\me\photo.png",
            "C:/Users/me/photo.png",
            r"\\server\share\photo.png",
        ] {
            let (mut store, id, mut rows) = fixture();
            rows[2].text = format!("Describe image\n\n[photo.png](<{path}>)");
            store.write_rows(id, 2, &rows[2..]).unwrap();
            assert!(store
                .prepare_edit(id, &rows, 2)
                .unwrap_err()
                .contains("attachment manifest"));
            assert_eq!(store.load(id).unwrap().rows, rows);
        }
        for link in [
            "A [web link](https://example.com)",
            "[photo.png](<https://example.com/photo.png>)",
            "[photo.png](<relative/photo.png>)",
        ] {
            assert!(!untracked_attachment_tail(link));
        }
    }
    #[test]
    fn an_in_place_rewind_with_a_conflict_leaves_a_durable_recovery_guard_and_blocks_recall() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("native.db");
        let mut store = Store::open(&path).unwrap();
        let session = store.create("pi", "/project").unwrap();
        store.provider_id(session.id, "session").unwrap();
        let mut transcript = Transcript::default();
        transcript.begin("user");
        transcript.append("Prompt");
        store.write_rows(session.id, 0, &transcript.rows).unwrap();
        let edit = store.prepare_edit(session.id, &transcript.rows, 0).unwrap();
        store.provider_id(session.id, "another-session").unwrap();
        // A copy leaves the old binding valid, so its conflict needs no guard.
        assert!(store.finish_edit(session.id, &edit, Some("fork")).is_err());
        assert!(store.edit_recovery_reason(session.id).unwrap().is_none());
        assert!(store
            .finish_edit(session.id, &edit, Some("session"))
            .is_err());
        assert!(store.edit_recovery_reason(session.id).unwrap().is_some());
        drop(store);
        let reopened = Store::open(&path).unwrap();
        assert!(reopened.edit_recovery_reason(session.id).unwrap().is_some());
        assert!(reopened
            .prepare_edit(session.id, &transcript.rows, 0)
            .is_err());
        assert_eq!(reopened.load(session.id).unwrap().rows, transcript.rows);
    }
}
