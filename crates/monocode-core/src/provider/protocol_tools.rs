use super::*;

/// Claude Desktop's wording: "Allow Claude to run `npm test`?".
fn claude_title(request: &Value) -> String {
    let tool = request["tool_name"].as_str().unwrap_or("this tool");
    let input = &request["input"];
    let short = |text: &str| {
        let line = text.lines().next().unwrap_or_default().trim();
        match line.char_indices().nth(80) {
            Some((at, _)) => format!("{}…", &line[..at]),
            None => line.to_owned(),
        }
    };
    let file = || {
        input["file_path"]
            .as_str()
            .or(input["notebook_path"].as_str())
            .map(|path| path.rsplit('/').next().unwrap_or(path).to_owned())
    };
    match tool {
        "Bash" => match input["description"].as_str().or(input["command"].as_str()) {
            Some(what) => format!("Allow Claude to run {}?", short(what)),
            None => "Allow Claude to run a command?".into(),
        },
        "Edit" | "MultiEdit" | "NotebookEdit" => match file() {
            Some(file) => format!("Allow Claude to edit {file}?"),
            None => "Allow Claude to edit a file?".into(),
        },
        "Write" => match file() {
            Some(file) => format!("Allow Claude to write {file}?"),
            None => "Allow Claude to write a file?".into(),
        },
        "WebFetch" => match input["url"].as_str() {
            Some(url) => format!("Allow Claude to fetch {}?", short(url)),
            None => "Allow Claude to fetch a page?".into(),
        },
        "WebSearch" => match input["query"].as_str() {
            Some(query) => format!("Allow Claude to search the web for {}?", short(query)),
            None => "Allow Claude to search the web?".into(),
        },
        tool => format!(
            "Allow Claude to use {}?",
            tool.rsplit("__").next().unwrap_or(tool)
        ),
    }
}

