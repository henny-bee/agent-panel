use super::*;

pub(crate) fn ensure_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS automations (
           id TEXT PRIMARY KEY,
           definition_json TEXT NOT NULL,
           enabled INTEGER NOT NULL,
           next_run_at INTEGER NOT NULL,
           updated_at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS automations_due_idx
           ON automations (enabled, next_run_at);
         CREATE TABLE IF NOT EXISTS automation_runs (
           id TEXT PRIMARY KEY,
           automation_id TEXT NOT NULL REFERENCES automations(id) ON DELETE CASCADE,
           created_at INTEGER NOT NULL,
           run_json TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS automation_runs_history_idx
           ON automation_runs (automation_id, created_at DESC);
         CREATE TABLE IF NOT EXISTS automation_event_claims (
           automation_id TEXT NOT NULL REFERENCES automations(id) ON DELETE CASCADE,
           event_key TEXT NOT NULL,
           created_at INTEGER NOT NULL,
           PRIMARY KEY (automation_id, event_key)
         );",
    )?;

    // Backfill claims from pre-ledger run history. Invalid legacy rows should not
    // prevent the session store from opening.
    let mut statement =
        conn.prepare("SELECT automation_id, created_at, run_json FROM automation_runs")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut claims = Vec::new();
    for row in rows {
        let (automation_id, created_at, raw) = row?;
        if let Ok(run) = serde_json::from_str::<AutomationRun>(&raw) {
            if let Some(event_key) = run.event_key {
                claims.push((automation_id, event_key, created_at));
            }
        }
    }
    drop(statement);
    for (automation_id, event_key, created_at) in claims {
        conn.execute(
            "INSERT OR IGNORE INTO automation_event_claims
             (automation_id, event_key, created_at) VALUES (?1, ?2, ?3)",
            params![automation_id, event_key, created_at],
        )?;
    }
    Ok(())
}

pub(super) fn hydrate_triggers(automation: &mut Automation) {
    if automation.triggers.is_some() {
        return;
    }
    automation.triggers = Some(vec![legacy_trigger(automation)]);
}

pub(super) fn legacy_trigger(automation: &Automation) -> AutomationTrigger {
    trigger_from_fields(
        &automation.id,
        &automation.trigger_kind,
        &automation.trigger_event,
        &automation.schedule_kind,
        automation.minute,
        &automation.time,
        automation.day_of_week,
    )
}

pub(super) fn parse_automation(value: String) -> Result<Automation, String> {
    let mut automation: Automation =
        serde_json::from_str(&value).map_err(|error| error.to_string())?;
    hydrate_triggers(&mut automation);
    Ok(automation)
}

pub(super) fn list(conn: &Connection) -> Result<Vec<Automation>, String> {
    let mut statement = conn
        .prepare("SELECT definition_json FROM automations ORDER BY updated_at DESC, id")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    rows.map(|row| {
        let value = row.map_err(|error| error.to_string())?;
        parse_automation(value)
    })
    .collect()
}

pub(super) fn get(conn: &Connection, id: &str) -> Result<Option<Automation>, String> {
    let raw = conn
        .query_row(
            "SELECT definition_json FROM automations WHERE id = ?1",
            [id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    raw.map(parse_automation).transpose()
}

pub(super) fn write_automation(conn: &Connection, automation: &Automation) -> Result<(), String> {
    let raw = serde_json::to_string(automation).map_err(|error| error.to_string())?;
    conn.execute(
        "INSERT INTO automations (id, definition_json, enabled, next_run_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET
           definition_json = excluded.definition_json,
           enabled = excluded.enabled,
           next_run_at = excluded.next_run_at,
           updated_at = excluded.updated_at",
        params![
            automation.id,
            raw,
            automation.enabled,
            automation.next_run_at,
            automation.updated_at
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn write_run(conn: &Connection, run: &AutomationRun) -> Result<(), String> {
    let raw = serde_json::to_string(run).map_err(|error| error.to_string())?;
    conn.execute(
        "INSERT INTO automation_runs (id, automation_id, created_at, run_json)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET run_json = excluded.run_json",
        params![run.id, run.automation_id, run.created_at, raw],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn new_run(
    automation_id: &str,
    trigger: &str,
    scheduled_for: i64,
    now: i64,
    prompt: &str,
) -> AutomationRun {
    AutomationRun {
        id: new_id(),
        automation_id: automation_id.to_string(),
        trigger: trigger.to_string(),
        scheduled_for,
        created_at: now,
        started_at: None,
        completed_at: None,
        status: "pending".into(),
        session_id: None,
        error: None,
        event_key: None,
        event_kind: None,
        event: None,
        prompt: Some(prompt.to_string()),
    }
}

pub(super) fn new_event_run(
    automation_id: &str,
    event_key: &str,
    event_kind: &str,
    event: &str,
    scheduled_for: i64,
    now: i64,
    prompt: &str,
) -> AutomationRun {
    let mut run = new_run(automation_id, "event", scheduled_for, now, prompt);
    run.event_key = Some(event_key.to_string());
    run.event_kind = Some(event_kind.to_string());
    run.event = Some(event.to_string());
    run
}

pub(super) fn list_runs(
    conn: &Connection,
    automation_id: &str,
) -> Result<Vec<AutomationRun>, String> {
    let mut statement = conn
        .prepare(
            "SELECT run_json FROM automation_runs WHERE automation_id = ?1
             ORDER BY created_at DESC LIMIT 100",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([automation_id], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    rows.map(|row| {
        let value = row.map_err(|error| error.to_string())?;
        serde_json::from_str(&value).map_err(|error| error.to_string())
    })
    .collect()
}

pub(super) fn trim_history(conn: &Connection, automation_id: &str) -> Result<(), String> {
    let mut statement = conn
        .prepare(
            "SELECT id, run_json FROM automation_runs WHERE automation_id = ?1
             ORDER BY created_at DESC, id DESC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([automation_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?;
    let mut delete_ids = Vec::new();
    for (index, row) in rows.enumerate() {
        let (id, raw) = row.map_err(|error| error.to_string())?;
        if index < MAX_RUNS_PER_AUTOMATION as usize {
            continue;
        }
        let run: AutomationRun = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
        if matches!(
            run.status.as_str(),
            "succeeded" | "failed" | "skipped" | "cancelled"
        ) {
            delete_ids.push(id);
        }
    }
    drop(statement);
    for id in delete_ids {
        conn.execute("DELETE FROM automation_runs WHERE id = ?1", [id])
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(super) fn apply_run_summary(automation: &mut Automation, run: &AutomationRun) {
    if automation
        .last_run_at
        .is_none_or(|last_run_at| run.created_at >= last_run_at)
    {
        automation.last_run_at = Some(run.created_at);
        automation.last_run_status = Some(run.status.clone());
        automation.last_run_error = run.error.clone();
        if run.session_id.is_some() {
            automation.last_session_id = run.session_id.clone();
        }
    }
}
