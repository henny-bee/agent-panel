//! What a Claude Code session did, read from the files the CLI writes: its
//! transcript, one transcript per subagent beside it, and its task list.
//! Transcripts grow to hundreds of megabytes, mostly tool output, so each file
//! is read from where the last read stopped and only lines that can matter are
//! parsed.
use monocode_core::{accounts::Profile, provider::Kind};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// An agent with no end on record and no writes for this long is taken to
/// have died with its session.
const STALE_MS: i64 = 15 * 60 * 1000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AgentStatus {
    #[default]
    Running,
    Done,
    Stopped,
    Failed,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Agent {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub model: String,
    pub tokens: u64,
    pub started: i64,
    pub last: i64,
    pub status: AgentStatus,
    /// Every tool call, in plain words.
    pub steps: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TodoStatus {
    Pending,
    Active,
    Done,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Todo {
    pub text: String,
    pub status: TodoStatus,
}

/// Files touched through Claude's file tools. Shell commands are not seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileKind {
    Read,
    Edited,
    Created,
    Written,
}
pub(crate) const FILE_KINDS: [FileKind; 4] = [
    FileKind::Read,
    FileKind::Edited,
    FileKind::Created,
    FileKind::Written,
];

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Snapshot {
    /// False until the first read finishes.
    pub loaded: bool,
    pub agents: Vec<Agent>,
    /// What the tokens would cost at API prices, in dollars.
    pub cost: f64,
    pub tokens: u64,
    pub agent_tokens: u64,
    pub started: i64,
    pub last: i64,
    pub todos: Vec<Todo>,
    /// MCP servers by first use, each with its tools and their call counts.
    pub mcp: Vec<(String, Vec<(String, u32)>)>,
    /// Built-in tools and their call counts, most used first.
    pub tools: Vec<(String, u32)>,
    /// Paths per `FILE_KINDS` entry, first touched first.
    pub files: [Vec<String>; 4],
    /// Instruction files Claude Code loaded on its own (CLAUDE.md, MEMORY.md).
    pub context: Vec<String>,
    /// Skills by name without the slash, with how often each was used.
    pub skills: Vec<(String, u32)>,
}

/// When something was used: its transcript time, then its line, which orders
/// uses within the same millisecond.
type At = (i64, u64);

/// A list that keeps first-seen order, counts repeats and remembers when each
/// item was last used.
#[derive(Default)]
struct Tally {
    order: Vec<String>,
    counts: HashMap<String, u32>,
    last: HashMap<String, At>,
}
impl Tally {
    fn add(&mut self, key: &str, at: At) {
        self.add_n(key, 1, at);
    }
    fn add_n(&mut self, key: &str, n: u32, at: At) {
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
    fn merge(&mut self, other: &Tally) {
        for key in &other.order {
            self.add_n(key, other.counts[key], other.last[key]);
        }
    }
    /// The items, most recently used first.
    fn recent(&self) -> Vec<String> {
        let mut keys = self.order.clone();
        keys.sort_by_key(|key| std::cmp::Reverse(self.last[key]));
        keys
    }
}

#[derive(Clone, Copy, Default)]
struct Usage {
    input: u64,
    cache_5m: u64,
    cache_1h: u64,
    cache_read: u64,
    output: u64,
    fast: bool,
}
impl Usage {
    fn from(value: &Value) -> Self {
        let n = |v: &Value| v.as_u64().unwrap_or(0);
        let created = n(&value["cache_creation_input_tokens"]);
        let hour = n(&value["cache_creation"]["ephemeral_1h_input_tokens"]).min(created);
        Self {
            input: n(&value["input_tokens"]),
            // Claude Code prices a write with no breakdown at the 5 minute rate.
            cache_5m: created - hour,
            cache_1h: hour,
            cache_read: n(&value["cache_read_input_tokens"]),
            output: n(&value["output_tokens"]),
            fast: value["speed"] == "fast",
        }
    }
    fn tokens(&self) -> u64 {
        self.input + self.cache_5m + self.cache_1h + self.cache_read + self.output
    }
    fn cost(&self, model: &str) -> f64 {
        let Some((input, output, read)) = price(model) else {
            return 0.;
        };
        let dollars = self.input as f64 * input
            + self.output as f64 * output
            + self.cache_read as f64 * read
            + self.cache_5m as f64 * input * 1.25
            + self.cache_1h as f64 * input * 2.;
        dollars / 1e6 * if self.fast { 2. } else { 1. }
    }
}

/// Dollars per million input, output and cache-read tokens. Checked against
/// the costs Claude Code records in its transcripts (`cost-state`).
// shortcut: a fixed table; a model released later costs 0 until it is added.
fn price(model: &str) -> Option<(f64, f64, f64)> {
    let model = model.strip_prefix("claude-").unwrap_or(model);
    let has = |prefix: &str| model.starts_with(prefix);
    Some(if has("fable-5-2") || has("mythos-5-2") {
        (8., 40., 0.2)
    } else if has("fable-5") || has("mythos-5") {
        (10., 50., 1.)
    } else if has("opus-5-5") {
        (4., 20., 0.2)
    } else if has("opus-5")
        || has("opus-4-8")
        || has("opus-4-7")
        || has("opus-4-6")
        || has("opus-4-5")
    {
        (5., 25., 0.5)
    } else if has("opus-4") {
        (15., 75., 1.5)
    } else if has("sonnet-5-5") {
        (2., 10., 0.1)
    } else if has("sonnet-5") {
        (2., 10., 0.2)
    } else if has("sonnet-4") {
        (3., 15., 0.3)
    } else if has("haiku-5-5") {
        (0.1, 0.5, 0.01)
    } else if has("haiku-4-5") {
        (1., 5., 0.1)
    } else {
        return None;
    })
}

/// "claude-opus-5-5" reads "Opus 5.5"; a dated id drops its date.
pub(crate) fn model_name(id: &str) -> String {
    let Some(rest) = id.strip_prefix("claude-") else {
        return id.to_owned();
    };
    let mut parts = rest.split('-').filter(|p| p.len() < 8);
    let Some(family) = parts.next() else {
        return id.to_owned();
    };
    let mut name: String = family[..1].to_uppercase() + &family[1..];
    let version: Vec<&str> = parts.collect();
    if !version.is_empty() {
        name.push(' ');
        name.push_str(&version.join("."));
    }
    name
}

/// "mcp__claude_ai_Gmail__get_thread" is the Gmail server's get_thread.
pub(crate) fn mcp_parts(tool: &str) -> Option<(&str, &str)> {
    tool.strip_prefix("mcp__")?.split_once("__")
}

fn basename(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
}

/// A tool call in the words the agent card shows.
fn step(name: &str, input: &Value) -> String {
    let text = |key: &str| input[key].as_str().unwrap_or_default();
    let file = |key: &str| basename(text(key)).to_owned();
    let words = match name {
        "Read" => format!("Reading {}", file("file_path")),
        "Write" => format!("Writing {}", file("file_path")),
        "Edit" | "MultiEdit" => format!("Editing {}", file("file_path")),
        "NotebookEdit" => format!("Editing {}", file("notebook_path")),
        "Bash" | "PowerShell" => match text("description") {
            "" => "Running a command".into(),
            description => description.to_owned(),
        },
        "Grep" => format!("Searching for \"{}\"", text("pattern")),
        "Glob" => format!("Finding {}", text("pattern")),
        "WebSearch" => format!("Searching the web: {}", text("query")),
        "WebFetch" => {
            let url = text("url");
            let host = url.split("://").nth(1).unwrap_or(url);
            format!("Reading {}", host.split('/').next().unwrap_or(host))
        }
        "Skill" => format!("Using /{}", text("skill")),
        "Agent" | "Task" => format!("Starting agent: {}", text("description")),
        "TaskCreate" => format!("Adding to-do: {}", text("subject")),
        "TaskUpdate" | "TodoWrite" => "Updating the to-do list".into(),
        "ToolSearch" => "Loading tools".into(),
        "SubagentHandback" | "StructuredOutput" => "Handing back the result".into(),
        "TaskStop" => "Stopping a background task".into(),
        "Monitor" => "Watching a background task".into(),
        "SendMessage" => "Messaging an agent".into(),
        "AskUserQuestion" => "Asking you a question".into(),
        _ => match mcp_parts(name) {
            Some((server, tool)) => format!("{}: {}", server_name(server), tool.replace('_', " ")),
            None => name.to_owned(),
        },
    };
    words.chars().take(120).collect()
}

/// "claude_ai_Google_Calendar" reads "Google Calendar"; a plugin's server
/// keeps its own name.
pub(crate) fn server_name(server: &str) -> String {
    if let Some(name) = server.strip_prefix("claude_ai_") {
        return name.replace('_', " ");
    }
    match server.strip_prefix("plugin_") {
        Some(name) => name.rsplit('_').next().unwrap_or(name).to_owned(),
        None => server.to_owned(),
    }
}

fn millis(timestamp: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|t| t.timestamp_millis())
}

/// The text between `<tag>` and `</tag>` in `text`, from `from` on.
fn tag<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&format!("</{name}>"))? + start;
    Some(text[start..end].trim())
}

