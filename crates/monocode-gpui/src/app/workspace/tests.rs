#[test]
fn legacy_empty_session_choices_survive_upgrade_and_new_choices_take_precedence() {
    for (preferences, expected) in [
        (serde_json::json!({}), "ink"),
        (serde_json::json!({"empty_session_games": false}), "none"),
        (serde_json::json!({"empty_session_games": true}), "games"),
        (
            serde_json::json!({"empty_session_games": false, "empty_session": "games"}),
            "games",
        ),
        (
            serde_json::json!({"empty_session_games": true, "empty_session": "none"}),
            "none",
        ),
    ] {
        let saved =
            serde_json::json!({"preferences": preferences, "sidebar_width": 280.}).to_string();
        let state = super::Workspace::restore(Some(&saved), &[], "/project");
        assert_eq!(state.preferences.empty_session, expected);
        assert_eq!(state.sidebar_width, 280.);
        let saved = serde_json::to_string(&state).unwrap();
        assert!(!saved.contains("empty_session_games"));
        let reopened = super::Workspace::restore(Some(&saved), &[], "/project");
        assert_eq!(reopened.preferences.empty_session, expected);
    }
}

#[test]
fn rail_moves_stay_inside_their_section_and_follow_the_drawn_order() {
    let mut state = super::Workspace {
        classic_layout: true,
        projects: ["/a", "/b", "/c", "/d", "/e", "/f"]
            .map(str::to_owned)
            .to_vec(),
        project_rail_pinned: ["/e".to_owned()].into(),
        project_rail_groups: vec![super::ProjectRailGroup {
            id: "g".into(),
            name: "Group".into(),
            ..Default::default()
        }],
        project_rail_assignments: [("/b", "g"), ("/d", "g"), ("/f", "deleted")]
            .map(|(p, g)| (p.to_owned(), g.to_owned()))
            .into(),
        ..Default::default()
    };
    // Pinned, then the group, then the rest. A deleted group's member
    // falls back to the ungrouped list.
    assert_eq!(
        state.visible_project_order(),
        ["/e", "/b", "/d", "/a", "/c", "/f"]
    );
    assert_eq!(state.project_peers("/f"), ["/a", "/c", "/f"]);
    assert!(state.step_project_order("/f", -1));
    assert_eq!(state.project_peers("/a"), ["/a", "/f", "/c"]);
    assert!(!state.step_project_order("/b", -1), "/b leads its group");
    assert!(state.move_project_to_end("/b", 1));
    assert_eq!(state.project_peers("/d"), ["/d", "/b"]);
    assert!(!state.step_project_order("/e", 1), "/e is the only pin");
    // The projects row shows the saved order as it is.
    state.classic_layout = false;
    assert_eq!(state.visible_project_order(), state.projects);
    assert!(state.move_project_to_end("/e", -1));
    assert_eq!(state.projects[0], "/e");
}

