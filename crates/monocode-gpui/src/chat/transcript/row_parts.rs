use super::*;

impl TranscriptView {
    pub(crate) fn reveal_search(&mut self, row: usize, query: &str, cx: &mut Context<Self>) {
        if row < self.history_start {
            self.history_search = Some((row, query.into()));
            self.list.pause_following_tail();
            self.load_older_history(cx);
            return;
        }
        let start = self
            .messages
            .iter()
            .rev()
            .find(|m| m.start <= row)
            .map(|m| m.start)
            .unwrap_or(row);
        let turn = self
            .messages
            .iter()
            .rev()
            .find(|m| m.start <= row)
            .map(|m| m.turn)
            .unwrap_or(0);
        self.search_match = Some((start, query.to_owned()));
        self.expanded.insert(start);
        self.expanded_prompts.insert(start);
        self.work_folds.insert(turn, true);
        for message in &self.messages {
            if message.turn == turn {
                self.phases.insert(message.start, true);
            }
        }
        self.list.pause_following_tail();
        self.rebuild(0, cx);
        let candidates = || {
            self.rows
                .iter()
                .enumerate()
                .filter(|(_, r)| r.key.0 == start && !matches!(r.body, Body::WorkLine { .. }))
        };
        let matching = candidates()
            .find(|(_, r)| {
                let text = match &r.body {
                    Body::Markdown(md) => markdown::row_text(md),
                    Body::Plain(text) => text.to_string(),
                    Body::Interjection(note) => note.text.clone(),
                    Body::Tool { .. } => query.to_owned(),
                    _ => String::new(),
                };
                !monocode_core::store::search::literal_ranges(&text, query).is_empty()
            })
            .map(|(ix, _)| ix);
        if let Some(item_ix) = matching.or_else(|| candidates().next().map(|(ix, _)| ix)) {
            self.list.scroll_to(ListOffset {
                item_ix,
                offset_in_item: px(0.),
            });
        }
        cx.notify();
    }

