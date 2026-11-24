//! A Claude Code session as chat rows, read from the transcript the CLI writes,
//! and the sessions a folder has, for `/resume`. Transcripts reach hundreds of
//! megabytes, so the chain is found from each line's ids alone and only the
//! entries on it are parsed in full.
use super::rewind::{claude_prompt_text, claude_resume};
use super::Kind;
use crate::session::{
    tool::{ToolCall, ToolState},
    transcript::{Row, Transcript},
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Ids {
    uuid: Option<String>,
    parent_uuid: Option<String>,
    /// A compaction starts a new chain; this links it to what came before.
    logical_parent_uuid: Option<String>,
    #[serde(default)]
    is_sidechain: bool,
    #[serde(rename = "type")]
    kind: Option<String>,
}

fn lines(path: &Path) -> Result<impl Iterator<Item = String>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    Ok(BufReader::new(file).lines().map_while(Result::ok))
}

/// The entries from the first to `leaf` (else the last message), compactions
/// included, so the chat shows the whole conversation.
fn chain(path: &Path, leaf: Option<&str>) -> Result<HashSet<String>, String> {
    let mut parents = HashMap::new();
    let mut last = None;
    for line in lines(path)?.filter(|line| line.contains("\"uuid\"")) {
        let Ok(ids) = serde_json::from_str::<Ids>(&line) else {
            continue;
        };
        let Some(id) = ids.uuid.filter(|_| !ids.is_sidechain) else {
            continue;
        };
        if matches!(ids.kind.as_deref(), Some("user" | "assistant")) {
            last = Some(id.clone());
        }
        parents.insert(id, ids.parent_uuid.or(ids.logical_parent_uuid));
    }
    let mut on = HashSet::new();
    let mut cursor = leaf.map(str::to_owned).or(last);
    // The set's size bound stops a cycle.
    while let Some(id) = cursor.take() {
        let Some(parent) = parents.get(&id) else {
            break;
        };
        if !on.insert(id) {
            break;
        }
        cursor = parent.clone();
    }
    Ok(on)
}

/// A slash command Claude saved as tags, read back as `/name args`.
fn shown_prompt(text: &str) -> String {
    let tag = |name: &str| {
        let open = format!("<{name}>");
        let start = text.find(&open)? + open.len();
        let end = text[start..].find(&format!("</{name}>"))?;
        Some(text[start..start + end].trim().to_owned())
    };
    match tag("command-name") {
        Some(name) => format!(
            "/{} {}",
            name.trim_start_matches('/'),
            tag("command-args").unwrap_or_default()
        )
        .trim_end()
        .to_owned(),
        None => text.to_owned(),
    }
}

fn sent_at(entry: &Value) -> Option<u64> {
    let at = chrono::DateTime::parse_from_rfc3339(entry["timestamp"].as_str()?).ok()?;
    u64::try_from(at.timestamp_millis()).ok()
}

/// The chat rows for a Claude binding (`session` or `session?at=entry`).
pub fn claude_history(path: &Path, binding: &str) -> Result<Vec<Row>, String> {
    let on = chain(path, claude_resume(binding).1)?;
    let mut t = Transcript::default();
    let mut tools: HashMap<String, (usize, ToolCall)> = HashMap::new();
    for line in lines(path)? {
        // Cheap filters before parsing: most lines are bookkeeping.
        if !(line.contains("\"type\":\"user\"") || line.contains("\"type\":\"assistant\"")) {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if !entry["uuid"].as_str().is_some_and(|id| on.contains(id)) {
            continue;
        }
        let at = sent_at(&entry);
        let content = &entry["message"]["content"];
        if entry["type"] == "user" {
            if let Some(text) = claude_prompt_text(&entry) {
                let images: Vec<PathBuf> = content
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|b| b["type"] == "image")
                    .enumerate()
                    .filter_map(|(ix, b)| {
                        crate::session::tool::save_image(
                            entry["uuid"].as_str()?,
                            ix,
                            b["source"]["media_type"].as_str()?,
                            b["source"]["data"].as_str()?,
                        )
                    })
                    .map(PathBuf::from)
                    .collect();
                if text.trim().is_empty() && images.is_empty() {
                    continue;
                }
                let row = t.begin("user");
                t.rows[row].sent_at = at;
                t.append(&shown_prompt(&text));
                t.attach_files(&images);
                continue;
            }
            for block in content.as_array().into_iter().flatten() {
                let Some((row, tool)) = block["tool_use_id"]
                    .as_str()
                    .and_then(|id| tools.get_mut(id))
                else {
                    continue;
                };
                tool.output = block["content"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        block["content"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|b| b["text"].as_str())
                            .collect::<Vec<_>>()
                            .join("\n")
                    });
                tool.bound_output();
                tool.images = block["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|b| b["type"] == "image")
                    .take(8)
                    .enumerate()
                    .filter_map(|(ix, b)| {
                        crate::session::tool::save_image(
                            &tool.id,
                            ix,
                            b["source"]["media_type"].as_str()?,
                            b["source"]["data"].as_str()?,
                        )
                    })
                    .collect();
                tool.state = if block["is_error"] == true {
                    ToolState::Failed
                } else {
                    ToolState::Completed
                };
                t.rows[*row].text = serde_json::to_string(tool).map_err(|e| e.to_string())?;
            }
            continue;
        }
        for block in content.as_array().into_iter().flatten() {
            match block["type"].as_str() {
                Some("thinking") => {
                    let text = block["thinking"].as_str().unwrap_or("");
                    if text.trim().is_empty() {
                        continue;
                    }
                    if t.rows.last().is_none_or(|r| r.role != "reasoning") {
                        let row = t.begin("reasoning");
                        t.rows[row].sent_at = at;
                    }
                    t.append(text);
                }
                Some("text") => {
                    let text = block["text"].as_str().unwrap_or("");
                    if text.trim().is_empty() {
                        continue;
                    }
                    if t.rows.last().is_none_or(|r| r.role != "assistant") {
                        let row = t.begin("assistant");
                        t.rows[row].sent_at = at;
                    } else {
                        t.append("\n\n");
                    }
                    t.append(text);
                }
                Some("tool_use") => {
                    let id = block["id"].as_str().unwrap_or("");
                    let mut tool = ToolCall::new(id, block["name"].as_str().unwrap_or("Tool"));
                    tool.input = serde_json::to_string_pretty(&block["input"]).unwrap_or_default();
                    tool.refresh_agent_input();
                    // Still running unless a result follows; a stopped session
                    // left it unfinished.
                    tool.state = ToolState::Cancelled;
                    let row = t.begin("tool");
                    t.rows[row].sent_at = at;
                    t.rows[row].text = serde_json::to_string(&tool).map_err(|e| e.to_string())?;
                    tools.insert(id.to_owned(), (row, tool));
                }
                _ => {}
            }
        }
    }
    Ok(t.rows)
}

