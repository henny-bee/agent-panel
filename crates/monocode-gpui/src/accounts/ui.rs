mod picker;
mod rows;
use super::masked::masked_text;
use super::*;
use crate::ui::{
    theme::*,
    widgets::{primary_button, secondary_button},
};
use gpui::{AnyElement, App, Div, FontWeight, MouseButton, Role, SharedString, div, px};
impl Accounts {
    fn meters(&self, kind: Kind, id: &str) -> AnyElement {
        let key = Key::new(kind, self.profile(id), None);
        let Some(snapshot) = self.usage.get(&key).and_then(|s| s.snapshot.as_ref()) else {
            return div().into_any_element();
        };
        div()
            .flex()
            .items_center()
            .gap_3()
            .children(snapshot.windows.iter().map(|w| {
                let used = if w.resets_at.is_some_and(|at| at <= usage::now_ms()) {
                    0.
                } else {
                    w.used_percent
                };
                let percent = if self.show_remaining {
                    100. - used
                } else {
                    used
                };
                div()
                    .w(px(64.))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .text_size(px(9.))
                            .text_color(content(0.4))
                            .child(w.label())
                            .child(format!("{percent:.0}%")),
                    )
                    .child(
                        div()
                            .mt_1()
                            .h(px(3.))
                            .rounded_full()
                            .bg(content(0.08))
                            .child(
                                div()
                                    .h_full()
                                    .w(px(64. * percent as f32 / 100.))
                                    .rounded_full()
                                    .bg(if used >= 100. {
                                        gpui::rgb(0xf87171).into()
                                    } else if used >= 80. {
                                        gpui::rgb(0xfbbf24).into()
                                    } else {
                                        content(0.55)
                                    }),
                            ),
                    )
            }))
            .into_any_element()
    }
    /// ProviderAccountSubtitle: "Pro · email", the email blurred until clicked.
    pub(crate) fn subtitle(&self, kind: Kind, id: &str, cx: &mut Context<Self>) -> Option<Div> {
        let identity = self.identities.get(&(kind, id.to_owned()))?;
        if identity.plan.is_none() && identity.email.is_none() {
            return None;
        }
        let key = (kind, id.to_owned());
        let masked = self.mask_emails && !self.revealed.contains(&key);
        Some(
            div()
                .flex()
                .items_baseline()
                .gap_1()
                .min_w_0()
                .children(
                    identity
                        .plan
                        .clone()
                        .map(|plan| div().flex_shrink_0().child(plan)),
                )
                .when(identity.plan.is_some() && identity.email.is_some(), |d| {
                    d.child("·")
                })
                .children(identity.email.clone().map(|email| {
                    div()
                        .id(SharedString::from(format!(
                            "account-email-{}-{id}",
                            kind.id()
                        )))
                        .role(Role::Button)
                        .aria_label(if masked { "Reveal email" } else { "Hide email" })
                        .min_w_0()
                        .relative()
                        .truncate()
                        .child(masked_text(email, masked))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.mask_emails && !this.revealed.remove(&key) {
                                this.revealed.insert(key.clone());
                            }
                            cx.stop_propagation();
                            cx.notify();
                        }))
                })),
        )
    }
}

/// Back arrow and title, as at the top of React's account popovers.
fn back_header(
    id: &'static str,
    label: &'static str,
    title: String,
    on_back: impl Fn(&mut Window, &mut App) + 'static,
) -> Div {
    div()
        .h_7()
        .flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(label)
                .size_6()
                .rounded_md()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(content(0.1)))
                .child(icon("arrow-left", 14.).text_color(content(0.45)))
                .on_click(move |_, window, cx| on_back(window, cx)),
        )
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .child(title),
        )
}

/// React's barClass, shared with the footer chip and usage cards.
pub(crate) fn bar_color(used: f64) -> gpui::Hsla {
    if used >= 90. {
        gpui::rgb(0xf87171).into()
    } else if used >= 80. {
        gpui::rgb(0xfbbf24).into()
    } else {
        content(0.45)
    }
}

fn accent(alpha: f32) -> gpui::Hsla {
    let mut color = accent_color();
    color.a = alpha;
    color
}

/// AccountStatusLabel: "● Ready" or "● Exhausted back in 31m".
pub(crate) fn status_label(status: &Status) -> Div {
    let tinted = |hex: u32, alpha: f32| {
        let mut color: gpui::Hsla = gpui::rgb(hex).into();
        color.a = alpha;
        color
    };
    let (dot, ink) = match status.tone {
        Tone::Ready => (tinted(0x34d399, 1.), content(0.6)),
        Tone::Low => (tinted(0xfbbf24, 1.), tinted(0xfbbf24, 0.9)),
        Tone::Exhausted => (tinted(0xf87171, 1.), tinted(0xf87171, 0.9)),
        Tone::Checking | Tone::Unknown => (content(0.25), content(0.35)),
    };
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .min_w_0()
        .child(div().size(px(6.)).flex_shrink_0().rounded_full().bg(dot))
        .child(
            div()
                .text_color(ink)
                .map(|d| {
                    if status.tone == Tone::Unknown {
                        d.min_w_0().truncate()
                    } else {
                        d.flex_shrink_0()
                    }
                })
                .child(status.label.clone()),
        )
        .children(status.detail.clone().map(|detail| {
            div()
                .min_w_0()
                .truncate()
                .text_color(content(0.4))
                .child(detail)
        }))
}

