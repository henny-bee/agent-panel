use super::*;

#[gpui::test]
fn the_new_layout_can_host_the_terminal_as_a_right_panel_tab(cx: &mut gpui::TestAppContext) {
    use crate::ui::panel::RightTab;
    let (window, _, _) = crate::app::shell::tests::window(cx);
    window
        .update(cx, |shell, window, cx| {
            cx.set_reduce_motion(true);
            shell.workspace.classic_layout = false;
            shell.workspace.new_layout_boxy = true;
            shell.layout_changed(cx);
            shell.workspace.preferences.set_terminal_placement("panel");
            shell.toggle_terminal(window, cx);
            let project = shell.workspace.project.clone();
            assert!(shell.workspace.right_panel_open);
            assert_eq!(shell.terminals[&project].len(), 1);
            let panel = shell.right_panel.clone().unwrap();
            assert_eq!(panel.read(cx).tab(), RightTab::Terminal);
            assert_eq!(
                panel.read(cx).tabs(),
                [RightTab::Files, RightTab::Terminal, RightTab::Graph]
            );
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    // In the boxy frame the terminal starts below the title bar's tabs.
    let tabs = visual.debug_bounds("right-panel-terminal").unwrap();
    let header = visual.debug_bounds("terminal-tab-0").unwrap();
    assert!(header.top() >= tabs.bottom(), "{header:?} under {tabs:?}");
    assert!(visual.debug_bounds("terminal-dock").is_none());
    assert!(visual.debug_bounds("footer-terminal").is_none());
    window
        .update(cx, |shell, window, cx| {
            // Closing the last tab hides the panel and keeps the shell.
            let project = shell.workspace.project.clone();
            let terminal = shell.terminals[&project][0].clone();
            shell.close_terminal(0, window, cx);
            assert!(!shell.workspace.right_panel_open);
            assert_eq!(shell.terminals[&project], [terminal]);
            assert_eq!(shell.workspace.docks[&project].terminals.len(), 1);
            // The footer button closes the panel it opened.
            shell.toggle_terminal(window, cx);
            assert!(shell.workspace.right_panel_open);
            shell.toggle_terminal(window, cx);
            assert!(!shell.workspace.right_panel_open);
            // Turned off, the tab goes and the dock is back.
            shell.workspace.preferences.set_terminal_placement("bottom");
            shell.apply_preferences(cx);
            let panel = shell.right_panel.clone().unwrap();
            assert_eq!(panel.read(cx).tab(), RightTab::Files);
            assert!(!panel.read(cx).tabs().contains(&RightTab::Terminal));
            shell.toggle_terminal(window, cx);
            assert!(shell.workspace.docks[&shell.workspace.project].open);
            shell.toggle_terminal(window, cx);
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    assert!(visual.debug_bounds("footer-terminal").is_some());
}

#[test]
fn chip_labels_count_repeated_processes_in_order() {
    assert_eq!(chip_label(["vite"]), "vite");
    assert_eq!(chip_label(["vite", "jest"]), "vite · jest");
    assert_eq!(chip_label(["vite", "jest", "vite"]), "vite ×2 · jest");
}
