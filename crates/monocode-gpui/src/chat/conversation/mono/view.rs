use super::*;

impl Conversation {
    pub(in crate::chat::conversation) fn render_mono(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let failed_ids: Vec<_> = self
            .mono_ui
            .pending
            .iter()
            .filter(|m| m.internal && m.error.is_some())
            .map(|m| m.id.clone())
            .collect();
        let list_focus = self
            .mono_ui
            .queue_list_focus
            .get_or_insert_with(|| cx.focus_handle().tab_stop(true))
            .clone();
        if self.mono_ui.queue_ids != failed_ids {
            let queue_focused = list_focus.is_focused(window)
                || self
                    .mono_ui
                    .queue_focus
                    .values()
                    .any(|(handles, _)| handles.iter().any(|handle| handle.is_focused(window)));
            if queue_focused {
                let focus = if failed_ids.is_empty() {
                    self.input.focus_handle(cx)
                } else {
                    list_focus.clone()
                };
                window.focus(&focus, cx);
            }
            self.mono_ui.queue_scroll = gpui::ScrollHandle::new();
            self.mono_ui.queue_ids.clone_from(&failed_ids);
        }
        if self.mono_ui.queue_window != Some(window.window_handle()) {
            self.mono_ui.queue_focus.clear();
            self.mono_ui.queue_window = Some(window.window_handle());
        }
        self.mono_ui
            .queue_focus
            .retain(|id, _| failed_ids.contains(id));
        for id in &failed_ids {
            if !self.mono_ui.queue_focus.contains_key(id) {
                let handles = [
                    cx.focus_handle().tab_stop(true),
                    cx.focus_handle().tab_stop(true),
                ];
                let subscribe =
                    |handle: &gpui::FocusHandle, cx: &mut Context<Self>, window: &mut Window| {
                        let id = id.clone();
                        cx.on_focus(handle, window, move |this, _, cx| {
                            if let Some(index) = this
                                .mono_ui
                                .pending
                                .iter()
                                .filter(|m| m.internal && m.error.is_some())
                                .position(|m| m.id == id)
                            {
                                this.mono_ui.queue_scroll.scroll_to_top_of_item(index);
                                cx.notify();
                            }
                        })
                    };
                let subscriptions = [
                    subscribe(&handles[0], cx, window),
                    subscribe(&handles[1], cx, window),
                ];
                self.mono_ui
                    .queue_focus
                    .insert(id.clone(), (handles, subscriptions));
            }
        }
        let mono = self.settings.mono.as_ref().unwrap().mono.clone();
        let status = self.mono_status();
        let name = mono.label();
        // MonoHeader's greeting: it shows only before the first message.
        let body = if !self.summary.has_messages {
            let greeting = div().flex().flex_col().items_center().justify_center().px_6().py_12()
                .child(crate::ui::avatar::avatar(&mono.mascot, mono.color, status, 56., false))
                .child(div().mt(px(-8.)).h(px(32.)).px(px(14.)).flex().items_center().rounded_full().border_1().border_color(content(0.14)).bg(content(0.06)).text_size(px(13.)).font_weight(gpui::FontWeight::SEMIBOLD).child(name))
                .child(div().mt_3().max_w(px(384.)).text_center().text_size(px(13.)).line_height(px(21.)).text_color(content(0.5)).child(if mono.projects.is_empty(){"Add the projects I should work on in my settings, or just ask me something.".into()}else{format!("I can see every session, note and folder in {}. Ask what’s going on, or hand me work.",mono.projects.iter().map(|p|std::path::Path::new(p).file_name().unwrap_or_default().to_string_lossy()).collect::<Vec<_>>().join(", "))}));
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(greeting)
                .into_any_element()
        } else {
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .child(
                    self.transcript
                        .clone()
                        .cached(StyleRefinement::default().size_full()),
                )
                .into_any_element()
        };
        self.attachment_drop_target("mono-chat", cx)
            .capture_action(cx.listener(|this, _: &input::Paste, _, cx| {
                if this.paste_attachments(cx) {
                    cx.stop_propagation();
                }
            }))
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .when_some(self.menu.as_ref(), |d, menu| {
                d.child(
                    menu_popover(&menu.menu, Anchor::Slot(self.anchor(menu.kind).clone()), cx)
                        .side(Side::Top)
                        .when(menu.kind == ComposerMenu::Account, |p| {
                            p.width(250.).max_height(260.)
                        }),
                )
            })
            .children(self.model_list_popover(cx))
            .child(body)
            .child(
                div()
                    .w_full()
                    .max_w(px(COMPOSER_COLUMN))
                    .mx_auto()
                    .px(px(10.))
                    .pb_1p5()
                    .flex()
                    .flex_col()
                    .when(!failed_ids.is_empty(), |d| d.child(
                        div()
                            .id("mono-internal-queue-list")
                            .debug_selector(|| "mono-internal-queue-list".into())
                            .focus_visible(|s| s.border_2().border_color(content(0.8)))
                            .track_focus(&list_focus)
                            .tab_index(0)
                            .track_scroll(&self.mono_ui.queue_scroll)
                            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                                let scroll = &this.mono_ui.queue_scroll;
                                let delta = match event.keystroke.key.as_str() {
                                    "up" => px(40.),
                                    "down" => px(-40.),
                                    "pageup" => px(160.),
                                    "pagedown" => px(-160.),
                                    _ => return,
                                };
                                let y = (scroll.offset().y + delta).max(-scroll.max_offset().y).min(px(0.));
                                scroll.set_offset(gpui::point(scroll.offset().x, y));
                                cx.stop_propagation();
                                cx.notify();
                            }))
                            .max_h(px(180.))
                            .overflow_y_scroll()
                            .children(
                                self.mono_ui
                                    .pending
                                    .iter()
                                    .filter(|m| m.internal && m.error.is_some())
                                    .map(|message| {
                                        let retry = message.id.clone();
                                        let remove = retry.clone();
                                        let focus = &self.mono_ui.queue_focus[&message.id].0;
                                        div()
                                            .id(SharedString::from(format!(
                                                "mono-queued-{}",
                                                message.id
                                            )))
                                            .debug_selector(|| "mono-internal-queue".into())
                                            .mb_1()
                                            .flex()
                                            .justify_end()
                                            .child(
                                                div()
                                                    .max_w(px(440.))
                                                    .px_3()
                                                    .py_2()
                                                    .rounded_xl()
                                                    .border_1()
                                                    .border_color(content(0.15))
                                                    .bg(content(0.04))
                                                    .text_size(px(13.))
                                                    .text_color(content(0.6))
                                                    .child(if message.internal {
                                                        "Completion update".into()
                                                    } else {
                                                        message.text.clone()
                                                    })
                                                    .when(message.error.is_some(), |d| {
                                                        d.child(
                                                            div()
                                                                .flex()
                                                                .gap_2()
                                                                .child(
                                                                    icon_button(
                                                                        "mono-queue-retry",
                                                                        "Retry queued message",
                                                                        "refresh-cw",
                                                                    )
                                                                    .track_focus(&focus[0]).tab_index(0)
                                                                    .focus_visible(|s| s.border_2().border_color(content(0.8)))
                                                                    .debug_selector({ let id = message.id.clone(); move || format!("mono-queue-retry-{id}") })
                                                                    .on_click(cx.listener(
                                                                        move |_, _, _, cx| {
                                                                            cx.emit(QueueChanged {
                                                                                id: retry.clone(),
                                                                                remove: false,
                                                                            })
                                                                        },
                                                                    )),
                                                                )
                                                                .child(
                                                                    icon_button(
                                                                        "mono-queue-discard",
                                                                        "Discard queued message",
                                                                        "close",
                                                                    )
                                                                    .track_focus(&focus[1]).tab_index(0)
                                                                    .focus_visible(|s| s.border_2().border_color(content(0.8)))
                                                                    .debug_selector({ let id = message.id.clone(); move || format!("mono-queue-discard-{id}") })
                                                                    .on_click(cx.listener(
                                                                        move |_, _, _, cx| {
                                                                            cx.emit(QueueChanged {
                                                                                id: remove.clone(),
                                                                                remove: true,
                                                                            })
                                                                        },
                                                                    )),
                                                                ),
                                                        )
                                                    })
                                                    .child(
                                                        div()
                                                            .text_size(px(11.))
                                                            .text_color(content(0.4))
                                                            .child(
                                                                message
                                                                    .error
                                                                    .clone()
                                                                    .unwrap_or_else(|| {
                                                                        "Pending".into()
                                                                    }),
                                                            ),
                                                    ),
                                            )
                                    }),
                            ),
                    ))
                    .when(!failed_ids.is_empty(), |d| {
                        d.child(div().text_size(px(11.)).text_color(content(0.5)).child("Scroll for completion updates"))
                    })
                    .children(self.usage_notice(true, cx))
                    .children(self.auth_notice(cx))
                    .child(self.approval_shelf(window, cx))
                    .child({
                        let composer = div()
                            .child(self.mono_composer_box(window, cx))
                            .child(self.mono_composer_tools(cx))
                            .into_any_element();
                        let focus = self.input.focus_handle(cx);
                        let card = self.questions.front().map(|(card, _)| card);
                        self.question_overlay
                            .wrap(composer, card, &focus, window, cx)
                    }),
            )
            .into_any_element()
    }

    /// The session composer's box: the field, then a toolbar with the model
    /// picker, its settings, access and send.
    pub(super) fn mono_composer_box(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let radius = COMPOSER_RADIUS;
        let has_value = self.input.read(cx).has_message_text() || !self.attachments.is_empty();
        // As the session composer: setting pills only with controls beside
        // the model, otherwise the effort rides inside the model pill.
        let beside = cx
            .try_global::<crate::ui::preferences::Preferences>()
            .is_some_and(|p| p.model_controls == "beside");
        let trigger_effort = (!beside).then(|| self.trigger_effort_label()).flatten();
        let trigger_fast = self.trigger_fast(beside);
        let controls = if beside {
            self.model_controls(cx)
        } else {
            Vec::new()
        };
        let models_open = self.menu_is(ComposerMenu::Model)
            || self.menu_is(ComposerMenu::Recent)
            || self.mono_ui.models.is_some();
        let kind = Kind::from_id(&self.summary.provider);
        div()
            .id("mono-composer")
            .debug_selector(|| "mono-composer".into())
            .relative()
            .rounded(px(radius))
            .border_1()
            .border_color(composer_border(
                self.input.focus_handle(cx).is_focused(window),
            ))
            .shadow(composer_shadow())
            .when(self.file_drag, |d| {
                d.border_color(accent_color().opacity(0.6))
            })
            .flex()
            .flex_col()
            .on_mouse_down(
                MouseButton::Left,
                // Keep a focus-tracking ancestor from taking focus back.
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.input.focus_handle(cx), cx);
                    window.prevent_default();
                }),
            )
            .child(self.composer_anchor.probe())
            .child(composer_surface(radius))
            .children(self.attachment_chips(cx))
            .child(
                div()
                    .px_3()
                    .pt(px(10.))
                    .pb(px(14.))
                    .child(self.input.clone()),
            )
            .child(
                div()
                    .debug_selector(|| "mono-composer-toolbar".into())
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .pb(px(10.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                tool_pill("mono-model", "Model")
                                    .debug_selector(|| "mono-model".into())
                                    .min_w_0()
                                    .max_w(px(220.))
                                    .flex_shrink(1.)
                                    .aria_label(format!(
                                        "{}, {}{}",
                                        kind.label(),
                                        self.model_label(),
                                        trigger_effort
                                            .as_ref()
                                            .map(|effort| format!(", effort {effort}"))
                                            .unwrap_or_default()
                                    ))
                                    .aria_expanded(models_open)
                                    .child(self.model_anchor.probe())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.mono_ui.models = None;
                                        this.toggle_menu(ComposerMenu::Model, window, cx)
                                    }))
                                    .on_mouse_down(
                                        MouseButton::Right,
                                        cx.listener(|this, _, window, cx| {
                                            cx.stop_propagation();
                                            this.open_recent_models(window, cx)
                                        }),
                                    )
                                    .child(provider_icon(&self.summary.provider, 16.))
                                    .child(pill_label(self.model_label()))
                                    .when_some(trigger_effort, |d, effort| {
                                        d.child(
                                            div()
                                                .flex_shrink_0()
                                                .text_size(px(11.))
                                                .text_color(content(0.5))
                                                .child(effort),
                                        )
                                    })
                                    .when(trigger_fast, |d| d.child(fast_bolt()))
                                    .child(icon("chevron-down", 12.)),
                            )
                            .children(controls)
                            .child(
                                tool_pill("mono-access", "Access")
                                    .aria_label(format!("Access, {}", self.settings.access.label()))
                                    .aria_expanded(self.menu_is(ComposerMenu::Access))
                                    .child(self.access_anchor.probe())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_menu(ComposerMenu::Access, window, cx)
                                    }))
                                    .child(
                                        icon(
                                            match self.settings.access {
                                                AccessMode::FullAccess => "shield-alert",
                                                AccessMode::AutoAcceptEdits => "pencil-edit",
                                                AccessMode::Auto => "sparkles",
                                                _ => "square-lock",
                                            },
                                            14.,
                                        )
                                        .text_color(
                                            if self.settings.access == AccessMode::FullAccess {
                                                tint(AMBER_400, 0.9)
                                            } else {
                                                content(1.)
                                            },
                                        ),
                                    )
                                    .child(pill_label(self.settings.access.label()))
                                    .child(icon("chevron-down", 12.)),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "mono-send".into())
                            .flex_shrink_0()
                            .child(self.action(has_value, window, cx)),
                    ),
            )
            .children(self.attachment_drop_overlay(12.))
            .into_any_element()
    }

    /// Under the box: attach and emoji on the left, the context meter on the
    /// right.
    pub(super) fn mono_composer_tools(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let tool = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(label)
                .group(id)
                .flex_shrink_0()
                .size(px(26.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .hover(|s| s.bg(content(0.08)))
        };
        let ratio = self.context.and_then(|usage| usage.ratio());
        let headline = ratio
            .map(|r| format!("{:.0}% context used", r * 100.))
            .unwrap_or_else(|| "No context used yet".into());
        let detail = self.context.map(|usage| {
            format!(
                "{} / {} tokens",
                monocode_core::session::context::format_tokens(usage.used),
                monocode_core::session::context::format_tokens(usage.window.unwrap_or(0))
            )
        });
        div()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(2.))
            .pt_1()
            .pl_1()
            // The box's 1px border plus its toolbar's 8px: the meter sits
            // straight under send.
            .pr(px(9.))
            .child(
                tool("mono-attach", "Attach files")
                    .debug_selector(|| "mono-attach".into())
                    .child(
                        icon("mono-attach", 17.)
                            .text_color(content(0.55))
                            .group_hover("mono-attach", |s| s.text_color(content(1.))),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.choose_attachments(cx))),
            )
            .child(
                tool("mono-emoji", "Add emoji")
                    .when(self.mono_ui.emoji, |d| d.bg(content(0.08)))
                    .child(
                        icon("smile", 13.)
                            .text_color(content(if self.mono_ui.emoji { 1. } else { 0.55 }))
                            .group_hover("mono-emoji", |s| s.text_color(content(1.))),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        if cfg!(target_os = "macos") {
                            window.focus(&this.input.focus_handle(cx), cx);
                            window.show_character_palette();
                            return;
                        }
                        this.mono_ui.emoji = !this.mono_ui.emoji;
                        cx.notify();
                    })),
            )
            .when(self.mono_ui.emoji, |d| {
                d.child(
                    div().flex().gap_1().children(
                        ["👍", "❤️", "🙂", "🎉", "👀", "✨", "🙏", "🚀"]
                            .into_iter()
                            .enumerate()
                            .map(|(ix, emoji)| {
                                div()
                                    .id(("mono-emoji", ix))
                                    .text_size(px(18.))
                                    .rounded_md()
                                    .hover(|s| s.bg(content(0.1)))
                                    .child(emoji)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.input.update(cx, |i, cx| {
                                            i.replace_text_in_range(None, emoji, window, cx)
                                        });
                                        this.mono_ui.emoji = false;
                                        cx.notify();
                                    }))
                            }),
                    ),
                )
            })
            .child(
                div()
                    .ml_auto()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        // A Mono is one long conversation; this is how full its
                        // current thread is. It rotates threads itself.
                        div()
                            .id("mono-context-meter")
                            .debug_selector(|| "mono-context-meter".into())
                            .role(Role::Image)
                            .aria_label(format!(
                                "{headline}{}",
                                detail
                                    .as_ref()
                                    .map(|s| format!(", {s}"))
                                    .unwrap_or_default()
                            ))
                            .flex_shrink_0()
                            .size(px(26.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|s| s.bg(content(0.08)))
                            .child(self.context_anchor.probe())
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.context_hovered = *hovered;
                                cx.notify();
                            }))
                            .child(crate::chat::composer::context_meter::ring(
                                ratio.unwrap_or(0.),
                            ))
                            .when(self.context_hovered, |d| {
                                d.child(
                                    popover(
                                        "mono-context-tooltip",
                                        Anchor::Slot(self.context_anchor.clone()),
                                    )
                                    .side(Side::Top)
                                    .align(Align::End)
                                    .max_width(240.)
                                    .padding(10.)
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .text_size(px(12.))
                                            .line_height(px(16.))
                                            .text_color(content(1.))
                                            .child(headline)
                                            .when_some(detail, |d, detail| {
                                                d.child(
                                                    div()
                                                        .text_size(px(11.))
                                                        .text_color(content(0.5))
                                                        .child(detail),
                                                )
                                            }),
                                    ),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }
}
