mod visual;
use super::*;
use serde_json::json;
fn call(name: &str, input: Value) -> ToolCall {
    let mut call = ToolCall::new("one", name);
    call.input = input.to_string();
    call
}

#[test]
fn mono_app_calls_read_as_what_they_do() {
    assert!(app_tool("mcp__monocode_app__app"));
    assert!(app_tool("monocode_app.app"));
    assert!(!app_tool("mcp__other__app"));
    assert!(!app_tool("mcp__monocode_visual__visual_show"));
    assert_eq!(
        app_cli_action(
            r#"'/Applications/Monocode.app/Contents/MacOS/monocode-gpui' app memory.add --json '{"fact":"x"}'"#
        ),
        Some("memory.add")
    );
    assert_eq!(app_cli_action("git log app memory.add"), None);
    assert_eq!(app_cli_action("monocode-gpui app help"), None);

    let running = call(
        "mcp__monocode_app__app",
        json!({"action":"sessions.start","input":{"prompt":"x"}}),
    );
    let shown = running.presentation();
    assert_eq!(
        (shown.action.as_str(), shown.target.as_str()),
        ("Starting", "a session")
    );
    let mut done = call(
        "Bash",
        json!({"command":"/opt/monocode-gpui app memory.add --json '{\"fact\":\"x\"}'"}),
    );
    done.state = ToolState::Completed;
    let shown = done.presentation();
    assert_eq!(
        (shown.action.as_str(), shown.target.as_str()),
        ("Remembered", "that")
    );
}

#[test]
fn search_pills_separate_simple_regex_alternatives() {
    for (name, input) in [
        (
            "Grep",
            json!({"pattern":"model_settings_menu|render_model_settings|self.models|models.clone"}),
        ),
        (
            "Bash",
            json!({"command":"rg -n 'model_settings_menu|render_model_settings|self.models|models.clone' src"}),
        ),
    ] {
        let shown = call(name, input).presentation();
        assert_eq!(
            shown.search_patterns(),
            [
                "model_settings_menu",
                "render_model_settings",
                "self.models",
                "models.clone"
            ]
        );
    }
}

