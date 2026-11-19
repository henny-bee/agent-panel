use super::*;

impl Render for TranscriptView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.perf_scroll(window, cx);
        let top = self.list.logical_scroll_top();
        // Rows that don't fill the viewport can't scroll, so their top being
        // on screen asks for older turns too. Layout must have run first.
        let short = top.item_ix == 0
            && top.offset_in_item <= px(0.)
            && !self.history_failed
            && (self.history_start > 0 || self.history_before.is_some())
            && self.list.bounds_for_item(0).is_some();
        if (std::mem::take(&mut self.history_requested)
            && !self.list.is_following_tail()
            && top.item_ix <= 2)
            || short
        {
            self.load_older_history(cx);
        }
        if self.selection_focus.is_none() {
            self.selection_focus = Some(cx.focus_handle());
        }
        self.selection.begin_frame();
        self.phase_max_height = live_phase::max_height(window);
        self.frame_now = Instant::now();
        for reveal in self.reveals.values_mut() {
            reveal.fades.advance(self.frame_now);
        }
        self.markdown_cache
            .begin_frame(&self.file_link_base, &self.search_match);
        self.begin_measurements(window, cx);
        if !self.scroll_bound {
            self.scroll_bound = true;
            self.list
                // The list holds its state borrow while delivering the event.
                // Read the final scroll geometry in our trailing layout probe.
                .set_scroll_handler(cx.listener(|this, _, _, cx| {
                    this.selection.clear();
                    this.history_requested =
                        this.history_start > 0 || this.history_before.is_some();
                    cx.notify()
                }));
        }
        self.update_follow(window, cx);
        let now = Instant::now();
        if self.prompt_rise_pending {
            self.prompt_rise_pending = false;
            if let Some((_, born)) = self.prompt_rise.as_mut() {
                *born = now;
            }
        }
        self.rendered = 0;
        self.render_passes += 1;
        let view = cx.entity().downgrade();
        let after = cx.entity().downgrade();
        div()
            .id("transcript")
            .key_context("TranscriptSelection")
            .track_focus(self.selection_focus.as_ref().unwrap())
            .on_action(cx.listener(Self::copy_selection))
            .on_action(cx.listener(Self::open_link_menu))
            .capture_any_mouse_down(cx.listener(
                |this, event: &gpui::MouseDownEvent, window, cx| {
                    if event.button == gpui::MouseButton::Right && this.links.open.is_none() {
                        window.focus(this.selection_focus.as_ref().unwrap(), cx);
                    }
                },
            ))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.selection.clear_if_not_dragging();
                    cx.notify();
                }),
            )
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && this.selection.active() {
                    this.selection.clear();
                    cx.notify();
                    cx.stop_propagation()
                }
            }))
            .on_mouse_move(cx.listener(Self::move_selection))
            .on_mouse_up(gpui::MouseButton::Left, cx.listener(Self::finish_selection))
            .on_mouse_up_out(gpui::MouseButton::Left, cx.listener(Self::finish_selection))
            .role(Role::Log)
            .aria_label(if self.document {
                "Note body"
            } else {
                "Conversation transcript"
            })
            .relative()
            .size_full()
            .overflow_hidden()
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let _ = view.update(cx, |this, cx| {
                            if this.viewport != bounds {
                                // Layout learns the width after the outline
                                // was built. Redraw after this frame only if
                                // its visibility changes; an extra layout
                                // otherwise advances chat motion needlessly.
                                if this.outline_visible_at(this.viewport.size.width)
                                    != this.outline_visible_at(bounds.size.width)
                                {
                                    let view = cx.entity();
                                    cx.defer(move |cx| view.update(cx, |_, cx| cx.notify()));
                                }
                                this.selection.clear();
                                this.viewport = bounds;
                                this.list
                                    .remeasure_items(this.footer_ix()..this.footer_ix() + 1);
                            }
                            let resized = this.update_follow_viewport(bounds, window);
                            // Refresh the row-height baseline after a real resize even
                            // when the chat is idle, ready for the next send.
                            this.schedule_frame(resized, window, cx);
                        });
                    },
                    |bounds, _, window, _| paint_edge_fade(bounds, true, window),
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .child(
                list(
                    self.list.clone(),
                    cx.processor(|this, ix: usize, window, cx| {
                        if ix == this.footer_ix() {
                            this.footer(window, cx)
                        } else {
                            let row = this.row(ix, window, cx);
                            this.reflow.wrap(this.rows[ix].key, row, this.follow.offset)
                        }
                    }),
                )
                .pt(px(TOP_INSET))
                .size_full(),
            )
            .child(
                canvas(
                    move |_, _, cx| {
                        let _ = after.update(cx, |this, cx| {
                            this.update_jump(cx);
                            this.update_outline(cx);
                        });
                    },
                    |bounds, _, window, _| paint_edge_fade(bounds, false, window),
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .children(self.outline(window, cx))
            .children(self.selection_menu(cx))
            .child(self.link_overlays(cx))
            .children(self.work_item_card(window, cx))
            .when(self.show_jump, |d| {
                // Centered, or the column's right end when the commit pill
                // takes the left.
                let beside_pill = self.jump_beside_pill;
                d.child(
                    div()
                        .absolute()
                        .bottom(px(8.))
                        .left_0()
                        .w_full()
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .w_full()
                                .max_w(px(crate::ui::theme::COLUMN))
                                .px(px(COLUMN_PAD))
                                .flex()
                                .map(|d| {
                                    if beside_pill {
                                        d.justify_end()
                                    } else {
                                        d.justify_center()
                                    }
                                })
                                .child(
                                    div()
                                        .id("jump-to-bottom")
                                        .child(self.jump_anchor.probe())
                                        .role(Role::Button)
                                        .aria_label("Scroll to bottom")
                                        .relative()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size(px(30.))
                                        .rounded_full()
                                        .border_1()
                                        .border_color(content(0.1))
                                        .text_color(content(0.7))
                                        .hover(|s| s.text_color(content(1.)).bg(content(0.04)))
                                        .child(floating_surface())
                                        .child(icon("chevron-down", 16.))
                                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                            cx.stop_propagation()
                                        })
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.jump_to_bottom(cx)),
                                        ),
                                ),
                        ),
                )
            })
    }
}
