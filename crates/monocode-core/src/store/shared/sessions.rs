use super::*;

impl Store {
    pub(in crate::store) fn shared_sessions(&self) -> Result<Vec<Session>, String> {
        let columns = [
            "id",
            "title",
            "harness",
            "cwd",
            "provider_session_id",
            "updated_at",
        ]
        .into_iter()
        .map(str::to_string)
        .chain(
            [
                (
                    "has_user_message",
                    "CASE WHEN blocks_json!='[]' THEN 1 ELSE 0 END",
                ),
                ("archived", "0"),
                ("pinned", "0"),
                ("linked_work_item_json", "NULL"),
                ("sidebar_hidden", "0"),
            ]
            .into_iter()
            .map(|(name, default)| self.shared_field(name, default)),
        )
        .collect::<Vec<_>>()
        .join(",");
        let index = if sql(self.conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name='sessions_cwd_cover_idx')",[],|r|r.get::<_,bool>(0)))? {
            " INDEXED BY sessions_cwd_cover_idx"
        } else { "" };
        let mut filters = vec!["1=1".to_string()];
        if self.shared.as_ref().unwrap().columns.contains("inbox_ask") {
            filters.push("id NOT IN (SELECT id FROM sessions WHERE inbox_ask IS NOT NULL)".into());
        }
        if has_table(&self.conn, "orchestration_workers")? {
            filters.push("id NOT IN (SELECT session_id FROM orchestration_workers)".into());
        }
        let mut query = sql(self.conn.prepare(&format!(
            "SELECT {columns} FROM sessions{index} WHERE {} ORDER BY updated_at DESC,id",
            filters.join(" AND ")
        )))?;
        let mut rows = sql(query.query([]))?;
        let mut result = Vec::new();
        let mut hidden = HashSet::new();
        let mut originals = HashMap::new();
        while let Some(row) = sql(rows.next())? {
            let original: String = sql(row.get(0))?;
            let id = self.shared_register(&original)?;
            if sql(row.get::<_, bool>(10))? {
                hidden.insert(id);
            }
            originals.insert(id, original);
            result.push(Session {
                id,
                title: sql(row.get(1))?,
                provider: sql(row.get(2))?,
                cwd: sql(row.get(3))?,
                provider_id: sql(row.get(4))?,
                updated_at: sql(row.get::<_, i64>(5))?.max(0) as u64,
                has_messages: sql(row.get(6))?,
                archived: sql(row.get(7))?,
                pinned: sql(row.get(8))?,
                work_item: sql(row.get::<_, Option<String>>(9))?.and_then(|raw| parse(&raw).ok()),
            });
        }
        // Keep the source sidebar's compatibility exception for hidden
        // /operator launches. A Mono's own hidden conversation cannot be a
        // listed caller. The UI also checks project/worktree and Mono scope.
        let shown: HashMap<_, _> = result
            .iter()
            .filter(|s| !hidden.contains(&s.id) && s.has_messages)
            .map(|s| (originals[&s.id].clone(), s.cwd.clone()))
            .collect();
        let shown_ids = shown.keys().cloned().collect();
        result.retain(|s| {
            !hidden.contains(&s.id)
                || started_caller(&originals[&s.id], &shown_ids)
                    .is_some_and(|caller| shown[caller] == s.cwd)
        });
        Ok(result)
    }

    pub(in crate::store) fn shared_create(
        &self,
        provider: &str,
        cwd: &str,
    ) -> Result<Session, String> {
        self.shared_create_named(provider, cwd, &crate::provider::accounts::request_id())
            .map(|(session, _)| session)
    }

    pub(in crate::store) fn shared_create_named(
        &self,
        provider: &str,
        cwd: &str,
        original: &str,
    ) -> Result<(Session, bool), String> {
        self.shared_writable()?;
        let label = crate::provider::Kind::parse(provider)
            .ok_or_else(|| format!("Unknown provider: {provider}"))?
            .label();
        let id = self.shared_register(original)?;
        let exists: bool = sql(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",
            [original],
            |row| row.get(0),
        ))?;
        if exists {
            let existing = self
                .shared_sessions()?
                .into_iter()
                .find(|s| s.id == id)
                .ok_or("The session created by this request was removed")?;
            if existing.provider != provider
                || crate::files::path_key(&existing.cwd) != crate::files::path_key(cwd)
            {
                return Err("Request ID was already used for another session".into());
            }
            return Ok((existing, false));
        }
        let now = crate::sys::time::now_ms().min(i64::MAX as u64) as i64;
        let title = format!("New {label} session");
        sql(self.conn.execute("INSERT INTO sessions(id,cwd,harness,model,model_settings,runtime_mode,title,blocks_json,created_at,updated_at)
            VALUES(?1,?2,?3,?4,'{}','supervised',?5,'[]',?6,?6)",
            params![original,cwd,provider,format!("{provider}:default"),title,now]))?;
        Ok((
            Session {
                id,
                title,
                provider: provider.into(),
                cwd: cwd.into(),
                provider_id: None,
                has_messages: false,
                updated_at: now as u64,
                archived: false,
                pinned: false,
                work_item: None,
            },
            true,
        ))
    }

    pub(in crate::store) fn shared_reset_provider_id_if(
        &self,
        id: i64,
        expected: Option<&str>,
    ) -> Result<(), String> {
        self.shared_writable()?;
        sql(self.conn.execute("UPDATE sessions SET provider_session_id=NULL WHERE id=?1 AND provider_session_id IS ?2",
            params![self.persisted_session_id(id)?,expected]))?;
        Ok(())
    }

    pub(in crate::store) fn shared_change_provider(
        &self,
        id: i64,
        provider: &str,
    ) -> Result<(), String> {
        self.shared_writable()?;
        sql(self.conn.execute(
            "UPDATE sessions SET harness=?2,provider_session_id=NULL WHERE id=?1 AND harness<>?2",
            params![self.persisted_session_id(id)?, provider],
        ))?;
        Ok(())
    }

    pub(in crate::store) fn shared_replace_title(
        &self,
        id: i64,
        expected: &str,
        title: &str,
    ) -> Result<bool, String> {
        self.shared_writable()?;
        sql(self.conn.execute(
            "UPDATE sessions SET title=?3 WHERE id=?1 AND title=?2",
            params![self.persisted_session_id(id)?, expected, title],
        ))
        .map(|n| n == 1)
    }

    pub(in crate::store) fn shared_worktrees(
        &self,
    ) -> Result<HashMap<i64, Option<String>>, String> {
        let mut query = sql(self.conn.prepare(&format!(
            "SELECT id,{} FROM sessions",
            self.shared_field("worktree_cwd", "NULL")
        )))?;
        let mut out = HashMap::new();
        for row in sql(query.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        }))? {
            let (id, path) = sql(row)?;
            out.insert(self.shared_register(&id)?, path);
        }
        Ok(out)
    }

    pub(in crate::store) fn shared_sidebar_hidden(&self) -> Result<HashSet<i64>, String> {
        let mut query = sql(self.conn.prepare(&format!(
            "SELECT id FROM sessions WHERE {}!=0",
            self.shared_field("sidebar_hidden", "0")
        )))?;
        let mut out = HashSet::new();
        for id in sql(query.query_map([], |r| r.get::<_, String>(0)))? {
            out.insert(self.shared_register(&sql(id)?)?);
        }
        Ok(out)
    }

    pub(in crate::store) fn shared_started_links(&self) -> Result<HashMap<i64, i64>, String> {
        let mut query = sql(self.conn.prepare(&format!(
            "SELECT id,{} FROM sessions",
            self.shared_field("sidebar_hidden", "0")
        )))?;
        let rows: Vec<(String, bool)> = sql(query.query_map([], |r| Ok((r.get(0)?, r.get(1)?))))?
            .collect::<rusqlite::Result<_>>()
            .map_err(|e| e.to_string())?;
        let ids = rows.iter().map(|(id, _)| id.clone()).collect();
        let shown = rows
            .iter()
            .filter(|(_, hidden)| !hidden)
            .map(|(id, _)| id.clone())
            .collect();
        rows.iter()
            .filter_map(|(id, hidden)| {
                started_caller(id, if *hidden { &shown } else { &ids }).map(|caller| (id, caller))
            })
            .map(|(id, caller)| Ok((self.shared_register(id)?, self.shared_register(caller)?)))
            .collect()
    }

    pub(in crate::store) fn shared_pins(&self, ids: &[i64], pinned: bool) -> Result<(), String> {
        let tx = sql(self.conn.unchecked_transaction())?;
        for id in ids {
            self.shared_patch(*id, &[("pinned", json!(pinned))])?;
        }
        sql(tx.commit())
    }

    pub(in crate::store) fn shared_drafts(&self) -> Result<HashSet<i64>, String> {
        let mut query = sql(self.conn.prepare(&format!(
            "SELECT id FROM sessions WHERE {}!=0",
            self.shared_field("is_draft", "0")
        )))?;
        let mut out = HashSet::new();
        for id in sql(query.query_map([], |r| r.get::<_, String>(0)))? {
            out.insert(self.shared_register(&sql(id)?)?);
        }
        Ok(out)
    }

    pub(in crate::store) fn shared_groups(
        &self,
        cwd: &str,
    ) -> Result<crate::session::groups::Groups, String> {
        if let Some(raw) = self.shared_setting(&format!("groups:{cwd}"))? {
            return parse(&raw);
        }
        Ok(self
            .desktop_preferences()
            .and_then(|preferences| {
                preferences.groups(cwd, |id| self.resolve_existing_session_id(id))
            })
            .unwrap_or_default())
    }

    pub(in crate::store) fn shared_delete(&self, id: i64) -> Result<(), String> {
        self.shared_writable()?;
        let original = self.persisted_session_id(id)?;
        let tx = sql(self.conn.unchecked_transaction())?;
        for (table, column) in [
            ("session_reminders", "session_id"),
            ("mono_blocks", "session_id"),
            ("mono_turns", "session_id"),
            ("mono_transcripts", "session_id"),
            ("in_flight_sessions", "session_id"),
            ("orchestration_workers", "session_id"),
            ("orchestration_sidebar", "lead_id"),
            ("orchestration_runs", "lead_id"),
        ] {
            if has_table(&self.conn, table)? {
                sql(self.conn.execute(
                    &format!("DELETE FROM {table} WHERE {column}=?1"),
                    [&original],
                ))?;
            }
        }
        sql(self
            .conn
            .execute("DELETE FROM sessions WHERE id=?1", [original]))?;
        sql(tx.commit())?;
        self.shared
            .as_ref()
            .unwrap()
            .snapshots
            .borrow_mut()
            .remove(&id);
        Ok(())
    }

    pub(in crate::store) fn shared_detach_worktree(
        &self,
        path: &str,
        main: &str,
    ) -> Result<Vec<(Session, Settings)>, String> {
        let tx = sql(self.conn.unchecked_transaction())?;
        let mut out = Vec::new();
        for mut session in self.shared_sessions()? {
            let mut settings = self.shared_agent(session.id)?.unwrap_or_default();
            let execution = settings.worktree_cwd.as_deref().unwrap_or(&session.cwd);
            if !crate::files::is_equal_or_inside(execution, path) {
                continue;
            }
            settings.worktree_cwd = Some(execution.into());
            settings.worktree_removed = true;
            settings.revision += 1;
            settings.new_worktree = false;
            settings.worktree_base = None;
            if crate::files::is_equal_or_inside(&session.cwd, path) {
                session.cwd = main.into();
            }
            session.provider_id = None;
            self.shared_patch(
                session.id,
                &[
                    ("cwd", json!(session.cwd)),
                    ("provider_session_id", Value::Null),
                ],
            )?;
            self.shared_save_agent_preferences(session.id, &settings)?;
            out.push((session, settings));
        }
        sql(tx.commit())?;
        Ok(out)
    }

    pub(in crate::store) fn shared_search(
        &self,
        query: &str,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<(Vec<crate::store::search::Hit>, bool), String> {
        use std::sync::atomic::Ordering;
        if query.trim().is_empty() || cancel.load(Ordering::Acquire) {
            return Ok((Vec::new(), false));
        }
        let flag = cancel.clone();
        sql(self
            .conn
            .progress_handler(2000, Some(move || flag.load(Ordering::Acquire))))?;
        let result = (|| {
            let summaries = self.shared_sessions()?;
            let mut hits = Vec::new();
            let mut truncated = false;
            'sessions: for summary in summaries.iter().filter(|s| !s.archived && s.has_messages) {
                if cancel.load(Ordering::Acquire) {
                    return Ok((Vec::new(), false));
                }
                let rows = blocks::project(&self.shared_all_blocks(summary.id)?)?;
                let mut ix = 0;
                while ix < rows.len() {
                    if cancel.load(Ordering::Acquire) {
                        return Ok((Vec::new(), false));
                    }
                    let from = ix;
                    let mut text = String::new();
                    loop {
                        let body = &rows[ix].text;
                        let mut end = body
                            .len()
                            .min((2 * 1024 * 1024usize).saturating_sub(text.len()));
                        while !body.is_char_boundary(end) {
                            end -= 1;
                        }
                        text.push_str(&body[..end]);
                        truncated |= end < body.len();
                        ix += 1;
                        if ix == rows.len() || rows[ix].first {
                            break;
                        }
                        if cancel.load(Ordering::Acquire) {
                            return Ok((Vec::new(), false));
                        }
                    }
                    if let Some(hit) = crate::store::search::message_hit(
                        summary,
                        from,
                        &rows[from].role,
                        &text,
                        query,
                        false,
                    ) {
                        hits.push(hit);
                    }
                    if hits.len() >= 400 {
                        truncated = true;
                        break 'sessions;
                    }
                }
            }
            hits.extend(crate::store::search::titles(&summaries, query));
            Ok((hits, truncated))
        })();
        sql(self.conn.progress_handler(0, None::<fn() -> bool>))?;
        if cancel.load(Ordering::Acquire) {
            Ok((Vec::new(), false))
        } else {
            result
        }
    }
}
