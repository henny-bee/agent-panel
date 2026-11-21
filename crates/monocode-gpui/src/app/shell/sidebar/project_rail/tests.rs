use super::*;
use gpui::{EntityInputHandler, TestAppContext, VisualTestContext, size};

#[gpui::test]
fn original_layout_keeps_project_icons_when_sessions_are_hidden(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = crate::app::shell::tests::window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(size(px(1500.), px(900.)));
    window
        .update(visual, |shell, _, cx| shell.toggle_layout(cx))
        .unwrap();
    visual.run_until_parked();
    let projects = visual.debug_bounds("project-rail-frame").unwrap();
    let sessions = visual.debug_bounds("sidebar-frame").unwrap();
    let main = visual.debug_bounds("main-pane-frame").unwrap();
    assert_eq!(projects.left(), px(0.));
    // The workspace sidebar holds the window's right edge.
    assert_eq!(main.left(), projects.right());
    assert_eq!(sessions.left(), main.right());
    let grip = visual.debug_bounds("project-rail-resize").unwrap().center();
    let end = grip + gpui::point(px(28.), px(0.));
    visual.simulate_mouse_down(grip, MouseButton::Left, Default::default());
    visual.simulate_mouse_move(end, MouseButton::Left, Default::default());
    visual.simulate_mouse_up(end, MouseButton::Left, Default::default());
    visual.run_until_parked();
    assert_eq!(
        visual
            .debug_bounds("project-rail-frame")
            .unwrap()
            .size
            .width,
        projects.size.width + px(28.)
    );
    assert_eq!(
        visual.debug_bounds("sidebar-frame").unwrap().size.width,
        sessions.size.width
    );
    let toggle = visual.debug_bounds("toggle-sidebar").unwrap();
    visual.simulate_click(toggle.center(), Default::default());
    visual.run_until_parked();
    assert_eq!(
        visual
            .debug_bounds("project-rail-frame")
            .unwrap()
            .size
            .width,
        px(48.)
    );
    assert!(visual.debug_bounds("sidebar-frame").is_some());
    window
        .update(visual, |shell, w, cx| shell.toggle_sidebar(w, cx))
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("sidebar-frame").is_none());
    assert_eq!(
        visual
            .debug_bounds("project-rail-frame")
            .unwrap()
            .size
            .width,
        px(48.)
    );
    assert_eq!(
        visual.debug_bounds("main-pane-frame").unwrap().left(),
        px(48.)
    );
    window
        .update(visual, |shell, w, cx| {
            shell.open_settings(crate::settings::Section::Appearance, w, cx);
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("sidebar-frame").is_none());
    assert!(visual.debug_bounds("settings-nav-appearance").is_some());
    window
        .update(visual, |shell, w, cx| shell.close_settings(w, cx))
        .unwrap();
    visual.run_until_parked();
    assert_eq!(
        visual
            .debug_bounds("project-rail-frame")
            .unwrap()
            .size
            .width,
        px(48.)
    );
}

