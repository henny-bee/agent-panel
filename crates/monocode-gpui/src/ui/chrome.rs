//! Titlebar geometry and drag area shared by every full-window view.
use crate::ui::popover::AnchorSlot;
use gpui::{MouseButton, Window, WindowControlArea, div, prelude::*};

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) const CONTROLS_WIDTH: f32 = 106.;

/// Windows has no caption of its own (`appears_transparent`), so the app
/// draws the window controls. Test windows keep the captionless geometry
/// that Linux tests get from server decorations.
#[cfg(target_os = "windows")]
pub(crate) const WINDOWS_CONTROLS: bool = cfg!(not(test));

pub(crate) fn client_controls_width(window: &Window) -> f32 {
    #[cfg(target_os = "linux")]
    if !window.is_fullscreen()
        && matches!(
            window.window_decorations(),
            gpui::Decorations::Client { .. }
        )
    {
        return CONTROLS_WIDTH;
    }
    #[cfg(target_os = "windows")]
    if WINDOWS_CONTROLS && !window.is_fullscreen() {
        return CONTROLS_WIDTH;
    }
    let _ = window;
    0.
}

/// Client-drawn window controls that hold the corner's right end, inside the
/// bar: Windows only. Client-decorated Linux shifts the whole bar left of its
/// controls instead (`titlebar_controls_space`).
pub(crate) fn corner_controls_width(window: &Window) -> f32 {
    if cfg!(target_os = "windows") {
        client_controls_width(window)
    } else {
        0.
    }
}

/// macOS must receive button clicks as app content. Only blank titlebar
/// regions initiate native dragging or the system double-click action.
/// Client-decorated Linux windows also need explicit native drag requests.
/// Windows hit-tests the drag area as the caption, which drags, snaps and
/// maximizes on double-click natively.
pub(crate) fn titlebar_drag_area() -> gpui::Div {
    let bounds = AnchorSlot::default();
    div()
        .relative()
        .h_full()
        .child(bounds.probe())
        .window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            if (cfg!(target_os = "macos")
                || (cfg!(target_os = "linux")
                    && matches!(
                        window.window_decorations(),
                        gpui::Decorations::Client { .. }
                    )))
                && bounds.get().is_some_and(|b| b.contains(&event.position))
            {
                // Check the measured spacer as well as GPUI's cached hit test:
                // native gestures must never start over a neighboring control.
                cx.stop_propagation();
                if event.click_count == 2 {
                    window.titlebar_double_click();
                } else if event.click_count == 1 {
                    window.start_window_move();
                }
            }
        })
}
