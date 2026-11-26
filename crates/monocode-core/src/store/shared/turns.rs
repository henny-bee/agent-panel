use super::*;

/// Turn keys are row offsets, including invisible document/card metadata rows.
/// Never substitute the session's current model for missing historical provenance.
pub(in crate::store) fn project_turns(
    blocks: &[Value],
) -> Result<HashMap<usize, crate::session::agent::TurnInfo>, String> {
    let mut turns = HashMap::new();
    let mut offset = 0;
    let mut root_turn = None;
    for block in blocks {
        let rows = blocks::project(std::slice::from_ref(block))?;
        if let Some(prompt) = rows.iter().rposition(Row::is_prompt) {
            let model = &block["turnModel"];
            let duration = block["durationMs"].as_u64();
            let started_at = turn_start(block);
            let turn = offset + prompt + 1;
            if started_at.is_some() {
                root_turn = Some(turn);
            }
            let joined_turn =
                (block["role"] == "user" && block["sentAt"].is_number() && started_at.is_none())
                    .then_some(root_turn)
                    .flatten();
            let metrics = block
                .get("turnMetrics")
                .filter(|value| value.is_object())
                .and_then(|value| serde_json::from_value(value.clone()).ok());
            if model.is_object()
                || duration.is_some()
                || started_at.is_some()
                || metrics.is_some()
                || block["providerTurnId"].is_string()
                || joined_turn.is_some()
            {
                turns.insert(
                    offset + prompt + 1,
                    crate::session::agent::TurnInfo {
                        joined_turn,
                        provider_turn_id: block["providerTurnId"].as_str().map(str::to_owned),
                        provider: model["harness"].as_str().unwrap_or("").into(),
                        model: model["name"].as_str().unwrap_or("").into(),
                        model_id: model["id"].as_str().map(str::to_owned),
                        duration_ms: duration.unwrap_or(0),
                        duration_known: duration.is_some(),
                        started_at,
                        completed_at: started_at
                            .and_then(|at| at.checked_add(duration.unwrap_or(0))),
                        metrics,
                    },
                );
            }
        }
        offset += rows.len();
    }
    Ok(turns)
}

