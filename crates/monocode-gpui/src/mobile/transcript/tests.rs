use super::*;

#[test]
fn waiting_approvals_and_questions_end_the_latest_turn() {
    let rows = [row("user", "fix it"), row("assistant", "on it")];
    let turns = HashMap::new();
    let pending = [
        Pending {
            id: "a1".into(),
            question: false,
            label: "Run npm test".into(),
        },
        Pending {
            id: "q1".into(),
            question: true,
            label: "Which branch?".into(),
        },
    ];
    let mut source = source(&rows, &turns, true);
    source.pending = &pending;
    let built = all(&source);
    let items = built.last().unwrap()["items"].as_array().unwrap();
    let notices: Vec<_> = items.iter().filter(|i| i["type"] == "notice").collect();
    assert_eq!(notices[0]["id"], "input:a1");
    assert_eq!(
        notices[0]["text"],
        "Waiting for approval on desktop: Run npm test"
    );
    assert_eq!(
        notices[1]["text"],
        "Question waiting on desktop: Which branch?"
    );
}
fn row(role: &str, text: &str) -> Row {
    Row {
        role: role.into(),
        text: text.into(),
        first: true,
        sent_at: None,
    }
}
fn sent(role: &str, text: &str, at: u64) -> Row {
    Row {
        sent_at: Some(at),
        ..row(role, text)
    }
}
fn tool(name: &str, input: Value, state: ToolState) -> Row {
    let mut call = ToolCall::new("call", name);
    call.input = input.to_string();
    call.state = state;
    row("tool", &serde_json::to_string(&call).unwrap())
}
fn source<'a>(rows: &'a [Row], turns: &'a HashMap<usize, TurnInfo>, busy: bool) -> Source<'a> {
    Source {
        rows,
        turns,
        cwd: "/repo",
        harness: "claude",
        model_name: Some("Opus".into()),
        busy,
        waiting: None,
        mono: None,
        pending: &[],
        launches: Box::leak(Box::default()),
    }
}
fn all(source: &Source) -> Vec<Value> {
    build(source, 0..usize::MAX)
}
#[test]
fn native_visuals_keep_a_readable_phone_fallback_outside_folded_work() {
    let document = monocode_core::session::visual::VisualDocument {
        id: "phone-example".into(),
        title: "Capacity explorer".into(),
        source: "return () => visual.text('Example');".into(),
    };
    let rows = vec![
        row("user", "Show capacity"),
        tool("Read", json!({"path":"data.json"}), ToolState::Completed),
        row("visual", &serde_json::to_string(&document).unwrap()),
        row("assistant", "Ready"),
    ];
    let info = HashMap::new();
    let turns = all(&source(&rows, &info, false));
    // Drawn inline from its rendered tree; the source stays on the desktop.
    let visual = turns[0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "visual")
        .unwrap();
    assert_eq!(visual["title"], "Capacity explorer");
    assert_eq!(visual["tree"]["kind"], "text");
    assert!(!turns[0].to_string().contains("visual.text"));
}
fn types(turn: &Value) -> Vec<&str> {
    turn["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["type"].as_str().unwrap())
        .collect()
}

#[test]
fn a_finished_reply_stays_while_the_next_message_is_on_its_way() {
    let rows = [sent("user", "Hi", 1_000), row("assistant", "Hello!")];
    let mut infos = HashMap::new();
    infos.insert(
        1,
        TurnInfo {
            started_at: Some(1_000),
            completed_at: Some(2_000),
            ..TurnInfo::default()
        },
    );
    let mut mono = source(&rows, &infos, true);
    mono.mono = Some("Ada");
    let turn = &all(&mono)[0];
    assert_eq!(types(turn), ["user", "header", "text"]);
    assert_eq!(turn["items"][1]["live"], false);
}

#[test]
fn prompt_and_answer_settle_with_a_footer() {
    let rows = [sent("user", "Hi 👋", 1_000), row("assistant", "Hello!")];
    let mut infos = HashMap::new();
    infos.insert(
        1,
        TurnInfo {
            provider: "codex".into(),
            model: "GPT".into(),
            duration_ms: 2_000,
            duration_known: true,
            started_at: Some(1_000),
            ..TurnInfo::default()
        },
    );
    let turns = all(&source(&rows, &infos, false));
    assert_eq!(turns.len(), 1);
    let turn = &turns[0];
    assert_eq!(turn["id"], "t0");
    assert_eq!(types(turn), ["user", "header", "text"]);
    let items = &turn["items"];
    assert_eq!(items[0]["text"], "Hi 👋");
    assert_eq!(items[0]["at"], 1_000);
    assert!(items[0].get("emojiOnly").is_none());
    assert_eq!(items[1]["harness"], "codex");
    assert_eq!(items[1]["model"], "GPT");
    assert_eq!(items[1]["durationMs"], 2_000);
    assert_eq!(items[1]["entries"], json!([]));
    assert!(items[1].get("label").is_none());
    assert_eq!(items[2]["text"], "Hello!");
    assert!(items[2].get("streaming").is_none());
    assert_eq!(turn["footer"], true);
    assert_eq!(turn["completedAt"], 3_000);
    assert_eq!(turn["copyText"], "Hello!");
    assert_eq!(turn["durationMs"], 2_000);
    assert!(turn.get("metrics").is_none());
    assert_eq!(
        messages(&rows),
        [
            json!({"id":"r0","role":"user","text":"Hi 👋"}),
            json!({"id":"r1","role":"assistant","text":"Hello!"})
        ]
    );
}

#[test]
fn answered_work_folds_into_the_header() {
    let rows = [
        row("user", "Fix it"),
        row("assistant", "Let me look."),
        tool(
            "Read",
            json!({"file_path": "/repo/src/a.rs"}),
            ToolState::Completed,
        ),
        tool("Bash", json!({"command": "cargo test"}), ToolState::Failed),
        row("assistant", "Fixed."),
    ];
    let infos = HashMap::new();
    let turn = &all(&source(&rows, &infos, false))[0];
    assert_eq!(types(turn), ["user", "header", "text"]);
    let header = &turn["items"][1];
    let entries = header["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0],
        json!({"type":"prose","id":"r1","text":"Let me look."})
    );
    assert_eq!(entries[1]["type"], "phase");
    assert_eq!(entries[1]["id"], "p2");
    // Ties go to the kind that changed the most: a run outranks a read.
    assert_eq!(entries[1]["kind"], "run");
    let steps = entries[1]["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 2);
    assert!(steps[0]["label"].as_str().unwrap().starts_with("Read"));
    assert_eq!(steps[1]["failed"], true);
    assert_eq!(turn["items"][2]["text"], "Fixed.");
    assert_eq!(turn["copyText"], "Let me look.\n\nFixed.");
    // Legacy turns without timing still settle, without an invented duration.
    assert!(header.get("durationMs").is_none());
    assert_eq!(turn["footer"], true);
}

#[test]
fn live_turn_streams_its_work_and_answer() {
    let mut rows = vec![
        row("user", "Go"),
        tool(
            "Read",
            json!({"file_path": "/repo/a.rs"}),
            ToolState::Completed,
        ),
        tool(
            "Read",
            json!({"file_path": "/repo/b.rs"}),
            ToolState::Running,
        ),
    ];
    let infos = HashMap::new();
    let mut live = source(&rows, &infos, true);
    live.waiting = Some("Waiting for approval");
    let turn = &all(&live)[0];
    assert_eq!(types(turn), ["user", "header", "work"]);
    let header = &turn["items"][1];
    assert_eq!(header["live"], true);
    assert_eq!(header["model"], "Opus");
    assert_eq!(header["waiting"], "Waiting for approval");
    let work = &turn["items"][2];
    assert_eq!(work["live"], true);
    assert_eq!(work["phases"][0]["title"], "Reading 2 files");
    assert!(turn.get("footer").is_none());

    rows.push(row("assistant", "Both read"));
    let turn = &all(&source(&rows, &infos, true))[0];
    assert_eq!(types(turn), ["user", "header", "text"]);
    assert_eq!(turn["items"][2]["streaming"], true);
    assert_eq!(turn["items"][1]["entries"].as_array().unwrap().len(), 1);

    // A lone call with no group is its own row.
    let rows = [
        row("user", "Go"),
        tool("Bash", json!({"command": "ls"}), ToolState::Running),
    ];
    let turn = &all(&source(&rows, &infos, true))[0];
    let phase = &turn["items"][2]["phases"][0];
    assert_eq!(phase["steps"], json!([]));
    assert_eq!(phase["kind"], "run");
    assert!(phase["title"].as_str().unwrap().contains("ls"));
}

#[test]
fn errors_and_interjections_are_notices() {
    let note = Interjection {
        text: "Heads up".into(),
        custom_type: "advisor".into(),
        severity: None,
    };
    let rows = [
        row("user", "Go"),
        row("interjection", &serde_json::to_string(&note).unwrap()),
        row("error", "Provider disconnected"),
        row("error", "Stopped"),
    ];
    let infos = HashMap::new();
    let turn = &all(&source(&rows, &infos, false))[0];
    assert_eq!(types(turn), ["user", "header", "notice", "notice"]);
    assert_eq!(
        turn["items"][2],
        json!({"type":"notice","id":"r1","text":"Heads up"})
    );
    assert_eq!(
        turn["items"][3],
        json!({"type":"notice","id":"r2","text":"Provider disconnected","error":true})
    );
}

#[test]
fn turn_ids_survive_appends_and_ranges_slice() {
    let mut rows = vec![
        row("user", "One"),
        row("assistant", "A"),
        row("user", "Two"),
    ];
    let infos = HashMap::new();
    let before = turn_ids(&source(&rows, &infos, false));
    assert_eq!(before, ["t0", "t2"]);
    rows.push(row("assistant", "B"));
    rows.push(row("user", "Three"));
    let after = turn_ids(&source(&rows, &infos, false));
    assert_eq!(after[..2], before[..]);
    assert_eq!(after.len(), 3);
    let built = build(&source(&rows, &infos, false), 1..3);
    assert_eq!(built.len(), 2);
    assert_eq!(built[0]["id"], "t2");
}

#[test]
fn stacked_prompts_compact_and_attachments_count() {
    let files = json!([{"name": "a.png", "kind": "image"}, {"name": "b.png", "kind": "image"}]);
    let rows = [
        row("user", "First"),
        row("user", "👍"),
        row(ATTACHMENTS, &files.to_string()),
        row("assistant", "Ok"),
    ];
    let infos = HashMap::new();
    let all_turns = all(&source(&rows, &infos, false));
    let items = &all_turns[0]["items"];
    assert_eq!(items[0]["compact"], true);
    assert_eq!(items[1]["emojiOnly"], true);
    assert_eq!(items[1]["attachments"], 2);
}

#[test]
fn mono_notifications_keep_only_the_final_message() {
    let rows = [
        row("user", "Review the heartbeat"),
        row("assistant", "Waiting for the review."),
        row("mono-system", "The review session finished"),
        row("assistant", "I'll verify the findings."),
        row("assistant", "Verification is complete."),
        row("assistant", "The review found three bugs."),
        Row {
            first: false,
            ..row("assistant", " The report is in Documents.")
        },
    ];
    let infos = HashMap::new();
    let mut mono = source(&rows, &infos, true);
    mono.mono = Some("Ada");
    let text_items = |turn: &Value| {
        turn["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["type"] == "text")
            .map(|item| item["text"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(text_items(&all(&mono)[0]), ["Waiting for the review."]);
    mono.busy = false;
    let turns = all(&mono);
    assert_eq!(turns.len(), 1);
    assert_eq!(
        text_items(&turns[0]),
        [
            "Waiting for the review.",
            "The review found three bugs. The report is in Documents.",
        ]
    );
    assert_eq!(
        turns[0]["copyText"],
        "Waiting for the review.\n\nThe review found three bugs. The report is in Documents."
    );
}

#[test]
fn mono_turns_sign_their_header_and_keep_their_opening_sentence() {
    let rows = [
        sent("user", "Plan my day", 1_000),
        row("assistant", "Checking your calendar"),
        tool(
            "Read",
            json!({"file_path": "/repo/cal"}),
            ToolState::Completed,
        ),
        row("assistant", "You are free after 3."),
        sent("user", "Thanks", 2_000),
        row("assistant", "<reaction>🙌</reaction>"),
    ];
    let infos = HashMap::new();
    let mut mono = source(&rows, &infos, false);
    mono.mono = Some("Ada");
    let turns = all(&mono);
    assert_eq!(turns.len(), 2);
    assert_eq!(types(&turns[0]), ["user", "header", "text", "text"]);
    assert_eq!(turns[0]["items"][1]["label"], "Ada");
    assert_eq!(turns[0]["items"][2]["text"], "Checking your calendar");
    assert_eq!(turns[0]["items"][3]["text"], "You are free after 3.");
    assert_eq!(turns[0]["stampAt"], 1_000);
    assert_eq!(
        turns[0]["copyText"],
        "Checking your calendar\n\nYou are free after 3."
    );
    // A reply that is only a reaction leaves the message on its own.
    assert_eq!(types(&turns[1]), ["user"]);
    assert_eq!(turns[1]["items"][0]["reaction"], "🙌");
    assert!(turns[1].get("footer").is_none());

    let mut live = source(&rows[..3], &infos, true);
    live.mono = Some("Ada");
    let turn = &all(&live)[0];
    assert_eq!(types(turn), ["user", "text", "header"]);
    assert_eq!(turn["items"][2]["label"], "Reading file…");
}

#[test]
fn a_monos_opening_sentence_stays_above_the_work_that_follows_it() {
    let rows = [
        sent("user", "Any PRs?", 1_000),
        row("assistant", "I'll check the latest PRs."),
        tool(
            "Bash",
            json!({"command": "gh pr list"}),
            ToolState::Completed,
        ),
        row("assistant", "Started a background check."),
    ];
    let infos = HashMap::new();
    let mut mono = source(&rows, &infos, false);
    mono.mono = Some("Ada");
    let turn = &all(&mono)[0];
    assert_eq!(types(turn), ["user", "header", "text", "text"]);
    assert_eq!(turn["items"][2]["text"], "I'll check the latest PRs.");

    // While it works: the opening shows, its status trails under it.
    let mut live = source(&rows[..3], &infos, true);
    live.mono = Some("Ada");
    let turn = &all(&live)[0];
    assert_eq!(types(turn), ["user", "text", "header"]);
}

#[test]
fn a_monos_launched_sessions_land_on_cards_under_its_reply() {
    let rows = [
        sent("user", "Fix the composer", 1_000),
        tool(
            "Bash",
            json!({"command": "monocode app sessions.start --json {}"}),
            ToolState::Completed,
        ),
        row("assistant", "Started a session on it."),
    ];
    let mut call: ToolCall = serde_json::from_str(&rows[1].text).unwrap();
    call.output = r#"{"ok":true,"result":{"sessionId":7,"accepted":true}}"#.into();
    let rows = [
        rows[0].clone(),
        row("tool", &serde_json::to_string(&call).unwrap()),
        rows[2].clone(),
    ];
    let infos = HashMap::new();
    let card = json!({"id": "s-7", "projectId": "%2Frepo", "project": "repo",
        "title": "Fix the composer", "harness": "codex", "state": "working"});
    let launches = HashMap::from([(7, card.clone())]);
    let mut mono = source(&rows, &infos, false);
    mono.mono = Some("Ada");
    mono.launches = &launches;
    let turn = &all(&mono)[0];
    assert_eq!(types(turn), ["user", "header", "text", "sessions"]);
    assert_eq!(turn["items"][3]["sessions"], json!([card]));

    // Not while the run is still going, and never for an unknown session.
    let mut live = source(&rows, &infos, true);
    live.mono = Some("Ada");
    live.launches = &launches;
    assert!(!types(&all(&live)[0]).contains(&"sessions"));
    mono.launches = Box::leak(Box::default());
    assert!(!types(&all(&mono)[0]).contains(&"sessions"));
}

#[test]
fn a_monos_choices_and_documents_come_as_cards() {
    let rows = [
        sent("user", "What next?", 1_000),
        row("assistant", "Pick one."),
        row(
            "mono-card",
            r#"{"id":"c","type":"choices","options":["Review it","Wait"]}"#,
        ),
        row(
            "mono-document",
            r#"{"id":"d1","type":"document","documentId":"doc","title":"Draft","kind":"document"}"#,
        ),
        row(
            "mono-document",
            r#"{"id":"d2","type":"document","documentId":"doc","title":"Report","kind":"document"}"#,
        ),
        sent("user", "Review it", 2_000),
    ];
    let infos = HashMap::new();
    let mut mono = source(&rows, &infos, false);
    mono.mono = Some("Ada");
    let turns = all(&mono);
    assert_eq!(
        types(&turns[0]),
        ["user", "header", "text", "choices", "document"]
    );
    let items = &turns[0]["items"];
    assert_eq!(items[3]["options"], json!(["Review it", "Wait"]));
    assert_eq!(items[3]["chosen"], "Review it");
    // A document written twice is listed once, at its latest title.
    assert_eq!(items[4]["documentId"], "doc");
    assert_eq!(items[4]["title"], "Report");

    // Unanswered until a reply follows.
    let mut open = source(&rows[..5], &infos, false);
    open.mono = Some("Ada");
    assert!(all(&open)[0]["items"][3].get("chosen").is_none());
}
