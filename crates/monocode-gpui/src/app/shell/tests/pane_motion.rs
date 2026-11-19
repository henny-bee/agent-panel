use super::*;

#[gpui::test]
fn a_split_pane_slides_in_from_its_edge_then_settles(cx: &mut TestAppContext) {
    let (window, new, b) = same_project_window(cx);
    window
        .update(cx, |shell, w, cx| {
            shell.open(b, w, cx);
            shell.focus_active(w, cx);
        })
        .unwrap();
    let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            // Freeze before the first paint: keyboard simulation can draw
            // enough slow frames for the animation to finish on CI. The
            // shortcut itself is covered by the split-and-drag test above.
            let before = shell.active_leaves();
            shell.workspace.split(new, SplitDir::Right);
            shell.begin_pane_motion(before, cx);
            shell
                .pane_motion
                .as_ref()
                .unwrap()
                .slide
                .freeze_elapsed(std::time::Duration::from_millis(16));
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    cx.refresh().unwrap();
    let new = window
        .update(cx, |shell, _, _| {
            let new = shell.workspace.active.unwrap();
            let motion = shell.pane_motion.as_ref().expect("the split animates");
            assert_eq!(motion.entering.get(&new), Some(&Edge::Right));
            assert_eq!(motion.from[&b].w, 1.);
            new
        })
        .unwrap();
    let entering = cx
        .debug_bounds(Box::leak(format!("pane-{new}").into_boxed_str()))
        .unwrap();
    let pane = cx
        .debug_bounds(Box::leak(format!("pane-{b}").into_boxed_str()))
        .unwrap();
    // Mid-slide: the chat still holds most of the width.
    assert!(
        entering.size.width < pane.size.width,
        "{entering:?} {pane:?}"
    );
    window
        .update(cx, |shell, _, cx| {
            shell
                .pane_motion
                .as_ref()
                .unwrap()
                .slide
                .elapsed(std::time::Duration::from_secs(1));
            cx.notify();
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| assert!(shell.pane_motion.is_none()))
        .unwrap();
    let entering = cx
        .debug_bounds(Box::leak(format!("pane-{new}").into_boxed_str()))
        .unwrap();
    let pane = cx
        .debug_bounds(Box::leak(format!("pane-{b}").into_boxed_str()))
        .unwrap();
    assert!((entering.size.width - pane.size.width).abs() < px(2.));
}

#[gpui::test]
fn a_closed_pane_slides_out_to_its_edge_while_its_neighbour_grows(cx: &mut TestAppContext) {
    let (window, _, b) = window(cx);
    cx.update(|cx| {
        bind_keys(cx);
        cx.set_reduce_motion(true);
    });
    window
        .update(cx, |shell, w, cx| {
            shell.open(b, w, cx);
            shell.focus_active(w, cx);
        })
        .unwrap();
    let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    cx.run_until_parked();
    cx.simulate_keystrokes("secondary-d");
    cx.run_until_parked();
    cx.update(|_, cx| cx.set_reduce_motion(false));
    let new = window
        .update(cx, |shell, w, cx| {
            let new = shell.workspace.active.unwrap();
            shell.close_pane(new, w, cx);
            let motion = shell.pane_motion.as_ref().expect("closing animates");
            assert_eq!(motion.header, Some(false));
            assert_eq!(motion.leaving.len(), 1);
            assert_eq!(
                (motion.leaving[0].0.id, motion.leaving[0].1),
                (new, Edge::Right)
            );
            assert_eq!(shell.workspace.layout(b).leaf_ids(), [b]);
            new
        })
        .unwrap();
    cx.refresh().unwrap();
    assert!(
        cx.debug_bounds(Box::leak(format!("split-header-{new}").into_boxed_str()))
            .is_some(),
        "still painted while it slides out"
    );
    window
        .update(cx, |shell, _, _| {
            // Midway it has slid half its width toward the right edge,
            // and the lone pane has grown over the same distance.
            let motion = shell.pane_motion.as_ref().unwrap();
            let (leaf, _, to) = motion.leaving[0];
            let mid = leaf.rect.lerp(to, 0.5);
            let grown = motion.from[&b].lerp(Rect::FULL, 0.5);
            assert!((mid.x - (grown.x + grown.w)).abs() < 1e-5);
            assert!((mid.w - 0.25).abs() < 1e-5);
        })
        .unwrap();
    window
        .update(cx, |shell, _, cx| {
            let motion = shell.pane_motion.as_ref().unwrap();
            motion.slide.elapsed(std::time::Duration::from_secs(1));
            cx.notify();
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| assert!(shell.pane_motion.is_none()))
        .unwrap();
    assert!(
        cx.debug_bounds(Box::leak(format!("pane-{new}").into_boxed_str()))
            .is_none()
    );
}

#[gpui::test]
fn closing_session_header_stays_hidden_on_the_settling_frame(cx: &mut TestAppContext) {
    assert_closed_header_stays_hidden(cx, false);
}

#[gpui::test]
fn closing_file_header_stays_hidden_on_the_settling_frame(cx: &mut TestAppContext) {
    assert_closed_header_stays_hidden(cx, true);
}

#[gpui::test]
fn a_crowding_pane_collapses_the_sidebar_and_its_room_brings_it_back(cx: &mut TestAppContext) {
    let (window, _, b) = window(cx);
    cx.update(|cx| {
        bind_keys(cx);
        cx.set_reduce_motion(true);
    });
    cx.simulate_window_resize(window.into(), gpui::size(px(1000.), px(800.)));
    window
        .update(cx, |shell, w, cx| {
            shell.open(b, w, cx);
            shell.focus_active(w, cx);
        })
        .unwrap();
    let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| {
            assert!(shell.workspace.sidebar_visible);
            assert_eq!(shell.viewport_width, 1000.);
        })
        .unwrap();
    cx.simulate_keystrokes("secondary-d");
    cx.run_until_parked();
    let new = window
        .update(cx, |shell, _, cx| {
            // Two panes beside a 260px sidebar would be ~365px each.
            assert!(!shell.workspace.sidebar_visible);
            assert!(shell.narrowest_chat(false, cx) >= MIN_CHAT_WIDTH);
            shell.workspace.active.unwrap()
        })
        .unwrap();
    window
        .update(cx, |shell, w, cx| shell.close_pane(new, w, cx))
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| assert!(shell.workspace.sidebar_visible))
        .unwrap();
    // A sidebar the user hid stays hidden when room comes back.
    cx.simulate_keystrokes("secondary-d");
    cx.run_until_parked();
    // Collapsed for the split; shown, then hidden again by hand.
    cx.simulate_keystrokes("secondary-b secondary-b");
    cx.run_until_parked();
    window
        .update(cx, |shell, w, cx| {
            assert!(!shell.workspace.sidebar_visible);
            let pane = shell.workspace.active.unwrap();
            shell.close_pane(pane, w, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| assert!(!shell.workspace.sidebar_visible))
        .unwrap();
}

#[gpui::test]
fn a_split_session_starts_with_the_default_access(cx: &mut TestAppContext) {
    let (window, _, b) = window(cx);
    cx.update(|cx| {
        bind_keys(cx);
        cx.set_reduce_motion(true);
        cx.set_global(crate::ui::preferences::Preferences::default());
        crate::chat::conversation::remember_access(
            monocode_core::session::agent::AccessMode::from_id("auto"),
            cx,
        );
    });
    window
        .update(cx, |shell, w, cx| {
            shell.open(b, w, cx);
            shell.focus_active(w, cx);
        })
        .unwrap();
    let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    cx.run_until_parked();
    for keys in ["secondary-d", "secondary-shift-d"] {
        cx.simulate_keystrokes(keys);
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                let new = shell.workspace.active.unwrap();
                assert_ne!(new, b, "{keys} opens a new pane");
                let saved = locked(&shell.store)
                    .unwrap()
                    .agent_settings(new)
                    .unwrap()
                    .expect("the split session saves its settings");
                assert_eq!(saved.access.id(), "auto", "{keys} keeps the default access");
                let live = shell.sessions[&new].read(cx).settings.access;
                assert_eq!(live.id(), "auto", "{keys} shows the default access");
            })
            .unwrap();
    }
}

