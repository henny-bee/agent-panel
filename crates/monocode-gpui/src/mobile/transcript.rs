//! The phone's transcript (mobileTranscript.ts): turns the desktop has
//! already grouped and folded as `TranscriptView` does, so the phone and the
//! desktop follow one set of rules. Pure: rows and turn provenance in, the
//! phone's `Turn` JSON (mobile/src/features/workspace/types.ts) out.
//!
//! Known gaps against the React port, because native rows do not carry them:
//! approvals are not persisted, so no "Waiting for approval on desktop" notice
//! (the live header still says `waiting`); there are no Mono habits, operator
//! prompts or background-task labels; and assignment wrappers stay in user
//! text, as the desktop shows them.
use crate::chat::transcript::{self as desk, attachments, dates, phases};
use monocode_core::session::{
    agent::TurnInfo,
    tool::{
        ToolCall, ToolState,
        tasks::{Panel, TaskStatus},
    },
    transcript::{Interjection, Row, attachments::ROLE as ATTACHMENTS},
};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

const MAX_STEPS: usize = 40;
const MAX_MESSAGES: usize = 200;

pub(crate) struct Source<'a> {
    pub rows: &'a [Row],
    /// Provenance by turn: the first row after the turn's prompt block.
    pub turns: &'a HashMap<usize, TurnInfo>,
    pub cwd: &'a str,
    /// Provider id, e.g. "claude".
    pub harness: &'a str,
    /// The current model's display name.
    pub model_name: Option<String>,
    pub busy: bool,
    /// "Waiting for approval" or "Waiting for answers", for the live header.
    pub waiting: Option<&'a str>,
    /// The Mono's display name, for a Mono conversation (inline work).
    pub mono: Option<&'a str>,
    /// Approvals and questions the live chat waits on. Rows never record
    /// them, so they come from the conversation and end the latest turn.
    pub pending: &'a [Pending],
    /// The sessions a Mono started, as the phone's cards show them, by
    /// session: where each runs and how it stands. Only these get cards.
    pub launches: &'a HashMap<i64, Value>,
}

/// An approval or question waiting on the desktop.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Pending {
    pub id: String,
    pub question: bool,
    pub label: String,
}

/// One prompt block and the response under it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Run {
    /// The prompt block's first row.
    prompt: usize,
    /// The first row after the prompt block: the `turns` key.
    key: usize,
    end: usize,
}

/// A turn as the phone shows it: one run, or a Mono's run and the completion
/// runs that continue it (groupMonoChatTurns).
struct Turn {
    runs: Vec<Run>,
}
impl Turn {
    fn id(&self) -> String {
        format!("t{}", self.runs[0].prompt)
    }
}

fn message_end(rows: &[Row], at: usize) -> usize {
    (at + 1..rows.len())
        .find(|&i| rows[i].first)
        .unwrap_or(rows.len())
}
fn joined(rows: &[Row]) -> String {
    rows.iter().map(|r| r.text.as_str()).collect()
}
fn readable(rows: &[Row]) -> bool {
    rows.iter().any(|r| !r.text.trim().is_empty())
}
/// Each message's first row and end, in order.
fn messages_in(rows: &[Row], from: usize, to: usize) -> impl Iterator<Item = (usize, usize)> {
    let mut at = from;
    std::iter::from_fn(move || {
        (at < to).then(|| {
            let end = message_end(rows, at).min(to);
            let message = (at, end);
            at = end;
            message
        })
    })
}

fn runs(rows: &[Row]) -> Vec<Run> {
    let next_prompt = |from: usize| {
        (from..rows.len())
            .find(|&i| rows[i].is_prompt())
            .unwrap_or(rows.len())
    };
    let mut out = Vec::new();
    let mut at = 0;
    if rows.first().is_some_and(|r| !r.is_prompt()) {
        at = next_prompt(0);
        out.push(Run {
            prompt: 0,
            key: 0,
            end: at,
        });
    }
    while at < rows.len() {
        let key = (at..rows.len())
            .find(|&i| !rows[i].is_prompt())
            .unwrap_or(rows.len());
        let end = next_prompt(key);
        out.push(Run {
            prompt: at,
            key,
            end,
        });
        at = end;
    }
    out
}

