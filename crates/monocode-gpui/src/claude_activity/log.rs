//! One transcript file, read line by line into what it records: tools,
//! files, skills, to-dos, task notices and work left running.
use super::names::{Usage, message_text, millis, step, tag};
use super::*;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};

/// When something was used: its transcript time, then its line, which orders
/// uses within the same millisecond.
pub(super) type At = (i64, u64);

/// A list that keeps first-seen order, counts repeats and remembers when each
/// item was last used.
#[derive(Default)]
pub(super) struct Tally {
    pub(super) order: Vec<String>,
    pub(super) counts: HashMap<String, u32>,
    pub(super) last: HashMap<String, At>,
}
impl Tally {
    pub(super) fn add(&mut self, key: &str, at: At) {
        self.add_n(key, 1, at);
    }
    pub(super) fn add_n(&mut self, key: &str, n: u32, at: At) {
        match self.counts.get_mut(key) {
            Some(count) => {
                *count += n;
                let last = self.last.get_mut(key).expect("counted keys have a time");
                *last = (*last).max(at);
            }
            None => {
                self.order.push(key.to_owned());
                self.counts.insert(key.to_owned(), n);
                self.last.insert(key.to_owned(), at);
            }
        }
    }
    pub(super) fn merge(&mut self, other: &Tally) {
        for key in &other.order {
            self.add_n(key, other.counts[key], other.last[key]);
        }
    }
    /// The items, most recently used first.
    pub(super) fn recent(&self) -> Vec<String> {
        let mut keys = self.order.clone();
        keys.sort_by_key(|key| std::cmp::Reverse(self.last[key]));
        keys
    }
}

/// A background call, matched to its end by tool id or, once its result
/// names one, task id.
pub(super) struct Task {
    pub(super) tool_id: String,
    pub(super) task_id: String,
    /// A monitor is killed at its deadline, which leaves no record.
    pub(super) until: Option<i64>,
    pub(super) shown: Background,
}

/// One transcript file, read incrementally.
#[derive(Default)]
pub(super) struct Log {
    pub(super) offset: u64,
    pub(super) partial: Vec<u8>,
    pub(super) agent: bool,
    pub(super) first: i64,
    pub(super) last: i64,
    /// Lines read so far, to order uses that share a timestamp.
    pub(super) lines: u64,
    /// Usage per API call; a call's blocks are separate lines that repeat it.
    pub(super) usage: HashMap<String, (String, Usage)>,
    pub(super) model: String,
    /// Whether the last reply ended the turn rather than calling a tool.
    pub(super) ended: bool,
    pub(super) tool_ids: HashSet<String>,
    pub(super) tools: Tally,
    pub(super) files: [Tally; 4],
    /// Write calls by id, with their path and time; the result says whether
    /// the file was new.
    pub(super) writes: Vec<(String, String, At)>,
    pub(super) created: HashSet<String>,
    pub(super) skills: Tally,
    pub(super) context: Tally,
    pub(super) todos: Option<Vec<Todo>>,
    pub(super) steps: Vec<String>,
    /// The latest task notification per agent id: status and when.
    pub(super) notices: HashMap<String, (AgentStatus, i64)>,
    /// The context the last reply was sent with, and the most it reached.
    pub(super) used: u64,
    pub(super) peak: u64,
    pub(super) background: Vec<Task>,
    /// Tool and task ids of background work that ended.
    pub(super) finished: HashSet<String>,
    /// Shell calls in the reply being read: one that outlives its timeout, or
    /// that the user sends away, comes back with a task id.
    pub(super) shells: Vec<(String, Background)>,
    pub(super) reply: String,
}