/// A Claude Code session `/resume` can open.
#[derive(Clone, Debug, PartialEq)]
pub struct ClaudeSession {
    pub id: String,
    pub title: String,
    /// When the CLI last wrote to it, in ms.
    pub modified: u64,
    pub branch: Option<String>,
}

/// Claude's folder for a project: every character other than a letter or digit
/// becomes `-`.
// shortcut: very long paths get a hashed name in newer CLIs and are not
// found; read each folder's first `cwd` if users with such paths report it.
fn project_folder(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// How much of each file's start and end the list reads.
const PEEK: u64 = 64 * 1024;
/// The newest sessions the list shows.
const NEWEST: usize = 200;

fn peek(path: &Path) -> Option<(String, String)> {
    let mut file = std::fs::File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let mut head = Vec::new();
    (&mut file).take(PEEK).read_to_end(&mut head).ok()?;
    let mut tail = Vec::new();
    if size > PEEK {
        file.seek(SeekFrom::Start(size.saturating_sub(PEEK).max(PEEK)))
            .ok()?;
        file.read_to_end(&mut tail).ok()?;
    }
    Some((
        String::from_utf8_lossy(&head).into_owned(),
        String::from_utf8_lossy(&tail).into_owned(),
    ))
}

fn session_info(path: &Path, modified: u64) -> Option<ClaudeSession> {
    let id = path.file_stem()?.to_str()?.to_owned();
    let (head, tail) = peek(path)?;
    let entries = || {
        head.lines()
            .chain(tail.lines())
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
    };
    let prompt = entries()
        .filter(|entry| entry["isSidechain"] != true)
        .filter_map(|entry| claude_prompt_text(&entry))
        .map(|text| shown_prompt(&text))
        .find(|text| !text.trim().is_empty() && !text.trim_start().starts_with('<'))?;
    let mut named = None;
    let mut titled = None;
    let mut branch = None;
    for entry in entries() {
        if let Some(title) = entry["customTitle"].as_str() {
            named = Some(title.to_owned());
        }
        if let Some(title) = entry["aiTitle"].as_str().or(entry["summary"].as_str()) {
            titled = Some(title.to_owned());
        }
        if let Some(name) = entry["gitBranch"].as_str().filter(|b| !b.is_empty()) {
            branch = Some(name.to_owned());
        }
    }
    let title = named
        .or(titled)
        .unwrap_or_else(|| prompt.split_whitespace().collect::<Vec<_>>().join(" "));
    Some(ClaudeSession {
        id,
        title,
        modified,
        branch,
    })
}

/// The Claude Code sessions for `cwd`, newest first, started here or not.
pub fn claude_sessions(
    account: &crate::session::profile::Profile,
    cwd: &str,
) -> Vec<ClaudeSession> {
    let Ok(home) = account.home(Kind::Claude) else {
        return Vec::new();
    };
    sessions_in(&home.join("projects").join(project_folder(cwd)))
}

fn sessions_in(folder: &Path) -> Vec<ClaudeSession> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut files: Vec<(PathBuf, u64)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "jsonl"))
        .filter_map(|path| {
            let modified = path
                .metadata()
                .ok()?
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_millis() as u64;
            Some((path, modified))
        })
        .collect();
    files.sort_by(|a, b| b.1.cmp(&a.1));
    files
        .iter()
        .take(NEWEST)
        .filter_map(|(path, modified)| session_info(path, *modified))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write(dir: &Path, name: &str, entries: &[Value]) -> PathBuf {
        let path = dir.join(name);
        let text: Vec<String> = entries.iter().map(Value::to_string).collect();
        std::fs::write(&path, text.join("\n")).unwrap();
        path
    }

    fn tool(row: &Row) -> ToolCall {
        serde_json::from_str(&row.text).unwrap()
    }

    #[test]
    fn a_session_file_reads_as_the_chat_it_was() {
        let dir = std::env::temp_dir().join(format!("mcode-claude-history-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = write(
            &dir,
            "s.jsonl",
            &[
                json!({"type":"permission-mode","permissionMode":"auto"}),
                json!({"type":"user","uuid":"u1","parentUuid":null,"timestamp":"2026-11-24T01:00:00Z","message":{"role":"user","content":"fix the bug"}}),
                json!({"type":"attachment","uuid":"x1","parentUuid":"u1"}),
                json!({"type":"assistant","uuid":"a1","parentUuid":"x1","message":{"content":[{"type":"thinking","thinking":"Look first."}]}}),
                json!({"type":"assistant","uuid":"a2","parentUuid":"a1","message":{"content":[{"type":"text","text":"Reading it."}]}}),
                json!({"type":"assistant","uuid":"a3","parentUuid":"a2","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}),
                json!({"type":"user","uuid":"r1","parentUuid":"a3","toolUseResult":{},"message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"a.rs"}]}}),
                // A subagent's own work and Claude's injected notes stay out.
                json!({"type":"assistant","uuid":"side","parentUuid":"r1","isSidechain":true,"message":{"content":[{"type":"text","text":"subagent"}]}}),
                json!({"type":"user","uuid":"n1","parentUuid":"r1","message":{"content":"<system-reminder>hi</system-reminder>"}}),
                json!({"type":"system","subtype":"compact_boundary","uuid":"c1","parentUuid":null,"logicalParentUuid":"n1"}),
                json!({"type":"user","uuid":"s1","parentUuid":"c1","isCompactSummary":true,"message":{"content":"Summary of before"}}),
                json!({"type":"user","uuid":"u2","parentUuid":"s1","message":{"content":"<command-name>/review</command-name><command-args>pr 3</command-args>"}}),
                json!({"type":"assistant","uuid":"a4","parentUuid":"u2","message":{"content":[{"type":"text","text":"Done."}]}}),
                // A branch left behind by a rewind inside the CLI.
                json!({"type":"user","uuid":"old","parentUuid":"a4","message":{"content":"abandoned"}}),
                json!({"type":"user","uuid":"u3","parentUuid":"a4","message":{"content":"thanks"}}),
            ],
        );
        let rows = claude_history(&path, "s").unwrap();
        let shown: Vec<(&str, &str)> = rows
            .iter()
            .map(|r| {
                (
                    r.role.as_str(),
                    if r.role == "tool" {
                        ""
                    } else {
                        r.text.as_str()
                    },
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                ("user", "fix the bug"),
                ("reasoning", "Look first."),
                ("assistant", "Reading it."),
                ("tool", ""),
                ("user", "/review pr 3"),
                ("assistant", "Done."),
                ("user", "thanks"),
            ]
        );
        let ran = tool(&rows[3]);
        assert_eq!((ran.name.as_str(), ran.output.as_str()), ("Bash", "a.rs"));
        assert_eq!(ran.state, ToolState::Completed);
        assert!(rows[0].sent_at.is_some());
        // A pending rewind shows the chat up to its entry.
        let cut = claude_history(&path, "s?at=a2").unwrap();
        assert_eq!(cut.last().unwrap().text, "Reading it.");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_list_names_each_session_by_its_title_and_skips_empty_ones() {
        let dir =
            std::env::temp_dir().join(format!("mcode-claude-sessions-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        write(
            &dir,
            "a.jsonl",
            &[
                json!({"type":"user","uuid":"u","gitBranch":"main","message":{"content":"first   prompt\nhere"}}),
            ],
        );
        write(
            &dir,
            "b.jsonl",
            &[
                json!({"type":"user","uuid":"u","gitBranch":"feat/x","message":{"content":"hello"}}),
                json!({"type":"ai-title","aiTitle":"Fix the parser"}),
            ],
        );
        write(
            &dir,
            "empty.jsonl",
            &[json!({"type":"mode","mode":"normal"})],
        );
        let mut found = sessions_in(&dir);
        found.sort_by(|a, b| a.id.cmp(&b.id));
        let named: Vec<(&str, &str, Option<&str>)> = found
            .iter()
            .map(|s| (s.id.as_str(), s.title.as_str(), s.branch.as_deref()))
            .collect();
        assert_eq!(
            named,
            [
                ("a", "first prompt here", Some("main")),
                ("b", "Fix the parser", Some("feat/x")),
            ]
        );
        assert_eq!(project_folder("/mnt/data/my app"), "-mnt-data-my-app");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
