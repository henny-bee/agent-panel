use super::*;

#[gpui::test]
fn sidebar_toggle_holds_still_and_history_follows_the_sidebar(cx: &mut TestAppContext) {
    let (window, _, _) = window(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    cx.run_until_parked();
    let width = window
        .update(cx, |shell, _, _| {
            assert!(shell.workspace.sidebar_visible);
            shell.workspace.sidebar_width
        })
        .unwrap();
    let open_toggle = cx.debug_bounds("toggle-sidebar").unwrap();
    let open_forward = cx.debug_bounds("visit-forward").unwrap();
    window
        .update(cx, |shell, window, cx| shell.toggle_sidebar(window, cx))
        .unwrap();
    cx.run_until_parked();
    let closed_toggle = cx.debug_bounds("toggle-sidebar").unwrap();
    let closed_back = cx.debug_bounds("visit-back").unwrap();
    let closed_forward = cx.debug_bounds("visit-forward").unwrap();
    // The toggle sits just past the traffic lights either way.
    assert_eq!(open_toggle.origin, closed_toggle.origin);
    assert_eq!(open_toggle.left(), px(TRAFFIC_LIGHTS));
    // Open, history rides the sidebar's edge, inside its `pr-1.5`.
    assert_eq!(open_forward.right(), px(width - 6.));
    // Closed, it follows the toggle directly.
    assert_eq!(closed_back.left(), closed_toggle.right());
    assert_eq!(closed_forward.right(), px(TRAFFIC_LIGHTS + 72.));
    // Fullscreen hides the native buttons, and the toggle takes their room.
    window
        .update(cx, |_, window, _| window.toggle_fullscreen())
        .unwrap();
    cx.run_until_parked();
    window.update(cx, |_, window, _| window.refresh()).unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.debug_bounds("toggle-sidebar").unwrap().left(),
        px(NO_TRAFFIC_LIGHTS)
    );
    assert_eq!(
        cx.debug_bounds("visit-forward").unwrap().right(),
        px(NO_TRAFFIC_LIGHTS + 72.)
    );
}

#[gpui::test]
fn collapsed_sidebar_peeks_from_the_edge_and_the_toggle(cx: &mut TestAppContext) {
    let (window, _, _) = window(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    cx.run_until_parked();
    window
        .update(cx, |shell, window, cx| shell.toggle_sidebar(window, cx))
        .unwrap();
    cx.run_until_parked();
    let width = window
        .update(cx, |shell, _, _| shell.workspace.sidebar_width)
        .unwrap();
    let open = |cx: &mut gpui::VisualTestContext| {
        window.update(cx, |shell, _, _| shell.peek_open()).unwrap()
    };
    assert!(cx.debug_bounds("sidebar-peek").is_none());
    // The window's left edge peeks it out, below the title bar.
    cx.simulate_mouse_move(gpui::point(px(2.), px(20.)), None, Default::default());
    assert!(!open(cx));
    cx.simulate_mouse_move(gpui::point(px(2.), px(300.)), None, Default::default());
    cx.run_until_parked();
    assert!(open(cx));
    let panel = cx.debug_bounds("sidebar-peek").unwrap();
    assert_eq!(panel.left(), px(6.));
    assert_eq!(panel.top(), px(TITLE_BAR_HEIGHT));
    assert_eq!(panel.size.width, px(width));
    // Crossing onto the panel keeps it out; leaving it puts it away after
    // a grace period.
    cx.simulate_mouse_move(gpui::point(px(100.), px(300.)), None, Default::default());
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    cx.run_until_parked();
    assert!(open(cx));
    cx.simulate_mouse_move(
        gpui::point(px(width + 200.), px(300.)),
        None,
        Default::default(),
    );
    cx.run_until_parked();
    assert!(open(cx));
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(200));
    cx.run_until_parked();
    assert!(!open(cx));
    assert!(cx.debug_bounds("sidebar-peek").is_none());
    // Hovering the toggle peeks it too, and a click elsewhere dismisses it.
    let toggle = cx.debug_bounds("toggle-sidebar").unwrap();
    cx.simulate_mouse_move(toggle.center(), None, Default::default());
    cx.run_until_parked();
    assert!(open(cx));
    cx.simulate_click(gpui::point(px(width + 200.), px(300.)), Default::default());
    cx.run_until_parked();
    assert!(!open(cx));
    // Docking the sidebar drops the peek, and an open sidebar never peeks.
    cx.simulate_mouse_move(toggle.center(), None, Default::default());
    cx.run_until_parked();
    assert!(open(cx));
    cx.simulate_click(toggle.center(), Default::default());
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| {
            assert!(shell.workspace.sidebar_visible);
            assert!(shell.sidebar_peek.is_none());
        })
        .unwrap();
    cx.simulate_mouse_move(gpui::point(px(2.), px(300.)), None, Default::default());
    cx.run_until_parked();
    assert!(!open(cx));
}