fn turns(source: &Source) -> Vec<Turn> {
    let mut out: Vec<Turn> = Vec::new();
    for run in runs(source.rows) {
        let continues = source.mono.is_some()
            && !out.is_empty()
            && desk::mono_continues(source.rows, run.key).is_some();
        match out.last_mut() {
            Some(turn) if continues => turn.runs.push(run),
            _ => out.push(Turn { runs: vec![run] }),
        }
    }
    out
}

/// Every turn's stable ID in order. An ID is its first row's index, which
/// appends never move.
pub(crate) fn turn_ids(source: &Source) -> Vec<String> {
    turns(source).iter().map(Turn::id).collect()
}

/// turns[range] as the phone's `Turn` objects.
pub(crate) fn build(source: &Source, range: std::ops::Range<usize>) -> Vec<Value> {
    let turns = turns(source);
    let end = range.end.min(turns.len());
    let start = range.start.min(end);
    (start..end)
        .map(|index| {
            // A run that has finished stays settled while the desktop is busy
            // taking the next message in, before that message has a row; else
            // its reply would fold back into the live status for a moment.
            let finished = turns[index]
                .runs
                .last()
                .and_then(|&run| info(source, run))
                .is_some_and(|info| info.completed_at.is_some());
            let live = source.busy && index + 1 == turns.len() && !finished;
            let mut turn = if let Some(name) = source.mono {
                mono_turn(source, &turns[index], live, name)
            } else {
                agent_turn(source, &turns[index], live)
            };
            if index + 1 == turns.len()
                && let Some(items) = turn["items"].as_array_mut()
            {
                items.extend(source.pending.iter().map(|pending| {
                    let what = if pending.question {
                        "Question waiting on desktop"
                    } else {
                        "Waiting for approval on desktop"
                    };
                    json!({
                        "type": "notice",
                        "id": format!("input:{}", pending.id),
                        "text": format!("{what}: {}", pending.label),
                    })
                }));
            }
            turn
        })
        .collect()
}

/// mobileMessages: the last 200 user and assistant messages.
pub(crate) fn messages(rows: &[Row]) -> Vec<Value> {
    let mut out: Vec<Value> = messages_in(rows, 0, rows.len())
        .filter(|&(at, _)| matches!(rows[at].role.as_str(), "user" | "assistant"))
        .map(|(at, end)| json!({"id": format!("r{at}"), "role": rows[at].role, "text": joined(&rows[at..end])}))
        .collect();
    out.drain(..out.len().saturating_sub(MAX_MESSAGES));
    out
}

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}
fn put(map: &mut Map<String, Value>, key: &str, value: Option<impl Into<Value>>) {
    if let Some(value) = value {
        map.insert(key.into(), value.into());
    }
}

/// The user bubbles of a prompt block; `mono-system` prompts are the app's.
fn user_items(rows: &[Row], run: Run, out: &mut Vec<Value>, by_row: &mut HashMap<usize, usize>) {
    for (at, end) in messages_in(rows, run.prompt, run.key) {
        if rows[at].role != "user" {
            continue;
        }
        let raw = joined(&rows[at..end]);
        let (shown, files) = attachments::message(rows, end, &raw);
        let text = shown.unwrap_or(raw).trim_end().to_owned();
        if text.trim().is_empty() && files.is_empty() {
            continue;
        }
        let mut item = object(json!({"type": "user", "id": format!("r{at}"), "text": text}));
        put(&mut item, "at", rows[at].sent_at);
        if !text.trim().is_empty() && desk::is_emoji_only(&text) {
            item.insert("emojiOnly".into(), true.into());
        }
        if !files.is_empty() {
            item.insert("attachments".into(), files.len().into());
        }
        by_row.insert(at, out.len());
        out.push(Value::Object(item));
    }
}

/// Messages stacked one after another share the last one's time.
fn compact(items: &mut [Value]) {
    for at in 1..items.len() {
        if items[at]["type"] == "user" && items[at - 1]["type"] == "user" {
            items[at - 1]["compact"] = true.into();
        }
    }
}

