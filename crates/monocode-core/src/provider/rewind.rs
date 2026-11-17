//! Explicit provider rollback. This never starts a prompt or edits local history.
//! The methods and message shapes mirror the checked-in Codex and Pi adapters.
use super::*;

#[derive(Debug)]
pub struct RewindError {
    pub message: String,
    /// A destructive request may have reached the provider without its reply.
    pub provider_may_have_changed: bool,
}
impl From<String> for RewindError {
    fn from(message: String) -> Self {
        Self {
            message,
            provider_may_have_changed: false,
        }
    }
}
impl From<&str> for RewindError {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
impl std::fmt::Display for RewindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for RewindError {}
fn transport_error(message: String, mutating: bool) -> RewindError {
    RewindError {
        message,
        provider_may_have_changed: mutating,
    }
}

pub fn supports_rewind(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Codex | Kind::Pi | Kind::Omp | Kind::OpenCode | Kind::Claude
    )
}

const CLAUDE_AT: &str = "?at=";
const CLAUDE_DROPS: &str = "&drops=";

/// Splits a Claude binding into its session and a rewind that is still pending:
/// the entry to resume at, and the prompt of the turn that resume discards.
pub(crate) fn claude_resume(binding: &str) -> (&str, Option<(&str, &str)>) {
    binding
        .split_once(CLAUDE_AT)
        .and_then(|(session, rest)| {
            let (at, drops) = rest.split_once(CLAUDE_DROPS)?;
            Some((session, Some((at, drops))))
        })
        .unwrap_or((binding, None))
}