#[gpui::test]
fn the_sidebar_follows_the_room_however_it_changes(cx: &mut TestAppContext) {
    let (window, _, b) = window(cx);
    cx.update(|cx| {
        bind_keys(cx);
        cx.set_reduce_motion(true);
    });
    cx.simulate_window_resize(window.into(), gpui::size(px(1400.), px(800.)));
    window
        .update(cx, |shell, w, cx| {
            shell.open(b, w, cx);
            shell.focus_active(w, cx);
        })
        .unwrap();
    let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    cx.run_until_parked();
    // Wide enough for two panes beside the sidebar.
    cx.simulate_keystrokes("secondary-d");
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| assert!(shell.workspace.sidebar_visible))
        .unwrap();
    // Narrowing the window crowds the panes; the sidebar makes room.
    cx.simulate_resize(gpui::size(px(1000.), px(800.)));
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| assert!(!shell.workspace.sidebar_visible))
        .unwrap();
    // Widening it again brings the sidebar back.
    cx.simulate_resize(gpui::size(px(1400.), px(800.)));
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| assert!(shell.workspace.sidebar_visible))
        .unwrap();
    // Shown by hand while crowded, it stays until the room changes again.
    cx.simulate_resize(gpui::size(px(1000.), px(800.)));
    cx.run_until_parked();
    cx.simulate_keystrokes("secondary-b");
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| assert!(shell.workspace.sidebar_visible))
        .unwrap();
    // Hidden by hand, it stays hidden however wide the window grows.
    cx.simulate_keystrokes("secondary-b");
    cx.simulate_resize(gpui::size(px(1600.), px(800.)));
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| assert!(!shell.workspace.sidebar_visible))
        .unwrap();
}