#[test]
fn legacy_boxy_layout_and_hidden_rail_migrate_without_changing_appearance() {
    let state = super::Workspace::restore(
        Some(r#"{"classic_layout":true,"preferences":{"collapsed_project_rail":"hidden"}}"#),
        &[],
        "/work",
    );
    assert!(!state.classic_layout);
    assert!(state.new_layout_boxy);
    assert!(state.boxy_layout());
    assert_eq!(state.preferences.collapsed_project_rail, "compact");
    let saved = serde_json::to_string(&state).unwrap();
    let mut state = super::Workspace::restore(Some(&saved), &[], "/work");
    state.classic_layout = true;
    state.project_rail_open = false;
    state.project_rail_width = 248.;
    let saved = serde_json::to_string(&state).unwrap();
    let state = super::Workspace::restore(Some(&saved), &[], "/work");
    assert!(state.classic_layout && state.new_layout_boxy);
    assert!(!state.project_rail_open);
    assert_eq!(state.project_rail_width, 248.);
}

use super::*;
#[test]
fn navigation_prunes_closed_tabs_and_a_new_visit_discards_forward() {
    let mut h = VisitHistory::default();
    h.record(1);
    h.record(2);
    h.record(3);
    assert_eq!(h.step(-1), Some(2));
    h.prune(&[1, 3], Some(3));
    assert_eq!(h.step(-1), Some(1));
    assert_eq!(h.step(1), Some(3));
    h.step(-1);
    h.record(4);
    assert!(h.forward.is_empty());
    for id in 5..100 {
        h.record(id);
    }
    assert_eq!(h.back.len(), 50);
}
fn mono(id: &str) -> TitleTab {
    TitleTab::Mono {
        id: id.into(),
        project: "/work".into(),
        worktree: "/work".into(),
    }
}
#[test]
fn a_mono_joins_a_split_and_becomes_its_tab_again_when_alone() {
    let mut s = Workspace::default();
    s.open(1);
    s.title_order.push(mono("m"));
    s.open(2);
    let pane = s.place_mono(mono("m"), 1, Edge::Right).unwrap();
    assert!(pane < 0);
    assert_eq!(s.layout(1).leaf_ids(), vec![1, pane]);
    assert_eq!(s.active, Some(pane));
    assert_eq!(s.scope_pane(1), 1, "a Mono never scopes its tab");
    assert_eq!(
        s.title_order,
        vec![TitleTab::Session(1), TitleTab::Session(2)]
    );
    assert_eq!(s.mono_pane("m"), Some(pane));
    assert_eq!(s.settle_mono_panes(), None);
    // Restarting keeps the Mono in its split.
    let saved = serde_json::to_string(&s).unwrap();
    let restored = Workspace::restore(
        Some(&saved),
        &[(1, "/work".into()), (2, "/work".into())],
        "/work",
    );
    assert_eq!(restored.layout(1).leaf_ids(), vec![1, pane]);
    assert_eq!(restored.mono_pane("m"), Some(pane));
    // Detached to the strip, it is the Mono tab again in the same slot.
    s.detach(pane, 1);
    assert_eq!(s.settle_mono_panes(), Some(mono("m")));
    assert_eq!(
        s.title_order,
        vec![TitleTab::Session(1), mono("m"), TitleTab::Session(2)]
    );
    assert!(s.mono_panes.is_empty());
    assert_eq!(s.tabs, vec![1, 2]);
}
#[test]
fn a_tab_dropped_on_the_visible_mono_splits_beside_it() {
    let mut s = Workspace::default();
    s.open(1);
    s.title_order.push(mono("m"));
    let pane = s.mono_tab_pane(mono("m"));
    assert_eq!(
        s.title_order,
        vec![TitleTab::Session(1), TitleTab::Session(pane)]
    );
    s.place_tab(1, pane, Edge::Left);
    assert_eq!(s.tabs, vec![1], "the conversation keys the tab");
    assert_eq!(s.layout(1).leaf_ids(), vec![1, pane]);
    assert_eq!(s.title_order, vec![TitleTab::Session(1)]);
    assert_eq!(s.settle_mono_panes(), None);
    // Closing the conversation's pane leaves the Mono on its own.
    assert!(s.close_pane(1));
    assert_eq!(s.settle_mono_panes(), Some(mono("m")));
    assert_eq!(s.title_order, vec![mono("m")]);
    assert!(s.tabs.is_empty());
    // A closed tab forgets its Mono pane.
    s.open(1);
    s.place_mono(mono("m"), 1, Edge::Bottom);
    s.close(1);
    assert_eq!(s.settle_mono_panes(), None);
    assert!(s.mono_panes.is_empty());
}
#[test]
fn close_and_reopen_keep_session_identity() {
    let mut s = Workspace::default();
    s.open(1);
    s.open(2);
    s.open(3);
    s.open(2);
    assert_eq!(s.tabs, vec![1, 2, 3]);
    s.close(1);
    assert_eq!(s.active, Some(2));
    s.close(2);
    assert_eq!(s.active, Some(3));
    s.open(2);
    s.open(3);
    assert_eq!(s.active, Some(3));
    s.close(3);
    s.close(2);
    assert_eq!(s.active, None);
    s.open(1);
    assert_eq!(s.tabs, vec![1]);
}
#[test]
fn split_tabs_place_detach_and_close_panes_without_losing_sessions() {
    let mut s = Workspace::default();
    s.open(1);
    s.open(2);
    s.open(1);
    s.split(3, SplitDir::Right);
    s.tab_workspaces.insert(1, "/selected".into());
    s.tab_workspaces.insert(2, "/other".into());
    assert_eq!(s.tabs, [1, 2]);
    assert_eq!(s.layout(1).leaf_ids(), [1, 3]);
    assert_eq!((s.active, s.active_tab()), (Some(3), Some(1)));
    // Another tab's session joins this one; its old tab closes.
    s.place(2, 1, Edge::Bottom);
    assert_eq!(s.tabs, [1]);
    assert!(!s.tab_workspaces.contains_key(&2));
    assert_eq!(s.layout(1).leaf_ids(), [1, 2, 3]);
    assert_eq!(s.open_sessions(), [1, 2, 3]);
    // Closing the key pane rekeys the tab and keeps its place.
    s.open(4);
    s.open(1);
    assert!(s.close_pane(1));
    assert_eq!(s.tabs, [2, 4]);
    assert_eq!(s.tab_workspaces[&2], "/selected");
    assert!(!s.tab_workspaces.contains_key(&1));
    assert_eq!(s.active, Some(2));
    assert_eq!(s.tab_focus(2), 2);
    s.open(3);
    s.open(s.tab_focus(4));
    assert_eq!(s.active, Some(4));
    s.open(s.tab_focus(2));
    assert_eq!(s.active, Some(3));
    // A pane dragged to the strip becomes a tab where it was dropped.
    s.detach(3, 0);
    assert_eq!(s.tabs, [3, 2, 4]);
    assert_eq!(s.tab_workspaces[&3], "/selected");
    assert!(!s.layouts.contains_key(&2));
    assert!(!s.close_pane(3));
    // A whole tab dropped on a pane moves in intact.
    s.split(5, SplitDir::Down);
    s.place_tab(3, 4, Edge::Left);
    assert_eq!(s.tabs, [2, 4]);
    assert!(!s.tab_workspaces.contains_key(&3));
    assert_eq!(s.layout(4).leaf_ids(), [3, 5, 4]);
    assert_eq!(s.active, Some(5));
    s.close(5);
    assert_eq!(s.tabs, [2]);
    assert_eq!(s.tab_workspaces.len(), 1);
    assert_eq!(s.active, Some(2));
}
#[test]
fn restore_keeps_split_tabs_and_prunes_their_missing_panes() {
    let mut s = Workspace::default();
    s.open(1);
    s.split(2, SplitDir::Right);
    s.split(3, SplitDir::Down);
    s.open(9);
    s.open(2);
    let json = serde_json::to_string(&s).unwrap();
    let sessions: Vec<_> = [2, 3].iter().map(|id| (*id, "/w".into())).collect();
    let s = Workspace::restore(Some(&json), &sessions, "/w");
    assert_eq!(s.tabs, [2]);
    assert_eq!(s.layout(2).leaf_ids(), [2, 3]);
    assert_eq!(s.active, Some(2));
    assert_eq!(s.tab_focus(2), 2);
}
#[test]
fn terminal_docks_add_close_move_and_clamp_like_project_terminal() {
    let mut dock = TerminalDock::default();
    assert_eq!((dock.side, dock.size), (DockSide::Bottom, 220.));
    dock.add("/w/a".into());
    dock.add("/w/b".into());
    dock.add("/w/c".into());
    assert_eq!(dock.active, 2);
    dock.active = 1;
    assert!(dock.close(1));
    assert_eq!((dock.terminals.len(), dock.active), (2, 1));
    assert!(dock.close(0));
    assert_eq!(dock.active, 0);
    assert!(!dock.close(0));
    dock.set_side(DockSide::Right);
    assert_eq!(dock.size, 360.);
    dock.size = 500.;
    dock.set_side(DockSide::Left);
    assert_eq!(dock.size, 500.);
    assert_eq!(DockSide::Bottom.clamp(10., (1280., 800.)), 88.);
    assert_eq!(DockSide::Bottom.clamp(900., (1280., 800.)), 560.);
    assert_eq!(DockSide::Left.clamp(f32::NAN, (1280., 800.)), 360.);
    let mut s = Workspace::default();
    s.docks.insert("/w".into(), dock.clone());
    let mut kept = dock.clone();
    kept.add("/w".into());
    s.docks.insert("/k".into(), kept);
    let json = serde_json::to_string(&s).unwrap();
    let s = Workspace::restore(Some(&json), &[], "/w");
    assert!(!s.docks.contains_key("/w"));
    assert_eq!(s.docks["/k"].terminals, ["/w"]);
}
#[test]
fn terminal_docks_reorder_keeping_the_active_terminal_and_close_others() {
    let mut dock = TerminalDock::default();
    for cwd in ["/a", "/b", "/c", "/d"] {
        dock.add(cwd.into());
    }
    dock.active = 1;
    dock.reorder(1, 3);
    assert_eq!(dock.terminals, ["/a", "/c", "/d", "/b"]);
    assert_eq!(dock.active, 3);
    dock.reorder(0, 2);
    assert_eq!(dock.terminals, ["/c", "/d", "/a", "/b"]);
    assert_eq!(dock.active, 3);
    dock.reorder(3, 0);
    assert_eq!(dock.terminals, ["/b", "/c", "/d", "/a"]);
    assert_eq!(dock.active, 0);
    dock.active = 2;
    dock.reorder(3, 1);
    assert_eq!(dock.terminals, ["/b", "/a", "/c", "/d"]);
    assert_eq!(dock.active, 3);
    dock.keep_only(2);
    assert_eq!(
        (dock.terminals.clone(), dock.active),
        (vec!["/c".to_string()], 0)
    );
}
#[test]
fn restore_keeps_project_selection_and_prunes_stale_project_focus() {
    let sessions = vec![(3, "/b".into()), (2, "/a".into()), (1, "/a".into())];
    let mut state = Workspace::default();
    for id in [1, 2, 3] {
        state.open(id);
    }
    state.project = "/a".into();
    state.active = Some(99);
    state.project_focus.insert("/a".into(), 1);
    state.project_focus.insert("/b".into(), 99);
    state.worktree_focus.insert(
        "/a".into(),
        crate::git::worktree_switcher::WorktreeFocus {
            path: "/a-tree".into(),
            branch: Some("feature".into()),
        },
    );
    state.tab_workspaces.insert(1, "/a-tree".into());
    let mut saved = serde_json::to_value(&state).unwrap();
    assert!(saved.get("worktree_focus").is_none());
    assert!(saved.get("tab_workspaces").is_none());
    // Older native sidecars saved this transient filter. Ignore it too.
    saved["worktree_focus"] = serde_json::json!({
        "/a": {"path":"/a-tree", "branch":"feature"}
    });
    let json = serde_json::to_string(&saved).unwrap();
    let restored = Workspace::restore(Some(&json), &sessions, "/b");
    assert_eq!(restored.active, Some(1));
    assert_eq!(restored.project, "/a");
    assert!(restored.worktree_focus.is_empty());
    assert!(restored.tab_workspaces.is_empty());
    assert!(!restored.project_focus.contains_key("/b"));
    state.active = Some(3);
    let json = serde_json::to_string(&state).unwrap();
    let restored = Workspace::restore(Some(&json), &sessions, "/a");
    assert_eq!(restored.active, Some(3));
    assert_eq!(restored.project, "/b");
    let fresh = Workspace::restore(None, &sessions, "/a");
    assert_eq!(fresh.active, Some(2));
    assert_eq!(fresh.project, "/a");
    assert!(fresh.new_layout_boxy);
    let rounded = Workspace::restore(Some(r#"{"new_layout_boxy":false}"#), &sessions, "/a");
    assert!(!rounded.new_layout_boxy);
}
#[test]
fn restoring_history_keeps_only_remembered_or_selected_projects() {
    let saved = serde_json::json!({
        "project": "/code/app",
        "projects": ["/code/app", "/code/other"],
        "tabs": [2],
        "active": 2,
        "desktop_project_preferences": {"projects": ["/code/app", "/code/other"]}
    })
    .to_string();
    let sessions = vec![
        (1, "/code/app".into()),
        (2, "/tmp/internal-mono-chat".into()),
        (3, "/private/var/folders/temporary".into()),
    ];
    let restored = Workspace::restore(Some(&saved), &sessions, "/code/app");
    assert_eq!(restored.projects, ["/code/app", "/code/other"]);
    assert_eq!(restored.project, "/code/app");
    assert_eq!(restored.active, None);
    let serialized = serde_json::to_value(restored).unwrap();
    assert_eq!(
        serialized["desktop_project_preferences"]["projects"],
        serde_json::json!(["/code/app", "/code/other"])
    );
}
#[test]
fn restore_prunes_stale_tabs_and_preserves_an_intentionally_empty_workspace() {
    let mut s = Workspace::default();
    s.open(1);
    s.open(8);
    s.tabs.push(1);
    s.sidebar_width = 900.;
    let json = serde_json::to_string(&s).unwrap();
    let s = Workspace::restore(Some(&json), &[(1, "/work/a".into())], "/work/b");
    assert_eq!(s.tabs, vec![1]);
    assert_eq!(s.active, None);
    assert_eq!(s.project, "/work/b");
    assert_eq!(s.sidebar_width, 560.);
    assert_eq!(s.projects, vec!["/work/b"]);
    let s = Workspace::restore(Some("{}"), &[(1, "/work/a".into())], "/work/a");
    assert!(s.tabs.is_empty());
    assert!(
        s.compact_session_cards,
        "older workspaces retain compact cards"
    );
    assert!(
        s.sidebar_activity_visible,
        "older workspaces keep the activity card visible"
    );
}
#[test]
fn desktop_snapshot_translation_deserializes_and_retains_unported_metadata() {
    let source = serde_json::json!({
        "projectCwd":"/work", "activeTabId":"tab",
        "tabs":[{"id":"tab", "focusedId":"b", "groupId":"assigned", "editorPanes":[], "layout":{"type":"split", "dir":"down", "children":[{"type":"leaf","id":"a"},{"type":"leaf","id":"b"}], "sizes":[0.4,0.6]}}],
        "projectTerminals":[{"projectPath":"/work", "side":"bottom", "size":240, "open":false, "pane":{"activeFileId":"term", "files":[{"id":"term", "terminal":true, "cwd":"/work/sub"}]}}]
    });
    let mapped = monocode_core::store::desktop_preferences::DesktopPreferences::workspace_snapshot(
        &source,
        |id| match id {
            "a" => Some(1),
            "b" => Some(2),
            _ => None,
        },
    );
    let saved = mapped.to_string();
    let restored = Workspace::restore(
        Some(&saved),
        &[(1, "/work".into()), (2, "/work".into())],
        "/work",
    );
    assert_eq!(restored.tabs, [1]);
    assert_eq!(restored.active, Some(2));
    assert!(restored.layouts[&1].is_split());
    assert_eq!(restored.focused[&1], 2);
    assert_eq!(restored.docks["/work"].terminals, ["/work/sub"]);
    assert!(!restored.docks["/work"].open);
    assert_eq!(restored.desktop_workspace_snapshot.as_ref(), Some(&source));
}
