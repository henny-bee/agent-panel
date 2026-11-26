use super::*;
fn protocol(kind: Kind) -> Protocol {
    Protocol::new(Request {
        kind,
        binary: "unused".into(),
        cwd: "/tmp".into(),
        prompt: "Hello".into(),
        provider_id: None,
        model: None,
        settings: Default::default(),
    })
}
#[test]
fn codex_file_changes_retain_every_edited_path() {
    let mut protocol = protocol(Kind::Codex);
    let mut events = Vec::new();
    protocol.codex_tool(&json!({
        "type":"fileChange", "id":"edit", "status":"completed",
        "changes":[{"path":"/repo/a.rs", "diff":"first"}, {"path":"/repo/b.rs", "diff":"second"}]
    }), true, &mut events);
    let Event::Tool(tool) = &events[0] else {
        panic!("missing edit tool")
    };
    let input: Value = serde_json::from_str(&tool.input).unwrap();
    assert_eq!(tool.state, ToolState::Completed);
    assert_eq!(input["path"], "/repo/a.rs");
    assert_eq!(input["paths"], json!(["/repo/a.rs", "/repo/b.rs"]));
}
#[test]
fn a_monos_rules_reach_codex_as_developer_instructions() {
    let mut p = protocol(Kind::Codex);
    p.request.settings.mono = Some(crate::session::agent::MonoContext {
        mono: crate::session::mono_model::Mono {
            projects: vec!["/repo".into()],
            ..Default::default()
        },
        root: "/monos".into(),
        executable: "/bin/monocode-gpui".into(),
    });
    let mut out = Vec::new();
    p.frame(json!({"id":1,"result":{}}), &mut out).unwrap();
    let started: Value = String::from_utf8_lossy(&out)
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|message| message["method"] == "thread/start")
        .unwrap();
    let instructions = started["params"]["developerInstructions"].as_str().unwrap();
    assert!(instructions.starts_with("You are the coding agent in Monocode."));
    assert!(instructions.contains("## Be quick"));
    assert!(instructions.contains("- /repo"));
}
#[test]
fn operator_thread_access_keeps_the_codex_app_bridge_reachable_on_later_turns() {
    for plan in [false, true] {
        let mut p = protocol(Kind::Codex);
        p.request.settings.operator_enabled = true;
        p.request.settings.plan = plan;
        assert!(!p.request.settings.operator);
        assert_eq!(p.codex_access().2["networkAccess"], true);
        let mut out = Vec::new();
        p.frame(json!({"id":1,"result":{}}), &mut out).unwrap();
        p.frame(json!({"id":2,"result":{"thread":{"id":"saved"}}}), &mut out)
            .unwrap();
        let started: Value = String::from_utf8_lossy(&out)
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .find(|message| message["method"] == "turn/start")
            .unwrap();
        assert_eq!(started["params"]["sandboxPolicy"]["networkAccess"], true);
        if plan {
            assert_eq!(started["params"]["sandboxPolicy"]["type"], "readOnly");
        }
    }
}
#[test]
fn context_reports_use_the_latest_request_and_include_cached_tokens() {
    let mut codex = protocol(Kind::Codex);
    let events = codex.frame(json!({"method":"thread/tokenUsage/updated","params":{"tokenUsage":{"last":{"totalTokens":4200},"total":{"totalTokens":99000},"modelContextWindow":272000}}}),&mut Vec::new()).unwrap();
    assert_eq!(
        events,
        vec![Event::Context {
            used: Some(4200),
            window: Some(272000)
        }]
    );
    let mut claude = protocol(Kind::Claude);
    let events = claude.frame(json!({"type":"assistant","message":{"content":[],"usage":{"input_tokens":100,"cache_read_input_tokens":9000,"cache_creation_input_tokens":500,"output_tokens":25}}}),&mut Vec::new()).unwrap();
    assert_eq!(
        events,
        vec![Event::Context {
            used: Some(9625),
            window: None
        }]
    );
    assert!(claude.frame(json!({"type":"assistant","parent_tool_use_id":"subagent","message":{"content":[],"usage":{"input_tokens":999999}}}),&mut Vec::new()).unwrap().is_empty());
    let events = claude.frame(json!({"type":"result","usage":{"input_tokens":90000,"iterations":[{"input_tokens":1000},{"input_tokens":2000,"output_tokens":50}]},"modelUsage":{"model":{"contextWindow":200000}}}),&mut Vec::new()).unwrap();
    assert!(events.contains(&Event::Context {
        used: Some(2050),
        window: Some(200000)
    }));
}
#[test]
fn claude_replayed_task_notification_result_does_not_end_the_turn() {
    let mut claude = protocol(Kind::Claude);
    let replayed = json!({"type":"result","subtype":"success","is_error":false,"num_turns":0,"result":"","origin":{"kind":"task-notification"}});
    assert!(!claude_turn_result(&replayed));
    let events = claude.frame(replayed, &mut Vec::new()).unwrap();
    assert!(!events.contains(&Event::Done));
    assert!(!claude.done);
    // A task that finishes during the session wakes a real turn, which ends.
    let woken = json!({"type":"result","num_turns":1,"result":"Done.","origin":{"kind":"task-notification"}});
    assert!(claude
        .frame(woken, &mut Vec::new())
        .unwrap()
        .contains(&Event::Done));
}
#[test]
fn claude_replayed_task_notification_result_drops_its_usage_limit() {
    let mut claude = protocol(Kind::Claude);
    claude
        .frame(
            json!({"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1}}),
            &mut Vec::new(),
        )
        .unwrap();
    assert!(claude.claude_usage_limit.is_some());
    claude.frame(json!({"type":"result","num_turns":0,"result":"","origin":{"kind":"task-notification"}}), &mut Vec::new()).unwrap();
    assert!(claude.claude_usage_limit.is_none());
    let error = claude.frame(json!({"type":"result","is_error":true,"num_turns":1,"result":"Something else went wrong"}), &mut Vec::new()).unwrap_err();
    assert_eq!(error, "Something else went wrong");
}
#[test]
fn codex_handshake_stream_snapshot_and_resume() {
    let mut p = protocol(Kind::Codex);
    p.request.provider_id = Some("saved".into());
    let mut out = Vec::new();
    p.frame(json!({"id":1,"result":{}}), &mut out).unwrap();
    assert!(String::from_utf8_lossy(&out).contains("thread/resume"));
    let resumed: Value = String::from_utf8_lossy(&out)
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|message| message["method"] == "thread/resume")
        .unwrap();
    assert_eq!(resumed["params"]["threadId"], "saved");
    assert_eq!(resumed["params"]["excludeTurns"], true);
    p.frame(json!({"id":2,"result":{"thread":{"id":"saved"}}}), &mut out)
        .unwrap();
    let e = p
        .frame(
            json!({"method":"item/agentMessage/delta","params":{"itemId":"a","delta":"Hi\n"}}),
            &mut out,
        )
        .unwrap();
    assert_eq!(e, vec![Event::BeginAssistant, Event::Text("Hi\n".into())]);
    assert_eq!(p.frame(json!({"method":"item/completed","params":{"item":{"type":"agentMessage","id":"a","text":"Hi\n\n"}}}), &mut out).unwrap(),vec![Event::Text("\n".into())]);
    assert_eq!(
        p.frame(
            json!({"method":"turn/completed","params":{"turn":{"status":"completed"}}}),
            &mut out
        )
        .unwrap(),
        vec![Event::Done]
    );
}
#[test]
fn claude_stream_and_completed_snapshot_do_not_duplicate() {
    let mut p = protocol(Kind::Claude);
    let mut out = Vec::new();
    p.frame(
        json!({"type":"stream_event","event":{"type":"message_start"}}),
        &mut out,
    )
    .unwrap();
    p.frame(json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hello 🙂"}}}), &mut out).unwrap();
    assert!(p
        .frame(
            json!({"type":"assistant","message":{"content":[{"type":"text","text":"Hello 🙂"}]}}),
            &mut out
        )
        .unwrap()
        .is_empty());
    assert!(p
        .frame(
            json!({"type":"result","is_error":true,"result":"Login required"}),
            &mut out
        )
        .unwrap_err()
        .contains("Login"));
}
#[test]
fn unknown_server_requests_are_resolved_and_errors_surface() {
    let mut p = protocol(Kind::Codex);
    let mut out = Vec::new();
    p.frame(
        json!({"id":"req-1","method":"unsupported/request","params":{}}),
        &mut out,
    )
    .unwrap();
    assert!(String::from_utf8_lossy(&out).contains("req-1"));
    assert!(p
        .frame(
            json!({"id":2,"error":{"message":"Resume failed"}}),
            &mut out
        )
        .unwrap_err()
        .contains("Resume failed"));
}
#[test]
fn approvals_wait_for_a_decision_and_keep_the_requested_input() {
    let mut codex = protocol(Kind::Codex);
    let mut out = Vec::new();
    let events = codex.frame(json!({"id":"permission-1","method":"item/commandExecution/requestApproval","params":{"command":"cargo build","itemId":"c1"}}), &mut out).unwrap();
    let Event::Approval(approval) = &events[0] else {
        panic!("expected an approval")
    };
    assert!(
        out.is_empty(),
        "never answer an approval before a native decision"
    );
    codex
        .respond(
            Control {
                id: approval.id.clone(),
                decision: Decision::Deny,
            },
            &mut out,
        )
        .unwrap();
    let response: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(response["id"], "permission-1");
    assert_eq!(response["result"]["decision"], "decline");
    let mut claude = protocol(Kind::Claude);
    out.clear();
    let input = json!({"file_path":"/work/example.rs","content":"approved"});
    let events = claude.frame(json!({"type":"control_request","request_id":"permission-2","request":{"subtype":"can_use_tool","tool_name":"Write","input":input}}), &mut out).unwrap();
    let Event::Approval(approval) = &events[0] else {
        panic!("expected an approval")
    };
    assert!(out.is_empty());
    claude
        .respond(
            Control {
                id: approval.id.clone(),
                decision: Decision::Allow,
            },
            &mut out,
        )
        .unwrap();
    let response: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(response["response"]["request_id"], "permission-2");
    assert_eq!(response["response"]["response"]["behavior"], "allow");
    assert_eq!(response["response"]["response"]["updatedInput"], input);
}
#[test]
fn a_plan_reaches_the_user_and_accepting_it_leaves_plan_mode() {
    let mut claude = protocol(Kind::Claude);
    claude.request.settings.plan = true;
    let mut out = Vec::new();
    let plan = json!({"plan":"1. Read the code\n2. Fix it"});
    let request = |id: &str| json!({"type":"control_request","request_id":id,"request":{"subtype":"can_use_tool","tool_name":"ExitPlanMode","input":plan}});
    let events = claude.frame(request("plan-1"), &mut out).unwrap();
    let Event::Approval(approval) = &events[0] else {
        panic!("expected the plan")
    };
    // Plan mode no longer denies the plan before the user sees it.
    assert!(out.is_empty());
    assert_eq!(
        approval.plan.as_deref(),
        Some("1. Read the code\n2. Fix it")
    );
    claude
        .respond(
            Control {
                id: approval.id.clone(),
                decision: Decision::RevisePlan("Add tests".into()),
            },
            &mut out,
        )
        .unwrap();
    let response: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(response["response"]["response"]["behavior"], "deny");
    assert!(response["response"]["response"]["message"]
        .as_str()
        .unwrap()
        .ends_with("Add tests"));
    assert!(claude.request.settings.plan);
    out.clear();
    let events = claude.frame(request("plan-2"), &mut out).unwrap();
    let Event::Approval(approval) = &events[0] else {
        panic!("expected the revised plan")
    };
    claude
        .respond(
            Control {
                id: approval.id.clone(),
                decision: Decision::AcceptPlan(AccessMode::AutoAcceptEdits),
            },
            &mut out,
        )
        .unwrap();
    let response: Value = serde_json::from_slice(&out).unwrap();
    let answer = &response["response"]["response"];
    assert_eq!(answer["behavior"], "allow");
    assert_eq!(answer["updatedPermissions"][0]["mode"], "acceptEdits");
    // Later requests in this run are no longer denied as plan mode.
    assert!(!claude.request.settings.plan);
    assert_eq!(claude.request.settings.access, AccessMode::AutoAcceptEdits);
}
#[test]
fn selected_model_effort_and_access_reach_the_codex_turn() {
    let mut p = protocol(Kind::Codex);
    p.request.model = Some("selected-model".into());
    p.request.settings.effort = Some("high".into());
    p.request.settings.access = AccessMode::AutoAcceptEdits;
    let mut out = Vec::new();
    p.frame(json!({"id":1,"result":{}}), &mut out).unwrap();
    let rows: Vec<Value> = String::from_utf8_lossy(&out)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(rows[1]["params"]["sandbox"], "workspace-write");
    out.clear();
    p.frame(
        json!({"id":2,"result":{"thread":{"id":"thread"},"model":"selected-model"}}),
        &mut out,
    )
    .unwrap();
    let row: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(row["params"]["model"], "selected-model");
    assert_eq!(row["params"]["effort"], "high");
    assert_eq!(row["params"]["sandboxPolicy"]["type"], "workspaceWrite");
    assert_eq!(row["params"]["approvalsReviewer"], "user");
}
#[test]
fn codex_tool_deltas_settle_to_one_completed_snapshot() {
    let mut p = protocol(Kind::Codex);
    let mut out = Vec::new();
    let started = p.frame(json!({"method":"item/started","params":{"item":{"id":"cmd-1","type":"commandExecution","command":"cargo check","status":"inProgress"}}}), &mut out).unwrap();
    assert!(
        matches!(&started[0], Event::Tool(t) if t.state == ToolState::Running && t.summary() == ("Run".into(), "cargo check".into()))
    );
    p.frame(json!({"method":"item/commandExecution/outputDelta","params":{"itemId":"cmd-1","delta":"Checking 🙂"}}), &mut out).unwrap();
    let completed = p.frame(json!({"method":"item/completed","params":{"item":{"id":"cmd-1","type":"commandExecution","command":"cargo check","status":"completed","aggregatedOutput":"Checking 🙂\nFinished"}}}), &mut out).unwrap();
    assert!(
        matches!(&completed[0], Event::Tool(t) if t.id == "cmd-1" && t.state == ToolState::Completed && t.output == "Checking 🙂\nFinished")
    );
    assert_eq!(p.tools.len(), 1);
}
#[test]
fn codex_live_tool_preserves_argv_and_semantic_rollout_metadata() {
    let mut protocol = protocol(Kind::Codex);
    let mut out = Vec::new();
    let command = json!(["/bin/zsh", "-lc", "sed -n '8,20p' 'src/my file.rs'"]);
    let events = protocol.frame(json!({"method":"item/started","params":{"item":{"id":"read","type":"commandExecution","command":command,"parsed_cmd":[{"type":"read","path":"/repo/src/my file.rs"}],"cwd":"/repo","status":"inProgress"}}}), &mut out).unwrap();
    let Event::Tool(call) = &events[0] else {
        panic!()
    };
    assert_eq!(call.summary(), ("Read".into(), "src/my file.rs".into()));
    assert_eq!(
        call.presentation().command.as_deref(),
        Some("sed -n '8,20p' 'src/my file.rs'")
    );
    assert_eq!(
        serde_json::from_str::<Value>(&call.input).unwrap()["command"],
        command
    );
    let events = protocol.frame(json!({"method":"item/completed","params":{"item":{"id":"read","type":"commandExecution","status":"failed","aggregatedOutput":"permission denied","exitCode":1}}}), &mut out).unwrap();
    let Event::Tool(call) = &events[0] else {
        panic!()
    };
    assert_eq!(call.summary().0, "Read");
    assert_eq!(call.output, "permission denied");
    assert_eq!(call.state, ToolState::Failed);
    assert_eq!(
        serde_json::from_str::<Value>(&call.input).unwrap()["command"],
        command
    );
}
#[test]
fn claude_tool_input_and_result_keep_identity_and_failure() {
    let mut p = protocol(Kind::Claude);
    let mut out = Vec::new();
    p.frame(json!({"type":"stream_event","event":{"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"read-1","name":"Read"}}}), &mut out).unwrap();
    p.frame(json!({"type":"stream_event","event":{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"file_path\":\"src/main.rs\"}"}}}), &mut out).unwrap();
    let input = p.tools["read-1"].summary();
    assert_eq!(input, ("Read".into(), "src/main.rs".into()));
    p.frame(json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"read-1","name":"Read","input":{"file_path":"src/main.rs"}}]}}), &mut out).unwrap();
    let events = p.frame(json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"read-1","is_error":true,"content":[{"type":"text","text":"File missing"}]}]}}), &mut out).unwrap();
    assert!(
        matches!(&events[0], Event::Tool(t) if t.id == "read-1" && t.state == ToolState::Failed && t.output == "File missing")
    );
    assert_eq!(p.tools.len(), 1);
}
#[test]
fn claude_partial_tool_input_waits_until_it_parses() {
    let mut p = protocol(Kind::Claude);
    let mut out = Vec::new();
    p.frame(json!({"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"sh","name":"Bash"}}}), &mut out).unwrap();
    let events = p.frame(json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"command\": \"git status"}}}), &mut out).unwrap();
    assert!(events.is_empty());
    assert_eq!(p.tools["sh"].input, "");
    let events = p.frame(json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":" --short\"}"}}}), &mut out).unwrap();
    let Event::Tool(call) = &events[0] else {
        panic!()
    };
    assert_eq!(call.input, r#"{"command": "git status --short"}"#);
}

#[cfg(all(test, unix))]
mod text;

#[cfg(all(test, unix))]
mod process;

#[test]
fn a_found_cli_is_kept_only_while_it_exists() {
    let remember = |path: &Path| {
        let mut found = FOUND.lock().unwrap();
        found.retain(|(kind, _)| *kind != Kind::Fx);
        found.push((Kind::Fx, path.to_owned()));
    };
    let running = std::env::current_exe().unwrap();
    remember(&running);
    assert_eq!(resolve_auto(Kind::Fx), Some(running.clone()));
    let gone = running.with_file_name("no-such-cli");
    remember(&gone);
    assert_ne!(resolve_auto(Kind::Fx), Some(gone));
}
