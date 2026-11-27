use super::*;

impl SettingsView {
    pub(in crate::settings::pages) fn mobile_page(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let status = self.ctx.mobile.clone();
        let showing = self.ctx.mobile_invite.is_some();
        let busy = self.mobile.pairing.is_some();
        let mut page = vec![self.mobile_header(status.is_some() && !busy && !showing, cx)];
        let Some(status) = status else {
            page.push(
                div()
                    .mt_5()
                    .rounded_xl()
                    .border_1()
                    .border_color(content(0.1))
                    .bg(content(0.03))
                    .child(note(
                        "Mobile access isn’t available on this desktop right now.",
                    ))
                    .into_any_element(),
            );
            return page;
        };
        if !status.devices.is_empty() {
            page.push(self.mobile_access(&status, cx));
        }
        page.push(self.mobile_devices(&status, showing, cx));
        if let Some(error) = &status.error {
            page.push(
                div()
                    .mt_5()
                    .rounded_lg()
                    .bg(red(0.05))
                    .p_3()
                    .text_size(px(12.))
                    .line_height(px(19.5))
                    .text_color(red(1.))
                    .child(error.clone())
                    .into_any_element(),
            );
        }
        if let Some(notice) = &self.mobile.notice {
            page.push(
                div()
                    .mt_5()
                    .text_size(px(13.))
                    .text_color(rgb(EMERALD))
                    .child(notice.clone())
                    .into_any_element(),
            );
        }
        page.extend(self.mobile_away(&status, cx));
        page
    }

