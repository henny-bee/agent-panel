use super::*;

impl Store {
    pub(crate) fn shared_follow_ups(
        &self,
        id: i64,
    ) -> Result<crate::session::follow_up::State, String> {
        let fields = format!(
            "{},{}",
            self.shared_field("queued_messages_json", "'[]'"),
            self.shared_field("queue_status", "NULL")
        );
        let (raw, status) = sql(self.conn.query_row(
            &format!("SELECT {fields} FROM sessions WHERE id=?1"),
            [self.persisted_session_id(id)?],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
        ))?;
        let mut state: crate::session::follow_up::State = self
            .shared_setting(&format!("follow-up:{id}"))?
            .map(|raw| parse(&raw))
            .transpose()?
            .unwrap_or_default();
        self.shared
            .as_ref()
            .unwrap()
            .queues
            .borrow_mut()
            .insert(id, raw.clone());
        state.messages = parse::<Vec<Value>>(&raw)?.iter().map(follow_up).collect();
        let blocks = self.shared_blocks(id)?;
        state.draft = None;
        if let Some((at, block)) = blocks.iter().enumerate().find(|(_, b)| b["draft"] == true) {
            let mut draft = follow_up(block);
            draft.at = blocks::project(&blocks[..at])?.len();
            state.draft = Some(draft);
        }
        state.paused = status.as_deref() == Some("paused") || !state.messages.is_empty();
        Ok(state)
    }

    pub(crate) fn shared_save_follow_ups(
        &self,
        id: i64,
        state: &crate::session::follow_up::State,
    ) -> Result<(), String> {
        self.shared_writable()?;
        let previous: crate::session::follow_up::State = self
            .shared_setting(&format!("follow-up:{id}"))?
            .map(|raw| parse(&raw))
            .transpose()?
            .unwrap_or_default();
        if previous.revision > state.revision {
            return Ok(());
        }
        let tx = sql(self.conn.unchecked_transaction())?;
        let fields = self.shared_field("queued_messages_json", "'[]'");
        let raw: String = sql(self.conn.query_row(
            &format!("SELECT {fields} FROM sessions WHERE id=?1"),
            [self.persisted_session_id(id)?],
            |r| r.get(0),
        ))?;
        if self
            .shared
            .as_ref()
            .unwrap()
            .queues
            .borrow()
            .get(&id)
            .is_some_and(|expected| expected != &raw)
        {
            return Err("The message queue changed in another window. Reopen it before saving; its data was preserved.".into());
        }
        let old: Vec<Value> = parse(&raw)?;
        let messages = state
            .messages
            .iter()
            .map(|m| {
                let mut value = old
                    .iter()
                    .find(|v| v["id"].as_str() == Some(&m.id))
                    .cloned()
                    .unwrap_or(json!({}));
                update_follow_up(&mut value, m);
                value
            })
            .collect::<Vec<_>>();
        self.shared_patch(
            id,
            &[
                ("queued_messages_json", json!(encode(&messages)?)),
                (
                    "queue_status",
                    json!(if state.paused { "paused" } else { "active" }),
                ),
            ],
        )?;
        let mut blocks = self.shared_blocks(id)?;
        if self
            .shared
            .as_ref()
            .unwrap()
            .snapshots
            .borrow()
            .get(&id)
            .is_some_and(|old| !old.matches(&blocks))
        {
            return Err("This conversation changed in another window. Reopen it before saving; its data was preserved.".into());
        }

        let old_draft = blocks.iter().find(|b| b["draft"] == true).cloned();
        blocks.retain(|b| b["draft"] != true);
        if let Some(draft) = &state.draft {
            let mut value = old_draft
                .filter(|v| v["id"].as_str() == Some(&draft.id))
                .unwrap_or(json!({"id":draft.id,"role":"user","draft":true}));
            update_follow_up(&mut value, draft);
            let mut at = blocks.len();
            let mut row = 0;
            for (ix, block) in blocks.iter().enumerate() {
                if row >= draft.at {
                    at = ix;
                    break;
                }
                row += blocks::project(std::slice::from_ref(block))?.len();
            }
            blocks.insert(at, value);
        }
        self.shared_store_blocks(id, &blocks)?;
        sql(tx.commit())?;
        self.shared
            .as_ref()
            .unwrap()
            .snapshots
            .borrow_mut()
            .insert(id, HistoryRevision::of(&blocks));
        self.shared
            .as_ref()
            .unwrap()
            .queues
            .borrow_mut()
            .insert(id, encode(&messages)?);
        self.shared_save_setting(&format!("follow-up:{id}"), &encode(state)?)
    }
}
