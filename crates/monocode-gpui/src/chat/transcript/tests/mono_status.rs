use super::super::*;

fn row(role: &str, text: &str) -> Row {
    Row {
        role: role.into(),
        text: text.into(),
        first: true,
        sent_at: None,
    }
}
fn tool(name: &str, input: serde_json::Value) -> Row {
    let mut call = ToolCall::new("t", name);
    call.input = input.to_string();
    row("tool", &serde_json::to_string(&call).unwrap())
}
fn label(rows: Vec<Row>) -> String {
    mono_work_label(&rows, 0)
}

#[test]
fn status_says_what_the_mono_is_doing() {
    let prompt = row("user", "Review the open PRs");
    assert_eq!(
        label(vec![
            prompt.clone(),
            row("reasoning", "**Finding the Monocode repo**\n\nLooking…")
        ]),
        "Finding the Monocode repo…"
    );
    // Raw thinking prose has no heading, so it stays generic.
    assert_eq!(
        label(vec![
            prompt.clone(),
            row("reasoning", "The user wants me to look at PRs.")
        ]),
        "Thinking…"
    );
    assert_eq!(
        label(vec![
            prompt.clone(),
            tool(
                "Bash",
                serde_json::json!({"command":"gh pr list","description":"List open PRs"})
            )
        ]),
        "List open PRs…"
    );
    assert_eq!(
        label(vec![
            prompt.clone(),
            tool(
                "Bash",
                serde_json::json!({"command":"gh pr list --state open"})
            )
        ]),
        "Looking at pull requests…"
    );
    assert_eq!(
        label(vec![
            prompt.clone(),
            tool(
                "mcp__monocode_app__app",
                serde_json::json!({"action":"memory.add","input":{"fact":"x"}})
            )
        ]),
        "Remembering that…"
    );
    assert_eq!(
        label(vec![
            prompt,
            tool(
                "mcp__github__search_repositories",
                serde_json::json!({"query":"monocode"})
            )
        ]),
        "Search repositories…"
    );
}
