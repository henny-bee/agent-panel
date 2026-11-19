use super::*;

#[gpui::test]
fn sidebar_shortcut_keeps_working_after_hiding_focused_search(cx: &mut TestAppContext) {
    let (window, _, _) = window(cx);
    cx.update(|cx| {
        input::bind(cx);
        bind_keys(cx);
    });
    window
        .update(cx, |shell, w, cx| shell.focus_search(w, cx))
        .unwrap();
    cx.refresh().unwrap();
    let m = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    for visible in [false, true, false, true] {
        cx.simulate_keystrokes(window.into(), &format!("{m}-b"));
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                assert_eq!(shell.workspace.sidebar_visible, visible)
            })
            .unwrap();
    }
}

#[gpui::test]
fn right_panel_shortcut_keeps_working_after_hiding_focused_history(cx: &mut TestAppContext) {
    let (window, _, _) = window(cx);
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        bind_keys(cx);
    });
    window
        .update(cx, |shell, w, cx| {
            shell.toggle_right_panel(w, cx);
            let panel = shell.right_panel.clone().unwrap();
            panel.update(cx, |p, cx| p.select(crate::ui::panel::RightTab::Graph, cx));
            let history = panel.read(cx).history.clone();
            w.focus(&history.focus_handle(cx), cx);
        })
        .unwrap();
    cx.refresh().unwrap();
    let m = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    for open in [false, true, false, true] {
        cx.simulate_keystrokes(window.into(), &format!("{m}-shift-b"));
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                assert_eq!(shell.workspace.right_panel_open, open)
            })
            .unwrap();
    }
}

#[gpui::test]
fn layout_shortcut_toggles_from_the_focused_composer(cx: &mut TestAppContext) {
    let (window, _, _) = window(cx);
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        bind_keys(cx);
        crate::ui::input::bind(cx);
    });
    window
        .update(cx, |shell, w, cx| shell.focus_active(w, cx))
        .unwrap();
    cx.refresh().unwrap();
    let m = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    for classic in [true, false] {
        cx.simulate_keystrokes(window.into(), &format!("{m}-shift-l"));
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| {
                assert_eq!(shell.workspace.classic_layout, classic);
                assert!(!shell.workspace.right_panel_open);
            })
            .unwrap();
    }
}

#[gpui::test]
fn title_tabs_draw_their_own_session_marks(cx: &mut TestAppContext) {
    let (window, a, _) = window(cx);
    window
        .update(cx, |shell, _, cx| {
            shell.sessions[&a].update(cx, |session, cx| {
                session.busy = true;
                cx.emit(SessionChanged);
            });
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            // One entity drawn in both the sidebar and the cached tab strip
            // only repaints along one path, freezing the tab's spinner.
            assert_ne!(
                shell.tab_marks[&a].entity_id(),
                shell.session_marks[&a].entity_id()
            );
            assert!(shell.tab_marks[&a].read(cx).state() == MarkState::Working);
        })
        .unwrap();
}