/// A card the Mono put in its chat, as the phone shows it: suggested replies,
/// checked once one is sent, or a document. Other cards stay on the desktop.
fn mono_card(rows: &[Row], at: usize, end: usize) -> Option<Value> {
    let value: Value = serde_json::from_str(&joined(&rows[at..end])).ok()?;
    match value["type"].as_str()? {
        "choices" => {
            let options: Vec<&str> = value["options"]
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .collect();
            if options.is_empty() {
                return None;
            }
            let mut item =
                object(json!({"type": "choices", "id": format!("r{at}"), "options": options}));
            // The reply sent after it answers it, as on the desktop.
            let reply = messages_in(rows, end, rows.len())
                .find(|&(at, _)| rows[at].role == "user")
                .map(|(at, end)| joined(&rows[at..end]));
            put(&mut item, "chosen", reply);
            Some(Value::Object(item))
        }
        "document" => document_item(&value, &format!("r{at}")),
        _ => None,
    }
}
/// ArtifactCard: a document the Mono wrote, opened in the phone's reader.
fn document_item(value: &Value, id: &str) -> Option<Value> {
    Some(json!({
        "type": "document",
        "id": id,
        "documentId": value["documentId"].as_str()?,
        "title": value["title"].as_str().unwrap_or("Document"),
        "kind": value["kind"].as_str().unwrap_or("document"),
    }))
}
/// The documents a run wrote, under its reply once it settles, each once at
/// its latest title, as the desktop's chat lists them.
fn run_documents(rows: &[Row], run: Run) -> Vec<Value> {
    let mut documents: Vec<Value> = Vec::new();
    for row in &rows[run.prompt..run.end] {
        if row.role == "mono-document"
            && let Ok(value) = serde_json::from_str::<Value>(&row.text)
        {
            match documents
                .iter_mut()
                .find(|old| old["documentId"] == value["documentId"])
            {
                Some(old) => *old = value,
                None => documents.push(value),
            }
        }
    }
    documents
        .iter()
        .filter_map(|value| {
            let id = format!("doc:{}", value["documentId"].as_str()?);
            document_item(value, &id)
        })
        .collect()
}

fn notice(rows: &[Row], at: usize, end: usize) -> Option<Value> {
    let text = joined(&rows[at..end]);
    let id = format!("r{at}");
    match rows[at].role.as_str() {
        // Older native versions saved a user's stop as an error; the desktop hides it.
        "error" if text.trim() == "Stopped" => None,
        "error" if !text.trim().is_empty() => {
            Some(json!({"type": "notice", "id": id, "text": text, "error": true}))
        }
        "interjection" => serde_json::from_str::<Interjection>(&text)
            .ok()
            .filter(|note| !note.text.trim().is_empty())
            .map(|note| json!({"type": "notice", "id": id, "text": note.text})),
        "visual" => {
            let document =
                serde_json::from_str::<monocode_core::session::visual::VisualDocument>(&text)
                    .ok()
                    .filter(|document| document.validate().is_ok());
            // Drawn inline on the phone from its first render; its source and
            // its handlers stay here.
            if let Some(document) = &document
                && let Some(tree) = visual_tree(&document.source)
            {
                return Some(
                    json!({"type": "visual", "id": id, "title": document.title, "tree": tree}),
                );
            }
            let title = document
                .map(|document| document.title)
                .unwrap_or_else(|| "Interactive visual".into());
            Some(
                json!({"type": "notice", "id": id, "text": format!("{title} · Open on desktop to interact")}),
            )
        }
        "system" if !text.trim().is_empty() => {
            Some(json!({"type": "notice", "id": id, "text": text}))
        }
        _ => None,
    }
}

/// A visual's first render, kept by its source: the phone's transcript is
/// rebuilt on every push while a turn streams, and a render runs script.
fn visual_tree(source: &str) -> Option<Value> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::sync::{LazyLock, Mutex};
    static RENDERED: LazyLock<Mutex<HashMap<u64, Option<Value>>>> = LazyLock::new(Default::default);
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    let key = hasher.finish();
    if let Some(tree) = RENDERED.lock().ok()?.get(&key) {
        return tree.clone();
    }
    let tree = monocode_core::session::visual::VisualRuntime::new(source)
        .and_then(|mut runtime| runtime.render())
        .ok()
        .and_then(|tree| serde_json::to_value(tree).ok());
    let mut rendered = RENDERED.lock().ok()?;
    if rendered.len() > 64 {
        rendered.clear();
    }
    rendered.insert(key, tree.clone());
    tree
}

