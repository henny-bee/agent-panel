use super::*;
use crate::{settings::SettingsContext, ui::preferences::Preferences};
use gpui::{
    Bounds, Render, TestAppContext, VisualTestContext, WindowBounds, WindowHandle, WindowOptions,
    point, size,
};

/// The same placement as Shell: the settings pane is a child, while the
/// pairing overlay belongs to the window and observes the settings entity.
struct Preview {
    settings: Entity<SettingsView>,
    cancellations: usize,
    _changes: Subscription,
    _events: Subscription,
}

impl Render for Preview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let overlay = self
            .settings
            .update(cx, |view, cx| view.mobile_overlay(window, cx));
        div()
            .relative()
            .size_full()
            .child(self.settings.clone())
            .children(overlay)
    }
}

fn preview(cx: &mut TestAppContext) -> WindowHandle<Preview> {
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(1200.), px(900.)),
                ))),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|cx| {
                    let settings = cx.new(|cx| {
                        let mut view = SettingsView::new(
                            Section::Connections,
                            Preferences {
                                notifications: false,
                                ..Default::default()
                            },
                            SettingsContext {
                                mobile: Some(Status::default()),
                                mobile_invite: Some(monocode_core::integrations::mobile::Invite {
                                    url: format!("monocode://pair?i={}", "A".repeat(600)),
                                    expires: now() + 900,
                                }),
                                ..Default::default()
                            },
                            cx,
                        );
                        view.discovery_task = None;
                        view.discovery_loading = false;
                        view.mobile.poll = None;
                        view
                    });
                    let changes = cx.observe(&settings, |_, _, cx| cx.notify());
                    let events = cx.subscribe(&settings, |this: &mut Preview, _, event, cx| {
                        if matches!(event, SettingsEvent::MobileCancelInvite) {
                            this.cancellations += 1;
                            this.settings.update(cx, |view, cx| {
                                view.ctx.mobile_invite = None;
                                cx.notify();
                            });
                            cx.notify();
                        }
                    });
                    Preview {
                        settings,
                        cancellations: 0,
                        _changes: changes,
                        _events: events,
                    }
                })
            },
        )
        .unwrap()
    });
    cx.refresh().unwrap();
    cx.run_until_parked();
    window
}

#[gpui::test]
fn pairing_dialog_matches_react_dimensions_and_scrolls_inside_small_windows(
    cx: &mut TestAppContext,
) {
    let window = preview(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let overlay = visual.debug_bounds("mobile-pair-overlay").unwrap();
    let dialog = visual.debug_bounds("mobile-pair-dialog").unwrap();
    let code = visual.debug_bounds("mobile-pair-code").unwrap();
    assert_eq!(overlay.size, size(px(1200.), px(900.)));
    assert_eq!(dialog.size.width, px(560.));
    assert!(dialog.size.height > px(620.) && dialog.size.height < px(720.));
    assert_eq!(dialog.center(), overlay.center());
    assert_eq!(code.size, size(px(384.), px(384.)));
    assert_eq!(code.center().x, dialog.center().x);

    visual.simulate_resize(size(px(420.), px(480.)));
    visual.run_until_parked();
    let dialog = visual.debug_bounds("mobile-pair-dialog").unwrap();
    let scroll = visual.debug_bounds("mobile-pair-scroll").unwrap();
    let code = visual.debug_bounds("mobile-pair-code").unwrap();
    assert_eq!(dialog.size.width, px(396.));
    assert!(dialog.top() >= px(16.) && dialog.bottom() <= px(464.));
    assert!(scroll.size.height <= dialog.size.height);
    assert_eq!(code.size, size(px(346.), px(346.)));
    assert!(code.left() > dialog.left() && code.right() < dialog.right());
}

#[gpui::test]
fn pairing_panel_keeps_inside_clicks_and_dismisses_on_the_window_scrim(cx: &mut TestAppContext) {
    let window = preview(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let code = visual.debug_bounds("mobile-pair-code").unwrap();
    visual.simulate_click(code.center(), Default::default());
    visual.run_until_parked();
    window
        .update(visual, |view, _, _| assert_eq!(view.cancellations, 0))
        .unwrap();
    assert!(visual.debug_bounds("mobile-pair-dialog").is_some());
    visual.simulate_click(point(px(8.), px(8.)), Default::default());
    visual.run_until_parked();
    window
        .update(visual, |view, _, _| assert_eq!(view.cancellations, 1))
        .unwrap();
    assert!(visual.debug_bounds("mobile-pair-dialog").is_none());
}

#[test]
fn dense_qr_codes_fill_the_image_with_crisp_device_pixel_edges() {
    let side = QR_SIDE - QR_PADDING * 2.;
    // A dense code used to lose almost a third of its side because each
    // module was rounded down to a whole logical pixel.
    let modules = 153;
    for scale in [1., 1.5, 2.] {
        assert_eq!(qr_edge(0, modules, side, scale), 0.);
        assert_eq!(qr_edge(modules, modules, side, scale), side);
        let left = qr_edge(2, modules, side, scale);
        let right = qr_edge(modules - 2, modules, side, scale);
        assert!(right - left > side * 0.96);
        for index in 0..=modules {
            let edge = qr_edge(index, modules, side, scale) * scale;
            assert!((edge - edge.round()).abs() < 0.001);
            if index > 0 {
                assert!(
                    qr_edge(index, modules, side, scale) > qr_edge(index - 1, modules, side, scale)
                );
            }
        }
    }
}

#[test]
fn countdown_reads_minutes_and_padded_seconds() {
    assert_eq!(countdown(0), "Code expired");
    assert_eq!(countdown(9), "Expires in 0:09");
    assert_eq!(countdown(14 * 60 + 32), "Expires in 14:32");
    assert_eq!(countdown(15 * 60), "Expires in 15:00");
}

#[test]
fn pairing_links_fit_a_scannable_code() {
    let url = format!("monocode://pair?i={}", "A".repeat(600));
    let code = qr(&url).unwrap();
    assert_eq!(code.dark.len(), code.width * code.width);
    assert!(code.dark.iter().any(|&dark| dark));
    // The three finder patterns start with a dark corner.
    assert!(code.dark[0] && code.dark[code.width - 1]);
    assert!(code.dark[code.width * (code.width - 1)]);
}

#[test]
fn fingerprints_shorten_only_when_long() {
    assert_eq!(short_fingerprint("abcd"), "abcd");
    assert_eq!(
        short_fingerprint("0123456789abcdef0123"),
        "0123456789abcdef…"
    );
}