#[gpui::test]
fn reordered_tabs_slide_into_place_instead_of_jumping(cx: &mut TestAppContext) {
    use gpui::{Modifiers, point};
    let (window, a, b) = same_project_window(cx);
    let cx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    cx.run_until_parked();
    let first = cx
        .debug_bounds(Box::leak(format!("title-tab-{a}").into_boxed_str()))
        .unwrap();
    let second = cx
        .debug_bounds(Box::leak(format!("title-tab-{b}").into_boxed_str()))
        .unwrap();
    let slot = (second.left() - first.left()).as_f32();
    let grab = first.center();
    cx.simulate_mouse_down(grab, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        grab + point(px(slot * 0.8), px(0.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| {
            // The neighbour makes room by easing toward the vacated slot.
            let shift = shell.tab_shifts[&TitleTab::Session(b)];
            assert_eq!(shift.to, -slot);
            assert!(shift.at(shift.start).0 == 0.);
            let (later, _) = shift.at(shift.start + std::time::Duration::from_millis(80));
            assert!(later < 0. && later > -slot);
        })
        .unwrap();
    cx.simulate_mouse_up(
        grab + point(px(slot * 0.8), px(0.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    window
        .update(cx, |shell, _, _| {
            assert_eq!(shell.workspace.tabs, [b, a]);
            // Released short of its slot, the tab glides the rest of
            // the way rather than snapping there.
            let shift = shell.tab_shifts[&TitleTab::Session(a)];
            assert!((shift.from - (-slot * 0.2)).abs() < 2., "{}", shift.from);
            assert_eq!(shift.to, 0.);
            let end = shift.start + std::time::Duration::from_millis(200);
            assert_eq!(shift.at(end), (0., true));
        })
        .unwrap();
}