impl Store {
    /// Add native turn receipts to their original desktop prompt objects.
    /// Unknown rich fields and existing provenance are retained verbatim.
    pub(super) fn shared_save_turns(
        &self,
        id: i64,
        turns: &HashMap<usize, crate::session::agent::TurnInfo>,
    ) -> Result<(), String> {
        if turns.is_empty() {
            return Ok(());
        }
        self.shared_writable()?;
        let tx = sql(self.conn.unchecked_transaction())?;
        let mut blocks = self.shared_blocks(id)?;
        if self
            .shared
            .as_ref()
            .unwrap()
            .snapshots
            .borrow()
            .get(&id)
            .is_some_and(|revision| !revision.matches(&blocks))
        {
            return Err("This conversation changed in another window. Reopen it before saving; its data was preserved.".into());
        }
        let (session_provider, session_model): (String, String) = sql(self.conn.query_row(
            "SELECT harness,model FROM sessions WHERE id=?1",
            [self.persisted_session_id(id)?],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ))?;
        let mut offset = 0;
        let mut changed = false;
        for block in &mut blocks {
            let rows = blocks::project(std::slice::from_ref(block))?;
            let turn = rows
                .iter()
                .rposition(Row::is_prompt)
                .map(|prompt| offset + prompt + 1);
            offset += rows.len();
            let Some(info) = turn.and_then(|turn| turns.get(&turn)) else {
                continue;
            };
            let mut fields = Vec::new();
            if !info.provider.is_empty() && (!info.model.is_empty() || info.model_id.is_some()) {
                let mut model = json!({"harness":info.provider});
                if !info.model.is_empty() {
                    model["name"] = json!(info.model);
                }
                if let Some(id) = &info.model_id {
                    model["id"] = json!(if info.provider == session_provider
                        && super::super::monocode_import::native_model(
                            &session_model,
                            &session_provider
                        )
                        .as_ref()
                            == Some(id)
                    {
                        session_model.clone()
                    } else if id.starts_with(&format!("{}:", info.provider)) {
                        id.clone()
                    } else {
                        format!("{}:{id}", info.provider)
                    });
                }
                fields.push(("turnModel", model));
            }
            if info.has_duration() {
                fields.push(("durationMs", json!(info.duration_ms)));
            }
            if let Some(id) = &info.provider_turn_id {
                fields.push(("providerTurnId", json!(id)));
            }
            if let Some(at) = info.started_at {
                // A native root prompt is written with its send time before
                // its turn receipt arrives; the desktop schema calls it startedAt.
                // A message sent after that start is a steer, never its root.
                if block["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("gpui-"))
                    && block["startedAt"].is_null()
                    && block["sentAt"]
                        .as_u64()
                        .is_none_or(|sent| at.saturating_add(START_SLACK_MS) >= sent)
                {
                    block["startedAt"] = json!(at);
                    if block["sentAt"].as_u64() == Some(at) {
                        block.as_object_mut().unwrap().remove("sentAt");
                    }
                    changed = true;
                }
            }
            if let Some(metrics) = info.metrics.filter(|metrics| metrics.reported()) {
                fields.push(("turnMetrics", json!({"inputTokens":metrics.input_tokens,
                    "outputTokens":metrics.output_tokens,"cacheReadTokens":metrics.cache_read_tokens,
                    "cacheWriteTokens":metrics.cache_write_tokens,"cacheHitPercent":metrics.cache_hit_percent})));
            }
            for (key, value) in fields {
                if block[key].is_null() {
                    block[key] = value;
                    changed = true;
                } else if key == "turnModel" {
                    if let (Some(saved), Some(incoming)) =
                        (block[key].as_object_mut(), value.as_object())
                    {
                        for (name, value) in incoming {
                            if !saved.contains_key(name) {
                                saved.insert(name.clone(), value.clone());
                                changed = true;
                            }
                        }
                    }
                }
            }
        }
        if changed {
            self.shared_store_blocks(id, &blocks)?;
        }
        sql(tx.commit())?;
        if changed {
            self.shared
                .as_ref()
                .unwrap()
                .snapshots
                .borrow_mut()
                .insert(id, HistoryRevision::of(&blocks));
        }
        Ok(())
    }

    /// The blocks whose rows this store numbers: an indexed Mono's window, or
    /// the whole history of one stored as a blob.
    pub(in crate::store) fn shared_blocks(&self, id: i64) -> Result<Vec<Value>, String> {
        let original = self.persisted_session_id(id)?;
        if let Some(from) = self.shared_window(id, &original)? {
            self.shared_indexed_blocks(&original, from, i64::MAX)
        } else {
            let raw: String = sql(self.conn.query_row(
                "SELECT blocks_json FROM sessions WHERE id=?1",
                [original],
                |r| r.get(0),
            ))?;
            parse(&raw)
        }
    }

    pub(in crate::store) fn shared_load(&self, id: i64) -> Result<Transcript, String> {
        let original = self.persisted_session_id(id)?;
        if let Some(from) = self.shared_window(id, &original)? {
            // Project one source block at a time. A long Mono must not retain
            // a second complete JSON history alongside its display rows.
            // The scalar count and the rows share one SQLite read snapshot.
            let mut query = sql(self.conn.prepare(
                "SELECT block_json,(SELECT count(*) FROM mono_blocks WHERE session_id=?1 AND seq>=?2)
                 FROM mono_blocks WHERE session_id=?1 AND seq>=?2 ORDER BY seq",
            ))?;
            let mut records = sql(query.query(params![original, from]))?;
            let mut rows = Vec::new();
            let mut digest = HistoryRevision::start(0);
            let mut first = true;
            while let Some(record) = sql(records.next())? {
                if first {
                    digest = HistoryRevision::start(sql(record.get::<_, i64>(1))? as usize);
                    first = false;
                }
                let raw: String = sql(record.get(0))?;
                let block: Value = parse(&raw)?;
                HistoryRevision::push(&mut digest, &block);
                rows.extend(blocks::project(std::slice::from_ref(&block))?);
            }
            self.shared
                .as_ref()
                .unwrap()
                .snapshots
                .borrow_mut()
                .insert(id, HistoryRevision(digest.finalize().into()));
            return Ok(Transcript { rows });
        }
        let blocks = self.shared_blocks(id)?;
        let rows = blocks::project(&blocks)?;
        self.shared
            .as_ref()
            .unwrap()
            .snapshots
            .borrow_mut()
            .insert(id, HistoryRevision::of(&blocks));
        Ok(Transcript { rows })
    }

    pub(in crate::store) fn shared_store_blocks(
        &self,
        id: i64,
        blocks: &[Value],
    ) -> Result<(), String> {
        let original = self.persisted_session_id(id)?;
        let (has_user, draft) = if let Some(from) = self.shared_window(id, &original)? {
            self.shared_store_indexed_blocks(&original, from, blocks)?;
            // Blocks before the window count too, and were never read.
            sql(self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM mono_blocks WHERE session_id=?1 AND is_user=1),
                 EXISTS(SELECT 1 FROM mono_blocks WHERE session_id=?1 AND is_user=1 AND is_draft=1)",
                [&original],
                |row| Ok((row.get::<_, bool>(0)?, row.get::<_, bool>(1)?)),
            ))?
        } else {
            self.shared_patch(id, &[("blocks_json", json!(encode(&blocks)?))])?;
            (
                blocks.iter().any(|b| b["role"] == "user"),
                blocks
                    .iter()
                    .any(|b| b["role"] == "user" && b["draft"] == true),
            )
        };
        self.shared_patch(
            id,
            &[
                ("has_user_message", json!(has_user)),
                ("is_draft", json!(draft)),
            ],
        )
    }

    /// Replace the window's blocks, from sequence `from` on. Earlier blocks are
    /// outside every row index this store handed out, so they stay untouched.
    pub(super) fn shared_store_indexed_blocks(
        &self,
        original: &str,
        from: i64,
        blocks: &[Value],
    ) -> Result<(), String> {
        let mut query = sql(self.conn.prepare(
            "SELECT seq,block_id,block_json FROM mono_blocks WHERE session_id=?1 AND seq>=?2 ORDER BY seq",
        ))?;
        let mut old = HashMap::new();
        // New blocks always land inside the window, after everything stored.
        let first = from.max(1);
        let mut max_seq = sql(self.conn.query_row(
            "SELECT coalesce(max(seq),0) FROM mono_blocks WHERE session_id=?1",
            [original],
            |row| row.get::<_, i64>(0),
        ))?
        .max(first - 1);
        for row in sql(query.query_map(params![original, from], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        }))? {
            let (seq, id, raw) = sql(row)?;
            old.insert(id, (seq, parse::<Value>(&raw)?));
        }
        let retained: HashSet<_> = blocks.iter().filter_map(|b| b["id"].as_str()).collect();
        if retained.len() != blocks.len() {
            return Err("Indexed conversation contains missing or duplicate block IDs".into());
        }
        let mut previous = 0;
        let mut appended = false;
        let mut resequence = false;
        for block in blocks {
            if let Some((seq, _)) = old.get(block["id"].as_str().unwrap()) {
                if appended || *seq <= previous {
                    resequence = true;
                }
                previous = *seq;
            } else {
                appended = true;
            }
        }
        if resequence {
            // An explicit insertion/reorder changes turn positions. Payloads
            // and block identities survive, in one enclosing transaction.
            sql(self.conn.execute(
                "DELETE FROM mono_turns WHERE session_id=?1 AND seq>=?2",
                params![original, from],
            ))?;
            sql(self.conn.execute(
                "DELETE FROM mono_blocks WHERE session_id=?1 AND seq>=?2",
                params![original, from],
            ))?;
            old.clear();
            max_seq = first - 1;
        } else {
            for (id, (seq, _)) in &old {
                if !retained.contains(id.as_str()) {
                    sql(self.conn.execute(
                        "DELETE FROM mono_turns WHERE session_id=?1 AND seq=?2",
                        params![original, seq],
                    ))?;
                    sql(self.conn.execute(
                        "DELETE FROM mono_blocks WHERE session_id=?1 AND block_id=?2",
                        params![original, id],
                    ))?;
                }
            }
        }
        let mut after_handoff = false;
        for (ix, block) in blocks.iter().enumerate() {
            let id = block["id"].as_str().unwrap();
            let (seq, changed) = match old.get(id) {
                Some((seq, old)) => (*seq, old != block),
                None => {
                    max_seq += 1;
                    (max_seq, true)
                }
            };
            if changed {
                let search = [
                    block["text"].as_str(),
                    block["tool"]["title"].as_str(),
                    block["tool"]["detail"].as_str(),
                    block["image"]["name"].as_str(),
                    block["image"]["alt"].as_str(),
                    block["tool"]["preview"]["path"].as_str(),
                    block["tool"]["preview"]["query"].as_str(),
                    block["tool"]["preview"]["output"].as_str(),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("\n")
                .to_lowercase();
                sql(self.conn.execute("INSERT INTO mono_blocks(session_id,seq,block_id,block_json,search_text,is_user,is_draft)
                    VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(session_id,seq) DO UPDATE SET block_json=excluded.block_json,
                    search_text=excluded.search_text,is_user=excluded.is_user,is_draft=excluded.is_draft",
                    params![original,seq,id,encode(block)?,search,block["role"]=="user",block["draft"]==true]))?;
            }
            if ix == 0
                || after_handoff
                || (block["role"] == "user" && block["internal"] != true)
                || block.get("monoHabit").is_some()
            {
                sql(self.conn.execute(
                    "INSERT OR IGNORE INTO mono_turns(session_id,seq) VALUES(?1,?2)",
                    params![original, seq],
                ))?;
            }
            after_handoff = block["role"] == "handoff";
        }
        Ok(())
    }

    pub(in crate::store) fn shared_replace_rows(
        &self,
        id: i64,
        rows: &[Row],
    ) -> Result<(), String> {
        self.shared_writable()?;
        let next = blocks::merge(&[], 0, rows)?;
        self.shared_store_blocks(id, &next)?;
        self.shared_patch(id, &[("updated_at", json!(crate::sys::time::now_ms()))])?;
        self.shared
            .as_ref()
            .unwrap()
            .snapshots
            .borrow_mut()
            .insert(id, HistoryRevision::of(&next));
        Ok(())
    }

    pub(in crate::store) fn shared_write_rows(
        &self,
        id: i64,
        start: usize,
        rows: &[Row],
        message_id: Option<&str>,
    ) -> Result<(), String> {
        self.shared_writable()?;
        let tx = sql(self.conn.unchecked_transaction())?;
        let current = self.shared_blocks(id)?;
        if self
            .shared
            .as_ref()
            .unwrap()
            .snapshots
            .borrow()
            .get(&id)
            .is_some_and(|old| !old.matches(&current))
        {
            return Err("This conversation changed in another window. Reopen it before saving; its data was preserved.".into());
        }
        let mut next = blocks::merge(&current, start, rows)?;
        if let Some(message) = message_id {
            if let Some(prompt) = next
                .iter_mut()
                .rev()
                .find(|b| b["role"] == "user" && b["draft"] != true)
            {
                prompt["appRequestId"] = json!(message);
            }
            next.retain(|b| !(b["draft"] == true && b["id"].as_str() == Some(message)));
        }
        self.shared_store_blocks(id, &next)?;
        let mut next_queue = None;
        if let Some(message) = message_id {
            let field = self.shared_field("queued_messages_json", "'[]'");
            let raw: String = sql(self.conn.query_row(
                &format!("SELECT {field} FROM sessions WHERE id=?1"),
                [self.persisted_session_id(id)?],
                |r| r.get(0),
            ))?;
            let mut queued: Vec<Value> = parse(&raw)?;
            queued.retain(|m| m["id"].as_str() != Some(message));
            let encoded = encode(&queued)?;
            self.shared_patch(id, &[("queued_messages_json", json!(encoded))])?;
            next_queue = Some(encoded);
        }

        if !rows.is_empty() {
            self.shared_patch(id, &[("updated_at", json!(crate::sys::time::now_ms()))])?;
        }
        sql(tx.commit())?;
        if let Some(queue) = next_queue {
            self.shared
                .as_ref()
                .unwrap()
                .queues
                .borrow_mut()
                .insert(id, queue);
        }
        self.shared
            .as_ref()
            .unwrap()
            .snapshots
            .borrow_mut()
            .insert(id, HistoryRevision::of(&next));
        // The source appRequestId is the durable, transactional receipt.
        Ok(())
    }
}
