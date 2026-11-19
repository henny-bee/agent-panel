use super::*;

impl Inbox {
    pub(super) fn detail_view(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(item) = self.item() else {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(12.))
                .child(icon("inbox", 24.).text_color(content(0.3)))
                .child(message("Select an inbox item").text_size(px(13.)))
                .into_any_element();
        };
        let is_pr = item.kind == "pr";
        let details = self.details.clone().unwrap_or_default();
        let assignees = item
            .assignees
            .iter()
            .filter(|person| {
                !person
                    .login
                    .trim()
                    .eq_ignore_ascii_case(details.author.trim())
            })
            .collect::<Vec<_>>();
        let base_ref = if details.base_ref_name.is_empty() {
            self.thread
                .as_ref()
                .map(|t| t.base_ref_name.as_str())
                .unwrap_or("")
        } else {
            &details.base_ref_name
        };
        let head_ref = if details.head_ref_name.is_empty() {
            self.thread
                .as_ref()
                .map(|t| t.head_ref_name.as_str())
                .unwrap_or("")
        } else {
            &details.head_ref_name
        };
        let title = div()
            .w_full()
            .flex_shrink_0()
            .border_b_1()
            .border_color(content(0.07))
            .child(
                div()
                    .w_full()
                    .px(px(12.))
                    .pt(px(20.))
                    .pb(px(if is_pr { 0. } else { 20. }))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .text_size(px(12.))
                            .text_color(content(0.5))
                            .child(provider_mark(&item.provider))
                            .child(if is_pr {
                                if item.provider == "gitlab" {
                                    "Merge request"
                                } else {
                                    "Pull request"
                                }
                            } else {
                                "Issue"
                            })
                            .child(item.reference())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.))
                                    .text_color(status_color(&item))
                                    .child(
                                        icon(status_icon(&item), 14.)
                                            .text_color(status_color(&item)),
                                    )
                                    .child(item.status()),
                            )
                            .when(!item.attention_reason.is_empty(), |d| {
                                d.child(
                                    div()
                                        .text_color(accent_color())
                                        .child(attention_label(&item.attention_reason)),
                                )
                            })
                            .child(div().min_w_0().truncate().child(
                                if item.team_name.is_empty() {
                                    item.repo.clone()
                                } else {
                                    item.team_name.clone()
                                },
                            )),
                    )
                    .child(
                        div()
                            .line_clamp(2)
                            .text_size(px(20.))
                            .line_height(px(25.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(content(1.))
                            .child(item.title.clone()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .overflow_hidden()
                            .text_size(px(12.))
                            .text_color(content(0.5))
                            .when(!details.author.is_empty(), |d| {
                                d.child(avatar::person(
                                    &item.provider,
                                    &details.author,
                                    &details.author_avatar_url,
                                    16.,
                                    window,
                                    cx,
                                ))
                            })
                            .when(!assignees.is_empty(), |d| {
                                d.child("·").children(assignees.iter().map(|person| {
                                    avatar::person(
                                        &item.provider,
                                        &person.login,
                                        &person.avatar_url,
                                        16.,
                                        window,
                                        cx,
                                    )
                                }))
                            })
                            .when(
                                item.assignees.is_empty()
                                    && matches!(
                                        item.provider.as_str(),
                                        "github" | "gitlab" | "azuredevops"
                                    ),
                                |d| d.child("·").child("Unassigned"),
                            )
                            .when(!item.created_at.is_empty(), |d| {
                                d.child("·")
                                    .child(format!("Created {}", relative_time(&item.created_at)))
                            })
                            .when(!item.updated_at.is_empty(), |d| {
                                d.child("·")
                                    .child(format!("Updated {}", relative_time(&item.updated_at)))
                            })
                            .when(!base_ref.is_empty() && !head_ref.is_empty(), |d| {
                                let branch = head_ref.to_owned();
                                d.child("·").child(
                                    div()
                                        .min_w_0()
                                        .flex()
                                        .items_center()
                                        .gap(px(4.))
                                        .child(icon("git-compare", 12.).text_color(content(0.5)))
                                        .child(
                                            div()
                                                .min_w_0()
                                                .truncate()
                                                .child(format!("{base_ref} ← {head_ref}")),
                                        )
                                        .child(
                                            icon_button(
                                                "inbox-copy-branch",
                                                "Copy branch name",
                                                "response-copy",
                                            )
                                            .size(px(18.))
                                            .on_click(
                                                move |_, _, cx| {
                                                    cx.write_to_clipboard(
                                                        ClipboardItem::new_string(branch.clone()),
                                                    )
                                                },
                                            ),
                                        ),
                                )
                            })
                            .when(!details.review_decision.is_empty(), |d| {
                                d.child("·").child(review_label(&details.review_decision))
                            }),
                    )
                    .when(!self.related_sessions(&item).is_empty(), |d| {
                        d.child(
                            div()
                                .min_w_0()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .overflow_hidden()
                                .child(icon("message-multiple", 14.).text_color(content(0.45)))
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(content(0.45))
                                        .child("Related threads"),
                                )
                                .children(self.related_sessions(&item).into_iter().map(
                                    |session| {
                                        action(
                                            format!("inbox-session-{}", session.id),
                                            session.title,
                                        )
                                        .h(px(24.))
                                        .bg(content(0.05))
                                        .max_w(px(256.))
                                        .truncate()
                                        .on_click(
                                            cx.listener(move |_, _, _, cx| {
                                                cx.emit(Event::OpenSession(session.id))
                                            }),
                                        )
                                    },
                                )),
                        )
                    })
                    .child(
                        div()
                            .pt(px(2.))
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(8.))
                            .when(!is_pr, |d| {
                                d.child(
                                    action("inbox-send-agent", "Open in new session")
                                        .px(px(12.))
                                        .bg(content(1.))
                                        .text_color(shell_fill())
                                        .font_weight(FontWeight::MEDIUM)
                                        .when(self.detail_loading || self.details.is_none(), |d| {
                                            d.opacity(0.4)
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if !this.detail_loading && this.details.is_some() {
                                                this.start(cx);
                                            }
                                        })),
                                )
                            })
                            .when(!is_pr && item.project_path.is_empty(), |d| {
                                d.child(
                                    icon_action(
                                        "inbox-choose-project",
                                        &self.project_label(&self.start_project.to_string_lossy()),
                                        "folder",
                                    )
                                    .relative()
                                    .border_1()
                                    .border_color(content(0.1))
                                    .child(icon("chevron-down", 12.))
                                    .child(self.project_anchor.probe())
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            this.project_picker = !this.project_picker;
                                            cx.notify();
                                        },
                                    )),
                                )
                            })
                            .when(is_pr && item.provider == "github", |d| {
                                d.child(self.pr_actions(&item, cx))
                            })
                            .child(self.copy_link_button(&item, cx))
                            .child(
                                icon_action(
                                    "inbox-open-external",
                                    &format!(
                                        "{} {}",
                                        if is_pr { "Review on" } else { "Open in" },
                                        model::provider_label(&item.provider)
                                    ),
                                    "external-link",
                                )
                                .when(item.url.is_empty(), |d| d.opacity(0.4))
                                .on_click({
                                    let url = item.url.clone();
                                    move |_, _, cx| {
                                        if !url.is_empty() {
                                            cx.open_url(&url);
                                        }
                                    }
                                }),
                            ),
                    )
                    .when(is_pr, |d| {
                        d.child(
                            div()
                                .id("inbox-pr-tabs")
                                .role(Role::TabList)
                                .aria_label("Pull request details")
                                .h(px(36.))
                                .flex()
                                .items_stretch()
                                .gap(px(16.))
                                .children(
                                    [
                                        (Tab::Summary, "Summary"),
                                        (Tab::Code, "Code"),
                                        (Tab::Checks, "Checks"),
                                    ]
                                    .into_iter()
                                    .filter(|(tab, _)| {
                                        *tab != Tab::Checks || item.provider == "github"
                                    })
                                    .map(|(tab, label)| {
                                        plain_action(format!("inbox-tab-{label}"), label)
                                            .role(Role::Tab)
                                            .h_full()
                                            .px_0()
                                            .rounded_none()
                                            .hover(|s| s.text_color(content(1.)))
                                            .when(self.tab == tab, |d| {
                                                d.border_b_2()
                                                    .border_color(content(1.))
                                                    .text_color(content(1.))
                                            })
                                            .when(tab == Tab::Checks, |d| {
                                                let failed = self
                                                    .checks
                                                    .as_ref()
                                                    .map(|checks| {
                                                        checks
                                                            .checks
                                                            .iter()
                                                            .filter(|check| {
                                                                failed_check(&check.state)
                                                            })
                                                            .count()
                                                    })
                                                    .unwrap_or(0);
                                                d.when(failed > 0, |d| {
                                                    d.child(
                                                        icon("circle-x", 13.)
                                                            .text_color(rgb(0xf87171)),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_color(rgb(0xf87171))
                                                            .child(failed.to_string()),
                                                    )
                                                })
                                            })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.set_tab(tab, cx)
                                            }))
                                    }),
                                ),
                        )
                    }),
            );
        let body = match self.tab {
            Tab::Summary => self.summary_view(&item, window, cx),
            Tab::Code => self.diff_view(),
            Tab::Checks => self.checks_view(cx),
        };
        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(title)
            .when_some(self.detail_error.clone(), |d, text| d.child(error(text)))
            .child(body)
            .into_any_element()
    }

    /// Copy link, which turns into a green tick reading "Copied link" for a
    /// moment once the address is on the clipboard.
    pub(super) fn copy_link_button(&self, item: &Item, cx: &mut Context<Self>) -> AnyElement {
        let copied = self
            .copied_link
            .as_ref()
            .filter(|(url, _)| *url == item.url)
            .map(|(_, press)| *press);
        let url = item.url.clone();
        let on_click = cx.listener(move |this, _, _, cx| this.copy_link(url.clone(), cx));
        let button = match copied {
            Some(press) => icon_action_with(
                "inbox-copy-link",
                "Copied link",
                action_icon("check", COPIED).with_animation(
                    ("inbox-copied-tick", press),
                    Animation::new(COPIED_POP).with_easing(|t| 1. - (1. - t).powi(3)),
                    |svg, t| svg.size(px(14. * (0.5 + 0.5 * t))).opacity(t),
                ),
                COPIED,
            )
            .text_color(content(1.)),
            None => icon_action("inbox-copy-link", "Copy link", "response-copy")
                .when(item.url.is_empty(), |d| d.opacity(0.4)),
        };
        button.on_click(on_click).into_any_element()
    }

    pub(super) fn pr_actions(&self, item: &Item, cx: &mut Context<Self>) -> AnyElement {
        let open = item.state.eq_ignore_ascii_case("open");
        let selected = self.merge_action.clone();
        let merge_label = match selected.as_str() {
            "squash" => "Squash and merge",
            "rebase" => "Rebase and merge",
            _ => "Merge pull request",
        };
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .when(open && !item.draft, |d| {
                d.child(
                    div()
                        .relative()
                        .h(px(28.))
                        .flex()
                        .items_stretch()
                        .rounded(px(6.))
                        .bg(content(1.))
                        .text_color(shell_fill())
                        .child(self.action_anchor.probe())
                        .child(
                            icon_action_ink(
                                "inbox-pr-merge",
                                merge_label,
                                "git-merge",
                                shell_fill(),
                            )
                            .px(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(shell_fill())
                            .when(self.acting, |d| d.opacity(0.4))
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if !this.acting {
                                        this.confirm_action = Some(selected.clone());
                                        this.action_error = None;
                                        this.action_notice = None;
                                        cx.notify();
                                    }
                                },
                            )),
                        )
                        .child(
                            div()
                                .id("inbox-merge-options")
                                .role(Role::Button)
                                .aria_label("Merge options")
                                .h_full()
                                .w(px(28.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .border_l_1()
                                .border_color(shell_fill().opacity(0.2))
                                .cursor_pointer()
                                .child(icon("chevron-down", 12.).text_color(shell_fill()))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if !this.acting {
                                        this.action_menu = !this.action_menu;
                                        cx.notify();
                                    }
                                })),
                        ),
                )
            })
            .children(
                [
                    (open && item.draft, "ready", "Ready for review"),
                    (open && !item.draft, "draft", "Convert to draft"),
                    (open, "close", "Close pull request"),
                    (
                        item.state.eq_ignore_ascii_case("closed"),
                        "reopen",
                        "Reopen pull request",
                    ),
                ]
                .into_iter()
                .filter(|(show, _, _)| *show)
                .map(|(_, action_name, label)| {
                    let anchor_ix = match action_name {
                        "ready" => 0,
                        "draft" => 1,
                        "close" => 2,
                        _ => 3,
                    };
                    icon_action(
                        format!("inbox-pr-{action_name}"),
                        label,
                        match action_name {
                            "draft" => "git-pull-request-draft",
                            "close" => "git-pull-request-closed",
                            _ => "git-pull-request",
                        },
                    )
                    .relative()
                    .child(self.pr_action_anchors[anchor_ix].probe())
                    .border_1()
                    .border_color(content(0.15))
                    .px(px(12.))
                    .when(self.acting, |d| d.opacity(0.4))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.acting {
                            this.confirm_action = Some(action_name.into());
                            this.action_error = None;
                            this.action_notice = None;
                            cx.notify();
                        }
                    }))
                }),
            )
            .when_some(self.action_notice.clone(), |d, notice| {
                d.child(
                    div()
                        .text_size(px(11.))
                        .text_color(content(0.55))
                        .child(notice),
                )
            })
            .into_any_element()
    }

    pub(super) fn summary_body_width(&self) -> Pixels {
        (self.summary_width.get().min(px(1024.)) - px(24.)).max(px(1.))
    }

    pub(super) fn summary_view(
        &self,
        item: &Item,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let discussion = self.thread.as_ref().is_some_and(|thread| {
            !thread.comments.is_empty() || !thread.commits.is_empty() || thread.truncated
        });
        let count = 2 + if discussion {
            1 + self.activity.len()
        } else {
            0
        };
        let mut rows = self.summary_rows.borrow_mut();
        if rows.keys.is_empty()
            || rows.revision != self.activity_revision
            || rows.discussion != discussion
        {
            let mut keys: Vec<SharedString> = vec!["intro".into()];
            if discussion {
                keys.push("activity".into());
                keys.extend(self.activity.iter().map(|activity| match activity {
                    Activity::Comment(comment) => format!("comment:{}", comment.id).into(),
                    Activity::Commits(commits) => format!("push:{}", commits[0].oid).into(),
                }));
            }
            keys.push("composer".into());
            // Retain unchanged prefix/suffix rows so refreshes preserve the
            // reader's item and pixel offset, including a focused composer.
            let prefix = rows
                .keys
                .iter()
                .zip(&keys)
                .take_while(|(old, new)| old == new)
                .count();
            let suffix = rows.keys[prefix..]
                .iter()
                .rev()
                .zip(keys[prefix..].iter().rev())
                .take_while(|(old, new)| old == new)
                .count();
            self.summary_scroll.splice_focusable(
                prefix..rows.keys.len() - suffix,
                (prefix..keys.len() - suffix)
                    .map(|ix| (ix + 1 == count).then(|| self.comment.focus_handle(cx))),
            );
            *rows = SummaryRows {
                revision: self.activity_revision,
                discussion,
                keys,
            };
        }
        drop(rows);
        let width = self.summary_width.clone();
        let entity = cx.entity();
        let item = item.clone();
        div()
            .id(format!("inbox-summary-{}", item.key()))
            .relative()
            .min_h_0()
            .min_w_0()
            .flex_1()
            .child(
                canvas(
                    move |bounds, _, _| {
                        width.set(bounds.size.width);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                list(self.summary_scroll.clone(), move |ix, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.summary_row(&item, ix, count, discussion, window, cx)
                    })
                })
                .size_full(),
            )
            .into_any_element()
    }

    pub(super) fn summary_row(
        &self,
        item: &Item,
        ix: usize,
        count: usize,
        discussion: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let can_post = !self.posting && !self.comment.read(cx).content.trim().is_empty();
        let body = if ix == 0 {
            div()
                .pt(px(20.))
                .pb(px(20.))
                .flex()
                .flex_col()
                .gap(px(20.))
                .when(!item.labels.is_empty(), |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(4.))
                            .children(item.labels.iter().map(|label| label_badge(label, false))),
                    )
                })
                .when(self.detail_loading && self.details.is_none(), |d| {
                    d.child(message("Loading description…"))
                })
                .when(
                    self.details
                        .as_ref()
                        .is_some_and(|d| d.body.trim().is_empty()),
                    |d| {
                        d.child(
                            div()
                                .text_size(px(13.))
                                .text_color(content(0.45))
                                .child("No description"),
                        )
                    },
                )
                .when_some(self.document.clone(), |d, document| {
                    d.child(
                        div()
                            .w_full()
                            .flex_shrink_0()
                            .child(document::DocumentElement::new(
                                document,
                                self.summary_body_width(),
                                None,
                            )),
                    )
                })
                .when(self.thread.is_none(), |d| {
                    d.when_some(self.thread_error.clone(), |d, text| d.child(error(text)))
                })
                .into_any_element()
        } else if ix + 1 == count {
            div()
                .pb(px(20.))
                .child(
                    div()
                        .w_full()
                        .flex_shrink_0()
                        .border_t_1()
                        .border_color(content(0.07))
                        .pt(px(20.))
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .when_some(self.reply.clone(), |d, (_, author)| {
                            d.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .text_size(px(12.))
                                    .text_color(content(0.5))
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .child(format!("Replying to {author}")),
                                    )
                                    .child(
                                        icon_button("inbox-cancel-reply", "Cancel reply", "close")
                                            .size(px(20.))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.reply = None;
                                                this.post_error = None;
                                                cx.notify();
                                            })),
                                    ),
                            )
                        })
                        .child(
                            div()
                                .rounded(px(6.))
                                .border_1()
                                .border_color(content(0.1))
                                .bg(content(0.05))
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .px(px(12.))
                                        .py(px(8.))
                                        .when(self.posting, |d| d.opacity(0.4))
                                        .child(self.comment.clone()),
                                )
                                .child(
                                    div().px(px(8.)).pb(px(8.)).flex().justify_end().child(
                                        action(
                                            "inbox-post-comment",
                                            if self.posting {
                                                "Posting..."
                                            } else if self.reply.is_some() {
                                                "Reply"
                                            } else {
                                                "Comment"
                                            },
                                        )
                                        .px(px(12.))
                                        .bg(content(1.))
                                        .text_color(shell_fill())
                                        .when(!can_post, |d| d.opacity(0.4))
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.post_comment(cx)),
                                        ),
                                    ),
                                ),
                        )
                        .when_some(self.post_error.clone(), |d, text| d.child(error(text))),
                )
                .into_any_element()
        } else if discussion && ix == 1 {
            div()
                .pb(px(if self.activity.is_empty() { 20. } else { 12. }))
                .children(self.discussion_heading(item, window, cx))
                .into_any_element()
        } else {
            div()
                .pb(px(if ix + 2 == count { 20. } else { 0. }))
                .child(self.discussion_row(item, ix - 2, window, cx))
                .into_any_element()
        };
        div()
            .id(("inbox-summary-row", ix))
            .w_full()
            .px(px(12.))
            .child(body)
            .into_any_element()
    }

    pub(super) fn diff_view(&self) -> AnyElement {
        div()
            .min_h_0()
            .flex_1()
            .flex()
            .flex_col()
            .when(self.extra_loading, |d| d.child(message("Loading changes…")))
            .when_some(self.extra_error.clone(), |d, text| d.child(error(text)))
            .when(self.diff.is_some(), |d| {
                d.child(div().flex_1().min_h_0().child(self.diff_view.clone()))
            })
            .when(self.diff.as_ref().is_some_and(|diff| diff.truncated), |d| {
                d.child(message(
                    "This diff is truncated. Open the pull request to see all changes.",
                ))
            })
            .into_any_element()
    }

    pub(super) fn checks_view(&self, cx: &mut Context<Self>) -> AnyElement {
        div().id("inbox-checks-scroll").min_h_0().flex_1().overflow_y_scroll().px(px(12.)).py(px(20.))
            .when(self.extra_loading, |d| d.child(message("Loading checks…")))
            .when_some(self.extra_error.clone(), |d, text| d.child(error(text)))
            .when_some(self.checks.clone(), |d, checks| {
                let failed = checks.checks.iter().filter(|c| matches!(c.state.to_lowercase().as_str(), "failure" | "failed" | "error" | "timed_out" | "cancelled" | "action_required")).cloned().collect::<Vec<_>>();
                d.when(checks.checks.is_empty(), |d| d.child(message("No checks reported")))
                    .when(!failed.is_empty(), |d| d.child(div().mb(px(12.)).flex().items_center().gap(px(8.))
                        .child(div().flex_1().text_size(px(12.)).text_color(content(0.7)).child(format!("{} failing checks", failed.len())))
                        .child(action("inbox-fix-checks", "Fix with agent").bg(content(0.1)).on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(mut item) = this.item() {
                                if item.project_path.is_empty() { item.project_path = this.start_project.to_string_lossy().into_owned(); }
                                let evidence = failed.iter().map(|check| format!("{}: {} {}", check.name, check.state, check.url.as_deref().unwrap_or(""))).collect::<Vec<_>>().join("\n");
                                let prompt = format!("Investigate and repair the failing CI checks for {} {}: {}\n{}\n\n{}\n\nRead the check logs, reproduce the failures, implement the fix, and run relevant checks.", item.repo, item.reference(), item.title, item.url, evidence);
                                cx.emit(Event::StartSession { item: Box::new(item), prompt });
                            }
                        })))))
                    .children(checks.checks.iter().enumerate().map(|(ix, check)| {
                        let state = check.state.to_lowercase();
                        let color: Hsla = if matches!(state.as_str(), "success" | "neutral" | "skipped") { rgb(0x4ade80).into() }
                            else if matches!(state.as_str(), "failure" | "failed" | "error" | "timed_out" | "cancelled" | "action_required") { rgb(0xf87171).into() }
                            else { rgb(0xfbbf24).into() };
                        let url = check.url.clone();
                        div().id(("inbox-check", ix)).py(px(12.)).border_b_1().border_color(content(0.07)).flex().items_center().gap(px(10.))
                            .child(icon(if state == "success" { "check-circle" } else { "circle-dot" }, 15.).text_color(color))
                            .child(div().flex_1().min_w_0().child(div().text_size(px(12.)).text_color(content(0.85)).child(check.name.clone()))
                                .child(div().text_size(px(11.)).text_color(content(0.4)).child(check.workflow.clone())))
                            .child(div().text_size(px(11.)).text_color(color).child(check.state.replace('_', " ").to_lowercase()))
                            .when(url.is_some(), |d| d.child(action(format!("inbox-check-link-{ix}"), "Details ↗").on_click(move |_, _, cx| { if let Some(url) = &url { cx.open_url(url); } })))
                    }))
            }).into_any_element()
    }
}