fn tool_label(call: &ToolCall, cwd: &str) -> String {
    let shown = call.presentation_in(Some(cwd));
    let label = [shown.action, shown.target]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if label.is_empty() {
        "Working".into()
    } else {
        label
    }
}

/// toolCategory, by the action `activity::summary` sorts calls by.
fn work_kind(call: &ToolCall) -> &'static str {
    match call.summary().0.as_str() {
        "Read" | "List" | "Search" | "Find" => "research",
        "Edit" | "Write" => "edit",
        "Run" => "run",
        "Agent" => "agent",
        _ => "other",
    }
}

/// dominantCountedWorkKind: ties go to the kind that changed the most.
fn dominant_kind(calls: &[ToolCall]) -> &'static str {
    let mut best = ("think", 0);
    for kind in ["edit", "run", "agent", "research", "other"] {
        let count = calls.iter().filter(|call| work_kind(call) == kind).count();
        if count > best.1 {
            best = (kind, count);
        }
    }
    best.0
}

/// One phase as the desktop groups it (`phases::collect`), titled as its
/// header is (`activity::summary`). A lone step is its own row, not a group.
fn phase(
    source: &Source,
    activity: &phases::Phases,
    group: &phases::Group,
    active: bool,
    budget: &mut usize,
) -> Value {
    let rows = source.rows;
    let mut steps = Vec::new();
    for (at, end) in messages_in(rows, group.start, group.last + 1) {
        if activity.at(at).is_none_or(|g| g.start != group.start) {
            continue;
        }
        let text = joined(&rows[at..end]);
        let step = if rows[at].role == "tool" {
            let Ok(call) = serde_json::from_str::<ToolCall>(&text) else {
                continue;
            };
            let mut step =
                object(json!({"id": format!("r{at}"), "label": tool_label(&call, source.cwd)}));
            if matches!(call.state, ToolState::Failed | ToolState::Cancelled) {
                step.insert("failed".into(), true.into());
            }
            step
        } else {
            let summary = desk::tasks::prose_summary(&text);
            object(json!({
                "id": format!("r{at}"),
                "label": if summary.is_empty() { "Thinking".into() } else { summary },
            }))
        };
        steps.push(Value::Object(step));
    }
    let id = format!("p{}", group.start);
    let kind = dominant_kind(&group.calls);
    if steps.len() == 1 {
        let title = steps[0]["label"].clone();
        *budget = budget.saturating_sub(1);
        return json!({"id": id, "kind": kind, "title": title, "steps": []});
    }
    let title = if group.calls.is_empty() {
        if active { "Thinking…" } else { "Thought" }.to_owned()
    } else {
        crate::chat::transcript::activity::summary(&group.calls, active).0
    };
    let keep = steps.len().min(*budget);
    *budget -= keep;
    steps.drain(..steps.len() - keep);
    json!({"id": id, "kind": kind, "title": title, "steps": steps})
}

/// A standalone call: a delegated run pinned outside the work.
fn lone_phase(source: &Source, at: usize, call: &ToolCall) -> Value {
    json!({
        "id": format!("p{at}"),
        "kind": work_kind(call),
        "title": tool_label(call, source.cwd),
        "steps": [],
    })
}

fn task_list(at: usize, call: &ToolCall) -> Option<Value> {
    let Some(Panel::Tasks(list)) = call.panel() else {
        return None;
    };
    (!list.items.is_empty()).then(|| {
        let text = list
            .items
            .iter()
            .map(|task| match task.status {
                TaskStatus::Completed => format!("- ~~{}~~", task.text),
                _ => format!("- {}", task.text),
            })
            .collect::<Vec<_>>()
            .join("\n");
        json!({"type": "text", "id": format!("r{at}"), "text": text})
    })
}

fn info<'a>(source: &'a Source, run: Run) -> Option<&'a TurnInfo> {
    source.turns.get(&run.key)
}
fn duration(source: &Source, run: Run) -> Option<u64> {
    info(source, run)
        .filter(|info| info.has_duration())
        .map(|info| info.duration_ms)
}
/// When the run started: its provenance, else the latest prompt it was sent with.
fn started_at(source: &Source, run: Run) -> Option<u64> {
    info(source, run)
        .and_then(|info| info.started_at)
        .or_else(|| {
            source.rows[run.prompt..run.key]
                .iter()
                .rev()
                .find_map(|row| row.sent_at.filter(|_| row.first))
        })
}
fn harness(source: &Source, run: Run) -> String {
    info(source, run)
        .map(|info| info.provider.trim())
        .filter(|provider| !provider.is_empty())
        .unwrap_or(source.harness)
        .to_owned()
}

