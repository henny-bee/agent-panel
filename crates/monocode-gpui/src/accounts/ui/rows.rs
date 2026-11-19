use super::*;

pub(super) fn account_button(
    id: SharedString,
    label: SharedString,
    path: &'static str,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .size_7()
        .rounded_md()
        .flex()
        .items_center()
        .justify_center()
        .text_color(content(0.4))
        .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
        .child(icon(path, 14.))
}

impl Accounts {
    pub(super) fn editor_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(editor) = &self.editor else {
            return div().into_any_element();
        };
        if editor.surface == Surface::Footer {
            let back = cx.listener(|this, _: &(), window, cx| {
                if this.working {
                    this.cancel_auth(cx);
                }
                if let Some(focus) = this.editor.take().and_then(|e| e.previous_focus) {
                    window.focus(&focus, cx);
                }
                this.error = None;
                cx.notify();
            });
            return div()
                .child(back_header(
                    "cancel-account-edit",
                    "Back to accounts",
                    format!("Add {} account", editor.kind.label()),
                    move |window, cx| back(&(), window, cx),
                ))
                .child(
                    div()
                        .mt_1()
                        .px_1()
                        .text_size(px(10.))
                        .line_height(px(16.))
                        .text_color(content(0.4))
                        .child(
                            "Give this account a local name, then finish sign-in in your browser.",
                        ),
                )
                .child(
                    div()
                        .mt_3()
                        .text_size(px(10.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(content(0.55))
                        .child("Account name"),
                )
                .child(
                    div()
                        .mt(px(6.))
                        .h_8()
                        .w_full()
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .rounded_lg()
                        .bg(content(0.04))
                        .border_1()
                        .border_color(content(0.1))
                        .text_size(px(11.))
                        .when(self.working, |d| d.opacity(0.55))
                        .child(editor.input.clone()),
                )
                .child(
                    primary_button(
                        "save-account-edit",
                        "Sign in and add account",
                        self.working || editor.input.read(cx).content.trim().is_empty(),
                    )
                    .mt_3()
                    .h_8()
                    .w_full()
                    .rounded_lg()
                    .justify_center()
                    .gap(px(6.))
                    .text_size(px(11.))
                    .when(self.working, |d| {
                        d.child(crate::ui::motion::pulse::looping(
                            "footer-account-spinner",
                            std::time::Duration::from_secs(1),
                            crate::ui::motion::pulse::Rate::Smooth,
                            move |delta| {
                                icon("refresh-cw", 14.).with_transformation(
                                    gpui::Transformation::rotate(gpui::percentage(delta)),
                                )
                            },
                        ))
                    })
                    .child(if self.working {
                        "Waiting for browser…"
                    } else {
                        "Sign in and add account"
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.submit_editor(cx))),
                )
                .children(self.error.as_ref().map(|error| {
                    div()
                        .mt_2()
                        .text_size(px(10.))
                        .line_height(px(16.))
                        .text_color(gpui::rgb(0xf87171))
                        .child(error.clone())
                }))
                .into_any_element();
        }
        let adding = editor.id.is_none();
        div()
            .h(px(48.))
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(content(0.05))
            .flex()
            .items_center()
            .child(
                div()
                    .h_8()
                    .min_w_0()
                    .flex_1()
                    .pr_1()
                    .rounded_md()
                    .overflow_hidden()
                    .border_1()
                    .border_color(content(0.1))
                    .bg(content(0.04))
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .px(px(10.))
                            .when(self.working, |d| d.opacity(0.5))
                            .child(editor.input.clone()),
                    )
                    .child(
                        secondary_button("cancel-account-edit", "Cancel", false, false)
                            .h_6()
                            .border_0()
                            .px(px(10.))
                            .rounded(px(4.5))
                            .bg(content(0.05))
                            .text_size(px(11.))
                            .text_color(content(0.45))
                            .child("Cancel")
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.working {
                                    this.cancel_auth(cx);
                                }
                                if let Some(focus) =
                                    this.editor.take().and_then(|e| e.previous_focus)
                                {
                                    window.focus(&focus, cx);
                                }
                                this.error = None;
                                this.signin = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        primary_button(
                            "save-account-edit",
                            if adding {
                                "Sign in and add"
                            } else {
                                "Save account"
                            },
                            self.working || editor.input.read(cx).content.trim().is_empty(),
                        )
                        .ml_1()
                        .h_6()
                        .px(px(10.))
                        .rounded(px(4.5))
                        .gap(px(6.))
                        .text_size(px(11.))
                        .when(self.working, |d| {
                            d.child(crate::ui::motion::pulse::looping(
                                "account-editor-spinner",
                                std::time::Duration::from_secs(1),
                                crate::ui::motion::pulse::Rate::Smooth,
                                move |delta| {
                                    icon("refresh-cw", 12.).with_transformation(
                                        gpui::Transformation::rotate(gpui::percentage(delta)),
                                    )
                                },
                            ))
                        })
                        .child(if adding {
                            if self.working {
                                "Waiting for browser…"
                            } else {
                                "Sign in and add"
                            }
                        } else {
                            "Save"
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.submit_editor(cx))),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn settings_rows(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut rows = Vec::new();
        for kind in accounts::PROVIDERS {
            let accounts = self.book.list(kind);
            let count = accounts.len();
            let adding = self.editor.as_ref().is_some_and(|e| {
                e.kind == kind && e.id.is_none() && e.surface == Surface::Settings
            });
            let header = div()
                .px_4()
                .py(px(14.))
                .flex()
                .items_center()
                .gap_4()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(
                            div()
                                .size_7()
                                .rounded_lg()
                                .bg(content(0.05))
                                .border_1()
                                .border_color(content(0.06))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(provider_icon(kind.id(), 16.)),
                        )
                        .child(
                            div()
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(kind.label()),
                                )
                                .child(
                                    div()
                                        .mt(px(2.))
                                        .text_size(px(11.))
                                        .text_color(content(0.4))
                                        .child(format!(
                                            "{count} {}",
                                            if count == 1 { "account" } else { "accounts" }
                                        )),
                                ),
                        ),
                )
                .when(accounts::supports_profiles(kind), |d| {
                    d.child(
                        secondary_button(
                            SharedString::from(format!("add-account-{}", kind.id())),
                            "Add account",
                            false,
                            self.working,
                        )
                        .child(icon("plus", 14.))
                        .child("Add account")
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.start_edit(kind, None, Surface::Settings, window, cx)
                            },
                        )),
                    )
                });
            let mut body = div()
                .border_t_1()
                .border_color(content(0.05))
                .bg(content(0.015))
                .pl_10();
            for account in accounts {
                if self.editor.as_ref().is_some_and(|e| {
                    e.kind == kind
                        && e.id.as_ref() == Some(&account.id)
                        && e.surface == Surface::Settings
                }) {
                    body = body.child(self.editor_row(cx));
                    continue;
                }
                let id = account.id.clone();
                let rename_id = id.clone();
                let key = (kind, id.clone());
                let identity = self.identities.get(&key);
                let status = self.status(kind, &id);
                let subtitle = identity
                    .and_then(|i| i.email.clone().or(i.name.clone()))
                    .unwrap_or_else(|| {
                        if account.is_default() {
                            "Provider CLI profile".into()
                        } else {
                            "Isolated profile".into()
                        }
                    });
                let masked = self.mask_emails
                    && identity.and_then(|i| i.email.as_ref()).is_some()
                    && !self.revealed.contains(&key);
                let org = identity
                    .and_then(|i| i.organization.as_ref().or(i.plan.as_ref()))
                    .cloned();
                let mut controls = div()
                    .w(px(96.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1();
                if account.is_default() {
                    controls = controls.child(
                        div()
                            .mr_1()
                            .text_size(px(10.))
                            .text_color(content(0.3))
                            .child("DEFAULT"),
                    );
                }
                controls = controls.child(
                    account_button(
                        SharedString::from(format!("rename-{id}-{}", kind.id())),
                        SharedString::from(format!("Rename {}", account.label)),
                        "pencil-edit-02",
                    )
                    .size_7()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.start_edit(
                            kind,
                            Some(rename_id.clone()),
                            Surface::Settings,
                            window,
                            cx,
                        )
                    })),
                );
                if !account.is_default() {
                    let remove = account.clone();
                    controls = controls.child(
                        account_button(
                            SharedString::from(format!("remove-{id}-{}", kind.id())),
                            SharedString::from(format!("Remove {}", account.label)),
                            "delete-02",
                        )
                        .size_7()
                        .on_click(cx.listener(
                            move |this, _, window, cx| this.remove(remove.clone(), window, cx),
                        )),
                    );
                }
                let row = div()
                    .h_12()
                    .border_b_1()
                    .border_color(content(0.05))
                    .px_4()
                    .py_2()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .truncate()
                                            .child(account.label.clone()),
                                    )
                                    .children(org.map(|org| {
                                        div()
                                            .px_1()
                                            .rounded_sm()
                                            .bg(content(0.07))
                                            .text_size(px(9.))
                                            .text_color(content(0.5))
                                            .child(org)
                                    })),
                            )
                            .child(
                                div()
                                    .mt(px(2.))
                                    .flex()
                                    .items_center()
                                    .gap(px(10.))
                                    .text_size(px(10.))
                                    .child(status_label(&status).flex_shrink_0())
                                    .child(
                                        div()
                                            .id(SharedString::from(format!(
                                                "account-email-{}-{id}",
                                                kind.id()
                                            )))
                                            .when(
                                                self.mask_emails
                                                    && identity
                                                        .and_then(|i| i.email.as_ref())
                                                        .is_some(),
                                                |d| {
                                                    d.role(Role::Button).aria_label(if masked {
                                                        "Reveal email"
                                                    } else {
                                                        "Hide email"
                                                    })
                                                },
                                            )
                                            .text_color(content(0.3))
                                            .relative()
                                            .truncate()
                                            .child(masked_text(subtitle, masked))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if this.mask_emails && !this.revealed.remove(&key) {
                                                    this.revealed.insert(key.clone());
                                                }
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    )
                    .child(self.meters(kind, &id))
                    .child(controls);
                body = body.child(row);
            }
            if adding {
                body = body.child(self.editor_row(cx));
            }
            rows.push(
                div()
                    .border_b_1()
                    .border_color(content(0.05))
                    .child(header)
                    .child(body)
                    .into_any_element(),
            );
        }
        if let Some(error) = &self.error {
            rows.push(
                div()
                    .id("account-settings-error")
                    .role(Role::Alert)
                    .px_4()
                    .py_2()
                    .text_size(px(11.))
                    .text_color(gpui::rgb(0xf87171))
                    .child(error.clone())
                    .into_any_element(),
            );
        }
        rows
    }
}
