use super::*;

#[gpui::test]
fn window_state_is_saved_on_resize_and_restored_with_the_workspace(cx: &mut TestAppContext) {
    let (window, _, _) = window(cx);
    cx.refresh().unwrap();
    cx.run_until_parked();
    let dimensions = gpui::size(px(1440.), px(900.));
    cx.simulate_window_resize(window.into(), dimensions);
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            let saved = locked(&shell.store).unwrap().ui_state().unwrap().unwrap();
            assert_eq!(*shell.latest_state.lock().unwrap(), saved);
            let restored = Workspace::restore(Some(&saved), &[], "/work/monocode");
            assert_eq!(restored.window.bounds(cx).get_bounds().size, dimensions);
        })
        .unwrap();
}

#[gpui::test]
fn window_state_preserves_fullscreen_when_backend_bounds_report_windowed(cx: &mut TestAppContext) {
    let (window, _, _) = window(cx);
    let normal_size = gpui::size(px(1440.), px(900.));
    cx.simulate_window_resize(window.into(), normal_size);
    cx.run_until_parked();
    window
        .update(cx, |_, window, _| window.toggle_fullscreen())
        .unwrap();
    cx.simulate_window_resize(window.into(), gpui::size(px(1920.), px(1080.)));
    cx.run_until_parked();
    window
        .update(cx, |shell, window, cx| {
            // The test backend reproduces X11: fullscreen is tracked separately from bounds.
            assert!(window.is_fullscreen());
            assert!(matches!(
                window.inner_window_bounds(),
                gpui::WindowBounds::Windowed(_)
            ));
            let saved = locked(&shell.store).unwrap().ui_state().unwrap().unwrap();
            assert_eq!(*shell.latest_state.lock().unwrap(), saved);
            let restored = Workspace::restore(Some(&saved), &[], "/work/monocode");
            let bounds = restored.window.bounds(cx);
            assert!(matches!(bounds, gpui::WindowBounds::Fullscreen(_)));
            assert_eq!(bounds.get_bounds().size, normal_size);
        })
        .unwrap();
}

#[gpui::test]
fn hovering_archive_preserves_history_and_does_not_select_the_row(cx: &mut TestAppContext) {
    let (window, a, b) = window(cx);
    window
        .update(cx, |shell, _, cx| {
            shell.sessions[&a].update(cx, |session, _| {
                session.summary.cwd = "/work/monocode".into();
                session.summary.has_messages = true;
            });
            shell.sessions[&b].update(cx, |session, _| session.summary.has_messages = true);
            shell.invalidate_chrome(cx);
        })
        .unwrap();
    cx.refresh().unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    let row = visual.debug_bounds("sidebar-session-1").unwrap();
    visual.simulate_mouse_move(row.center(), None, Default::default());
    visual.run_until_parked();
    visual.refresh().unwrap();
    let archive = visual
        .debug_bounds("archive-session-1")
        .expect("hover reveals Archive");
    visual.simulate_click(archive.center(), Default::default());
    visual.run_until_parked();
    window
        .update(visual, |shell, window, cx| {
            assert_eq!(
                shell.workspace.active,
                Some(b),
                "the archive click must not select its row"
            );
            assert!(!shell.workspace.tabs.contains(&a));
            assert!(shell.sessions[&a].read(cx).summary.archived);
            assert!(!shell.sidebar_session_ids(cx).contains(&a));
            assert!(
                !locked(&shell.store)
                    .unwrap()
                    .load(a)
                    .unwrap()
                    .rows
                    .is_empty()
            );
            shell.run_menu(MenuTarget::SessionFilter, "archived", None, window, cx);
            assert!(shell.sidebar_session_ids(cx).contains(&a));
            shell.toggle_archive(a, cx);
        })
        .unwrap();
    visual.run_until_parked();
    window
        .update(visual, |shell, window, cx| {
            assert!(!shell.sessions[&a].read(cx).summary.archived);
            shell.run_menu(MenuTarget::SessionFilter, "clear", None, window, cx);
            assert!(shell.sidebar_session_ids(cx).contains(&a));
            assert_eq!(shell.workspace.active, Some(b));
        })
        .unwrap();
}

