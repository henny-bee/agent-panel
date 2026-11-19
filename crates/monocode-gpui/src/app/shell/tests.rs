mod basics;
mod classic_surface;
mod dock;
mod image_lightbox;
mod new_chat_choice;
mod pane_motion;
mod panes;
mod project_appearance;
mod sessions;
mod shortcuts;
mod sidebar_layout;
mod tab_lifecycle;
use super::*;
use gpui::{EntityInputHandler, TestAppContext, WindowHandle};
use monocode_core::store::Store;
fn same_project_window(cx: &mut TestAppContext) -> (WindowHandle<Shell>, i64, i64) {
    let (window, a, b) = window(cx);
    window
        .update(cx, |shell, _, cx| {
            shell.sessions[&a].update(cx, |s, _| s.summary.cwd = "/work/monocode".into());
            shell.invalidate_chrome(cx);
        })
        .unwrap();
    cx.run_until_parked();
    (window, a, b)
}
pub(super) fn window(cx: &mut TestAppContext) -> (WindowHandle<Shell>, i64, i64) {
    let mut store = Store::open(std::path::Path::new(":memory:")).unwrap();
    let mut a = store.create("codex", "/work/another-project").unwrap();
    let b = store.create("claude", "/work/monocode").unwrap();
    store
        .write_rows(
            a.id,
            0,
            &[monocode_core::session::transcript::Row {
                role: "assistant".into(),
                text: "History loaded when the adjacent tab becomes active".into(),
                first: true,
                sent_at: None,
            }],
        )
        .unwrap();
    a.has_messages = true;
    let ids = (a.id, b.id);
    // Layout tests start from the rounded frame and opt into boxy.
    let mut state = Workspace {
        new_layout_boxy: false,
        ..Default::default()
    };
    state.add_project("/work/another-project");
    state.add_project("/work/monocode");
    state.open(a.id);
    state.open(b.id);
    let saved = serde_json::to_string(&state).unwrap();
    let store = Arc::new(Mutex::new(store));
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| {
                let mut shell =
                    Shell::new(store, vec![b, a], "/work/monocode".into(), Some(saved), cx);
                // This shared fixture intentionally exposes both entities to
                // interaction tests; production restoration stays lazy.
                shell.ensure_session(ids.0, cx);
                shell
            })
        })
        .unwrap()
    });
    cx.refresh().unwrap();
    cx.run_until_parked();
    (window, ids.0, ids.1)
}
fn assert_closed_header_stays_hidden(cx: &mut TestAppContext, file: bool) {
    use std::time::Duration;
    cx.update(|cx| cx.set_reduce_motion(true));
    let (window, session, other) = same_project_window(cx);
    window
        .update(cx, |shell, w, cx| {
            shell.open(session, w, cx);
            if file {
                shell.open_review(
                    crate::git::review::ReviewTab::File {
                        cwd: "/work/monocode".into(),
                        path: "/tmp/header-close.rs".into(),
                    },
                    w,
                    cx,
                );
            } else {
                shell.workspace.place(other, session, Edge::Right);
                shell.open(session, w, cx);
            }
        })
        .unwrap();
    cx.run_until_parked();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    let header = Box::leak(format!("split-header-{session}").into_boxed_str());
    assert!(visual.debug_bounds(header).is_some());
    visual.update(|_, cx| cx.set_reduce_motion(false));
    window
        .update(visual, |shell, w, cx| {
            if file {
                shell.close_review(session, cx);
            } else {
                shell.close_pane(other, w, cx);
            }
        })
        .unwrap();

    // Inspect the exact frame that consumes the finished animation. A
    // later refresh would hide a one-frame return of the full header.
    visual
        .update_window(window.into(), |_, w, cx| {
            let root = w.root::<Shell>().flatten().unwrap();
            root.update(cx, |shell, _| {
                if file {
                    shell
                        .review_slide
                        .as_ref()
                        .unwrap()
                        .1
                        .elapsed(Duration::from_secs(1));
                } else {
                    shell
                        .pane_motion
                        .as_ref()
                        .unwrap()
                        .slide
                        .elapsed(Duration::from_secs(1));
                }
            });
            w.refresh();
            w.draw(cx).clear(cx);
        })
        .unwrap();
    assert!(
        visual.debug_bounds(header).is_none(),
        "the settled frame must already omit the disappearing header"
    );
    let transcript = window
        .update(visual, |shell, _, cx| {
            shell.sessions[&session].read(cx).transcript.clone()
        })
        .unwrap();
    let settled = transcript.read_with(visual, |t, _| t.list.viewport_bounds());
    visual
        .update_window(window.into(), |_, w, cx| {
            w.refresh();
            w.draw(cx).clear(cx);
        })
        .unwrap();
    assert!(visual.debug_bounds(header).is_none());
    assert_eq!(
        transcript.read_with(visual, |t, _| t.list.viewport_bounds()),
        settled,
        "the transcript must not jump after the closing animation settles"
    );
}

#[cfg(all(test, target_os = "macos"))]
mod metal;
