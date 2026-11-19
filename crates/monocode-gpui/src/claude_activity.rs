//! What a Claude Code session did, read from the files the CLI writes: its
//! transcript, one transcript per subagent beside it, and its task list.
//! Transcripts grow to hundreds of megabytes, mostly tool output, so each file
//! is read from where the last read stopped and only lines that can matter are
//! parsed.
use monocode_core::{accounts::Profile, provider::Kind};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

mod log;
mod names;
#[cfg(test)]
mod tests;

use self::log::{Log, Tally, todo_status};
pub(crate) use self::names::{mcp_parts, model_name, server_name};

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

/// Work Claude left running: a shell command, subagent or monitor.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Background {
    pub tool: String,
    /// The command, or the description when there is none.
    pub detail: String,
    pub started: i64,
}

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
    /// The main chat's context as of its last reply, and the window it fills.
    pub context_used: u64,
    pub context_window: u64,
    /// Background work still running, oldest first.
    pub background: Vec<Background>,
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
    /// The session's own transcript, once found.
    pub(crate) fn transcript(&self) -> Option<&Path> {
        self.transcript.as_deref()
    }
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

    /// `wide` is a model picked with a 1M window.
    pub(crate) fn snapshot(&self, account: &Profile, now: i64, wide: bool) -> Snapshot {
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
        let finished: HashSet<&String> = logs().flat_map(|log| &log.finished).collect();
        let mut background: Vec<Background> = logs()
            .flat_map(|log| &log.background)
            .filter(|t| !finished.contains(&t.tool_id) && !finished.contains(&t.task_id))
            .filter(|t| t.until.is_none_or(|until| now < until))
            // A background agent also ends when its own transcript says so.
            .filter(|t| {
                agents
                    .iter()
                    .find(|agent| agent.id == t.task_id)
                    .is_none_or(|agent| agent.status == AgentStatus::Running)
            })
            .map(|t| t.shown.clone())
            .collect();
        background.sort_by_key(|b| b.started);
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
            context_used: self.main.used,
            // ponytail: the CLI does not record the window; past 200K it must be 1M.
            context_window: if wide || self.main.peak > 200_000 {
                1_000_000
            } else {
                200_000
            },
            background,
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
