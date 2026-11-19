use super::*;

impl Monos {
    pub(in crate::monos) fn clear_habit_fields(&mut self) {
        self.fields.retain(|key, _| !key.starts_with("habit-"));
        self.habit_ui.error = None;
        self.habit_ui.revision = self.habit_ui.revision.wrapping_add(1);
    }

    pub(super) fn new_habit_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fresh = !self.fields.contains_key("habit-name");
        let name = self.field("habit-name", "Name", String::new(), false, cx);
        let instructions = if let Some(field) = self.fields.get("habit-instructions") {
            field.clone()
        } else {
            let field = cx.new(Input::habit_instructions);
            self.observations
                .push(cx.subscribe(&field, |_, _, _: &input::Changed, cx| cx.notify()));
            self.fields
                .insert("habit-instructions".into(), field.clone());
            field
        };
        if fresh {
            // Views are rendered lazily during prepaint. The chat composer may
            // already have claimed this frame's accessibility focus, so moving
            // focus here would let the new field claim it a second time.
            let manager = cx.weak_entity();
            let focus = name.focus_handle(cx);
            let field_id = name.entity_id();
            let revision = self.habit_ui.revision;
            window.on_next_frame(move |window, cx| {
                let _ = manager.update(cx, |this, cx| {
                    if this.route == Some(Route::NewHabit)
                        && this.habit_ui.revision == revision
                        && this
                            .fields
                            .get("habit-name")
                            .is_some_and(|field| field.entity_id() == field_id)
                    {
                        window.focus(&focus, cx);
                    }
                });
            });
            name.update(cx, |f, _| f.set_max_chars(80));
            instructions.update(cx, |f, _| {
                f.set_max_chars(4000);
                f.set_font_size(12.);
                f.set_line_height(20.);
            });
            for field in [&name, &instructions] {
                self.observations
                    .push(cx.subscribe(field, |this, _, _: &input::Submitted, cx| {
                        this.save_habit(cx)
                    }));
            }
        }
        for (key, text) in [
            ("habit-kind", "daily"),
            ("habit-time", "09:00"),
            ("habit-minute", "0"),
            ("habit-weekday", "1"),
        ] {
            self.field(key, key, text.into(), false, cx);
        }
        self.fields["habit-time"].update(cx, |f, _| f.set_max_chars(5));
        let ready = !self.habit_ui.saving
            && !self.value("habit-name", cx).trim().is_empty()
            && !self.value("habit-instructions", cx).trim().is_empty();
        let next = self
            .habit_schedule(cx)
            .next(now_ms())
            .map(|at| when(at, now_ms()))
            .unwrap_or_else(|_| "Choose a valid time (HH:MM)".into());
        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(
                self.page_header("New habit", cx).child(
                    button(
                        "mono-habit-save",
                        if self.habit_ui.saving {
                            "Creating…"
                        } else {
                            "Create"
                        },
                    )
                    .debug_selector(|| "mono-habit-save".into())
                    .flex_shrink_0()
                    .h(px(24.))
                    .py_0()
                    .px_2()
                    .bg(content(0.1))
                    .when(!ready, |d| d.opacity(0.4))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if ready {
                            this.save_habit(cx);
                        }
                    })),
                ),
            )
            .child(
                div()
                    .id("mono-habit-editor")
                    .debug_selector(|| "mono-habit-editor".into())
                    .w_full()
                    .min_w_0()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("mono-habit-name-field")
                            .debug_selector(|| "mono-habit-name-field".into())
                            .w_full()
                            .min_w_0()
                            .flex_shrink_0()
                            .px_2()
                            .pt_3()
                            .child(
                                habit_field(name)
                                    .h(px(32.))
                                    .py_1p5()
                                    .font_weight(gpui::FontWeight::MEDIUM),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .flex_shrink_0()
                            .px_4()
                            .py_3()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(property("Runs", self.schedule_controls(cx)))
                            .child(property("Next", next)),
                    )
                    .child(
                        section("What it does")
                            .child(
                                div()
                                    .id("mono-habit-prompt-field")
                                    .debug_selector(|| "mono-habit-prompt-field".into())
                                    .w_full()
                                    .min_w_0()
                                    .flex_shrink_0()
                                    .child(habit_field(instructions)),
                            )
                            .when_some(self.habit_ui.error.clone(), |d, error| {
                                d.child(
                                    div()
                                        .px_2()
                                        .pt_2()
                                        .text_size(px(12.))
                                        .text_color(rgb(0xf87171))
                                        .child(error),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn habit_schedule(&self, cx: &App) -> Schedule {
        Schedule {
            schedule_kind: self.value("habit-kind", cx),
            time: self.value("habit-time", cx).trim().into(),
            minute: self.value("habit-minute", cx).parse().unwrap_or(60),
            day_of_week: self.value("habit-weekday", cx).parse().unwrap_or(7),
        }
    }

    pub(super) fn schedule_picker(
        &self,
        field: &'static str,
        label: &'static str,
        current: String,
        choices: Vec<(String, String)>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = choices
            .iter()
            .find(|(value, _)| value == &current)
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| current.clone());
        let anchor = AnchorSlot::default();
        let focus = self.fields[field].focus_handle(cx);
        let keyboard_choices = choices.clone();
        let keyboard_anchor = anchor.clone();
        let expanded = self.picker.is_some() && self.habit_ui.picker_field == Some(field);
        div()
            .id(field)
            .debug_selector(move || format!("mono-{field}-picker"))
            .role(Role::ComboBox)
            .aria_label(format!("{label}: {selected}"))
            .aria_expanded(expanded)
            .track_focus(&focus)
            .tab_index(0)
            .relative()
            .h(px(28.))
            .max_w_full()
            .flex_shrink_0()
            .px_2()
            .rounded_md()
            .bg(content(0.1))
            .hover(|s| s.bg(content(0.14)))
            .flex()
            .items_center()
            .gap_1()
            .text_size(px(12.))
            .line_height(px(16.))
            .child(anchor.probe())
            .child(div().min_w_0().truncate().child(selected))
            .child(icon("chevron-down", 12.))
            .on_key_down(
                cx.listener(move |this, event: &gpui::KeyDownEvent, window, cx| {
                    if matches!(
                        event.keystroke.key.as_str(),
                        "enter" | "space" | "down" | "up"
                    ) {
                        this.open_schedule_picker(
                            field,
                            label,
                            keyboard_choices.clone(),
                            keyboard_anchor.clone(),
                            window,
                            cx,
                        );
                        window.prevent_default();
                        cx.stop_propagation();
                    }
                }),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                window.focus(&this.fields[field].focus_handle(cx), cx);
                this.open_schedule_picker(
                    field,
                    label,
                    choices.clone(),
                    anchor.clone(),
                    window,
                    cx,
                );
            }))
            .into_any_element()
    }

    pub(super) fn open_schedule_picker(
        &mut self,
        field: &'static str,
        label: &'static str,
        choices: Vec<(String, String)>,
        anchor: AnchorSlot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self.value(field, cx);
        let items = choices
            .into_iter()
            .map(|(value, label)| {
                crate::ui::popover::MenuAction::new(value.clone(), label)
                    .checked(value == current)
                    .into()
            })
            .collect();
        let menu = cx.new(|cx| {
            Menu::new(label, items, window, cx)
                .width(240.)
                .select_value(&current)
        });
        let picked = cx.subscribe(&menu, move |this, _, event: &MenuEvent, cx| {
            if let MenuEvent::Picked(value, _) = event
                && let Some(input) = this.fields.get(field)
            {
                input.update(cx, |input, cx| input.set_text(value.to_string(), cx));
            }
            this.picker = None;
            this.habit_ui.picker_field = None;
            cx.notify();
        });
        self.habit_ui.picker_field = Some(field);
        self.picker = Some(OpenPicker {
            menu,
            anchor: Anchor::Slot(anchor),
            width: 240.,
            align: crate::ui::popover::Align::Start,
            _events: vec![picked],
        });
        cx.notify();
    }

    pub(super) fn schedule_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let kind = self.value("habit-kind", cx);
        let choices = |items: &[(&str, &str)]| {
            items
                .iter()
                .map(|(value, label)| (value.to_string(), label.to_string()))
                .collect()
        };
        let days = [
            "Sunday",
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
        ];
        div()
            .id("mono-habit-schedule")
            .debug_selector(|| "mono-habit-schedule".into())
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1p5()
            .py_0p5()
            .child(self.schedule_picker(
                "habit-kind",
                "Repeats",
                kind.clone(),
                choices(&[
                    ("daily", "Daily"),
                    ("weekdays", "Weekdays"),
                    ("weekly", "Weekly"),
                    ("hourly", "Hourly"),
                ]),
                cx,
            ))
            .when(kind == "weekly", |d| {
                d.child(div().text_color(content(0.45)).child("on")).child(
                    self.schedule_picker(
                        "habit-weekday",
                        "Day",
                        self.value("habit-weekday", cx),
                        days.into_iter()
                            .enumerate()
                            .map(|(ix, label)| (ix.to_string(), label.into()))
                            .collect(),
                        cx,
                    ),
                )
            })
            .child(div().text_color(content(0.45)).child("at"))
            .when(kind == "hourly", |d| {
                d.child(
                    self.schedule_picker(
                        "habit-minute",
                        "Minute",
                        self.value("habit-minute", cx),
                        [0, 15, 30, 45]
                            .into_iter()
                            .map(|minute| (minute.to_string(), format!(":{minute:02}")))
                            .collect(),
                        cx,
                    ),
                )
            })
            .when(kind != "hourly", |d| {
                d.child(
                    habit_field(self.fields["habit-time"].clone())
                        .w(px(84.))
                        .h(px(28.))
                        .bg(content(0.1)),
                )
            })
            .into_any_element()
    }
}
