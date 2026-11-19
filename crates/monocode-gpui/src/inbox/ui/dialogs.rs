use super::*;

impl Inbox {
    pub(super) fn connection_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        popover(
            "inbox-connections-menu",
            Anchor::Slot(self.connection_anchor.clone()),
        )
        .width(272.)
        .padding(4.)
        .on_dismiss(move |_, cx| {
            let _ = weak.update(cx, |this, cx| {
                this.connections_open = false;
                cx.notify();
            });
        })
        .child(caption("CONNECTIONS"))
        .children(model::PROVIDERS.into_iter().map(|provider| {
            let connected = self
                .snapshot
                .connections
                .get(provider)
                .is_some_and(|s| s.connected);
            let label = format!(
                "{} {}",
                if connected { "Manage" } else { "Connect" },
                model::provider_label(provider)
            );
            div()
                .id(SharedString::from(format!("inbox-connect-{provider}")))
                .role(Role::Button)
                .aria_label(label.clone())
                .w_full()
                .h(px(28.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(6.))
                .rounded(px(6.))
                .text_size(px(12.))
                .text_color(content(0.7))
                .cursor_pointer()
                .hover(|s| s.bg(content(0.07)).text_color(content(1.)))
                .child(provider_mark(provider))
                .child(label)
                .on_click(
                    cx.listener(move |this, _, _, cx| this.open_connection(provider.into(), cx)),
                )
        }))
        .into_any_element()
    }