    pub(super) fn mobile_header(&self, can_pair: bool, cx: &mut Context<Self>) -> AnyElement {
        let paragraph = |text: &'static str| {
            div()
                .mt_1()
                .text_size(px(12.))
                .line_height(px(19.5))
                .text_color(content(0.45))
                .child(text)
        };
        // Below Your machines, set off as MobileSettings.tsx was.
        div()
            .relative()
            .mt_10()
            .pt_8()
            .border_t_1()
            .border_color(content(0.1))
            .child(self.reveal_probe("mobile-access"))
            .flex()
            .items_end()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Your phones"),
                            )
                            .child(
                                div()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(content(0.1))
                                    .bg(content(0.05))
                                    .px(px(6.))
                                    .py(px(2.))
                                    .text_size(px(10.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(content(0.6))
                                    .child("Invite-only beta"),
                            ),
                    )
                    .child(paragraph(
                        "Check on projects and conversations from the MonoCode mobile app. Your phone connects straight to this desktop.",
                    ))
                    .child(
                        paragraph("The iPhone app is in beta on TestFlight.")
                            .flex()
                            .flex_wrap()
                            .gap_x_1()
                            .child(
                                div()
                                    .id("mobile-testflight")
                                    .debug_selector(|| "mobile-testflight".into())
                                    .role(Role::Link)
                                    .aria_label("Get the iPhone app on TestFlight")
                                    .text_color(content(0.7))
                                    .underline()
                                    .cursor_pointer()
                                    .hover(|s| s.text_color(content(1.)))
                                    .child("Get the iPhone app")
                                    .on_click(|_, _, cx| cx.open_url(TESTFLIGHT_URL)),
                            ),
                    ),
            )
            .child(
                secondary_button("pair-phone", "Pair phone", false, !can_pair)
                    .child(icon("plus", 16.))
                    .child("Pair phone")
                    .when(can_pair, |d| {
                        d.on_click(cx.listener(|this, _, _, cx| this.pair_phone(cx)))
                    }),
            )
            .into_any_element()
    }

    /// Whether paired phones can reach this desktop, and the switch for it.
    pub(super) fn mobile_access(&self, status: &Status, cx: &mut Context<Self>) -> AnyElement {
        let stuck = status.enabled && !status.listening;
        let description: SharedString = if status.listening {
            "Your phones can reach this desktop.".into()
        } else {
            "Your phones can’t connect because this desktop isn’t accepting connections right now."
                .into()
        };
        let (open_router, remote) = (status.open_router, status.remote_address.clone());
        let mut controls = Vec::new();
        if stuck {
            let remote = remote.clone();
            controls.push(
                secondary_button("mobile-reconnect", "Reconnect", false, false)
                    .child("Reconnect")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.configure_mobile(true, open_router, remote.clone(), cx)
                    }))
                    .into_any_element(),
            );
        }
        controls.push(self.toggle(
            "mobile-enabled",
            "Accept connections",
            status.enabled,
            false,
            cx,
            move |this, on, cx| this.configure_mobile(on, open_router, remote.clone(), cx),
        ));
        let firewall = self.mobile.firewall.map(|(kind, open)| {
            let on = open.unwrap_or(status.firewall_open);
            let (label, ask) = if kind == firewall::Kind::Windows {
                ("Allow through Windows Firewall", "administrator permission")
            } else {
                ("Allow through the firewall", "your password")
            };
            let mut description = if on {
                "Phones on your network can get through.".to_owned()
            } else {
                format!("The firewall blocks phones. Turning this on asks for {ask}.")
            };
            if open.is_none() {
                description += " ufw shows its rules only to administrators, so this is what you last set here.";
            }
            let toggle = self.toggle(
                "mobile-firewall",
                label,
                on,
                self.mobile.firewall_busy,
                cx,
                move |this, on, cx| this.firewall_work(Some((kind, on)), cx),
            );
            self.row(None, false, label, Some(description.into()), vec![toggle])
        });
        div()
            .mt_5()
            .overflow_hidden()
            .rounded_xl()
            .border_1()
            .border_color(if stuck {
                let mut amber: gpui::Hsla = rgb(AMBER).into();
                amber.a = 0.3;
                amber
            } else {
                content(0.1)
            })
            .bg(content(0.03))
            .child(self.row(
                None,
                true,
                "Accept connections",
                Some(description),
                controls,
            ))
            .children(firewall)
            .into_any_element()
    }

    pub(super) fn mobile_devices(
        &self,
        status: &Status,
        showing: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if status.devices.is_empty() {
            return div()
                .mt_5()
                .rounded_xl()
                .border_1()
                .border_color(content(0.15))
                .px_5()
                .py_8()
                .text_center()
                .text_size(px(13.))
                .text_color(content(0.45))
                .child("Pair your iPhone or Android phone to get started.")
                .into_any_element();
        }
        let last = status.devices.len() == 1;
        let (open_router, remote) = (status.open_router, status.remote_address.clone());
        div()
            .mt_5()
            .overflow_hidden()
            .rounded_xl()
            .border_1()
            .border_color(content(0.1))
            .flex()
            .flex_col()
            .children(status.devices.iter().enumerate().map(|(ix, device)| {
                let id = device.id.clone();
                let name = device.name.clone();
                let confirming = self.mobile.removing.as_deref() == Some(id.as_str());
                div()
                    .when(ix > 0, |d| d.border_t_1().border_color(content(0.1)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .px_4()
                            .py_4()
                            .child(icon("smart-phone", 20.).text_color(content(0.45)))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(name.clone()),
                                    )
                                    .child(
                                        div()
                                            .mt_1()
                                            .text_size(px(12.))
                                            .text_color(content(0.5))
                                            .child("Paired"),
                                    ),
                            )
                            .child(
                                icon_action(
                                    SharedString::from(format!("mobile-remove-{id}")),
                                    format!("Remove {name}"),
                                    "trash",
                                    32.,
                                    16.,
                                    0.4,
                                    false,
                                )
                                .on_click({
                                    let id = id.clone();
                                    cx.listener(move |this, _, _, cx| {
                                        this.mobile.removing = Some(id.clone());
                                        cx.notify();
                                    })
                                }),
                            ),
                    )
                    .when(confirming, |d| {
                        let remote = remote.clone();
                        d.child(
                            div()
                                .id(SharedString::from(format!("mobile-confirm-{id}")))
                                .role(Role::Group)
                                .aria_label(format!("Confirm removing {name}"))
                                .flex()
                                .flex_col()
                                .gap_3()
                                .border_t_1()
                                .border_color(content(0.1))
                                .bg(content(0.03))
                                .px_4()
                                .py_4()
                                .text_size(px(12.))
                                .line_height(px(19.5))
                                .text_color(content(0.6))
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(content(1.))
                                        .child(format!("Remove {name}?")),
                                )
                                .child(
                                    "It disconnects right away and can’t reach this desktop again until you pair it with a new code.",
                                )
                                .child(
                                    div()
                                        .flex()
                                        .gap_2()
                                        .child(
                                            secondary_button(
                                                "mobile-confirm-remove",
                                                "Remove phone",
                                                true,
                                                false,
                                            )
                                            .child("Remove phone")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.emit(SettingsEvent::MobileRevoke(id.clone()));
                                                if last && !showing {
                                                    cx.emit(SettingsEvent::MobileConfigure {
                                                        enabled: false,
                                                        open_router,
                                                        remote_address: remote.clone(),
                                                    });
                                                }
                                                this.mobile.removing = None;
                                                this.mobile.notice =
                                                    Some(format!("{name} was removed."));
                                                cx.notify();
                                            })),
                                        )
                                        .child(
                                            secondary_button(
                                                "mobile-cancel-remove",
                                                "Cancel",
                                                false,
                                                false,
                                            )
                                            .border_0()
                                            .child("Cancel")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.mobile.removing = None;
                                                cx.notify();
                                            })),
                                        ),
                                ),
                        )
                    })
            }))
            .into_any_element()
    }

    /// MobileSettings.tsx's "Connect away from home" details.
    pub(super) fn mobile_away(
        &mut self,
        status: &Status,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let open = self.mobile.away;
        let mut page = vec![
            div()
                .id("mobile-away")
                .relative()
                .child(self.reveal_probe("mobile-remote"))
                .role(Role::Button)
                .aria_label("Connect away from home")
                .aria_expanded(open)
                .mt_8()
                .text_size(px(12.))
                .text_color(content(0.45))
                .cursor_pointer()
                .child(if open {
                    "▾ Connect away from home"
                } else {
                    "▸ Connect away from home"
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.mobile.away = !this.mobile.away;
                    cx.notify();
                }))
                .into_any_element(),
        ];
        if !open {
            return page;
        }
        let field = self.remote_field(status, cx);
        let unsaved = field.read(cx).content.trim() != status.remote_address;
        let (enabled, remote) = (status.enabled, status.remote_address.clone());
        let paragraph = |text: SharedString| {
            div()
                .text_size(px(12.))
                .line_height(px(19.5))
                .text_color(content(0.45))
                .child(text)
        };
        let addresses = status
            .addresses
            .iter()
            .map(|a| format!("{}:{}", a.host, a.port))
            .collect::<Vec<_>>()
            .join(", ");
        page.push(
            div()
                .mt_4()
                .flex()
                .flex_col()
                .gap_3()
                .child(paragraph(
                    "On the same Wi-Fi, phones connect without any setup. To reach this desktop from anywhere else, it needs a public address. Keep the desktop awake.".into(),
                ))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(13.))
                        .text_color(content(0.7))
                        .child(self.toggle(
                            "mobile-open-router",
                            "Open the router port automatically",
                            status.open_router,
                            false,
                            cx,
                            move |this, on, cx| {
                                this.configure_mobile(enabled, on, remote.clone(), cx)
                            },
                        ))
                        .child("Open the router port automatically"),
                )
                .when(!status.router_status.is_empty(), |d| {
                    d.child(paragraph(status.router_status.clone().into()))
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .text_size(px(12.))
                        .text_color(content(0.65))
                        .child("Remote address")
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .h(px(32.))
                                        .min_w_0()
                                        .flex_1()
                                        .rounded_md()
                                        .border_1()
                                        .border_color(content(0.1))
                                        .px_2()
                                        .flex()
                                        .items_center()
                                        .child(field),
                                )
                                .child(
                                    secondary_button(
                                        "mobile-save-remote",
                                        "Save",
                                        false,
                                        !unsaved,
                                    )
                                    .child("Save")
                                    .when(unsaved, |d| {
                                        d.on_click(cx.listener(|this, _, _, cx| {
                                            this.save_remote_address(cx)
                                        }))
                                    }),
                                ),
                        ),
                )
                .child(paragraph(
                    "Automatic mapping needs a public IP. Otherwise, forward UDP port 38416 to this desktop or use a VPN address. Set the address before making a pairing code.".into(),
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .text_size(px(12.))
                        .line_height(px(19.5))
                        .text_color(content(0.45))
                        .child(format!("This desktop: {}", status.name))
                        .when_some(status.fingerprint.as_deref(), |d, fingerprint| {
                            d.child(format!("Fingerprint: {}", short_fingerprint(fingerprint)))
                        })
                        .child(if addresses.is_empty() {
                            "Not reachable on any network yet.".to_owned()
                        } else {
                            format!("Reachable at {addresses}")
                        }),
                )
                .into_any_element(),
        );
        page
    }

    pub(crate) fn mobile_overlay(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.mobile_overlay_open() {
            return None;
        }
        let invite = self.ctx.mobile_invite.clone()?;
        let left = invite.expires.saturating_sub(now());
        let expired = left == 0;
        let busy = self.mobile.pairing.is_some();
        let qr = self.invite_qr(&invite.url);
        let viewport = window.viewport_size();
        let width = DIALOG_WIDTH.min((viewport.width.as_f32() - 24.).max(0.));
        let max_height = (viewport.height.as_f32() - 32.).max(0.);
        // Keep the QR square on narrow windows, without growing past size-96.
        let qr_side = QR_SIDE.min((width - 50.).max(0.));
        let lead = "Open MonoCode on your phone, tap ";
        let action = "Connect to desktop";
        let instructions = format!("{lead}{action}, and scan the code.");
        let bold = lead.len()..lead.len() + action.len();
        let url = invite.url.clone();
        let code = div()
            .id("mobile-pair-code")
            .debug_selector(|| "mobile-pair-code".into())
            .role(Role::Image)
            .aria_label("Scan this one-use pairing code in the MonoCode mobile app")
            .relative()
            .size(px(qr_side))
            .flex_shrink_0()
            .overflow_hidden()
            .rounded_2xl()
            .bg(rgb(0xffffff))
            .p(px(QR_PADDING))
            .when_some(qr, |d, qr| {
                d.child(
                    div()
                        .size_full()
                        .when(expired, |d| d.opacity(0.15))
                        .child(qr_canvas(qr)),
                )
            })
            .when(expired, |d| {
                d.child(expired_qr_blur()).child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .id("mobile-new-code-expired")
                                .debug_selector(|| "mobile-new-code-expired".into())
                                .role(Role::Button)
                                .aria_label("New code")
                                .rounded_lg()
                                .bg(rgb(0x000000))
                                .px_3()
                                .py_2()
                                .text_size(px(13.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(0xffffff))
                                .child("New code")
                                .map(|d| {
                                    if busy {
                                        d.opacity(0.4)
                                    } else {
                                        d.cursor_pointer().on_click(
                                            cx.listener(|this, _, _, cx| this.pair_phone(cx)),
                                        )
                                    }
                                }),
                        ),
                )
            });
        let status = div()
            .mt(px(6.))
            .h(px(40.))
            .flex_shrink_0()
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .pl_1()
            .text_size(px(12.))
            .text_color(content(0.5))
            .child(icon("clock", 14.))
            .child(
                div()
                    .debug_selector(|| "mobile-countdown".into())
                    .flex_1()
                    .child(countdown(left)),
            )
            .child(
                pairing_icon_action(
                    "mobile-copy-link",
                    if self.mobile.copied {
                        "Copied pairing link"
                    } else {
                        "Copy pairing link"
                    },
                    if self.mobile.copied {
                        "check"
                    } else {
                        "response-copy"
                    },
                    self.mobile.copied,
                    expired,
                )
                .when(!expired, |d| {
                    d.on_click(cx.listener(move |this, _, _, cx| this.copy_invite(url.clone(), cx)))
                }),
            )
            .child(
                pairing_icon_action(
                    "mobile-new-code",
                    "Make a new code",
                    "refresh-cw",
                    false,
                    busy,
                )
                .when(!busy, |d| {
                    d.on_click(cx.listener(|this, _, _, cx| this.pair_phone(cx)))
                }),
            );
        let body = div()
            .flex()
            .flex_col()
            .items_center()
            .px_6()
            .pb_6()
            .child(
                div()
                    .text_size(px(22.))
                    .line_height(px(33.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(content(1.))
                    .child("Connect a phone to this desktop"),
            )
            .child(
                div()
                    .mt(px(6.))
                    .text_center()
                    .text_size(px(12.))
                    .line_height(px(19.5))
                    .text_color(content(0.5))
                    .child(StyledText::new(instructions).with_highlights([(
                        bold,
                        HighlightStyle {
                            color: Some(content(0.8)),
                            font_weight: Some(FontWeight::MEDIUM),
                            ..Default::default()
                        },
                    )])),
            )
            .child(
                div()
                    .mt_5()
                    .w(px(qr_side))
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(code)
                    .child(status),
            )
            .child(
                div()
                    .mt_4()
                    .text_center()
                    .text_size(px(12.))
                    .line_height(px(19.5))
                    .text_color(content(0.45))
                    .child("Can’t scan? Copy the link and open it on your phone. Each code works once."),
            );
        Some(
            div()
                .id("mobile-pair-overlay")
                .debug_selector(|| "mobile-pair-overlay".into())
                .absolute()
                .inset_0()
                .occlude()
                .child(crate::ui::modal::scrim("mobile-pair-scrim"))
                .flex()
                .items_center()
                .justify_center()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.close_invite(cx);
                    }),
                )
                .child(
                    div()
                        .id("mobile-pair-dialog")
                        .debug_selector(|| "mobile-pair-dialog".into())
                        .role(Role::Dialog)
                        .aria_label("Pair a phone")
                        .relative()
                        .w(px(width))
                        .max_h(px(max_height))
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .rounded(px(DIALOG_RADIUS))
                        .shadow_2xl()
                        .border_1()
                        .border_color(content(0.07))
                        .child(pairing_backdrop(
                            self.ctx.classic || self.ctx.new_layout_boxy,
                        ))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .id("mobile-pair-scroll")
                                .debug_selector(|| "mobile-pair-scroll".into())
                                .relative()
                                .min_h_0()
                                .overflow_y_scroll()
                                .child(mascot_lineup())
                                .child(body),
                        ),
                )
                .into_any_element(),
        )
    }
}
