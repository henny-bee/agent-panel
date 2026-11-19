use super::*;

impl DiffView {
    pub(super) fn overview(&self, cx: &mut Context<Self>) -> AnyElement {
        use crate::editor::scrollbar::WIDTH;
        let total = self.total.max(1.);
        let top = self.top_px();
        let active = self.overview_drag.is_some() || self.overview_hovered;
        div()
            .id("diff-overview")
            .debug_selector(|| "diff-overview".into())
            .absolute()
            .right_0()
            .top_0()
            .bottom_0()
            .w(px(WIDTH))
            .cursor(gpui::CursorStyle::Arrow)
            .child(self.overview_bounds.probe())
            .child(
                canvas(
                    move |bounds, _, _| bounds,
                    move |bounds, _, window, _| {
                        let m = crate::editor::scrollbar::Metrics::new(
                            top,
                            total,
                            bounds.size.height.as_f32(),
                            bounds.size.height.as_f32(),
                        );
                        window.paint_quad(gpui::fill(
                            Bounds::new(bounds.origin, gpui::size(px(1.), bounds.size.height)),
                            content(0.07),
                        ));
                        if m.scrollable {
                            window.paint_quad(
                                gpui::fill(
                                    Bounds::new(
                                        bounds.origin + gpui::point(px(3.), px(m.thumb_top)),
                                        gpui::size(px(WIDTH - 6.), px(m.thumb_height)),
                                    ),
                                    content(if active { 0.38 * 0.88 } else { 0.3 * 0.72 }),
                                )
                                .corner_radii(px(2.)),
                            );
                        }
                    },
                )
                .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    let Some(bounds) = this.overview_bounds.get() else {
                        return;
                    };
                    let y = (event.position.y - bounds.top()).as_f32();
                    let height = bounds.size.height.as_f32();
                    let m = this.overview_metrics(height);
                    if !m.scrollable {
                        return;
                    }
                    this.pending = None;
                    this.overview_drag = Some(
                        if event.position.x >= bounds.left() + px(3.)
                            && event.position.x <= bounds.right() - px(3.)
                            && y >= m.thumb_top
                            && y <= m.thumb_top + m.thumb_height
                        {
                            y - m.thumb_top
                        } else {
                            m.thumb_height / 2.
                        },
                    );
                    this.overview_scroll(event.position.y, cx);
                    cx.notify();
                }),
            )
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.overview_hovered = *hovered;
                cx.notify();
            }))
            .into_any_element()
    }

    pub(super) fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.file_count.unwrap_or(self.files.len());
        let (additions, deletions) = self.totals.unwrap_or_else(|| {
            self.files
                .iter()
                .fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions))
        });
        let current = self.current_hunk();
        let total = self.hunks.len();
        let position = if total > 0 {
            format!("{}/{total}", current.map_or(0, |c| c + 1))
        } else {
            "0/0".into()
        };
        let can_prev = total > 0 && current.is_some_and(|c| c > 0);
        let can_next = total > 0 && current.is_none_or(|c| c + 1 < total);
        div()
            .id("diff-tools")
            .role(Role::Toolbar)
            .aria_label("Diff tools")
            .h(px(32.))
            .flex_shrink_0()
            .min_w_0()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .border_b_1()
            .border_color(stroke())
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(content(0.85))
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(content(0.65))
                    .child(if count == 1 {
                        "1 file".to_owned()
                    } else {
                        format!("{count} files")
                    }),
            )
            .child(diff_counts(additions, deletions))
            .child(
                div()
                    .ml_auto()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .child(
                        tool_button("previous-hunk", "Previous hunk", "chevron-up", can_prev)
                            .when(can_prev, |d| {
                                d.on_click(cx.listener(|this, _, _, cx| this.navigate(-1, cx)))
                            }),
                    )
                    .child(
                        div()
                            .min_w(px(36.))
                            .text_center()
                            .text_size(px(10.))
                            .font_weight(FontWeight::NORMAL)
                            .font_features(tabular())
                            .text_color(content(0.4))
                            .child(position),
                    )
                    .child(
                        tool_button("next-hunk", "Next hunk", "chevron-down", can_next)
                            .when(can_next, |d| {
                                d.on_click(cx.listener(|this, _, _, cx| this.navigate(1, cx)))
                            }),
                    )
                    .child(div().mx_1().h(px(12.)).w(px(1.)).bg(stroke()))
                    .child(
                        tool_button("expand-all", "Expand all files", "unfold-vertical", true)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open = this.files.iter().map(|f| f.id.clone()).collect();
                                this.rebuild(cx);
                            })),
                    )
                    .child(
                        tool_button(
                            "collapse-all",
                            "Collapse all files",
                            "fold-vertical",
                            !self.open.is_empty(),
                        )
                        .when(!self.open.is_empty(), |d| {
                            d.on_click(cx.listener(|this, _, _, cx| {
                                this.open.clear();
                                this.rebuild(cx);
                            }))
                        }),
                    ),
            )
    }

    pub(super) fn file_header(
        &self,
        file: usize,
        sticky: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let f = &self.files[file];
        let expanded = self.open.contains(&f.id);
        let busy = self.busy_id.as_ref() == Some(&f.id);
        let name = f.path.rsplit('/').next().unwrap_or(&f.path).to_owned();
        let directory = f
            .label
            .strip_suffix(&name)
            .map(|d| d.trim_end_matches('/').to_owned())
            .unwrap_or_default();
        let id = f.id.clone();
        let (discard_id, stage_id) = (id.clone(), id.clone());
        div()
            .id(("diff-file", file * 2 + usize::from(sticky)))
            .h(px(FILE_HEADER))
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .border_b_1()
            .border_color(stroke())
            .hover(|s| s.bg(content(0.08)))
            .child(
                div()
                    .id(("diff-file-toggle", file * 2 + usize::from(sticky)))
                    .role(Role::Button)
                    .aria_label(SharedString::from(format!(
                        "{} {}",
                        if expanded { "Collapse" } else { "Expand" },
                        f.label
                    )))
                    .aria_expanded(expanded)
                    .h_full()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        icon(
                            if expanded {
                                "chevron-down"
                            } else {
                                "chevron-right"
                            },
                            14.,
                        )
                        .text_color(content(0.4)),
                    )
                    .child(file_type_icon(&name, false, false, 15.))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(content(0.9))
                            .child(if directory.is_empty() {
                                f.label.clone()
                            } else {
                                name.clone()
                            }),
                    )
                    .when(!directory.is_empty(), |d| {
                        d.child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.))
                                .text_color(content(0.4))
                                .child(directory),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(file, cx))),
            )
            .child(diff_counts(f.additions, f.deletions))
            .when(f.can_discard, |d| {
                d.child(
                    tool_button(("discard-file", file), "Discard file", "undo", !busy).when(
                        !busy,
                        |d| {
                            d.on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(DiffEvent::DiscardFile(discard_id.clone()))
                            }))
                        },
                    ),
                )
            })
            .when(f.can_stage, |d| {
                d.child(
                    tool_button(("stage-file", file), "Stage file", "check", !busy).when(
                        !busy,
                        |d| {
                            d.on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(DiffEvent::StageFile(stage_id.clone()))
                            }))
                        },
                    ),
                )
            })
            .into_any_element()
    }

    pub(super) fn row(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(row) = self.rows.get(ix).cloned() else {
            return div().into_any_element();
        };
        let gutter = self.gutter_width();
        let number_width = self.number_width;
        let accent: Hsla = accent_color();
        match row.kind {
            RowKind::File => self.file_header(row.file, false, cx),
            RowKind::Notice(text) => div()
                .h(px(NOTICE))
                .w_full()
                .flex()
                .items_center()
                .child(div().w(px(gutter)).flex_shrink_0())
                .child(
                    div()
                        .px_3()
                        .text_size(px(12.))
                        .text_color(content(0.45))
                        .child(text),
                )
                .into_any_element(),
            RowKind::Hunk(text) => div()
                .h(px(HUNK))
                .w_full()
                .flex()
                .items_center()
                .bg(accent.opacity(0.06))
                .border_t_1()
                .border_b_1()
                .border_color(accent.opacity(0.1))
                .child(
                    div()
                        .w(px(gutter))
                        .flex_shrink_0()
                        .flex()
                        .pl(px(3.))
                        .text_size(px(9.))
                        .text_color(content(0.35))
                        .children(["Old", "New"].map(|label| {
                            div().w(px(number_width)).pr_2().text_right().child(label)
                        })),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .px_3()
                        .font_family(crate::ui::fonts::mono())
                        .text_size(px(11.))
                        .text_color(content(0.5))
                        .child(text),
                )
                .into_any_element(),
            RowKind::Line { block, line, stage } => {
                let file = &self.files[row.file];
                let Some(l) = file.blocks.get(block).and_then(|b| b.lines().get(line)) else {
                    return div().h(px(LINE)).into_any_element();
                };
                let (added, removed) = (l.kind == LineKind::Add, l.kind == LineKind::Del);
                let ink: Option<Hsla> = if added {
                    Some(rgb(DIFF_ADD).into())
                } else if removed {
                    Some(rgb(DIFF_DEL).into())
                } else {
                    None
                };
                let hovered = self.hovered == Some(ix);
                let busy = self.busy_id.as_ref() == Some(&file.id);
                let text = expand_tabs(&l.text).into_owned();
                let styles: Vec<_> = self
                    .tokens(file, block, line)
                    .unwrap_or(&[])
                    .iter()
                    .filter(|span| span.range.end <= text.len())
                    .map(|span| {
                        (
                            span.range.clone(),
                            HighlightStyle {
                                color: Some(rgb(span.color).into()),
                                ..Default::default()
                            },
                        )
                    })
                    .collect();
                let number = if removed { l.old_number } else { l.new_number };
                let comment_line = l.clone();
                let file_ix = row.file;
                let file_id = file.id.clone();
                let slot = self.action_slots.entry(ix).or_default().clone();
                let gutter_ink = ink.unwrap_or(content(0.4));
                let numbers = |n: Option<u32>, hide: bool| {
                    div()
                        .w(px(number_width))
                        .flex_shrink_0()
                        .pr_2()
                        .text_right()
                        .text_size(px(11.))
                        .font_features(tabular())
                        .when(hide, |d| d.invisible())
                        .child(n.map(|n| n.to_string()).unwrap_or_default())
                };
                let scroll_x = self.scroll_x;
                let _ = window;
                div()
                    .id(("diff-line", ix))
                    .h(px(LINE))
                    .w_full()
                    .flex()
                    .line_height(px(LINE))
                    .when_some(ink, |d, ink| d.bg(ink.opacity(0.06)))
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
                    .child(
                        div()
                            .relative()
                            .w(px(gutter))
                            .flex_shrink_0()
                            .flex()
                            .text_color(gutter_ink)
                            .child(
                                div()
                                    .w(px(3.))
                                    .flex_shrink_0()
                                    .when_some(ink, |d, ink| d.bg(ink.opacity(0.55))),
                            )
                            .child(numbers(l.old_number, hovered))
                            .child(numbers(l.new_number, false))
                            .child(
                                div()
                                    .w(px(24.))
                                    .flex_shrink_0()
                                    .text_center()
                                    .text_size(px(12.))
                                    .when(hovered && stage.is_some(), |d| d.invisible())
                                    .child(if added {
                                        "+"
                                    } else if removed {
                                        "−"
                                    } else {
                                        ""
                                    }),
                            )
                            .when(hovered, |d| {
                                d.child(
                                    div()
                                        .id(("diff-comment", ix))
                                        .role(Role::Button)
                                        .aria_label(SharedString::from(format!(
                                            "Comment on line {}",
                                            number.map(|n| n.to_string()).unwrap_or_default()
                                        )))
                                        .absolute()
                                        .top_0()
                                        .bottom_0()
                                        .left(px(3.))
                                        .w(px(number_width))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(3.))
                                        .hover(|s| s.bg(content(0.12)))
                                        .child(slot.probe())
                                        .child(icon("comment-add", 12.).text_color(content(1.)))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.open_comment(
                                                file_ix,
                                                comment_line.clone(),
                                                ix,
                                                window,
                                                cx,
                                            )
                                        })),
                                )
                                .when_some(stage, |d, chunk| {
                                    d.child(
                                        div()
                                            .id(("diff-stage-hunk", ix))
                                            .role(Role::Button)
                                            .aria_label("Stage hunk")
                                            .absolute()
                                            .top_0()
                                            .bottom_0()
                                            .right_0()
                                            .w(px(24.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(3.))
                                            .when(busy, |d| d.opacity(0.35))
                                            .when(!busy, |d| d.hover(|s| s.bg(content(0.12))))
                                            .child(icon("check", 12.).text_color(content(1.)))
                                            .when(!busy, |d| {
                                                d.on_click(cx.listener(move |_, _, _, cx| {
                                                    cx.emit(DiffEvent::StageHunk(
                                                        file_id.clone(),
                                                        chunk,
                                                    ))
                                                }))
                                            }),
                                    )
                                })
                            }),
                    )
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_hidden()
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .left(px(-scroll_x))
                                    .pl_3()
                                    .pr_6()
                                    .whitespace_nowrap()
                                    .font_family(crate::ui::fonts::mono())
                                    .text_size(px(CODE_SIZE))
                                    .text_color(content(0.92))
                                    .child(StyledText::new(text).with_highlights(styles)),
                            ),
                    )
                    .into_any_element()
            }
        }
    }

    /// The header of the file whose body is at the top, pinned until the
    /// next file's header pushes it away.
    pub(super) fn sticky_header(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let top = self.list.logical_scroll_top();
        let row = self.rows.get(top.item_ix)?;
        if matches!(row.kind, RowKind::File) && top.offset_in_item <= px(0.) {
            return None;
        }
        let file = row.file;
        // A collapsed file has no body for its header to stick over.
        if !self.open.contains(&self.files[file].id) {
            return None;
        }
        let top_px = self.top_px();
        let next = self
            .file_rows
            .get(file + 1)
            .map(|&r| self.offsets[r] - top_px - FILE_HEADER)
            .unwrap_or(0.)
            .min(0.);
        Some(
            div()
                .absolute()
                .top(px(next))
                .left_0()
                .right_0()
                .occlude()
                .child(blur())
                .child(self.file_header(file, true, cx))
                .into_any_element(),
        )
    }

    pub(super) fn horizontal_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let max = self.max_scroll_x();
        let lane = self.lane_width();
        let code = self.code_width();
        let track = lane.max(1.);
        let thumb = if code > 0. {
            (lane / code * track).max(24.)
        } else {
            track
        };
        let left = if max > 0. {
            self.scroll_x / max * (track - thumb)
        } else {
            0.
        };
        let gutter = self.gutter_width();
        div()
            .id("diff-horizontal-scroll")
            .role(Role::ScrollBar)
            .aria_label("Scroll diff horizontally")
            .h(px(14.))
            .flex_shrink_0()
            .relative()
            .when(max > 0., |d| {
                d.child(
                    div()
                        .id("diff-horizontal-thumb")
                        .absolute()
                        .top(px(4.))
                        .h(px(6.))
                        .left(px(gutter + left))
                        .w(px(thumb))
                        .rounded_full()
                        .bg(content(if self.thumb_drag.is_some() { 0.35 } else { 0.2 }))
                        .hover(|s| s.bg(content(0.35)))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, event: &MouseDownEvent, _, cx| {
                                this.thumb_drag = Some((event.position.x, this.scroll_x));
                                cx.stop_propagation();
                                cx.notify();
                            }),
                        ),
                )
            })
    }
}

