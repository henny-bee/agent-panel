use super::*;

impl Protocol {
    pub(super) fn new(request: Request) -> Self {
        Self {
            request,
            initialized: false,
            compact: false,
            command_probe: false,
            compact_confirmed: false,
            active_turn: None,
            previous_turn: None,
            interrupt_id: None,
            claude_prompt_sent: false,
            claude_result_seen: false,
            claude_usage_limit: None,
            claude_resume_at: None,
            follow_up_ready: false,
            steering: HashMap::new(),
            emitted: HashMap::new(),
            claude_message: 0,
            begun: false,
            done: false,
            tools: HashMap::new(),
            active_compaction: None,
            activities: tasks::Activities::default(),
            claude_tools: HashMap::new(),
            text_key: None,
            approvals: HashMap::new(),
            metrics: TurnMetrics::default(),
            usage_total: 0,
            acp: Default::default(),
            pi: Default::default(),
            probe: false,
            catalog_result: None,
            questions: HashMap::new(),
            question_order: Default::default(),
            visible_question: None,
            async_question_items: Default::default(),
            async_answers: HashMap::new(),
        }
    }

    pub(super) fn text(&mut self, key: &str, text: &str, snapshot: bool, events: &mut Vec<Event>) {
        let already = self.emitted.entry(key.to_owned()).or_default();
        let text = if snapshot {
            snapshot_remainder(already, text)
        } else {
            text
        };
        if text.is_empty() {
            return;
        }
        already.push_str(text);
        let text = text.to_owned();
        if self.text_key.as_deref() != Some(key) {
            events.push(Event::BeginAssistant);
            self.begun = true;
            self.text_key = Some(key.to_owned());
        }
        events.push(Event::Text(text));
    }

