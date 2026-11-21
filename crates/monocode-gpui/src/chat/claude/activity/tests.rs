use super::names::{Usage, millis, step};
use super::*;

fn feed(log: &mut Log, lines: &[&str]) {
    for line in lines {
        log.line(line);
    }
}

#[test]
fn counts_usage_once_per_call_and_prices_it_like_claude_code() {
    let mut log = Log::default();
    // Claude Code's own record: 2 in, 20 out, 13,423 read, 7,617 written
    // for an hour cost $0.064029 on Opus 5.5.
    let reply = r#"{"type":"assistant","message":{"id":"m1","model":"claude-opus-5-5","stop_reason":"tool_use","usage":{"input_tokens":2,"output_tokens":20,"cache_read_input_tokens":13423,"cache_creation_input_tokens":7617,"cache_creation":{"ephemeral_1h_input_tokens":7617}},"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/p/a.rs"}}]},"timestamp":"2026-11-14T10:00:00.000Z"}"#;
    let second = r#"{"type":"assistant","message":{"id":"m1","model":"claude-opus-5-5","usage":{"input_tokens":2,"output_tokens":20,"cache_read_input_tokens":13423,"cache_creation_input_tokens":7617,"cache_creation":{"ephemeral_1h_input_tokens":7617}},"content":[{"type":"tool_use","id":"t2","name":"mcp__claude_ai_Gmail__get_thread","input":{}}]},"timestamp":"2026-11-14T10:00:30.000Z"}"#;
    feed(&mut log, &[reply, second]);
    assert_eq!(log.tokens(), 2 + 20 + 13423 + 7617);
    assert!((log.cost() - 0.064029).abs() < 1e-6, "{}", log.cost());
    assert_eq!(log.last - log.first, 30_000);
    assert_eq!(log.files[0].order, ["/p/a.rs"]);
    assert_eq!(
        log.tools.order,
        ["Read", "mcp__claude_ai_Gmail__get_thread"]
    );
    // A write without a breakdown is priced at the 5 minute rate:
    // 20 in, 1,500 out, 480,916 read, 18,147 written cost $0.216998.
    let usage = Usage::from(
        &serde_json::json!({"input_tokens":20,"output_tokens":1500,"cache_read_input_tokens":480916,"cache_creation_input_tokens":18147}),
    );
    assert!((usage.cost("claude-opus-5-5") - 0.216998).abs() < 1e-6);
}

#[test]
fn reads_skills_writes_notices_and_context() {
    let mut log = Log::default();
    feed(
        &mut log,
        &[
            r#"{"type":"user","message":{"content":"<command-message>ponytail:ponytail</command-message>\n<command-name>/ponytail:ponytail</command-name>"}}"#,
            r#"{"type":"user","message":{"content":"<command-name>/clear</command-name>\n<command-message>clear</command-message>"}}"#,
            r#"{"type":"assistant","message":{"id":"m","model":"claude-fable-5-2","content":[{"type":"tool_use","id":"w1","name":"Write","input":{"file_path":"/p/new.md"}},{"type":"tool_use","id":"w2","name":"Write","input":{"file_path":"/p/old.md"}},{"type":"tool_use","id":"s","name":"Skill","input":{"skill":"ponytail:ponytail"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"w1"}]},"toolUseResult":{"type":"create","filePath":"/p/new.md"}}"#,
            r#"{"type":"user","message":{"content":"<task-notification>\n<task-id>a1</task-id>\n<status>failed</status>\n</task-notification>"},"timestamp":"2026-11-14T10:00:00.000Z"}"#,
            r#"{"type":"attachment","attachment":{"type":"instructions","files":[{"path":"/home/u/CLAUDE.md","type":"Project"}]}}"#,
        ],
    );
    assert_eq!(log.skills.order, ["ponytail:ponytail"]);
    assert_eq!(log.skills.counts["ponytail:ponytail"], 2);
    assert!(log.created.contains("w1") && !log.created.contains("w2"));
    assert_eq!(log.notices["a1"].0, AgentStatus::Failed);
    assert_eq!(log.context.order, ["/home/u/CLAUDE.md"]);
}

