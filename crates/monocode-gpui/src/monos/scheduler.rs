mod editor;
mod view;
use super::panels::{button, empty_state, matches_query, search_bar};
use super::*;
use monocode_core::mono::habits::{Run, Schedule};
use monocode_core::mono::heartbeat;
use monocode_core::sys::time::now_ms;
use view::property;

enum Claimed {
    Habit(Box<Habit>),
    Heartbeat(String),
}
/// What a worker needs to decide whether a Mono's heartbeat is due.
struct HeartbeatCandidate {
    mono: Mono,
    /// The Mono's own chat, habit runs and other app-hidden sessions.
    skip: HashSet<i64>,
}
/// The user's inbox across the Mono's projects. Fetched without holding the
/// database: providers are network calls.
fn fetch_inbox(store: &Database, mono: &Mono) -> Vec<monocode_core::integrations::inbox::Item> {
    let Some(cwd) = mono.projects.first() else {
        return Vec::new();
    };
    let Some((data_dir, state)) = locked(store).ok().and_then(|db| {
        Some((
            db.data_dir()?.to_path_buf(),
            monocode_core::integrations::inbox::state::ViewState::load(&db),
        ))
    }) else {
        return Vec::new();
    };
    let service = monocode_core::integrations::inbox::Service {
        data_dir,
        cwd: cwd.into(),
    };
    let projects: Vec<std::path::PathBuf> = mono.projects.iter().map(Into::into).collect();
    service.list(&projects, &state).items
}
/// Runs on a worker. Advances the cursor whenever a heartbeat is due, even when
/// nothing changed, so a quiet Mono costs no model call.
fn prepare_heartbeat(
    store: &Database,
    root: &std::path::Path,
    beat: &HeartbeatCandidate,
    now: u64,
    inbox: fn(&Database, &Mono) -> Vec<monocode_core::integrations::inbox::Item>,
) -> Result<Option<String>, String> {
    let mono = &beat.mono;
    let state = heartbeat::load(root, &mono.id)?;
    if state.last_run_at == 0 {
        // Enabling heartbeats or a first launch is not inactivity.
        heartbeat::update(root, &mono.id, |s| s.last_run_at = now)?;
        return Ok(None);
    }
    let (rows, sessions, notes) = {
        let db = locked(store)?;
        let chat = mono
            .session_id
            .map(|id| db.mono_session(id))
            .transpose()?
            .flatten();
        let activity = chat.as_ref().map_or(0, |s| s.updated_at);
        if !heartbeat::due(mono.heartbeat_minutes, &state, activity, now) {
            return Ok(None);
        }
        let rows = chat
            .map(|s| db.load(s.id))
            .transpose()?
            .map(|t| t.rows)
            .unwrap_or_default();
        if activity > state.last_run_at {
            heartbeat::update(root, &mono.id, |s| s.quiet_streak = 0)?;
        }
        let all = db.sessions()?;
        let since = state.last_run_at;
        let mut sessions = Vec::new();
        for s in heartbeat::idle_sessions(
            &all,
            &mono.projects,
            |id| beat.skip.contains(&id),
            since,
            now,
        )
        .into_iter()
        .take(5)
        {
            let (user, agent) = heartbeat::last_exchange(&db.load(s.id)?.rows);
            sessions.push(heartbeat::SessionLead {
                id: s.id,
                title: s.title.clone(),
                idle_ms: now.saturating_sub(s.updated_at),
                user,
                agent,
            });
        }
        let notes = heartbeat::changed_notes(&db.list_notes()?, &mono.projects, since, now);
        (rows, sessions, notes)
    };
    let since = state.last_run_at;
    heartbeat::update(root, &mono.id, |s| s.last_run_at = now)?;
    let inbox = heartbeat::changed_inbox(&inbox(store, mono), &mono.projects, since, now);
    let files = mono::load_files(root, mono)?;
    let standing = files
        .soul
        .as_deref()
        .and_then(heartbeat::standing_instructions);
    let digest = heartbeat::digest(sessions, notes, inbox);
    if digest.is_empty() && standing.is_none() {
        return Ok(None);
    }
    let brief = heartbeat::brief(&rows);
    Ok(Some(heartbeat::message(
        standing.as_deref(),
        &digest,
        &state.recent,
        &brief,
        since,
        now,
    )))
}

