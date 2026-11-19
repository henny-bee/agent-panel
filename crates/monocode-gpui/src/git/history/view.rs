use super::*;

impl History {
    /// VS Code's Graph hover: author, age, message, stat, refs, SHA and link.
    pub(super) fn card(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let card = self.card.as_ref()?;
        let item = self.items.iter().find(|i| i.commit.sha == card.sha)?;
        let commit = &item.commit;
        let details = card.details.as_ref();
        let message = details
            .map(|d| d.message.clone())
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| commit.subject.clone());
        let when = {
            use chrono::TimeZone;
            let ago = chrono::DateTime::from_timestamp(commit.timestamp, 0)
                .map(|at| crate::inbox::ui::relative_time(&at.to_rfc3339()))
                .unwrap_or_default();
            match chrono::Local.timestamp_opt(commit.timestamp, 0).single() {
                Some(at) => format!("{ago} ({})", at.format("%B %-d, %Y at %-I:%M %p")),
                None => ago,
            }
        };
        let rule = || div().h(px(1.)).flex_shrink_0().bg(content(0.1));
        let row = || div().px_3().py_2().flex().items_center().gap_1();
        let stat = details.filter(|d| d.files > 0).map(|d| {
            let plural =
                |n: i64, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
            row()
                .child(format!("{} changed", plural(d.files, "file")))
                .when(d.additions > 0, |r| {
                    r.child(
                        div()
                            .text_color(rgb(DIFF_ADD))
                            .child(format!(", {}(+)", plural(d.additions, "insertion"))),
                    )
                })
                .when(d.deletions > 0, |r| {
                    r.child(
                        div()
                            .text_color(rgb(DIFF_DEL))
                            .child(format!(", {}(-)", plural(d.deletions, "deletion"))),
                    )
                })
        });
        let url = details.and_then(|d| d.url.clone());
        let github = url
            .as_deref()
            .is_some_and(|u| u.starts_with("https://github.com/"));
        let sha = commit.sha.clone();
        let weak = cx.entity().downgrade();
        Some(
            popover("history-card", Anchor::Point(card.at))
                .width(360.)
                .on_dismiss(move |_, cx| {
                    let _ = weak.update(cx, |this, cx| this.close_card(cx));
                })
                .child(
                    div()
                        .id("history-card-body")
                        .role(Role::Dialog)
                        .aria_label("Commit details")
                        .flex()
                        .flex_col()
                        .text_size(px(12.))
                        .text_color(content(0.85))
                        .child(
                            row()
                                .flex_wrap()
                                .child(icon("account-user", 14.).text_color(content(0.55)))
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(accent_color())
                                        .child(format!("{},", commit.author)),
                                )
                                .child(icon("clock", 12.).ml_1().text_color(content(0.55)))
                                .child(div().text_color(content(0.55)).child(when)),
                        )
                        .child(div().px_3().pb_2().child(message))
                        .when_some(stat, |d, stat| d.child(rule()).child(stat))
                        .when(!item.row.refs.is_empty(), |d| {
                            d.child(rule()).child(
                                row()
                                    .flex_wrap()
                                    .children(item.row.refs.iter().map(ref_pill)),
                            )
                        })
                        .child(rule())
                        .child(
                            row()
                                .gap_2()
                                .child(icon("git-commit", 14.).text_color(content(0.55)))
                                .child(
                                    div()
                                        .font_family(crate::ui::fonts::mono())
                                        .text_color(accent_color())
                                        .child(commit.short_sha.clone()),
                                )
                                .child(
                                    icon_action(
                                        "history-card-copy",
                                        "Copy commit SHA",
                                        "copy",
                                        true,
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                sha.clone(),
                                            ));
                                            this.close_card(cx);
                                        },
                                    )),
                                )
                                .when_some(url, |d, url| {
                                    d.child(div().text_color(content(0.3)).child("|")).child(
                                        div()
                                            .id("history-card-open")
                                            .role(Role::Link)
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .cursor_pointer()
                                            .text_color(accent_color())
                                            .hover(|s| s.underline())
                                            .child(
                                                icon(
                                                    if github {
                                                        "inbox-github"
                                                    } else {
                                                        "external-link"
                                                    },
                                                    14.,
                                                )
                                                .text_color(accent_color()),
                                            )
                                            .child(if github {
                                                "Open on GitHub"
                                            } else {
                                                "Open in browser"
                                            })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.open_url(&url);
                                                this.close_card(cx);
                                            })),
                                    )
                                }),
                        ),
                )
                .into_any_element(),
        )
    }

    pub(super) fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let head = self.items.iter().find(|i| i.commit.head);
        let branch = head.and_then(|h| {
            h.commit
                .refs
                .iter()
                .find(|r| r.kind == "local")
                .map(|r| r.name.clone())
        });
        let label: SharedString = branch
            .clone()
            .or_else(|| head.map(|h| h.commit.short_sha.clone()))
            .unwrap_or_else(|| "History".into())
            .into();
        let can_refresh = self.available() && !self.loading;
        div()
            .flex_shrink_0()
            .child(pane_header("history-toolbar", "History").map(|d| {
                if self.search_open {
                    d.key_context("HistorySearch")
                        .on_action(cx.listener(|this, _: &CloseSearch, window, cx| {
                            this.close_search(window, cx)
                        }))
                        .child(icon("search", 14.).text_color(content(0.45)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .font_weight(FontWeight::NORMAL)
                                .text_color(content(1.))
                                .child(self.search.clone()),
                        )
                        .child(
                            header_action(
                                "history-close-search",
                                "Close history search",
                                "x",
                                12.,
                                true,
                                false,
                            )
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_search(window, cx)),
                            ),
                        )
                } else if self.section {
                    let open = self.open;
                    let code = self.code;
                    d.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .id("history-section")
                                    .role(Role::Button)
                                    .aria_label(if code { "Code" } else { "Graph" })
                                    .aria_expanded(open)
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .child(
                                        icon(
                                            if open {
                                                "chevron-down"
                                            } else {
                                                "chevron-right"
                                            },
                                            14.,
                                        )
                                        .text_color(content(0.5)),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open = !this.open;
                                        if this.open && this.code {
                                            this.load_languages(cx);
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                section_tab("graph-tab", "GRAPH", !code)
                                    .on_click(cx.listener(|this, _, _, cx| this.choose(false, cx))),
                            )
                            .child(div().w(px(1.)).h(px(10.)).mx(px(4.)).bg(content(0.15)))
                            .child(
                                section_tab("code-tab", "CODE", code)
                                    .on_click(cx.listener(|this, _, _, cx| this.choose(true, cx))),
                            ),
                    )
                    .when(open && !code, |d| d.child(self.actions(can_refresh, cx)))
                } else {
                    d.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(icon("git-branch-hi", 14.).text_color(content(0.45)))
                            .child(div().min_w_0().truncate().child(label)),
                    )
                    .child(self.actions(can_refresh, cx))
                }
            }))
            .when_some(self.error.clone(), |d, error| {
                d.child(
                    div()
                        .id("history-error")
                        .role(Role::Alert)
                        .border_b_1()
                        .border_color(stroke())
                        .px_3()
                        .py_2()
                        .text_size(px(11.))
                        .text_color(rgb(RED_400))
                        .child(error),
                )
            })
    }

    /// Search and refresh, shared by the tab's toolbar and the section header.
    pub(super) fn actions(&self, can_refresh: bool, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(2.))
            .child(
                header_action(
                    "history-search",
                    "Search history",
                    "search",
                    14.,
                    true,
                    false,
                )
                .on_click(cx.listener(|this, _, window, cx| this.open_search(window, cx))),
            )
            .child(
                header_action(
                    "history-refresh",
                    "Refresh history",
                    "refresh-cw",
                    14.,
                    can_refresh,
                    self.loading,
                )
                .when(can_refresh, |d| {
                    d.on_click(cx.listener(|this, _, _, cx| this.load(cx)))
                }),
            )
    }

    pub(super) fn row(&self, ix: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let item = &self.items[ix];
        let commit = &item.commit;
        let active = self.selected.as_deref() == Some(commit.sha.as_str());
        let hovered = self.hovered == Some(ix);
        let filtered = !self.query.trim().is_empty();
        let head = item.row.kind == NodeKind::Head;
        let badge = item
            .row
            .refs
            .iter()
            .find(|r| r.color.is_some())
            .or(item.row.refs.first())
            .cloned();
        let date = short_date(commit.timestamp);
        let graph = item.graph.clone();
        let width = if filtered {
            SWIMLANE_WIDTH * 2.
        } else {
            graph.width
        };
        let title: SharedString = {
            let mut t = format!("{} {}", commit.short_sha, commit.subject);
            if !commit.author.is_empty() {
                t.push_str(&format!(" — {}", commit.author));
            }
            t.into()
        };
        div()
            .id(("history-row", ix))
            .role(Role::Button)
            .aria_label(title)
            .aria_selected(active)
            .h(px(ROW))
            .w_full()
            .min_w_0()
            .flex()
            .items_stretch()
            .pl_1()
            .pr_3()
            .text_color(content(1.))
            .when(active, |d| d.bg(selection()))
            .when(!active, |d| d.hover(|s| s.bg(content(0.05))))
            .on_hover(cx.listener(move |this, hovering: &bool, _, cx| {
                let next = if *hovering {
                    Some(ix)
                } else if this.hovered == Some(ix) {
                    None
                } else {
                    this.hovered
                };
                if next != this.hovered {
                    this.hovered = next;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.select(ix, cx)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_card(ix, event.position, cx);
                }),
            )
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        paint_graph(&graph, bounds, filtered, head, active, hovered, window)
                    },
                )
                .w(px(width))
                .h(px(ROW))
                .flex_shrink_0(),
            )
            .child(
                div()
                    .ml_1()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .gap(px(2.))
                    .overflow_hidden()
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.))
                                    .line_height(px(16.))
                                    .when(head, |d| d.font_weight(FontWeight::MEDIUM))
                                    .child(if commit.subject.is_empty() {
                                        commit.short_sha.clone()
                                    } else {
                                        commit.subject.clone()
                                    }),
                            )
                            .when_some(badge, |d, badge| d.child(ref_pill(&badge))),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(10.))
                            .line_height(px(14.))
                            .text_color(content(0.4))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(commit.author.clone()),
                            )
                            .when_some(date, |d, date| d.child(div().flex_shrink_0().child(date)))
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .font_family(crate::ui::fonts::mono())
                                    .text_color(content(0.3))
                                    .child(commit.short_sha.clone()),
                            ),
                    ),
            )
    }
}

