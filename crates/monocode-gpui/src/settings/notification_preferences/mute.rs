use super::*;

impl SettingsView {
    pub(super) fn open_notification_mute(
        &mut self,
        ids: Vec<String>,
        anchor: AnchorSlot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let now = Local::now();
        let mut items = vec![];
        for hours in [1, 4, 8] {
            let at = now + chrono::Duration::hours(hours);
            let day = if at.date_naive() == now.date_naive() {
                ""
            } else {
                "Tomorrow, "
            };
            items.push(
                MenuAction::new(
                    format!("mute:{hours}"),
                    format!(
                        "{hours} {} ({day}{})",
                        if hours == 1 { "hour" } else { "hours" },
                        at.format("%-H:%M")
                    ),
                )
                .into(),
            );
        }
        items.push(MenuAction::new("mute:indefinite", "Until resumed").into());
        items.push(MenuAction::new("mute:custom", "Choose date and time").into());
        let menu = cx.new(|cx| Menu::new("Mute notifications", items, window, cx).width(244.));
        let targets = ids.clone();
        let events = cx.subscribe(&menu, move |this, _, event: &MenuEvent, cx| {
            if let MenuEvent::Picked(id, _) = event {
                let now = monocode_core::sys::time::now_ms();
                if id.as_ref() == "mute:custom" {
                    let initial = if targets.len() == 1 {
                        this.prefs.project_notifications[&targets[0]]["mutedUntil"]
                            .as_u64()
                            .filter(|at| *at > now)
                            .unwrap_or(now + 3_600_000)
                    } else {
                        now + 3_600_000
                    };
                    let rounded = initial.div_ceil(60_000) * 60_000;
                    let text = chrono::DateTime::from_timestamp_millis(rounded as i64)
                        .map(|at| at.with_timezone(&Local).format("%H:%M").to_string())
                        .unwrap_or_default();
                    let field = cx.new(|cx| {
                        let mut field = input::Input::field("Time", "HH:mm", content(0.4), cx);
                        field.set_text(text, cx);
                        field
                    });
                    let date = chrono::DateTime::from_timestamp_millis(rounded as i64)
                        .map(|at| at.with_timezone(&Local).date_naive())
                        .unwrap_or_else(|| Local::now().date_naive());
                    this.notification_state.custom = Some(CustomMute {
                        ids: targets.clone(),
                        input: field,
                        date,
                        focused_date: date,
                        focus_pending: true,
                        focus: cx.focus_handle(),
                        anchor: this
                            .notification_state
                            .menu
                            .as_ref()
                            .map(|menu| menu.anchor.clone())
                            .unwrap_or_default(),
                        month: date.with_day(1).unwrap(),
                        error: None,
                    });
                } else {
                    let until = if id.as_ref() == "mute:indefinite" {
                        Some(Value::Null)
                    } else {
                        id.strip_prefix("mute:")
                            .and_then(|h| h.parse::<u64>().ok())
                            .map(|h| json!(now + h * 3_600_000))
                    };
                    if let Some(until) = until {
                        this.update(cx, |p| p.mute_projects(&targets, Some(until), now));
                    }
                }
            }
            this.notification_state.menu = None;
            cx.notify();
        });
        self.notification_state.menu = Some(MuteMenu {
            menu,
            anchor,
            _events: events,
        });
        cx.notify();
    }

    pub(super) fn mute_controls(
        &self,
        ids: Vec<String>,
        key: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = monocode_core::sys::time::now_ms();
        let muted: Vec<_> = ids
            .iter()
            .filter(|id| self.prefs.project_muted(id, now))
            .collect();
        let status = if muted.is_empty() {
            String::new()
        } else if ids.len() > 1 {
            format!("{} of {} projects muted", muted.len(), ids.len())
        } else {
            match self.prefs.project_notifications[&ids[0]]["mutedUntil"].as_u64() {
                Some(at) => chrono::DateTime::from_timestamp_millis(at as i64)
                    .map(|at| {
                        format!(
                            "Muted until {}",
                            at.with_timezone(&Local).format("%b %-d, %Y, %-H:%M")
                        )
                    })
                    .unwrap_or_default(),
                None => "Muted until resumed".into(),
            }
        };
        let anchor = AnchorSlot::default();
        let trigger = anchor.clone();
        let resume_ids = ids.clone();
        div()
            .flex()
            .items_center()
            .gap_2()
            .flex_wrap()
            .when(!status.is_empty(), |d| {
                d.child(
                    div()
                        .id("project-notification-accessible-1")
                        .role(Role::Status)
                        .text_size(px(11.))
                        .text_color(content(0.5))
                        .child(status),
                )
            })
            .when(!muted.is_empty(), |d| {
                d.child(
                    secondary_button(
                        format!("resume-{key}"),
                        "Resume notifications",
                        false,
                        false,
                    )
                    .child("Resume notifications")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.update(cx, |p| {
                            p.mute_projects(&resume_ids, None, monocode_core::sys::time::now_ms())
                        })
                    })),
                )
            })
            .child(
                secondary_button(
                    format!("mute-{key}"),
                    if muted.is_empty() {
                        "Mute notifications"
                    } else {
                        "Change mute duration"
                    },
                    false,
                    ids.is_empty(),
                )
                .relative()
                .child(anchor.probe())
                .child(icon("bell-off", 14.))
                .child(if muted.is_empty() { "Mute" } else { "Muted" })
                .child(icon("chevron-down", 12.))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_notification_mute(ids.clone(), trigger.clone(), window, cx)
                })),
            )
            .into_any_element()
    }
}