    pub(super) fn frame(&mut self, v: Value, stdin: &mut impl Write) -> Result<Vec<Event>, String> {
        if self.request.kind == Kind::Codex {
            // Codex requests its clock before thread/start may have replied.
            if v["method"] == "currentTime/read" && v.get("id").is_some() {
                write_json(
                    stdin,
                    json!({"id":v["id"],"result":{"currentTimeAt":crate::sys::time::now_ms()/1000}}),
                )?;
                return Ok(Vec::new());
            }
            let turn = v["params"]["turnId"]
                .as_str()
                .or_else(|| v["params"]["turn"]["id"].as_str());
            if turn.is_some() && turn == self.previous_turn.as_deref() {
                return Ok(Vec::new());
            }
            if let Some(id) = v["id"].as_str() {
                if let Some((wire, kind, request, question)) = self.async_answers.remove(id) {
                    if let Some(error) = v.get("error").filter(|e| !e.is_null()) {
                        let message = codex_error(error, "Could not send your answer to Codex");
                        self.questions
                            .insert(question.id.clone(), (wire, kind, request, question.clone()));
                        self.visible_question = Some(question.id.clone());
                        return Ok(vec![Event::Status(message), Event::Question(question)]);
                    }
                    return Ok(vec![Event::QuestionResolved(question.id)]);
                }
                if id.starts_with("native-answer-") || id.starts_with("native-interrupt-") {
                    return Ok(Vec::new());
                }
            }
            let method = v["method"].as_str().unwrap_or("");
            if self.previous_turn.is_some()
                && self.active_turn.is_none()
                && method != "turn/started"
                && (method.starts_with("item/") || method.starts_with("turn/") || method == "error")
            {
                return Ok(Vec::new());
            }
        }
        if self.request.kind == Kind::Claude {
            if v["type"] == "control_cancel_request" {
                let id = v["request_id"].to_string();
                let mut events = Vec::new();
                if self.approvals.remove(&id).is_some() {
                    events.push(Event::ApprovalResolved(id.clone()));
                }
                if self.questions.remove(&id).is_some() {
                    if self.visible_question.as_deref() == Some(id.as_str()) {
                        self.visible_question = None;
                    }
                    events.push(Event::QuestionResolved(id));
                    events.extend(self.show_next_question());
                }
                return Ok(events);
            }
            if v["type"] == "control_response"
                || (v["type"] == "system"
                    && matches!(v["subtype"].as_str(), Some("init" | "initialized")))
            {
                self.claude_prompt(stdin)?;
            }
            let starts_message = v["parent_tool_use_id"].is_null()
                && (v["type"] == "assistant"
                    || (v["type"] == "stream_event" && v["event"]["type"] == "message_start")
                    || (v["type"] == "system" && v["subtype"] == "init"));
            if starts_message && self.claude_result_seen {
                self.claude_result_seen = false;
                self.claude_resume_at = None;
                self.text_key = None;
            }
        }
        let mut activity_events = Vec::new();
        if self.activity(&v, &mut activity_events) {
            return Ok(activity_events);
        }
        if let Some(events) = self.steer_response(&v) {
            return Ok(events);
        }
        let mut started_turn = None;
        if self.request.kind == Kind::Codex {
            if v["id"] == 2 {
                if let Some(id) = v["result"]["thread"]["id"].as_str() {
                    self.request.provider_id = Some(id.into());
                }
            }
            if v["method"] == "turn/started" || v["id"] == 3 {
                let turn = if v["id"] == 3 {
                    &v["result"]["turn"]
                } else {
                    &v["params"]["turn"]
                };
                if let Some(id) = turn["id"].as_str() {
                    if self.active_turn.as_deref() != Some(id) {
                        started_turn = Some(id.to_owned());
                    }
                    self.active_turn = Some(id.into());
                }
            }
        }
        if (self.request.kind == Kind::Claude && v["type"] == "system" && v["subtype"] == "init")
            || (matches!(self.request.kind, Kind::Pi | Kind::Omp) && v["type"] == "agent_start")
        {
            self.follow_up_ready = true;
        }
        let mut events = activity_events;
        if let Some(id) = started_turn {
            events.push(Event::TurnId(id));
        }
        if self.request.kind == Kind::Claude && v["type"] == "system" {
            if v["status"] == "compacting" {
                self.compaction(None, ToolState::Running, &mut events);
            } else if v["subtype"]
                .as_str()
                .is_some_and(|subtype| subtype.starts_with("compact"))
            {
                self.compaction(None, ToolState::Completed, &mut events);
            }
        }
        if let Some(commands) = self.command_updates(&v) {
            events.push(Event::Commands(commands));
        }
        if self.request.kind == Kind::Codex {
            self.codex_async_questions(&v, &mut events, stdin)?;
        }
        if self.compact
            && self.request.kind == Kind::Claude
            && v["type"] == "system"
            && v["subtype"]
                .as_str()
                .is_some_and(|s| s.starts_with("compact"))
        {
            self.compact_confirmed = true;
        }
        match self.request.kind {
            Kind::Codex => {
                if let Some(error) = v.get("error").filter(|e| !e.is_null()) {
                    let message = codex_error(error, "RPC request failed");
                    if v["id"] == 2
                        && self.request.provider_id.is_some()
                        && lifecycle::recoverable_codex_resume(&message)
                    {
                        if self.request.settings.mono.is_some() {
                            return Err("The saved Mono Codex context could not be resumed. Retry to keep its saved context.".into());
                        }
                        self.request.provider_id = None;
                        return self.frame(json!({"id":1,"result":{}}), stdin);
                    }
                    return Err(message);
                }
                match v["id"].as_u64() {
                    Some(1) if v.get("result").is_some() => {
                        if !self.initialized {
                            write_json(stdin, json!({"method":"initialized"}))?;
                            self.initialized = true;
                        }
                        let (policy, sandbox, sandbox_policy, reviewer) = self.codex_access();
                        let mut params = json!({"cwd":self.request.cwd,"approvalPolicy":policy,"sandbox":sandbox,"sandboxPolicy":sandbox_policy,"approvalsReviewer":reviewer,
                            "developerInstructions":format!("You are the coding agent in Monocode. Use the available tools to inspect and work in the supplied cwd. Do not ask the user to paste files or git output you can inspect yourself. Respect the selected access policy and request approval when required. {}", if self.request.settings.plan { "This is a planning turn: inspect and propose a plan without changing files." } else { "" })});
                        if let Some(instructions) = crate::provider::visual_bridge::instructions(
                            &self.request.settings.environment,
                            false,
                        ) {
                            params["developerInstructions"] = format!(
                                "{}\n\n{instructions}",
                                params["developerInstructions"].as_str().unwrap_or_default()
                            )
                            .into();
                        }
                        if let Some(mono) = self
                            .request
                            .settings
                            .mono
                            .as_ref()
                            .filter(|_| !self.request.settings.text_only)
                        {
                            params["developerInstructions"] = format!(
                                "{}\n\n{}",
                                params["developerInstructions"].as_str().unwrap_or_default(),
                                crate::mono::instructions(&mono.mono, &mono.executable)
                            )
                            .into();
                        }
                        if self.request.settings.text_only {
                            params["ephemeral"] = true.into();
                            params["developerInstructions"] = "Answer the supplied text request directly. Do not call tools or inspect the workspace.".into();
                        }
                        if let Some(model) = &self.request.model {
                            params["model"] = model.clone().into();
                        }
                        if let Some(tier) = self
                            .request
                            .settings
                            .service_tier
                            .as_deref()
                            .filter(|tier| *tier != "default")
                        {
                            params["serviceTier"] = tier.into();
                        }
                        let method = if let Some(id) = &self.request.provider_id {
                            params["threadId"] = id.clone().into();
                            // The app already owns its transcript. Resume the
                            // provider context without serializing its history.
                            params["excludeTurns"] = true.into();
                            "thread/resume"
                        } else {
                            "thread/start"
                        };
                        write_json(stdin, json!({"id":2,"method":method,"params":params}))?;
                    }
                    Some(2) if v.get("result").is_some() => {
                        let id = v["result"]["thread"]["id"]
                            .as_str()
                            .ok_or("Codex returned no thread id")?;
                        events.push(Event::SessionId(id.into()));
                        if let Some(model) = v["result"]["model"].as_str() {
                            events.push(Event::Model(model.into()));
                        }
                        if self.compact {
                            write_json(
                                stdin,
                                json!({"id":3,"method":"thread/compact/start","params":{"threadId":id}}),
                            )?;
                            return Ok(events);
                        }
                        let (policy, _, sandbox, reviewer) = self.codex_access();
                        let mut params = json!({"threadId":id,"input":self.request.codex_input(),
                            "approvalPolicy":policy,"sandboxPolicy":sandbox,"approvalsReviewer":reviewer});
                        if let Some(effort) = &self.request.settings.effort {
                            params["effort"] = effort.clone().into();
                        }
                        if self.request.settings.fast {
                            params["serviceTier"] = "fast".into();
                        }
                        if let Some(tier) = self
                            .request
                            .settings
                            .service_tier
                            .as_deref()
                            .filter(|tier| *tier != "default")
                        {
                            params["serviceTier"] = tier.into();
                        }
                        if self.request.settings.plan {
                            params["sandboxPolicy"] = json!({"type":"readOnly"});
                            if self.request.settings.operator
                                || self.request.settings.operator_enabled
                                || self.request.settings.orchestrator
                            {
                                params["sandboxPolicy"]["networkAccess"] = true.into();
                            }
                            params["approvalPolicy"] = "never".into();
                        }
                        if let Some(model) = self
                            .request
                            .model
                            .as_deref()
                            .or_else(|| v["result"]["model"].as_str())
                        {
                            params["collaborationMode"] = json!({"mode": if self.request.settings.plan { "plan" } else { "default" }, "settings":{"model":model,"reasoning_effort":self.request.settings.effort,"developer_instructions":null}});
                        }
                        if let Some(model) = &self.request.model {
                            params["model"] = model.clone().into();
                        }
                        write_json(stdin, json!({"id":3,"method":"turn/start","params":params}))?;
                    }
                    _ => {}
                }
                let p = &v["params"];
                match v["method"].as_str().unwrap_or("") {
                    "thread/tokenUsage/updated" => {
                        let usage = &p["tokenUsage"];
                        events.push(Event::Context {
                            used: usage["last"]["totalTokens"].as_u64(),
                            window: usage["modelContextWindow"].as_u64().filter(|w| *w > 0),
                        });
                        let total = usage["total"]["totalTokens"].as_u64().unwrap_or(0);
                        if usage["last"].is_object() && (total == 0 || total != self.usage_total) {
                            self.usage_total = total;
                            let last = &usage["last"];
                            self.metrics.input_tokens += last["inputTokens"].as_u64().unwrap_or(0);
                            self.metrics.output_tokens +=
                                last["outputTokens"].as_u64().unwrap_or(0);
                            self.metrics.cache_read_tokens +=
                                last["cachedInputTokens"].as_u64().unwrap_or(0);
                            self.metrics.cache_write_tokens +=
                                last["cacheWriteInputTokens"].as_u64().unwrap_or(0);
                            if (last.get("cachedInputTokens").is_some()
                                || last.get("cacheWriteInputTokens").is_some())
                                && self.metrics.input_tokens > 0
                            {
                                self.metrics.cache_hit_percent = Some(
                                    self.metrics.cache_read_tokens as f64
                                        / (self.metrics.input_tokens
                                            + self.metrics.cache_write_tokens)
                                            as f64
                                        * 100.,
                                );
                            }
                            if self.metrics.reported() {
                                events.push(Event::Metrics(self.metrics));
                            }
                        }
                    }
                    "item/agentMessage/delta" => {
                        self.text(
                            p["itemId"].as_str().unwrap_or("assistant"),
                            p["delta"].as_str().unwrap_or(""),
                            false,
                            &mut events,
                        );
                    }
                    "item/completed" if p["item"]["type"] == "agentMessage" => {
                        self.text(
                            p["item"]["id"].as_str().unwrap_or("assistant"),
                            p["item"]["text"].as_str().unwrap_or(""),
                            true,
                            &mut events,
                        );
                    }
                    "item/started" if p["item"]["type"] == "agentMessage" => {}
                    "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                        events.push(Event::Reasoning(p["delta"].as_str().unwrap_or("").into()));
                        events.push(Event::Status("Thinking…".into()));
                    }
                    "item/started" | "item/completed" => {
                        self.codex_tool(&p["item"], v["method"] == "item/completed", &mut events)
                    }
                    "thread/compacted" if self.active_compaction.is_some() => {
                        self.compaction(None, ToolState::Completed, &mut events);
                    }
                    "item/commandExecution/outputDelta" => {
                        let id = p["itemId"].as_str().unwrap_or("");
                        let tool = self
                            .tools
                            .entry(id.into())
                            .or_insert_with(|| ToolCall::new(id, "commandExecution"));
                        tool.append_output(p["delta"].as_str().unwrap_or(""));
                        events.push(Event::Tool(tool.clone()));
                    }
                    "turn/completed" | "turn/aborted" => {
                        let status = p["turn"]["status"].as_str().unwrap_or("completed");
                        if status == "failed" {
                            return Err(codex_error(&p["turn"]["error"], "Codex turn failed"));
                        }
                        let interrupted =
                            matches!(status, "interrupted" | "cancelled" | "canceled")
                                || v["method"] == "turn/aborted";
                        for item in p["turn"]["items"].as_array().into_iter().flatten() {
                            if interrupted && item["type"] == "contextCompaction" {
                                // An aborted turn can include its unfinished
                                // compaction item in the final snapshot.
                                continue;
                            }
                            self.codex_tool(item, true, &mut events);
                        }
                        if interrupted {
                            for tool in self
                                .tools
                                .values_mut()
                                .filter(|tool| tool.state == ToolState::Running)
                            {
                                tool.state = ToolState::Cancelled;
                                events.push(Event::Tool(tool.clone()));
                            }
                        }
                        events.push(Event::Done);
                        self.done = true;
                    }
                    "error" if p["willRetry"] != true => {
                        return Err(codex_error(&p["error"], "Codex stream error"));
                    }
                    "error" => {
                        events.push(Event::Status(codex_error(
                            &p["error"],
                            "Codex is reconnecting…",
                        )));
                    }
                    "mcpServer/elicitation/request" => {
                        self.codex_elicitation(v["id"].clone(), p.clone(), &mut events, stdin)?
                    }
                    "item/tool/requestUserInput" => {
                        self.question(v["id"].clone(), "codex", p.clone(), &mut events, stdin)?
                    }
                    method
                        if v.get("id").is_some()
                            && (method.ends_with("/requestApproval")
                                || matches!(
                                    method,
                                    "execCommandApproval" | "applyPatchApproval"
                                )) =>
                    {
                        let detail = p["command"]
                            .as_str()
                            .or(p["reason"].as_str())
                            .map(str::to_owned)
                            .unwrap_or_default();
                        self.approval(
                            v["id"].clone(),
                            method,
                            p.clone(),
                            detail,
                            stdin,
                            &mut events,
                        )?;
                    }
                    method if v.get("id").is_some() && !method.is_empty() => {
                        // No approval UX in this gate: resolve every server request promptly.
                        write_json(
                            stdin,
                            json!({"id":v["id"],"error":{"code":-32601,"message":format!("Unsupported method: {method}")}}),
                        )?;
                        events.push(Event::Status(format!(
                            "Unsupported Codex request: {method}"
                        )));
                    }
                    _ => {}
                }
            }
            Kind::Claude => {
                // A refused rewind reports the id of a fork that was never written.
                let refused_rewind = v["is_error"] == true
                    && self
                        .request
                        .provider_id
                        .as_deref()
                        .is_some_and(|binding| rewind::claude_resume(binding).1.is_some());
                if let Some(id) = v["session_id"].as_str().filter(|_| !refused_rewind) {
                    events.push(Event::SessionId(id.into()));
                }
                if let Some(model) = v["model"].as_str().filter(|_| v["type"] == "system") {
                    events.push(Event::Model(model.into()));
                }
                match v["type"].as_str().unwrap_or("") {
                    "rate_limit_event" if v["parent_tool_use_id"].is_null() => {
                        self.claude_usage_limit =
                            crate::session::agent::Limit::from_claude_info(&v["rate_limit_info"]);
                    }
                    "control_response" if v["response"]["request_id"] == "native-init" => {
                        self.claude_prompt(stdin)?;
                    }
                    "stream_event" => {
                        let e = &v["event"];
                        if e["type"] == "message_start" {
                            self.claude_message += 1;
                            self.claude_tools.clear();
                        }
                        if e["type"] == "content_block_start"
                            && e["content_block"]["type"] == "tool_use"
                        {
                            let block = &e["content_block"];
                            let id = block["id"].as_str().unwrap_or("");
                            let tool = ToolCall::new(id, block["name"].as_str().unwrap_or("Tool"));
                            self.claude_tools.insert(
                                e["index"].as_u64().unwrap_or(0),
                                (id.into(), String::new()),
                            );
                            self.tools.insert(id.into(), tool.clone());
                            events.push(Event::Tool(tool));
                        }
                        if e["type"] == "content_block_delta"
                            && e["delta"]["type"] == "input_json_delta"
                        {
                            // Like the desktop adapter, keep the last input that
                            // parses; a half-streamed object is not shown.
                            if let Some((id, partial)) =
                                self.claude_tools.get_mut(&e["index"].as_u64().unwrap_or(0))
                            {
                                partial.push_str(e["delta"]["partial_json"].as_str().unwrap_or(""));
                                let parsed = serde_json::from_str::<Value>(partial)
                                    .is_ok_and(|value| value.is_object());
                                if let Some(tool) =
                                    self.tools.get_mut(id.as_str()).filter(|_| parsed)
                                {
                                    tool.input = partial.clone();
                                    events.push(Event::Tool(tool.clone()));
                                }
                            }
                        }
                        if e["type"] == "content_block_delta"
                            && e["delta"]["type"] == "thinking_delta"
                        {
                            events.push(Event::Reasoning(
                                e["delta"]["thinking"].as_str().unwrap_or("").into(),
                            ));
                        }
                        if e["type"] == "content_block_delta" && e["delta"]["type"] == "text_delta"
                        {
                            self.text(
                                &self.claude_message.to_string(),
                                e["delta"]["text"].as_str().unwrap_or(""),
                                false,
                                &mut events,
                            );
                        }
                    }
                    "assistant" => {
                        if let Some(error) = v["error"].as_str() {
                            return Err(self.claude_error(error, &v));
                        }
                        if v["parent_tool_use_id"].is_null() {
                            if let Some(used) =
                                crate::session::context::claude_used(&v["message"]["usage"])
                            {
                                events.push(Event::Context {
                                    used: Some(used),
                                    window: None,
                                });
                            }
                        }
                        for block in v["message"]["content"].as_array().into_iter().flatten() {
                            if block["type"] == "tool_use" {
                                let id = block["id"].as_str().unwrap_or("");
                                let tool = self.tools.entry(id.into()).or_insert_with(|| {
                                    ToolCall::new(id, block["name"].as_str().unwrap_or("Tool"))
                                });
                                tool.input = serde_json::to_string_pretty(&block["input"])
                                    .unwrap_or_default();
                                tool.refresh_agent_input();
                                events.push(Event::Tool(tool.clone()));
                            }
                        }
                        let text = v["message"]["content"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|b| b["type"] == "text")
                            .filter_map(|b| b["text"].as_str())
                            .collect::<String>();
                        self.text(&self.claude_message.to_string(), &text, true, &mut events);
                    }
                    "user" => {
                        for block in v["message"]["content"].as_array().into_iter().flatten() {
                            if block["type"] == "tool_result" {
                                // A background Agent's launch receipt is not its final report.
                                if block["is_error"] != true
                                    && block["tool_use_id"].as_str().is_some_and(|id| {
                                        self.activities.background_calls.contains(id)
                                    })
                                {
                                    continue;
                                }
                                if let Some(tool) = block["tool_use_id"]
                                    .as_str()
                                    .and_then(|id| self.tools.get_mut(id))
                                {
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
                                    tool.state = if block["is_error"] == true {
                                        ToolState::Failed
                                    } else {
                                        ToolState::Completed
                                    };
                                    events.push(Event::Tool(tool.clone()));
                                }
                            }
                        }
                    }
                    "result" if claude_turn_result(&v) => {
                        if self.compact && !self.compact_confirmed && v["is_error"] != true {
                            return Err("Claude Code did not confirm context compaction".into());
                        }
                        let usage = &v["usage"];
                        let input = usage["input_tokens"].as_u64().unwrap_or(0);
                        let output = usage["output_tokens"].as_u64().unwrap_or(0);
                        let read = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
                        let write = usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);
                        let metrics = TurnMetrics {
                            input_tokens: input,
                            output_tokens: output,
                            cache_read_tokens: read,
                            cache_write_tokens: write,
                            cache_hit_percent: ((usage.get("cache_read_input_tokens").is_some()
                                || usage.get("cache_creation_input_tokens").is_some())
                                && input + read + write > 0)
                                .then(|| read as f64 / (input + read + write) as f64 * 100.),
                        };
                        if metrics.reported() {
                            events.push(Event::Metrics(metrics));
                        }
                        let last = v["usage"]["iterations"]
                            .as_array()
                            .and_then(|iterations| iterations.last())
                            .unwrap_or(&v["usage"]);
                        let used = crate::session::context::claude_used(last);
                        let window = v["modelUsage"]
                            .as_object()
                            .into_iter()
                            .flat_map(|models| models.values())
                            .filter_map(|model| model["contextWindow"].as_u64())
                            .filter(|w| *w > 0)
                            .max();
                        if used.is_some() || window.is_some() {
                            events.push(Event::Context { used, window });
                        }
                        if v["is_error"] == true {
                            let error = v["result"]
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| v["errors"].to_string());
                            return Err(self.claude_error(&error, &v));
                        }
                        // Some versions omit partial/assistant frames in simple turns.
                        if !self.begun {
                            self.text(
                                &self.claude_message.to_string(),
                                v["result"].as_str().unwrap_or(""),
                                true,
                                &mut events,
                            );
                        }
                        self.claude_result_seen = true;
                        if self.activities.claude_background.is_empty()
                            && self.activities.claude_pending.is_empty()
                            && self.claude_resume_at.is_none()
                        {
                            events.push(Event::Done);
                            self.done = true;
                        } else {
                            events.push(Event::Status("Waiting for background tasks…".into()));
                        }
                    }
                    // A limit reported for the replay must not carry over to the real turn.
                    "result" => self.claude_usage_limit = None,
                    "control_request" => {
                        let request = &v["request"];
                        if request["subtype"] == "can_use_tool" {
                            if request["tool_name"] == "AskUserQuestion" {
                                self.question(
                                    v["request_id"].clone(),
                                    "claude",
                                    request.clone(),
                                    &mut events,
                                    stdin,
                                )?;
                                return Ok(events);
                            }
                            let detail =
                                serde_json::to_string_pretty(&request["input"]).unwrap_or_default();
                            self.approval(
                                v["request_id"].clone(),
                                "claude",
                                request.clone(),
                                detail,
                                stdin,
                                &mut events,
                            )?;
                        } else {
                            write_json(
                                stdin,
                                json!({"type":"control_response","response":{"subtype":"error","request_id":v["request_id"],"error":"Unsupported host request"}}),
                            )?;
                        }
                    }
                    _ => {}
                }
            }
            kind if kind.is_pi() => events.extend(self.pi_frame(v, stdin)?),
            _ => events.extend(self.acp_frame(v, stdin)?),
        }
        Ok(events)
    }
}
