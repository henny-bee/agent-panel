use super::super::*;
use gpui::{
    ListOffset, Pixels, Point, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase,
    VisualTestContext, point,
};

#[gpui::test]
fn wheel_over_whats_new_leaves_the_session_behind_it_in_place(cx: &mut TestAppContext) {
    let (window, _, session) = crate::app::shell::tests::window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let transcript = window
        .update(visual, |shell, _, cx| {
            shell.sessions[&session].update(cx, |session, cx| {
                session.summary.has_messages = true;
                cx.notify();
                session.transcript.clone()
            })
        })
        .unwrap();
    transcript.update(visual, |view, cx| {
        let rows = (0..200)
            .map(|ix| monocode_core::session::transcript::Row {
                role: if ix % 2 == 0 { "user" } else { "assistant" }.into(),
                text: "A message in the history".into(),
                first: true,
                sent_at: None,
            })
            .collect();
        view.patch(0, rows, cx);
        view.list = view.list.clone().measure_all();
    });
    visual.run_until_parked();
    visual.refresh().unwrap();
    transcript.update(visual, |view, _| {
        view.list.pause_following_tail();
        view.list.scroll_to(ListOffset {
            item_ix: 100,
            offset_in_item: px(0.),
        });
    });
    visual.refresh().unwrap();

    let offset = |visual: &mut VisualTestContext| {
        transcript.read_with(visual, |view, _| {
            view.list.scroll_px_offset_for_scrollbar().y
        })
    };
    let wheel = |visual: &mut VisualTestContext, position: Point<Pixels>| {
        visual.simulate_event(ScrollWheelEvent {
            position,
            delta: ScrollDelta::Pixels(point(px(0.), px(40.))),
            modifiers: Default::default(),
            touch_phase: TouchPhase::Moved,
        });
        visual.run_until_parked();
        visual.refresh().unwrap();
    };

    let pane = visual.debug_bounds("main-pane-frame").unwrap();
    let start = offset(visual);
    wheel(visual, pane.center());
    let open_position = offset(visual);
    assert_ne!(
        open_position, start,
        "the wheel reaches an unblocked session"
    );

    window
        .update(visual, |shell, _, cx| {
            shell.open_whats_new(crate::notifications::updater::version().into(), cx);
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    // The pane center falls on the card; its corner falls on the scrim.
    wheel(visual, pane.center());
    wheel(visual, pane.origin + point(px(8.), px(8.)));
    assert_eq!(offset(visual), open_position);
}