impl SettingsView {
    pub(super) fn mute_calendar(&self, custom: &CustomMute, cx: &mut Context<Self>) -> AnyElement {
        let today = Local::now().date_naive();
        let month = custom.month;
        let previous_disabled = month <= today.with_day(1).unwrap();
        let shift = |date: NaiveDate, amount: i32| {
            let total = date.year() * 12 + date.month0() as i32 + amount;
            NaiveDate::from_ymd_opt(total.div_euclid(12), total.rem_euclid(12) as u32 + 1, 1)
                .unwrap_or(date)
        };
        let previous = shift(month, -1);
        let next = shift(month, 1);
        let count = (next - month).num_days() as usize;
        let offset = month.weekday().num_days_from_monday() as usize;
        let cells = (offset + count).div_ceil(7) * 7;
        let mut weeks = Vec::new();
        for week in 0..cells / 7 {
            let mut days = Vec::new();
            for column in 0..7 {
                let cell = week * 7 + column;
                let date = cell
                    .checked_sub(offset)
                    .filter(|index| *index < count)
                    .map(|index| month + chrono::Duration::days(index as i64));
                let Some(date) = date else {
                    days.push(div().w(px(36.)).h(px(32.)).into_any_element());
                    continue;
                };
                let disabled = date < today;
                let selected = date == custom.date;
                let focused = date == custom.focused_date;
                days.push(
                    div()
                        .w(px(36.))
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .id(("mute-calendar-day", cell))
                                .role(Role::Button)
                                .aria_label(date.to_string())
                                .aria_selected(selected)
                                .size(px(32.))
                                .rounded_md()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(12.))
                                .text_color(content(if disabled { 0.2 } else { 0.7 }))
                                .when(focused, |d| d.border_1().border_color(accent_color()))
                                .when(selected, |d| {
                                    d.bg(content(0.1)).border_1().border_color(content(0.2))
                                })
                                .when(!disabled, |d| {
                                    d.cursor_pointer().hover(|s| s.bg(content(0.05)))
                                })
                                .child(date.day().to_string())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if !disabled {
                                        if let Some(custom) = &mut this.notification_state.custom {
                                            custom.date = date;
                                            custom.focused_date = date;
                                            custom.error = None;
                                        }
                                        cx.notify();
                                    }
                                })),
                        )
                        .into_any_element(),
                );
            }
            weeks.push(div().flex().py(px(2.)).children(days));
        }
        div()
            .child(
                div()
                    .h(px(32.))
                    .mb_2()
                    .px_1()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(12.))
                            .child(month.format("%B %Y").to_string()),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(2.))
                            .child(
                                secondary_button(
                                    "mute-previous-month",
                                    "Previous month",
                                    false,
                                    previous_disabled,
                                )
                                .child(icon("chevron-left", 14.))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        if !previous_disabled {
                                            if let Some(custom) =
                                                &mut this.notification_state.custom
                                            {
                                                custom.month = previous;
                                            }
                                            cx.notify();
                                        }
                                    },
                                )),
                            )
                            .child(
                                secondary_button("mute-next-month", "Next month", false, false)
                                    .child(icon("chevron-right", 14.))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(custom) = &mut this.notification_state.custom {
                                            custom.month = next;
                                        }
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .id("notification-mute-calendar")
                    .role(Role::Grid)
                    .aria_label("Choose date")
                    .track_focus(&custom.focus)
                    .tab_index(0)
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            if let Some(custom) = &this.notification_state.custom {
                                window.focus(&custom.focus, cx);
                            }
                        }),
                    )
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                        let Some(custom) = &mut this.notification_state.custom else {
                            return;
                        };
                        let day = custom.focused_date;
                        let offset = day.weekday().num_days_from_monday() as i64;
                        let next = match event.keystroke.key.as_str() {
                            "enter" | "space" => {
                                custom.date = day;
                                custom.error = None;
                                Some(day)
                            }
                            "left" => Some(day - chrono::Duration::days(1)),
                            "right" => Some(day + chrono::Duration::days(1)),
                            "up" => Some(day - chrono::Duration::days(7)),
                            "down" => Some(day + chrono::Duration::days(7)),
                            "home" => Some(day - chrono::Duration::days(offset)),
                            "end" => Some(day + chrono::Duration::days(6 - offset)),
                            "pageup" => day.checked_sub_months(chrono::Months::new(1)),
                            "pagedown" => day.checked_add_months(chrono::Months::new(1)),
                            _ => None,
                        };
                        if let Some(next) = next {
                            let next = next.max(Local::now().date_naive());
                            custom.focused_date = next;
                            custom.month = next.with_day(1).unwrap();
                            cx.stop_propagation();
                            cx.notify();
                        }
                    }))
                    .flex()
                    .flex_col()
                    .child(
                        div().flex().children(
                            ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"]
                                .into_iter()
                                .map(|day| {
                                    div()
                                        .w(px(36.))
                                        .text_center()
                                        .pb(px(6.))
                                        .text_size(px(10.))
                                        .text_color(content(0.4))
                                        .child(day)
                                }),
                        ),
                    )
                    .children(weeks),
            )
            .into_any_element()
    }
}
