//! ACP lifecycle shared by Cursor, Grok, OpenCode, Antigravity, fx, Hermes and Devin.
//! Mirrors the ACP adapters in src/integrations/harness/providers. Providers own
//! filesystem/terminal access; we never claim client capabilities we cannot serve.
mod frames;
mod options;
use super::*;
#[cfg(test)]
use frames::usage_events;
use options::config_id;
use std::collections::VecDeque;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Stage {
    #[default]
    Initialize,
    Authenticate,
    ListModels,
    Resume,
    Load,
    New,
    Configure,
    Prompt,
    Background,
    ThoughtProbe,
    DeleteProbe,
}
struct BackgroundDispatch {
    call_id: String,
    delegation_id: String,
    transcripts: Vec<String>,
}
#[derive(Default)]
pub(super) struct State {
    sequence: u64,
    stage: Stage,
    session: String,
    options: VecDeque<(&'static str, Value)>,
    message: usize,
    setup: Value,
    settings_applied: bool,
    tool_kinds: HashMap<String, String>,
    pending_method: &'static str,
    pending_params: Value,
    deadline: Option<Instant>,
    cancelling: bool,
    background: HashMap<String, BackgroundDispatch>,
    background_poll: Option<Instant>,
    spent_effort: bool,
    probe_models: VecDeque<String>,
    probe_budget: Option<Instant>,
    notices: Vec<Event>,
    stale_model: Option<String>,
    mode: Option<&'static str>,
}

pub(super) fn configure(command: &mut Command, request: &Request) -> Result<(), String> {
    let settings = &request.settings;
    match request.kind {
        Kind::Antigravity => {
            if cfg!(target_os = "linux") {
                command.arg("--uid=");
            }
        }
        Kind::Grok => {
            command.arg("--no-auto-update");
            if settings.plan || settings.access == AccessMode::ReadOnly {
                command.args(["--permission-mode", "plan"]);
            }
            command.args(["agent", "--no-leader"]);
            if let Some(model) = &request.model {
                command.args(["--model", model]);
            }
            if let Some(effort) = &settings.effort {
                command.args(["--reasoning-effort", effort]);
            }
            if settings.access == AccessMode::FullAccess && !settings.plan {
                command.arg("--always-approve");
            }
            command.arg("stdio");
            command.env("GROK_NO_AUTO_UPDATE", "1");
        }
        Kind::Devin => {
            if settings.access == AccessMode::Supervised {
                command.args(super::devin::supervised_args()?);
            }
            command.arg("acp");
        }
        Kind::Cursor => {
            command.arg("acp");
        }
        Kind::Fx => {
            command.arg("acp");
            if let Some(model) = &request.model {
                command.args(["--model", model]);
            }
        }
        _ => {
            command.arg("acp");
        }
    }
    Ok(())
}

impl Protocol {
    /// The native visual tools, plus a Mono's app tool.
    fn mcp_servers(&self) -> Value {
        let settings = &self.request.settings;
        let mut servers = crate::provider::visual_bridge::acp_servers(&settings.environment);
        if let (Some(list), Some(app)) = (
            servers.as_array_mut(),
            crate::mono::bridge::acp_server(settings.mono.as_ref(), &settings.environment),
        ) {
            list.push(app);
        }
        servers
    }
    pub(super) fn acp_cancel(&mut self, stdin: &mut impl Write) -> Result<(), String> {
        self.acp.cancelling = true;
        self.acp.deadline = None;
        self.acp.background.clear();
        if matches!(self.request.kind, Kind::Antigravity | Kind::Devin) {
            for (_, (wire, _, _)) in self.approvals.drain() {
                write_json(
                    stdin,
                    json!({"jsonrpc":"2.0","id":wire,"result":{"outcome":{"outcome":"cancelled"}}}),
                )?;
            }
        }
        for id in self
            .approvals
            .keys()
            .chain(self.questions.keys())
            .cloned()
            .collect::<Vec<_>>()
        {
            self.respond(
                Control {
                    id,
                    decision: Decision::Deny,
                },
                stdin,
            )?;
        }
        // Tauri rejects the local prompt promise immediately after notifying;
        // ACP does not promise a response to a cancelled prompt.
        self.done = true;
        if self.acp.session.is_empty() {
            return Ok(());
        }
        write_json(
            stdin,
            json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":self.acp.session}}),
        )
    }

