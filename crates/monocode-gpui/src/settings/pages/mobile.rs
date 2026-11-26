//! Port of src/features/mobile/ui/MobileSettings.tsx and PairPhoneDialog.tsx:
//! the phones paired with this desktop, and the one-use code a new one scans.
mod view;
use crate::{
    settings::{Section, SettingsEvent, SettingsView},
    ui::{
        input::{self, Input},
        theme::*,
        widgets::{icon_action, note, red, secondary_button},
    },
};
use gpui::{
    Animation, AnimationExt, AnyElement, ClipboardItem, Context, Div, Entity, FontWeight,
    HighlightStyle, MouseButton, Role, SharedString, Stateful, StyledText, Subscription, Task,
    Window, canvas, div, prelude::*, px, relative, rgb,
};
use monocode_core::integrations::mobile::Status;
use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Tailwind's amber-500 and emerald-500.
const AMBER: u32 = 0xf59e0b;
const EMERALD: u32 = 0x10b981;
/// The public TestFlight invite for the iPhone beta.
const TESTFLIGHT_URL: &str = "https://testflight.apple.com/join/xdwX38bB";
/// How long Pair phone waits for the desktop to listen and make a code.
const PAIRING_WAIT: Duration = Duration::from_secs(15);
/// PairPhoneDialog.tsx's LINEUP: mascots fanning out from the middle,
/// nearest first, each side in turn, in TAB_GROUP_COLORS after the wash.
const LINEUP: [(&str, u32); 9] = [
    ("robot", 0x459bf7),
    ("cat", 0xea603e),
    ("ghost", 0xf4c025),
    ("frog", 0x39c66d),
    ("invader", 0xe25a9e),
    ("crab", 0xb069d3),
    ("dino", 0x37beb3),
    ("mushroom", 0xef8539),
    ("rocket", 0x459bf7),
];
/// Modal.tsx's medium, fitViewport panel and PairPhoneDialog.tsx's QR tile.
const DIALOG_WIDTH: f32 = 560.;
const DIALOG_RADIUS: f32 = 16.;
const QR_SIDE: f32 = 384.;
const QR_PADDING: f32 = 14.;

/// What the Mobile page remembers between frames.
#[derive(Default)]
pub(in crate::settings) struct MobileState {
    /// Device ids already paired when the visible code was made.
    known: Option<HashSet<String>>,
    /// Since Pair phone was pressed, until its code arrives.
    pairing: Option<Instant>,
    notice: Option<String>,
    /// The device whose removal is waiting for confirmation.
    removing: Option<String>,
    copied: bool,
    copied_reset: Option<Task<()>>,
    /// "Connect away from home" is open.
    away: bool,
    remote: Option<Entity<Input>>,
    qr: Option<(String, Arc<Qr>)>,
    poll: Option<Task<()>>,
    _remote_events: Vec<Subscription>,
}

/// A QR code's modules, row by row, without the quiet zone.
#[derive(Debug, PartialEq)]
pub(super) struct Qr {
    width: usize,
    dark: Vec<bool>,
}

/// The QR code phones scan; low error correction keeps the modules large.
pub(super) fn qr(text: &str) -> Option<Qr> {
    let code =
        qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::L).ok()?;
    Some(Qr {
        width: code.width(),
        dark: code
            .to_colors()
            .into_iter()
            .map(|color| color == qrcode::Color::Dark)
            .collect(),
    })
}

/// PairPhoneDialog.tsx's status line.
pub(super) fn countdown(left: u64) -> String {
    if left == 0 {
        "Code expired".into()
    } else {
        format!("Expires in {}:{:02}", left / 60, left % 60)
    }
}