/// A turn as `TranscriptView::rebuild` lays it out for an agent session: the
/// prompt, the work line, then the response with its work folded behind the
/// line once an answer follows it.
fn agent_turn(source: &Source, turn: &Turn, live: bool) -> Value {
    let rows = source.rows;
    let run = turn.runs[0];
    let mut items = Vec::new();
    user_items(rows, run, &mut items, &mut HashMap::new());
    let header_at = items.len();

    let tail = run.end == rows.len();
    let activity = phases::collect(&rows[..run.end], run.key, live && tail);
    let answer = (run.key..run.end)
        .rev()
        .find(|&i| rows[i].role == "assistant" && rows[i].first);
    let ready =
        answer.is_some_and(|answer| !live || readable(&rows[answer..message_end(rows, answer)]));
    let fold =
        answer.filter(|&answer| ready && rows[run.key..answer].iter().any(|r| r.role == "tool"));
    let active = |group: &phases::Group| {
        live && (group.end == rows.len() || (answer == Some(group.end) && !ready))
    };

    let mut entries = Vec::new();
    let mut fold_budget = MAX_STEPS;
    let mut work: Option<(usize, Vec<Value>, bool, usize)> = None;
    let mut responded = false;
    let close = |work: &mut Option<(usize, Vec<Value>, bool, usize)>, items: &mut Vec<Value>| {
        if let Some((at, phases, live, _)) = work.take() {
            items.push(
                json!({"type": "work", "id": format!("w{at}"), "live": live, "phases": phases}),
            );
        }
    };
    for (at, end) in messages_in(rows, run.key, run.end) {
        let role = rows[at].role.as_str();
        if matches!(
            role,
            "mono-card" | "mono-document" | "mono-note" | "mono-launch"
        ) || role == ATTACHMENTS
            || activity.hidden.contains(&at)
        {
            continue;
        }
        responded = true;
        if role == "visual" {
            close(&mut work, &mut items);
            if let Some(notice) = notice(rows, at, end) {
                items.push(notice);
            }
            continue;
        }
        let group = activity.at(at);
        if fold.is_some_and(|answer| at < answer) && !activity.pinned.contains(&at) {
            if let Some(group) = group.filter(|g| g.start == at) {
                let mut entry = object(phase(source, &activity, group, false, &mut fold_budget));
                entry.insert("type".into(), "phase".into());
                entries.push(Value::Object(entry));
            } else if role == "assistant" && readable(&rows[at..end]) {
                entries.push(json!({"type": "prose", "id": format!("r{at}"), "text": joined(&rows[at..end])}));
            }
            continue;
        }
        if let Some(group) = group {
            if group.start == at {
                let active = active(group);
                let (_, phases, live, budget) =
                    work.get_or_insert_with(|| (at, Vec::new(), false, MAX_STEPS));
                phases.push(phase(source, &activity, group, active, budget));
                *live |= active;
            }
            continue;
        }
        if role == "tool" {
            let Ok(call) = serde_json::from_str::<ToolCall>(&joined(&rows[at..end])) else {
                continue;
            };
            if let Some(list) = task_list(at, &call) {
                close(&mut work, &mut items);
                items.push(list);
            } else if !matches!(call.panel(), Some(Panel::Tasks(_))) {
                let (_, phases, _, _) =
                    work.get_or_insert_with(|| (at, Vec::new(), false, MAX_STEPS));
                phases.push(lone_phase(source, at, &call));
            }
            continue;
        }
        close(&mut work, &mut items);
        if role == "assistant" {
            if readable(&rows[at..end]) {
                let mut item = object(
                    json!({"type": "text", "id": format!("r{at}"), "text": joined(&rows[at..end])}),
                );
                if live && end == rows.len() {
                    item.insert("streaming".into(), true.into());
                }
                items.push(Value::Object(item));
            }
        } else if let Some(notice) = notice(rows, at, end) {
            items.push(notice);
        }
    }
    close(&mut work, &mut items);

    let info = info(source, run);
    let started = started_at(source, run);
    let duration = duration(source, run).filter(|_| !live);
    if live || responded {
        let model = info
            .map(|info| info.model.trim().to_owned())
            .filter(|model| !model.is_empty())
            .or_else(|| source.model_name.clone().filter(|_| live));
        let mut header = object(json!({
            "type": "header",
            "id": format!("{}:header", turn.id()),
            "live": live,
            "harness": harness(source, run),
            "entries": entries,
        }));
        put(&mut header, "startedAt", started);
        put(&mut header, "durationMs", duration);
        put(&mut header, "model", model);
        if live {
            put(&mut header, "waiting", source.waiting);
        }
        items.insert(header_at, Value::Object(header));
    }
    compact(&mut items);

    let mut out = object(json!({"id": turn.id(), "items": items}));
    if !live && responded {
        out.insert("footer".into(), true.into());
        put(
            &mut out,
            "completedAt",
            info.and_then(|info| info.completed_at)
                .or_else(|| started.map(|at| at + duration.unwrap_or(0))),
        );
        out.insert(
            "copyText".into(),
            monocode_core::session::response::prose(&rows[run.prompt..run.end]).into(),
        );
        put(&mut out, "durationMs", duration);
        if let Some(metrics) = info.and_then(|info| info.metrics).filter(|m| m.reported()) {
            let mut value = object(json!({
                "inputTokens": metrics.input_tokens,
                "outputTokens": metrics.output_tokens,
                "cacheReadTokens": metrics.cache_read_tokens,
                "cacheWriteTokens": metrics.cache_write_tokens,
            }));
            put(&mut value, "cacheHitPercent", metrics.cache_hit_percent);
            out.insert("metrics".into(), Value::Object(value));
        }
    }
    Value::Object(out)
}

