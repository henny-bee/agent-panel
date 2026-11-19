use super::*;
use gpui::{TestAppContext, VisualTestContext, WindowHandle, point, size};

fn original_window(cx: &mut TestAppContext) -> WindowHandle<Shell> {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = crate::app::shell::tests::window(cx);
    window
        .update(cx, |shell, _, cx| {
            shell.workspace.classic_layout = true;
            shell.workspace.project_rail_open = true;
            shell.classic_sidebar_tab = ClassicSidebarTab::Sessions;
            shell.layout_changed(cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
}

fn tab_has_fill(
    window: WindowHandle<Shell>,
    bounds: Bounds<Pixels>,
    alpha: f32,
    cx: &mut VisualTestContext,
) -> bool {
    window
        .update(cx, |_, window, _| {
            let scale = window.scale_factor();
            window.painted_quads().iter().any(|quad| {
                (quad.bounds.left().0 - bounds.left().as_f32() * scale).abs() < 0.5
                    && (quad.bounds.top().0 - bounds.top().as_f32() * scale).abs() < 0.5
                    && (quad.bounds.size.width.0 - bounds.size.width.as_f32() * scale).abs() < 0.5
                    && (quad.bounds.size.height.0 - bounds.size.height.as_f32() * scale).abs() < 0.5
                    && quad
                        .background
                        .as_solid()
                        .is_some_and(|color| (color.a - alpha).abs() < 0.001)
            })
        })
        .unwrap()
}

#[gpui::test]
fn original_workspace_tabs_hover_without_replacing_the_selected_fill(cx: &mut TestAppContext) {
    let window = original_window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(size(px(1500.), px(900.)));
    visual.simulate_mouse_move(
        point(px(1200.), px(500.)),
        None::<MouseButton>,
        Default::default(),
    );
    visual.run_until_parked();
    let sessions = visual
        .debug_bounds("classic-workspace-tab-sessions")
        .unwrap();
    let files = visual.debug_bounds("classic-workspace-tab-files").unwrap();
    assert!(tab_has_fill(window, sessions, 0.1, visual));
    assert!(!tab_has_fill(window, files, 0.05, visual));

    visual.simulate_mouse_move(files.center(), None::<MouseButton>, Default::default());
    visual.run_until_parked();
    assert!(
        tab_has_fill(window, files, 0.05, visual),
        "an inactive workspace tab gains the Tauri hover wash"
    );
    assert!(
        tab_has_fill(window, sessions, 0.1, visual),
        "hovering another tab keeps the current selection"
    );

    visual.simulate_mouse_move(sessions.center(), None::<MouseButton>, Default::default());
    visual.run_until_parked();
    assert!(tab_has_fill(window, sessions, 0.1, visual));
    assert!(
        !tab_has_fill(window, files, 0.05, visual),
        "the hover wash follows the pointer"
    );
    assert!(
        !tab_has_fill(window, sessions, 0.05, visual),
        "the selected tab keeps its stronger wash on hover"
    );
}

#[gpui::test]
fn original_workspace_changes_counts_fit_completely_after_sidebar_resize(cx: &mut TestAppContext) {
    let window = original_window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(size(px(1500.), px(900.)));
    for (width, additions, deletions) in [
        (220., 5_476, 408),
        (260., 5_476, 408),
        (360., 5_476, 408),
        (220., 99_999_999, 99_999_999),
        (220., 5_476, 0),
        (220., 0, 408),
    ] {
        window
            .update(visual, |shell, _, cx| {
                shell.workspace.sidebar_width = width;
                shell.git.update(cx, |git, cx| {
                    git.stats = Some(monocode_core::vcs::git::scm::DiffStats {
                        files: 1,
                        additions,
                        deletions,
                    });
                    cx.notify();
                });
                shell.notify_sidebar(cx);
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        let tab = visual
            .debug_bounds("classic-workspace-tab-changes")
            .unwrap();
        let stat = visual.debug_bounds("classic-changes-stat").unwrap();
        assert!(stat.left() >= tab.left() + px(8.));
        assert!(
            stat.right() <= tab.right() - px(8.),
            "the complete diff count stays inside its padded tab at {width}pt"
        );
        let added = visual.debug_bounds("classic-changes-added");
        let deleted = visual.debug_bounds("classic-changes-deleted");
        assert_eq!(added.is_some(), additions > 0);
        assert_eq!(deleted.is_some(), deletions > 0);
        for (name, count) in [("added", added), ("deleted", deleted)]
            .into_iter()
            .filter_map(|(name, count)| count.map(|count| (name, count)))
        {
            assert!(count.left() >= stat.left() - px(0.25));
            assert!(
                count.right() <= stat.right() + px(0.25),
                "both full glyph runs fit, including every deletion digit: sidebar={width}, counts=+{additions}/-{deletions}, run={name}, run_bounds={count:?}, stat_bounds={stat:?}, tab_bounds={tab:?}"
            );
        }
        if let (Some(added), Some(deleted)) = (added, deleted) {
            assert!(
                added.right() < deleted.left(),
                "the separating gap scales along with both numbers"
            );
        }
    }
}
