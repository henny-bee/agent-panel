use super::*;

pub(super) fn timeline_event(name: &str, action: &str, time: &str) -> Div {
    div()
        .h(px(20.))
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(12.))
        .text_color(content(0.5))
        .child(
            div()
                .min_w_0()
                .truncate()
                .font_weight(FontWeight::MEDIUM)
                .text_color(content(1.))
                .child(if name.is_empty() {
                    "ghost".to_owned()
                } else {
                    name.to_owned()
                }),
        )
        .child(action.to_owned())
        .when(!time.is_empty(), |d| {
            d.child("·").child(relative_time(time))
        })
}

impl Inbox {
    pub(super) fn comment_card(
        &self,
        item: &Item,
        comment: &model::Comment,
        style: CommentStyle,
        width: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let nested = matches!(style, CommentStyle::Reply);
        let timeline = matches!(style, CommentStyle::Timeline);
        let has_body = !comment.body.trim().is_empty();
        let has_replies = !nested && !comment.replies.is_empty();
        let author = if comment.author.is_empty() {
            "ghost"
        } else {
            &comment.author
        };
        let reply_id = if item.provider == "linear" {
            comment.id.clone()
        } else if item.provider == "github" {
            comment.thread_id.clone()
        } else {
            String::new()
        };
        let state = comment.state.trim().to_uppercase();
        let review_color = if state == "APPROVED" {
            rgb(0x4ade80).into()
        } else if state == "CHANGES_REQUESTED" {
            rgb(0xf87171).into()
        } else {
            content(0.5)
        };
        let document = self
            .comments
            .iter()
            .find(|(cached, _)| cached.id == comment.id)
            .map(|(_, document)| document.clone());
        let clamped = timeline
            && !self.expanded_comments.contains(&comment.id)
            && (comment.body.len() > 1200 || comment.body.lines().count() > 12);
        let expandable =
            timeline && (comment.body.len() > 1200 || comment.body.lines().count() > 12);
        let header = div()
            .min_w_0()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .text_size(px(12.))
            .text_color(content(0.5))
            .when(!nested, |d| {
                d.px(px(12.))
                    .py(px(if timeline { 6. } else { 8. }))
                    .min_h(px(if timeline { 36. } else { 0. }))
            })
            .when(!nested && (has_body || has_replies), |d| {
                d.border_b_1().border_color(content(0.07))
            })
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(content(1.))
                    .child(if timeline {
                        div().child(author.to_string()).into_any_element()
                    } else {
                        avatar::person(
                            &item.provider,
                            author,
                            &comment.author_avatar_url,
                            20.,
                            window,
                            cx,
                        )
                    }),
            )
            .when(!state.is_empty(), |d| {
                d.child("·")
                    .child(div().text_color(review_color).child(review_label(&state)))
            })
            .when(!comment.path.is_empty(), |d| {
                d.child("·").child(div().min_w_0().truncate().child(format!(
                        "{}{}",
                        comment.path,
                        comment
                            .line
                            .map(|line| format!(":{line}"))
                            .unwrap_or_default()
                    )))
            })
            .when(comment.resolved, |d| {
                d.child("·")
                    .child(div().text_color(rgb(0x4ade80)).child("Resolved"))
            })
            .when(!comment.created_at.is_empty(), |d| {
                d.child("·").child(
                    action(
                        format!("inbox-comment-time-{}", comment.id),
                        relative_time(&comment.created_at),
                    )
                    .h_auto()
                    .px_0()
                    .text_color(content(0.5))
                    .on_click({
                        let url = comment.url.clone();
                        move |_, _, cx| {
                            if !url.is_empty() {
                                cx.open_url(&url);
                            }
                        }
                    }),
                )
            })
            .when(!reply_id.is_empty(), |d| {
                let name = author.to_string();
                d.child("·").child(
                    action(format!("inbox-reply-{}", comment.id), "Reply")
                        .h_auto()
                        .px_0()
                        .text_color(content(0.5))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.reply = Some((reply_id.clone(), name.clone()));
                            this.summary_scroll.scroll_to_end();
                            window.focus(&this.comment.focus_handle(cx), cx);
                            cx.notify();
                        })),
                )
            });
        div()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .when(!nested, |d| {
                d.rounded(px(6.))
                    .border_1()
                    .border_color(content(0.1))
                    .bg(content(0.05))
                    .overflow_hidden()
            })
            .child(header)
            .when(has_body, |d| {
                d.child(
                    div()
                        .when(nested, |d| d.mt(px(8.)))
                        .when(!nested, |d| d.px(px(12.)).py(px(10.)))
                        .child(
                            div()
                                .id(format!("inbox-comment-body-{}", comment.id))
                                .w_full()
                                .when(clamped, |d| d.max_h(px(180.)).overflow_hidden())
                                .when_some(document, |d, document| {
                                    d.child(document::DocumentElement::new(
                                        document,
                                        if nested {
                                            width
                                        } else {
                                            (width - px(26.)).max(px(1.))
                                        },
                                        clamped.then_some(px(180.)),
                                    ))
                                }),
                        )
                        .when(expandable, |d| {
                            let id = comment.id.clone();
                            d.child(
                                action(
                                    format!("inbox-expand-comment-{}", comment.id),
                                    if clamped { "Show more" } else { "Show less" },
                                )
                                .mt(px(6.))
                                .h_auto()
                                .px_0()
                                .text_color(content(0.5))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        if !this.expanded_comments.remove(&id) {
                                            this.expanded_comments.insert(id.clone());
                                        }
                                        this.summary_scroll.remeasure();
                                        cx.notify();
                                    },
                                )),
                            )
                        }),
                )
            })
            .when(has_replies, |d| {
                d.child(
                    div()
                        .border_t_1()
                        .border_color(content(0.07))
                        .px(px(12.))
                        .children(comment.replies.iter().enumerate().map(|(ix, reply)| {
                            div()
                                .py(px(10.))
                                .when(ix > 0, |d| d.border_t_1().border_color(content(0.07)))
                                .child(self.comment_card(
                                    item,
                                    reply,
                                    CommentStyle::Reply,
                                    (width - px(26.)).max(px(1.)),
                                    window,
                                    cx,
                                ))
                        })),
                )
            })
            .into_any_element()
    }

    pub(super) fn timeline_rail(
        &self,
        node: AnyElement,
        first: bool,
        last: bool,
        card: bool,
    ) -> Div {
        div()
            .w(px(20.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .items_center()
            .child(
                div()
                    .w(px(1.))
                    .h(px(if card { 8. } else { 0. }))
                    .flex_shrink_0()
                    .when(!first, |d| d.bg(content(0.1))),
            )
            .child(
                div()
                    .size(px(20.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(node),
            )
            .when(!last, |d| {
                d.child(div().w(px(1.)).flex_1().bg(content(0.1)))
            })
    }

    pub(in crate::inbox) fn rebuild_activity(&mut self) {
        self.activity.clear();
        self.activity_revision += 1;
        let Some(thread) = &self.thread else {
            return;
        };
        let timeline = self
            .item()
            .is_some_and(|item| item.kind == "pr" && item.provider == "github");
        if timeline {
            let mut entries = thread
                .comments
                .iter()
                .map(|comment| {
                    (
                        model::timestamp_ms(&comment.created_at),
                        Activity::Comment(std::sync::Arc::new(comment.clone())),
                    )
                })
                .chain(thread.commits.iter().map(|commit| {
                    (
                        model::timestamp_ms(&commit.committed_date),
                        Activity::Commits(vec![commit.clone()]),
                    )
                }))
                .collect::<Vec<_>>();
            entries.sort_by_key(|(at, _)| *at);
            let mut groups: Vec<Activity> = Vec::new();
            for (_, entry) in entries {
                match entry {
                    Activity::Commits(mut commits) => {
                        if let Some(Activity::Commits(previous)) = groups.last_mut()
                            && previous
                                .last()
                                .is_some_and(|c| c.author == commits[0].author)
                        {
                            previous.append(&mut commits);
                            continue;
                        }
                        groups.push(Activity::Commits(commits));
                    }
                    entry => groups.push(entry),
                }
            }
            self.activity = groups;
        } else {
            self.activity = thread
                .comments
                .iter()
                .cloned()
                .map(std::sync::Arc::new)
                .map(Activity::Comment)
                .collect();
        }
        self.summary_scroll.remeasure();
    }

    pub(super) fn discussion_row(
        &self,
        item: &Item,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entry = self.activity[ix].clone();
        let timeline = item.kind == "pr" && item.provider == "github";
        let width = self.summary_body_width();
        if !timeline {
            let Activity::Comment(comment) = entry else {
                unreachable!()
            };
            return div()
                .pb(px(if ix + 1 == self.activity.len() {
                    0.
                } else {
                    8.
                }))
                .child(self.comment_card(item, &comment, CommentStyle::Plain, width, window, cx))
                .into_any_element();
        }
        let first = ix == 0;
        let last = ix + 1 == self.activity.len();
        let mut activity = Vec::new();
        match entry {
            Activity::Comment(comment) => {
                let event = review_event(&comment);
                let state = comment.state.to_uppercase();
                let color = if state == "APPROVED" {
                    rgb(0x4ade80).into()
                } else if state == "CHANGES_REQUESTED" {
                    rgb(0xf87171).into()
                } else {
                    content(0.45)
                };
                let node = if event {
                    icon(
                        if state == "APPROVED" {
                            "check-circle"
                        } else if state == "CHANGES_REQUESTED" {
                            "circle-x"
                        } else {
                            "message-multiple"
                        },
                        16.,
                    )
                    .text_color(color)
                    .into_any_element()
                } else {
                    avatar::image(
                        &item.provider,
                        &comment.author,
                        &comment.author_avatar_url,
                        20.,
                        window,
                        cx,
                    )
                };
                let body = if event {
                    let verb = match state.as_str() {
                        "APPROVED" => "approved",
                        "CHANGES_REQUESTED" => "requested changes",
                        "DISMISSED" => "had a review dismissed",
                        _ => "reviewed",
                    };
                    timeline_event(&comment.author, verb, &comment.created_at).into_any_element()
                } else {
                    self.comment_card(
                        item,
                        &comment,
                        CommentStyle::Timeline,
                        (width - px(32.)).max(px(1.)),
                        window,
                        cx,
                    )
                };
                activity.push(
                    div()
                        .min_w_0()
                        .flex()
                        .gap(px(12.))
                        .child(self.timeline_rail(node, first, last, !event))
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .when(!last, |d| d.pb(px(12.)))
                                .child(body),
                        )
                        .into_any_element(),
                );
            }
            Activity::Commits(commits) => {
                let author = commits[0].author.clone();
                let time = commits
                    .last()
                    .map(|commit| commit.committed_date.clone())
                    .unwrap_or_default();
                let node = avatar::image(&item.provider, &author, "", 20., window, cx);
                activity.push(
                    div()
                        .min_w_0()
                        .flex()
                        .gap(px(12.))
                        .child(self.timeline_rail(node, first, false, false))
                        .child(div().min_w_0().flex_1().pb(px(12.)).child(timeline_event(
                            &author,
                            &format!(
                                "added {}",
                                if commits.len() == 1 {
                                    "a commit".into()
                                } else {
                                    format!("{} commits", commits.len())
                                }
                            ),
                            &time,
                        )))
                        .into_any_element(),
                );
                let commit_count = commits.len();
                for (commit_ix, commit) in commits.into_iter().enumerate() {
                    let final_commit = last && commit_ix + 1 == commit_count;
                    let url = commit.url.clone();
                    activity.push(
                        div()
                            .min_w_0()
                            .flex()
                            .gap(px(12.))
                            .child(
                                div()
                                    .w(px(20.))
                                    .flex_shrink_0()
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .child(div().h(px(6.)).w(px(1.)).bg(content(0.1)))
                                    .child(
                                        div()
                                            .size(px(8.))
                                            .rounded_full()
                                            .border_1()
                                            .border_color(content(0.35)),
                                    )
                                    .when(!final_commit, |d| {
                                        d.child(div().w(px(1.)).flex_1().bg(content(0.1)))
                                    }),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .when(!final_commit, |d| d.pb(px(12.)))
                                    .child(
                                        div()
                                            .id(format!("inbox-commit-{}", commit.oid))
                                            .role(Role::Button)
                                            .aria_label("Open commit")
                                            .h(px(20.))
                                            .flex()
                                            .items_center()
                                            .gap(px(12.))
                                            .text_size(px(12.))
                                            .cursor_pointer()
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .flex_1()
                                                    .truncate()
                                                    .text_color(content(0.7))
                                                    .child(commit.message_headline),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(11.))
                                                    .font_family("monospace")
                                                    .text_color(content(0.35))
                                                    .child(
                                                        commit
                                                            .oid
                                                            .chars()
                                                            .take(7)
                                                            .collect::<String>(),
                                                    ),
                                            )
                                            .on_click(move |_, _, cx| {
                                                if !url.is_empty() {
                                                    cx.open_url(&url);
                                                }
                                            }),
                                    ),
                            )
                            .into_any_element(),
                    );
                }
            }
        }
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .children(activity)
            .into_any_element()
    }

    pub(super) fn discussion_heading(
        &self,
        item: &Item,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let thread = self.thread.as_ref()?;
        if thread.comments.is_empty() && thread.commits.is_empty() && !thread.truncated {
            return None;
        }
        let timeline = item.kind == "pr" && item.provider == "github";
        let count = thread
            .comments
            .iter()
            .filter(|comment| !(timeline && review_event(comment)))
            .map(|comment| 1 + comment.replies.len())
            .sum::<usize>();
        Some(
            div()
                .w_full()
                .flex_shrink_0()
                .border_t_1()
                .border_color(content(0.07))
                .pt(px(20.))
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(px(12.))
                        .text_color(content(0.5))
                        .child(div().text_color(content(0.7)).child(if timeline {
                            "Activity".into()
                        } else {
                            format!(
                                "{count} {}",
                                if count == 1 { "comment" } else { "comments" }
                            )
                        }))
                        .when(timeline, |d| {
                            d.child(format!(
                                "{count} {}{}",
                                if count == 1 { "comment" } else { "comments" },
                                if thread.commits.is_empty() {
                                    String::new()
                                } else {
                                    format!(
                                        " · {} {}",
                                        thread.commits.len(),
                                        if thread.commits.len() == 1 {
                                            "commit"
                                        } else {
                                            "commits"
                                        }
                                    )
                                }
                            ))
                        })
                        .when(thread.truncated, |d| {
                            d.child(format!(
                                "Latest comments · more on {}",
                                model::provider_label(&item.provider)
                            ))
                        }),
                )
                .when_some(self.thread_error.clone(), |d, text| d.child(error(text)))
                .into_any_element(),
        )
    }
}
