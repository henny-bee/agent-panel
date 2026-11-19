use super::*;

impl Store {
    pub fn reminders(&self) -> Result<Vec<Reminder>, String> {
        if self.shared.is_some() {
            return self.shared_reminders();
        }

        let mut stmt = self
            .conn
            .prepare("SELECT session_id,due_at,fired_at FROM native_reminders ORDER BY due_at,session_id")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Reminder {
                    session_id: r.get(0)?,
                    due_at: r.get::<_, i64>(1)?.max(0) as u64,
                    fired_at: r.get::<_, Option<i64>>(2)?.map(|t| t.max(0) as u64),
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    /// Schedules (or reschedules, re-arming) each session's reminder.
    pub fn set_reminders(&mut self, ids: &[i64], due_at: u64) -> Result<(), String> {
        if self.shared.is_some() {
            return self.shared_set_reminders(ids, due_at);
        }

        let tx = self.conn.transaction().map_err(|e| e.to_string())?;
        for id in ids {
            tx.execute(
                "INSERT INTO native_reminders(session_id,due_at,fired_at) VALUES(?1,?2,NULL)
                 ON CONFLICT(session_id) DO UPDATE SET due_at=excluded.due_at,fired_at=NULL",
                params![id, due_at.min(i64::MAX as u64) as i64],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }

    /// Re-arms only the reminder occurrence the user snoozed. A stale popup
    /// must not overwrite a newer schedule created in another window.
    pub fn snooze_reminder_if(
        &self,
        id: i64,
        expected_due: u64,
        new_due: u64,
    ) -> Result<bool, String> {
        if self.shared.is_some() {
            return self.shared_snooze_reminder_if(id, expected_due, new_due);
        }
        self.conn.execute("UPDATE native_reminders SET due_at=?3,fired_at=NULL WHERE session_id=?1 AND due_at=?2",
            params![id, expected_due.min(i64::MAX as u64) as i64, new_due.min(i64::MAX as u64) as i64]).map(|changed|changed==1).map_err(|e|e.to_string())
    }

    /// Clears the sessions' reminders; with `expected`, only that one, so a
    /// stale delivery never clears a newer reminder.
    pub fn clear_reminders(&self, ids: &[i64], expected: Option<u64>) -> Result<(), String> {
        if self.shared.is_some() {
            return self.shared_clear_reminders(ids, expected);
        }

        for id in ids {
            match expected {
                Some(due) => self.conn.execute(
                    "DELETE FROM native_reminders WHERE session_id=?1 AND due_at=?2",
                    params![id, due.min(i64::MAX as u64) as i64],
                ),
                None => self
                    .conn
                    .execute("DELETE FROM native_reminders WHERE session_id=?1", [id]),
            }
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Marks due, undelivered reminders delivered and returns them, once.
    pub fn take_due_reminders(&mut self, now: u64) -> Result<Vec<Reminder>, String> {
        let due: Vec<_> = self
            .reminders()?
            .into_iter()
            .filter(|r| r.due_at <= now && r.fired_at.is_none())
            .collect();
        if self.shared.is_some() {
            return self.shared_fire_reminders(&due, now);
        }
        let tx = self.conn.transaction().map_err(|e| e.to_string())?;
        for reminder in &due {
            tx.execute(
                "UPDATE native_reminders SET fired_at=?2 WHERE session_id=?1",
                params![reminder.session_id, now.min(i64::MAX as u64) as i64],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(due)
    }
}
