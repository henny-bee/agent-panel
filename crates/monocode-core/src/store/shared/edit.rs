use super::*;
use crate::store::EditSnapshot;

/// Whether `seen`, the rows on screen, is what `blocks` holds. A step streamed
/// live lacks the display details the store adds when saving it, so rows that
/// would save without changing anything count as the same.
fn same_rows(blocks: &[Value], seen: &[Row]) -> Result<bool, String> {
    let saved = blocks::project(blocks)?;
    Ok(saved == seen || blocks::project(&blocks::merge(blocks, 0, seen)?)? == saved)
}

impl Store {
    pub(in crate::store) fn shared_validate_edit_rows(
        &self,
        id: i64,
        expected: &[Value],
        rows: &[Row],
    ) -> Result<(), String> {
        let current = self.shared_blocks(id)?;
        if current != expected || !same_rows(&current, rows)? {
            return Err("Conversation metadata changed; its history was preserved".into());
        }
        Ok(())
    }
    pub(in crate::store) fn shared_edit_queue_empty(&self, id: i64) -> Result<(), String> {
        // Validation must not refresh the optimistic queue/history revision:
        // a failed edit leaves the visible revision old and still guarded.
        let field = self.shared_field("queued_messages_json", "'[]'");
        let raw: String = sql(self.conn.query_row(
            &format!("SELECT {field} FROM sessions WHERE id=?1"),
            [self.persisted_session_id(id)?],
            |row| row.get(0),
        ))?;
        let queued: Vec<Value> = parse(&raw)?;
        if !queued.is_empty()
            || self
                .shared_blocks(id)?
                .iter()
                .any(|block| block["draft"] == true)
        {
            return Err(
                "Clear queued messages and saved drafts before forking, rewinding or editing a message".into(),
            );
        }
        Ok(())
    }
    pub(in crate::store) fn shared_prepare_edit(
        &self,
        id: i64,
        expected: &[Row],
        at: usize,
    ) -> Result<EditSnapshot, String> {
        self.shared_writable()?;
        self.edit_queue_empty(id)?;
        let session = self
            .sessions()?
            .into_iter()
            .find(|s| s.id == id)
            .ok_or("Conversation no longer exists")?;
        if !crate::provider::supports_rewind(crate::provider::Kind::from_id(&session.provider)) {
            return Err("This provider does not support rewinding".into());
        }
        let blocks = self.shared_blocks(id)?;
        if self
            .shared_all_blocks(id)?
            .iter()
            .any(|b| b["role"] == "handoff")
        {
            return Err("Editing a turn across a provider handoff is unavailable".into());
        }
        if !same_rows(&blocks, expected)? {
            return Err("Conversation changed; reopen it before editing".into());
        }
        // The rows the app shows, which the app compares against later.
        let rows = expected.to_vec();
        // The user block whose first projected row is `at`.
        let mut offset = 0;
        let mut latest = None;
        for (index, block) in blocks.iter().enumerate() {
            if offset == at
                && block["role"] == "user"
                && block["internal"] != true
                && block["draft"] != true
            {
                latest = Some(index);
                break;
            }
            offset += blocks::project(std::slice::from_ref(block))?.len();
        }
        let latest = latest.ok_or("No user message to edit")?;
        let block = &blocks[latest];
        if ["secondOpinion", "noteCard", "ciContext"]
            .iter()
            .any(|field| !block[*field].is_null())
        {
            return Err("This contextual user message cannot be edited".into());
        }
        let settings = self.agent_settings(id)?.unwrap_or_default();
        let provider_turn_id = block["providerTurnId"]
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .map(str::to_owned)
            .or_else(|| super::super::edit::turn_id_after(&rows, &settings, at));
        let mut first = latest;
        if session.provider == "codex" {
            let exact = provider_turn_id
                .as_deref()
                .ok_or("Codex did not expose an exact user turn to edit")?;
            for index in (0..latest).rev() {
                let prior = &blocks[index];
                if prior["role"] == "user" {
                    if prior["providerTurnId"].as_str() != Some(exact) {
                        break;
                    }
                    if prior["internal"] != true && prior["draft"] != true {
                        first = index;
                    }
                }
            }
            if block["providerTurnId"].is_null() {
                if let Some(row_start) =
                    super::super::edit::codex_turn_start(&rows, &settings, exact)
                {
                    let mut offset = 0;
                    for (index, candidate) in blocks.iter().enumerate().take(latest + 1) {
                        if offset == row_start
                            && candidate["role"] == "user"
                            && candidate["internal"] != true
                        {
                            first = index;
                            break;
                        }
                        offset += blocks::project(std::slice::from_ref(candidate))?.len();
                    }
                } else if blocks[..latest].iter().any(|block| {
                    block["role"] == "user" && block["internal"] != true && block["draft"] != true
                }) {
                    return Err(
                        "Codex user-turn boundaries are unavailable; history was preserved".into(),
                    );
                }
            }
        }
        let start = blocks::project(&blocks[..first])?.len();
        let mut prompt = block["text"].as_str().unwrap_or("").to_owned();
        if block["monocode"] == true {
            prompt = format!("/operator {prompt}");
        }
        let attachments = block["attachments"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|a| {
                a["path"]
                    .as_str()
                    .map(PathBuf::from)
                    .ok_or("An original attachment cannot be restored safely".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        if attachments.is_empty() && super::super::edit::untracked_attachment_tail(&prompt) {
            return Err("This message has attachment links without a saved attachment manifest and cannot be edited safely".into());
        }
        if prompt.trim().is_empty() && attachments.is_empty() {
            return Err("No editable prompt text or attachments".into());
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
            blocks: Some(blocks),
            block_start: first,
            provider: session.provider,
        })
    }
    pub(in crate::store) fn shared_finish_edit(
        &self,
        id: i64,
        edit: &EditSnapshot,
        provider_id: Option<&str>,
    ) -> Result<(), String> {
        self.shared_writable()?;
        let tx = sql(self.conn.unchecked_transaction())?;
        self.validate_edit(id, edit)?;
        let blocks = edit
            .blocks
            .as_ref()
            .ok_or("Missing source revision for edit")?;
        let next = &blocks[..edit.block_start];
        self.shared_store_blocks(id, next)?;
        self.shared_patch(
            id,
            &[
                ("provider_session_id", json!(provider_id)),
                ("updated_at", json!(crate::sys::time::now_ms())),
                (
                    "has_user_message",
                    json!(next.iter().any(|b| b["role"] == "user"
                        && b["internal"] != true
                        && b["draft"] != true)),
                ),
            ],
        )?;
        if let Some(mut settings) = self.agent_settings(id)? {
            settings.turns.retain(|&row, _| row < edit.start);
            // The validated prefix already retains its original rich turn
            // receipts. Save only the trimmed sidecar/preferences in this tx.
            self.shared_save_agent_preferences(id, &settings)?;
        }
        self.save_context_usage(id, &Default::default())?;
        sql(tx.commit())?;
        self.shared
            .as_ref()
            .unwrap()
            .snapshots
            .borrow_mut()
            .insert(id, HistoryRevision::of(next));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn last_user(rows: &[Row]) -> usize {
        rows.iter()
            .rposition(|r| r.first && r.role == "user")
            .unwrap()
    }

    #[test]
    fn a_live_turn_whose_step_returned_images_can_be_edited() {
        use crate::session::{
            tool::{ToolCall, ToolState},
            transcript::Transcript,
        };
        let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
        let id = store.create("claude", "/project").unwrap().id;
        store.provider_id(id, "session").unwrap();
        let mut live = Transcript::default();
        live.begin("user");
        live.append("what images are here");
        let mut read = ToolCall::new("read-1", "Read");
        read.input = json!({"file_path":"/a.png"}).to_string();
        read.state = ToolState::Completed;
        read.images = vec!["/cache/read-1-0.png".into()];
        live.begin("tool");
        live.append(&serde_json::to_string(&read).unwrap());
        live.begin("assistant");
        live.append("One picture.");
        store.write_rows(id, 0, &live.rows).unwrap();
        // Before, the saved step's extra display details made this refuse with
        // "Conversation changed" until the chat was reopened.
        let edit = store.prepare_edit(id, &live.rows, 0).unwrap();
        assert!(edit.matches(&live.rows));
        store.finish_edit(id, &edit, Some("session")).unwrap();
        assert!(store.load(id).unwrap().rows.is_empty());
    }
    fn fixture(blocks: Value) -> (Store, i64) {
        let store = Store::open_monocode(Path::new(":memory:")).unwrap();
        let session = store.create("codex", "/project").unwrap();
        store.provider_id(session.id, "thread").unwrap();
        store
            .shared_store_blocks(session.id, blocks.as_array().unwrap())
            .unwrap();
        (store, session.id)
    }
    #[test]
    fn rich_prefix_and_hidden_metadata_survive_grouped_codex_turn_replacement() {
        let blocks = json!([
            {"id":"initial","role":"user","text":"Earlier","providerTurnId":"old","future":{"keep":true},
             "startedAt":100,"durationMs":5000,"turnModel":{"harness":"codex","id":"codex:old-model","name":"Earlier Model"}},
            {"id":"answer","role":"assistant","text":"Kept","citations":["source"]},
            {"id":"hidden","role":"system","internal":true,"text":"Prior metadata","future":7},
            {"id":"original","role":"user","text":"Original turn","providerTurnId":"turn"},
            {"id":"work","role":"assistant","text":"Working"},
            {"id":"steer","role":"user","text":"Edited follow-up","providerTurnId":"turn","attachments":[{"id":"image","path":"/tmp/image.png","future":true}]},
            {"id":"replaced","role":"assistant","text":"Old answer"}
        ]);
        let (mut store, id) = fixture(blocks.clone());
        let rows = store.load(id).unwrap().rows;
        let edit = store.prepare_edit(id, &rows, last_user(&rows)).unwrap();
        assert_eq!(edit.start, 2);
        assert_eq!(edit.prompt, "Edited follow-up");
        assert_eq!(edit.attachments, vec![PathBuf::from("/tmp/image.png")]);
        assert_eq!(
            store.shared_blocks(id).unwrap(),
            blocks.as_array().unwrap().to_vec()
        );
        store.finish_edit(id, &edit, Some("thread")).unwrap();
        assert_eq!(
            store.shared_blocks(id).unwrap(),
            blocks.as_array().unwrap()[..3]
        );
        assert_eq!(store.load(id).unwrap().rows, rows[..2]);
        let retained = store.agent_settings(id).unwrap().unwrap();
        assert_eq!(retained.turns.len(), 1);
        assert_eq!(retained.turns[&1].model, "Earlier Model");
        assert_eq!(retained.turns[&1].duration_ms, 5000);
    }
    #[test]
    fn hidden_metadata_changes_during_provider_rewind_preserve_every_source_block() {
        let (mut store, id) = fixture(json!([
            {"id":"user","role":"user","text":"Prompt","providerTurnId":"turn"},
            {"id":"answer","role":"assistant","text":"Answer","future":"first"}
        ]));
        let rows = store.load(id).unwrap().rows;
        let edit = store.prepare_edit(id, &rows, last_user(&rows)).unwrap();
        let mut newer = store.shared_blocks(id).unwrap();
        newer[1]["future"] = json!("another window");
        store.shared_store_blocks(id, &newer).unwrap();
        assert!(store.finish_edit(id, &edit, Some("fork")).is_err());
        assert_eq!(store.shared_blocks(id).unwrap(), newer);
        assert_eq!(
            store.sessions().unwrap()[0].provider_id.as_deref(),
            Some("thread")
        );
    }
    #[test]
    fn saved_drafts_queues_handoffs_and_missing_provider_ids_do_not_allow_rewind() {
        for blocks in [
            json!([{"role":"user","id":"u","text":"Prompt"}]),
            json!([{"role":"handoff","id":"h","text":"Handoff"},{"role":"user","id":"u","text":"Prompt","providerTurnId":"turn"}]),
            json!([{"role":"user","id":"u","text":"Prompt","providerTurnId":"turn"},{"role":"user","id":"draft","draft":true,"text":"Saved"}]),
            json!([{"role":"user","id":"u","text":"Prompt","providerTurnId":"turn","noteCard":{"title":"Context"}}]),
        ] {
            let (store, id) = fixture(blocks.clone());
            let rows = store.load(id).unwrap().rows;
            assert!(store.prepare_edit(id, &rows, last_user(&rows)).is_err());
            assert_eq!(
                store.shared_blocks(id).unwrap(),
                blocks.as_array().unwrap().to_vec()
            );
        }
    }
    #[test]
    fn a_queue_added_after_preparation_is_never_removed_by_confirmed_rewind() {
        let (mut store, id) =
            fixture(json!([{"role":"user","id":"u","text":"Prompt","providerTurnId":"turn"}]));
        let rows = store.load(id).unwrap().rows;
        let edit = store.prepare_edit(id, &rows, last_user(&rows)).unwrap();
        let queue = crate::session::follow_up::State {
            revision: 1,
            messages: vec![crate::session::follow_up::Message {
                id: "queued".into(),
                text: "New queued prompt".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        store.save_follow_ups(id, &queue).unwrap();
        assert!(store.finish_edit(id, &edit, Some("fork")).is_err());
        assert_eq!(store.load(id).unwrap().rows, rows);
        assert_eq!(
            store.follow_ups(id).unwrap().messages[0].text,
            "New queued prompt"
        );
    }
}