#[gpui::test]
fn working_marks_animate_independently_and_completed_replies_clear_when_seen(
    cx: &mut TestAppContext,
) {
    let (window, a, b) = window(cx);
    let transcript = window
        .update(cx, |shell, _, cx| {
            shell.sessions[&a].update(cx, |session, cx| {
                session.busy = true;
                session.summary.cwd = "/work/monocode".into();
                session.summary.has_messages = true;
                cx.emit(SessionChanged);
            });
            shell.sessions[&b].read(cx).transcript.clone()
        })
        .unwrap();
    cx.run_until_parked();
    cx.refresh().unwrap();
    let before = transcript.read_with(cx, |view, _| view.render_passes);
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(160));
    cx.run_until_parked();
    assert_eq!(
        before,
        transcript.read_with(cx, |view, _| view.render_passes)
    );
    window
        .update(cx, |shell, _, cx| {
            shell.sessions[&a].update(cx, |session, cx| {
                session.busy = false;
                cx.emit(SessionChanged);
            });
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, window, cx| {
            assert!(
                shell.unseen_finished.contains(&a),
                "a reply completed in the background"
            );
            shell.open(a, window, cx);
            assert!(
                !shell.unseen_finished.contains(&a),
                "opening it restores the provider mark"
            );
        })
        .unwrap();
}

#[gpui::test]
fn hover_preview_waits_and_never_materializes_a_conversation(cx: &mut TestAppContext) {
    let (window, _, _) = window(cx);
    let (id, count) = window
        .update(cx, |shell, window, cx| {
            // Classic previews remain available without loading a conversation.
            shell.workspace.classic_layout = true;
            let mut summary = locked(&shell.store)
                .unwrap()
                .create("codex", "/work/monocode")
                .unwrap();
            summary.has_messages = true;
            let id = summary.id;
            shell.order.push(id);
            shell.session_summaries.insert(id, summary);
            let anchor = AnchorSlot::default();
            anchor.set(Some(Bounds::new(
                window.mouse_position() - gpui::point(px(5.), px(5.)),
                gpui::size(px(20.), px(20.)),
            )));
            let count = shell.sessions.len();
            shell.hover_session(id, anchor, true, true, window, cx);
            assert!(shell.session_preview.is_none());
            (id, count)
        })
        .unwrap();
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(130));
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| {
            assert_eq!(
                shell.session_preview.as_ref().map(|preview| preview.id),
                Some(id)
            );
            assert_eq!(shell.sessions.len(), count);
            assert!(!shell.sessions.contains_key(&id));
        })
        .unwrap();
}

#[gpui::test]
fn sidebar_rows_have_a_gap_and_preview_survives_crossing_into_the_card(cx: &mut TestAppContext) {
    let (window, a, b) = window(cx);
    window
        .update(cx, |shell, _, cx| {
            shell.sessions[&a].update(cx, |session, _| {
                session.summary.cwd = "/work/monocode".into();
                session.summary.has_messages = true;
            });
            shell.sessions[&b].update(cx, |session, _| session.summary.has_messages = true);
            shell.invalidate_chrome(cx);
        })
        .unwrap();
    cx.refresh().unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    let first = visual
        .debug_bounds(Box::leak(format!("sidebar-session-{b}").into_boxed_str()))
        .unwrap();
    let second = visual
        .debug_bounds(Box::leak(format!("sidebar-session-{a}").into_boxed_str()))
        .unwrap();
    assert_eq!(first.size.height, px(32.));
    assert_eq!(
        second.top() - first.bottom(),
        px(1.),
        "rows retain their 1px gap"
    );
    let move_to = |position, cx: &mut TestAppContext| {
        cx.update_window(window.into(), |_, window, cx| {
            window.simulate_mouse_move(position, cx)
        })
        .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
    };
    move_to(first.origin + gpui::point(px(20.), px(16.)), cx);
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(130));
    cx.run_until_parked();
    cx.refresh().unwrap();
    let (first, surface) = window
        .update(cx, |shell, _, _| {
            let preview = shell
                .session_preview
                .as_ref()
                .expect("hover opens a preview");
            assert_eq!(preview.id, b);
            (
                preview.anchor.get().unwrap(),
                preview.surface.get().unwrap(),
            )
        })
        .unwrap();
    // The close timer lets the pointer cross the 8px gap into the preview.
    move_to(
        gpui::point(first.right() + px(4.), first.top() + px(16.)),
        cx,
    );
    move_to(surface.center(), cx);
    window
        .update(cx, |shell, _, _| {
            assert!(shell.session_preview.is_some());
            assert!(
                shell.preview_close.is_none(),
                "entering the card cancels the close timer"
            );
        })
        .unwrap();
    move_to(second.origin + gpui::point(px(20.), px(16.)), cx);
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(130));
    cx.run_until_parked();
    cx.refresh().unwrap();
    window
        .update(cx, |shell, window, cx| {
            let preview = shell.session_preview.as_ref().unwrap();
            assert_eq!(preview.id, a, "another row replaces the old preview");
            assert_eq!(preview.anchor.get().unwrap().top() - first.bottom(), px(1.));
            shell.open(a, window, cx);
            assert!(
                shell.session_preview.is_none(),
                "selection dismisses the card"
            );
        })
        .unwrap();
}

