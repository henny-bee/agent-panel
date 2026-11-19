use super::age_label;
use crate::app::shell::*;
use chrono::TimeZone;
use gpui::TestAppContext;

#[gpui::test]
fn focused_navigation_trailing_items_share_the_filter_axis(cx: &mut TestAppContext) {
    let (window, _, _) = crate::app::shell::tests::window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    for count in [0, 128] {
        window
            .update(visual, |shell, _, cx| {
                shell.note_count = Some(count);
                shell.notify_sidebar(cx);
            })
            .unwrap();
        visual.run_until_parked();
        let axis = visual
            .debug_bounds("sidebar-filter-button")
            .unwrap()
            .center()
            .x;
        for selector in [
            "sidebar-navigation-trailing-New",
            "sidebar-navigation-trailing-Notes",
        ] {
            let trailing = visual.debug_bounds(selector).unwrap();
            assert_eq!(trailing.center().x, axis, "{selector} with {count} notes");
        }
    }
}
#[gpui::test]
fn focused_search_sits_beside_sidebar_options(cx: &mut TestAppContext) {
    let (window, _, _) = crate::app::shell::tests::window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    assert!(visual.debug_bounds("sidebar-search-field").is_none());
    let search = visual.debug_bounds("sidebar-search-button").unwrap();
    let options = visual.debug_bounds("sidebar-options").unwrap();
    assert!(search.right() <= options.left());
    assert!((search.center().y - options.center().y).abs() <= px(1.));
    visual.simulate_click(search.center(), Default::default());
    visual.run_until_parked();
    window
        .update(visual, |shell, _, _| assert!(shell.app_search.is_some()))
        .unwrap();
}

