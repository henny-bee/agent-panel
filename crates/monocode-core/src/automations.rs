//! Persisted automations, scheduling, event deduplication, and run history.
pub mod db;
pub mod model;
mod validate;
use db::{
    apply_run_summary, ensure_tables, get, legacy_trigger, list, list_runs, new_event_run, new_run,
    trim_history, write_automation, write_run,
};
use model::{
    default_actor, next_run_at, Automation, AutomationEventClaim, AutomationRun, AutomationTrigger,
    AutomationUpsert, DueAutomationRun,
};
use std::collections::HashMap;
use validate::{
    has_event_trigger, has_time_trigger, normalize_triggers, trigger_from_fields,
    validate_event_key, validate_id, validate_upsert,
};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use chrono::{Datelike, Local, TimeZone, Timelike};
use std::path::Path;

fn now_millis() -> i64 {
    crate::sys::time::now_ms().min(i64::MAX as u64) as i64
}

pub fn new_id() -> String {
    format!("{:032x}", rand::random::<u128>())
}

const MAX_NAME: usize = 200;
const MAX_PROMPT: usize = 1_000_000;
const MAX_TRIGGERS: usize = 20;
const MAX_RUNS_PER_AUTOMATION: i64 = 100;

/// Access to the same tables and JSON used by the Tauri application.
/// Open with the session database path to preserve existing definitions and history.
pub struct Service {
    conn: Connection,
}
impl Service {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(|e| e.to_string())?;
        ensure_tables(&conn).map_err(|e| e.to_string())?;
        Ok(Self { conn })
    }
    fn lock_conn(&mut self) -> Result<&mut Connection, String> {
        Ok(&mut self.conn)
    }
    pub fn list(&mut self) -> Result<Vec<Automation>, String> {
        let conn = self.lock_conn()?;
        list(conn)
    }

    pub fn upsert(&mut self, automation: AutomationUpsert) -> Result<Automation, String> {
        let now = now_millis();
        validate_upsert(&automation, now)?;
        let triggers = Some(normalize_triggers(&automation));
        let conn = self.lock_conn()?;
        let previous = get(conn, &automation.id)?;
        let created_at = previous
            .as_ref()
            .map(|value| value.created_at)
            .unwrap_or(now);
        let worktree_cwd = if automation.workspace_mode == "existing" {
            automation.worktree_cwd.trim().to_string()
        } else {
            String::new()
        };
        let saved = Automation {
            id: automation.id,
            name: automation.name.trim().to_string(),
            prompt: automation.prompt,
            harness: automation.harness,
            model: automation.model,
            model_settings: automation.model_settings,
            cwd: automation.cwd,
            workspace_mode: automation.workspace_mode,
            worktree_cwd,
            session_folder_id: automation.session_folder_id.trim().to_string(),
            reuse_session: automation.reuse_session,
            runtime_mode: automation.runtime_mode,
            trigger_kind: automation.trigger_kind,
            trigger_event: automation.trigger_event.trim().to_string(),
            schedule_kind: automation.schedule_kind,
            minute: automation.minute,
            time: automation.time,
            day_of_week: automation.day_of_week,
            triggers,
            missed_run_grace_minutes: automation.missed_run_grace_minutes,
            enabled: automation.enabled,
            next_run_at: automation.next_run_at,
            last_run_at: previous.as_ref().and_then(|value| value.last_run_at),
            last_run_status: previous
                .as_ref()
                .and_then(|value| value.last_run_status.clone()),
            last_run_error: previous
                .as_ref()
                .and_then(|value| value.last_run_error.clone()),
            last_session_id: previous.and_then(|value| value.last_session_id),
            created_at,
            updated_at: now,
        };
        write_automation(conn, &saved)?;
        Ok(saved)
    }

    pub fn delete(&mut self, id: &str) -> Result<(), String> {
        validate_id(id, "automation")?;
        let conn = self.lock_conn()?;
        conn.execute("DELETE FROM automations WHERE id = ?1", [id])
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn runs(&mut self, automation_id: &str) -> Result<Vec<AutomationRun>, String> {
        validate_id(automation_id, "automation")?;
        let conn = self.lock_conn()?;
        let mut runs = list_runs(conn, automation_id)?;
        // Launch prompts can be large and are only needed by restart recovery, not
        // by the run-history surface.
        for run in &mut runs {
            run.prompt = None;
        }
        Ok(runs)
    }

    pub fn recover(
        &mut self,
        started_before: i64,
        now: i64,
    ) -> Result<Vec<DueAutomationRun>, String> {
        let conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let mut statement = tx
            .prepare(
                "SELECT run_json FROM automation_runs WHERE created_at <= ?1
             ORDER BY created_at, id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([started_before], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        let mut runs = Vec::new();
        for row in rows {
            let raw = row.map_err(|error| error.to_string())?;
            runs.push(
                serde_json::from_str::<AutomationRun>(&raw).map_err(|error| error.to_string())?,
            );
        }
        drop(statement);

        let mut due = Vec::new();
        let mut changed_automation_ids = Vec::new();
        for mut run in runs {
            if run.status != "pending" && run.status != "running" {
                continue;
            }
            let Some(mut automation) = get(&tx, &run.automation_id)? else {
                continue;
            };
            if run.status == "running" {
                run.status = "cancelled".into();
                run.completed_at = Some(now);
                run.error = Some("Interrupted when MCode last stopped.".into());
                apply_run_summary(&mut automation, &run);
                write_run(&tx, &run)?;
                write_automation(&tx, &automation)?;
                changed_automation_ids.push(run.automation_id.clone());
                continue;
            }
            if run.trigger == "event"
                && run
                    .prompt
                    .as_deref()
                    .is_none_or(|prompt| prompt.trim().is_empty())
            {
                run.status = "failed".into();
                run.completed_at = Some(now);
                run.error = Some("The Inbox event prompt was not available after restart.".into());
                apply_run_summary(&mut automation, &run);
                write_run(&tx, &run)?;
                write_automation(&tx, &automation)?;
                changed_automation_ids.push(run.automation_id.clone());
                continue;
            }
            due.push(DueAutomationRun { automation, run });
        }
        changed_automation_ids.sort();
        changed_automation_ids.dedup();
        for automation_id in changed_automation_ids {
            trim_history(&tx, &automation_id)?;
        }
        tx.commit().map_err(|error| error.to_string())?;
        Ok(due)
    }

    pub fn run_now(&mut self, automation_id: &str, now: i64) -> Result<AutomationRun, String> {
        validate_id(automation_id, "automation")?;
        let conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let mut automation =
            get(&tx, automation_id)?.ok_or_else(|| "Automation not found.".to_string())?;
        let run = new_run(automation_id, "manual", now, now, &automation.prompt);
        automation.last_run_at = Some(now);
        automation.last_run_status = Some(run.status.clone());
        automation.last_run_error = None;
        write_run(&tx, &run)?;
        write_automation(&tx, &automation)?;
        trim_history(&tx, automation_id)?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(run)
    }

    pub fn claim_occurrence(
        &mut self,
        automation_id: &str,
        expected_next_run_at: i64,
        next_run_at: i64,
        now: i64,
    ) -> Result<Option<DueAutomationRun>, String> {
        validate_id(automation_id, "automation")?;
        if next_run_at <= expected_next_run_at {
            return Err("The following automation occurrence must advance the schedule.".into());
        }
        let conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let current =
            get(&tx, automation_id)?.ok_or_else(|| "Automation not found.".to_string())?;
        if !has_time_trigger(&current) {
            tx.rollback().map_err(|error| error.to_string())?;
            return Ok(None);
        }
        let claimed = tx
            .execute(
                "UPDATE automations SET next_run_at = ?1
             WHERE id = ?2 AND enabled = 1 AND next_run_at = ?3 AND next_run_at <= ?4",
                params![next_run_at, automation_id, expected_next_run_at, now],
            )
            .map_err(|error| error.to_string())?;
        if claimed == 0 {
            tx.rollback().map_err(|error| error.to_string())?;
            return Ok(None);
        }
        let mut automation =
            get(&tx, automation_id)?.ok_or_else(|| "Automation not found.".to_string())?;
        automation.next_run_at = next_run_at;
        let mut run = new_run(
            automation_id,
            "scheduled",
            expected_next_run_at,
            now,
            &automation.prompt,
        );
        let grace_ms = automation.missed_run_grace_minutes.saturating_mul(60_000);
        let missed = now.saturating_sub(expected_next_run_at);
        let dispatch = missed <= grace_ms;
        if !dispatch {
            run.status = "skipped".into();
            run.completed_at = Some(now);
            run.error = Some("Missed the scheduled run beyond its grace period.".into());
        }
        automation.last_run_at = Some(now);
        automation.last_run_status = Some(run.status.clone());
        automation.last_run_error = run.error.clone();
        write_run(&tx, &run)?;
        write_automation(&tx, &automation)?;
        trim_history(&tx, automation_id)?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(Some(DueAutomationRun { automation, run }))
    }

    pub fn claim_event(
        &mut self,
        automation_id: &str,
        claim: AutomationEventClaim,
        now: i64,
    ) -> Result<Option<DueAutomationRun>, String> {
        validate_id(automation_id, "automation")?;
        validate_event_key(&claim.event_key)?;
        if !matches!(
            claim.event_kind.as_str(),
            "github" | "linear" | "jira" | "gitlab" | "azuredevops"
        ) {
            return Err("Invalid automation trigger.".into());
        }
        if claim.event.is_empty() || claim.event.len() > 200 {
            return Err("Invalid automation trigger event.".into());
        }
        if claim.prompt.trim().is_empty() || claim.prompt.len() > MAX_PROMPT + 10_000 {
            return Err("Invalid automation run prompt.".into());
        }
        let scheduled_for = if claim.scheduled_for > 0 {
            claim.scheduled_for
        } else {
            now
        };
        let conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let mut automation =
            get(&tx, automation_id)?.ok_or_else(|| "Automation not found.".to_string())?;
        if !automation.enabled || !has_event_trigger(&automation, &claim.event_kind, &claim.event) {
            tx.rollback().map_err(|error| error.to_string())?;
            return Ok(None);
        }
        let claimed = tx
            .execute(
                "INSERT OR IGNORE INTO automation_event_claims
             (automation_id, event_key, created_at) VALUES (?1, ?2, ?3)",
                params![automation_id, claim.event_key, now],
            )
            .map_err(|error| error.to_string())?;
        if claimed == 0 {
            tx.rollback().map_err(|error| error.to_string())?;
            return Ok(None);
        }
        let run = new_event_run(
            automation_id,
            &claim.event_key,
            &claim.event_kind,
            &claim.event,
            scheduled_for,
            now,
            &claim.prompt,
        );
        automation.last_run_at = Some(now);
        automation.last_run_status = Some(run.status.clone());
        automation.last_run_error = None;
        write_run(&tx, &run)?;
        write_automation(&tx, &automation)?;
        trim_history(&tx, automation_id)?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(Some(DueAutomationRun { automation, run }))
    }

    pub fn update_run(
        &mut self,
        run_id: &str,
        status: &str,
        session_id: Option<String>,
        error: Option<String>,
        now: i64,
    ) -> Result<AutomationRun, String> {
        validate_id(run_id, "automation run")?;
        if let Some(session_id) = session_id.as_deref() {
            validate_id(session_id, "session")?;
        }
        if !matches!(
            status,
            "pending" | "running" | "succeeded" | "failed" | "skipped" | "cancelled"
        ) {
            return Err("Invalid automation run status.".into());
        }
        let conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        let raw = tx
            .query_row(
                "SELECT run_json FROM automation_runs WHERE id = ?1",
                [&run_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Automation run not found.".to_string())?;
        let mut run: AutomationRun =
            serde_json::from_str(&raw).map_err(|error| error.to_string())?;
        run.status = status.to_owned();
        if run.status == "running" && run.started_at.is_none() {
            run.started_at = Some(now);
        }
        if matches!(
            run.status.as_str(),
            "succeeded" | "failed" | "skipped" | "cancelled"
        ) {
            run.completed_at = Some(now);
        }
        if session_id.is_some() {
            run.session_id = session_id;
        }
        run.error = error.filter(|value| !value.trim().is_empty());
        let mut automation =
            get(&tx, &run.automation_id)?.ok_or_else(|| "Automation not found.".to_string())?;
        apply_run_summary(&mut automation, &run);
        write_run(&tx, &run)?;
        write_automation(&tx, &automation)?;
        trim_history(&tx, &run.automation_id)?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(run)
    }
}

impl Service {
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<Automation, String> {
        let automation = get(&self.conn, id)?.ok_or("Automation not found.")?;
        let mut draft = AutomationUpsert::from_automation(&automation);
        draft.enabled = enabled;
        draft.sync_triggers(now_millis());
        self.upsert(draft)
    }
    /// Atomically claims each due occurrence, including skipped occurrences outside
    /// the grace period. Bounded work prevents an offline period monopolizing UI polling.
    pub fn claim_due(&mut self, now: i64) -> Result<Vec<DueAutomationRun>, String> {
        let mut due = Vec::new();
        for mut automation in self.list()? {
            if !automation.enabled || !has_time_trigger(&automation) {
                continue;
            }
            for _ in 0..100 {
                if automation.next_run_at > now {
                    break;
                }
                let next = next_run_at(&automation.triggers(), automation.next_run_at);
                if next <= automation.next_run_at {
                    break;
                }
                let claim = match self.claim_occurrence(
                    &automation.id,
                    automation.next_run_at,
                    next,
                    now,
                ) {
                    Ok(Some(claim)) => claim,
                    // A failed transaction leaves this occurrence due for the
                    // next poll. Preserve already committed claims so callers
                    // can dispatch them instead of stranding pending runs.
                    Ok(None) | Err(_) => break,
                };
                automation = claim.automation.clone();
                if claim.run.status == "pending" {
                    due.push(claim);
                }
            }
        }
        Ok(due)
    }
}

/// Matches the original Inbox appeared-event filters before using the durable
/// claim ledger. Polling, restarting and overlapping refreshes cannot rerun an item.
pub fn claim_inbox_events(
    service: &mut Service,
    items: &[crate::integrations::inbox::Item],
    now: i64,
) -> Result<Vec<DueAutomationRun>, String> {
    claim_inbox_events_retry(service, items, now).map(|(runs, _)| runs)
}

/// A failed match leaves its Inbox item retryable while successful claims are
/// still returned for dispatch. The ledger makes retries of partial batches safe.
pub fn claim_inbox_events_retry(
    service: &mut Service,
    items: &[crate::integrations::inbox::Item],
    now: i64,
) -> Result<(Vec<DueAutomationRun>, Vec<crate::integrations::inbox::Item>), String> {
    let automations = service.list()?;
    let mut claimed = Vec::new();
    let mut retry = Vec::new();
    for item in items {
        let event = match (item.provider.as_str(), item.kind.as_str(), item.draft) {
            ("github", "pr", true) => "draft_opened",
            ("github", "pr", false) => "pull_request_opened",
            ("github" | "gitlab", "issue", _) => "issue_opened",
            ("gitlab", "pr", _) => "merge_request_opened",
            ("linear" | "jira", _, _) => "issue_created",
            ("azuredevops", "pr", _) => "pull_request_appeared",
            ("azuredevops", "issue", _) => "work_item_appeared",
            _ => continue,
        };
        let identity = if crate::integrations::inbox::is_tracker(&item.provider) {
            let id = if !item.id.is_empty() {
                item.id.clone()
            } else if !item.identifier.is_empty() {
                item.identifier.clone()
            } else {
                item.number.to_string()
            };
            format!("{}:issue:{id}", item.provider)
        } else {
            format!(
                "{}:{}:{}:{}",
                item.provider, item.kind, item.repo, item.number
            )
        };
        let event_key = identity
            .trim()
            .to_lowercase()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '-' | '.' | '/') {
                    c
                } else {
                    '_'
                }
            })
            .take(400)
            .collect::<String>();
        for automation in &automations {
            if !automation.enabled {
                continue;
            }
            let tracker_without_project = crate::integrations::inbox::is_tracker(&item.provider)
                && item.project_path.trim().is_empty();
            let path_key = |path: &str| {
                let normalized = path.trim().replace('\\', "/");
                let normalized = normalized.trim_end_matches('/');
                let normalized = if normalized.is_empty() {
                    "/"
                } else {
                    normalized
                };
                if normalized.as_bytes().get(1) == Some(&b':') || normalized.starts_with("//") {
                    normalized.to_lowercase()
                } else {
                    normalized.to_string()
                }
            };
            if !tracker_without_project && path_key(&item.project_path) != path_key(&automation.cwd)
            {
                continue;
            }
            let trigger = automation.triggers().into_iter().find(|trigger| {
                if trigger.kind != item.provider || trigger.event != event {
                    return false;
                }
                let actor = trigger.actor.trim();
                if !actor.is_empty() && !actor.eq_ignore_ascii_case("anyone") {
                    return false;
                }
                let repos: Vec<_> = trigger
                    .repos
                    .iter()
                    .chain(std::iter::once(&trigger.repo))
                    .map(|repo| repo.trim())
                    .filter(|repo| !repo.is_empty())
                    .collect();
                repos.is_empty()
                    || repos
                        .iter()
                        .any(|repo| repo.eq_ignore_ascii_case(item.repo.trim()))
            });
            let Some(trigger) = trigger else {
                continue;
            };
            let provider = crate::integrations::inbox::provider_label(&item.provider);
            let kind =
                if crate::integrations::inbox::is_tracker(&item.provider) || item.kind != "pr" {
                    "issue"
                } else if item.provider == "gitlab" {
                    "merge request"
                } else {
                    "pull request"
                };
            let reference = if crate::integrations::inbox::is_tracker(&item.provider) {
                if item.identifier.trim().is_empty() {
                    format!("{provider} #{}", item.number)
                } else {
                    item.identifier.trim().to_string()
                }
            } else {
                format!("#{}", item.number)
            };
            let title = if item.title.trim().is_empty() {
                if crate::integrations::inbox::is_tracker(&item.provider) {
                    reference.clone()
                } else {
                    format!("{provider} {kind} #{}", item.number)
                }
            } else {
                item.title.trim().to_string()
            };
            let mut prompt = format!(
                "{}\n\nWork on this {provider} {kind}:\n\n{reference} {title}",
                automation.prompt.trim()
            );
            if !item.url.trim().is_empty() {
                prompt.push('\n');
                prompt.push_str(item.url.trim());
            }
            let created = crate::integrations::inbox::timestamp_ms(&item.created_at);
            let occurred_at = if created != 0 {
                created
            } else {
                item.updated_ms()
            };
            match service.claim_event(
                &automation.id,
                AutomationEventClaim {
                    event_key: event_key.clone(),
                    event_kind: trigger.kind,
                    event: trigger.event,
                    scheduled_for: if occurred_at == 0 { now } else { occurred_at },
                    prompt,
                },
                now,
            ) {
                Ok(Some(run)) => claimed.push(run),
                Ok(None) => {}
                Err(_) => {
                    if !retry
                        .iter()
                        .any(|existing: &crate::integrations::inbox::Item| existing == item)
                    {
                        retry.push(item.clone());
                    }
                }
            }
        }
    }
    Ok((claimed, retry))
}

#[cfg(test)]
mod tests;
