use super::*;

impl Store {
    pub fn reset_provider_id_if(&self, id: i64, expected: Option<&str>) -> Result<(), String> {
        if self.shared.is_some() {
            return self.shared_reset_provider_id_if(id, expected);
        }

        self.conn
            .execute(
                "UPDATE native_sessions SET provider_id=NULL WHERE id=?1 AND provider_id IS ?2",
                params![id, expected],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub(super) fn local_sessions(&self) -> Result<Vec<Session>, String> {
        if self.shared.is_some() {
            return self.shared_sessions();
        }

        let mut stmt = self
            .conn
            .prepare(
                "SELECT id,title,provider,cwd,provider_id,
                 (EXISTS(SELECT 1 FROM native_rows WHERE session_id=native_sessions.id)
                  OR EXISTS(SELECT 1 FROM native_settings WHERE key='follow-up:'||native_sessions.id AND json_valid(value) AND (json_type(value,'$.draft')='object' OR json_array_length(value,'$.messages')>0))),updated_at,archived,
                 pinned,work_item
                 FROM native_sessions WHERE NOT EXISTS(SELECT 1 FROM native_settings WHERE key='inbox-ask:'||native_sessions.id) ORDER BY updated_at DESC,id DESC",
            )
            .map_err(|e| e.to_string())?;
        let result = stmt
            .query_map([], |r| {
                Ok(Session {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    provider: r.get(2)?,
                    cwd: r.get(3)?,
                    provider_id: r.get(4)?,
                    has_messages: r.get(5)?,
                    updated_at: r.get::<_, i64>(6)?.max(0) as u64,
                    archived: r.get(7)?,
                    pinned: r.get(8)?,
                    work_item: r
                        .get::<_, Option<String>>(9)?
                        .and_then(|text| serde_json::from_str(&text).ok()),
                })
            })
            .map_err(|e| e.to_string())?;
        result
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    /// Sessions whose latest message is saved as a draft and not yet sent.
    pub fn draft_sessions(&self) -> Result<std::collections::HashSet<i64>, String> {
        if self.shared.is_some() {
            return self.shared_drafts();
        }

        let mut stmt = self
            .conn
            .prepare(
                "SELECT CAST(substr(key,11) AS INTEGER) FROM native_settings
                 WHERE key LIKE 'follow-up:%' AND json_valid(value)
                 AND json_type(value,'$.draft')='object'",
            )
            .map_err(|e| e.to_string())?;
        let ids = stmt
            .query_map([], |r| r.get::<_, i64>(0))
            .map_err(|e| e.to_string())?;
        ids.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }

    pub(super) fn local_create(&self, provider: &str, cwd: &str) -> Result<Session, String> {
        if self.shared.is_some() {
            return self.shared_create(provider, cwd);
        }

        let updated_at = crate::session::response::now_ms();
        let title = format!(
            "New {} session",
            crate::provider::Kind::parse(provider)
                .ok_or_else(|| format!("Unknown provider: {provider}"))?
                .label()
        );
        self.conn
            .execute(
                "INSERT INTO native_sessions(title,provider,cwd,updated_at) VALUES(?1,?2,?3,?4)",
                params![title, provider, cwd, updated_at.min(i64::MAX as u64) as i64],
            )
            .map_err(|e| e.to_string())?;
        Ok(Session {
            id: self.conn.last_insert_rowid(),
            title,
            provider: provider.into(),
            cwd: cwd.into(),
            provider_id: None,
            has_messages: false,
            updated_at,
            archived: false,
            pinned: false,
            work_item: None,
        })
    }

    /// A WAL reader for background search, independent of the writer's mutex.
    /// In-memory fixture stores must continue using their original connection.
    pub fn search_reader(&self) -> Result<Option<Self>, String> {
        if self.path == Path::new(":memory:") {
            return Ok(None);
        }
        if self.shared.is_some() {
            return self.shared_search_reader().map(Some);
        }
        let conn =
            Connection::open_with_flags(&self.path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|e| e.to_string())?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        Ok(Some(Self {
            conn,
            path: self.path.clone(),
            shared: None,
        }))
    }

    /// Stream saved message bodies without loading conversations into the UI.
    /// Rejoin native row fragments so phrase matches span storage boundaries.
    pub fn search_sessions(
        &self,
        query: &str,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<(Vec<crate::store::search::Hit>, bool), String> {
        if self.shared.is_some() {
            return self.shared_search(query, cancel);
        }

        use std::sync::atomic::Ordering;
        if query.trim().is_empty() || cancel.load(Ordering::Acquire) {
            return Ok((vec![], false));
        }
        let flag = cancel.clone();
        self.conn
            .progress_handler(2000, Some(move || flag.load(Ordering::Acquire)))
            .map_err(|e| e.to_string())?;
        let result = (|| {
            let summaries = self.sessions()?;
            // Walk each conversation's primary-key range in recency order.
            // Sorting a join containing every message body would copy the entire
            // transcript database into SQLite's sorter on every keystroke.
            let mut statement=self.conn.prepare("SELECT row_ix,role,body,first FROM native_rows WHERE session_id=?1 ORDER BY row_ix").map_err(|e|e.to_string())?;
            let mut user = self
                .conn
                .prepare(
                    "SELECT EXISTS(SELECT 1 FROM native_rows WHERE session_id=?1 AND role='user')",
                )
                .map_err(|e| e.to_string())?;
            let mut hits = Vec::new();
            let mut truncated = false;
            'sessions: for summary in summaries.iter().filter(|s| !s.archived && s.has_messages) {
                if cancel.load(Ordering::Acquire) {
                    return Ok((vec![], false));
                }
                if !user
                    .query_row([summary.id], |r| r.get::<_, bool>(0))
                    .map_err(|e| e.to_string())?
                {
                    continue;
                }
                let flush =
                    |current: Option<(usize, String, String)>,
                     hits: &mut Vec<crate::store::search::Hit>| {
                        if let Some((row, role, text)) = current {
                            if let Some(hit) = crate::store::search::message_hit(
                                summary, row, &role, &text, query, false,
                            ) {
                                hits.push(hit);
                            }
                        }
                    };
                let mut current: Option<(usize, String, String)> = None;
                let mut rows = statement.query([summary.id]).map_err(|e| e.to_string())?;
                while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                    if cancel.load(Ordering::Acquire) {
                        return Ok((vec![], false));
                    }
                    let ix = row.get::<_, i64>(0).map_err(|e| e.to_string())?.max(0) as usize;
                    let role: String = row.get(1).map_err(|e| e.to_string())?;
                    let body: String = row.get(2).map_err(|e| e.to_string())?;
                    let first: bool = row.get(3).map_err(|e| e.to_string())?;
                    if first || current.is_none() {
                        flush(current.take(), &mut hits);
                        if hits.len() >= 400 {
                            truncated = true;
                            break 'sessions;
                        }
                        current = Some((ix, role, String::new()));
                    }
                    if let Some((_, _, text)) = &mut current {
                        let mut end = body
                            .len()
                            .min((2 * 1024 * 1024usize).saturating_sub(text.len()));
                        while !body.is_char_boundary(end) {
                            end -= 1;
                        }
                        text.push_str(&body[..end]);
                        truncated |= end < body.len();
                    }
                }
                flush(current, &mut hits);
                if hits.len() >= 400 {
                    truncated = true;
                    break;
                }
            }
            let titles = summaries
                .into_iter()
                .filter(|s| !s.archived && s.has_messages)
                .collect::<Vec<_>>();
            hits.extend(crate::store::search::titles(&titles, query));
            Ok((hits, truncated))
        })();
        self.conn
            .progress_handler(0, None::<fn() -> bool>)
            .map_err(|e| e.to_string())?;
        if cancel.load(Ordering::Acquire) {
            Ok((vec![], false))
        } else {
            result
        }
    }

    /// The cursor for the turns before `load`'s rows, when a Mono's history
    /// is paged rather than loaded whole.
    pub fn history_before(&self, session: i64) -> Result<Option<i64>, String> {
        if self.shared.is_none() || self.remote_session(session)?.is_some() {
            return Ok(None);
        }
        self.shared_history_before(session)
    }

    /// The page of turns before `before`, or with `all` every older turn.
    pub fn load_before(&self, session: i64, before: i64, all: bool) -> Result<HistoryPage, String> {
        if self.shared.is_none() {
            return Ok(HistoryPage::default());
        }
        self.shared_load_before(session, before, all)
    }

    pub fn load(&self, session: i64) -> Result<Transcript, String> {
        if let Some(remote) = self.remote_session(session)? {
            let Some(id) = remote.session_id.as_deref() else {
                return Ok(Transcript::default());
            };
            let client = crate::integrations::remote::default_connections()?;
            let machine = remote.machine(&client)?;
            let mut snapshot = client.load_session(&machine.id, id, None)?;
            crate::integrations::remote::attachments::qualify_snapshot(
                &mut snapshot,
                &remote.project.environment_id,
                id,
            );
            return Self::project_remote_history(&snapshot).map(|(history, _)| history);
        }
        if self.shared.is_some() {
            return self.shared_load(session);
        }

        let mut stmt = self
            .conn
            .prepare("SELECT role,body,first,sent_at FROM native_rows WHERE session_id=? ORDER BY row_ix")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([session], |r| {
                Ok(Row {
                    role: r.get(0)?,
                    text: r.get(1)?,
                    first: r.get(2)?,
                    sent_at: r
                        .get::<_, Option<i64>>(3)?
                        .and_then(|at| u64::try_from(at).ok()),
                })
            })
            .map_err(|e| e.to_string())?;
        let mut rows = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        // Pre-timestamp histories recorded completion and duration per run.
        // Recover their start time where known, leaving undated history undated.
        if let Ok(Some(settings)) = self.agent_settings(session) {
            for (turn, info) in settings.turns {
                if turn > rows.len() {
                    continue;
                }
                let mut prompt = turn;
                while prompt > 0 && rows[prompt - 1].is_prompt() {
                    prompt -= 1;
                }
                if let Some(row) = rows.get_mut(prompt) {
                    if row.sent_at.is_none() {
                        row.sent_at = info
                            .completed_at
                            .and_then(|at| at.checked_sub(info.duration_ms));
                    }
                }
            }
        }
        Ok(Transcript { rows })
    }

    pub fn write_rows(&mut self, session: i64, start: usize, rows: &[Row]) -> Result<(), String> {
        self.write_rows_with_message(session, start, rows, None)
    }

    pub fn write_rows_with_message(
        &mut self,
        session: i64,
        start: usize,
        rows: &[Row],
        message_id: Option<&str>,
    ) -> Result<(), String> {
        if self.remote_session(session)?.is_some() {
            return Err("Remote transcripts are owned by the host".into());
        }
        if self.shared.is_some() {
            return self.shared_write_rows(session, start, rows, message_id);
        }

        let tx = self.conn.transaction().map_err(|e| e.to_string())?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO native_rows(session_id,row_ix,role,body,first,sent_at) VALUES(?1,?2,?3,?4,?5,?6)
                ON CONFLICT(session_id,row_ix) DO UPDATE SET body=excluded.body",
                )
                .map_err(|e| e.to_string())?;
            for (offset, row) in rows.iter().enumerate() {
                stmt.execute(params![
                    session,
                    (start + offset) as i64,
                    row.role,
                    row.text,
                    row.first,
                    row.sent_at.and_then(|at| i64::try_from(at).ok())
                ])
                .map_err(|e| e.to_string())?;
            }
        }
        if !rows.is_empty() {
            tx.execute(
                "UPDATE native_sessions SET updated_at=?2 WHERE id=?1",
                params![
                    session,
                    crate::session::response::now_ms().min(i64::MAX as u64) as i64
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        if let Some(id) = message_id {
            tx.execute("INSERT INTO native_settings(key,value) VALUES(?1,'1') ON CONFLICT(key) DO UPDATE SET value='1'",[format!("mono-delivered:{session}:{id}")]).map_err(|e|e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }

    /// Archiving preserves the transcript, provider context and configuration.
    pub fn set_archived(&self, id: i64, archived: bool) -> Result<(), String> {
        if self.remote_session(id)?.is_some() {
            return self.patch_remote_session(id, serde_json::json!({"archived": archived}));
        }
        if self.shared.is_some() {
            return self.shared_patch(id, &[("archived", serde_json::json!(archived))]);
        }

        self.conn
            .execute(
                "UPDATE native_sessions SET archived=?2 WHERE id=?1",
                params![id, archived],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn set_pinned(&self, ids: &[i64], pinned: bool) -> Result<(), String> {
        let any_remote = ids
            .iter()
            .map(|id| self.remote_session(*id).map(|remote| remote.is_some()))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .any(|remote| remote);
        if any_remote {
            for id in ids {
                if self.remote_session(*id)?.is_some() {
                    self.patch_remote_session(*id, serde_json::json!({"pinned": pinned}))?;
                } else {
                    self.set_pinned(&[*id], pinned)?;
                }
            }
            return Ok(());
        }
        if self.shared.is_some() {
            return self.shared_pins(ids, pinned);
        }

        for id in ids {
            self.conn
                .execute(
                    "UPDATE native_sessions SET pinned=?2 WHERE id=?1",
                    params![id, pinned],
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn set_work_item(
        &self,
        id: i64,
        item: Option<&crate::session::groups::WorkItem>,
    ) -> Result<(), String> {
        if self.remote_session(id)?.is_some() {
            return self.patch_remote_session(id, serde_json::json!({"linkedWorkItem": item}));
        }
        if self.shared.is_some() {
            return self.shared_patch(
                id,
                &[(
                    "linked_work_item_json",
                    item.map(shared::encode)
                        .transpose()?
                        .map(serde_json::Value::String)
                        .unwrap_or(serde_json::Value::Null),
                )],
            );
        }

        let text = item
            .map(|item| serde_json::to_string(item).map_err(|e| e.to_string()))
            .transpose()?;
        self.conn
            .execute(
                "UPDATE native_sessions SET work_item=?2 WHERE id=?1",
                params![id, text],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Deletes the conversation, its settings, reminder and folder for good.
    pub fn delete(&mut self, id: i64) -> Result<(), String> {
        // Resolve the folder first; the persisted id is gone after deletion.
        let dir = self.thread_dir(id).ok().flatten();
        self.delete_rows(id)?;
        thread_files::remove_dir(dir);
        Ok(())
    }

    pub(super) fn delete_rows(&mut self, id: i64) -> Result<(), String> {
        if self.remote_session(id)?.is_some() {
            return self.delete_remote_session(id);
        }
        if self.shared.is_some() {
            return self.shared_delete(id);
        }

        let tx = self.conn.transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM native_rows WHERE session_id=?1", [id])
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM native_reminders WHERE session_id=?1", [id])
            .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM native_settings WHERE key IN (?1,?2,?3,?5,?6,?7,?8) OR key LIKE ?4",
            params![
                format!("agent:{id}"),
                format!("context:{id}"),
                format!("follow-up:{id}"),
                format!("mono-delivered:{id}:%"),
                format!("started-by:{id}"),
                format!("orchestration:{id}"),
                format!("inbox-ask:{id}"),
                format!("automation:{id}")
            ],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM native_sessions WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }

    pub fn provider_id(&self, id: i64, provider_id: &str) -> Result<(), String> {
        if self.shared.is_some() {
            return self.shared_patch(
                id,
                &[("provider_session_id", serde_json::json!(provider_id))],
            );
        }

        self.conn
            .execute(
                "UPDATE native_sessions SET provider_id=?2 WHERE id=?1",
                params![id, provider_id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Keep conversation history while detaching a removed working copy.
    pub fn detach_worktree(
        &self,
        path: &str,
        main: &str,
    ) -> Result<Vec<(Session, crate::session::agent::Settings)>, String> {
        if self.shared.is_some() {
            return self.shared_detach_worktree(path, main);
        }

        let sessions = self.sessions()?;
        let mut changed = Vec::new();
        for mut session in sessions {
            let mut settings = self.agent_settings(session.id)?.unwrap_or_default();
            let execution = settings.worktree_cwd.as_deref().unwrap_or(&session.cwd);
            if !crate::files::is_equal_or_inside(execution, path) {
                continue;
            }
            settings.worktree_cwd = Some(execution.to_owned());
            settings.worktree_removed = true;
            settings.revision += 1;
            settings.new_worktree = false;
            settings.worktree_base = None;
            if crate::files::is_equal_or_inside(&session.cwd, path) {
                session.cwd = main.into();
            }
            session.provider_id = None;
            changed.push((session, settings));
        }
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        for (session, settings) in &changed {
            tx.execute(
                "UPDATE native_sessions SET cwd=?2,provider_id=NULL WHERE id=?1",
                params![session.id, session.cwd],
            )
            .map_err(|e| e.to_string())?;
            tx.execute("INSERT INTO native_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![format!("agent:{}",session.id),serde_json::to_string(settings).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
            tx.execute(
                "DELETE FROM native_settings WHERE key=?1",
                [format!("context:{}", session.id)],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(changed)
    }

    pub fn reset_provider_id(&self, id: i64) -> Result<(), String> {
        if self.shared.is_some() {
            return self.shared_patch(id, &[("provider_session_id", serde_json::Value::Null)]);
        }

        self.conn
            .execute(
                "UPDATE native_sessions SET provider_id=NULL WHERE id=?1",
                [id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn title(&self, id: i64, title: &str) -> Result<(), String> {
        if self.remote_session(id)?.is_some() {
            return self.patch_remote_session(id, serde_json::json!({"title": title}));
        }
        if self.shared.is_some() {
            return self.shared_patch(id, &[("title", serde_json::json!(title))]);
        }

        self.conn
            .execute(
                "UPDATE native_sessions SET title=?2 WHERE id=?1",
                params![id, title],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// A late generated title cannot replace a manual rename or recreate a deleted session.
    pub fn replace_title(&self, id: i64, expected: &str, title: &str) -> Result<bool, String> {
        if self.shared.is_some() {
            return self.shared_replace_title(id, expected, title);
        }

        self.conn
            .execute(
                "UPDATE native_sessions SET title=?3 WHERE id=?1 AND title=?2",
                params![id, expected, title],
            )
            .map(|count| count == 1)
            .map_err(|error| error.to_string())
    }
}
