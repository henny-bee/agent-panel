use super::*;

#[gpui::test]
fn chat_composer_matches_the_session_box_and_opens_the_model_list(cx: &mut TestAppContext) {
    let s = setup(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let handle: gpui::AnyWindowHandle = s.window.into();
    handle
        .update(cx, |_, w, cx| {
            s.manager.update(cx, |m, cx| m.dismiss_settings(w, cx));
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    let field = visual.debug_bounds("mono-composer").unwrap();
    let send = visual.debug_bounds("mono-send").unwrap();
    let attach = visual.debug_bounds("mono-attach").unwrap();
    let model = visual.debug_bounds("mono-model").unwrap();
    // A click anywhere in the box, toolbar included, focuses the field.
    let toolbar = visual.debug_bounds("mono-composer-toolbar").unwrap();
    for point in [
        gpui::point(toolbar.center().x, toolbar.top() + px(13.)),
        gpui::point(send.left() - px(4.), toolbar.top() + px(13.)),
    ] {
        handle.update(visual, |_, w, cx| w.blur(cx)).unwrap();
        visual.simulate_click(point, Default::default());
        visual.run_until_parked();
        let chat = visual.update(|_, cx| s.manager.read(cx).chat().unwrap());
        assert!(
            handle
                .update(visual, |_, w, cx| chat
                    .read(cx)
                    .input
                    .focus_handle(cx)
                    .is_focused(w))
                .unwrap(),
            "a click at {point:?} focuses the composer"
        );
    }
    // One line: the session composer's 84px box; send sits 10px above the border.
    assert_eq!(field.size.height, px(84.));
    assert_eq!(field.bottom() - send.bottom(), px(11.));
    // The session composer's box: the model picker and send inside it,
    // attach underneath on the left.
    assert!(field.contains(&send.center()));
    assert!(field.contains(&model.center()));
    assert!(model.right() < send.left());
    assert!(attach.top() >= field.bottom());
    assert_eq!(attach.size, gpui::size(px(26.), px(26.)));
    let meter = visual.debug_bounds("mono-context-meter").unwrap();
    assert!(
        (f32::from(meter.center().x) - f32::from(send.center().x)).abs() < 0.5,
        "the context meter sits under send: {meter:?} vs {send:?}"
    );
    let chat = visual.update(|_, cx| s.manager.read(cx).chat().unwrap());
    for draft in [
        "".to_owned(),
        "First line\nSecond line".to_owned(),
        "A message that wraps automatically. ".repeat(40),
        "".to_owned(),
    ] {
        chat.update(visual, |chat, cx| {
            chat.input.update(cx, |input, cx| input.set_text(draft, cx));
        });
        visual.run_until_parked();
        visual.refresh().unwrap();
        let field = visual.debug_bounds("mono-composer").unwrap();
        let send = visual.debug_bounds("mono-send").unwrap();
        let attach = visual.debug_bounds("mono-attach").unwrap();
        assert!(field.contains(&send.center()));
        assert!(attach.top() >= field.bottom());
    }
    handle
        .update(visual, |_, w, cx| {
            chat.update(cx, |c, cx| c.toggle_model_menu(w, cx))
        })
        .unwrap();
    visual.run_until_parked();
    visual.update(|_, cx| {
        let (models, menu) = chat.read(cx).mono_menus_open();
        assert!(models, "the trigger opens the model list itself");
        assert!(!menu, "not the session composer's settings menu");
    });
}

#[gpui::test]
fn details_tabs_share_the_session_titlebar_in_both_layouts(cx: &mut TestAppContext) {
    let s = setup(cx);
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        crate::app::shell::commands::bind_keys(cx);
    });
    let session = s
        .window
        .update(cx, |shell, w, cx| {
            s.manager.update(cx, |m, cx| {
                m.settings = None;
                m.focus_active(w, cx);
                cx.notify();
            });
            shell.workspace.active.unwrap()
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    for classic in [false, true] {
        s.window
            .update(visual, |shell, _, cx| {
                shell.workspace.new_layout_boxy = classic;
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        let panel = visual.debug_bounds("right-panel-frame").unwrap();
        let tabs = visual.debug_bounds("mono-overview-tabs").unwrap();
        let toggle = visual.debug_bounds("toggle-right-panel").unwrap();
        let title_tabs = visual.debug_bounds("title-tab-strip").unwrap();
        assert!((tabs.top() - title_tabs.top()).abs() <= px(0.5));
        assert_eq!(tabs.size.height, px(TITLE_BAR_HEIGHT));
        assert_eq!(
            tabs.left(),
            panel.left() + px(if classic { 6. } else { 7. })
        );
        let activity = visual.debug_bounds("mono-activity").unwrap();
        let search = visual.debug_bounds("titlebar-file-search").unwrap();
        assert_eq!(search.left() - activity.right(), px(6.));
        assert_eq!(toggle.left() - search.right(), px(4.));
        assert!(tabs.bottom() <= panel.top() + px(if classic { TITLE_BAR_HEIGHT } else { 0. }));
        assert_eq!(
            visual.debug_bounds("mono-dashboard").unwrap().top(),
            panel.top() + px(if classic { TITLE_BAR_HEIGHT } else { 0. })
        );
        assert!(
            visual.debug_bounds("mono-overview-toggle").is_none(),
            "only the shell owns the panel toggle"
        );
        assert!(title_tabs.right() <= tabs.left());
        s.window
            .update(visual, |shell, w, cx| shell.open(session, w, cx))
            .unwrap();
        visual.run_until_parked();
        if !s
            .window
            .update(visual, |shell, _, _| shell.workspace.right_panel_open)
            .unwrap()
        {
            let button = visual.debug_bounds("toggle-right-panel").unwrap();
            visual.simulate_click(button.center(), Default::default());
            visual.run_until_parked();
        }
        assert_eq!(
            visual.debug_bounds("toggle-right-panel").unwrap(),
            toggle,
            "the toggle stays put when changing tab types"
        );
        let selector = if classic {
            "right-panel-explorer"
        } else {
            "project-tabs-separator"
        };
        assert_eq!(
            visual.debug_bounds(selector).unwrap().center().y,
            tabs.center().y
        );
        s.window
            .update(visual, |_, w, cx| {
                s.manager.update(cx, |m, cx| m.open(&s.id, w, cx))
            })
            .unwrap();
        visual.run_until_parked();
    }
}

#[gpui::test]
fn titlebar_details_tabs_switch_pages_and_keep_activity_and_habit_actions(cx: &mut TestAppContext) {
    let s = setup(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    s.window
        .update(cx, |shell, _, cx| {
            shell.workspace.right_panel_width = 400.;
            s.manager.update(cx, |m, cx| {
                m.settings = None;
                cx.notify();
            })
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    for (selector, tab) in [
        ("mono-overview-Documents", OverviewTab::Documents),
        ("mono-overview-Habits", OverviewTab::Habits),
    ] {
        let button = visual.debug_bounds(selector).unwrap();
        visual.simulate_click(button.center(), Default::default());
        visual.run_until_parked();
        assert!(s.manager.read_with(visual, |m, _| m.overview_tab == tab));
        assert!(visual.debug_bounds(selector).unwrap().size.width > px(26.));
    }
    let add = visual.debug_bounds("mono-new-habit").unwrap();
    visual.simulate_click(add.center(), Default::default());
    visual.run_until_parked();
    assert!(
        s.manager
            .read_with(visual, |m, _| matches!(m.route, Some(Route::NewHabit)))
    );
    assert!(visual.debug_bounds("mono-overview-tabs").is_some());
    let sessions = visual.debug_bounds("mono-overview-Sessions").unwrap();
    visual.simulate_click(sessions.center(), Default::default());
    visual.run_until_parked();
    assert!(s.manager.read_with(visual, |m, _| m.route.is_none()
        && m.overview_tab == OverviewTab::Sessions));
    for expected in [OverviewTab::Activity, OverviewTab::Sessions] {
        let activity = visual.debug_bounds("mono-activity").unwrap();
        visual.simulate_click(activity.center(), Default::default());
        visual.run_until_parked();
        assert!(
            s.manager
                .read_with(visual, |m, _| m.overview_tab == expected)
        );
    }
}

#[gpui::test]
fn titlebar_divider_resizes_mono_details_and_closed_panels_keep_the_corner(
    cx: &mut TestAppContext,
) {
    let s = setup(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    s.window
        .update(cx, |shell, _, cx| {
            // The titlebar divider belongs to the rounded frame.
            shell.workspace.new_layout_boxy = false;
            s.manager.update(cx, |m, cx| {
                m.settings = None;
                cx.notify();
            })
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    for width in [220., 280., 400., 500.] {
        s.window
            .update(visual, |shell, _, cx| {
                shell.workspace.right_panel_width = width;
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        assert_eq!(
            visual
                .debug_bounds("project-tabs-separator")
                .unwrap()
                .left(),
            visual.debug_bounds("right-panel-frame").unwrap().left()
        );
        let activity = visual.debug_bounds("mono-activity").unwrap();
        let toggle = visual.debug_bounds("toggle-right-panel").unwrap();
        assert!(
            activity.right() < toggle.left(),
            "Mono controls fit the session sidebar's minimum width"
        );
    }
    let start = visual
        .debug_bounds("titlebar-right-panel-resize")
        .unwrap()
        .center();
    let end = start - gpui::point(px(40.), px(0.));
    visual.simulate_mouse_down(start, MouseButton::Left, Default::default());
    visual.simulate_mouse_move(end, MouseButton::Left, Default::default());
    visual.simulate_mouse_up(end, MouseButton::Left, Default::default());
    visual.run_until_parked();
    s.window
        .update(visual, |shell, _, _| {
            assert_eq!(shell.workspace.right_panel_width, 540.);
        })
        .unwrap();
    assert_eq!(
        visual
            .debug_bounds("project-tabs-separator")
            .unwrap()
            .left(),
        visual.debug_bounds("right-panel-frame").unwrap().left()
    );
    for classic in [false, true] {
        s.window
            .update(visual, |shell, _, cx| {
                shell.workspace.new_layout_boxy = classic;
                shell.git.update(cx, |g, cx| g.set_project("~".into(), cx));
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        let open = visual.debug_bounds("toggle-right-panel").unwrap();
        visual.simulate_click(open.center(), Default::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds("mono-overview-tabs").is_none());
        assert!(visual.debug_bounds("mono-overview-panel").is_none());
        assert_eq!(visual.debug_bounds("toggle-right-panel").unwrap(), open);
        visual.simulate_click(open.center(), Default::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds("mono-overview-tabs").is_some());
    }
}

#[gpui::test]
fn opening_a_session_card_uses_the_shared_sidebar_state(cx: &mut TestAppContext) {
    let s = setup(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let session = s
        .window
        .update(cx, |shell, _, cx| {
            s.manager.update(cx, |m, cx| {
                m.settings = None;
                cx.notify();
            });
            shell.workspace.active.unwrap()
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    let toggle = visual.debug_bounds("toggle-right-panel").unwrap();
    visual.simulate_click(toggle.center(), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("right-panel-frame").is_none());
    s.window
        .update(visual, |_, _, cx| {
            s.manager.update(cx, |m, cx| {
                m.card_action(&s.id, &json!({"type":"session", "sessionId":session}), cx);
            })
        })
        .unwrap();
    visual.run_until_parked();
    let details = visual.debug_bounds("right-panel-frame").unwrap();
    assert!(s.manager.read_with(
        visual,
        |m, _| matches!(m.route, Some(Route::Session(id)) if id == session)
    ));
    s.window
        .update(visual, |shell, w, cx| {
            assert!(shell.workspace.right_panel_open);
            shell.open(session, w, cx);
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(visual.debug_bounds("right-panel-frame").unwrap(), details);
}

#[gpui::test]
fn a_session_open_in_mono_details_keeps_its_history(cx: &mut TestAppContext) {
    let s = setup(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let session = s
        .window
        .update(cx, |shell, _, cx| {
            s.manager.update(cx, |m, cx| {
                m.settings = None;
                cx.notify();
            });
            shell.workspace.active.unwrap()
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    s.window
        .update(visual, |_, _, cx| {
            s.manager.update(cx, |m, cx| {
                m.card_action(&s.id, &json!({"type":"session", "sessionId":session}), cx);
            })
        })
        .unwrap();
    visual.run_until_parked();
    // The details panel shows this session, so memory trimming must not
    // release its transcript while the Mono view is active.
    s.window
        .update(visual, |shell, _, cx| {
            assert!(shell.workspace.right_panel_open);
            let chat = shell.mono_ensure_session(session, cx).unwrap();
            assert!(chat.read(cx).history_loaded());
            assert!(!chat.update(cx, |chat, cx| chat.unload_history(cx)));
        })
        .unwrap();
}
