use super::*;

impl Store {
    pub(in crate::store) fn shared_reminders(&self) -> Result<Vec<Reminder>, String> {
        if !has_table(&self.conn, "session_reminders")? {
            return Ok(Vec::new());
        }
        let mut query = sql(self.conn.prepare(
            "SELECT session_id,due_at,fired_at FROM session_reminders ORDER BY due_at,session_id",
        ))?;
        let mut out = Vec::new();
        for row in sql(query.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<i64>>(2)?,
            ))
        }))? {
            let (id, due, fired) = sql(row)?;
            out.push(Reminder {
                session_id: self.shared_register(&id)?,
                due_at: due.max(0) as u64,
                fired_at: fired.map(|at| at.max(0) as u64),
            });
        }
        Ok(out)
    }

    pub(in crate::store) fn shared_set_reminders(
        &self,
        ids: &[i64],
        due: u64,
    ) -> Result<(), String> {
        self.shared_writable()?;
        let tx = sql(self.conn.unchecked_transaction())?;
        for id in ids {
            sql(self.conn.execute(
                "INSERT INTO session_reminders(session_id,due_at,fired_at) VALUES(?1,?2,NULL)
                ON CONFLICT(session_id) DO UPDATE SET due_at=excluded.due_at,fired_at=NULL",
                params![
                    self.persisted_session_id(*id)?,
                    due.min(i64::MAX as u64) as i64
                ],
            ))?;
        }
        sql(tx.commit())
    }

    pub(in crate::store) fn shared_snooze_reminder_if(
        &self,
        id: i64,
        expected_due: u64,
        new_due: u64,
    ) -> Result<bool, String> {
        self.shared_writable()?;
        sql(self.conn.execute("UPDATE session_reminders SET due_at=?3,fired_at=NULL WHERE session_id=?1 AND due_at=?2",
            params![self.persisted_session_id(id)?, expected_due.min(i64::MAX as u64) as i64, new_due.min(i64::MAX as u64) as i64])).map(|changed|changed==1)
    }

    pub(in crate::store) fn shared_clear_reminders(
        &self,
        ids: &[i64],
        expected: Option<u64>,
    ) -> Result<(), String> {
        self.shared_writable()?;
        for id in ids {
            sql(self.conn.execute(
                "DELETE FROM session_reminders WHERE session_id=?1 AND (?2 IS NULL OR due_at=?2)",
                params![
                    self.persisted_session_id(*id)?,
                    expected.map(|at| at.min(i64::MAX as u64) as i64)
                ],
            ))?;
        }
        Ok(())
    }

    pub(in crate::store) fn shared_fire_reminders(
        &self,
        due: &[Reminder],
        now: u64,
    ) -> Result<Vec<Reminder>, String> {
        self.shared_writable()?;
        let tx = sql(self.conn.unchecked_transaction())?;
        let mut fired = Vec::new();
        for reminder in due {
            let changed=sql(self.conn.execute("UPDATE session_reminders SET fired_at=?2 WHERE session_id=?1 AND fired_at IS NULL AND due_at=?3",
                params![self.persisted_session_id(reminder.session_id)?, now.min(i64::MAX as u64) as i64, reminder.due_at.min(i64::MAX as u64) as i64]))?;
            if changed == 1 {
                fired.push(reminder.clone());
            }
        }
        sql(tx.commit())?;
        Ok(fired)
    }
}