impl Log {
    pub(super) fn agent() -> Self {
        Self {
            agent: true,
            ..Self::default()
        }
    }
    /// Reads what was appended since the last pull. A file that shrank was
    /// rewritten, so it is read again from the start.
    pub(super) fn pull(&mut self, path: &Path) {
        let Ok(len) = std::fs::metadata(path).map(|m| m.len()) else {
            return;
        };
        if len < self.offset {
            *self = Self {
                agent: self.agent,
                ..Self::default()
            };
        }
        if len == self.offset {
            return;
        }
        let Ok(mut file) = std::fs::File::open(path) else {
            return;
        };
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return;
        }
        // In chunks: only the unfinished line is held between them.
        let mut chunk = vec![0; ((len - self.offset) as usize).clamp(1, 1 << 22)];
        let mut buffer = std::mem::take(&mut self.partial);
        while let Ok(read @ 1..) = file.read(&mut chunk) {
            self.offset += read as u64;
            buffer.extend_from_slice(&chunk[..read]);
            let mut start = 0;
            while let Some(end) = buffer[start..].iter().position(|b| *b == b'\n') {
                if let Ok(line) = std::str::from_utf8(&buffer[start..start + end]) {
                    self.line(line);
                }
                start += end + 1;
            }
            buffer.drain(..start);
        }
        // Every followed transcript keeps its log, so it keeps the unfinished
        // line and not the megabytes of room it was read into.
        buffer.shrink_to_fit();
        self.partial = buffer;
    }

    /// Now, as far as this transcript has been read.
    fn at(&self) -> At {
        (self.last, self.lines)
    }

    pub(super) fn line(&mut self, line: &str) {
        self.lines += 1;
        // Keys follow the message, so the entry's own time is the last one.
        if let Some(at) = line.rfind("\"timestamp\":\"")
            && let Some(time) = line.get(at + 13..at + 13 + 24).and_then(millis)
        {
            if self.first == 0 {
                self.first = time;
            }
            self.last = self.last.max(time);
        }
        const WANTED: [&str; 8] = [
            "\"type\":\"assistant\"",
            "\"type\":\"instructions\"",
            "<task-notification>",
            "<command-message>",
            "\"type\":\"create\"",
            "\"backgroundTaskId\"",
            "\"async_launched\"",
            "\"compact_boundary\"",
        ];
        if !WANTED.iter().any(|marker| line.contains(marker)) {
            return;
        }
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            return;
        };
        match entry["type"].as_str() {
            Some("assistant") => self.reply(&entry["message"]),
            Some("user") => self.user(&entry),
            Some("attachment") if entry["attachment"]["type"] == "instructions" => {
                for file in entry["attachment"]["files"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    if let Some(path) = file["path"].as_str() {
                        // Loaded again: it moves back to the top.
                        self.context.add(path, self.at());
                    }
                }
            }
            // A notice that arrives while Claude works is queued here first.
            Some("attachment") => {
                if let Some(text) = entry["attachment"]["prompt"].as_str() {
                    self.notices(text, self.last);
                }
            }
            Some("queue-operation") => {
                if let Some(text) = entry["content"].as_str() {
                    self.notices(text, self.last);
                }
            }
            Some("system") if entry["subtype"] == "compact_boundary" => {
                if let Some(after) = entry["compactMetadata"]["postTokens"].as_u64() {
                    self.used = after;
                }
            }
            _ => {}
        }
    }

    fn reply(&mut self, message: &Value) {
        let model = message["model"].as_str().unwrap_or_default();
        if model.is_empty() || model.starts_with('<') {
            return;
        }
        self.model = model.to_owned();
        if let Some(used) = monocode_core::context::claude_used(&message["usage"]) {
            self.used = used;
            self.peak = self.peak.max(used);
        }
        if let Some(id) = message["id"].as_str() {
            // Every call of the last reply has its result by now.
            if self.reply != id {
                self.reply = id.to_owned();
                self.shells.clear();
            }
            self.usage.insert(
                id.to_owned(),
                (model.to_owned(), Usage::from(&message["usage"])),
            );
        }
        if !message["stop_reason"].is_null() {
            self.ended = message["stop_reason"] == "end_turn";
        }
        for block in message["content"].as_array().into_iter().flatten() {
            if block["type"] != "tool_use" {
                continue;
            }
            let (Some(id), Some(name)) = (block["id"].as_str(), block["name"].as_str()) else {
                continue;
            };
            if self.tool_ids.insert(id.to_owned()) {
                self.tool(id, name, &block["input"]);
            }
        }
    }

    fn tool(&mut self, id: &str, name: &str, input: &Value) {
        let at = self.at();
        self.tools.add(name, at);
        if self.agent {
            self.steps.push(step(name, input));
        }
        let path = |key: &str| input[key].as_str().filter(|p| !p.is_empty());
        // Agents write the flag as a string.
        let flag = &input["run_in_background"];
        let background = name == "Monitor" || *flag == true || *flag == "true";
        if background || matches!(name, "Bash" | "PowerShell") {
            let text = |key: &str| input[key].as_str().filter(|t| !t.trim().is_empty());
            let detail = text("command").or(text("description")).unwrap_or_default();
            let shown = Background {
                tool: name.to_owned(),
                detail: detail.trim().lines().next().unwrap_or_default().to_owned(),
                started: at.0,
            };
            if background {
                self.background.push(Task {
                    tool_id: id.to_owned(),
                    task_id: String::new(),
                    until: (name == "Monitor").then(|| {
                        at.0 + input["timeout_ms"]
                            .as_i64()
                            .unwrap_or(300_000)
                            .min(1_800_000)
                    }),
                    shown,
                });
            } else {
                self.shells.push((id.to_owned(), shown));
            }
        }
        match name {
            "Read" => path("file_path")
                .into_iter()
                .for_each(|p| self.files[0].add(p, at)),
            "Edit" | "MultiEdit" => path("file_path")
                .into_iter()
                .for_each(|p| self.files[1].add(p, at)),
            "NotebookEdit" => path("notebook_path")
                .into_iter()
                .for_each(|p| self.files[1].add(p, at)),
            "Write" => {
                if let Some(path) = path("file_path") {
                    self.writes.push((id.to_owned(), path.to_owned(), at));
                }
            }
            // A stop leaves no notice behind.
            "TaskStop" | "KillShell" | "KillBash" => {
                if let Some(task) = input["task_id"].as_str().or(input["shell_id"].as_str()) {
                    self.finished.insert(task.to_owned());
                }
            }
            "Skill" => {
                if let Some(skill) = input["skill"].as_str() {
                    self.skills.add(skill.trim_start_matches('/'), at);
                }
            }
            "TodoWrite" => {
                let todos = input["todos"].as_array().into_iter().flatten();
                self.todos = Some(
                    todos
                        .filter_map(|todo| {
                            Some(Todo {
                                text: todo["content"].as_str()?.to_owned(),
                                status: todo_status(todo["status"].as_str()),
                            })
                        })
                        .collect(),
                );
            }
            _ => {}
        }
    }

    fn user(&mut self, entry: &Value) {
        let result = &entry["toolUseResult"];
        if let Some(id) = entry["message"]["content"][0]["tool_use_id"].as_str() {
            if result["type"] == "create" {
                self.created.insert(id.to_owned());
            }
            let task = result["backgroundTaskId"]
                .as_str()
                .or(result["agentId"].as_str());
            if let Some(task) = task {
                if let Some(call) = self.background.iter_mut().find(|t| t.tool_id == id) {
                    call.task_id = task.to_owned();
                } else if let Some(ix) = self.shells.iter().position(|(tool, _)| tool == id) {
                    let (tool_id, shown) = self.shells.swap_remove(ix);
                    self.background.push(Task {
                        tool_id,
                        task_id: task.to_owned(),
                        until: None,
                        shown,
                    });
                }
            }
        }
        let text = message_text(&entry["message"]["content"]);
        // A skill typed as a slash command names its message first; built-in
        // commands such as /clear name the command first.
        // shortcut: tag order is Claude Code's own; recheck if typed skills vanish.
        if text.trim_start().starts_with("<command-message>")
            && let Some(name) = tag(&text, "command-name")
        {
            let at = self.at();
            self.skills.add(name.trim_start_matches('/'), at);
        }
        let time = entry["timestamp"]
            .as_str()
            .and_then(millis)
            .unwrap_or(self.last);
        self.notices(&text, time);
    }

    /// Every status a notice carries is an end: completed, failed, killed or
    /// stopped.
    fn notices(&mut self, text: &str, time: i64) {
        for notice in text.split("<task-notification>").skip(1) {
            let (Some(task), Some(status)) = (tag(notice, "task-id"), tag(notice, "status")) else {
                continue;
            };
            let ids = [Some(task), tag(notice, "tool-use-id")];
            self.finished.extend(
                ids.into_iter()
                    .flatten()
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned),
            );
            let status = match status {
                "completed" => AgentStatus::Done,
                "failed" => AgentStatus::Failed,
                _ => AgentStatus::Stopped,
            };
            self.notices.insert(task.to_owned(), (status, time));
        }
    }

    pub(super) fn tokens(&self) -> u64 {
        self.usage.values().map(|(_, usage)| usage.tokens()).sum()
    }
    pub(super) fn cost(&self) -> f64 {
        self.usage
            .values()
            .map(|(model, usage)| usage.cost(model))
            .sum()
    }
}

pub(super) fn todo_status(status: Option<&str>) -> TodoStatus {
    match status {
        Some("completed") => TodoStatus::Done,
        Some("in_progress") => TodoStatus::Active,
        _ => TodoStatus::Pending,
    }
}