#[gpui::test]
fn focused_new_session_leads_navigation_and_marks_blank_sessions(cx: &mut TestAppContext) {
    let (window, _, active) = crate::app::shell::tests::window(cx);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let new = visual.debug_bounds("sidebar-navigation-New").unwrap();
    let inbox = visual.debug_bounds("sidebar-navigation-Inbox").unwrap();
    assert!(new.bottom() <= inbox.top());
    let selected = |visual: &mut gpui::VisualTestContext| {
        window
            .update(visual, |shell, _, cx| shell.active_session_is_blank(cx))
            .unwrap()
    };
    window
        .update(visual, |shell, _, cx| {
            assert_eq!(shell.workspace.active, Some(active));
            assert!(!shell.session_metadata(active, cx).summary.has_messages);
        })
        .unwrap();
    assert!(selected(visual));
    visual.simulate_click(inbox.center(), Default::default());
    visual.run_until_parked();
    assert!(!selected(visual), "Inbox takes the highlight");
    window
        .update(visual, |shell, w, cx| {
            shell.close_app_view(w, cx);
            shell.sessions[&active].update(cx, |session, _| session.summary.has_messages = true);
        })
        .unwrap();
    visual.run_until_parked();
    assert!(!selected(visual), "a conversation is not a new session");
    window
        .update(visual, |shell, _, cx| {
            shell.workspace.classic_layout = true;
            shell.invalidate_chrome(cx);
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("sidebar-navigation-New").is_none());
}

#[gpui::test]
fn sidebar_options_switch_card_heights_and_persist_without_loading_history(
    cx: &mut TestAppContext,
) {
    let (window, _, _) = crate::app::shell::tests::window(cx);
    let (id, loaded) = window
        .update(cx, |shell, _, cx| {
            let mut summary = locked(&shell.store)
                .unwrap()
                .create("codex", "/work/monocode")
                .unwrap();
            summary.has_messages = true;
            summary.title =
                "A detailed session with a long title that must stay inside its card".into();
            let id = summary.id;
            shell.session_details.insert(
                id,
                monocode_core::store::MobileSessionMetadata {
                    model: Some("gpt-6.1-sol".into()),
                    branch: Some("feature/session-cards".into()),
                    ..Default::default()
                },
            );
            shell.session_summaries.insert(id, summary);
            shell.order.insert(0, id);
            shell.notify_sidebar(cx);
            (id, shell.sessions.len())
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    let selector: &'static str = Box::leak(format!("sidebar-session-{id}").into_boxed_str());
    let model_selector: &'static str = Box::leak(format!("session-model-{id}").into_boxed_str());
    let workspace_selector: &'static str =
        Box::leak(format!("session-workspace-{id}").into_boxed_str());
    let title_selector: &'static str = Box::leak(format!("session-title-{id}").into_boxed_str());
    for boxy in [false, true] {
        window
            .update(visual, |shell, _, cx| {
                shell.workspace.new_layout_boxy = boxy;
                shell.layout_changed(cx);
            })
            .unwrap();
        visual.run_until_parked();
        assert_eq!(visual.debug_bounds(selector).unwrap().size.height, px(32.));
        let options = visual.debug_bounds("sidebar-options").unwrap();
        visual.simulate_click(options.center(), Default::default());
        visual.run_until_parked();
        let compact = visual
            .debug_bounds("menu-action-compact-session-cards")
            .unwrap();
        visual.simulate_click(compact.center(), Default::default());
        visual.run_until_parked();
        let row = visual.debug_bounds(selector).unwrap();
        assert_eq!(row.size.height, px(60.));
        let model = visual.debug_bounds(model_selector).unwrap();
        let workspace = visual.debug_bounds(workspace_selector).unwrap();
        let title = visual.debug_bounds(title_selector).unwrap();
        assert!(row.top() <= title.top() && title.bottom() <= workspace.top());
        assert_eq!(model.center().y, workspace.center().y);
        assert!(workspace.right() <= model.left());
        assert!(workspace.bottom() <= row.bottom());
        assert!(
            model.right() <= row.right()
                && workspace.right() <= row.right()
                && title.right() <= row.right()
        );
        window
            .update(visual, |shell, _, _| {
                assert_eq!(shell.sessions.len(), loaded);
                assert!(!shell.sessions.contains_key(&id));
                let saved = shell.latest_state.lock().unwrap().clone();
                let restored = Workspace::restore(Some(&saved), &[], "/work/monocode");
                assert!(!restored.compact_session_cards);
            })
            .unwrap();
        // Switches keep the popover open for more choices.
        let compact = visual
            .debug_bounds("menu-action-compact-session-cards")
            .unwrap();
        visual.simulate_click(compact.center(), Default::default());
        visual.run_until_parked();
        assert_eq!(visual.debug_bounds(selector).unwrap().size.height, px(32.));
        assert!(visual.debug_bounds(model_selector).is_none());
        visual.simulate_click(options.center(), Default::default());
        visual.run_until_parked();
    }
}

#[gpui::test]
fn sidebar_visibility_switches_stay_open_preserve_notes_and_survive_restart(
    cx: &mut TestAppContext,
) {
    let (window, _, _) = crate::app::shell::tests::window(cx);
    window
        .update(cx, |shell, w, cx| shell.open_notes(w, cx))
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let options = visual.debug_bounds("sidebar-options").unwrap();
    visual.simulate_click(options.center(), Default::default());
    visual.run_until_parked();
    window
        .update(visual, |shell, _, cx| {
            let menu = shell.menu.as_ref().unwrap().menu.read(cx);
            let bounds = menu.surface_region().get().unwrap();
            assert_eq!(bounds.left(), options.left());
            assert_eq!(bounds.size.width, px(184.));
        })
        .unwrap();
    let search = visual.debug_bounds("menu-action-sidebar-search").unwrap();
    let notes = visual.debug_bounds("menu-action-sidebar-notes").unwrap();
    assert!(search.bottom() <= notes.top());
    let automations = visual
        .debug_bounds("menu-action-sidebar-automations")
        .unwrap();
    let inbox = visual.debug_bounds("menu-action-sidebar-inbox").unwrap();
    let monos = visual.debug_bounds("menu-action-sidebar-monos").unwrap();
    let activity = visual.debug_bounds("menu-action-sidebar-activity").unwrap();
    let compact = visual
        .debug_bounds("menu-action-compact-session-cards")
        .unwrap();
    assert!(notes.bottom() <= automations.top() && automations.bottom() <= inbox.top());
    assert!(inbox.bottom() <= monos.top() && monos.bottom() <= activity.top());
    assert!(
        activity.bottom() < compact.top(),
        "a divider separates Compact view"
    );
    for (action, navigation) in [
        ("menu-action-sidebar-search", "sidebar-search-button"),
        ("menu-action-sidebar-notes", "sidebar-navigation-Notes"),
        (
            "menu-action-sidebar-automations",
            "sidebar-navigation-Automations",
        ),
        ("menu-action-sidebar-inbox", "sidebar-navigation-Inbox"),
        ("menu-action-sidebar-monos", "new-mono"),
    ] {
        assert!(visual.debug_bounds(navigation).is_some());
        let row = visual.debug_bounds(action).unwrap();
        visual.simulate_click(row.center(), Default::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds(navigation).is_none());
        assert!(
            visual
                .debug_bounds("menu-action-compact-session-cards")
                .is_some()
        );
    }
    window
        .update(visual, |shell, _, cx| {
            assert_eq!(shell.active_app_view, Some(glue::app_views::AppView::Notes));
            assert!(shell.notes.is_some());
            assert!(
                shell.workspace.preferences.notes,
                "hiding the shortcut leaves Notes enabled"
            );
            let menu = shell.menu.as_ref().unwrap().menu.read(cx);
            for id in [
                "sidebar-search",
                "sidebar-notes",
                "sidebar-automations",
                "sidebar-inbox",
                "sidebar-monos",
            ] {
                assert_eq!(menu.toggle_state(id), Some(false));
            }
            assert!(!cx.global::<crate::ui::preferences::Preferences>().monos);
            let saved = shell.latest_state.lock().unwrap().clone();
            let restored = Workspace::restore(Some(&saved), &[], "/work/monocode");
            assert!(!restored.sidebar_search_visible);
            assert!(!restored.sidebar_notes_visible);
            assert!(!restored.sidebar_automations_visible);
            assert!(!restored.sidebar_inbox_visible);
            assert!(!restored.preferences.monos);
        })
        .unwrap();
    for (action, navigation) in [
        ("menu-action-sidebar-search", "sidebar-search-button"),
        ("menu-action-sidebar-notes", "sidebar-navigation-Notes"),
        (
            "menu-action-sidebar-automations",
            "sidebar-navigation-Automations",
        ),
        ("menu-action-sidebar-inbox", "sidebar-navigation-Inbox"),
        ("menu-action-sidebar-monos", "new-mono"),
    ] {
        let row = visual.debug_bounds(action).unwrap();
        visual.simulate_click(row.center(), Default::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds(navigation).is_some());
    }
    visual.simulate_click(options.center(), Default::default());
    visual.run_until_parked();
    window
        .update(visual, |shell, w, cx| {
            shell.open_settings(crate::settings::Section::Monos, w, cx)
        })
        .unwrap();
    visual.run_until_parked();
    let toggle = visual
        .debug_bounds("settings-toggle-monos-enabled")
        .unwrap();
    visual.simulate_click(toggle.center(), Default::default());
    visual.run_until_parked();
    window
        .update(visual, |shell, w, cx| {
            assert!(!shell.workspace.preferences.monos);
            shell.close_settings(w, cx);
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("new-mono").is_none());
    let options = visual.debug_bounds("sidebar-options").unwrap();
    visual.simulate_click(options.center(), Default::default());
    visual.run_until_parked();
    window
        .update(visual, |shell, _, cx| {
            let menu = shell.menu.as_ref().unwrap().menu.read(cx);
            assert_eq!(menu.toggle_state("sidebar-monos"), Some(false));
        })
        .unwrap();
}

#[gpui::test]
fn activity_visibility_switch_preserves_agents_and_survives_restart(cx: &mut TestAppContext) {
    let (window, a, b) = crate::app::shell::tests::window(cx);
    window
        .update(cx, |shell, _, cx| {
            for id in [a, b] {
                shell.sessions[&id].update(cx, |session, _| session.busy = true);
            }
            shell.invalidate_chrome(cx);
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    for boxy in [false, true] {
        window
            .update(visual, |shell, _, cx| {
                shell.workspace.new_layout_boxy = boxy;
                shell.layout_changed(cx);
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("sidebar-activity-card").is_some());
        let options = visual.debug_bounds("sidebar-options").unwrap();
        visual.simulate_click(options.center(), Default::default());
        visual.run_until_parked();
        let activity = visual.debug_bounds("menu-action-sidebar-activity").unwrap();
        visual.simulate_click(activity.center(), Default::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds("sidebar-activity-card").is_none());
        window
            .update(visual, |shell, _, cx| {
                assert!(shell.workspace.preferences.working_agents);
                for id in [a, b] {
                    assert!(shell.sessions[&id].read(cx).responding());
                }
                let menu = shell.menu.as_ref().unwrap().menu.read(cx);
                assert_eq!(menu.toggle_state("sidebar-activity"), Some(false));
                let saved = shell.latest_state.lock().unwrap().clone();
                let restored = Workspace::restore(Some(&saved), &[], "/work/monocode");
                assert!(!restored.sidebar_activity_visible);
            })
            .unwrap();
        let activity = visual.debug_bounds("menu-action-sidebar-activity").unwrap();
        visual.simulate_click(activity.center(), Default::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds("sidebar-activity-card").is_some());
        window
            .update(visual, |shell, _, _| {
                let saved = shell.latest_state.lock().unwrap().clone();
                let restored = Workspace::restore(Some(&saved), &[], "/work/monocode");
                assert!(restored.sidebar_activity_visible);
            })
            .unwrap();
        visual.simulate_click(options.center(), Default::default());
        visual.run_until_parked();
    }
    window
        .update(visual, |shell, _, cx| {
            shell.workspace.preferences.working_agents = false;
            shell.invalidate_chrome(cx);
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("sidebar-activity-card").is_none());
}

#[gpui::test]
fn scroll_and_hover_redraws_reuse_the_sidebar_index(cx: &mut TestAppContext) {
    let (window, _, _) = crate::app::shell::tests::window(cx);
    window
        .update(cx, |shell, _, cx| {
            shell.refresh_sidebar_structure(cx);
            let rebuilds = shell.sidebar_list.structural_rebuilds;
            for _ in 0..100 {
                // This is the path used by scroll/hover redraws: visible
                // cards refresh, while membership/order stays cached.
                shell.refresh_sidebar_structure(cx);
            }
            assert_eq!(shell.sidebar_list.structural_rebuilds, rebuilds);
            shell.sidebar_list.invalidate_structure();
            shell.refresh_sidebar_structure(cx);
            assert_eq!(shell.sidebar_list.structural_rebuilds, rebuilds + 1);
            // Time expiry is a content refresh, not navigation: retain
            // the user's position instead of snapping back to the active row.
            shell.sidebar_list.state.scroll_to(gpui::ListOffset {
                item_ix: 1,
                offset_in_item: px(5.),
            });
            let offset = shell.sidebar_list.state.logical_scroll_top();
            shell.sidebar_list.indexed_minute = 0;
            shell.refresh_sidebar_structure(cx);
            let refreshed = shell.sidebar_list.state.logical_scroll_top();
            assert_eq!(refreshed.item_ix, offset.item_ix);
            assert_eq!(refreshed.offset_in_item, offset.offset_in_item);
            assert_eq!(shell.sidebar_list.structural_rebuilds, rebuilds + 2);
            // Scope changes are also detected without a notification hook.
            shell.query.push_str("unmatched-session-query");
            shell.refresh_sidebar_structure(cx);
            assert_eq!(shell.sidebar_list.structural_rebuilds, rebuilds + 3);
            assert!(shell.sidebar_list.rows.contains(&super::Row::Empty));
        })
        .unwrap();
}

#[gpui::test]
fn large_project_only_lays_out_nearby_session_cards(cx: &mut TestAppContext) {
    let (window, _, _) = crate::app::shell::tests::window(cx);
    let started = std::time::Instant::now();
    let (first, last) = window
        .update(cx, |shell, window, cx| {
            let mut summaries = Vec::new();
            {
                let store = locked(&shell.store).unwrap();
                for i in 0..1000 {
                    let mut summary = store.create("codex", "/work/many-sessions").unwrap();
                    summary.title =
                        format!("Session {i}: {}", "A long conversation title 🙂 ".repeat(4));
                    summary.has_messages = true;
                    summaries.push(summary);
                }
            }
            let first = summaries.last().unwrap().id;
            let last = summaries[0].id;
            for summary in summaries {
                shell.order.push(summary.id);
                shell.session_summaries.insert(summary.id, summary);
            }
            shell.workspace.open(first);
            shell.select_project("/work/many-sessions".into(), window, cx);
            (first, last)
        })
        .unwrap();
    cx.refresh().unwrap();
    window
        .update(cx, |shell, _, cx| {
            let laid_out = shell
                .session_row_bounds
                .values()
                .filter(|slot| slot.get().is_some())
                .count();
            eprintln!(
                "1,000-session project: {laid_out} laid-out cards, {:?} setup + first frame",
                started.elapsed()
            );
            assert!(laid_out > 0);
            assert!(
                shell.sessions.len() < 10,
                "scrolling the sidebar must not construct off-screen conversations"
            );
            assert!(
                laid_out < 80,
                "off-screen session cards should not be laid out: {laid_out}"
            );
            assert!(shell.session_row_bounds[&first].get().is_some());
            assert!(shell.session_row_bounds[&last].get().is_none());
            shell.sidebar_list.state.scroll_to_end();
            shell.sidebar_frame.update(cx, |_, cx| cx.notify());
        })
        .unwrap();
    cx.refresh().unwrap();
    window
        .update(cx, |shell, _, cx| {
            assert!(
                shell.session_row_bounds[&first].get().is_none(),
                "scrolled-away cards cannot remain hit targets"
            );
            assert!(
                shell.session_row_bounds[&last].get().is_some(),
                "the oldest conversation is reachable"
            );
            let members = shell.sidebar_session_ids(cx);
            shell.groups.insert(
                "/work/many-sessions".into(),
                monocode_core::session::groups::Groups {
                    folders: vec![monocode_core::session::groups::Folder {
                        id: "large-folder".into(),
                        name: "All history".into(),
                        session_ids: members,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            );
            shell.notify_sidebar(cx);
        })
        .unwrap();
    cx.refresh().unwrap();
    window
        .update(cx, |shell, _, _| {
            let laid_out = shell
                .session_row_bounds
                .values()
                .filter(|slot| slot.get().is_some())
                .count();
            assert!(
                laid_out > 0 && laid_out < 80,
                "expanded folders must also be virtualized: {laid_out}"
            );
            shell.sidebar_list.state.scroll_to_end();
        })
        .unwrap();
    cx.refresh().unwrap();
    let offset = window
        .update(cx, |shell, window, cx| {
            // Resolve scroll_to_end's end sentinel before checking selection.
            let offset = shell.sidebar_list.state.logical_scroll_top();
            shell.open(first - 1, window, cx);
            offset
        })
        .unwrap();
    cx.refresh().unwrap();
    window
        .update(cx, |shell, _, _| {
            // Like the Tauri sidebar, selecting a session (e.g. from a
            // title tab) leaves the user's scroll position alone.
            let after = shell.sidebar_list.state.logical_scroll_top();
            assert_eq!(
                (after.item_ix, after.offset_in_item),
                (offset.item_ix, offset.offset_in_item)
            );
            assert!(shell.session_row_bounds[&(first - 1)].get().is_none());
        })
        .unwrap();
}

#[test]
fn sidebar_dates_use_local_calendar_days_and_match_the_tauri_sections() {
    let ms = |day, hour, minute| {
        chrono::Local
            .with_ymd_and_hms(2026, 10, day, hour, minute, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64
    };
    let now = ms(20, 0, 5);
    assert_eq!(age_label(ms(20, 0, 1), now), "Today");
    assert_eq!(age_label(ms(19, 23, 59), now), "Yesterday");
    assert_eq!(age_label(ms(14, 0, 0), now), "Last 7 days");
    assert_eq!(age_label(ms(13, 23, 59), now), "Older");
    assert_eq!(age_label(0, now), "Older");
    // A local calendar day remains Yesterday across daylight-saving changes.
    assert_eq!(age_label(ms(25, 0, 1), ms(26, 0, 5)), "Yesterday");
}