#[gpui::test]
fn switching_layout_with_working_session_marks_keeps_cached_paint_valid(cx: &mut TestAppContext) {
    use std::time::Duration;
    let (window, a, b) = window(cx);
    cx.update(|cx| {
        bind_keys(cx);
        crate::ui::input::bind(cx);
    });
    window
        .update(cx, |shell, w, cx| {
            for id in [a, b] {
                shell.sessions[&id].update(cx, |session, cx| {
                    session.summary.cwd = "/work/monocode".into();
                    session.summary.has_messages = true;
                    cx.emit(SessionChanged);
                });
            }
            shell.sessions[&a].update(cx, |session, cx| {
                session.busy = true;
                cx.emit(SessionChanged);
            });
            shell.focus_active(w, cx);
            shell.toggle_right_panel(w, cx);
            for _ in 0..30 {
                shell.workspace.tabs.extend([a, b]);
            }
            shell.invalidate_chrome(cx);
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    let draw = |cx: &mut TestAppContext| {
        cx.update_window(window.into(), |_, w, cx| {
            w.simulate_next_frame(cx);
            w.draw(cx).clear(cx);
        })
        .unwrap();
    };
    draw(cx);
    // Reusing the sidebar after an earlier sibling shrinks relocates its
    // text cache. A subsequent uncached slide must not replay old indices.
    window
        .update(cx, |shell, _, cx| {
            shell.workspace.tabs.truncate(2);
            shell.tabs_frame.update(cx, |_, cx| cx.notify());
            cx.notify();
        })
        .unwrap();
    draw(cx);
    window
        .update(cx, |shell, w, cx| shell.toggle_sidebar(w, cx))
        .unwrap();
    draw(cx);
    for step in 0..20 {
        for _ in 0..5 {
            cx.executor().advance_clock(Duration::from_millis(20));
            cx.run_until_parked();
            cx.update_window(window.into(), |_, w, cx| {
                w.simulate_next_frame(cx);
                w.draw(cx).clear(cx);
            })
            .unwrap();
        }
        window
            .update(cx, |shell, w, cx| {
                if step % 3 == 0 {
                    shell.toggle_sidebar(w, cx);
                }
                shell.toggle_layout(cx);
            })
            .unwrap();
        cx.run_until_parked();
    }
}

#[gpui::test]
fn titlebar_panel_buttons_keep_window_bounds_for_single_and_double_clicks(cx: &mut TestAppContext) {
    let (window, _, _) = window(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    window
        .update(cx, |shell, w, cx| shell.focus_active(w, cx))
        .unwrap();
    let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    let original = window.update(cx, |_, w, _| w.window_bounds()).unwrap();
    for selector in ["toggle-sidebar", "toggle-right-panel"] {
        for click_count in [1, 2, 1, 2] {
            let position = cx.debug_bounds(selector).unwrap().center();
            cx.simulate_mouse_move(position, None, Default::default());
            cx.run_until_parked();
            let before = window
                .update(cx, |shell, _, _| {
                    (
                        shell.workspace.sidebar_visible,
                        shell.workspace.right_panel_open,
                    )
                })
                .unwrap();
            cx.simulate_event(MouseDownEvent {
                position,
                button: MouseButton::Left,
                click_count,
                first_mouse: false,
                modifiers: Default::default(),
            });
            cx.simulate_event(gpui::MouseUpEvent {
                position,
                button: MouseButton::Left,
                click_count,
                modifiers: Default::default(),
            });
            cx.run_until_parked();
            window
                .update(cx, |shell, w, _| {
                    assert_eq!(w.window_bounds(), original);
                    if selector == "toggle-sidebar" {
                        assert_eq!(shell.workspace.sidebar_visible, !before.0);
                    } else {
                        assert_eq!(shell.workspace.right_panel_open, !before.1);
                    }
                })
                .unwrap();
        }
    }
}

#[gpui::test]
fn panel_tab_switches_and_animated_reversals_keep_shortcuts_live(cx: &mut TestAppContext) {
    let (window, _, _) = window(cx);
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        bind_keys(cx);
    });
    window
        .update(cx, |shell, w, cx| {
            shell.focus_active(w, cx);
            shell.toggle_right_panel(w, cx);
            shell.open_review(
                crate::git::review::ReviewTab::File {
                    cwd: "/work/monocode".into(),
                    path: "README.md".into(),
                },
                w,
                cx,
            );
        })
        .unwrap();
    let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    cx.run_until_parked();
    for width in [
        crate::ui::panel::WIDTH_MIN,
        420.,
        crate::ui::panel::WIDTH_DEFAULT,
    ] {
        window
            .update(cx, |shell, _, cx| {
                shell.workspace.right_panel_width = width;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            cx.debug_bounds("project-tabs-separator").unwrap().left(),
            cx.debug_bounds("right-panel-frame").unwrap().left(),
            "the title-bar rule follows the panel's divider"
        );
    }
    let handle = cx.debug_bounds("titlebar-right-panel-resize").unwrap();
    let start = handle.center();
    let end = start - gpui::point(px(40.), px(0.));
    cx.simulate_event(MouseDownEvent {
        position: start,
        button: MouseButton::Left,
        click_count: 1,
        first_mouse: false,
        modifiers: Default::default(),
    });
    cx.simulate_mouse_move(end, Some(MouseButton::Left), Default::default());
    cx.simulate_event(gpui::MouseUpEvent {
        position: end,
        button: MouseButton::Left,
        click_count: 1,
        modifiers: Default::default(),
    });
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| {
            assert_eq!(
                shell.workspace.right_panel_width,
                crate::ui::panel::WIDTH_DEFAULT + 40.
            );
            assert!(shell.right_resize.is_none());
        })
        .unwrap();
    assert_eq!(
        cx.debug_bounds("project-tabs-separator").unwrap().left(),
        cx.debug_bounds("right-panel-frame").unwrap().left()
    );
    for (selector, header) in [
        ("right-panel-tab-History", "history-toolbar"),
        ("project-diff-stat", "changes-header"),
        ("right-panel-tab-Explorer", "explorer-header"),
    ] {
        let tab = cx.debug_bounds(selector).unwrap();
        cx.simulate_click(tab.center(), Default::default());
        cx.run_until_parked();
        assert_eq!(
            cx.debug_bounds(header).unwrap().bottom(),
            cx.debug_bounds("review-tab-strip").unwrap().bottom(),
            "tool headers align with the open-file tabs"
        );
        window
            .update(cx, |shell, w, cx| {
                assert!(
                    shell
                        .right_panel
                        .as_ref()
                        .unwrap()
                        .focus_handle(cx)
                        .contains_focused(w, cx)
                );
            })
            .unwrap();
    }
    cx.update(|_, cx| cx.set_reduce_motion(false));
    let m = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    for open in [false, true, false, true, false] {
        cx.simulate_keystrokes(&format!("{m}-shift-b"));
        cx.run_until_parked();
        window
            .update(cx, |shell, w, cx| {
                assert_eq!(shell.workspace.right_panel_open, open);
                assert!(shell.focus.contains_focused(w, cx));
            })
            .unwrap();
    }
}

#[gpui::test]
fn monos_and_session_title_tabs_share_width_spacing_and_scroll_slots(cx: &mut TestAppContext) {
    let (window, a, b) = same_project_window(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let mono = window
        .update(cx, |shell, w, cx| {
            let manager = shell.mono_manager();
            manager.update(cx, |m, cx| m.create(Vec::new(), w, cx));
            manager.read(cx).active.clone().unwrap()
        })
        .unwrap();
    let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    cx.run_until_parked();
    let mono = cx
        .debug_bounds(Box::leak(format!("mono-title-tab-{mono}").into_boxed_str()))
        .unwrap();
    let first = cx
        .debug_bounds(Box::leak(format!("title-tab-{a}").into_boxed_str()))
        .unwrap();
    let second = cx
        .debug_bounds(Box::leak(format!("title-tab-{b}").into_boxed_str()))
        .unwrap();
    assert_eq!(mono.left() - second.right(), px(2.));
    assert_eq!(second.left() - first.right(), px(2.));
    assert_eq!(mono.size, first.size);
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(shell.visible_tabs(cx).len(), 3);
            assert_eq!(shell.tab_scroll.bounds_for_item(2), Some(mono));
            assert_eq!(shell.tab_scroll.bounds_for_item(0), Some(first));
        })
        .unwrap();
}