#[test]
fn names_models_servers_and_steps() {
    assert_eq!(model_name("claude-opus-5-5"), "Opus 5.5");
    assert_eq!(model_name("claude-haiku-4-5-20251001"), "Haiku 4.5");
    assert_eq!(model_name("claude-fable-5"), "Fable 5");
    for wide in [
        "claude-opus-5-5",
        "claude-fable-5",
        "claude-haiku-5-5",
        "claude-sonnet-5",
    ] {
        assert!(super::names::wide_by_default(wide), "{wide}");
    }
    for narrow in [
        "claude-opus-4-8",
        "claude-haiku-4-5-20251001",
        "claude-3-5-sonnet-20241022",
        "",
    ] {
        assert!(!super::names::wide_by_default(narrow), "{narrow}");
    }
    assert_eq!(server_name("claude_ai_Google_Calendar"), "Google Calendar");
    assert_eq!(
        server_name("plugin_antislop_antislop-contrast"),
        "antislop-contrast"
    );
    assert_eq!(server_name("figcraft"), "figcraft");
    assert_eq!(
        mcp_parts("mcp__figcraft__open_file"),
        Some(("figcraft", "open_file"))
    );
    assert_eq!(
        step("Bash", &serde_json::json!({"description": "Run the tests"})),
        "Run the tests"
    );
    assert_eq!(
        step("Write", &serde_json::json!({"file_path": "/a/b/plan.md"})),
        "Writing plan.md"
    );
}