fn message_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// One transcript file, read incrementally.
#[derive(Default)]
struct Log {
    offset: u64,
    partial: Vec<u8>,
    agent: bool,
    first: i64,
    last: i64,
    /// Lines read so far, to order uses that share a timestamp.
    lines: u64,
    /// Usage per API call; a call's blocks are separate lines that repeat it.
    usage: HashMap<String, (String, Usage)>,
    model: String,
    /// Whether the last reply ended the turn rather than calling a tool.
    ended: bool,
    tool_ids: HashSet<String>,
    tools: Tally,
    files: [Tally; 4],
    /// Write calls by id, with their path and time; the result says whether
    /// the file was new.
    writes: Vec<(String, String, At)>,
    created: HashSet<String>,
    skills: Tally,
    context: Tally,
    todos: Option<Vec<Todo>>,
    steps: Vec<String>,
    /// The latest task notification per agent id: status and when.
    notices: HashMap<String, (AgentStatus, i64)>,
}

impl Log {
    fn agent() -> Self {
        Self {
            agent: true,
            ..Self::default()
        }
    }
    /// Reads what was appended since the last pull. A file that shrank was
    /// rewritten, so it is read again from the start.
    fn pull(&mut self, path: &Path) {
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

    fn line(&mut self, line: &str) {
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
        const WANTED: [&str; 5] = [
            "\"type\":\"assistant\"",
            "\"type\":\"instructions\"",
            "<task-notification>",
            "<command-message>",
            "\"type\":\"create\"",
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
            _ => {}
        }
    }

    fn reply(&mut self, message: &Value) {
        let model = message["model"].as_str().unwrap_or_default();
        if model.is_empty() || model.starts_with('<') {
            return;
        }
        self.model = model.to_owned();
        if let Some(id) = message["id"].as_str() {
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
        if entry["toolUseResult"]["type"] == "create"
            && let Some(id) = entry["message"]["content"][0]["tool_use_id"].as_str()
        {
            self.created.insert(id.to_owned());
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
        for notice in text.split("<task-notification>").skip(1) {
            let (Some(task), Some(status)) = (tag(notice, "task-id"), tag(notice, "status")) else {
                continue;
            };
            let status = match status {
                "completed" => AgentStatus::Done,
                "failed" => AgentStatus::Failed,
                _ => AgentStatus::Stopped,
            };
            self.notices.insert(task.to_owned(), (status, time));
        }
    }

    fn tokens(&self) -> u64 {
        self.usage.values().map(|(_, usage)| usage.tokens()).sum()
    }
    fn cost(&self) -> f64 {
        self.usage
            .values()
            .map(|(model, usage)| usage.cost(model))
            .sum()
    }
}

fn todo_status(status: Option<&str>) -> TodoStatus {
    match status {
        Some("completed") => TodoStatus::Done,
        Some("in_progress") => TodoStatus::Active,
        _ => TodoStatus::Pending,
    }
}

#[derive(Default)]
struct AgentLog {
    log: Log,
    meta: Value,
}

/// Reads one session's files on a background thread; keep it between reads.
#[derive(Default)]
pub(crate) struct Tracker {
    session: String,
    transcript: Option<PathBuf>,
    main: Log,
    agents: BTreeMap<String, AgentLog>,
}

impl Tracker {
    pub(crate) fn refresh(&mut self, account: &Profile, session: &str) {
        if self.session != session {
            *self = Self {
                session: session.to_owned(),
                ..Self::default()
            };
        }
        if self.transcript.is_none() {
            self.transcript = monocode_core::provider::claude_transcript(account, session);
        }
        let Some(transcript) = self.transcript.clone() else {
            return;
        };
        self.main.pull(&transcript);
        let folder = transcript.with_extension("").join("subagents");
        for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            let path = entry.path();
            let Some(id) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_prefix("agent-")?.strip_suffix(".jsonl"))
            else {
                continue;
            };
            let agent = self
                .agents
                .entry(id.to_owned())
                .or_insert_with(|| AgentLog {
                    log: Log::agent(),
                    meta: Value::Null,
                });
            agent.log.pull(&path);
            // The meta file marks the end; a finished agent's stays as it is.
            if agent.meta["finished"] != true {
                agent.meta = std::fs::read_to_string(path.with_extension("meta.json"))
                    .ok()
                    .and_then(|text| serde_json::from_str(&text).ok())
                    .unwrap_or(Value::Null);
            }
        }
    }

    pub(crate) fn snapshot(&self, account: &Profile, now: i64) -> Snapshot {
        let logs = || std::iter::once(&self.main).chain(self.agents.values().map(|a| &a.log));
        let mut agents: Vec<Agent> = self
            .agents
            .iter()
            .filter(|(_, agent)| agent.log.first > 0)
            .map(|(id, agent)| self.agent(id, agent, now))
            .collect();
        agents.sort_by_key(|agent| agent.started);
        let agent_tokens = agents.iter().map(|a| a.tokens).sum();
        let mut tools = Tally::default();
        let mut files: [Tally; 4] = Default::default();
        let mut skills = Tally::default();
        let mut context = Tally::default();
        for log in logs() {
            tools.merge(&log.tools);
            for (kind, tally) in log.files.iter().enumerate() {
                files[kind].merge(tally);
            }
            for (id, path, at) in &log.writes {
                files[if log.created.contains(id) { 2 } else { 3 }].add(path, *at);
            }
            skills.merge(&log.skills);
            context.merge(&log.context);
        }
        let mut mcp: Vec<(String, Vec<(String, u32)>)> = Vec::new();
        let mut builtin = Vec::new();
        for name in &tools.order {
            let count = tools.counts[name];
            match mcp_parts(name) {
                Some((server, tool)) => match mcp.iter_mut().find(|(s, _)| s == server) {
                    Some((_, list)) => list.push((tool.to_owned(), count)),
                    None => mcp.push((server.to_owned(), vec![(tool.to_owned(), count)])),
                },
                None => builtin.push((name.clone(), count)),
            }
        }
        builtin.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        Snapshot {
            loaded: true,
            agents,
            cost: logs().map(Log::cost).sum(),
            tokens: logs().map(Log::tokens).sum(),
            agent_tokens,
            started: self.main.first,
            last: logs().map(|log| log.last).max().unwrap_or(0),
            todos: tasks(account, &self.session)
                .or_else(|| self.main.todos.clone())
                .unwrap_or_default(),
            mcp,
            tools: builtin,
            // Most recently used first; tools and MCP keep first-use order.
            files: files.map(|tally| tally.recent()),
            context: context.recent(),
            skills: skills
                .recent()
                .into_iter()
                .map(|name| {
                    let uses = skills.counts[&name];
                    (name, uses)
                })
                .collect(),
        }
    }

    fn agent(&self, id: &str, agent: &AgentLog, now: i64) -> Agent {
        let (log, meta) = (&agent.log, &agent.meta);
        // A notice older than the agent's last write was followed by more
        // work: the agent was continued after it.
        let notice = self
            .main
            .notices
            .get(id)
            .filter(|(_, at)| *at + 5_000 >= log.last)
            .map(|(status, _)| *status);
        let status = if meta["stoppedByUser"] == true {
            AgentStatus::Stopped
        } else if let Some(status) = notice {
            status
        } else if meta["finished"] == true || log.ended {
            AgentStatus::Done
        } else if now - log.last > STALE_MS {
            AgentStatus::Stopped
        } else {
            AgentStatus::Running
        };
        let model = match log.model.as_str() {
            "" => meta["model"]
                .as_str()
                .or(meta["pickedEffortModel"].as_str())
                .unwrap_or_default(),
            model => model,
        };
        Agent {
            id: id.to_owned(),
            title: meta["description"].as_str().unwrap_or("Agent").to_owned(),
            kind: meta["agentType"].as_str().unwrap_or("agent").to_owned(),
            model: model_name(model),
            tokens: log.tokens(),
            started: log.first,
            last: log.last,
            status,
            steps: log.steps.clone(),
        }
    }
}

/// The task list Claude keeps with its to-do tools, by task number.
fn tasks(account: &Profile, session: &str) -> Option<Vec<Todo>> {
    let folder = account.home(Kind::Claude).ok()?.join("tasks").join(session);
    let mut tasks: Vec<(u64, Todo)> = std::fs::read_dir(folder)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let task: Value =
                serde_json::from_str(&std::fs::read_to_string(entry.path()).ok()?).ok()?;
            Some((
                task["id"].as_str()?.parse().ok()?,
                Todo {
                    text: task["subject"].as_str()?.to_owned(),
                    status: todo_status(task["status"].as_str()),
                },
            ))
        })
        .collect();
    tasks.sort_by_key(|(id, _)| *id);
    Some(tasks.into_iter().map(|(_, todo)| todo).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(log: &mut Log, lines: &[&str]) {
        for line in lines {
            log.line(line);
        }
    }

    #[test]
    fn counts_usage_once_per_call_and_prices_it_like_claude_code() {
        let mut log = Log::default();
        // Claude Code's own record: 2 in, 20 out, 13,423 read, 7,617 written
        // for an hour cost $0.064029 on Opus 5.5.
        let reply = r#"{"type":"assistant","message":{"id":"m1","model":"claude-opus-5-5","stop_reason":"tool_use","usage":{"input_tokens":2,"output_tokens":20,"cache_read_input_tokens":13423,"cache_creation_input_tokens":7617,"cache_creation":{"ephemeral_1h_input_tokens":7617}},"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/p/a.rs"}}]},"timestamp":"2026-11-14T10:00:00.000Z"}"#;
        let second = r#"{"type":"assistant","message":{"id":"m1","model":"claude-opus-5-5","usage":{"input_tokens":2,"output_tokens":20,"cache_read_input_tokens":13423,"cache_creation_input_tokens":7617,"cache_creation":{"ephemeral_1h_input_tokens":7617}},"content":[{"type":"tool_use","id":"t2","name":"mcp__claude_ai_Gmail__get_thread","input":{}}]},"timestamp":"2026-11-14T10:00:30.000Z"}"#;
        feed(&mut log, &[reply, second]);
        assert_eq!(log.tokens(), 2 + 20 + 13423 + 7617);
        assert!((log.cost() - 0.064029).abs() < 1e-6, "{}", log.cost());
        assert_eq!(log.last - log.first, 30_000);
        assert_eq!(log.files[0].order, ["/p/a.rs"]);
        assert_eq!(
            log.tools.order,
            ["Read", "mcp__claude_ai_Gmail__get_thread"]
        );
        // A write without a breakdown is priced at the 5 minute rate:
        // 20 in, 1,500 out, 480,916 read, 18,147 written cost $0.216998.
        let usage = Usage::from(
            &serde_json::json!({"input_tokens":20,"output_tokens":1500,"cache_read_input_tokens":480916,"cache_creation_input_tokens":18147}),
        );
        assert!((usage.cost("claude-opus-5-5") - 0.216998).abs() < 1e-6);
    }

    #[test]
    fn reads_skills_writes_notices_and_context() {
        let mut log = Log::default();
        feed(
            &mut log,
            &[
                r#"{"type":"user","message":{"content":"<command-message>ponytail:ponytail</command-message>\n<command-name>/ponytail:ponytail</command-name>"}}"#,
                r#"{"type":"user","message":{"content":"<command-name>/clear</command-name>\n<command-message>clear</command-message>"}}"#,
                r#"{"type":"assistant","message":{"id":"m","model":"claude-fable-5-2","content":[{"type":"tool_use","id":"w1","name":"Write","input":{"file_path":"/p/new.md"}},{"type":"tool_use","id":"w2","name":"Write","input":{"file_path":"/p/old.md"}},{"type":"tool_use","id":"s","name":"Skill","input":{"skill":"ponytail:ponytail"}}]}}"#,
                r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"w1"}]},"toolUseResult":{"type":"create","filePath":"/p/new.md"}}"#,
                r#"{"type":"user","message":{"content":"<task-notification>\n<task-id>a1</task-id>\n<status>failed</status>\n</task-notification>"},"timestamp":"2026-11-14T10:00:00.000Z"}"#,
                r#"{"type":"attachment","attachment":{"type":"instructions","files":[{"path":"/home/u/CLAUDE.md","type":"Project"}]}}"#,
            ],
        );
        assert_eq!(log.skills.order, ["ponytail:ponytail"]);
        assert_eq!(log.skills.counts["ponytail:ponytail"], 2);
        assert!(log.created.contains("w1") && !log.created.contains("w2"));
        assert_eq!(log.notices["a1"].0, AgentStatus::Failed);
        assert_eq!(log.context.order, ["/home/u/CLAUDE.md"]);
    }

    #[test]
    fn names_models_servers_and_steps() {
        assert_eq!(model_name("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(model_name("claude-haiku-4-5-20251001"), "Haiku 4.5");
        assert_eq!(model_name("claude-fable-5"), "Fable 5");
        assert_eq!(server_name("claude_ai_Google_Calendar"), "Google Calendar");
        assert_eq!(
            server_name("plugin_antislop_antislop-contrast"),
            "antislop-contrast"
        );
        assert_eq!(server_name("figcraft"), "figcraft");
        assert_eq!(
            mcp_parts("mcp__figcraft__open_file"),
            Some(("figcraft", "open_file"))
        );
        assert_eq!(
            step("Bash", &serde_json::json!({"description": "Run the tests"})),
            "Run the tests"
        );
        assert_eq!(
            step("Write", &serde_json::json!({"file_path": "/a/b/plan.md"})),
            "Writing plan.md"
        );
    }

    #[test]
    fn reads_appended_lines_and_starts_over_after_a_rewrite() {
        let dir = std::env::temp_dir().join(format!("claude-activity-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        let reply = |id: &str| {
            format!(
                "{{\"type\":\"assistant\",\"message\":{{\"id\":\"{id}\",\"model\":\"claude-opus-5-5\",\"usage\":{{\"output_tokens\":10}},\"content\":[]}}}}\n"
            )
        };
        std::fs::write(&path, reply("a") + &reply("b")[..20]).unwrap();
        let mut log = Log::default();
        log.pull(&path);
        assert_eq!(log.tokens(), 10, "the half-written line waits");
        std::fs::write(&path, reply("a") + &reply("b")).unwrap();
        log.pull(&path);
        assert_eq!(log.tokens(), 20);
        std::fs::write(&path, reply("c")).unwrap();
        log.pull(&path);
        assert_eq!(log.tokens(), 10);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn lists_files_most_recently_used_first_across_agents() {
        let read = |id: &str, path: &str, second: &str| {
            format!(
                r#"{{"type":"assistant","message":{{"id":"{id}","model":"claude-opus-5-5","content":[{{"type":"tool_use","id":"{id}","name":"Read","input":{{"file_path":"{path}"}}}}]}},"timestamp":"2026-11-17T10:00:{second}.000Z"}}"#
            )
        };
        let mut main = Log::default();
        feed(
            &mut main,
            &[
                &read("r1", "/p/a.rs", "01"),
                &read("r2", "/p/b.rs", "02"),
                &read("r3", "/p/a.rs", "03"),
            ],
        );
        assert_eq!(
            main.files[0].recent(),
            ["/p/a.rs", "/p/b.rs"],
            "a file read again moves back to the top"
        );
        // An agent's later read comes first, whichever log it is in.
        let mut agent = Log::agent();
        feed(&mut agent, &[&read("r4", "/p/c.rs", "04")]);
        let mut merged = Tally::default();
        merged.merge(&main.files[0]);
        merged.merge(&agent.files[0]);
        assert_eq!(merged.recent(), ["/p/c.rs", "/p/a.rs", "/p/b.rs"]);
        assert_eq!(merged.counts["/p/a.rs"], 2);
    }

    #[test]
    fn a_big_transcript_leaves_only_its_unfinished_line_in_memory() {
        let dir = std::env::temp_dir().join(format!("claude-activity-big-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        // About 9 MB, so it is read in several chunks.
        let line = "{\"type\":\"user\",\"message\":{\"content\":\"x\"}}\n";
        std::fs::write(&path, line.repeat(200_000) + "{\"half").unwrap();
        let mut log = Log::default();
        log.pull(&path);
        assert_eq!(log.partial, b"{\"half");
        assert!(log.partial.capacity() < 1024, "{}", log.partial.capacity());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