#[gpui::test]
fn high_usage_chip_opens_running_work_and_escape_closes_it(cx: &mut TestAppContext) {
    let (window, _, _) = window(cx);
    cx.update(crate::ui::popover::bind);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    assert!(visual.debug_bounds("resource-pressure-chip").is_none());
    window
        .update(visual, |shell, _, cx| {
            shell
                .resources
                .update(cx, |monitor, cx| monitor.saturate(cx));
        })
        .unwrap();
    visual.run_until_parked();
    let chip = visual.debug_bounds("resource-pressure-chip").unwrap();
    let viewport = window.update(visual, |_, w, _| w.viewport_size()).unwrap();
    assert!(
        viewport.height - chip.bottom() < px(10.),
        "the chip sits in the footer"
    );
    visual.simulate_click(chip.center(), Default::default());
    visual.run_until_parked();
    let dialog = visual.debug_bounds("resource-dialog").unwrap();
    assert_eq!(dialog.size.width, px(418.));
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(visual.debug_bounds("resource-dialog").is_none());
    assert!(visual.debug_bounds("resource-pressure-chip").is_some());
}

#[gpui::test]
fn usage_footer_follows_active_tabs_and_provider_changes(cx: &mut TestAppContext) {
    let (window, a, b) = window(cx);
    window
        .update(cx, |shell, w, cx| {
            assert_eq!(shell.usage_footer.read(cx).kind(), Kind::Claude);
            shell.open(a, w, cx);
            assert_eq!(shell.usage_footer.read(cx).kind(), Kind::Codex);
            shell.close(a, w, cx);
            assert_eq!(shell.workspace.project, "/work/another-project");
            assert_eq!(shell.workspace.active, None);
            shell.open(b, w, cx);
            assert_eq!(shell.usage_footer.read(cx).kind(), Kind::Claude);
            shell.sessions[&b].update(cx, |conversation, cx| {
                conversation.summary.provider = "codex".into();
                cx.emit(SessionChanged);
            });
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(shell.usage_footer.read(cx).kind(), Kind::Codex);
        })
        .unwrap();
}

#[gpui::test]
fn context_menu_click_does_not_activate_the_tab_beneath_it(cx: &mut TestAppContext) {
    let (window, a, b) = same_project_window(cx);
    window
        .update(cx, |shell, w, cx| {
            shell.open(a, w, cx);
            let anchor = Anchor::Point(gpui::point(px(600.), px(5.)));
            shell.open_menu(MenuTarget::Tab(b), anchor, w, cx);
        })
        .unwrap();
    cx.refresh().unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    // The first action overlaps the tab strip. Pin it without selecting it.
    let pin = visual.debug_bounds("menu-action-pin").unwrap();
    visual.simulate_click(pin.center(), Default::default());
    visual.run_until_parked();
    window
        .update(visual, |shell, _, _| {
            assert_eq!(shell.workspace.tabs, [a, b]);
            assert_eq!(shell.workspace.pinned_tabs, [TitleTab::Session(b)]);
            assert_eq!(shell.workspace.active, Some(a));
            assert!(shell.menu.is_none());
        })
        .unwrap();
}
