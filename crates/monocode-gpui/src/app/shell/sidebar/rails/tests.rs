use super::*;
use gpui::{TestAppContext, VisualTestContext};

fn bounds(cx: &mut VisualTestContext, selector: String) -> Option<gpui::Bounds<Pixels>> {
    cx.debug_bounds(Box::leak(selector.into_boxed_str()))
}
/// Rail motion runs on the wall clock, as the window's frames do.
fn settle(window: gpui::WindowHandle<Shell>, cx: &mut VisualTestContext) {
    for _ in 0..2 {
        std::thread::sleep(OPEN + Duration::from_millis(20));
        window
            .update(cx, |shell, _, cx| {
                shell.invalidate_chrome(cx);
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
    }
}

#[gpui::test]
fn operator_and_orchestrator_threads_hang_their_work_on_a_rail_that_follows_selection(
    cx: &mut TestAppContext,
) {
    let (window, _, lead) = crate::app::shell::tests::window(cx);
    let cx = &mut VisualTestContext::from_window(window.into(), cx);
    let (child, other, worker) = window
        .update(cx, |shell, _, cx| {
            let mut ids = Vec::new();
            for _ in 0..3 {
                let summary = locked(&shell.store)
                    .unwrap()
                    .create("codex", &shell.workspace.project)
                    .unwrap();
                ids.push(summary.id);
                shell.add(summary, cx);
            }
            for id in ids.iter().copied().chain([lead]) {
                shell.sessions[&id].update(cx, |s, _| s.summary.has_messages = true);
            }
            let (child, other, worker) = (ids[0], ids[1], ids[2]);
            shell.started_by.insert(child, lead);
            shell.orchestration_summaries.insert(
                other,
                monocode_core::session::orchestration::Summary {
                    live: false,
                    tasks: vec![monocode_core::session::orchestration::SummaryTask {
                        session: Some(worker),
                        title: "Write the tests".into(),
                        provider: "codex".into(),
                        model: None,
                        status: "completed".into(),
                    }],
                },
            );
            shell.selected_sessions.clear();
            shell.workspace.active = Some(lead);
            shell.invalidate_chrome(cx);
            (child, other, worker)
        })
        .unwrap();
    settle(window, cx);
    let head = bounds(cx, format!("sidebar-session-{lead}")).expect("the thread is listed");
    let nested = bounds(cx, format!("sidebar-session-{child}")).expect("its session hangs below");
    assert!(nested.top() >= head.bottom(), "under its caller");
    assert!(nested.left() > head.left(), "on the rail, inset");
    assert_eq!(nested.size.height, px(28.));
    assert!(
        bounds(cx, format!("sidebar-session-{worker}")).is_none(),
        "a worker is never listed on its own"
    );
    assert!(
        bounds(cx, format!("rail-agent-{other}-0")).is_none(),
        "an orchestrator's rail is closed while it is out of play"
    );
    // Clicking the orchestrator opens its agents and folds the other rail;
    // the click does not also land on an agent that grows in under it.
    let row = bounds(cx, format!("sidebar-session-{other}")).unwrap();
    let clicked = Instant::now();
    cx.simulate_click(row.center(), Default::default());
    cx.run_until_parked();
    window
        .read_with(cx, |shell, _| {
            assert_eq!(shell.workspace.active, Some(other));
            assert!(shell.rails.open_tasks.is_empty());
        })
        .unwrap();
    let opening = window
        .read_with(cx, |shell, _| shell.rails.motions[&other])
        .unwrap();
    // A motion that starts at the click, rather than one already settled.
    assert!(
        opening.open && opening.start >= clicked,
        "the rail grows in"
    );
    assert!(opening.value(opening.start) < 0.01);
    settle(window, cx);
    let agent = bounds(cx, format!("rail-agent-{other}-0")).expect("its agents show");
    window
        .read_with(cx, |shell, _| assert!(shell.rails.open_tasks.is_empty()))
        .unwrap();
    cx.simulate_click(agent.center(), Default::default());
    cx.run_until_parked();
    window
        .read_with(cx, |shell, _| {
            assert!(
                shell.rails.open_tasks.contains(&(other, 0)),
                "a click opens its details"
            )
        })
        .unwrap();
    settle(window, cx);
    let agent = bounds(cx, format!("rail-agent-{other}-0")).unwrap();
    cx.simulate_click(agent.center(), Default::default());
    cx.run_until_parked();
    window
        .read_with(cx, |shell, _| {
            assert!(shell.rails.open_tasks.is_empty(), "and closes them")
        })
        .unwrap();
    assert!(
        bounds(cx, format!("sidebar-session-{child}")).is_none(),
        "the operator rail folds away once its thread leaves play"
    );
}