/// A pending rewind forks the session before the dropped turn, leaving the
/// original untouched. The fork's own id then replaces the binding.
pub(super) fn claude_resume_args(command: &mut Command, binding: &str) {
    let (session, rewind) = claude_resume(binding);
    command.args(["--resume", session]);
    if let Some((at, drops)) = rewind {
        command.args([
            "--resume-session-at",
            at,
            "--resume-drops-turn",
            drops,
            "--fork-session",
        ]);
    }
}

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The text of a prompt the user sent, as opposed to tool results, injected
/// context, compaction summaries and the marker a stopped turn leaves behind.
fn claude_prompt_text(entry: &Value) -> Option<String> {
    if entry["type"] != "user"
        || entry["isMeta"] == true
        || entry["isCompactSummary"] == true
        || entry.get("toolUseResult").is_some()
    {
        return None;
    }
    let text = match &entry["message"]["content"] {
        Value::String(text) => text.clone(),
        Value::Array(blocks) if !blocks.iter().any(|block| block["type"] == "tool_result") => {
            blocks
                .iter()
                .filter_map(|block| block["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        }
        _ => return None,
    };
    (!text.starts_with("[Request interrupted")).then_some(text)
}

/// Where Claude keeps a session. It saves one only once it holds a message.
pub fn claude_transcript(account: &crate::accounts::Profile, session: &str) -> Option<PathBuf> {
    let name = format!("{session}.jsonl");
    std::fs::read_dir(account.home(Kind::Claude).ok()?.join("projects"))
        .ok()?
        .flatten()
        .map(|entry| entry.path().join(&name))
        .find(|path| path.is_file())
}

/// The session a running Claude CLI is on now. `/clear` and `/resume` typed
/// inside it move it away from the id it was started with.
// shortcut: reads Claude Code's own undocumented `sessions/<pid>.json`; if a
// release drops that file this returns None and the caller keeps the id it has.
pub fn claude_live_session(account: &crate::accounts::Profile, pid: u32) -> Option<String> {
    let record = account
        .home(Kind::Claude)
        .ok()?
        .join("sessions")
        .join(format!("{pid}.json"));
    let record: Value = serde_json::from_str(&std::fs::read_to_string(record).ok()?).ok()?;
    record["sessionId"]
        .as_str()
        // The id becomes a CLI argument: accept only what a session id looks like.
        .filter(|id| {
            record["pid"] == pid
                && id.len() == 36
                && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        })
        .map(str::to_owned)
}

/// The first message the user sent in a Claude session, to name it. Slash
/// commands and their output are stored as tagged text and are skipped.
pub fn claude_first_prompt(account: &crate::accounts::Profile, session: &str) -> Option<String> {
    std::fs::read_to_string(claude_transcript(account, session)?)
        .ok()?
        .lines()
        .filter(|line| line.contains("\"uuid\""))
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|entry| entry["isSidechain"] != true)
        .filter_map(|entry| claude_prompt_text(&entry))
        .find(|text| !text.trim().is_empty() && !text.trim_start().starts_with('<'))
}

/// Finds the last prompt on the active chain of a Claude transcript. Returns the
/// entry before it and the prompt itself. `expected` is the message being edited:
/// a transcript that ends with a different prompt is left alone.
fn claude_boundary(transcript: &str, expected: &str) -> Result<(String, String), String> {
    let mut entries = HashMap::new();
    let mut cursor = None;
    for line in transcript.lines().filter(|line| line.contains("\"uuid\"")) {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(id) = entry["uuid"]
            .as_str()
            .filter(|_| entry["isSidechain"] != true)
        else {
            continue;
        };
        cursor = Some(id.to_owned());
        entries.insert(
            id.to_owned(),
            (
                entry["parentUuid"].as_str().map(str::to_owned),
                claude_prompt_text(&entry),
            ),
        );
    }
    // A chain is never longer than the transcript; the bound only stops a cycle.
    for _ in 0..entries.len() {
        let Some((id, (parent, prompt))) = cursor
            .take()
            .and_then(|id| entries.get(&id).map(|entry| (id, entry)))
        else {
            break;
        };
        let Some(prompt) = prompt else {
            cursor = parent.clone();
            continue;
        };
        if !squash(prompt).contains(&squash(expected)) {
            return Err(
                "Claude's saved conversation does not end with this message; history was preserved"
                    .into(),
            );
        }
        let at = parent.clone().ok_or(
            "Editing the first message is not supported for Claude; start a new conversation",
        )?;
        return Ok((at, id));
    }
    Err("Claude did not save a user message to edit".into())
}

/// Claude has no in-process rollback. This asks the CLI whether a resume that
/// drops the last turn would be accepted, without sending a prompt, and returns
/// the binding that makes the next launch perform it.
fn claude_rewind(request: &Request) -> Result<String, RewindError> {
    let session = request
        .provider_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .ok_or("The conversation has no saved provider session to rewind")?;
    // ponytail: one pending rewind per session. Dropping two turns in a row
    // would need a resume that discards both, which the drop guard refuses.
    if claude_resume(session).1.is_some() {
        return Err("Send a message before editing the last one again".into());
    }
    let transcript = claude_transcript(&request.settings.account, session)
        .ok_or("Could not find the Claude conversation to rewind")?;
    let transcript = std::fs::read_to_string(transcript)
        .map_err(|error| format!("Could not read the Claude conversation: {error}"))?;
    let (at, drops) = claude_boundary(&transcript, &request.prompt)?;
    let binding = format!("{session}{CLAUDE_AT}{at}{CLAUDE_DROPS}{drops}");
    // The warm process still holds the turn about to be dropped.
    parked::discard_session(request);
    let mut command = Command::new(&request.binary);
    crate::scm::shell::apply_gui_env(&mut command);
    command.envs(&request.settings.environment);
    request.settings.account.apply(Kind::Claude, &mut command)?;
    command.args([
        "--verbose",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--strict-mcp-config",
        "--mcp-config",
        "{\"mcpServers\":{}}",
        "--settings",
        "{\"disableAllHooks\":true}",
    ]);
    claude_resume_args(&mut command, &binding);
    // With no input the CLI loads the truncated session, writes nothing and exits.
    command
        .current_dir(&request.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    process::isolate(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not start Claude: {error}"))?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < Duration::from_secs(60) => thread::sleep(TICK),
            _ => {
                process::terminate(&mut child);
                return Err("Claude did not confirm the rewind; history was preserved".into());
            }
        }
    };
    if !status.success() {
        let mut detail = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_string(&mut detail);
        }
        let detail = detail.lines().find(|line| !line.trim().is_empty());
        return Err(detail
            .unwrap_or("Claude refused the rewind; history was preserved")
            .trim()
            .into());
    }
    Ok(binding)
}

/// Runs on a worker, after the caller has reserved its conversation. Returns the
/// provider binding to persist with the confirmed local truncation. Pi forks to
/// a new session; Codex keeps the current thread. No prompt is sent here.
pub fn rewind_last_turn(
    request: Request,
    before_turn: Option<&str>,
) -> Result<String, RewindError> {
    if !supports_rewind(request.kind) || request.settings.text_only {
        return Err("This provider does not support editing the last turn".into());
    }
    if request.kind == Kind::OpenCode {
        return super::opencode::rewind(&request, before_turn);
    }
    if request.kind == Kind::Claude {
        return claude_rewind(&request);
    }
    let session = request
        .provider_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .ok_or("The conversation has no saved provider session to rewind")?;
    // Remove the previous warm process before reverting its underlying session.
    // It must not later resume with a pre-revert in-memory context.
    parked::discard_session(&request);
    let mut command = Command::new(&request.binary);
    crate::scm::shell::apply_gui_env(&mut command);
    command.envs(&request.settings.environment);
    request.settings.account.apply(request.kind, &mut command)?;
    if request.kind == Kind::Codex {
        command.arg("app-server");
    } else {
        pi::configure(&mut command, &request, false);
    }
    command
        .current_dir(&request.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process::isolate(&mut command);
    let mut wire = parked::Wire::spawn(&mut command, &request)?;
    let mut rpc = Rpc {
        wire: &mut wire,
        serial: 100,
        pi: request.kind.is_pi(),
    };
    if rpc.pi {
        let state = rpc.call("get_state", json!({}))?;
        if pi_binding(&state).as_deref() != Some(session) {
            return Err("Pi resumed a different session; history was preserved".into());
        }
        if state["isStreaming"] == true || state["isCompacting"] == true {
            return Err("Stop the current turn before editing the last message".into());
        }
        let messages = rpc.call("get_fork_messages", json!({}))?;
        let entry = last_fork_entry(&messages)?;
        let fork = rpc.call("fork", json!({"entryId":entry}))?;
        if fork["cancelled"] == true {
            return Err("Edit cancelled".into());
        }
        let state = rpc.call("get_state", json!({}))?;
        return pi_binding(&state).ok_or_else(|| "Pi did not expose the forked session".into());
    }
    rpc.call("initialize", json!({"clientInfo":{"name":"monocode_native","title":"MonoCode Native","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}))?;
    write_json(&mut rpc.wire.stdin, json!({"method":"initialized"}))?;
    let resumed = rpc.call(
        "thread/resume",
        json!({"threadId":session,"cwd":request.cwd}),
    )?;
    if resumed["thread"]["id"].as_str() != Some(session) {
        return Err("Codex resumed a different thread; history was preserved".into());
    }
    if resumed["thread"]["turns"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|turn| matches!(turn["status"].as_str(), Some("inProgress" | "running")))
    {
        return Err("Stop the current turn before editing the last message".into());
    }
    let before_turn = if let Some(id) = before_turn.filter(|id| !id.trim().is_empty()) {
        id.to_owned()
    } else {
        let page = rpc.call(
            "thread/turns/list",
            json!({"threadId":session,"limit":100,"sortDirection":"desc","itemsView":"summary"}),
        )?;
        page["data"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|turn| {
                turn["items"]
                    .as_array()
                    .is_some_and(|items| items.iter().any(|item| item["type"] == "userMessage"))
            })
            .and_then(|turn| turn["id"].as_str())
            .ok_or("Codex did not expose a user turn id to edit")?
            .to_owned()
    };
    rpc.call(
        "thread/revert",
        json!({"threadId":session,"beforeTurnId":before_turn}),
    )?;
    Ok(session.into())
}

fn last_fork_entry(data: &Value) -> Result<String, String> {
    data["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .rev()
        .find_map(|item| item["entryId"].as_str().filter(|id| !id.trim().is_empty()))
        .map(str::to_owned)
        .ok_or_else(|| "No user message to edit".into())
}
fn pi_binding(state: &Value) -> Option<String> {
    let id = state["sessionId"]
        .as_str()
        .or_else(|| state["session_id"].as_str())?
        .trim();
    (!id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
    .then(|| id.to_owned())
}
struct Rpc<'a> {
    wire: &'a mut parked::Wire,
    serial: u64,
    pi: bool,
}
impl Rpc<'_> {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, RewindError> {
        let mutating = method == "thread/revert";
        self.serial += 1;
        let id = if self.pi {
            json!(format!("native-edit-{}", self.serial))
        } else {
            json!(self.serial)
        };
        let message = if self.pi {
            let mut message = params;
            message["id"] = id.clone();
            message["type"] = method.into();
            message
        } else {
            json!({"id":id,"method":method,"params":params})
        };
        write_json(&mut self.wire.stdin, message)
            .map_err(|error| transport_error(error, mutating))?;
        let started = Instant::now();
        loop {
            let timeout = self
                .pi
                .then(|| Duration::from_secs(if method == "get_state" { 45 } else { 15 }));
            if timeout.is_some_and(|timeout| started.elapsed() > timeout) {
                return Err(transport_error(
                    format!("Provider did not confirm {method}; local history was preserved"),
                    mutating,
                ));
            }
            match self.wire.rx.recv_timeout(TICK) {
                Ok(Ok(line)) => {
                    let Ok(value) = serde_json::from_slice::<Value>(&line) else {
                        continue;
                    };
                    if !self.pi
                        && value["method"] == "currentTime/read"
                        && value.get("id").is_some()
                    {
                        write_json(
                            &mut self.wire.stdin,
                            json!({"id":value["id"],"result":{"currentTimeAt":crate::time::now_ms()/1000}}),
                        )?;
                        continue;
                    }
                    if value["id"] != id {
                        continue;
                    }
                    if let Some(error) = value
                        .get("error")
                        .filter(|error| !self.pi && !error.is_null())
                    {
                        return Err(codex_error(error, "Provider rejected rewind").into());
                    }
                    if self.pi {
                        if value["type"] != "response" {
                            continue;
                        }
                        if value["success"] != true {
                            return Err(value["error"]
                                .as_str()
                                .unwrap_or("Provider rejected rewind")
                                .into());
                        }
                        return Ok(value["data"].clone());
                    }
                    if let Some(result) = value.get("result") {
                        return Ok(result.clone());
                    }
                }
                Ok(Err(error)) => return Err(transport_error(error, mutating)),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(transport_error(
                        format!(
                            "Provider exited before confirming {method}; local history was preserved"
                        ),
                        mutating,
                    ));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lost_destructive_confirmation_requires_recovery_but_explicit_rejection_does_not() {
        assert!(transport_error("Lost reply".into(), true).provider_may_have_changed);
        assert!(!transport_error("Resume failed".into(), false).provider_may_have_changed);
        assert!(!RewindError::from("Provider rejected rewind").provider_may_have_changed);
    }
    #[test]
    fn only_native_transports_with_an_explicit_rewind_protocol_advertise_support() {
        for kind in Kind::ALL {
            assert_eq!(
                supports_rewind(kind),
                matches!(
                    kind,
                    Kind::Codex | Kind::Pi | Kind::Omp | Kind::OpenCode | Kind::Claude
                )
            );
        }
    }
    #[test]
    fn claude_live_session_follows_the_record_the_cli_keeps_for_its_process() {
        let root = tempfile::tempdir().unwrap();
        let account = crate::accounts::Profile {
            id: "work".into(),
            root: Some(root.path().to_owned()),
        };
        let sessions = account.home(Kind::Claude).unwrap().join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let id = "9b5b21da-dd32-4809-b5c4-fef6dd3aac13";
        assert_eq!(claude_live_session(&account, 7), None);
        std::fs::write(
            sessions.join("7.json"),
            json!({"pid":7,"sessionId":id}).to_string(),
        )
        .unwrap();
        assert_eq!(claude_live_session(&account, 7).as_deref(), Some(id));
        // Another process's record, or something that is not a session id.
        std::fs::write(
            sessions.join("8.json"),
            json!({"pid":7,"sessionId":id}).to_string(),
        )
        .unwrap();
        assert_eq!(claude_live_session(&account, 8), None);
        std::fs::write(
            sessions.join("7.json"),
            json!({"pid":7,"sessionId":"--fork"}).to_string(),
        )
        .unwrap();
        assert_eq!(claude_live_session(&account, 7), None);
    }
    fn claude_line(id: &str, parent: Option<&str>, kind: &str, content: Value) -> String {
        json!({"uuid":id,"parentUuid":parent,"type":kind,"isSidechain":false,"message":{"content":content}})
            .to_string()
    }
    #[test]
    fn claude_boundary_is_the_entry_before_the_last_prompt_on_the_active_chain() {
        let transcript = [
            claude_line("p1", None, "user", json!("First")),
            claude_line("a1", Some("p1"), "assistant", json!([{"type":"text","text":"One"}])),
            claude_line("x1", Some("a1"), "attachment", Value::Null),
            claude_line("p2", Some("x1"), "user", json!([{"type":"text","text":"Second\n  prompt"}])),
            claude_line("a2", Some("p2"), "assistant", json!([{"type":"tool_use"}])),
            claude_line("t2", Some("a2"), "user", json!([{"type":"tool_result","content":"ok"}])),
            claude_line("i2", Some("t2"), "user", json!([{"type":"text","text":"[Request interrupted by user]"}])),
            // A subagent's prompt is not part of the conversation being edited.
            json!({"uuid":"s1","parentUuid":null,"type":"user","isSidechain":true,"message":{"content":"Side"}}).to_string(),
            "{\"type\":\"last-prompt\"}".into(),
        ]
        .join("\n");
        assert_eq!(
            claude_boundary(&transcript, "Second prompt").unwrap(),
            ("x1".into(), "p2".into())
        );
        // The local message must be the one Claude stored last.
        assert!(claude_boundary(&transcript, "First").is_err());
        let first = claude_line("p1", None, "user", json!("First"));
        assert!(
            claude_boundary(&first, "First")
                .unwrap_err()
                .contains("first message")
        );
        assert!(claude_boundary("", "First").is_err());
    }
    #[test]
    fn claude_binding_carries_a_pending_rewind_until_the_fork_replaces_it() {
        assert_eq!(claude_resume("session"), ("session", None));
        assert_eq!(
            claude_resume("session?at=before&drops=prompt"),
            ("session", Some(("before", "prompt")))
        );
        let args = |binding| {
            let mut command = Command::new("claude");
            claude_resume_args(&mut command, binding);
            command
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(args("session"), ["--resume", "session"]);
        assert_eq!(
            args("session?at=before&drops=prompt"),
            [
                "--resume",
                "session",
                "--resume-session-at",
                "before",
                "--resume-drops-turn",
                "prompt",
                "--fork-session"
            ]
        );
    }
    #[test]
    fn pi_fork_uses_the_last_valid_user_entry_and_rejects_missing_entries() {
        assert_eq!(
            last_fork_entry(&json!({"messages":[{"entryId":"first"},{"entryId":"last"},{}]}))
                .unwrap(),
            "last"
        );
        assert!(last_fork_entry(&json!({"messages":[]})).is_err());
        assert!(last_fork_entry(&json!([])).is_err());
    }
    #[test]
    fn pi_binding_never_persists_a_session_file_as_an_id() {
        assert_eq!(
            pi_binding(&json!({"sessionId":"fork-123"})),
            Some("fork-123".into())
        );
        assert_eq!(pi_binding(&json!({"sessionId":"/tmp/session.jsonl"})), None);
        assert_eq!(pi_binding(&json!({"sessionId":" "})), None);
    }
}