#[derive(Default)]
pub(super) struct HabitUi {
    saving: bool,
    revision: u64,
    error: Option<String>,
    picker_field: Option<&'static str>,
    expanded: HashSet<(String, u64)>,
}

fn duration(ms: u64) -> String {
    let seconds = (ms.saturating_add(500) / 1000).max(1);
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds.is_multiple_of(60) {
        format!("{}m", seconds / 60)
    } else {
        format!("{}m {}s", seconds / 60, seconds % 60)
    }
}
fn when(at: u64, now: u64) -> String {
    let Some(at) = chrono::DateTime::from_timestamp_millis(at as i64) else {
        return String::new();
    };
    let at = at.with_timezone(&chrono::Local);
    let today = chrono::DateTime::from_timestamp_millis(now as i64)
        .map(|d| d.with_timezone(&chrono::Local).date_naive());
    if Some(at.date_naive()) == today {
        format!("Today {}", at.format("%H:%M"))
    } else if now.saturating_sub(at.timestamp_millis().max(0) as u64) < 6 * 24 * 60 * 60 * 1000 {
        at.format("%a %H:%M").to_string()
    } else {
        at.format("%b %-d, %H:%M").to_string()
    }
}
fn same_day(a: u64, b: u64) -> bool {
    let day = |at| {
        chrono::DateTime::from_timestamp_millis(at as i64)
            .map(|d| d.with_timezone(&chrono::Local).date_naive())
    };
    day(a).is_some() && day(a) == day(b)
}
/// One row per habit, following HabitsList's today/upcoming/done/later order.
fn grouped_habits(
    habits: &[Habit],
    now: u64,
    running: impl Fn(&Habit) -> Option<u64>,
) -> [Vec<usize>; 3] {
    let mut upcoming = Vec::new();
    let mut done = Vec::new();
    let mut later = Vec::new();
    for (ix, h) in habits.iter().enumerate() {
        if let Some(since) = running(h) {
            upcoming.push((0, since, ix));
        } else if h.run_requested || h.enabled && h.next_run_at <= now {
            upcoming.push((1, now, ix));
        } else if h.enabled && same_day(h.next_run_at, now) {
            upcoming.push((2, h.next_run_at, ix));
        } else if let Some(last) = h
            .runs
            .first()
            .map(|r| r.at)
            .or(h.last_run_at)
            .filter(|at| same_day(*at, now))
        {
            done.push((last, ix));
        } else {
            later.push((!h.enabled, h.next_run_at, ix));
        }
    }
    upcoming.sort_by_key(|item| (item.0, item.1));
    done.sort_by_key(|item| item.0);
    later.sort_by_key(|item| (item.0, item.1));
    [
        upcoming.into_iter().map(|item| item.2).collect(),
        done.into_iter().map(|item| item.1).collect(),
        later.into_iter().map(|item| item.2).collect(),
    ]
}
fn section(title: &'static str) -> gpui::Div {
    div()
        .w_full()
        .min_w_0()
        .flex_shrink_0()
        .border_t_1()
        .border_color(content(0.1))
        .px_2()
        .pt_3()
        .pb_4()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .px_2()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(content(0.4))
                .child(title),
        )
}
/// Fixed controls must never be compressed by a large, scrollable draft.
fn habit_field(field: Entity<Input>) -> gpui::Div {
    div()
        .px_2()
        .py_1()
        .rounded_md()
        .hover(|s| s.bg(content(0.05)))
        .w_full()
        .min_w_0()
        .flex_shrink_0()
        .overflow_hidden()
        .child(field)
}
fn habit_icon_button(
    id: SharedString,
    label: &'static str,
    glyph: &'static str,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .flex_shrink_0()
        .size(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .text_color(content(0.5))
        .hover(|s| s.bg(content(0.08)).text_color(content(1.)))
        .child(icon(glyph, 14.))
}
impl Monos {
    pub(super) fn habit_running_since(&self, habit: &Habit) -> Option<u64> {
        self.running
            .values()
            .find(|r| r.habit == habit.id && self.active.as_deref() == Some(r.mono.as_str()))
            .map(|r| r.launched)
            .or(habit.running_since)
    }
    fn habit_busy(&self, habit: &Habit) -> bool {
        self.habit_running_since(habit).is_some()
            || habit.run_requested
            || habit.enabled && habit.next_run_at <= now_ms()
    }
    fn save_habit(&mut self, cx: &mut Context<Self>) {
        if self.habit_ui.saving || self.route != Some(Route::NewHabit) {
            return;
        }
        let Some(mono) = self.active.clone() else {
            return;
        };
        let name = self
            .value("habit-name", cx)
            .trim()
            .chars()
            .take(80)
            .collect();
        let instructions = self
            .value("habit-instructions", cx)
            .trim()
            .chars()
            .take(4000)
            .collect();
        let habit = match Habit::new(name, instructions, self.habit_schedule(cx), now_ms()) {
            Ok(h) => h,
            Err(error) => {
                self.habit_ui.error = Some(error);
                cx.notify();
                return;
            }
        };
        self.habit_ui.saving = true;
        self.habit_ui.error = None;
        let root = self.root.clone();
        let target = mono.clone();
        let revision = self.habit_ui.revision;
        let work = cx.background_executor().spawn(async move {
            habits::update(&root, &target, |all| {
                all.push(habit.clone());
                Ok(())
            })
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.habit_ui.saving = false;
                match result {
                    Ok(()) => {
                        this.reload_files(&mono, cx);
                        if this.active.as_deref() == Some(&mono)
                            && this.route == Some(Route::NewHabit)
                            && this.habit_ui.revision == revision
                        {
                            this.route = None;
                            this.clear_habit_fields();
                        }
                    }
                    Err(error) => {
                        if this.active.as_deref() == Some(&mono)
                            && this.habit_ui.revision == revision
                        {
                            this.habit_ui.error = Some(error);
                        }
                    }
                }
                this.changed(cx);
            });
        })
        .detach();
        cx.notify();
    }
    fn request_habit(&mut self, id: &str, cx: &mut Context<Self>) {
        if self
            .habits
            .iter()
            .find(|h| h.id == id)
            .is_none_or(|h| self.habit_busy(h))
        {
            return;
        }
        let Some(mono) = self.active.clone() else {
            return;
        };
        let root = self.root.clone();
        let target = mono.clone();
        let id = id.to_string();
        if let Some(h) = self.habits.iter_mut().find(|h| h.id == id) {
            h.run_requested = true;
        }
        let work = cx.background_executor().spawn(async move {
            habits::update(&root, &target, |all| {
                all.iter_mut()
                    .find(|h| h.id == id)
                    .ok_or("Habit not found")?
                    .run_requested = true;
                Ok(())
            })
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.error = Some(error);
                }
                this.reload_files(&mono, cx);
                this.check_habits(cx);
                this.changed(cx);
            });
        })
        .detach();
        cx.notify();
    }
    fn remove_habit(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(mono) = self.active.clone() else {
            return;
        };
        let root = self.root.clone();
        let target = mono.clone();
        let id = id.to_string();
        let removed = id.clone();
        let work = cx.background_executor().spawn(async move {
            habits::update(&root, &target, |all| {
                all.retain(|h| h.id != id);
                Ok(())
            })
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.reload_files(&mono, cx);
                        if this.active.as_deref() == Some(&mono)
                            && this.route == Some(Route::Habit(removed.clone()))
                        {
                            this.route = None;
                            this.clear_habit_fields();
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                this.changed(cx);
            });
        })
        .detach();
    }
    /// Monos whose chat is idle with nothing queued; habits run first.
    fn heartbeat_candidates(&self, cx: &App) -> Vec<HeartbeatCandidate> {
        let mut skip: HashSet<i64> = self.roster.hidden_sessions.iter().copied().collect();
        for m in &self.roster.monos {
            skip.extend(m.session_id);
            skip.extend(m.habit_sessions.iter().copied());
        }
        self.roster
            .monos
            .iter()
            .filter(|m| m.heartbeat_minutes > 0 && m.session_id.is_some() && m.outbox.is_empty())
            .filter(|m| !self.chats.get(&m.id).is_some_and(|v| v.read(cx).busy))
            .filter(|m| !self.inflight.contains_key(&m.id))
            .map(|m| HeartbeatCandidate {
                mono: m.clone(),
                skip: skip.clone(),
            })
            .collect()
    }
    /// A quiet heartbeat leaves nothing behind: no hidden session, no history.
    fn discard_quiet_heartbeat(&mut self, mono: &str, session: i64, cx: &mut Context<Self>) {
        if let Some(m) = self.roster.get_mut(mono) {
            m.habit_sessions.retain(|&s| s != session);
        }
        self.roster.hidden_sessions.retain(|&s| s != session);
        self.save(cx);
        let store = self.store.clone();
        cx.background_executor()
            .spawn(async move {
                if let Ok(mut db) = locked(&store) {
                    let _ = db.delete(session);
                }
            })
            .detach();
    }
    pub(super) fn tick(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.check_habits(cx);
        cx.notify();
    }
    fn check_habits(&mut self, cx: &mut Context<Self>) {
        let now = now_ms();
        if let Some(id) = self.active.clone() {
            self.refresh_work(&id, cx);
        }
        self.refresh_cards(cx);
        let waiting_monos = self
            .running
            .values()
            .filter(|r| !self.attention_views(&r.mono, cx).is_empty())
            .map(|r| r.mono.clone())
            .collect::<HashSet<_>>();
        let timed_out: Vec<_> = self
            .running
            .iter_mut()
            .filter_map(|(&id, run)| {
                let waiting = waiting_monos.contains(&run.mono);
                if waiting {
                    let since = *run.waiting_since.get_or_insert(now);
                    (now.saturating_sub(since) > habits::APPROVAL_WAIT_MS).then_some(id)
                } else {
                    if let Some(since) = run.waiting_since.take() {
                        run.started += now.saturating_sub(since);
                    }
                    (now.saturating_sub(run.started) > habits::TIMEOUT_MS).then_some(id)
                }
            })
            .collect();
        for id in timed_out {
            if let Some(run) = self.running.get_mut(&id) {
                run.failure = Some("Habit timed out while waiting for work or an answer".into());
                run.mailbox.clear();
                run.view.update(cx, |v, cx| v.mono_stop(cx));
            }
            let children = self
                .roster
                .monos
                .iter_mut()
                .flat_map(|m| m.launches.iter_mut())
                .filter(|l| (l.origin_turn >> 32) as i64 == id && !l.completed && !l.dismissed)
                .map(|l| {
                    l.dismissed = true;
                    l.session_id
                })
                .collect::<Vec<_>>();
            if let Some(owner) = self.owner.upgrade() {
                for child in children {
                    if let Some(view) = owner.read(cx).mono_session_entity(child) {
                        view.update(cx, |v, cx| v.stop_if_busy(cx));
                    }
                }
            }
            self.habit_changed(id, cx);
        }
        self.check_completions(cx);
        if self.scheduling || self.active_run.is_some() {
            return;
        }
        self.scheduling = true;
        let root = self.root.clone();
        let ids: Vec<_> = self.roster.monos.iter().map(|m| m.id.clone()).collect();
        let beats = self.heartbeat_candidates(cx);
        let store = self.store.clone();
        let work = cx.background_executor().spawn(async move {
            for id in ids {
                let claimed = habits::update(&root, &id, |all| {
                    // A process restart cannot resume an old provider turn.
                    // Record the interruption instead of leaving it claimed forever.
                    for h in all.iter_mut() {
                        if let Some(start) = h.running_since {
                            h.finish(Run {
                                at: start,
                                duration_ms: now.saturating_sub(start),
                                outcome: "interrupted".into(),
                                report: None,
                                error: Some("The app closed before this run finished".into()),
                            })?;
                        }
                        if h.enabled
                            && h.next_run_at < now
                            && now - h.next_run_at > habits::GRACE_MS
                        {
                            h.next_run_at = h.schedule.next(now)?;
                        }
                    }
                    if let Some(h) = all.iter_mut().find(|h| h.due(now)) {
                        h.running_since = Some(now);
                        let manual = std::mem::take(&mut h.run_requested);
                        if !manual || h.next_run_at <= now {
                            h.next_run_at = h.schedule.next(now)?;
                        }
                        return Ok(Some(h.clone()));
                    }
                    Ok(None)
                })?;
                if let Some(h) = claimed {
                    return Ok::<_, String>(Some((id, Claimed::Habit(Box::new(h)))));
                }
            }
            for beat in beats {
                if let Some(text) = prepare_heartbeat(&store, &root, &beat, now, fetch_inbox)? {
                    return Ok(Some((beat.mono.id.clone(), Claimed::Heartbeat(text))));
                }
            }
            Ok(None)
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.scheduling = false;
                match result {
                    Ok(Some((id, Claimed::Habit(habit)))) => this.start_habit(&id, *habit, cx),
                    Ok(Some((id, Claimed::Heartbeat(text)))) => this.start_heartbeat(&id, text, cx),
                    Ok(None) => (),
                    Err(e) => {
                        this.error = Some(e);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
    fn start_habit(&mut self, id: &str, habit: Habit, cx: &mut Context<Self>) {
        let text = format!(
            "[Scheduled habit: {}]\n{}\n\nDo this scheduled work independently. Return a concise report; if nothing requires attention, reply exactly NOTHING_TO_REPORT. Any session you start belongs to this habit. Do not change the habit unless asked.",
            habit.name, habit.instructions
        );
        self.start_run(id, habit.id, habit.name, text, false, cx);
    }
    pub(super) fn start_heartbeat(&mut self, id: &str, text: String, cx: &mut Context<Self>) {
        self.start_run(
            id,
            heartbeat::RUN_ID.into(),
            "Heartbeat".into(),
            text,
            true,
            cx,
        );
    }
    /// Habits and heartbeats run alike: a hidden side session that reports
    /// back into the Mono's chat only when it has something to say.
    fn start_run(
        &mut self,
        id: &str,
        run_id: String,
        name: String,
        text: String,
        triage: bool,
        cx: &mut Context<Self>,
    ) {
        self.active_run = Some(-1);
        let Some(mono) = self.roster.get(id).cloned() else {
            return;
        };
        self.ensure_chat(id, cx);
        let mut inherited = self
            .chats
            .get(id)
            .map(|v| v.read(cx).settings.clone())
            .unwrap_or_default();
        let provider = self
            .chats
            .get(id)
            .map(|v| v.read(cx).summary.provider.clone())
            .unwrap_or_else(|| "codex".into());
        let store = self.store.clone();
        let target = id.to_string();
        let work = cx.background_executor().spawn(async move {
            let db = locked(&store)?;
            let cwd = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            let mut s = db.create(&provider, &cwd)?;
            s.title = format!("{} · {}", mono.label(), name);
            db.title(s.id, &s.title)?;
            drop(db);
            if triage && inherited.usage_limit.is_none() {
                // The catalog is cached for a few minutes; a miss keeps the Mono's model.
                let models = Kind::parse(&provider)
                    .and_then(|kind| {
                        monocode_core::provider::catalog::models_for(kind, &inherited.account).ok()
                    })
                    .unwrap_or_default();
                if let Some((model, effort)) =
                    heartbeat::model(&provider, inherited.model.as_deref(), &models)
                {
                    inherited.model = Some(model);
                    inherited.effort = effort;
                    inherited.model_settings.clear();
                    inherited.fast = false;
                    inherited.service_tier = None;
                }
            }
            Ok::<_, String>((s, mono, inherited))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Err(e) => {
                        this.active_run = None;
                        this.error = Some(e);
                    }
                    Ok((summary, mono, mut settings)) => {
                        let session = summary.id;
                        let view = cx.new(|cx| Conversation::new(summary, this.store.clone(), cx));
                        let root = this.root.clone();
                        let env = this
                            .bridge
                            .as_ref()
                            .map(|b| b.environment(session))
                            .unwrap_or_default();
                        if !triage {
                            settings.access = AccessMode::FullAccess;
                        }
                        settings.internal_message = true;
                        settings.mono = None;
                        settings.environment.clear();
                        view.update(cx, |v, cx| {
                            v.install_transfer(settings, None, cx);
                            v.configure_mono(mono, root, env, cx);
                        });
                        this.observations.push(
                            cx.subscribe(&view, move |this, _, _: &SessionChanged, cx| {
                                this.habit_changed(session, cx)
                            }),
                        );
                        this.running.insert(
                            session,
                            RunningHabit {
                                mono: target.clone(),
                                habit: run_id,
                                view: view.clone(),
                                started: now_ms(),
                                launched: now_ms(),
                                failure: None,
                                waiting_since: None,
                                cards: Vec::new(),
                                mailbox: Vec::new(),
                            },
                        );
                        this.active_run = Some(session);
                        this.roster.hidden_sessions.push(session);
                        if let Some(m) = this.roster.get_mut(&target) {
                            m.habit_sessions.push(session);
                        }
                        this.save(cx);
                        let message = Message {
                            sent_at: Some(monocode_core::sys::time::now_ms()),
                            id: mono::new_id(),
                            text,
                            internal: true,
                            error: None,
                            attachments: Vec::new(),
                        };
                        view.update(cx, |v, cx| v.mono_dispatch(&message, cx));
                    }
                }
                this.changed(cx);
            });
        })
        .detach();
    }
    pub(super) fn habit_changed(&mut self, session: i64, cx: &mut Context<Self>) {
        cx.emit(NotificationChanged);
        let Some(run) = self.running.get(&session) else {
            return;
        };
        if run.view.read(cx).busy {
            self.changed(cx);
            return;
        }
        if let Some(text) = self.running.get_mut(&session).and_then(|r| {
            if r.mailbox.is_empty() {
                None
            } else {
                Some(r.mailbox.remove(0))
            }
        }) {
            let view = self.running[&session].view.clone();
            let message = Message {
                id: mono::new_id(),
                text,
                sent_at: Some(monocode_core::sys::time::now_ms()),
                internal: true,
                error: None,
                attachments: Vec::new(),
            };
            view.update(cx, |v, cx| v.mono_dispatch(&message, cx));
            return;
        }
        if self
            .roster
            .monos
            .iter()
            .flat_map(|m| m.launches.iter())
            .any(|l| {
                (l.origin_turn >> 32) as i64 == session
                    && l.notify_on_complete
                    && !l.completed
                    && !l.dismissed
            })
        {
            self.changed(cx);
            return;
        }
        let run = self.running.remove(&session).unwrap();
        self.active_run = None;
        let completion = {
            let view = run.view.read(cx);
            NotificationFinished {
                summary: view.summary.clone(),
                body: view.notification_reply_body(cx),
                habit: true,
                was_cancelled: view.notification_was_cancelled(),
            }
        };
        cx.emit(completion);
        let (mut ok, mut text) = run.view.read(cx).external_outcome(cx);
        if let Some(error) = run.failure {
            ok = false;
            text = error;
        }
        let report = ok
            .then(|| {
                habits::report(&monocode_core::session::response::final_prose(
                    run.view.read(cx).mono_rows(cx),
                ))
            })
            .flatten();
        let root = self.root.clone();
        let id = run.mono.clone();
        let habit = run.habit.clone();
        let at = run.launched;
        let duration = now_ms().saturating_sub(at);
        let error = (!ok).then_some(text);
        let stored = report.clone();
        let result = Run {
            at,
            duration_ms: duration,
            outcome: if ok {
                if report.is_some() { "posted" } else { "quiet" }
            } else {
                "failed"
            }
            .into(),
            report: stored,
            error,
        };
        let beat = habit == heartbeat::RUN_ID;
        if beat {
            let remembered = report.clone();
            let quiet = ok && report.is_none();
            self.file_work(
                id.clone(),
                move || {
                    heartbeat::update(&root, &id, |state| {
                        if let Some(report) = &remembered {
                            state.remember(report);
                            state.quiet_streak = 0;
                        } else if quiet {
                            state.quiet_streak = state.quiet_streak.saturating_add(1);
                        }
                    })
                },
                cx,
            );
            if ok && report.is_none() && run.cards.is_empty() {
                self.discard_quiet_heartbeat(&run.mono, session, cx);
            }
        } else {
            self.file_work(
                id.clone(),
                move || {
                    habits::update(&root, &id, |all| {
                        if let Some(h) = all.iter_mut().find(|h| h.id == habit) {
                            h.finish(result.clone())?;
                        }
                        Ok(())
                    })
                },
                cx,
            );
        }
        if let Some(report) = report {
            if let Some(m) = self.roster.get_mut(&run.mono) {
                m.pending_reports.push(if beat {
                    report
                } else {
                    format!("**Habit report**\n\n{report}")
                });
                for card in &run.cards {
                    m.pending_reports.push(format!("mono-card:{card}"));
                }
            }
            self.ensure_chat(&run.mono, cx);
            self.save(cx);
            self.dispatch(&run.mono, cx);
        }
        self.changed(cx);
    }
}

#[cfg(test)]
mod tests;