#[gpui::test]
fn boxy_collapsed_sidebar_peeks_too(cx: &mut TestAppContext) {
    let (window, _, _) = window(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    cx.run_until_parked();
    window
        .update(cx, |shell, window, cx| {
            shell.workspace.new_layout_boxy = true;
            shell.layout_changed(cx);
            shell.toggle_sidebar(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    let open = |cx: &mut gpui::VisualTestContext| {
        window.update(cx, |shell, _, _| shell.peek_open()).unwrap()
    };
    cx.simulate_mouse_move(gpui::point(px(2.), px(300.)), None, Default::default());
    cx.run_until_parked();
    assert!(open(cx));
    assert!(cx.debug_bounds("sidebar-peek").is_some());
    // The classic layout never peeks.
    window
        .update(cx, |shell, _, cx| {
            shell.workspace.classic_layout = true;
            shell.layout_changed(cx);
        })
        .unwrap();
    cx.simulate_mouse_move(gpui::point(px(300.), px(300.)), None, Default::default());
    cx.simulate_mouse_move(gpui::point(px(2.), px(300.)), None, Default::default());
    cx.run_until_parked();
    assert!(!open(cx));
}

#[gpui::test]
fn original_sidebar_geometry_matches_tauri_chrome(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(gpui::size(px(1500.), px(900.)));
    window
        .update(visual, |shell, _, cx| {
            shell.workspace.classic_layout = true;
            shell.workspace.project_rail_open = true;
            shell.classic_sidebar_tab = sidebar::classic::ClassicSidebarTab::Sessions;
            shell.layout_changed(cx);
        })
        .unwrap();
    visual.run_until_parked();

    let projects = visual.debug_bounds("project-rail-frame").unwrap();
    let sidebar = visual.debug_bounds("sidebar-frame").unwrap();
    let header = visual.debug_bounds("classic-workspace-header").unwrap();
    let tabs = visual.debug_bounds("classic-workspace-tabs").unwrap();
    let search = visual.debug_bounds("classic-conversation-search").unwrap();
    assert_eq!(projects.size.width, px(220.));
    assert_eq!(sidebar.size.width, px(260.));
    assert_eq!(projects.top(), px(0.));
    assert_eq!(sidebar.top(), px(0.));
    assert_eq!(sidebar.right(), px(1500.));
    assert_eq!(header.top(), sidebar.top());
    assert_eq!(header.size.height, px(40.));
    assert_eq!(tabs.top(), header.bottom());
    assert_eq!(tabs.size.height, px(36.));
    assert_eq!(search.top(), tabs.bottom());
    assert_eq!(search.size.height, px(36.));
    assert_eq!(
        visual.debug_bounds("sidebar-sessions").unwrap().top(),
        search.bottom()
    );
    let main = visual.debug_bounds("main-pane-frame").unwrap();
    assert_eq!(main.left(), projects.right());
    assert_eq!(main.right(), sidebar.left());

    // The corner button beside it hides the sidebar and brings it back.
    let button = visual.debug_bounds("toggle-right-panel").unwrap();
    assert!(button.right() <= sidebar.left());
    visual.simulate_click(button.center(), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("sidebar-frame").is_none());
    assert_eq!(
        visual.debug_bounds("main-pane-frame").unwrap().right(),
        px(1500.)
    );
    let button = visual.debug_bounds("toggle-right-panel").unwrap();
    visual.simulate_click(button.center(), Default::default());
    visual.run_until_parked();
    assert_eq!(visual.debug_bounds("sidebar-frame").unwrap(), sidebar);

    let back = visual.debug_bounds("visit-back").unwrap();
    let forward = visual.debug_bounds("visit-forward").unwrap();
    let toggle = visual.debug_bounds("toggle-sidebar").unwrap();
    assert_eq!(back.right(), forward.left());
    assert_eq!(toggle.left(), px(TRAFFIC_LIGHTS));
    assert!(toggle.right() <= back.left());
    assert_eq!(forward.right(), projects.right() - px(6.));
    assert_eq!(toggle.center().y, header.center().y);
    assert!(visual.debug_bounds("sidebar-project-button").is_none());
}

#[gpui::test]
fn original_sidebar_tools_replace_sessions_in_the_left_column(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(gpui::size(px(1500.), px(900.)));
    window
        .update(visual, |shell, _, cx| {
            shell.workspace.classic_layout = true;
            shell.workspace.project_rail_open = true;
            shell.layout_changed(cx);
        })
        .unwrap();
    visual.run_until_parked();
    let sidebar = visual.debug_bounds("sidebar-frame").unwrap();
    let tabs = visual.debug_bounds("classic-workspace-tabs").unwrap();

    for (id, selector) in [("files", "explorer-header"), ("changes", "changes-header")] {
        let tab = visual
            .debug_bounds(if id == "files" {
                "classic-workspace-tab-files"
            } else {
                "classic-workspace-tab-changes"
            })
            .unwrap();
        visual.simulate_click(tab.center(), Default::default());
        visual.run_until_parked();
        let tool = visual.debug_bounds(selector).unwrap();
        assert_eq!(tool.top(), tabs.bottom());
        assert_eq!(tool.left(), sidebar.left());
        assert!(tool.right() <= sidebar.right());
        assert!(visual.debug_bounds("right-panel-frame").is_none());
        assert!(visual.debug_bounds("classic-conversation-search").is_none());
        window
            .update(visual, |shell, _, _| {
                assert_eq!(shell.classic_sidebar_tab.id(), id);
                assert_eq!(
                    shell
                        .workspace
                        .classic_sidebar_tabs
                        .get(&shell.workspace.project)
                        .map(String::as_str),
                    Some(id)
                );
            })
            .unwrap();
    }
    let sessions = visual
        .debug_bounds("classic-workspace-tab-sessions")
        .unwrap();
    visual.simulate_click(sessions.center(), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("classic-conversation-search").is_some());
    assert!(visual.debug_bounds("explorer-header").is_none());
    assert!(visual.debug_bounds("changes-header").is_none());
}

#[gpui::test]
fn original_sidebar_compact_rail_uses_vertical_workspace_tabs(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(gpui::size(px(1500.), px(900.)));
    window
        .update(visual, |shell, _, cx| {
            shell.workspace.classic_layout = true;
            shell.workspace.project_rail_open = true;
            shell.layout_changed(cx);
        })
        .unwrap();
    visual.run_until_parked();
    let toggle = visual.debug_bounds("toggle-sidebar").unwrap();
    visual.simulate_click(toggle.center(), Default::default());
    visual.run_until_parked();

    let projects = visual.debug_bounds("project-rail-frame").unwrap();
    let sidebar = visual.debug_bounds("sidebar-frame").unwrap();
    let header = visual.debug_bounds("classic-workspace-header").unwrap();
    assert_eq!(projects.size.width, px(48.));
    assert_eq!(sidebar.right(), px(1500.));
    assert_eq!(sidebar.size.width, px(260.));
    assert!(visual.debug_bounds("classic-workspace-tabs").is_none());
    assert!(visual.debug_bounds("rail-workspace-sessions").is_some());
    assert!(visual.debug_bounds("rail-workspace-files").is_some());
    assert!(visual.debug_bounds("rail-workspace-changes").is_some());
    assert_eq!(
        visual
            .debug_bounds("classic-conversation-search")
            .unwrap()
            .top(),
        header.bottom()
    );
    #[cfg(target_os = "macos")]
    {
        let strip = visual.debug_bounds("title-tab-strip").unwrap();
        assert_eq!(strip.top(), px(0.));
        assert_eq!(strip.size.height, px(40.));
        assert_eq!(projects.top(), px(40.));
        assert_eq!(header.top(), px(40.));
    }

    let expand = visual.debug_bounds("toggle-sidebar").unwrap();
    assert_eq!(expand.origin, toggle.origin);
    assert!(visual.debug_bounds("rail-expand-projects").is_none());
    assert_eq!(
        visual.debug_bounds("visit-back").unwrap().left(),
        expand.right(),
        "collapsed history follows the toggle, independent of the sessions sidebar"
    );
    assert_eq!(
        visual.debug_bounds("visit-forward").unwrap().right(),
        px(TRAFFIC_LIGHTS + 72.)
    );
    visual.simulate_click(expand.center(), Default::default());
    visual.run_until_parked();
    assert_eq!(
        visual
            .debug_bounds("project-rail-frame")
            .unwrap()
            .size
            .width,
        px(220.)
    );
    assert!(visual.debug_bounds("classic-workspace-tabs").is_some());
}

#[gpui::test]
fn original_sidebar_workspace_tabs_reorder_and_persist(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(gpui::size(px(1500.), px(900.)));
    window
        .update(visual, |shell, _, cx| {
            shell.workspace.classic_layout = true;
            shell.workspace.project_rail_open = true;
            shell.layout_changed(cx);
        })
        .unwrap();
    visual.run_until_parked();
    let sessions = visual
        .debug_bounds("classic-workspace-tab-sessions")
        .unwrap();
    let changes = visual
        .debug_bounds("classic-workspace-tab-changes")
        .unwrap();
    visual.simulate_mouse_down(sessions.center(), MouseButton::Left, Default::default());
    visual.simulate_mouse_move(changes.center(), MouseButton::Left, Default::default());
    visual.simulate_mouse_up(changes.center(), MouseButton::Left, Default::default());
    visual.run_until_parked();
    let files = visual.debug_bounds("classic-workspace-tab-files").unwrap();
    let changes = visual
        .debug_bounds("classic-workspace-tab-changes")
        .unwrap();
    let sessions = visual
        .debug_bounds("classic-workspace-tab-sessions")
        .unwrap();
    assert!(files.left() < changes.left() && changes.left() < sessions.left());
    window
        .update(visual, |shell, _, _| {
            assert_eq!(
                shell.workspace.classic_sidebar_tab_order,
                ["files", "changes", "sessions"]
            );
            assert_eq!(
                shell.classic_sidebar_tab,
                sidebar::classic::ClassicSidebarTab::Sessions
            );
            let saved = shell.latest_state.lock().unwrap().clone();
            let restored = Workspace::restore(Some(&saved), &[], "/work/monocode");
            assert_eq!(
                restored.classic_sidebar_tab_order,
                ["files", "changes", "sessions"]
            );
        })
        .unwrap();
}

#[gpui::test]
fn new_boxy_layout_runs_the_sidebar_and_workspace_full_height(cx: &mut TestAppContext) {
    crate::ui::theme::configure(&crate::ui::preferences::Preferences::default(), false);
    let (window, _, _) = window(cx);
    let frames = |cx: &mut TestAppContext| {
        cx.update_window(window.into(), |_, w, cx| {
            w.simulate_next_frame(cx);
            w.draw(cx).clear(cx);
        })
        .unwrap();
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        window
            .update(cx, |_, w, _| {
                let (blur, radius) = (24., 11.);
                let marker = gpui::backdrop_blur_marker(blur * w.scale_factor());
                assert!(
                    w.painted_quads().iter().any(|q| q.border_color == marker
                        && q.corner_radii.top_left.0 == radius * w.scale_factor()),
                    "composer frosting keeps the same blur and radius in both layouts"
                );
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        (
            visual.debug_bounds("sidebar-frame"),
            visual.debug_bounds("main-pane-frame").unwrap(),
            visual.debug_bounds("title-tab-strip").unwrap(),
        )
    };
    let (sidebar, main, tabs) = frames(cx);
    let sidebar = sidebar.unwrap();
    // Inset: the title bar spans the window above the sidebar and card.
    assert_eq!(sidebar.top(), px(TITLE_BAR_HEIGHT));
    assert_eq!(main.top(), px(TITLE_BAR_HEIGHT));
    assert!(tabs.bottom() <= main.top());
    window
        .update(cx, |shell, _, cx| {
            shell.workspace.new_layout_boxy = true;
            shell.layout_changed(cx)
        })
        .unwrap();
    let (sidebar, main, tabs) = frames(cx);
    let sidebar = sidebar.unwrap();
    assert_eq!(sidebar.top(), px(0.));
    assert_eq!(main.top(), px(0.));
    assert_eq!(main.left(), sidebar.right());
    // The strip lives in the workspace column, beside the sidebar.
    assert!(tabs.left() >= main.left());
    // The right panel toggle holds the window's corner, and the strip
    // runs up to it.
    let corner = |cx: &mut TestAppContext| {
        gpui::VisualTestContext::from_window(window.into(), cx)
            .debug_bounds("toggle-right-panel")
            .unwrap()
    };
    let closed_corner = corner(cx);
    assert_eq!(closed_corner.top(), px(TITLE_BAR_HEIGHT / 2. - 12.));
    assert!(tabs.right() <= closed_corner.left());
    window
        .update(cx, |shell, w, cx| {
            cx.set_reduce_motion(true);
            shell.toggle_right_panel(w, cx);
        })
        .unwrap();
    let (_, main_open, _) = frames(cx);
    assert_eq!(corner(cx), closed_corner);
    assert!(main_open.right() < main.right());
    // Open, the tools start at the panel's left edge as it is resized;
    // the toggle stays at the window's right edge.
    for width in [
        crate::ui::panel::WIDTH_MIN,
        420.,
        crate::ui::panel::WIDTH_MAX,
    ] {
        window
            .update(cx, |shell, _, cx| {
                shell.workspace.right_panel_width = width;
                cx.notify();
            })
            .unwrap();
        let (_, main, tabs) = frames(cx);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        let explorer = visual.debug_bounds("right-panel-explorer").unwrap();
        assert!(visual.debug_bounds("right-panel-changes").is_none());
        let history = visual.debug_bounds("right-panel-history").unwrap();
        assert_eq!(explorer.left(), main.right() + px(6.));
        assert_eq!(explorer.center().y, closed_corner.center().y);
        assert_eq!(history.top(), explorer.top());
        assert!(explorer.right() <= history.left());
        assert!(history.right() <= closed_corner.left());
        assert!(tabs.right() <= explorer.left());
        assert_eq!(corner(cx), closed_corner);
    }
    let history = gpui::VisualTestContext::from_window(window.into(), cx)
        .debug_bounds("right-panel-history")
        .expect("the panel's pane buttons show beside the toggle");
    assert!(history.right() <= closed_corner.left());
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_click(history.center(), Default::default());
    visual.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            let panel = shell.right_panel.clone().unwrap();
            assert_eq!(panel.read(cx).tab(), crate::ui::panel::RightTab::Graph);
            assert_eq!(
                shell.workspace.right_panel_tab,
                crate::ui::panel::RightTab::Graph
            );
        })
        .unwrap();
    window
        .update(cx, |shell, w, cx| {
            shell.toggle_right_panel(w, cx);
            cx.set_reduce_motion(false);
        })
        .unwrap();
    frames(cx);
    assert!(
        gpui::VisualTestContext::from_window(window.into(), cx)
            .debug_bounds("right-panel-history")
            .is_none(),
        "closed, the corner holds only the toggle"
    );
    // The visit arrows stay at the open sidebar's edge, not in the strip.
    let forward = gpui::VisualTestContext::from_window(window.into(), cx)
        .debug_bounds("visit-forward")
        .unwrap();
    assert!(forward.right() <= sidebar.right());
    assert!(forward.right() > sidebar.right() - px(16.));
    // The sidebar toggle holds still as the sidebar closes and reopens.
    let toggle = |cx: &mut TestAppContext| {
        gpui::VisualTestContext::from_window(window.into(), cx)
            .debug_bounds("toggle-sidebar")
            .unwrap()
    };
    let open = toggle(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    for visible in [false, true] {
        window
            .update(cx, |shell, w, cx| {
                shell.toggle_sidebar(w, cx);
                assert_eq!(shell.workspace.sidebar_visible, visible);
            })
            .unwrap();
        let (_, main, _) = frames(cx);
        assert_eq!(main.left() == px(0.), !visible);
        assert_eq!(toggle(cx), open);
    }
    cx.update(|cx| cx.set_reduce_motion(false));
    window
        .update(cx, |shell, _, cx| {
            assert!(shell.workspace.new_layout_boxy);
            let saved = serde_json::to_string(&shell.workspace).unwrap();
            assert!(Workspace::restore(Some(&saved), &[], "/work/monocode").new_layout_boxy);
            shell.workspace.new_layout_boxy = !shell.workspace.new_layout_boxy;
            shell.layout_changed(cx);
            assert!(!shell.workspace.new_layout_boxy);
        })
        .unwrap();
}

#[gpui::test]
fn toggling_the_sidebar_after_its_cached_frames_does_not_replay_stale_paint(
    cx: &mut TestAppContext,
) {
    let (window, a, b) = window(cx);
    let draw = |cx: &mut TestAppContext| {
        cx.update_window(window.into(), |_, w, cx| {
            w.simulate_next_frame(cx);
            w.draw(cx).clear(cx);
        })
        .unwrap();
    };
    // The sidebar's search records its paint behind a long tab strip.
    window
        .update(cx, |shell, _, cx| {
            for _ in 0..30 {
                shell.workspace.tabs.extend([a, b]);
            }
            shell.invalidate_chrome(cx);
            cx.notify();
        })
        .unwrap();
    draw(cx);
    // The strip shrinks while the sidebar replays its cached frame.
    window
        .update(cx, |shell, _, cx| {
            shell.workspace.tabs.truncate(2);
            shell.tabs_frame.update(cx, |_, cx| cx.notify());
            cx.notify();
        })
        .unwrap();
    draw(cx);
    // Sliding paints the sidebar uncached; its search must not replay
    // the indices it recorded two frames ago.
    window
        .update(cx, |shell, w, cx| shell.toggle_sidebar(w, cx))
        .unwrap();
    draw(cx);
    draw(cx);
}

#[gpui::test]
fn main_pane_covers_a_fixed_sidebar_with_a_clipped_fade(cx: &mut TestAppContext) {
    use std::time::Duration;
    let (window, _, active) = window(cx);
    window
        .update(cx, |shell, _, cx| {
            shell.sessions[&active].update(cx, |session, _| {
                session.summary.has_messages = true;
            });
            shell.invalidate_chrome(cx);
            cx.notify();
        })
        .unwrap();
    cx.update_window(window.into(), |_, w, cx| {
        w.simulate_next_frame(cx);
        w.draw(cx).clear(cx);
    })
    .unwrap();
    let row = window
        .update(cx, |shell, w, _| {
            w.painted_quads()
                .into_iter()
                .find(|q| {
                    q.background == content(0.1).into()
                        && q.bounds.size.height.0 == 32. * w.scale_factor()
                        && q.bounds.size.width.0
                            > shell.workspace.sidebar_width / 2. * w.scale_factor()
                        && q.bounds.right().0 < shell.workspace.sidebar_width * w.scale_factor()
                })
                .expect("selected session has a painted background")
                .bounds
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    let toggle = visual.debug_bounds("toggle-sidebar").unwrap();
    for visible in [false, true] {
        window
            .update(visual, |shell, w, cx| {
                shell.toggle_sidebar(w, cx);
                assert_eq!(shell.workspace.sidebar_visible, visible);
            })
            .unwrap();
        let mut previous = if visible { 0. } else { 1. };
        for ms in [30, 65, 110, 170] {
            window
                .update(visual, |shell, _, cx| {
                    shell
                        .sidebar_slide
                        .as_ref()
                        .unwrap()
                        .freeze_elapsed(Duration::from_millis(ms));
                    cx.notify();
                })
                .unwrap();
            visual
                .update_window(window.into(), |_, w, cx| {
                    w.simulate_next_frame(cx);
                    w.draw(cx).clear(cx);
                })
                .unwrap();
            let clip = visual.debug_bounds("sidebar-frame").unwrap();
            let content = visual.debug_bounds("sidebar-content").unwrap();
            let main = visual.debug_bounds("main-pane-frame").unwrap();
            assert_eq!(
                visual.debug_bounds("toggle-sidebar").unwrap().origin,
                toggle.origin
            );
            assert_eq!(content.left(), px(0.));
            assert!((clip.right() - main.left()).abs() < px(1.));
            window
                .update(visual, |shell, w, _| {
                    let fraction = shell.sidebar_fraction;
                    assert!(if visible {
                        fraction > previous
                    } else {
                        fraction < previous
                    });
                    previous = fraction;
                    assert_eq!(content.size.width, px(shell.workspace.sidebar_width));
                    let quad = w.painted_quads().into_iter().find(|q| q.bounds == row);
                    if clip.right().as_f32() * w.scale_factor() <= row.left().0 {
                        assert!(quad.is_none(), "fully covered rows are culled");
                        return;
                    }
                    let quad = quad.expect("exposed sidebar row paints at its full size");
                    let alpha = quad.background.as_solid().unwrap().a;
                    assert!(
                        (alpha - 0.1 * fraction).abs() < 0.0001,
                        "painted alpha {alpha} follows {fraction}"
                    );
                    assert!(
                        (quad.content_mask.bounds.right().0
                            - main.left().as_f32() * w.scale_factor())
                        .abs()
                            < w.scale_factor()
                    );
                })
                .unwrap();
        }
        window
            .update(visual, |shell, _, cx| {
                shell
                    .sidebar_slide
                    .as_ref()
                    .unwrap()
                    .elapsed(Duration::from_secs(1));
                cx.notify();
            })
            .unwrap();
        visual
            .update_window(window.into(), |_, w, cx| {
                w.simulate_next_frame(cx);
                w.draw(cx).clear(cx);
            })
            .unwrap();
        window
            .update(visual, |shell, _, _| {
                assert!(shell.sidebar_slide.is_none());
                assert_eq!(shell.sidebar_fraction, if visible { 1. } else { 0. });
            })
            .unwrap();
    }
    // Reopening the sidebar reverses its closing motion without jumping its edge.
    window
        .update(visual, |shell, w, cx| {
            shell.toggle_sidebar(w, cx);
            shell
                .sidebar_slide
                .as_ref()
                .unwrap()
                .elapsed(Duration::from_millis(65));
            let before = shell
                .sidebar_slide
                .as_ref()
                .unwrap()
                .at(std::time::Instant::now())
                .0;
            shell.toggle_sidebar(w, cx);
            assert!(shell.workspace.sidebar_visible);
            let after = shell
                .sidebar_slide
                .as_ref()
                .unwrap()
                .at(std::time::Instant::now())
                .0;
            assert!((after - before).abs() < 0.01);
        })
        .unwrap();
    visual.update(|_, cx| cx.set_reduce_motion(true));
    visual
        .update_window(window.into(), |_, w, cx| {
            w.simulate_next_frame(cx);
            w.draw(cx).clear(cx);
        })
        .unwrap();
    window
        .update(visual, |shell, _, _| {
            assert!(shell.sidebar_slide.is_none());
            assert_eq!(shell.sidebar_fraction, 1.);
        })
        .unwrap();
}