/// UsageMeter with React's short "5h · 3h 10m" title for account rows.
fn meter(w: &usage::Window, now: i64, show_remaining: bool) -> Div {
    let used = w.used_percent.clamp(0., 100.);
    let remaining = 100. - used;
    let shown = if show_remaining { remaining } else { used };
    let full = used >= 100. && w.resets_at.is_none_or(|at| at > now);
    let reset = match w.resets_at {
        None => w.label(),
        Some(at) if at <= now => "reset due".into(),
        Some(at) => usage::duration(at - now),
    };
    div()
        .flex_1()
        .min_w_0()
        .child(
            div()
                .flex()
                .items_baseline()
                .justify_between()
                .gap_2()
                .text_size(px(10.))
                .line_height(px(12.))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(content(0.4))
                        .child(format!("{} · {reset}", w.label())),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .map(|d| {
                            if full {
                                d.font_weight(FontWeight::MEDIUM)
                                    .text_color(gpui::rgb(0xf87171))
                            } else {
                                d.text_color(content(0.6))
                            }
                        })
                        .child(if show_remaining {
                            format!("{remaining:.0}% left")
                        } else if full {
                            "Full".into()
                        } else {
                            format!("{used:.0}%")
                        }),
                ),
        )
        .child(
            div()
                .mt(px(6.))
                .h_1()
                .rounded_full()
                .bg(content(0.1))
                .overflow_hidden()
                .child(
                    div()
                        .h_full()
                        .w(gpui::relative(shown as f32 / 100.))
                        .rounded_full()
                        .bg(bar_color(used)),
                ),
        )
}

impl Render for Accounts {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(signin) = &self.signin else {
            return div().into_any_element();
        };
        if !self.has_modal_auth() {
            return div().into_any_element();
        }
        let kind = signin.kind;
        let complete = signin.complete;
        let failed = signin.error.is_some();
        let action = signin.action;
        let title = if complete {
            format!(
                "Signed {} {}",
                if action == AuthAction::SignIn {
                    "in to"
                } else {
                    "out of"
                },
                kind.label()
            )
        } else if failed {
            "Authentication required".into()
        } else {
            "Waiting for browser…".into()
        };
        let error = signin.error.clone();
        let retry_id = signin.profile.id.clone();
        let adding = signin.adding.clone();
        if !self.dialog_presented {
            self.dialog_presented = true;
            self.previous_focus = self
                .editor
                .as_ref()
                .filter(|e| signin.adding.is_some() && e.surface == Surface::Settings)
                .and_then(|e| e.previous_focus.clone())
                .or_else(|| window.focused(cx));
            window.focus(&self.dialog_focus, cx);
        }
        div()
            .absolute()
            .inset_0()
            .occlude()
            .child(crate::ui::modal::scrim("provider-signin-scrim"))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .id("provider-signin-panel")
                    .role(Role::Dialog)
                    .aria_label("Authentication required")
                    .track_focus(&self.dialog_focus)
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                        if event.keystroke.key == "escape" {
                            this.close_auth(window, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .w(px(400.))
                    .relative()
                    .rounded(px(crate::ui::modal::RADIUS))
                    .border_1()
                    .border_color(content(crate::ui::modal::BORDER))
                    .child(crate::ui::modal::surface(crate::ui::modal::RADIUS - 1.))
                    .p_6()
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(div().w_full().flex().justify_end().child(
                        icon_button("close-provider-signin", "Close sign-in", "close").on_click(
                            cx.listener(|this, _, window, cx| {
                                this.close_auth(window, cx);
                            }),
                        ),
                    ))
                    .child(
                        div()
                            .size_16()
                            .rounded(px(16.))
                            .bg(content(0.06))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(provider_icon(kind.id(), 36.)),
                    )
                    .child(
                        div()
                            .mt(px(14.))
                            .text_size(px(15.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(title),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_size(px(11.))
                            .text_color(content(0.45))
                            .child(if complete {
                                "You can retry your last message now."
                            } else {
                                "The provider CLI opens its browser to complete sign-in."
                            }),
                    )
                    .child(
                        secondary_button(
                            "provider-auth-action",
                            if complete {
                                "Continue"
                            } else if failed {
                                "Retry sign-in"
                            } else {
                                "Waiting for browser"
                            },
                            true,
                            (!complete && !failed) || self.working,
                        )
                        .mt_4()
                        .child(if complete {
                            "Continue"
                        } else if failed {
                            "Retry sign-in"
                        } else {
                            "Waiting for browser…"
                        })
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                if complete {
                                    this.close_auth(window, cx);
                                } else if failed {
                                    this.begin_auth(kind, &retry_id, action, adding.clone(), cx);
                                }
                            },
                        )),
                    )
                    .children(error.map(|error| {
                        div()
                            .id("provider-signin-error")
                            .role(Role::Alert)
                            .mt_2()
                            .text_size(px(10.))
                            .text_color(gpui::rgb(0xf87171))
                            .child(error)
                    })),
            )
            .into_any_element()
    }
}

impl Accounts {
    fn close_auth(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_auth(cx);
        if let Some(focus) = self.previous_focus.take() {
            window.focus(&focus, cx);
        }
        cx.notify();
    }
    /// Abandon a sign-in still waiting on the browser. The CLI cannot tell
    /// that its browser tab was closed and would otherwise wait 10 minutes,
    /// leaving every Sign in button disabled until then.
    pub(crate) fn cancel_auth(&mut self, cx: &mut Context<Self>) {
        if let Some(signin) = self.signin.take() {
            signin.cancel.store(true, Ordering::Relaxed);
            if !signin.complete {
                self.working = false;
                if signin.adding.is_some() {
                    cx.background_executor()
                        .spawn(async move {
                            accounts::stop_processes(signin.kind, &signin.profile);
                            let _ = signin.profile.remove_credentials(signin.kind);
                        })
                        .detach();
                }
            }
        }
        if let Some(editor) = &self.editor {
            editor
                .input
                .update(cx, |input, _| input.set_read_only(false));
        }
        cx.notify();
    }
}
