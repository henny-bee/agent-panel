use super::*;

impl Accounts {
    pub(crate) fn picker(
        &self,
        kind: Kind,
        selected: &str,
        project: &str,
        session: Option<i64>,
        on_back: impl Fn(&mut Window, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let adding = self
            .editor
            .as_ref()
            .is_some_and(|e| e.kind == kind && e.surface == Surface::Footer);
        if adding {
            return self.editor_row(cx);
        }
        let now = usage::now_ms();
        let mut rows = div()
            .id("provider-account-choices")
            .role(Role::Group)
            .aria_label(format!("{} accounts", kind.label()))
            .mt_2()
            .flex()
            .flex_col()
            .gap_1();
        for (index, account) in self.book.list(kind).into_iter().enumerate() {
            let status = self.status(kind, &account.id);
            let active = account.id == selected;
            let id = account.id.clone();
            let project = project.to_owned();
            let org = self
                .identities
                .get(&(kind, id.clone()))
                .and_then(organization_tag);
            let windows = self
                .usage
                .get(&Key::new(kind, self.profile(&id), None))
                .and_then(|s| s.snapshot.as_ref())
                .map(|s| s.windows.clone())
                .unwrap_or_default();
            let highlighted = self
                .picker
                .as_ref()
                .is_some_and(|p| p.kind == kind && p.index == index);
            let fill = if active {
                accent(if highlighted { 0.14 } else { 0.1 })
            } else {
                content(if highlighted { 0.075 } else { 0.035 })
            };
            rows = rows.child(
                div()
                    .id(SharedString::from(format!("pick-account-{}", id)))
                    .role(Role::MenuItem)
                    .aria_label(account.label.clone())
                    .px(px(10.))
                    .py_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(if active { accent(0.2) } else { content(0.06) })
                    .bg(fill)
                    .hover(move |s| s.bg(if active { accent(0.14) } else { content(0.075) }))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap_3()
                    .text_size(px(11.))
                    .text_color(content(if active { 1. } else { 0.7 }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .py(px(2.))
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap(px(6.))
                                    .min_w_0()
                                    .child(div().flex_shrink_0().child(account.label))
                                    .children(
                                        self.subtitle(kind, &id, cx).map(|d| {
                                            d.text_size(px(10.)).text_color(content(0.35))
                                        }),
                                    )
                                    .children(org.map(|org| {
                                        div()
                                            .max_w(px(96.))
                                            .flex_shrink_0()
                                            .truncate()
                                            .px_1()
                                            .rounded_sm()
                                            .bg(content(0.07))
                                            .text_size(px(9.))
                                            .line_height(px(16.))
                                            .text_color(content(0.5))
                                            .child(org)
                                    })),
                            )
                            .child(
                                div()
                                    .mt_1()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .min_w_0()
                                    .text_size(px(10.))
                                    .child(status_label(&status).map(|d| {
                                        if windows.is_empty() {
                                            d.min_w_0()
                                        } else {
                                            d.flex_shrink_0()
                                        }
                                    }))
                                    .when(!windows.is_empty(), |d| {
                                        d.child(
                                            div().flex().flex_1().min_w_0().gap(px(10.)).children(
                                                windows
                                                    .iter()
                                                    .map(|w| meter(w, now, self.show_remaining)),
                                            ),
                                        )
                                    }),
                            ),
                    )
                    .children(active.then(|| icon("check", 14.).text_color(accent(1.))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.choose(kind, id.clone(), project.clone(), session, cx)
                    })),
            );
        }
        let list_row = |id: &'static str, label: &'static str, ink: f32| {
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(label)
                .h_8()
                .w_full()
                .px(px(10.))
                .rounded_lg()
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(11.))
                .text_color(content(ink))
                .cursor_pointer()
                .hover(|s| s.bg(content(0.07)).text_color(content(1.)))
        };
        div()
            .track_focus(&self.picker_focus)
            .on_key_down(cx.listener(Self::picker_key))
            .child(back_header(
                "back-to-usage",
                "Back to usage",
                format!("{} accounts", kind.label()),
                on_back,
            ))
            .child(
                div()
                    .mt_1()
                    .px_1()
                    .text_size(px(10.))
                    .line_height(px(16.))
                    .text_color(content(0.4))
                    .child("Choosing an account moves the current conversation to it."),
            )
            .child(rows)
            .when(accounts::supports_profiles(kind), |d| {
                d.child(
                    list_row("footer-add-account", "Add account", 0.55)
                        .mt_2()
                        .when(self.working, |d| d.opacity(0.4))
                        .child(icon("plus", 14.))
                        .child("Add account")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if !this.working {
                                this.start_edit(kind, None, Surface::Footer, window, cx)
                            }
                        })),
                )
            })
            .child(
                list_row("manage-provider-accounts", "Manage accounts", 0.45)
                    .mt(px(2.))
                    .child("Manage accounts…")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(AccountEvent::Manage))),
            )
            .into_any_element()
    }
}

/// identityOrganizationTag: "Jane's Organization" is a personal workspace.
pub(super) fn organization_tag(identity: &accounts::Identity) -> Option<String> {
    let name = identity.organization.as_deref()?.trim();
    if name.is_empty() {
        None
    } else if name.ends_with("'s Organization") || name.ends_with("’s Organization") {
        Some("Personal".into())
    } else {
        Some(name.into())
    }
}
