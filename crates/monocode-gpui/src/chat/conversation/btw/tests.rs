use super::*;
fn row(role: &str, text: &str, first: bool) -> Row {
    Row {
        role: role.into(),
        text: text.into(),
        first,
        sent_at: None,
    }
}
#[test]
fn btw_tabs_label_and_motion_match_the_sheet() {
    assert_eq!(compact_question(None), "New question");
    assert_eq!(compact_question(Some("  a\n  b ")), "a b");
    let long = "x".repeat(60);
    assert_eq!(
        compact_question(Some(&long)),
        format!("{}…", "x".repeat(45))
    );
    assert_eq!(status_color("draft"), None);
    assert!(status_color("stopped").is_some());
    for value in [0., 0.3, 0.75, 1.] {
        assert!((fx::ease(invert(OPEN_EASE, value), OPEN_EASE) - value).abs() < 1e-3);
    }
    assert_eq!(outline_opacity(0.), 0.);
    assert_eq!(outline_opacity(0.5), 1.);
    assert!(outline_opacity(1.) <= 1e-6);
}
#[test]
fn snapshots_exclude_private_reasoning_and_keep_utf8_bounds() {
    let rows = vec![
        row("user", "Why?", true),
        row("reasoning", "private", true),
        row("assistant", &"🙂".repeat(20_000), true),
    ];
    let text = snapshot(&rows);
    assert!(!text.contains("private"));
    assert!(text.contains("Why?"));
    assert!(text.chars().count() <= SNAPSHOT_CHARS);
}
#[test]
fn prompts_keep_reference_and_side_messages_separate() {
    let thread = Thread {
        id: "one".into(),
        source_turn: 0,
        snapshot: "User: main".into(),
        provider: "codex".into(),
        model: None,
        cwd: "/repo".into(),
        account: Default::default(),
        effort: None,
        model_settings: HashMap::new(),
        environment: HashMap::new(),
        rows: vec![row("user", "Side question", true)],
        draft: String::new(),
        status: "draft".into(),
        error: None,
        desktop: None,
    };
    let prompt = Btw::prompt(&thread);
    assert!(prompt.contains("reference context only"));
    assert!(prompt.contains("Side question"));
    assert!(prompt.contains("Do not change files"));
}