#[gpui::test]
fn crowded_tabs_fit_and_hold_the_new_tab_button_still_while_one_opens(cx: &mut TestAppContext) {
    let (window, _, _) = window(cx);
    let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    window
        .update(cx, |shell, _, cx| {
            shell.workspace.preferences.tab_animations = true;
            for _ in 0..12 {
                shell.create(Kind::Codex, cx);
            }
        })
        .unwrap();
    // Tab motion runs on the wall clock, as the window's frames do.
    let settle = |cx: &mut gpui::VisualTestContext| {
        std::thread::sleep(
            crate::ui::tabs::motion::DURATION + std::time::Duration::from_millis(20),
        );
        window
            .update(cx, |shell, _, cx| {
                shell.tabs_frame.update(cx, |_, cx| cx.notify())
            })
            .unwrap();
        cx.run_until_parked();
    };
    // Once to start the motions a render notices, once to finish them.
    settle(cx);
    settle(cx);
    let strip = cx.debug_bounds("title-tab-strip").unwrap();
    let first_control = cx
        .debug_bounds("project-diff-stat")
        .or_else(|| cx.debug_bounds("titlebar-file-search"))
        .unwrap();
    // Keep the strip's 6px gap before the trailing controls.
    assert_eq!(strip.right(), first_control.left() - px(6.));
    let settled = cx.debug_bounds("new-tab").unwrap();
    assert!(settled.right() <= strip.right(), "every tab fits");
    let last = window
        .update(cx, |shell, _, cx| {
            shell.visible_tabs(cx).last().and_then(|t| t.session())
        })
        .unwrap()
        .unwrap();
    let last = cx
        .debug_bounds(Box::leak(format!("title-tab-{last}").into_boxed_str()))
        .unwrap();
    // The strip's 2px gap and the button's 2px margin, as when it has room.
    assert!(
        settled.left() - last.right() <= px(4.5),
        "a full strip keeps the usual gap before the new-tab button: {:?}",
        settled.left() - last.right()
    );
    let (selected, other) = window
        .update(cx, |shell, _, cx| {
            let tabs = shell.visible_tabs(cx);
            let selected = shell
                .active_title_tab(cx)
                .and_then(|t| t.session())
                .unwrap();
            let other = tabs
                .iter()
                .filter_map(|t| t.session())
                .find(|id| *id != selected);
            (selected, other.unwrap())
        })
        .unwrap();
    let bounds = |cx: &mut gpui::VisualTestContext, id: i64| {
        cx.debug_bounds(Box::leak(format!("title-tab-{id}").into_boxed_str()))
            .unwrap()
    };
    assert_eq!(
        bounds(cx, selected).size.width,
        px(SELECTED_TAB_MIN_WIDTH),
        "the selected tab shrinks only down to its floor"
    );
    assert!(bounds(cx, other).size.width < px(SELECTED_TAB_MIN_WIDTH));
    let close = |cx: &mut gpui::VisualTestContext, id: i64| {
        cx.debug_bounds(Box::leak(format!("close-title-tab-{id}").into_boxed_str()))
    };
    assert!(close(cx, selected).is_some(), "the selected tab can close");
    if bounds(cx, other).size.width < px(TAB_CLOSE_MIN_WIDTH) {
        assert!(close(cx, other).is_none(), "a collapsed tab only selects");
    }
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(shell.tab_scroll.max_offset().x, px(0.), "nothing scrolls");
            shell.create(Kind::Codex, cx);
        })
        .unwrap();
    let mut seen = Vec::new();
    for _ in 0..16 {
        std::thread::sleep(std::time::Duration::from_millis(16));
        window
            .update(cx, |shell, _, cx| {
                shell.tabs_frame.update(cx, |_, cx| cx.notify())
            })
            .unwrap();
        cx.run_until_parked();
        seen.push(cx.debug_bounds("new-tab").unwrap().left());
    }
    assert!(
        seen.iter().all(|x| (*x - settled.left()).abs() < px(0.5)),
        "the new-tab button holds still while a crowded strip makes room: {settled:?} {seen:?}"
    );
    window
        .update(cx, |shell, w, cx| {
            let first = shell.visible_tabs(cx)[0].session().unwrap();
            shell.close(first, w, cx);
        })
        .unwrap();
    let mut closing = Vec::new();
    for _ in 0..16 {
        std::thread::sleep(std::time::Duration::from_millis(16));
        window
            .update(cx, |shell, _, cx| {
                shell.tabs_frame.update(cx, |_, cx| cx.notify())
            })
            .unwrap();
        cx.run_until_parked();
        closing.push(cx.debug_bounds("new-tab").unwrap().left());
    }
    assert!(
        closing
            .iter()
            .all(|x| (*x - settled.left()).abs() < px(0.5)),
        "and while a closed tab collapses: {closing:?}"
    );
}
