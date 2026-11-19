use super::*;

/// A bounded, separate conversation for titles and other background text work.
/// Never resume the user's conversation or inherit its app capabilities/tools.
pub fn run_text(mut request: Request, cancel: Arc<AtomicBool>) -> Result<String, String> {
    request.provider_id = None;
    request.settings = Settings {
        account: request.settings.account.clone(),
        text_only: true,
        access: AccessMode::ReadOnly,
        effort: (request.kind == Kind::Codex).then(|| "low".into()),
        ..Default::default()
    };
    let mut output = String::new();
    run(request, cancel, |events| {
        for event in events {
            match event {
                Event::Text(text) if output.len() + text.len() <= 64 * 1024 => {
                    output.push_str(&text)
                }
                Event::Text(_) => return Err("Background text response exceeded its limit".into()),
                Event::Tool(tool) if tool.is_compaction() => {}
                Event::Tool(_) | Event::Approval(_) | Event::Question(_) => {
                    return Err(
                        "Background text request attempted to use tools or request input".into(),
                    );
                }
                _ => {}
            }
        }
        Ok(())
    })?;
    Ok(output)
}

pub(super) fn run_operation(
    request: Request,
    cancel: Arc<AtomicBool>,
    controls: Option<mpsc::Receiver<Control>>,
    probe: bool,
    compact: bool,
    command_probe: bool,
    mut emit: impl FnMut(Vec<Event>) -> Result<(), String>,
) -> Result<Option<Value>, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Stopped".into());
    }
    if request.kind == Kind::OpenCode {
        return opencode::run(request, cancel, controls, probe, compact, emit);
    }
    let claude_new_session = (request.kind == Kind::Claude
        && request.provider_id.is_none()
        && !request.settings.text_only)
        .then(lifecycle::claude_session_id);
    #[cfg(unix)]
    if std::env::var_os("MONOCODE_NATIVE_TRACE").is_some() {
        unsafe {
            let mut signals = std::mem::zeroed();
            libc::pthread_sigmask(libc::SIG_SETMASK, std::ptr::null(), &mut signals);
            eprintln!(
                "provider-launch sigchld_blocked={}",
                libc::sigismember(&signals, libc::SIGCHLD)
            );
        }
    }
    let mut command = Command::new(&request.binary);
    crate::vcs::git::scm::launch_env::apply_gui_env(&mut command);
    command.envs(&request.settings.environment);
    request.settings.account.apply(request.kind, &mut command)?;
    match request.kind {
        Kind::Codex => {
            command.arg("app-server");
            crate::provider::visual_bridge::configure_codex(
                &mut command,
                &request.settings.environment,
            );
            crate::mono::bridge::configure_codex(
                &mut command,
                request.settings.mono.as_ref(),
                &request.settings.environment,
            );
            if let Some(mono) = &request.settings.mono {
                if let Some(home) =
                    crate::mono::codex_home(&mono.root, &request.settings.account.id)?
                {
                    let sqlite_home = serde_json::to_string(&home).map_err(|e| e.to_string())?;
                    command
                        .env("CODEX_HOME", &home)
                        .env("CODEX_SQLITE_HOME", &home)
                        .args(["-c", &format!("sqlite_home={sqlite_home}")]);
                }
            }
        }
        Kind::Claude => {
            command.args([
                "--verbose",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--include-partial-messages",
                "--permission-prompt-tool",
                "stdio",
            ]);
            let settings = &request.settings;
            let mode = if settings.plan {
                "plan"
            } else {
                settings.access.claude()
            };
            command.args(["--permission-mode", mode]);
            if settings.text_only {
                command.args([
                    "--tools",
                    "",
                    "--max-turns",
                    "1",
                    "--no-session-persistence",
                    "--strict-mcp-config",
                    "--mcp-config",
                    "{\"mcpServers\":{}}",
                ]);
            }
            let visual_server = (!settings.text_only)
                .then(|| crate::provider::visual_bridge::mcp_server(&settings.environment))
                .flatten();
            let mut allowed = Vec::new();
            if !settings.text_only && settings.access == AccessMode::ReadOnly {
                command.args(["--tools", "Read,Glob,Grep,Bash"]);
                allowed.push("Read,Glob,Grep,Bash(git status *),Bash(git diff *),Bash(git log *),Bash(git show *),Bash(pwd),Bash(ls *),Bash(rg *)");
            }
            let app_server = (!settings.text_only)
                .then(|| {
                    crate::mono::bridge::mcp_server(settings.mono.as_ref(), &settings.environment)
                })
                .flatten();
            let mut servers = serde_json::Map::new();
            if let Some(server) = app_server {
                servers.insert(crate::mono::bridge::MCP_SERVER.into(), server);
                allowed.push(crate::mono::bridge::CLAUDE_ALLOWED_TOOLS);
            }
            let mut system = Vec::new();
            if let Some(mono) = settings.mono.as_ref().filter(|_| !settings.text_only) {
                system.push(crate::mono::instructions(&mono.mono, &mono.executable));
            }
            if let Some(server) = visual_server {
                servers.insert("monocode_visual".into(), server);
                allowed.push(crate::provider::visual_bridge::CLAUDE_ALLOWED_TOOLS);
                if let Some(instructions) =
                    crate::provider::visual_bridge::instructions(&settings.environment, true)
                {
                    system.push(instructions);
                }
            }
            if !system.is_empty() {
                command.args(["--append-system-prompt", &system.join("\n\n")]);
            }
            if !servers.is_empty() {
                command.args(["--mcp-config", &json!({"mcpServers":servers}).to_string()]);
            }
            if !allowed.is_empty() {
                command.args(["--allowedTools", &allowed.join(",")]);
            }
            if mode == "bypassPermissions" {
                command.arg("--allow-dangerously-skip-permissions");
            }
            let mut config = json!({});
            if settings.disable_claude_hooks || settings.text_only {
                config["disableAllHooks"] = true.into();
            }
            if settings.fast {
                config["fastMode"] = true.into();
            }
            if settings.thinking == Some(true) {
                config["alwaysThinkingEnabled"] = true.into();
            }
            if let Some(effort) = settings.effort.as_deref().filter(|e| *e != "ultrathink") {
                let effort = if effort == "ultracode" {
                    config["ultracode"] = true.into();
                    "xhigh"
                } else if effort == "xhigh"
                    && request.model.as_deref().is_some_and(|m| {
                        matches!(
                            m,
                            "claude-opus-4-6"
                                | "claude-sonnet-4-6"
                                | "claude-opus-4-5"
                                | "claude-haiku-4-5"
                                | "claude-opus-4-1"
                                | "claude-opus-4-0"
                                | "claude-sonnet-4-0"
                                | "claude-sonnet-4-5"
                        )
                    })
                {
                    "max"
                } else if effort == "max" && request.model.as_deref() == Some("claude-sonnet-4-6") {
                    "high"
                } else {
                    effort
                };
                command.args(["--effort", effort]);
            }
            command.args(["--settings", &config.to_string()]);
            if !settings.text_only {
                command.arg("--setting-sources=user,project,local");
            }
            if let Some(id) = &request.provider_id {
                rewind::claude_resume_args(&mut command, id);
            } else if let Some(id) = &claude_new_session {
                command.args(["--session-id", id]);
            }
            if let Some(model) = &request.model {
                let model = if settings
                    .model_settings
                    .get("context")
                    .is_some_and(|value| value == "1m")
                    && !model.ends_with("[1m]")
                {
                    format!("{model}[1m]")
                } else {
                    model.clone()
                };
                command.args(["--model", &model]);
            }
        }
        kind if kind.is_pi() => pi::configure(&mut command, &request, probe || command_probe),
        _ => acp::configure(&mut command, &request)?,
    }
    command
        .current_dir(if request.kind == Kind::Antigravity {
            request.binary.parent().unwrap_or(&request.cwd)
        } else {
            &request.cwd
        })
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process::isolate(&mut command);
    if cancel.load(Ordering::Relaxed) {
        return Err("Stopped".into());
    }
    let eligible = !probe && !command_probe && !request.settings.text_only;
    let cached = eligible
        .then(|| parked::take(&parked::key(&request)))
        .flatten();
    let (mut wire, mut protocol, warm) = if let Some(cached) = cached {
        let mut protocol = Protocol::new(request);
        protocol.acp = cached.protocol.acp;
        protocol.pi = cached.protocol.pi;
        protocol.previous_turn = cached.protocol.active_turn;
        (cached.wire, protocol, true)
    } else {
        if eligible {
            // A new process supersedes older warm launch configurations and
            // any cancelled process still draining for this provider binding.
            parked::discard_session(&request);
        }
        (
            parked::Wire::spawn(&mut command, &request)?,
            Protocol::new(request),
            false,
        )
    };
    let rx = &wire.rx;
    let tail = &wire.tail;
    let mut stdin = &mut wire.stdin;
    let mut pending = Vec::new();
    if let Some(id) = claude_new_session {
        protocol.request.provider_id = Some(id.clone());
        pending.push(Event::SessionId(id));
    }
    let mut stop_confirmed = false;
    let result = (|| {
        protocol.probe = probe;
        protocol.compact = compact;
        protocol.command_probe = command_probe;
        if warm {
            if protocol.request.kind.is_acp() {
                protocol.acp_resume_warm(&mut stdin)?;
            } else if protocol.request.kind.is_pi() {
                protocol.pi_start(&mut stdin)?;
            } else if protocol.request.kind == Kind::Codex {
                protocol.initialized = true;
                protocol.frame(json!({"id":1,"result":{}}), &mut stdin)?;
            } else {
                protocol.follow_up_ready = true;
                protocol.frame(
                    json!({"type":"control_response","response":{"request_id":"native-init"}}),
                    &mut stdin,
                )?;
            }
        } else if protocol.request.kind.is_acp() {
            protocol.acp_start(&mut stdin)?;
        } else if protocol.request.kind.is_pi() {
            protocol.pi_start(&mut stdin)?;
        } else if protocol.request.kind == Kind::Codex {
            write_json(
                &mut stdin,
                json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"monocode_native","title":"MonoCode Native","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}}),
            )?;
        } else {
            write_json(
                &mut stdin,
                json!({"type":"control_request","request_id":"native-init","request":{"subtype":"initialize"}}),
            )?;
        }
        let mut pending_bytes = 0;
        let mut last_emit = Instant::now();
        let started = Instant::now();
        let mut ready_emitted = false;
        let mut interrupted = false;
        loop {
            if cancel.load(Ordering::Relaxed) && !interrupted {
                interrupted = true;
                if !protocol.interrupt(&mut stdin)? {
                    return Err("Stopped".into());
                }
                if protocol.request.kind == Kind::Claude {
                    return Err("Stopped".into());
                }
            }
            if !interrupted {
                if let Some(controls) = &controls {
                    while let Ok(control) = controls.try_recv() {
                        if let Decision::Steer(message) = control.decision {
                            let id = message.id.clone();
                            match protocol.steer(message, &mut stdin) {
                                Ok(events) => pending.extend(events),
                                Err(error) => pending.push(Event::FollowUpFailed { id, error }),
                            }
                        } else {
                            protocol.respond(control, &mut stdin)?;
                        }
                    }
                }
                pending.extend(protocol.expire_steering());
                pending.extend(protocol.lifecycle_tick(started, &mut stdin)?);
                if protocol.ready() && !ready_emitted {
                    ready_emitted = true;
                    pending.push(Event::Ready);
                }
            }
            if interrupted && protocol.request.kind.is_pi() {
                let _ = protocol.pi_tick(&mut stdin);
            }
            if interrupted && protocol.done {
                stop_confirmed = true;
                return Err("Stopped".into());
            }
            if protocol.request.settings.text_only && started.elapsed() >= Duration::from_secs(45) {
                return Err("Background text request timed out".into());
            }
            match rx.recv_timeout(TICK) {
                Ok(Ok(line)) => {
                    let value: Value = match serde_json::from_slice(&line) {
                        Ok(value) => value,
                        // Some CLIs print startup banners before their NDJSON stream.
                        Err(_)
                            if !line
                                .iter()
                                .copied()
                                .find(|b| !b.is_ascii_whitespace())
                                .is_some_and(|b| b == b'{') =>
                        {
                            continue;
                        }
                        Err(e) => return Err(format!("Invalid provider JSON: {e}")),
                    };
                    if std::env::var_os("MONOCODE_NATIVE_TRACE").is_some() {
                        let method = value["method"].as_str().unwrap_or("");
                        let item = &value["params"]["item"];
                        if !method.is_empty()
                            && !method.ends_with("Delta")
                            && !method.ends_with("/delta")
                        {
                            eprintln!(
                                "provider-frame method={method} type={} status={} id={} output_bytes={} keys={:?} legacy={}",
                                item["type"],
                                item["status"],
                                item["id"],
                                item["aggregatedOutput"].as_str().map_or(0, str::len),
                                value["params"]
                                    .as_object()
                                    .map(|o| o.keys().collect::<Vec<_>>()),
                                value["params"]["msg"]["type"]
                            );
                        }
                    }
                    if interrupted {
                        let reply_id = value["id"].as_str();
                        let turn_id = value["params"]["turnId"]
                            .as_str()
                            .or_else(|| value["params"]["turn"]["id"].as_str());
                        let terminal = matches!(
                            value["method"].as_str(),
                            Some("turn/completed" | "turn/aborted")
                        ) && (turn_id.is_some()
                            && turn_id == protocol.active_turn.as_deref()
                            || turn_id.is_none() && protocol.previous_turn.is_none());
                        let codex_ack = protocol.request.kind == Kind::Codex
                            && (reply_id.is_some() && reply_id == protocol.interrupt_id.as_deref()
                                || terminal);
                        let claude_end =
                            protocol.request.kind == Kind::Claude && claude_turn_result(&value);
                        if codex_ack || claude_end {
                            stop_confirmed = value.get("error").is_none_or(Value::is_null);
                            if terminal || claude_end {
                                protocol.done = true;
                            }
                            return Err("Stopped".into());
                        }
                    }
                    let events = match protocol.frame(value, &mut stdin) {
                        Ok(events) => events,
                        Err(_) if interrupted => return Err("Stopped".into()),
                        Err(error) => return Err(error),
                    };
                    for event in events {
                        if controls.is_none() || interrupted {
                            if let Event::Approval(approval) = &event {
                                protocol.respond(
                                    Control {
                                        id: approval.id.clone(),
                                        decision: Decision::Deny,
                                    },
                                    &mut stdin,
                                )?;
                            }
                            if let Event::Question(question) = &event {
                                protocol.respond(
                                    Control {
                                        id: question.id.clone(),
                                        decision: Decision::Deny,
                                    },
                                    &mut stdin,
                                )?;
                            }
                        }
                        if interrupted {
                            continue;
                        }
                        pending_bytes += match &event {
                            Event::Text(text) | Event::Reasoning(text) | Event::Status(text) => {
                                text.len()
                            }
                            Event::Tool(tool) => tool.input.len() + tool.output.len(),
                            Event::Approval(approval) => approval.detail.len(),
                            Event::Question(question) => {
                                question.items.iter().map(|q| q.prompt.len()).sum()
                            }
                            _ => 64,
                        };
                        if let Event::SessionId(id) = &event {
                            protocol.request.provider_id = Some(id.clone());
                        }
                        pending.push(event);
                    }
                }
                Ok(Err(error)) => return Err(error),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if interrupted {
                        return Err("Stopped".into());
                    }
                    return Err(format!(
                        "Provider exited before completing the turn. {}",
                        String::from_utf8_lossy(&tail.lock().unwrap())
                    ));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if !pending.is_empty()
                && (last_emit.elapsed() >= TICK || pending_bytes >= BATCH_BYTES || protocol.done)
            {
                emit(std::mem::take(&mut pending))?;
                pending_bytes = 0;
                last_emit = Instant::now();
            }
            // A visual publication may arrive while the provider is waiting
            // for its tool result and therefore emits no stream events.
            if crate::provider::visual_bridge::configured(&protocol.request.settings.environment) {
                emit(Vec::new())?;
            }
            if protocol.done {
                if interrupted {
                    stop_confirmed = true;
                    return Err("Stopped".into());
                }
                return Ok(protocol.catalog_result.take());
            }
            if command_probe && started.elapsed() > Duration::from_secs(45) {
                return Err("Command discovery timed out".into());
            }
            let discovery_timeout = match protocol.request.kind {
                Kind::Cursor | Kind::Grok => Some(15),
                Kind::Hermes => Some(30),
                Kind::Pi | Kind::Omp => Some(45),
                Kind::Antigravity => None,
                _ => Some(60),
            };
            if probe
                && discovery_timeout
                    .is_some_and(|seconds| started.elapsed() > Duration::from_secs(seconds))
            {
                return Err("Model discovery timed out".into());
            }
        }
    })();
    // Cancellation can arrive while publishing an event batch. Its callback
    // then returns Stopped before the next receive-loop iteration can interrupt.
    if cancel.load(Ordering::Relaxed) && protocol.interrupt_id.is_none() {
        let _ = protocol.interrupt(&mut wire.stdin);
    }
    if cancel.load(Ordering::Relaxed) && protocol.done {
        stop_confirmed = true;
    }
    if result.is_err() && !cancel.load(Ordering::Relaxed) {
        if let Some(limit) = protocol.claude_usage_limit.take() {
            pending.push(Event::UsageLimited(limit));
        }
    }
    if !pending.is_empty() {
        let _ = emit(std::mem::take(&mut pending));
    }
    if eligible
        && cancel.load(Ordering::Relaxed)
        && !protocol.done
        && protocol.request.kind != Kind::Antigravity
        && protocol.interrupt_id.is_some()
    {
        parked::settle_cancelled(wire, protocol);
    } else if eligible
        && ((result.is_ok() && !cancel.load(Ordering::Relaxed))
            || (stop_confirmed && protocol.request.kind != Kind::Antigravity))
    {
        parked::put(wire, protocol);
    }
    result
}