impl Protocol {
    pub(super) fn approval(
        &mut self,
        wire: Value,
        kind: &str,
        request: Value,
        detail: String,
        stdin: &mut impl Write,
        events: &mut Vec<Event>,
    ) -> Result<(), String> {
        let id = wire.to_string();
        let title = if kind == "claude" {
            claude_title(&request)
        } else if kind.contains("fileChange") || kind == "applyPatchApproval" {
            "Allow file changes?".into()
        } else if kind.contains("permissions") {
            "Allow additional permissions?".into()
        } else {
            "Allow this command?".into()
        };
        let mut detail =
            if let Some(tool) = request["itemId"].as_str().and_then(|id| self.tools.get(id)) {
                let detail = if detail.is_empty() {
                    tool.summary().1
                } else {
                    detail
                };
                if tool.output.is_empty() {
                    detail
                } else {
                    format!("{detail}\n\n{}", tool.output)
                }
            } else {
                detail
            };
        if detail.is_empty() {
            detail = request
                .get("permissions")
                .and_then(|p| serde_json::to_string_pretty(p).ok())
                .unwrap_or_else(|| "The agent is requesting permission to continue.".into());
        }
        // Leaving plan mode is the user's call: Claude's plan goes to them.
        let plan = (kind == "claude" && request["tool_name"] == "ExitPlanMode").then(|| {
            request["input"]["plan"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        });
        self.approvals
            .insert(id.clone(), (wire, kind.into(), request));
        if let Some(plan) = plan {
            events.push(Event::Approval(Approval {
                id,
                title: "Claude proposed a plan".into(),
                detail: String::new(),
                can_remember: false,
                plan: Some(plan),
            }));
            return Ok(());
        }
        if self.request.settings.plan || self.request.settings.access == AccessMode::ReadOnly {
            return self.respond(
                Control {
                    id,
                    decision: Decision::Deny,
                },
                stdin,
            );
        }
        let file_change = kind.contains("fileChange") || kind == "applyPatchApproval";
        let permissions = kind.contains("permissions");
        if self.request.settings.access == AccessMode::FullAccess
            || (self.request.kind == Kind::Codex
                && self.request.settings.access == AccessMode::AutoAcceptEdits
                && file_change)
            || (self.request.kind == Kind::Codex
                && permissions
                && matches!(
                    self.request.settings.access,
                    AccessMode::Auto | AccessMode::AutoAcceptEdits
                ))
        {
            return self.respond(
                Control {
                    id,
                    decision: Decision::Allow,
                },
                stdin,
            );
        }
        events.push(Event::Approval(Approval {
            id,
            title,
            detail,
            can_remember: self.request.kind != Kind::Codex && !file_change,
            plan: None,
        }));
        Ok(())
    }

    pub(super) fn respond(
        &mut self,
        control: Control,
        stdin: &mut impl Write,
    ) -> Result<(), String> {
        if matches!(control.decision, Decision::KeepOpen) {
            if let Some((_, _, _, question)) = self.questions.get_mut(&control.id) {
                question.auto_resolve_at = None;
            }
            return Ok(());
        }
        if let Some((wire, kind, request, mut question)) = self.questions.remove(&control.id) {
            if self.visible_question.as_deref() == Some(control.id.as_str()) {
                self.visible_question = None;
            }
            if kind == "codex-async" {
                if let Decision::Answer(answers) = &control.decision {
                    let text = question
                        .items
                        .iter()
                        .zip(answers)
                        .filter(|(_, a)| !a.is_empty())
                        .map(|(q, a)| format!("{}\n{}", q.prompt, a.join("\n")))
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    if !text.is_empty() && request["turnId"].as_str() == self.active_turn.as_deref()
                    {
                        question.auto_resolve_at = None;
                        let id = format!(
                            "native-answer-{}-{}",
                            question.id,
                            crate::sys::time::now_ms()
                        );
                        write_json(
                            stdin,
                            json!({"id":id,"method":"turn/steer","params":{"threadId":request["threadId"],"expectedTurnId":request["turnId"],"input":[{"type":"text","text":text}]}}),
                        )?;
                        self.async_answers
                            .insert(id, (wire, kind, request, question));
                    }
                }
                return Ok(());
            }
            return questions::respond(stdin, wire, &kind, &request, &question, control.decision);
        }
        let Some((wire, kind, request)) = self.approvals.remove(&control.id) else {
            return Ok(());
        };
        let allow = matches!(
            control.decision,
            Decision::Allow | Decision::Session | Decision::AcceptPlan(_)
        );
        let session = matches!(control.decision, Decision::Session);
        if kind == "acp" {
            return acp::respond(stdin, wire, &request, control.decision);
        }
        if kind == "codex-mcp" {
            return write_json(
                stdin,
                json!({"id":wire,"result":{"action":if allow {"accept"} else {"decline"},"content":if allow {request["nativeContent"].clone()} else {Value::Null},"_meta":null}}),
            );
        }
        if kind == "pi" {
            return write_json(
                stdin,
                json!({"type":"extension_ui_response","id":wire,"confirmed":allow,"cancelled":!allow}),
            );
        }
        if kind == "claude" {
            let mut response = match &control.decision {
                Decision::AcceptPlan(mode) => {
                    // The session leaves plan mode for the mode the user picked.
                    self.request.settings.plan = false;
                    self.request.settings.access = *mode;
                    json!({"behavior":"allow","updatedInput":request["input"],
                        "updatedPermissions":[{"type":"setMode","mode":mode.claude(),"destination":"session"}]})
                }
                Decision::RevisePlan(feedback) if feedback.trim().is_empty() => {
                    json!({"behavior":"deny",
                    "message":"The user rejected this plan. Stay in plan mode and wait for their next message."})
                }
                Decision::RevisePlan(feedback) => json!({"behavior":"deny",
                    "message":format!("The user wants the plan revised. Stay in plan mode, revise the plan with this feedback, then propose it again:\n\n{feedback}")}),
                _ if allow => json!({"behavior":"allow","updatedInput":request["input"]}),
                _ => json!({"behavior":"deny","message":"Denied by the user"}),
            };
            if session && allow {
                if let Some(suggestions) = request.get("permission_suggestions") {
                    response["updatedPermissions"] = suggestions.clone();
                }
            }
            write_json(
                stdin,
                json!({"type":"control_response","response":{"subtype":"success","request_id":wire,"response":response}}),
            )
        } else {
            let result = if kind.contains("permissions") {
                json!({"permissions": if allow { request["permissions"].clone() } else { json!({}) }, "scope":if self.request.settings.access == AccessMode::FullAccess { "session" } else { "turn" }})
            } else if kind == "execCommandApproval" || kind == "applyPatchApproval" {
                json!({"decision":if !allow { "denied" } else if session { "approved_for_session" } else { "approved" }})
            } else {
                json!({"decision":if !allow { "decline" } else { "accept" }})
            };
            write_json(stdin, json!({"id":wire,"result":result}))
        }
    }

    pub(super) fn codex_tool(&mut self, item: &Value, completed: bool, events: &mut Vec<Event>) {
        let name = item["type"].as_str().unwrap_or("");
        if name == "contextCompaction" {
            self.compaction(
                item["id"].as_str(),
                if completed {
                    ToolState::Completed
                } else {
                    ToolState::Running
                },
                events,
            );
            return;
        }
        if !matches!(
            name,
            "commandExecution" | "fileChange" | "mcpToolCall" | "webSearch" | "dynamicToolCall"
        ) {
            return;
        }
        let id = item["id"].as_str().unwrap_or("");
        let tool = self
            .tools
            .entry(id.into())
            .or_insert_with(|| ToolCall::new(id, name));
        tool.name = item["tool"].as_str().unwrap_or(name).to_owned();
        if name == "mcpToolCall" {
            tool.name = format!("{}:{}", item["server"].as_str().unwrap_or("mcp"), tool.name);
        }
        let previous_input = tool.input.clone();
        tool.input = item["command"]
            .as_str()
            .or_else(|| item["query"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                serde_json::to_string_pretty(
                    item.get("arguments")
                        .or_else(|| item.get("changes"))
                        .unwrap_or(&Value::Null),
                )
                .unwrap_or_default()
            });
        if name == "commandExecution" {
            // Keep argv and parsed provider metadata intact. Presentation unwraps
            // recognized launchers without changing the recorded execution.
            let previous = serde_json::from_str::<Value>(&previous_input).ok();
            let mut input = previous
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({}));
            for field in [
                "command",
                "commandActions",
                "parsed_cmd",
                "parsedCmd",
                "cwd",
                "exitCode",
                "durationMs",
            ] {
                if let Some(value) = item.get(field) {
                    input[field] = value.clone();
                }
            }
            tool.input = input.to_string();
        }
        if let Some(changes) = item["changes"].as_array().filter(|_| name == "fileChange") {
            tool.input = json!({
                "path": changes.first().and_then(|c| c["path"].as_str()).unwrap_or("Files"),
                "paths": changes.iter().filter_map(|c| c["path"].as_str()).collect::<Vec<_>>(),
            })
            .to_string();
            tool.output = changes
                .iter()
                .map(|c| {
                    format!(
                        "{}\n{}",
                        c["path"].as_str().unwrap_or(""),
                        c["diff"].as_str().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
        if let Some(output) = item["aggregatedOutput"]
            .as_str()
            .or_else(|| item["output"].as_str())
        {
            tool.output = output.to_owned();
        } else if let Some(result) = item.get("result") {
            tool.output = serde_json::to_string_pretty(result).unwrap_or_default();
        }
        tool.bound_output();
        tool.state = match item["status"].as_str().unwrap_or(if completed {
            "completed"
        } else {
            "inProgress"
        }) {
            "completed" => ToolState::Completed,
            "failed" | "declined" => ToolState::Failed,
            "cancelled" | "canceled" | "interrupted" | "aborted" => ToolState::Cancelled,
            _ => ToolState::Running,
        };
        if item["exitCode"].as_i64().is_some_and(|code| code != 0) {
            tool.state = ToolState::Failed;
        }
        events.push(Event::Tool(tool.clone()));
    }
}

#[cfg(test)]
mod title_tests {
    use super::*;
    #[test]
    fn claude_requests_read_as_what_claude_wants_to_do() {
        let title =
            |tool: &str, input: Value| claude_title(&json!({"tool_name":tool,"input":input}));
        assert_eq!(
            title("Bash", json!({"command":"npm test"})),
            "Allow Claude to run npm test?"
        );
        assert_eq!(
            title(
                "Bash",
                json!({"command":"rm -rf x","description":"Clean the build"})
            ),
            "Allow Claude to run Clean the build?"
        );
        assert_eq!(
            title("Edit", json!({"file_path":"/p/src/app.rs"})),
            "Allow Claude to edit app.rs?"
        );
        assert_eq!(
            title("mcp__github__create_issue", json!({})),
            "Allow Claude to use create_issue?"
        );
    }
}