    pub(super) fn connection_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let provider = self.connect_provider.clone().unwrap_or_default();
        let connected = self
            .snapshot
            .connections
            .get(&provider)
            .is_some_and(|s| s.connected);
        let title = format!(
            "{} {}",
            if connected { "Manage" } else { "Connect" },
            model::provider_label(&provider)
        );
        let hint = match provider.as_str() {
            "github" => {
                "Install GitHub CLI, then run gh auth login in a terminal. Check again when signed in."
            }
            "linear" => "Create a personal API key in Linear → Settings → Security & Access.",
            "jira" => "Connect your Jira Cloud site with your Atlassian email and an API token.",
            "gitlab" => {
                "Connect GitLab.com or your GitLab instance with a personal access token with API access."
            }
            _ => {
                "Connect your ADO organization using a personal access token with Boards and Repos read & write access."
            }
        };
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .child(crate::ui::modal::scrim("inbox-connect-scrim"))
            .child(crate::ui::modal::enter(
                "inbox-connect-card",
                div()
                    .relative()
                    .w(px(440.))
                    .max_w_full()
                    .p(px(24.))
                    .rounded(px(crate::ui::modal::RADIUS))
                    .border_1()
                    .border_color(content(crate::ui::modal::BORDER))
                    .child(crate::ui::modal::surface(crate::ui::modal::RADIUS - 1.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(provider_mark(&provider))
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(content(1.))
                                    .child(title),
                            )
                            .child(
                                icon_button("inbox-close-connect", "Close connection", "close")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if !this.connecting {
                                            this.connect_provider = None;
                                            this.connect_token.update(cx, |input, cx| {
                                                input.set_text(String::new(), cx)
                                            });
                                            cx.notify();
                                        }
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .line_height(px(19.))
                            .text_color(content(0.5))
                            .child(hint),
                    )
                    .when(provider != "github" && provider != "linear", |d| {
                        d.child(
                            div()
                                .px(px(10.))
                                .py(px(8.))
                                .rounded(px(6.))
                                .border_1()
                                .border_color(content(0.1))
                                .child(self.connect_url.clone()),
                        )
                    })
                    .when(provider == "jira", |d| {
                        d.child(
                            div()
                                .px(px(10.))
                                .py(px(8.))
                                .rounded(px(6.))
                                .border_1()
                                .border_color(content(0.1))
                                .child(self.connect_email.clone()),
                        )
                    })
                    .when(provider != "github", |d| {
                        d.child(
                            div()
                                .px(px(10.))
                                .py(px(8.))
                                .rounded(px(6.))
                                .border_1()
                                .border_color(content(0.1))
                                .child(self.connect_token.clone()),
                        )
                    })
                    .when_some(self.connect_error.clone(), |d, text| d.child(error(text)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .when(connected && provider != "github", |d| {
                                d.child(
                                    action("inbox-disconnect", "Disconnect")
                                        .text_color(rgb(0xf87171))
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.connect(true, cx)),
                                        ),
                                )
                            })
                            .when(provider == "github", |d| {
                                d.child(
                                    action("inbox-gh-guide", "Installation guide").on_click(
                                        |_, _, cx| cx.open_url("https://cli.github.com/"),
                                    ),
                                )
                            })
                            .child(div().flex_1())
                            .child(
                                crate::ui::modal::disabled(
                                    crate::ui::modal::primary_button(
                                        "inbox-connect-submit",
                                        if self.connecting {
                                            "Connecting…"
                                        } else if provider == "github" {
                                            "Check again"
                                        } else {
                                            "Connect"
                                        },
                                    ),
                                    self.connecting,
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.connect(false, cx))),
                            ),
                    ),
            ))
            .into_any_element()
    }

    pub(super) fn action_menu_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        popover(
            "inbox-action-menu",
            Anchor::Slot(self.action_anchor.clone()),
        )
        .width(272.)
        .padding(4.)
        .on_dismiss(move |_, cx| {
            let _ = weak.update(cx, |this, cx| {
                this.action_menu = false;
                cx.notify();
            });
        })
        .children(
            [
                (
                    "merge",
                    "Create a merge commit",
                    "Add every commit to the base branch.",
                ),
                (
                    "squash",
                    "Squash and merge",
                    "Combine the commits into one.",
                ),
                (
                    "rebase",
                    "Rebase and merge",
                    "Add the commits without a merge commit.",
                ),
            ]
            .into_iter()
            .map(|(name, label, description)| {
                div()
                    .id(format!("inbox-merge-{name}"))
                    .role(Role::MenuItemRadio)
                    .aria_label(label)
                    .w_full()
                    .px(px(8.))
                    .py(px(8.))
                    .rounded(px(8.))
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .cursor_pointer()
                    .hover(|s| s.bg(content(0.08)))
                    .child(div().mt(px(4.)).size(px(6.)).rounded_full().bg(
                        if self.merge_action == name {
                            accent_color()
                        } else {
                            content(0.2)
                        },
                    ))
                    .child(
                        div()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(content(0.9))
                                    .child(label),
                            )
                            .child(
                                div()
                                    .mt(px(2.))
                                    .text_size(px(11.))
                                    .line_height(px(15.))
                                    .text_color(content(0.45))
                                    .child(description),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.merge_action = name.into();
                        this.action_menu = false;
                        cx.notify();
                    }))
            }),
        )
        .into_any_element()
    }

    pub(super) fn action_confirmation(&self, cx: &mut Context<Self>) -> AnyElement {
        let action_name = self.confirm_action.clone().unwrap_or_default();
        let details = self.details.clone().unwrap_or_default();
        let source = if details.head_ref_name.is_empty() {
            "this branch".into()
        } else {
            format!("“{}”", details.head_ref_name)
        };
        let destination = if details.base_ref_name.is_empty() {
            "the base branch".into()
        } else {
            format!("“{}”", details.base_ref_name)
        };
        let (title, detail, label, progress) = match action_name.as_str() {
            "merge" => (
                "Merge this pull request?",
                format!(
                    "Every commit from {source} will be added to {destination} with a merge commit."
                ),
                "Merge pull request",
                "Merging…",
            ),
            "squash" => (
                "Squash and merge?",
                format!(
                    "The commits from {source} will be combined into one commit on {destination}."
                ),
                "Squash and merge",
                "Merging…",
            ),
            "rebase" => (
                "Rebase and merge?",
                format!(
                    "The commits from {source} will be rebased individually onto {destination}."
                ),
                "Rebase and merge",
                "Merging…",
            ),
            "close" => (
                "Close this pull request?",
                "The pull request will close without merging. You can reopen it later.".into(),
                "Close pull request",
                "Closing…",
            ),
            "reopen" => (
                "Reopen this pull request?",
                "The pull request will return to the open state.".into(),
                "Reopen pull request",
                "Reopening…",
            ),
            "ready" => (
                "Mark as ready for review?",
                "Reviewers will see that this pull request is ready for feedback.".into(),
                "Ready for review",
                "Updating…",
            ),
            _ => (
                "Convert to draft?",
                "Reviewers will see that this pull request is not ready to merge.".into(),
                "Convert to draft",
                "Converting…",
            ),
        };
        let anchor = match action_name.as_str() {
            "ready" => self.pr_action_anchors[0].clone(),
            "draft" => self.pr_action_anchors[1].clone(),
            "close" => self.pr_action_anchors[2].clone(),
            "reopen" => self.pr_action_anchors[3].clone(),
            _ => self.action_anchor.clone(),
        };
        let weak = cx.entity().downgrade();
        popover("inbox-pr-confirmation", Anchor::Slot(anchor))
            .width(320.)
            .gap(5.)
            .padding(12.)
            .on_dismiss(move |_, cx| {
                let _ = weak.update(cx, |this, cx| {
                    if !this.acting {
                        this.confirm_action = None;
                        this.action_error = None;
                        cx.notify();
                    }
                });
            })
            .child(
                div()
                    .id("inbox-pr-confirmation-dialog")
                    .role(Role::Dialog)
                    .aria_label(title)
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(content(1.))
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .line_height(px(17.))
                            .text_color(content(0.55))
                            .child(detail),
                    ),
            )
            .when_some(self.action_error.clone(), |d, text| {
                d.child(
                    div()
                        .mt(px(8.))
                        .text_size(px(11.))
                        .text_color(rgb(0xf87171))
                        .child(text),
                )
            })
            .child(
                div()
                    .mt(px(12.))
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        action("inbox-action-cancel", "Cancel")
                            .when(self.acting, |d| d.opacity(0.4))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if !this.acting {
                                    this.confirm_action = None;
                                    this.action_error = None;
                                    cx.notify();
                                }
                            })),
                    )
                    .child(
                        action(
                            "inbox-action-confirm",
                            if self.acting { progress } else { label },
                        )
                        .font_weight(FontWeight::MEDIUM)
                        .bg(if action_name == "close" {
                            Hsla::from(rgb(0xf43f5e)).opacity(0.2)
                        } else if matches!(action_name.as_str(), "merge" | "squash" | "rebase") {
                            Hsla::from(rgb(0x10b981)).opacity(0.2)
                        } else {
                            content(1.)
                        })
                        .text_color(if action_name == "close" {
                            rgb(0xfda4af).into()
                        } else if matches!(action_name.as_str(), "merge" | "squash" | "rebase") {
                            rgb(0x6ee7b7).into()
                        } else {
                            shell_fill()
                        })
                        .when(self.acting, |d| d.opacity(0.6))
                        .on_click(cx.listener(|this, _, _, cx| this.run_action(cx))),
                    ),
            )
            .into_any_element()
    }
}
