use super::super::*;
use std::os::unix::fs::PermissionsExt;
fn fixture(script: &str) -> (tempfile::TempDir, Request) {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("provider");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let request = Request {
        kind: Kind::Codex,
        binary: path,
        cwd: tmp.path().into(),
        prompt: "hello".into(),
        provider_id: None,
        model: None,
        settings: Default::default(),
    };
    (tmp, request)
}
#[test]
fn cancellation_reaps_child_and_descendant_promptly() {
    for kind in Kind::ALL {
        let (_tmp, mut request) = fixture("#!/bin/sh\nsleep 30 &\nprintf ready > ready\nwait\n");
        request.kind = kind;
        request.settings.access = AccessMode::ReadOnly;
        let ready = request.cwd.join("ready");
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let timer = thread::spawn(move || {
            // Wait until the descendant exists, then measure cancellation
            // independently of provider startup and test-runner load.
            let deadline = Instant::now() + Duration::from_secs(10);
            while !ready.exists() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            let started = ready.exists();
            let stopped_at = Instant::now();
            stop.store(true, Ordering::Relaxed);
            (started, stopped_at)
        });
        assert_eq!(run(request, cancel, |_| Ok(())).unwrap_err(), "Stopped");
        let (started, stopped_at) = timer.join().unwrap();
        assert!(started, "{} fixture did not start", kind.label());
        assert!(
            stopped_at.elapsed() < Duration::from_secs(2),
            "{} cancellation must not wait on provider output",
            kind.label()
        );
    }
}
#[test]
fn perf_replay_streams_a_whole_claude_turn() {
    // scripts/perf/fake-claude-stream.py stands in for `claude` when
    // profiling; it has to keep speaking the protocol this parser reads.
    if std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/perf/fake-claude-stream.py");
    let tmp = tempfile::tempdir().unwrap();
    let mut request = Request {
        kind: Kind::Claude,
        binary: script,
        cwd: tmp.path().into(),
        prompt: "stream".into(),
        provider_id: None,
        model: None,
        settings: Default::default(),
    };
    request
        .settings
        .environment
        .insert("REPLAY_SECONDS".into(), "0.4".into());
    request
        .settings
        .environment
        .insert("REPLAY_INTERVAL_MS".into(), "10".into());
    let (mut text, mut done) = (String::new(), false);
    run(request, Arc::new(AtomicBool::new(false)), |events| {
        for event in events {
            match event {
                Event::Text(t) => text.push_str(&t),
                Event::Done => done = true,
                _ => {}
            }
        }
        Ok(())
    })
    .unwrap();
    assert!(done, "the replay's result must end the turn");
    assert_eq!(text.chars().count(), 40 * 16);
    assert!(text.starts_with("## Step 1: tighten the frame loop"));
}
#[test]
fn crash_preserves_coalesced_partial_text() {
    // Read initialization before exiting so startup cannot race a closed stdin.
    let (_tmp, request) = fixture(
        "#!/bin/sh\nIFS= read -r init\nprintf '%s\\n' '{\"method\":\"item/agentMessage/delta\",\"params\":{\"itemId\":\"a\",\"delta\":\"partial\"}}'\n",
    );
    let mut text = String::new();
    let result = run(request, Arc::new(AtomicBool::new(false)), |events| {
        for event in events {
            if let Event::Text(t) = event {
                text.push_str(&t);
            }
        }
        Ok(())
    });
    assert!(result.unwrap_err().contains("exited"));
    assert_eq!(text, "partial");
}
#[test]
fn large_history_and_tool_frames_complete_the_turn() {
    let (_tmp, mut request) = fixture(
        r#"#!/usr/bin/env python3
import json, sys
payload = 'x' * (3 * 1024 * 1024) + 'end'
def send(value):
    print(json.dumps(value), flush=True)
for line in sys.stdin:
    value = json.loads(line)
    if value.get('id') == 1:
        send({'id': 1, 'result': {}})
    elif value.get('id') == 2:
        assert value['method'] == 'thread/resume'
        send({'id': 2, 'result': {'thread': {'id': 'large-thread', 'turns': [
            {'items': [{'id': 'history', 'type': 'agentMessage', 'text': payload}]}
        ]}}})
    elif value.get('id') == 3:
        send({'method': 'item/started', 'params': {'item': {
            'id': 'large-tool', 'type': 'commandExecution', 'command': 'large output', 'status': 'inProgress'
        }}})
        send({'method': 'item/completed', 'params': {'item': {
            'id': 'large-tool', 'type': 'commandExecution', 'status': 'completed', 'aggregatedOutput': payload
        }}})
        send({'method': 'item/agentMessage/delta', 'params': {'itemId': 'answer', 'delta': 'Complete'}})
        send({'method': 'turn/completed', 'params': {'turn': {'status': 'completed'}}})
"#,
    );
    request.provider_id = Some("large-thread".into());
    let mut text = String::new();
    let mut output = String::new();
    let mut done = false;
    run(request, Arc::new(AtomicBool::new(false)), |events| {
        for event in events {
            match event {
                Event::Text(delta) => text.push_str(&delta),
                Event::Tool(tool)
                    if tool.id == "large-tool" && tool.state == ToolState::Completed =>
                {
                    output = tool.output;
                }
                Event::Done => done = true,
                _ => {}
            }
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(text, "Complete");
    assert!(output.ends_with("end"));
    assert!(done);
}

#[test]
fn malformed_provider_json_fails() {
    // Exercise JSON parsing after startup, rather than a broken initialization pipe.
    let (_tmp, request) = fixture("#!/bin/sh\nIFS= read -r init\nprintf '{invalid json\\n'\n");
    let error = run(request, Arc::new(AtomicBool::new(false)), |_| Ok(())).unwrap_err();
    assert!(error.starts_with("Invalid provider JSON:"), "{error}");
}