    /// ACP deadlines apply to individual requests, never to a quiet stream.
    pub(super) fn acp_tick(&mut self, stdin: &mut impl Write) -> Result<Vec<Event>, String> {
        if self
            .acp
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.acp.deadline = None;
            let method = self.acp.pending_method;
            self.acp_failure(
                &json!({"message":format!("{method} timed out")}),
                true,
                stdin,
            )?;
        }
        if self.acp.stage == Stage::Background
            && self
                .acp
                .background_poll
                .is_none_or(|poll| Instant::now() >= poll)
        {
            self.acp.background_poll = Some(Instant::now() + Duration::from_millis(500));
            let mut events = self.acp_background(stdin)?;
            events.append(&mut self.acp.notices);
            return Ok(events);
        }
        Ok(std::mem::take(&mut self.acp.notices))
    }

    pub(super) fn acp_active_session(&self) -> Option<&str> {
        (matches!(self.acp.stage, Stage::Prompt | Stage::Background)
            && !self.acp.session.is_empty())
        .then_some(self.acp.session.as_str())
    }

    fn acp_send(
        &mut self,
        stage: Stage,
        method: &'static str,
        params: Value,
        stdin: &mut impl Write,
    ) -> Result<(), String> {
        self.acp.sequence += 1;
        self.acp.stage = stage;
        self.acp.pending_method = method;
        self.acp.pending_params = params.clone();
        let seconds = if self.probe {
            match (self.request.kind, stage) {
                (Kind::Devin, Stage::ThoughtProbe) => Some(5),
                (Kind::Cursor | Kind::Grok | Kind::Antigravity, _) => Some(12),
                (Kind::Hermes, _) => Some(20),
                (Kind::Devin, _) => Some(30),
                _ => None,
            }
        } else {
            match (self.request.kind, stage) {
                (Kind::Cursor, _) => None,
                (Kind::Grok | Kind::Fx | Kind::Antigravity, Stage::Initialize) => Some(12),
                (Kind::Hermes | Kind::Devin, Stage::Initialize) => Some(20),
                (Kind::Grok, Stage::Authenticate) => Some(15),
                (Kind::Devin, Stage::Authenticate) => {
                    Some(if params["_meta"]["api_key"].is_string() {
                        20
                    } else {
                        300
                    })
                }
                (Kind::Devin, Stage::New) if self.probe => Some(30),
                (_, Stage::New | Stage::Load | Stage::Resume) => Some(45),
                (Kind::Grok | Kind::Fx | Kind::Antigravity, Stage::Configure) => Some(15),
                (Kind::Hermes | Kind::Devin, Stage::Configure) => Some(20),
                (_, Stage::Prompt) => Some(30 * 60),
                (Kind::Devin, Stage::ThoughtProbe) => Some(5),
                (Kind::Devin, Stage::DeleteProbe) => Some(30),
                _ => None,
            }
        };
        self.acp.deadline = seconds.map(|seconds| Instant::now() + Duration::from_secs(seconds));
        write_json(
            stdin,
            json!({"jsonrpc":"2.0","id":self.acp.sequence,"method":method,"params":params}),
        )
    }
    pub(super) fn acp_resume_warm(&mut self, stdin: &mut impl Write) -> Result<(), String> {
        self.acp.message = 0;
        self.acp.cancelling = false;
        self.acp.options.clear();
        self.acp.settings_applied = false;
        self.acp.spent_effort = false;
        let setup = self.acp.setup.clone();
        self.acp_options(&setup)?;
        self.acp_next(stdin)
    }
    pub(super) fn acp_start(&mut self, stdin: &mut impl Write) -> Result<(), String> {
        let mut params = json!({"protocolVersion":1,"clientInfo":{"name":"monocode","version":env!("CARGO_PKG_VERSION")},"clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false,"session":{"configOptions":{"boolean":{}}}}});
        if self.request.kind == Kind::Cursor {
            params["clientCapabilities"]["_meta"] = json!({"parameterizedModelPicker":true});
        }
        self.acp_send(Stage::Initialize, "initialize", params, stdin)
    }
    fn acp_session(&mut self, stdin: &mut impl Write) -> Result<(), String> {
        let mut params = json!({"cwd":self.request.cwd,"mcpServers":self.mcp_servers()});
        if let Some(id) = &self.request.provider_id {
            params["sessionId"] = id.clone().into();
            if matches!(self.request.kind, Kind::Grok | Kind::Fx | Kind::Antigravity) {
                if self.request.kind != Kind::Antigravity {
                    params = json!({"sessionId":id});
                }
                return self.acp_send(Stage::Resume, "session/resume", params, stdin);
            }
            return self.acp_send(Stage::Load, "session/load", params, stdin);
        }
        self.acp_new(stdin)
    }
    fn acp_new(&mut self, stdin: &mut impl Write) -> Result<(), String> {
        let mut params = json!({"cwd":self.request.cwd,"mcpServers":self.mcp_servers()});
        if self.request.kind == Kind::Grok {
            if self.request.settings.access == AccessMode::FullAccess {
                params["_meta"] = json!({"yoloMode":true});
            }
            if self.request.settings.access == AccessMode::Auto {
                params["_meta"] = json!({"autoMode":true});
            }
        }
        self.acp_send(Stage::New, "session/new", params, stdin)
    }
    fn acp_next(&mut self, stdin: &mut impl Write) -> Result<(), String> {
        if let Some((method, params)) = self.acp.options.pop_front() {
            if method == "session/set_config_option"
                && self.acp.setup["configOptions"]
                    .as_array()
                    .is_some_and(|options| {
                        options.iter().any(|option| {
                            config_id(option) == params["configId"].as_str().unwrap_or("")
                                && option.get("currentValue").is_some_and(|value| {
                                    value
                                        .as_str()
                                        .map(str::to_owned)
                                        .unwrap_or_else(|| value.to_string())
                                        == params["value"]
                                            .as_str()
                                            .map(str::to_owned)
                                            .unwrap_or_else(|| params["value"].to_string())
                                })
                        })
                    })
            {
                return self.acp_next(stdin);
            }
            self.acp_send(Stage::Configure, method, params, stdin)
        } else if !self.acp.settings_applied {
            self.acp.settings_applied = true;
            let setup = self.acp.setup.clone();
            self.acp_settings(&setup);
            self.acp_next(stdin)
        } else if let Some(mode) = self.acp.mode.take() {
            self.acp_send(
                Stage::Configure,
                "session/set_mode",
                json!({"sessionId":self.acp.session,"modeId":mode}),
                stdin,
            )
        } else if self.compact && self.request.kind == Kind::Grok {
            self.acp_send(
                Stage::Prompt,
                "_x.ai/compact_conversation",
                json!({"sessionId":self.acp.session}),
                stdin,
            )
        } else {
            self.acp_send(
                Stage::Prompt,
                "session/prompt",
                json!({"sessionId":self.acp.session,"prompt":self.request.acp_content()?}),
                stdin,
            )
        }
    }
    fn acp_failure(
        &mut self,
        error: &Value,
        timed_out: bool,
        stdin: &mut impl Write,
    ) -> Result<(), String> {
        let kind = self.request.kind;
        let detail = error["message"].as_str().unwrap_or("RPC request failed");
        if self.acp.cancelling {
            self.done = true;
            return Ok(());
        }
        match self.acp.stage {
            Stage::DeleteProbe => {
                self.done = true;
                return Ok(());
            }
            Stage::ThoughtProbe => {
                if timed_out {
                    self.acp.probe_models.clear();
                }
                return self.acp_probe_next(stdin);
            }
            Stage::ListModels => return self.acp_session(stdin),
            Stage::Authenticate if matches!(kind, Kind::Cursor | Kind::Grok) => {
                return self.acp_session(stdin);
            }
            Stage::Resume if !(kind == Kind::Antigravity && timed_out) => {
                return self.acp_send(Stage::Load, "session/load", json!({"sessionId":self.request.provider_id,"cwd":self.request.cwd,"mcpServers":self.mcp_servers()}), stdin);
            }
            Stage::Load => {
                let lower = detail.to_lowercase();
                let devin_missing = lower.contains("unknown session")
                    || lower.contains("no such session")
                    || (lower.contains("session") && lower.contains("not found"));
                let replace = match kind {
                    Kind::Cursor | Kind::Grok | Kind::Fx | Kind::Hermes => true,
                    Kind::Antigravity => !timed_out,
                    Kind::Devin => !timed_out && devin_missing,
                    _ => false,
                };
                if replace {
                    return self.acp_new(stdin);
                }
            }
            Stage::Configure => {
                let method = self.acp.pending_method;
                // Cursor's adapter falls back from config selection to the
                // older model method, then tolerates unsupported controls.
                if kind == Kind::Cursor {
                    if method == "session/set_config_option"
                        && self.acp.pending_params["value"]
                            == self
                                .request
                                .model
                                .as_deref()
                                .map(Value::from)
                                .unwrap_or(Value::Null)
                    {
                        return self.acp_send(
                            Stage::Configure,
                            "session/set_model",
                            json!({"sessionId":self.acp.session,"modelId":self.request.model}),
                            stdin,
                        );
                    }
                    return self.acp_next(stdin);
                }
                // Grok/fx tolerate unsupported configuration methods; broken
                // transports and request timeouts still retire the child.
                if matches!(kind, Kind::Grok | Kind::Fx)
                    && !timed_out
                    && !["timed out", "not running", "exited", "closed", "pipe"]
                        .iter()
                        .any(|marker| detail.to_lowercase().contains(marker))
                {
                    return self.acp_next(stdin);
                }
            }
            _ => {}
        }
        // A JSON-RPC failure is not evidence that the user needs to log in.
        Err(format!("{}: {detail}", kind.label()))
    }
    fn acp_probe_next(&mut self, stdin: &mut impl Write) -> Result<(), String> {
        if self
            .acp
            .probe_budget
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.acp.probe_models.clear();
        }
        if let Some(model) = self.acp.probe_models.pop_front() {
            self.acp_send(
                Stage::ThoughtProbe,
                "session/set_config_option",
                json!({"sessionId":self.acp.session,"configId":"model","value":model}),
                stdin,
            )
        } else {
            self.acp_send(
                Stage::DeleteProbe,
                "session/delete",
                json!({"sessionId":self.acp.session}),
                stdin,
            )
        }
    }
}

pub(super) fn content_text(value: &Value) -> String {
    if let Some(s) = value.as_str() {
        return s.into();
    }
    if let Some(values) = value.as_array() {
        return values
            .iter()
            .map(content_text)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
    }
    if let Some(text) = value["text"].as_str() {
        return text.into();
    }
    if let Some(content) = value.get("content") {
        return content_text(content);
    }
    if value["type"] == "diff" {
        return format!(
            "{}\n{}",
            value["path"].as_str().unwrap_or(""),
            value["newText"].as_str().unwrap_or("")
        );
    }
    if value.is_null() {
        String::new()
    } else {
        serde_json::to_string_pretty(value).unwrap_or_default()
    }
}

pub(super) fn respond(
    stdin: &mut impl Write,
    wire: Value,
    request: &Value,
    decision: Decision,
) -> Result<(), String> {
    let preference = match &decision {
        Decision::Allow => ["allow_once", "allow_always"],
        Decision::Session => ["allow_always", "allow_once"],
        Decision::Deny | Decision::KeepOpen | Decision::Answer(_) | Decision::Steer(_) => {
            ["reject_once", "reject_always"]
        }
    };
    let options: Vec<_> = request["options"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|option| {
            !(request["_monocodeOneTime"] == true
                && matches!(&decision, Decision::Allow | Decision::Session)
                && (option["kind"] == "allow_always"
                    || option["optionId"] == "allow_always"
                    || option["optionId"] == "allow-always"))
        })
        .collect();
    let option = preference
        .iter()
        .find_map(|kind| options.iter().find(|o| o["kind"] == *kind))
        .or_else(|| {
            preference.iter().find_map(|kind| {
                options
                    .iter()
                    .find(|o| o["optionId"] == *kind || o["optionId"] == kind.replace('_', "-"))
            })
        });
    let outcome = option
        .and_then(|o| o["optionId"].as_str().or_else(|| o["option_id"].as_str()))
        .map(|id| json!({"outcome":"selected","optionId":id}))
        .unwrap_or_else(|| json!({"outcome":"cancelled"}));
    write_json(
        stdin,
        json!({"jsonrpc":"2.0","id":wire,"result":{"outcome":outcome}}),
    )
}

#[cfg(test)]
mod tests;