#[test]
fn reads_appended_lines_and_starts_over_after_a_rewrite() {
    let dir = std::env::temp_dir().join(format!("claude-activity-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.jsonl");
    let reply = |id: &str| {
        format!(
            "{{\"type\":\"assistant\",\"message\":{{\"id\":\"{id}\",\"model\":\"claude-opus-5-5\",\"usage\":{{\"output_tokens\":10}},\"content\":[]}}}}\n"
        )
    };
    std::fs::write(&path, reply("a") + &reply("b")[..20]).unwrap();
    let mut log = Log::default();
    log.pull(&path);
    assert_eq!(log.tokens(), 10, "the half-written line waits");
    std::fs::write(&path, reply("a") + &reply("b")).unwrap();
    log.pull(&path);
    assert_eq!(log.tokens(), 20);
    std::fs::write(&path, reply("c")).unwrap();
    log.pull(&path);
    assert_eq!(log.tokens(), 10);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn lists_files_most_recently_used_first_across_agents() {
    let read = |id: &str, path: &str, second: &str| {
        format!(
            r#"{{"type":"assistant","message":{{"id":"{id}","model":"claude-opus-5-5","content":[{{"type":"tool_use","id":"{id}","name":"Read","input":{{"file_path":"{path}"}}}}]}},"timestamp":"2026-11-17T10:00:{second}.000Z"}}"#
        )
    };
    let mut main = Log::default();
    feed(
        &mut main,
        &[
            &read("r1", "/p/a.rs", "01"),
            &read("r2", "/p/b.rs", "02"),
            &read("r3", "/p/a.rs", "03"),
        ],
    );
    assert_eq!(
        main.files[0].recent(),
        ["/p/a.rs", "/p/b.rs"],
        "a file read again moves back to the top"
    );
    // An agent's later read comes first, whichever log it is in.
    let mut agent = Log::agent();
    feed(&mut agent, &[&read("r4", "/p/c.rs", "04")]);
    let mut merged = Tally::default();
    merged.merge(&main.files[0]);
    merged.merge(&agent.files[0]);
    assert_eq!(merged.recent(), ["/p/c.rs", "/p/a.rs", "/p/b.rs"]);
    assert_eq!(merged.counts["/p/a.rs"], 2);
}

#[test]
fn follows_background_work_until_it_ends_and_the_context_it_fills() {
    let call = |id: &str, name: &str, input: &str, used: u32, second: &str| {
        format!(
            r#"{{"type":"assistant","message":{{"id":"m{id}","model":"claude-opus-5-5","usage":{{"input_tokens":10,"cache_read_input_tokens":{used}}},"content":[{{"type":"tool_use","id":"{id}","name":"{name}","input":{input}}}]}},"timestamp":"2026-11-17T10:00:{second}.000Z"}}"#
        )
    };
    let mut tracker = Tracker::default();
    feed(
        &mut tracker.main,
        &[
            &call(
                "b1",
                "Bash",
                r#"{"command":"npm run build\nnpm test","run_in_background":true}"#,
                9,
                "01",
            ),
            &call(
                "b2",
                "Bash",
                r#"{"command":"cargo test","run_in_background":true}"#,
                9,
                "02",
            ),
            &call(
                "b3",
                "PowerShell",
                r#"{"command":"Start-Sleep 99","run_in_background":true}"#,
                9,
                "03",
            ),
            &call(
                "a1",
                "Agent",
                r#"{"description":"Review the diff","prompt":"p","run_in_background":"true"}"#,
                9,
                "04",
            ),
            &call(
                "m1",
                "Monitor",
                r#"{"command":"tail -f log","description":"errors","timeout_ms":60000}"#,
                9,
                "05",
            ),
            &call("r1", "Read", r#"{"file_path":"/p/a.rs"}"#, 150_000, "06"),
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"b3"}]},"toolUseResult":{"backgroundTaskId":"bg3"}}"#,
            // Ended by a notice queued while Claude worked, and by a stop.
            r#"{"type":"queue-operation","operation":"enqueue","content":"<task-notification>\n<task-id>bg2</task-id>\n<tool-use-id>b2</tool-use-id>\n<status>completed</status>\n</task-notification>","timestamp":"2026-11-17T10:00:07.000Z"}"#,
            &call("s1", "TaskStop", r#"{"task_id":"bg3"}"#, 9, "08"),
        ],
    );
    let account = Profile::default();
    let at = |time: &str| millis(time).unwrap();
    let running = |tracker: &Tracker, now: i64| {
        let snapshot = tracker.snapshot(&account, now, false);
        let shown = snapshot.background.iter();
        shown
            .map(|b| format!("{} {}", b.tool, b.detail))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        running(&tracker, at("2026-11-17T10:00:30.000Z")),
        [
            "Bash npm run build",
            "Agent Review the diff",
            "Monitor tail -f log"
        ]
    );
    // A monitor dies at its deadline without a word.
    assert_eq!(
        running(&tracker, at("2026-11-17T10:01:10.000Z")),
        ["Bash npm run build", "Agent Review the diff"]
    );
    feed(
        &mut tracker.main,
        &[
            r#"{"type":"attachment","attachment":{"type":"queued_command","prompt":"<task-notification>\n<task-id>x</task-id>\n<tool-use-id>a1</tool-use-id>\n<status>killed</status>\n</task-notification>"},"timestamp":"2026-11-17T10:00:09.000Z"}"#,
        ],
    );
    assert_eq!(
        running(&tracker, at("2026-11-17T10:00:30.000Z")),
        ["Bash npm run build", "Monitor tail -f log"]
    );
    // The context is the last reply's; a compaction shrinks it. Opus 5.5 has 1M.
    let snapshot = tracker.snapshot(&account, 0, false);
    assert_eq!(
        (snapshot.context_used, snapshot.context_window),
        (19, 1_000_000)
    );
    feed(
        &mut tracker.main,
        &[
            r#"{"type":"system","subtype":"compact_boundary","compactMetadata":{"postTokens":20004}}"#,
        ],
    );
    assert_eq!(tracker.snapshot(&account, 0, false).context_used, 20004);
    assert_eq!(
        tracker.snapshot(&account, 0, true).context_window,
        1_000_000
    );
    // Past 200K, the window can only be 1M.
    feed(
        &mut tracker.main,
        &[&call("r2", "Read", "{}", 250_000, "09")],
    );
    assert_eq!(
        tracker.snapshot(&account, 0, false).context_window,
        1_000_000
    );
    // A command that outlives its timeout moves to the background.
    feed(
        &mut tracker.main,
        &[
            &call("b4", "Bash", r#"{"command":"cargo test --all"}"#, 9, "10"),
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"b4"}]},"toolUseResult":{"backgroundTaskId":"bg4","timedOutAfterMs":600000}}"#,
            &call("b5", "Bash", r#"{"command":"git status"}"#, 9, "11"),
        ],
    );
    assert_eq!(
        running(&tracker, at("2026-11-17T10:00:30.000Z")),
        [
            "Bash npm run build",
            "Monitor tail -f log",
            "Bash cargo test --all"
        ]
    );
}