fn reaction(text: &str) -> Option<&str> {
    text.trim()
        .strip_prefix("<reaction>")
        .and_then(|s| s.strip_suffix("</reaction>"))
        .filter(|emoji| emoji.chars().count() <= 16)
}

/// A Mono's turn, as its chat shows it: the user's messages, the Mono's
/// signature line, and the prose it settled on; its process stays in the
/// signature's live status.
fn mono_turn(source: &Source, turn: &Turn, live: bool, name: &str) -> Value {
    let rows = source.rows;
    let mut items = Vec::new();
    let mut by_row = HashMap::new();
    let mut reacted = false;
    let mut standalone = true;
    let mut copy = Vec::new();
    for &run in &turn.runs {
        let before = items.len();
        user_items(rows, run, &mut items, &mut by_row);
        standalone &= items.len() == before;
        let run_live = live && run.end == rows.len();
        let internal = run.key > run.prompt && rows[run.key - 1].role == "mono-system";
        for (at, end) in messages_in(rows, run.key, run.end) {
            let role = rows[at].role.as_str();
            let text = joined(&rows[at..end]);
            if role == "assistant" {
                if let Some(emoji) = reaction(&text) {
                    // MonoReactionBadge: the emoji sits on the message it answers.
                    let prompt = rows[..at]
                        .iter()
                        .rposition(Row::is_prompt)
                        .and_then(|p| rows[..=p].iter().rposition(|r| r.first));
                    if let Some(&item) = prompt.and_then(|p| by_row.get(&p)) {
                        items[item]["reaction"] = emoji.into();
                        reacted = true;
                    }
                    continue;
                }
                if !readable(&rows[at..end]) {
                    continue;
                }
                // groupMonoTurnItems (mono_prose_hidden on the desktop): a
                // direct reply keeps its opening sentence once it's whole;
                // other prose said while working stays in the activity
                // trail, as does all of it while the run is live.
                let opening = !internal
                    && rows[run.key..at]
                        .iter()
                        .all(|r| !r.first || r.role == "reasoning");
                let hidden = if opening {
                    run_live && end == rows.len()
                } else {
                    run_live
                        || rows[at + 1..run.end].iter().any(|r| {
                            r.first
                                && (matches!(r.role.as_str(), "tool" | "reasoning")
                                    || (internal && r.role == "assistant"))
                        })
                };
                if hidden {
                    continue;
                }
                let mut item =
                    object(json!({"type": "text", "id": format!("r{at}"), "text": text}));
                if live && end == rows.len() {
                    item.insert("streaming".into(), true.into());
                }
                copy.push(text.replace("\r\n", "\n").trim().to_owned());
                items.push(Value::Object(item));
            } else if matches!(role, "mono-card" | "mono-note") {
                items.extend(mono_card(rows, at, end));
            } else if let Some(notice) = notice(rows, at, end) {
                items.push(notice);
            }
        }
        if !run_live {
            items.extend(run_documents(rows, run));
        }
        // LaunchedSessionCard: the sessions the run started, under its
        // reply once it settles, as the desktop's chat shows them.
        if !run_live {
            let sessions: Vec<Value> = desk::mono::run_launches(rows, run.prompt, run.end, |id| {
                source.launches.contains_key(&id)
            })
            .into_iter()
            .filter_map(|id| source.launches.get(&id).cloned())
            .collect();
            if !sessions.is_empty() {
                items.push(json!({
                    "type": "sessions",
                    "id": format!("l{}", run.prompt),
                    "sessions": sessions,
                }));
            }
        }
    }
    let has_prose = turn.runs.iter().any(|run| {
        messages_in(rows, run.key, run.end).any(|(at, end)| {
            rows[at].role == "assistant"
                && readable(&rows[at..end])
                && reaction(&joined(&rows[at..end])).is_none()
        })
    });
    let standalone = standalone && has_prose;
    let first = turn.runs[0];
    let last = *turn.runs.last().unwrap();
    let durations: Vec<u64> = turn
        .runs
        .iter()
        .filter_map(|&run| duration(source, run))
        .collect();
    let duration = (!durations.is_empty()).then(|| durations.iter().sum::<u64>());
    let started = started_at(source, first);
    let first_work = items.iter().position(|item| item["type"] != "user");
    let reacted_only = !live && first_work.is_none() && reacted;
    if !reacted_only && (standalone || live || duration.is_some() || first_work.is_some()) {
        let label = if live && !standalone {
            source
                .waiting
                .map(|waiting| format!("{waiting}…"))
                .unwrap_or_else(|| desk::mono::mono_work_label(rows, first.key))
        } else {
            name.to_owned()
        };
        let mut header = object(json!({
            "type": "header",
            "id": format!("{}:header", turn.id()),
            "live": live,
            "label": label,
            "model": name,
            "harness": harness(source, first),
            "entries": [],
        }));
        put(&mut header, "startedAt", started);
        put(&mut header, "durationMs", duration);
        // While it works, its status trails the turn, under anything it has
        // already posted, as a messaging app's typing line.
        let at = if live {
            items.len()
        } else {
            first_work.unwrap_or(items.len())
        };
        items.insert(at, Value::Object(header));
    }
    compact(&mut items);

    // A user opens its stretch above its bubble; a run without one, above the
    // Mono's header.
    let stamp = (first.prompt..first.key)
        .find(|&i| rows[i].first && rows[i].role == "user")
        .unwrap_or(first.prompt);
    let stamp_at = rows
        .get(stamp)
        .filter(|_| dates::mono_opens_stretch(rows, stamp))
        .and_then(|row| row.sent_at);
    let mut out = object(json!({"id": turn.id(), "items": items}));
    put(&mut out, "stampAt", stamp_at);
    if !live && !reacted_only && (duration.is_some() || standalone || first_work.is_some()) {
        let completed = info(source, last)
            .and_then(|info| info.completed_at)
            .or_else(|| {
                if turn.runs.len() > 1 {
                    started_at(source, last).map(|at| at + duration_or_zero(source, last))
                } else {
                    started.map(|at| at + duration.unwrap_or(0))
                }
            });
        out.insert("footer".into(), true.into());
        put(&mut out, "completedAt", completed);
        out.insert(
            "copyText".into(),
            copy.into_iter()
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n")
                .into(),
        );
        put(&mut out, "durationMs", duration);
    }
    Value::Object(out)
}
fn duration_or_zero(source: &Source, run: Run) -> u64 {
    duration(source, run).unwrap_or(0)
}

#[cfg(test)]
mod tests;