/// Enough of a fingerprint to compare by eye.
pub(super) fn short_fingerprint(fingerprint: &str) -> String {
    let short: String = fingerprint.chars().take(16).collect();
    if short.len() < fingerprint.len() {
        format!("{short}…")
    } else {
        short
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl SettingsView {
    /// Status and the router change on their own, and the code counts down:
    /// read them every second while the page is open.
    pub(in crate::settings) fn watch_mobile(&mut self, cx: &mut Context<Self>) {
        if self.section != Section::Connections {
            self.mobile.poll = None;
            return;
        }
        cx.emit(SettingsEvent::MobileRefresh);
        self.mobile.poll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let alive = this.update(cx, |this, cx| {
                    if this
                        .mobile
                        .pairing
                        .is_some_and(|since| since.elapsed() > PAIRING_WAIT)
                    {
                        this.mobile.pairing = None;
                        this.mobile.known = None;
                        cx.notify();
                    }
                    if this.ctx.mobile_invite.is_some() {
                        cx.notify();
                    }
                    cx.emit(SettingsEvent::MobileRefresh);
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
    }

    /// Leaving the page closes its code, as unmounting MobileSettings did.
    pub(in crate::settings) fn leave_mobile(&mut self, cx: &mut Context<Self>) {
        if self.section != Section::Connections {
            return;
        }
        if self.ctx.mobile_invite.is_some() {
            cx.emit(SettingsEvent::MobileCancelInvite);
        }
        self.mobile.known = None;
        self.mobile.pairing = None;
        self.mobile.removing = None;
        self.mobile.notice = None;
    }

    /// Escape closes the pairing code, then a pending removal.
    pub(in crate::settings) fn dismiss_mobile(&mut self, cx: &mut Context<Self>) -> bool {
        if self.section != Section::Connections {
            return false;
        }
        if self.ctx.mobile_invite.is_some() {
            self.close_invite(cx);
            return true;
        }
        if self.mobile.removing.take().is_some() {
            cx.notify();
            return true;
        }
        false
    }

    /// A phone that appears while the code is showing just paired with it.
    pub(in crate::settings) fn mobile_context_changed(&mut self, cx: &mut Context<Self>) {
        let showing = self.ctx.mobile_invite.is_some();
        if showing {
            self.mobile.pairing = None;
        } else if self.mobile.pairing.is_none() {
            self.mobile.known = None;
        }
        if self
            .ctx
            .mobile
            .as_ref()
            .is_some_and(|status| status.error.is_some())
        {
            self.mobile.pairing = None;
        }
        let paired = showing
            .then(|| {
                let known = self.mobile.known.as_ref()?;
                self.ctx
                    .mobile
                    .as_ref()?
                    .devices
                    .iter()
                    .find(|device| !known.contains(&device.id))
            })
            .flatten();
        if let Some(device) = paired {
            self.mobile.notice = Some(format!("{} is paired.", device.name));
            self.mobile.known = None;
            cx.emit(SettingsEvent::MobileCancelInvite);
        }
    }

    fn pair_phone(&mut self, cx: &mut Context<Self>) {
        let Some(status) = &self.ctx.mobile else {
            return;
        };
        self.mobile.known = Some(status.devices.iter().map(|d| d.id.clone()).collect());
        self.mobile.notice = None;
        self.mobile.pairing = Some(Instant::now());
        cx.emit(SettingsEvent::MobilePair);
        cx.notify();
    }

    /// The desktop only listens while a phone is paired or being paired.
    fn close_invite(&mut self, cx: &mut Context<Self>) {
        self.mobile.known = None;
        self.mobile.pairing = None;
        cx.emit(SettingsEvent::MobileCancelInvite);
        if let Some(status) = &self.ctx.mobile
            && status.devices.is_empty()
        {
            cx.emit(SettingsEvent::MobileConfigure {
                enabled: false,
                open_router: status.open_router,
                remote_address: status.remote_address.clone(),
            });
        }
        cx.notify();
    }

    fn configure_mobile(
        &mut self,
        enabled: bool,
        open_router: bool,
        remote_address: String,
        cx: &mut Context<Self>,
    ) {
        self.mobile.notice = None;
        cx.emit(SettingsEvent::MobileConfigure {
            enabled,
            open_router,
            remote_address,
        });
        cx.notify();
    }

    fn save_remote_address(&mut self, cx: &mut Context<Self>) {
        let (Some(status), Some(field)) = (&self.ctx.mobile, &self.mobile.remote) else {
            return;
        };
        let remote = field.read(cx).content.trim().to_owned();
        if remote == status.remote_address {
            return;
        }
        let (enabled, open_router) = (status.enabled, status.open_router);
        self.configure_mobile(enabled, open_router, remote, cx);
    }

    fn remote_field(&mut self, status: &Status, cx: &mut Context<Self>) -> Entity<Input> {
        if let Some(field) = &self.mobile.remote {
            return field.clone();
        }
        let initial = status.remote_address.clone();
        let field = cx.new(|cx| {
            let mut input = Input::field(
                "Remote address",
                "Public hostname or VPN IP, optionally :port",
                content(0.35),
                cx,
            );
            input.set_font_size(12.);
            input.set_text(initial, cx);
            input
        });
        self.mobile._remote_events = vec![
            cx.subscribe(&field, |_, _, _: &input::Changed, cx| cx.notify()),
            cx.subscribe(&field, |this, _, _: &input::Submitted, cx| {
                this.save_remote_address(cx)
            }),
        ];
        self.mobile.remote = Some(field.clone());
        field
    }

    fn copy_invite(&mut self, url: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(url));
        self.mobile.copied = true;
        self.mobile.copied_reset = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.mobile.copied = false;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn invite_qr(&mut self, url: &str) -> Option<Arc<Qr>> {
        if let Some((cached, qr)) = &self.mobile.qr
            && cached == url
        {
            return Some(qr.clone());
        }
        let qr = Arc::new(qr(url)?);
        self.mobile.qr = Some((url.to_owned(), qr.clone()));
        Some(qr)
    }

    /// PairPhoneDialog.tsx, presented by Shell alongside the other window
    /// overlays so its scrim and centering cover the entire viewport.
    pub(crate) fn mobile_overlay_open(&self) -> bool {
        self.section == Section::Connections && self.ctx.mobile_invite.is_some()
    }
}

/// PairPhoneDialog's compact controls: an emerald copied check stays green
/// while hovered; the remaining icons brighten with the button.
fn pairing_icon_action(
    id: &'static str,
    label: &'static str,
    path: &'static str,
    copied: bool,
    disabled: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(Role::Button)
        .aria_label(label)
        .group("pairing-icon-action")
        .size(px(28.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .when(disabled, |d| d.opacity(0.4))
        .when(!disabled, |d| {
            d.cursor_pointer().hover(|s| s.bg(content(0.08)))
        })
        .child(if copied {
            // The tick grows in as the copy lands, like the Inbox's Copy link.
            icon(path, 14.)
                .text_color(rgb(EMERALD))
                .with_animation(
                    "mobile-copied-tick",
                    Animation::new(Duration::from_millis(260))
                        .with_easing(|t| 1. - (1. - t).powi(3)),
                    |svg, t| svg.size(px(14. * (0.5 + 0.5 * t))).opacity(t),
                )
                .into_any_element()
        } else {
            icon(path, 14.)
                .text_color(content(0.45))
                .when(!disabled, |i| {
                    i.group_hover("pairing-icon-action", |s| s.text_color(content(1.)))
                })
                .into_any_element()
        })
}

/// Modal.tsx's stationary GlassBackdrop, below the scrolling content.
fn pairing_backdrop(classic: bool) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            // Linux has no backdrop blur, so the fill stays opaque there.
            let opaque =
                content(1.).l < 0.5 || cfg!(not(any(target_os = "macos", target_os = "windows")));
            let fill = if classic || opaque {
                let mut fill = shell_fill();
                fill.a = if opaque { 1. } else { 0.55 };
                fill
            } else {
                content(0.02)
            };
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            let quad = gpui::fill(bounds, fill)
                .border_color(gpui::backdrop_blur_marker(24. * window.scale_factor()));
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            let quad = gpui::fill(bounds, fill);
            window.paint_quad(quad.corner_radii(px(DIALOG_RADIUS)));
        },
    )
    .absolute()
    .size_full()
}

/// A module edge in physical pixels. Snapping each edge independently fills
/// the image area, like React's pixelated QR image, without shaving an entire
/// logical pixel from every module on Retina displays.
fn qr_edge(index: usize, modules: usize, side: f32, scale: f32) -> f32 {
    (index as f32 * side * scale / modules as f32).round() / scale
}

/// React's expired image has opacity-15 and blur-[2px]. The QR ink is already
/// dimmed; blur that image rectangle before painting the New code button.
fn expired_qr_blur() -> impl IntoElement {
    canvas(
        |_, _, _| {},
        |bounds, _, window, _| {
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                // A fully transparent fill is optimized out by GPUI before
                // reaching Metal. This invisible white tint retains the
                // marker without changing the dimmed QR's appearance.
                let mut tint: gpui::Hsla = rgb(0xffffff).into();
                tint.a = 0.000001;
                window.paint_quad(
                    gpui::fill(bounds, tint)
                        .border_color(gpui::backdrop_blur_marker(2. * window.scale_factor())),
                );
            });
            // Other renderers keep the dimmed expired image and the usable
            // New code control; they have no backdrop-blur primitive.
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            let _ = (bounds, window);
        },
    )
    .absolute()
    .top(px(QR_PADDING))
    .left(px(QR_PADDING))
    .right(px(QR_PADDING))
    .bottom(px(QR_PADDING))
}

/// Dark modules on white with the same two-module quiet zone as React's
/// QRCode.toDataURL. All edges land on device pixels so cameras see crisp ink.
fn qr_canvas(qr: Arc<Qr>) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let modules = qr.width + 4;
            let side = bounds.size.width.min(bounds.size.height).as_f32();
            let scale = window.scale_factor();
            let origin = gpui::point(
                px((bounds.origin.x.as_f32() * scale).round() / scale),
                px((bounds.origin.y.as_f32() * scale).round() / scale),
            );
            for (y, row) in qr.dark.chunks(qr.width).enumerate() {
                let top = qr_edge(y + 2, modules, side, scale);
                let bottom = qr_edge(y + 3, modules, side, scale);
                let mut x = 0;
                while x < row.len() {
                    if !row[x] {
                        x += 1;
                        continue;
                    }
                    let start = x;
                    while x < row.len() && row[x] {
                        x += 1;
                    }
                    let left = qr_edge(start + 2, modules, side, scale);
                    let right = qr_edge(x + 2, modules, side, scale);
                    window.paint_quad(gpui::fill(
                        gpui::Bounds::new(
                            origin + gpui::point(px(left), px(top)),
                            gpui::size(px(right - left), px(bottom - top)),
                        ),
                        rgb(0x000000),
                    ));
                }
            }
        },
    )
    .size_full()
}