/// GRAPH and CODE in the section header: the chosen one bright and underlined.
pub(super) fn section_tab(
    id: &'static str,
    label: &'static str,
    active: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(Role::Tab)
        .aria_label(label)
        .aria_selected(active)
        .group(id)
        .h_full()
        .flex()
        .items_center()
        .child(
            div()
                .py(px(1.))
                .border_b_1()
                .text_size(px(10.))
                .font_weight(FontWeight::SEMIBOLD)
                .map(|d| {
                    if active {
                        d.border_color(content(0.75)).text_color(content(0.85))
                    } else {
                        d.border_color(gpui::transparent_black())
                            .text_color(content(0.4))
                            .group_hover(id, |s| s.text_color(content(0.7)))
                    }
                })
                .child(label),
        )
}

/// HistoryToolbar's 24px actions: 50% ink, 5% hover.
pub(super) fn header_action(
    id: &'static str,
    label: &'static str,
    path: &'static str,
    side: f32,
    enabled: bool,
    spinning: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .group(id)
        .size(px(24.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .when(!enabled, |d| d.opacity(0.4))
        .when(enabled, |d| d.hover(|s| s.bg(content(0.05))))
        .child(if spinning {
            spinner(id, path, side)
        } else {
            icon(path, side)
                .text_color(content(0.5))
                .when(enabled, |i| {
                    i.group_hover(id, |s| s.text_color(content(1.)))
                })
                .into_any_element()
        })
}

pub(super) fn ref_pill(r: &graph::GraphRef) -> impl IntoElement {
    let local = r.kind == "local";
    div()
        .ml_1()
        .h(px(14.))
        .min_w_0()
        .max_w(px(104.))
        .flex_shrink_0()
        .self_center()
        .flex()
        .items_center()
        .gap(px(2.))
        .px(px(6.))
        .rounded_full()
        .text_size(px(10.))
        .line_height(px(10.))
        .map(|d| match r.color {
            Some(color) => d.bg(rgb(color)).text_color(rgb(BACKGROUND_BASE)),
            None => d.bg(content(0.1)).text_color(content(0.55)),
        })
        .when(local, |d| {
            d.child(icon("git-branch-2", 10.).text_color(match r.color {
                Some(_) => rgb(BACKGROUND_BASE).into(),
                None => content(0.55),
            }))
        })
        .child(div().min_w_0().truncate().child(r.name.clone()))
}

/// The row SVG and index.css's `.git-history-item` cutouts: strokes in the
/// background colour, cleared or tinted on hover and selection.
pub(super) fn paint_graph(
    graph: &RowGraph,
    bounds: Bounds<gpui::Pixels>,
    filtered: bool,
    head: bool,
    active: bool,
    hovered: bool,
    window: &mut Window,
) {
    let origin = bounds.origin;
    if !filtered {
        for path in &graph.paths {
            let color = soft(path.color).opacity(0.8);
            graph::stroke_path(&path.d, origin, path.stroke_width, color, window);
        }
    }
    let count = graph.circles.len();
    for (ix, circle) in graph.circles.iter().enumerate() {
        let cx = if filtered { SWIMLANE_WIDTH } else { circle.cx };
        let centre = origin + point(px(cx), px(circle.cy));
        let base: Hsla = rgb(BACKGROUND_BASE).into();
        let tint = if active {
            over_base(0.1)
        } else if hovered {
            over_base(0.05)
        } else {
            base
        };
        let stroke = match ix {
            0 if hovered => None,
            1 if active || hovered => Some(tint),
            _ => Some(base),
        };
        let fill_color = if head && ix + 1 == count {
            Some(tint)
        } else {
            circle.fill.map(soft)
        };
        let disc = |r: f32, color: Hsla, window: &mut Window| {
            window.paint_quad(
                fill(
                    Bounds::new(centre - point(px(r), px(r)), size(px(r * 2.), px(r * 2.))),
                    color,
                )
                .corner_radii(px(r)),
            );
        };
        if let Some(color) = fill_color {
            disc(circle.r, color, window);
        }
        if let Some(stroke) = stroke {
            let outer = circle.r + circle.stroke_width / 2.;
            window.paint_quad(gpui::quad(
                Bounds::new(
                    centre - point(px(outer), px(outer)),
                    size(px(outer * 2.), px(outer * 2.)),
                ),
                px(outer),
                gpui::transparent_black(),
                px(circle.stroke_width),
                stroke,
                gpui::BorderStyle::Solid,
            ));
        }
    }
}

impl Render for History {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let empty: Option<(&'static str, bool)> = if !self.available() {
            Some(("No project folder", false))
        } else if self.error.is_some() && self.items.is_empty() {
            None
        } else if self.items.is_empty() {
            Some((
                if self.loading {
                    "Loading history…"
                } else {
                    "No commits yet"
                },
                false,
            ))
        } else if self.visible.is_empty() {
            Some(("No matching commits", true))
        } else {
            None
        };
        let list_len = self.visible.len();
        let entity = cx.entity();
        div()
            .id("history")
            .role(if self.section {
                Role::Group
            } else {
                Role::TabPanel
            })
            .aria_label(if self.section { "Graph" } else { "History" })
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(self.toolbar(cx))
            .children(self.card(cx))
            .when(self.code && self.open, |d| {
                d.child(languages::view(self.languages.as_ref()))
            })
            .when(
                (!self.section || self.open) && !self.code,
                |d| match empty {
                    Some((text, centred)) => d.child(
                        div()
                            .px_3()
                            .when(centred, |d| d.py_4().text_center())
                            .when(!centred, |d| d.py_2())
                            .text_size(px(12.))
                            .text_color(content(0.45))
                            .child(text),
                    ),
                    None if self.items.is_empty() => d,
                    None => d.child(
                        uniform_list("history-rows", list_len, move |range, _, cx| {
                            entity.update(cx, |this, cx| {
                                range
                                    .filter_map(|i| this.visible.get(i).copied())
                                    .map(|ix| this.row(ix, cx).into_any_element())
                                    .collect::<Vec<_>>()
                            })
                        })
                        .track_scroll(&self.scroll)
                        .flex_1()
                        .min_h_0(),
                    ),
                },
            )
    }
}
