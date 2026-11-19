use super::super::*;
use monocode_core::session::tool::{ToolCall, ToolState};
use serde_json::json;

#[test]
fn successful_edits_keep_real_paths_across_the_whole_session() {
    let row = |name: &str, state, input: serde_json::Value| {
        let mut tool = ToolCall::new("tool", name);
        tool.state = state;
        tool.input = input.to_string();
        Row {
            role: "tool".into(),
            text: serde_json::to_string(&tool).unwrap(),
            first: true,
            sent_at: None,
        }
    };
    let rows = vec![
        row(
            "Edit",
            ToolState::Completed,
            json!({"file_path":"/repo/a.rs", "cwd":"/repo"}),
        ),
        row(
            "Read",
            ToolState::Completed,
            json!({"file_path":"/repo/read.rs"}),
        ),
        row(
            "Write",
            ToolState::Failed,
            json!({"file_path":"/repo/failed.rs"}),
        ),
        row(
            "Edit",
            ToolState::Running,
            json!({"file_path":"/repo/running.rs"}),
        ),
        Row {
            role: "user".into(),
            text: "A plain question".into(),
            first: true,
            sent_at: None,
        },
        row(
            "fileChange",
            ToolState::Completed,
            json!({"path":"/repo/a.rs", "paths":["/repo/a.rs", "relative.rs"]}),
        ),
    ];
    assert_eq!(edited_paths(&rows), ["/repo/a.rs", "relative.rs"]);
}
