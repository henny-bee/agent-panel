//! Prices and plain-word names for what a transcript records, and the text
//! helpers reading one needs.
use super::*;

#[derive(Clone, Copy, Default)]
pub(super) struct Usage {
    pub(super) input: u64,
    pub(super) cache_5m: u64,
    pub(super) cache_1h: u64,
    pub(super) cache_read: u64,
    pub(super) output: u64,
    pub(super) fast: bool,
}
impl Usage {
    pub(super) fn from(value: &Value) -> Self {
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
    pub(super) fn tokens(&self) -> u64 {
        self.input + self.cache_5m + self.cache_1h + self.cache_read + self.output
    }
    pub(super) fn cost(&self, model: &str) -> f64 {
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
pub(super) fn price(model: &str) -> Option<(f64, f64, f64)> {
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

pub(super) fn basename(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
}

/// A tool call in the words the agent card shows.
pub(super) fn step(name: &str, input: &Value) -> String {
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

pub(super) fn millis(timestamp: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|t| t.timestamp_millis())
}

/// The text between `<tag>` and `</tag>` in `text`, from `from` on.
pub(super) fn tag<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&format!("</{name}>"))? + start;
    Some(text[start..end].trim())
}

pub(super) fn message_text(content: &Value) -> String {
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