    pub(super) fn code(
        &mut self,
        fence: Arc<Fence>,
        lines: std::ops::Range<usize>,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        self.highlight(&fence, cx);
        let first = lines.start == 0;
        let last = lines.end == fence.lines.len();
        let key = fence.key;
        let code = fence.code.clone();
        let copied = self.copied == Some(key);
        let file_link = fence.path.as_ref().map(|raw| {
            let (path, location) =
                crate::ui::files::file_location(raw, self.file_link_base.as_deref().unwrap_or(""));
            let (line, column) = location.unwrap_or((fence.start_line, 1));
            format!("{path}:{line}:{column}")
        });
        div()
            .relative()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .rounded_t(px(if first { 10. } else { 0. }))
            .rounded_b(px(if last { 10. } else { 0. }))
            .border_x_1()
            .when(first, |d| d.border_t_1())
            .when(last, |d| d.border_b_1())
            .border_color(content(0.1))
            .bg(content(0.06))
            .when(first, |d| {
                d.child(
                    div()
                        .when(!fence.label.is_empty(), |d| d.min_h(px(36.)))
                        .when(fence.label.is_empty(), |d| {
                            d.absolute().top(px(6.)).right(px(6.))
                        })
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.))
                        .text_color(content(0.65))
                        .when(!fence.label.is_empty(), |d| {
                            let extension = match fence.language.as_str() {
                                "rust" => "rs",
                                "javascript" => "js",
                                "typescript" => "ts",
                                "python" => "py",
                                "bash" | "shell" => "sh",
                                "markdown" => "md",
                                other => other,
                            };
                            let file = if fence.label.contains('.') {
                                fence.label.clone()
                            } else {
                                format!("code.{extension}")
                            };
                            d.child(crate::ui::file_icons::file_icon(&file, 16.))
                        })
                        .child(
                            div()
                                .id(("code-file-link", ix))
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .when_some(file_link, |d, path| {
                                    let menu_path = path.clone();
                                    d.role(Role::Link)
                                        .cursor_pointer()
                                        .hover(|s| s.text_color(rgb(0x7dd3fc)))
                                        .on_click(move |event, window, cx| {
                                            if event.click_count() == 1 {
                                                window.dispatch_action(
                                                    Box::new(crate::ui::files::OpenPath(
                                                        path.clone(),
                                                    )),
                                                    cx,
                                                );
                                            }
                                        })
                                        .on_mouse_down(
                                            gpui::MouseButton::Right,
                                            move |event, window, cx| {
                                                markdown::link_menu(
                                                    &menu_path,
                                                    event.position,
                                                    window,
                                                    cx,
                                                );
                                                cx.stop_propagation();
                                            },
                                        )
                                })
                                .child(fence.label.clone()),
                        )
                        .child(
                            icon_button(
                                "copy-code",
                                if copied { "Copied" } else { "Copy code" },
                                if copied { "check" } else { "copy" },
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        code.strip_suffix("\r\n")
                                            .or_else(|| code.strip_suffix('\n'))
                                            .unwrap_or(&code)
                                            .to_owned(),
                                    ));
                                    this.copied = Some(key);
                                    cx.notify();
                                    cx.spawn(async move |this, cx| {
                                        cx.background_executor()
                                            .timer(std::time::Duration::from_millis(1500))
                                            .await;
                                        let _ = this.update(cx, |this, cx| {
                                            if this.copied == Some(key) {
                                                this.copied = None;
                                                cx.notify();
                                            }
                                        });
                                    })
                                    .detach();
                                },
                            )),
                        ),
                )
            })
            .child(
                div()
                    .id(("code-scroll", ix))
                    .overflow_x_scroll()
                    .w_full()
                    .py(px(10.))
                    .pr_2()
                    .when(first && !fence.label.is_empty(), |d| {
                        d.border_t_1().border_color(content(0.1))
                    })
                    .child(markdown::selection::code_lines_selected(
                        &fence,
                        lines,
                        self.highlights.get(&key).and_then(|v| v.as_deref()),
                        self.search_match.as_ref().map_or("", |(_, q)| q.as_str()),
                        self.search_match
                            .as_ref()
                            .is_some_and(|(start, _)| *start == self.rows[ix].key.0),
                        self.text_selection(
                            self.rows[ix].key,
                            self.messages[self.rows[ix].message].start,
                            self.messages[self.rows[ix].message].role == "assistant",
                            cx,
                        ),
                    )),
            )
            .into_any_element()
    }

    pub(super) fn row(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        self.rendered += 1;
        if let Some(group) = self.rows[ix].phase_group()
            && self.windowed_phase(group)
        {
            // Keep source/list indices stable for message anchors, selection
            // and reconciliation. The phase header owns these rows' layout
            // while live; settled, opened history is virtualized normally.
            return div().h_0().into_any_element();
        }
        if let Body::Phase { group, active, .. } = self.rows[ix].body
            && self.windowed_phase(group)
        {
            return self.phase_window(ix, group, active, window, cx);
        }
        match self.rows[ix].body {
            Body::SavedDraft => return self.saved_draft_row(cx),
            Body::Pending(i) => return self.pending_row(i, cx),
            Body::Launch(id) => return self.launch_row(ix, id, cx),
            _ => {}
        }
        let group = self.motions.get(&self.rows[ix].key).and_then(|m| m.group);
        if let Some(group) = group
            && self
                .phase_layouts
                .get(&group)
                .is_none_or(|layout| layout.width != self.viewport.size.width)
        {
            // Measure once per content/width change while the body moves.
            // Retain individual list items so settled history stays virtualized.
            let indices: Vec<_> = self
                .rows
                .iter()
                .enumerate()
                .filter(|(_, row)| {
                    self.motions
                        .get(&row.key)
                        .is_some_and(|m| m.group == Some(group))
                })
                .map(|(ix, _)| ix)
                .collect();
            let mut layout = PhaseLayout {
                width: self.viewport.size.width,
                height: 0.,
                rows: HashMap::new(),
            };
            for ix in indices {
                let natural = self.row_body(ix, false, window, cx).layout_as_root(
                    size(
                        AvailableSpace::Definite(layout.width),
                        AvailableSpace::MinContent,
                    ),
                    window,
                    cx,
                );
                let height = f32::from(natural.height);
                layout
                    .rows
                    .insert(self.rows[ix].key, (layout.height, height));
                layout.height += height;
            }
            self.phase_layouts.insert(group, layout);
        }
        self.row_body(ix, true, window, cx)
    }

    pub(super) fn windowed_phase(&self, group: usize) -> bool {
        self.windowed_phases.contains(&group)
    }

    pub(super) fn phase_window(
        &mut self,
        ix: usize,
        group: usize,
        active: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let header = self.row_body(ix, true, window, cx);
        let open = self
            .phases
            .get(&group)
            .copied()
            .unwrap_or(self.phase_opens(active));
        let indices: Vec<_> = (ix + 1..self.rows.len())
            .take_while(|&i| self.rows[i].phase_group() == Some(group))
            .collect();
        let progress = indices
            .iter()
            .find_map(|&i| {
                self.motions
                    .get(&self.rows[i].key)
                    .filter(|m| m.group == Some(group))
                    .map(|m| m.value(self.frame_now))
            })
            .unwrap_or((1., 1.));
        let mut steps = div().w_full().min_w_0().flex().flex_col();
        for i in indices {
            steps = steps.child(self.row_body(i, true, window, cx));
        }
        let body = live_phase::live_phase(
            steps.into_any_element(),
            self.phase_scrolls.entry(group).or_default(),
            live_phase::max_height(window),
            active && open && self.visible,
            cx,
        );
        div()
            .id(("phase-window", group))
            .w_full()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .w_full()
                    .overflow_hidden()
                    .opacity(progress.1)
                    .child(Disclosure::new(body, self.viewport.size.width, progress.0)),
            )
            .into_any_element()
    }

    /// The first reply the user sent after `message`, queued ones included,
    /// which answers any choices that message offered.
    pub(super) fn reply_after(&self, message: usize) -> Option<String> {
        self.messages
            .get(message + 1..)?
            .iter()
            .find(|m| m.role == "user")
            .and_then(|m| self.source.get(m.start))
            .map(|row| row.text.clone())
            .or_else(|| self.pending.first().map(|p| p.text.to_string()))
    }

    pub(super) fn toggle(&mut self, start: usize, cx: &mut Context<Self>) {
        if !self.expanded.remove(&start) {
            self.expanded.insert(start);
        }
        self.list.pause_following_tail();
        self.rebuild(start, cx);
    }

    /// How far phase `group`'s tile has morphed into its chevron, eased:
    /// 0 is the tile, 1 the chevron.
    /// Whether a tool group nobody has opened or closed shows its steps: a
    /// running one does, unless groups are set to stay collapsed.
    pub(super) fn phase_opens(&self, active: bool) -> bool {
        active && !self.collapse_groups
    }

    pub(super) fn toggle_phase(&mut self, group: usize, open: bool, cx: &mut Context<Self>) {
        self.phase_toggle = Some(group);
        self.phases.insert(group, open);
        self.list.pause_following_tail();
        self.rebuild(group, cx);
    }
}