#[gpui::test]
fn original_project_rail_reorders_groups_and_projects(cx: &mut TestAppContext) {
    cx.update(crate::app::shell::commands::bind_keys);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = crate::app::shell::tests::window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(size(px(1500.), px(900.)));
    window
        .update(visual, |shell, _, cx| {
            shell.workspace.classic_layout = true;
            shell.workspace.project_rail_open = true;
            shell.workspace.preferences.monos = false;
            shell.workspace.project = "/rail/a".into();
            shell.workspace.projects = ["/rail/a", "/rail/b", "/rail/c", "/rail/d"]
                .into_iter()
                .map(str::to_owned)
                .collect();
            shell.workspace.project_rail_groups = [("first", "First"), ("second", "Second")]
                .into_iter()
                .map(|(id, name)| workspace::ProjectRailGroup {
                    id: id.into(),
                    name: name.into(),
                    ..Default::default()
                })
                .collect();
            shell.workspace.project_rail_assignments = [
                ("/rail/a", "first"),
                ("/rail/b", "first"),
                ("/rail/c", "second"),
                ("/rail/d", "second"),
            ]
            .into_iter()
            .map(|(p, g)| (p.into(), g.into()))
            .collect();
            shell.invalidate_chrome(cx);
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    let first = visual.debug_bounds("rail-group-first").unwrap();
    let second = visual.debug_bounds("rail-group-second").unwrap();
    let member = visual.debug_bounds("rail-project-button-0").unwrap();
    let delta = second.top() - first.top();
    visual.simulate_mouse_down(first.center(), MouseButton::Left, Default::default());
    window
        .update(visual, |shell, _, _| {
            let drag = shell
                .project_rail_drag
                .as_ref()
                .expect("pressing a group header starts its reorder");
            assert_eq!(drag.path, "first");
            assert_eq!(drag.order, vec!["first", "second"]);
        })
        .unwrap();
    visual.simulate_mouse_move(second.center(), MouseButton::Left, Default::default());
    window
        .update(visual, |shell, _, _| {
            let drag = shell
                .project_rail_drag
                .as_ref()
                .expect("drag survives pointer movement");
            assert!(drag.active);
            assert_eq!(drag.offset, delta);
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(
        visual.debug_bounds("rail-group-first").unwrap().top(),
        first.top() + delta
    );
    let moved = visual.debug_bounds("rail-project-button-0").unwrap();
    assert_eq!(
        moved.top(),
        member.top() + delta,
        "the group's members must travel with its header"
    );
    visual.simulate_mouse_up(second.center(), MouseButton::Left, Default::default());
    visual.run_until_parked();
    window
        .update(visual, |shell, w, cx| {
            assert_eq!(
                shell
                    .workspace
                    .project_rail_groups
                    .iter()
                    .map(|g| g.id.as_str())
                    .collect::<Vec<_>>(),
                vec!["second", "first"]
            );
            assert!(
                !shell.project_rail_animating(),
                "reduced motion must settle immediately"
            );
            assert!(shell.project_rail_drag.is_none());
            assert_eq!(shell.workspace.project_rail_assignments["/rail/a"], "first");
            let saved = locked(&shell.store).unwrap().ui_state().unwrap().unwrap();
            let restored = workspace::Workspace::restore(Some(&saved), &[], "/rail/a");
            assert_eq!(restored.project_rail_groups[0].id, "second");
            shell.focus_workspace(w, cx);
        })
        .unwrap();
    let a = visual.debug_bounds("rail-project-button-0").unwrap();
    let b = visual.debug_bounds("rail-project-button-1").unwrap();
    visual.simulate_mouse_down(a.center(), MouseButton::Left, Default::default());
    visual.simulate_mouse_move(b.center(), MouseButton::Left, Default::default());
    visual.simulate_mouse_up(b.center(), MouseButton::Left, Default::default());
    visual.run_until_parked();
    window
        .update(visual, |shell, _, _| {
            assert_eq!(
                shell.workspace.projects,
                vec!["/rail/b", "/rail/a", "/rail/c", "/rail/d"]
            );
            let saved = locked(&shell.store).unwrap().ui_state().unwrap().unwrap();
            let restored = workspace::Workspace::restore(Some(&saved), &[], "/rail/a");
            assert_eq!(restored.projects, shell.workspace.projects);
        })
        .unwrap();
    let second = visual.debug_bounds("rail-group-second").unwrap();
    let first = visual.debug_bounds("rail-group-first").unwrap();
    visual.simulate_mouse_down(second.center(), MouseButton::Left, Default::default());
    visual.simulate_mouse_move(first.center(), MouseButton::Left, Default::default());
    visual.simulate_keystrokes("escape");
    visual.simulate_mouse_up(first.center(), MouseButton::Left, Default::default());
    visual.run_until_parked();
    window
        .update(visual, |shell, _, _| {
            assert_eq!(
                shell
                    .workspace
                    .project_rail_groups
                    .iter()
                    .map(|g| g.id.as_str())
                    .collect::<Vec<_>>(),
                vec!["second", "first"]
            );
            assert!(shell.project_rail_drag.is_none());
        })
        .unwrap();
}

#[gpui::test]
fn project_rail_virtualizes_projects_without_loading_their_sessions(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = crate::app::shell::tests::window(cx);
    let count = window
        .update(cx, |shell, _, cx| {
            for i in 0..1000 {
                shell
                    .workspace
                    .add_project(&format!("/projects/project-{i}"));
            }
            shell.toggle_layout(cx);
            shell.sessions.len()
        })
        .unwrap();
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    assert!(visual.debug_bounds("rail-project-0").is_some());
    assert!(visual.debug_bounds("rail-project-999").is_none());
    window
        .update(visual, |shell, _, _| {
            assert_eq!(shell.sessions.len(), count)
        })
        .unwrap();
}

#[gpui::test]
fn project_rail_and_transcript_stay_cached_during_independent_updates(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, active) = crate::app::shell::tests::window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(size(px(1500.), px(900.)));
    window
        .update(visual, |shell, _, cx| shell.toggle_layout(cx))
        .unwrap();
    visual.run_until_parked();
    for compact in [false, true] {
        window
            .update(visual, |shell, _, cx| {
                shell.workspace.project_rail_open = !compact;
                shell.invalidate_chrome(cx);
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        let (projects, sidebar, tabs, transcript) = window
            .update(visual, |shell, _, cx| {
                (
                    shell.projects_frame.clone(),
                    shell.sidebar_frame.clone(),
                    shell.tabs_frame.clone(),
                    shell.sessions[&active].read(cx).transcript.clone(),
                )
            })
            .unwrap();
        let before = [
            projects.read_with(visual, |v, _| v.renders),
            sidebar.read_with(visual, |v, _| v.renders),
            tabs.read_with(visual, |v, _| v.renders),
        ];
        assert!(before.iter().all(|count| *count > 0));
        window
            .update(visual, |shell, w, cx| {
                shell.sessions[&active].update(cx, |view, cx| {
                    view.input.update(cx, |input, cx| {
                        input.replace_text_in_range(None, "typing", w, cx)
                    });
                    view.transcript.update(cx, |t, cx| {
                        t.patch(
                            0,
                            vec![monocode_core::session::transcript::Row {
                                role: "assistant".into(),
                                text: "Streaming output".into(),
                                first: true,
                                sent_at: None,
                            }],
                            cx,
                        )
                    });
                    cx.notify();
                });
            })
            .unwrap();
        visual.refresh().unwrap();
        visual.run_until_parked();
        assert_eq!(
            projects.read_with(visual, |v, _| v.renders),
            before[0],
            "streaming must reuse the project rail"
        );
        assert_eq!(sidebar.read_with(visual, |v, _| v.renders), before[1]);
        assert_eq!(tabs.read_with(visual, |v, _| v.renders), before[2]);
        let before = transcript.read_with(visual, |v, _| v.render_passes);
        if !compact {
            let inbox = visual.debug_bounds("sidebar-navigation-Inbox").unwrap();
            visual.simulate_mouse_move(inbox.center(), None, Default::default());
            visual.run_until_parked();
            let notes = visual.debug_bounds("sidebar-navigation-Notes").unwrap();
            visual.simulate_mouse_move(notes.center(), None, Default::default());
            visual.run_until_parked();
            assert_eq!(
                transcript.read_with(visual, |v, _| v.render_passes),
                before,
                "rail hover must not rebuild the chat"
            );
        }
    }
}

#[gpui::test]
fn new_boxy_style_is_independent_of_the_navigation_layout(cx: &mut TestAppContext) {
    let (window, _, active) = crate::app::shell::tests::window(cx);
    window
        .update(cx, |shell, w, cx| {
            shell.open_settings(crate::settings::Section::Appearance, w, cx);
            shell.settings.clone().unwrap().update(cx, |_, cx| {
                cx.emit(crate::settings::SettingsEvent::NewLayoutBoxy(true))
            });
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            assert!(!shell.workspace.classic_layout);
            assert!(shell.workspace.new_layout_boxy);
            shell.toggle_layout(cx);
            assert!(shell.workspace.classic_layout);
            shell.toggle_layout(cx);
            assert!(!shell.workspace.classic_layout && shell.workspace.new_layout_boxy);
            assert!(shell.sessions.contains_key(&active));
        })
        .unwrap();
}