#[test]
fn drops_subagent_work_that_died_with_its_agent_or_was_stopped() {
    let call = |id: &str, name: &str, input: &str, second: &str| {
        format!(
            r#"{{"type":"assistant","message":{{"id":"m{id}","model":"claude-opus-5-5","content":[{{"type":"tool_use","id":"{id}","name":"{name}","input":{input}}}]}},"timestamp":"2026-11-20T10:00:{second}.000Z"}}"#
        )
    };
    let result = |id: &str, result: &str| {
        format!(
            r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"{id}"}}]}},"toolUseResult":{result}}}"#
        )
    };
    let mut agent = Log::agent();
    let mut tracker = Tracker::default();
    feed(
        &mut agent,
        &[
            &call(
                "b1",
                "Bash",
                r#"{"command":"cargo test","run_in_background":true}"#,
                "01",
            ),
            &result("b1", r#"{"backgroundTaskId":"bg1"}"#),
        ],
    );
    // Killed with its agent: only the agent's own notice says so.
    feed(
        &mut tracker.main,
        &[
            r#"{"type":"queue-operation","operation":"enqueue","content":"<task-notification>\n<task-id>a1</task-id>\n<tool-use-id>t1</tool-use-id>\n<status>killed</status>\n</task-notification>","timestamp":"2026-11-20T10:00:02.000Z"}"#,
        ],
    );
    // Continued, then a monitor stopped by the id its result names.
    feed(
        &mut agent,
        &[
            &call(
                "m2",
                "Monitor",
                r#"{"command":"tail -f log","timeout_ms":1800000}"#,
                "03",
            ),
            &result(
                "m2",
                r#"{"taskId":"mon2","timeoutMs":1800000,"persistent":false}"#,
            ),
            &call("s2", "TaskStop", r#"{"task_id":"mon2"}"#, "04"),
            &call(
                "b3",
                "Bash",
                r#"{"command":"npm test","run_in_background":true}"#,
                "05",
            ),
        ],
    );
    tracker.agents.insert(
        "a1".to_owned(),
        AgentLog {
            log: agent,
            meta: Value::Null,
        },
    );
    let now = millis("2026-11-20T10:00:30.000Z").unwrap();
    let snapshot = tracker.snapshot(&Profile::default(), now, false);
    let shown: Vec<_> = snapshot.background.iter().map(|b| &b.detail).collect();
    assert_eq!(shown, ["npm test"]);
}

#[test]
fn a_big_transcript_leaves_only_its_unfinished_line_in_memory() {
    let dir = std::env::temp_dir().join(format!("claude-activity-big-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.jsonl");
    // About 9 MB, so it is read in several chunks.
    let line = "{\"type\":\"user\",\"message\":{\"content\":\"x\"}}\n";
    std::fs::write(&path, line.repeat(200_000) + "{\"half").unwrap();
    let mut log = Log::default();
    log.pull(&path);
    assert_eq!(log.partial, b"{\"half");
    assert!(log.partial.capacity() < 1024, "{}", log.partial.capacity());
    std::fs::remove_dir_all(dir).unwrap();
}
