use super::*;
use crate::{app::shell::tests::window, monos::Monos};
use gpui::{TestAppContext, WindowHandle};
use std::time::{Duration, Instant};

fn mixed_window(cx: &mut TestAppContext) -> (WindowHandle<Shell>, Entity<Monos>, String, i64) {
    let (window, _, session) = window(cx);
    let (manager, id) = window
        .update(cx, |shell, w, cx| {
            let manager = shell.mono_manager();
            manager.update(cx, |m, cx| {
                m.create(Vec::new(), w, cx);
                m.dismiss_settings(w, cx);
            });
            let id = manager.read(cx).active.clone().unwrap();
            (manager, id)
        })
        .unwrap();
    cx.run_until_parked();
    (window, manager, id, session)
}

#[gpui::test]
fn project_tabs_abbreviate_then_become_icons_without_losing_header_controls(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, _, _) = window(cx);
    window
        .update(cx, |shell, w, cx| {
            shell.toggle_right_panel(w, cx);
            shell.git.update(cx, |git, cx| {
                git.stats = Some(monocode_core::vcs::git::scm::DiffStats {
                    files: 550,
                    additions: 246_137,
                    deletions: 11_081,
                });
                cx.notify();
            });
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    for boxy in [false, true] {
        let selectors = if boxy {
            ["right-panel-explorer", "right-panel-history"]
        } else {
            ["right-panel-tab-Explorer", "right-panel-tab-History"]
        };
        let label_selector = Box::leak(format!("{}-label", selectors[1]).into_boxed_str());
        let mut full_label_width = None;
        for width in [420., 290., 270., 260., 220., 420.] {
            window
                .update(visual, |shell, _, cx| {
                    shell.workspace.new_layout_boxy = boxy;
                    shell.workspace.right_panel_width = width;
                    shell
                        .right_panel
                        .as_ref()
                        .unwrap()
                        .update(cx, |panel, cx| panel.select(RightTab::Graph, cx));
                    shell.invalidate_chrome(cx);
                    cx.notify();
                })
                .unwrap();
            visual.run_until_parked();
            visual.refresh().unwrap();
            let tabs = selectors.map(|selector| visual.debug_bounds(selector).unwrap());
            let stats = visual.debug_bounds("project-diff-stat").unwrap();
            let toggle = visual.debug_bounds("toggle-right-panel").unwrap();
            let search = visual.debug_bounds("titlebar-file-search").unwrap();
            assert!(tabs[0].right() <= tabs[1].left());
            assert!(tabs[1].right() <= stats.left());
            assert!(stats.right() <= search.left());
            assert!(search.right() <= toggle.left());
            assert_eq!(search.left() - stats.right(), px(CORNER_CONTROL_GAP));
            assert_eq!(toggle.left() - search.right(), px(CORNER_CONTROL_GAP));
            assert_eq!(toggle.size.width, px(24.));
            if width == 420. {
                let label = visual.debug_bounds(label_selector).unwrap();
                if let Some(full_width) = full_label_width {
                    assert_eq!(label.size.width, full_width);
                }
                full_label_width = Some(label.size.width);
            } else if width == 290. {
                let label = visual.debug_bounds(label_selector).unwrap();
                assert!(label.size.width > px(0.));
                assert!(label.size.width < full_label_width.unwrap());
            } else {
                assert!(visual.debug_bounds(label_selector).is_none());
                assert!(tabs.iter().all(|tab| tab.size.width == px(26.)));
                // The icon keeps its action after the label disappears.
                visual.simulate_click(tabs[0].center(), Default::default());
                visual.run_until_parked();
                window
                    .update(visual, |shell, _, cx| {
                        assert_eq!(
                            shell.right_panel.as_ref().unwrap().read(cx).tab(),
                            RightTab::Files
                        )
                    })
                    .unwrap();
            }
        }
    }
}

#[gpui::test]
fn right_sidebar_width_and_visibility_follow_both_tab_types_and_layouts(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, manager, mono, session) = mixed_window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    for classic in [false, true] {
        window
            .update(visual, |shell, _, cx| {
                shell.workspace.classic_layout = classic;
                assert!(
                    !shell.workspace.right_panel_open,
                    "opening a Mono respects the session's closed sidebar"
                );
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("right-panel-frame").is_none());
        let toggle = visual
            .debug_bounds(if classic {
                "classic-mono-overview"
            } else {
                "toggle-right-panel"
            })
            .unwrap();
        visual.simulate_click(toggle.center(), Default::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds("mono-overview-panel").is_some());
        let frame = visual.debug_bounds("right-panel-frame").unwrap();
        let bounds = visual.debug_bounds("right-panel-resize").unwrap();
        assert_eq!(bounds.size.width, px(6.));
        assert!((bounds.center().x - frame.left() - px(0.5)).abs() <= px(0.5));
        // Both halves of the transparent grab area must receive presses,
        // including the half outside the panel's clipped contents.
        for x in [bounds.left() + px(0.5), bounds.right() - px(0.5)] {
            let point = gpui::point(x, bounds.center().y);
            visual.simulate_mouse_down(point, MouseButton::Left, Default::default());
            window
                .update(visual, |shell, _, _| assert!(shell.right_resize.is_some()))
                .unwrap();
            visual.simulate_mouse_up(point, MouseButton::Left, Default::default());
            visual.run_until_parked();
        }
        let handle = visual.debug_bounds("right-panel-resize").unwrap().center();
        let end = handle - gpui::point(px(30.), px(0.));
        visual.simulate_mouse_down(handle, MouseButton::Left, Default::default());
        visual.simulate_mouse_move(end, MouseButton::Left, Default::default());
        visual.simulate_mouse_up(end, MouseButton::Left, Default::default());
        visual.run_until_parked();
        let expanded = window
            .update(visual, |shell, _, _| shell.workspace.right_panel_width)
            .unwrap();
        let handle = visual.debug_bounds("right-panel-resize").unwrap().center();
        let end = handle + gpui::point(px(30.), px(0.));
        visual.simulate_mouse_down(handle, MouseButton::Left, Default::default());
        visual.simulate_mouse_move(end, MouseButton::Left, Default::default());
        // The pointer is now over a Mono detail layer that occludes mouse
        // events. Shrinking and releasing must still reach the shell.
        window
            .update(visual, |shell, _, _| {
                assert_eq!(shell.workspace.right_panel_width, expanded - 30.);
            })
            .unwrap();
        visual.simulate_mouse_up(
            end + gpui::point(px(20.), px(0.)),
            MouseButton::Left,
            Default::default(),
        );
        visual.run_until_parked();
        let mono_bounds = visual.debug_bounds("right-panel-frame").unwrap();
        let width = window
            .update(visual, |shell, w, cx| {
                assert!(shell.right_resize.is_none());
                let width = shell.workspace.right_panel_width;
                assert_eq!(mono_bounds.size.width, px(width));
                shell.open(session, w, cx);
                width
            })
            .unwrap();
        visual.run_until_parked();
        if classic {
            assert!(visual.debug_bounds("right-panel-frame").is_none());
        } else {
            assert_eq!(
                visual.debug_bounds("right-panel-frame").unwrap(),
                mono_bounds
            );
        }
        assert!(visual.debug_bounds("mono-overview-panel").is_none());
        if !classic {
            let toggle = visual.debug_bounds("toggle-right-panel").unwrap();
            visual.simulate_click(toggle.center(), Default::default());
            visual.run_until_parked();
        }
        window
            .update(visual, |_, w, cx| {
                manager.update(cx, |m, cx| m.open(&mono, w, cx))
            })
            .unwrap();
        visual.run_until_parked();
        if classic {
            let toggle = visual.debug_bounds("classic-mono-overview").unwrap();
            visual.simulate_click(toggle.center(), Default::default());
            visual.run_until_parked();
        }
        assert!(visual.debug_bounds("right-panel-frame").is_none());
        window
            .update(visual, |shell, _, _| {
                assert_eq!(shell.workspace.right_panel_width, width);
                let saved = crate::runtime::locked(&shell.store)
                    .unwrap()
                    .ui_state()
                    .unwrap()
                    .unwrap();
                let saved: crate::app::workspace::Workspace = serde_json::from_str(&saved).unwrap();
                assert_eq!(saved.right_panel_width, width);
                assert!(!saved.right_panel_open);
            })
            .unwrap();
    }
}

#[gpui::test]
fn switching_tabs_keeps_the_same_sidebar_spring_and_clipped_contents(cx: &mut TestAppContext) {
    let (window, manager, mono, session) = mixed_window(cx);
    cx.update(|cx| cx.set_reduce_motion(false));
    window
        .update(cx, |shell, w, cx| shell.toggle_right_panel(w, cx))
        .unwrap();
    for ms in [35, 70, 110] {
        window
            .update(cx, |shell, w, cx| {
                let slide = shell.right_slide.as_ref().unwrap();
                slide.elapsed(Duration::from_millis(ms));
                let now = Instant::now();
                let before = slide.at(now);
                if shell.mono_active(cx) {
                    shell.open(session, w, cx);
                } else {
                    manager.update(cx, |m, cx| m.open(&mono, w, cx));
                }
                assert_eq!(
                    shell.right_slide.as_ref().unwrap().at(now),
                    before,
                    "tab switches preserve the spring's position and velocity"
                );
                assert!(shell.workspace.right_panel_open);
                cx.notify();
            })
            .unwrap();
        cx.update_window(window.into(), |_, w, cx| {
            w.simulate_next_frame(cx);
            w.draw(cx).clear(cx);
        })
        .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        let frame = visual.debug_bounds("right-panel-frame").unwrap();
        let content = visual.debug_bounds("right-panel-content").unwrap();
        let width = window
            .update(visual, |shell, _, _| shell.workspace.right_panel_width)
            .unwrap();
        assert!(frame.size.width > px(0.) && frame.size.width < px(width));
        assert_eq!(content.size.width, px(width - 1.));
    }
    window
        .update(cx, |shell, w, cx| {
            shell.toggle_right_panel(w, cx);
            assert!(!shell.workspace.right_panel_open);
            assert_eq!(shell.right_slide.as_ref().unwrap().to, 0.);
            shell
                .right_slide
                .as_ref()
                .unwrap()
                .elapsed(Duration::from_millis(45));
            shell.open(session, w, cx);
            shell.toggle_right_panel(w, cx);
            assert!(shell.workspace.right_panel_open);
            assert_eq!(
                shell.right_slide.as_ref().unwrap().to,
                shell.workspace.right_panel_width
            );
            manager.update(cx, |m, cx| m.open(&mono, w, cx));
            shell
                .right_slide
                .as_ref()
                .unwrap()
                .elapsed(Duration::from_secs(1));
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| assert!(shell.right_slide.is_none()))
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    assert_eq!(
        visual.debug_bounds("right-panel-frame").unwrap().size.width,
        px(window
            .update(visual, |shell, _, _| shell.workspace.right_panel_width)
            .unwrap())
    );
    assert!(visual.debug_bounds("mono-overview-panel").is_some());
}
