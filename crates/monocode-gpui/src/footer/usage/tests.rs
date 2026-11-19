use super::*;
#[gpui::test]
fn details_open_with_keyboard_escape_restores_focus_and_provider_caches_stay_separate(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(bind);
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| UsageFooter::new(Kind::Codex, cx))
        })
        .unwrap()
    });
    window
        .update(cx, |footer, w, cx| {
            footer.cache[0].complete(Ok(Snapshot {
                windows: vec![usage::Window {
                    used_percent: 28.,
                    minutes: 300,
                    resets_at: None,
                }],
                updated_at: usage::now_ms(),
                ..Default::default()
            }));
            w.focus(&footer.trigger, cx);
        })
        .unwrap();
    cx.refresh().unwrap();
    cx.simulate_keystrokes(window.into(), "enter");
    cx.run_until_parked();
    window
        .update(cx, |footer, w, cx| {
            assert!(footer.open);
            assert!(footer.details.is_focused(w));
            cx.notify();
        })
        .unwrap();
    cx.simulate_keystrokes(window.into(), "escape");
    cx.run_until_parked();
    window
        .update(cx, |footer, w, cx| {
            assert!(!footer.open);
            assert!(footer.trigger.is_focused(w));
            footer.provider(Kind::Claude, cx);
            assert!(footer.cache[index(footer.kind)].snapshot.is_none());
            // A late Codex response belongs to Codex, while Claude stays in view.
            footer.cache[0].complete(Ok(Snapshot {
                windows: vec![usage::Window {
                    used_percent: 42.,
                    minutes: 300,
                    resets_at: None,
                }],
                updated_at: usage::now_ms(),
                ..Default::default()
            }));
            assert!(footer.cache[index(footer.kind)].snapshot.is_none());
            footer.provider(Kind::Codex, cx);
            assert_eq!(
                footer.cache[0].snapshot.as_ref().unwrap().windows[0].used_percent,
                42.
            );
        })
        .unwrap();
}
#[test]
fn failed_refresh_keeps_snapshot_but_sign_out_clears_it() {
    let mut state = CachedUsage {
        snapshot: Some(Snapshot::default()),
        fetching: true,
        ..Default::default()
    };
    state.complete(Err(FetchError {
        message: "Offline".into(),
        unavailable: false,
    }));
    assert!(state.snapshot.is_some());
    assert!(!state.fetching);
    state.complete(Err(FetchError {
        message: "Signed out".into(),
        unavailable: true,
    }));
    assert!(state.snapshot.is_none());
    state.complete(Ok(Snapshot::default()));
    assert!(state.error.is_none());
}
