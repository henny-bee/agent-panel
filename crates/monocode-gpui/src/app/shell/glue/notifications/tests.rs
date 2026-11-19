mod whats_new;
use super::*;
use gpui::{TestAppContext, VisualTestContext};

#[gpui::test]
fn error_cards_share_top_right_stack_and_dismiss_independently(cx: &mut TestAppContext) {
    let (window, _, _) = crate::app::shell::tests::window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let body = visual.debug_bounds("main-pane-frame").unwrap();
    window
        .update(visual, |shell, _, cx| {
            shell.error = Some("Could not save workspace".into());
            shell
                .notification_state
                .file_error
                .update(cx, |notice, cx| {
                    notice.set_message(Some("Could not rename file".into()), cx);
                });
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    let app = visual.debug_bounds("app-error").unwrap();
    let file = visual.debug_bounds("file-action-error").unwrap();
    let viewport = window.update(visual, |_, w, _| w.viewport_size()).unwrap();
    assert_eq!(app.top(), px(12.));
    assert_eq!(app.right(), viewport.width - px(12.));
    assert_eq!(app.size.width, px(360.));
    assert_eq!(file.size.width, app.size.width);
    assert_eq!(file.right(), app.right());
    assert_eq!(file.top(), app.bottom() + px(8.));
    assert_eq!(visual.debug_bounds("main-pane-frame").unwrap(), body);

    let close = visual.debug_bounds("dismiss-error").unwrap();
    visual.simulate_click(close.center(), Default::default());
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("app-error").is_none());
    assert_eq!(
        visual.debug_bounds("file-action-error").unwrap().top(),
        px(12.)
    );

    visual.simulate_resize(gpui::size(px(300.), px(800.)));
    visual.run_until_parked();
    visual.refresh().unwrap();
    let file = visual.debug_bounds("file-action-error").unwrap();
    assert_eq!(file.left(), px(12.));
    assert_eq!(file.right(), px(288.));
    let close = visual.debug_bounds("dismiss-file-action-error").unwrap();
    visual.simulate_click(close.center(), Default::default());
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("file-action-error").is_none());
    window
        .update(visual, |shell, _, cx| {
            assert!(shell.error.is_none());
            assert!(!shell.notification_state.file_error.read(cx).is_visible());
        })
        .unwrap();
}

#[gpui::test]
fn dismissing_installed_update_does_not_open_whats_new(cx: &mut TestAppContext) {
    let (window, _, _) = crate::app::shell::tests::window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    window
        .update(visual, |shell, _, cx| {
            shell.notification_state.installed.update(cx, |notice, cx| {
                notice.set_version(Some("1.2.3".into()), cx)
            });
            shell.invalidate_chrome(cx);
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    let close = visual.debug_bounds("dismiss-installed-update").unwrap();
    visual.simulate_click(close.center(), Default::default());
    visual.run_until_parked();
    window
        .update(visual, |shell, _, cx| {
            assert!(!shell.notification_state.installed.read(cx).is_visible());
            assert!(shell.notification_state.whats_new.is_none());
        })
        .unwrap();
}
