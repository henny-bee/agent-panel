use super::super::*;

#[test]
fn visual_tools_read_as_chart_steps_under_every_provider_name() {
    for name in [
        "visual_show",
        "monocode_visual:visual_show",
        "mcp__monocode_visual__visual_show",
        "monocode_visual.visual_show",
    ] {
        assert_eq!(visual_tool(name), Some(VisualStep::Show), "{name}");
    }
    assert_eq!(visual_tool("other_server:visual_show"), None);
    assert_eq!(visual_tool("visual_publish"), None);
    let mut call = ToolCall::new("1", "monocode_visual:visual_show");
    call.input = r#"{"title":"Stars per week","source":"return () => null"}"#.into();
    call.state = ToolState::Completed;
    let shown = call.presentation();
    assert_eq!(
        (shown.action.as_str(), shown.target.as_str()),
        ("Drew", "Stars per week")
    );
}