/// UnifiedDiffView's IconButton: 24px, 45% ink, 30% when disabled.
pub(super) fn tool_button(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    path: &'static str,
    enabled: bool,
) -> gpui::Stateful<gpui::Div> {
    let id = id.into();
    let group: SharedString = format!("diff-tool-{id:?}").into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .group(group.clone())
        .size(px(24.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .when(!enabled, |d| d.opacity(0.3))
        .when(enabled, |d| d.hover(|s| s.bg(content(0.05))))
        .child(
            icon(path, 14.)
                .text_color(content(0.45))
                .when(enabled, |i| {
                    i.group_hover(group, |s| s.text_color(content(1.)))
                }),
        )
}

impl Render for DiffView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.files.is_empty() {
            return div()
                .id("diff-review")
                .key_context("DiffReview")
                .track_focus(&self.focus)
                .px_4()
                .py_6()
                .text_size(px(13.))
                .text_color(content(0.45))
                .child("No file changes")
                .into_any_element();
        }
        self.char_width(window);
        self.clamp_scroll_x();
        // Files with lines on screen get highlighted.
        let top = self.list.logical_scroll_top().item_ix;
        let height = self.viewport.get().map_or(800., |b| b.size.height.as_f32());
        let end = self
            .offsets
            .partition_point(|o| *o <= self.offsets.get(top).copied().unwrap_or(0.) + height);
        let mut visible: Vec<usize> = self.rows[top.min(self.rows.len())..end.min(self.rows.len())]
            .iter()
            .filter(|r| matches!(r.kind, RowKind::Line { .. }))
            .map(|r| r.file)
            .collect();
        visible.dedup();
        self.request_highlights(&visible, cx);
        let entity = cx.entity();
        let viewport = self.viewport.clone();
        let view_id = cx.entity_id();
        let weak = cx.weak_entity();
        let sticky = self.sticky_header(cx);
        let comment = self.comment.as_ref().map(|c| {
            let location = diff::comment_location(&c.path, &c.line);
            let can_add = !c.input.read(cx).content.trim().is_empty();
            popover(
                "diff-comment",
                crate::ui::popover::Anchor::Slot(c.anchor.clone()),
            )
            .side(Side::Right)
            .gap(6.)
            .width(320.)
            .padding(8.)
            .on_dismiss({
                let weak = cx.weak_entity();
                move |window, cx| {
                    let _ = weak.update(cx, |this, cx| this.dismiss_comment(window, cx));
                }
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .mb(px(6.))
                            .px(px(2.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_family(crate::ui::fonts::mono())
                                    .text_size(px(11.))
                                    .text_color(content(0.55))
                                    .child(location),
                            )
                            .child(
                                tool_button("cancel-comment", "Cancel comment", "x", true)
                                    .size(px(20.))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.dismiss_comment(window, cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .min_h(px(72.))
                            .rounded_lg()
                            .border_1()
                            .border_color(content(0.1))
                            .bg(crate::ui::theme::background_base().opacity(0.7))
                            .px(px(10.))
                            .py_2()
                            .text_size(px(13.))
                            .text_color(content(1.))
                            .child(c.input.clone()),
                    )
                    .child(
                        div()
                            .mt_2()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(div().text_size(px(10.)).text_color(content(0.35)).child(
                                if cfg!(target_os = "macos") {
                                    "⌘↩ to add"
                                } else {
                                    "Ctrl+↩ to add"
                                },
                            ))
                            .child(
                                div()
                                    .id("add-comment")
                                    .role(Role::Button)
                                    .aria_label("Add to chat")
                                    .h(px(28.))
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .px(px(10.))
                                    .rounded_md()
                                    .bg(content(1.))
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(crate::ui::theme::background_base())
                                    .when(!can_add, |d| d.opacity(0.4))
                                    .when(can_add, |d| d.hover(|s| s.opacity(0.8)))
                                    .child(
                                        icon("comment-add", 14.)
                                            .text_color(crate::ui::theme::background_base()),
                                    )
                                    .child("Add to chat")
                                    .when(can_add, |d| {
                                        d.on_click(cx.listener(|this, _, window, cx| {
                                            this.add_comment(window, cx)
                                        }))
                                    }),
                            ),
                    ),
            )
        });
        div()
            .id("diff-review")
            .key_context("DiffReview")
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_action(cx.listener(|this, _: &PreviousHunk, _, cx| this.navigate(-1, cx)))
            .on_action(cx.listener(|this, _: &NextHunk, _, cx| this.navigate(1, cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.pending = None;
                    if this.comment.is_none() {
                        window.focus(&this.focus, cx);
                    }
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if this.overview_drag.is_some() {
                    if event.pressed_button == Some(MouseButton::Left) {
                        this.overview_scroll(event.position.y, cx);
                    } else {
                        this.overview_drag = None;
                        cx.notify();
                    }
                }
                if let Some((start, from)) = this.thumb_drag {
                    if event.pressed_button != Some(MouseButton::Left) {
                        this.thumb_drag = None;
                    } else {
                        let lane = this.lane_width().max(1.);
                        let code = this.code_width().max(1.);
                        let thumb = (lane / code * lane).max(24.);
                        let span = (lane - thumb).max(1.);
                        let dx = (event.position.x - start).as_f32();
                        this.scroll_x =
                            (from + dx / span * this.max_scroll_x()).clamp(0., this.max_scroll_x());
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.overview_drag.take().is_some() {
                        cx.notify();
                    }
                    if this.thumb_drag.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.overview_drag.take().is_some() | this.thumb_drag.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .child(self.toolbar(cx))
            .child(
                div()
                    .id("unified-diff")
                    .role(Role::Region)
                    .aria_label("Unified diff")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    // The pinned header slides up under the toolbar, never over it.
                    .overflow_hidden()
                    .pr(px(crate::editor::scrollbar::WIDTH))
                    .child(
                        canvas(
                            move |bounds, _, cx| {
                                // A new width moves the scroll limits: lay out again.
                                if viewport.get().is_none_or(|old| old.size != bounds.size) {
                                    cx.notify(view_id);
                                }
                                viewport.set(Some(bounds));
                            },
                            move |bounds, _, window, _| {
                                let weak = weak.clone();
                                window.on_mouse_event(
                                    move |event: &ScrollWheelEvent, phase, _, cx| {
                                        if phase != gpui::DispatchPhase::Capture
                                            || !bounds.contains(&event.position)
                                        {
                                            return;
                                        }
                                        let took = weak
                                            .update(cx, |this, cx| this.wheel(event, cx))
                                            .unwrap_or(false);
                                        if took {
                                            cx.stop_propagation();
                                        }
                                    },
                                );
                            },
                        )
                        .absolute()
                        .size_full(),
                    )
                    .child(
                        list(self.list.clone(), move |ix, window, cx| {
                            entity.update(cx, |this, cx| this.row(ix, window, cx))
                        })
                        .size_full(),
                    )
                    .children(sticky)
                    .child(self.overview(cx)),
            )
            .child(self.horizontal_bar(cx))
            .children(comment)
            .into_any_element()
    }
}