/// The mascot crew, gathered round a glow, standing in for an illustration.
fn mascot_lineup() -> AnyElement {
    div()
        .relative()
        .h(px(96.))
        .flex_shrink_0()
        .overflow_hidden()
        .child(
            div()
                .absolute()
                .bottom(px(14.))
                .left(relative(0.5))
                .ml(px(-112.))
                .h(px(12.))
                .w(px(224.))
                .child(mascot_glow()),
        )
        // The farthest first, so the nearest paint over them.
        .children(LINEUP.iter().enumerate().rev().map(|(i, &(name, color))| {
            let step = i.div_ceil(2) as f32;
            let side = if i % 2 == 1 { -1. } else { 1. };
            let size = 40. - step * 6.;
            div()
                .absolute()
                .left(relative(0.5))
                .ml(px(side * step * 42. - size / 2.))
                .bottom(px(18. + step * 2.))
                .size(px(size))
                .opacity(1. - step * 0.17)
                .child(crate::ui::avatar::avatar(name, color, "idle", size, true))
        }))
        .into_any_element()
}

/// PairPhoneDialog's elliptical closest-side radial gradient, fading from
/// twelve-percent content ink at its center to transparent at its edge.
fn mascot_glow() -> impl IntoElement {
    canvas(
        |_, _, _| {},
        |bounds, _, window, _| {
            const STEPS: usize = 24;
            let center = bounds.center();
            for layer in 0..STEPS {
                let radius = 1. - layer as f32 / STEPS as f32;
                let rx = bounds.size.width.as_f32() / 2. * radius;
                let ry = bounds.size.height.as_f32() / 2. * radius;
                let mut path = gpui::PathBuilder::fill();
                for segment in 0..=64 {
                    let angle = std::f32::consts::TAU * segment as f32 / 64.;
                    let point = center + gpui::point(px(rx * angle.cos()), px(ry * angle.sin()));
                    if segment == 0 {
                        path.move_to(point);
                    } else {
                        path.line_to(point);
                    }
                }
                path.close();
                if let Ok(path) = path.build() {
                    let previous_alpha = 0.12 * layer as f32 / STEPS as f32;
                    window.paint_path(path, content((0.12 / STEPS as f32) / (1. - previous_alpha)));
                }
            }
        },
    )
    .size_full()
}

#[cfg(test)]
mod tests;