#[test]
fn search_pills_preserve_regex_structure_and_literal_searches() {
    for query in [
        r"foo\|bar",
        "[a|b]",
        "(foo|bar)|baz",
        "(?i)foo|bar",
        "foo||bar",
        "|foo",
        "foo|",
    ] {
        let shown = call("Grep", json!({"pattern":query})).presentation();
        assert_eq!(shown.search_patterns(), [query]);
    }
    for command in [
        "rg -F 'foo|bar' src",
        "rg -nF 'foo|bar' src",
        "rg --fixed-strings 'foo|bar' src",
        "grep 'foo|bar' src/a.rs",
        "fgrep 'foo|bar' src/a.rs",
    ] {
        let shown = call("Bash", json!({"command":command})).presentation();
        assert_eq!(shown.search_patterns(), ["foo|bar"], "{command}");
    }
    let shown = call("WebSearch", json!({"query":"foo|bar"})).presentation();
    assert_eq!(shown.search_patterns(), ["foo|bar"]);
}
#[test]
fn live_codex_uses_last_meaningful_stage_and_cwd_boundary() {
    let call = call(
        "commandExecution",
        json!({"command":"cat /repo/src/a.rs | rg needle", "cwd":"/repo", "commandActions":[{"type":"read", "path":"/repo/src/a.rs"},{"type":"search","query":"needle","path":"/repo/src/a.rs"},{"type":"unknown"}]}),
    );
    let shown = call.presentation();
    assert_eq!(
        (shown.action.as_str(), shown.target.as_str()),
        ("Find", "needle")
    );
    assert_eq!(shown.file_path.as_deref(), Some("/repo/src/a.rs"));
    assert_eq!(
        display_path("/repository/file", Some("/repo")),
        "/repository/file"
    );
}
#[test]
fn read_supports_nested_arguments_and_line() {
    let call = call(
        "Read",
        json!({"arguments":{"file_path":"/repo/src/lib.rs","offset":42},"cwd":"/repo"}),
    );
    let shown = call.presentation();
    assert_eq!(shown.target, "src/lib.rs");
    assert_eq!(shown.file_path.as_deref(), Some("/repo/src/lib.rs"));
    assert_eq!(shown.start_line, Some(42));
}
#[test]
fn scripts_are_unwrapped_only_for_display() {
    let input =
        json!({"command":["/bin/zsh","-lc","sed -n '12,30p' 'src/my file.rs'"], "cwd":"/repo"});
    let call = call("commandExecution", input.clone());
    let shown = call.presentation();
    assert_eq!(
        (shown.action.as_str(), shown.target.as_str()),
        ("Read", "src/my file.rs")
    );
    assert_eq!(shown.start_line, Some(12));
    assert_eq!(
        shown.command.as_deref(),
        Some("sed -n '12,30p' 'src/my file.rs'")
    );
    assert_eq!(call.input, input.to_string());
}
#[test]
fn opaque_or_mutating_commands_stay_commands() {
    for command in [
        "npm run cat",
        "python -c 'print(1)'",
        "cat x; cargo test",
        "sed -i '1,5p' file.rs",
        "cat x > y",
        "cat $(touch x)",
        "rg --pre script needle src",
        "head -n 20",
    ] {
        let shown = call("Bash", json!({"command":command})).presentation();
        assert_eq!(shown.action, "Run", "{command}");
        assert_eq!(shown.target, command);
        assert_eq!(shown.file_path, None);
    }
}
#[test]
fn old_rollout_actions_and_pathless_file_lists_are_readable() {
    let shown = call(
        "commandExecution",
        json!({"command":"rg --files -g AGENTS.md", "parsed_cmd":[{"type":"listFiles"}]}),
    )
    .presentation();
    assert_eq!(
        (shown.action.as_str(), shown.target.as_str()),
        ("Find", "files")
    );
    let shown = call("commandExecution", json!({"command":"opaque", "parsed_cmd":[{"type":"read","path":"/repo/a.rs"}], "cwd":"/repo"})).presentation();
    assert_eq!(shown.target, "a.rs");
}
#[test]
fn encoded_arguments_and_worktree_base_are_display_only() {
    let input = json!({"arguments":"{ \"filePath\": \"/repo/feature/src/main.rs\", \"line\": 9 }"});
    let call = call("Read", input.clone());
    let shown = call.presentation_in(Some("/repo/feature"));
    assert_eq!(shown.target, "src/main.rs");
    assert_eq!(
        shown.file_path.as_deref(),
        Some("/repo/feature/src/main.rs")
    );
    assert_eq!(shown.start_line, Some(9));
    assert_eq!(call.input, input.to_string());
    assert_eq!(
        serde_json::from_str::<Value>(&call.input_detail()).unwrap(),
        input
    );
}
#[test]
fn chained_commands_split_into_captioned_steps() {
    let shown = call(
        "Bash",
        json!({"command":"git status --short && cat package.json && sed -n '1,260p' /repo/scripts/build.mjs; npm test || true", "cwd":"/repo", "description":"Inspect build scripts\nmore"}),
    )
    .presentation();
    assert_eq!(shown.action, "Run");
    assert_eq!(shown.description.as_deref(), Some("Inspect build scripts"));
    let steps: Vec<_> = shown
        .steps
        .iter()
        .map(|s| (s.op, s.action, s.target.as_str()))
        .collect();
    assert_eq!(
        steps,
        [
            (None, "Run", "git status --short"),
            (Some("&&"), "Read", "package.json"),
            (Some("&&"), "Read", "scripts/build.mjs"),
            (Some(";"), "Run", "npm test"),
            (Some("||"), "Run", "true"),
        ]
    );
    assert_eq!(shown.steps[2].start_line, Some(1));
    assert_eq!(
        shown.steps[2].file_path.as_deref(),
        Some("/repo/scripts/build.mjs")
    );
}
#[test]
fn opaque_or_single_commands_have_no_steps() {
    for command in [
        "cargo test",
        "rg foo src | head",
        "echo 'a && b'",
        "cd $(pwd) && ls",
        "(cd x && make) && ls",
        "cat <<EOF && ls\nx\nEOF",
        "cargo build 2>&1 | tail",
    ] {
        let shown = call("Bash", json!({"command":command})).presentation();
        assert!(shown.steps.is_empty(), "{command}");
    }
    let shown = call("Bash", json!({"command":"cd src && cat a.rs"})).presentation();
    assert_eq!(shown.action, "Read");
    assert!(shown.steps.is_empty());
}
