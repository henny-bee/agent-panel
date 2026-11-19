use super::*;

impl Shell {
    pub(crate) fn clear_notification_reminder(
        &mut self,
        id: i64,
        expected: u64,
        cx: &mut Context<Self>,
    ) {
        let removed = self
            .reminders
            .iter()
            .find(|r| r.session_id == id && r.due_at == expected)
            .cloned();
        self.reminders
            .retain(|r| !(r.session_id == id && r.due_at == expected));
        platform::dismiss(
            &Target::Reminder {
                session_id: id,
                due_at: expected,
            }
            .tag(0),
        );
        let store = self.store.clone();
        let previous = self.group_writes.take();
        self.group_writes = Some(cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let work = cx.background_executor().spawn(async move {
                locked(&store).and_then(|s| s.clear_reminders(&[id], Some(expected)))
            });
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Err(error) => {
                        if let Some(removed) = removed
                            && !this.reminders.iter().any(|r| r.session_id == id)
                        {
                            this.reminders.push(removed);
                        }
                        this.notification_state.reminder_error = Some(error);
                    }
                    Ok(()) => this.notification_state.reminder_error = None,
                }
                this.invalidate_chrome(cx);
                cx.notify();
            });
        }));
        self.invalidate_chrome(cx);
        cx.notify();
    }

    pub(super) fn snooze_notification_reminder(
        &mut self,
        id: i64,
        expected: u64,
        due_at: u64,
        cx: &mut Context<Self>,
    ) {
        if !self
            .reminders
            .iter()
            .any(|r| r.session_id == id && r.due_at == expected)
        {
            return;
        }
        let store = self.store.clone();
        let previous = self.group_writes.take();
        self.group_writes = Some(cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let work = cx.background_executor().spawn(async move {
                locked(&store).and_then(|s| s.snooze_reminder_if(id, expected, due_at))
            });
            let result = work.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(_) => {
                    platform::dismiss(
                        &Target::Reminder {
                            session_id: id,
                            due_at: expected,
                        }
                        .tag(0),
                    );
                    this.notification_state.reminder_error = None;
                    this.reload_notification_reminders(cx);
                }
                Err(error) => {
                    this.notification_state.reminder_error = Some(error);
                    cx.notify();
                }
            });
        }));
    }

    pub(super) fn reload_notification_reminders(&mut self, cx: &mut Context<Self>) {
        let expected_snapshot = self.reminders.clone();
        let store = self.store.clone();
        let previous = self.group_writes.take();
        self.group_writes = Some(cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let work = cx
                .background_executor()
                .spawn(async move { locked(&store).and_then(|s| s.reminders()) });
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(reminders) => {
                        // Preserve a newer optimistic schedule/cancel while its
                        // ordered write waits behind this reload operation.
                        if this.reminders == expected_snapshot {
                            this.reminders = reminders;
                        }
                        this.notification_state.reminder_error = None;
                        this.poll_reminders(cx);
                    }
                    Err(error) => this.notification_state.reminder_error = Some(error),
                }
                this.invalidate_chrome(cx);
                cx.notify();
            });
        }));
    }

    pub(crate) fn open_whats_new(&mut self, version: String, cx: &mut Context<Self>) {
        let view = cx.new(|_| WhatsNew::new(version.clone()));
        let weak = view.downgrade();
        let store = self.store.clone();
        let work = cx.background_executor().spawn(async move {
            locked(&store).and_then(|store| store.setting("monocode.nativeReleaseNotes"))
        });
        cx.spawn(async move |_, cx| {
            if let Ok(Some(raw)) = work.await
                && let Ok(notes) = serde_json::from_str::<serde_json::Value>(&raw)
                && notes["version"].as_str() == Some(version.as_str())
                && let Some(body) = notes["body"].as_str()
            {
                let _ = weak.update(cx, |view, cx| view.set_release_notes(body, cx));
            }
        })
        .detach();
        let events = cx.subscribe(&view, |this, _, _: &WhatsNewEvent, cx| {
            this.notification_state.whats_new = None;
            cx.notify();
        });
        self.notification_state.whats_new = Some((view, events, cx.focus_handle()));
        cx.notify();
    }

    pub(crate) fn notices_overlay(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.notification_state;
        let mono_error = self.monos.as_ref().and_then(|monos| {
            monos
                .read(cx)
                .error
                .clone()
                .map(|message| (monos.clone(), message))
        });
        let stack = div()
            .id("notification-stack")
            .absolute()
            .top(px(12.))
            .right(px(12.))
            .w(px(360.))
            .max_w(px((self.viewport_width - 24.).max(0.)))
            .flex()
            .flex_col()
            .items_end()
            .gap(px(8.))
            .child(state.region.probe())
            .when_some(self.error.clone(), |stack, message| {
                stack.child(errors::card(
                    "app-error",
                    message,
                    icon_button("dismiss-error", "Dismiss error", "close")
                        .debug_selector(|| "dismiss-error".into())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.error = None;
                            cx.notify();
                        })),
                ))
            })
            .when_some(mono_error, |stack, (monos, message)| {
                stack.child(errors::card(
                    "mono-error",
                    message,
                    icon_button("mono-error-close", "Dismiss Mono error", "close")
                        .debug_selector(|| "mono-error-close".into())
                        .on_click(cx.listener(move |_, _, _, cx| {
                            monos.update(cx, |monos, cx| monos.dismiss_error(cx));
                        })),
                ))
            })
            .when(state.file_error.read(cx).is_visible(), |stack| {
                stack.child(state.file_error.clone())
            })
            .when(!state.reminders.read(cx).is_empty(), |d| {
                d.child(
                    div()
                        .w(px(320.))
                        .max_w_full()
                        .child(state.reminders.clone()),
                )
            })
            .when(state.harness.read(cx).is_visible(), |d| {
                d.child(div().w(px(340.)).max_w_full().child(state.harness.clone()))
            })
            .when(state.layout.read(cx).is_visible(), |d| {
                d.child(div().w(px(340.)).max_w_full().child(state.layout.clone()))
            })
            .when(!state.approvals.read(cx).is_empty(), |d| {
                d.child(
                    div()
                        .w(px(360.))
                        .max_w_full()
                        .child(state.approvals.clone()),
                )
            });
        // Toasts remain actionable above dialogs, including Mono settings.
        gpui::deferred(stack).with_priority(110).into_any_element()
    }

    pub(crate) fn notice_modals(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut elements = Vec::new();
        if let Some((view, _, focus)) = &self.notification_state.whats_new {
            let view = view.clone();
            let focus = focus.clone();
            if !focus.contains_focused(window, cx) {
                window.focus(&focus, cx);
            }
            elements.push(
                div()
                    .id("whats-new-overlay")
                    .absolute()
                    .inset_0()
                    .occlude()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(crate::ui::modal::scrim("whats-new-scrim"))
                    .track_focus(&focus)
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                        if event.keystroke.key == "escape" {
                            this.notification_state.whats_new = None;
                            cx.notify();
                            cx.stop_propagation();
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.notification_state.whats_new = None;
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(view),
                    )
                    .into_any_element(),
            );
        }
        elements
    }
}
