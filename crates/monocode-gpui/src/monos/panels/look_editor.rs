use super::*;

pub(super) struct LookChanged {
    pub(super) mono: Mono,
}

/// Enter in the name field: MonoMenu commits the name and closes.
pub(super) struct LookSubmitted;

pub(super) struct LookEditor {
    pub(super) mono: Mono,
    pub(super) name: Entity<Input>,
    pub(super) color: Entity<Input>,
    pub(super) settings: bool,
    pub(super) custom_open: bool,
    pub(super) custom: Entity<super::super::color_picker::ColorPicker>,
    pub(super) focus: gpui::FocusHandle,
    pub(super) _events: Vec<Subscription>,
}

impl LookEditor {
    pub(super) fn new(mono: Mono, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| {
            let mut f = Input::field("Mono name", mono.label(), content(0.45), cx);
            f.set_text(mono.name.clone(), cx);
            f
        });
        let color = cx.new(|cx| {
            let mut f = Input::field("Custom Mono color", "#rrggbb", content(0.4), cx);
            f.set_text(format!("#{:06x}", mono.color), cx);
            f
        });
        let custom = cx.new(|cx| super::super::color_picker::ColorPicker::new(mono.color, cx));
        let events = vec![
            cx.subscribe(
                &custom,
                |this, _, event: &super::super::color_picker::Picked, cx| {
                    this.mono.color = event.0;
                    this.color.update(cx, |field, cx| {
                        field.set_text(format!("#{:06x}", event.0), cx)
                    });
                    cx.emit(LookChanged {
                        mono: this.mono.clone(),
                    });
                    cx.notify();
                },
            ),
            cx.subscribe(&name, |this, input, _: &input::Changed, cx| {
                this.mono.name = input.read(cx).content.chars().take(40).collect();
                cx.emit(LookChanged {
                    mono: this.mono.clone(),
                });
            }),
            cx.subscribe(&name, |_, _, _: &input::Submitted, cx| {
                cx.emit(LookSubmitted)
            }),
            cx.subscribe(&color, |this, input, _: &input::Changed, cx| {
                let text = input.read(cx).content.trim();
                if text.len() == 7
                    && let Ok(color) = u32::from_str_radix(text.trim_start_matches('#'), 16)
                {
                    this.mono.color = color;
                    this.custom
                        .update(cx, |picker, cx| picker.set_color(color, cx));
                    cx.emit(LookChanged {
                        mono: this.mono.clone(),
                    });
                    cx.notify();
                }
            }),
        ];
        Self {
            mono,
            name,
            color,
            settings: false,
            custom_open: false,
            custom,
            focus: cx.focus_handle(),
            _events: events,
        }
    }
}

impl gpui::Render for LookEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.settings {
            return self.settings_controls(cx);
        }
        // MonoMenu: the name, MascotPicker and the inline ColorPicker, each
        // ten columns wide.
        const COLOR_NAMES: [&str; 9] = [
            "Blue", "Coral", "Yellow", "Green", "Pink", "Purple", "Teal", "Orange", "Indigo",
        ];
        let preset = mono::COLORS_MONO.contains(&self.mono.color);
        let swatch = |id: gpui::ElementId, label: SharedString, ring: bool, dot: AnyElement| {
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(label)
                .aria_selected(ring)
                .flex_1()
                .min_w_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .size(px(20.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(ring, |d| d.border_2().border_color(content(0.8)))
                        .child(dot),
                )
        };
        div()
            .w_full()
            .child(
                div()
                    .debug_selector(|| "mono-options-name".into())
                    .mb_2()
                    .px_2p5()
                    .py_1p5()
                    .rounded_lg()
                    .border_1()
                    .border_color(content(0.1))
                    .bg(content(0.05))
                    .text_size(px(13.))
                    .child(self.name.clone()),
            )
            .child(
                div()
                    .mb_2()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .id("mono-mascot-options")
                            .role(Role::RadioGroup)
                            .aria_label("Mascot")
                            .px_1()
                            .flex()
                            .gap(px(2.))
                            .children(MASCOTS.iter().enumerate().map(|(ix, name)| {
                                let selected = self.mono.mascot == *name;
                                div()
                                    .id(("mono-mascot", ix))
                                    .debug_selector(move || format!("mono-mascot-{ix}"))
                                    .role(Role::RadioButton)
                                    .aria_label(*name)
                                    .aria_selected(selected)
                                    .flex_1()
                                    .min_w_0()
                                    .h(px(32.))
                                    .rounded_md()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .when(selected, |d| {
                                        d.bg(content(0.1)).border_1().border_color(content(0.2))
                                    })
                                    .when(!selected, |d| d.hover(|s| s.bg(content(0.06))))
                                    .child(crate::ui::avatar::avatar(
                                        name,
                                        self.mono.color,
                                        "idle",
                                        20.,
                                        !selected,
                                    ))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.mono.mascot = (*name).into();
                                        cx.emit(LookChanged {
                                            mono: this.mono.clone(),
                                        });
                                        cx.notify();
                                    }))
                            })),
                    )
                    .child(
                        div()
                            .id("mono-color-options")
                            .role(Role::Group)
                            .aria_label("Color")
                            .child(
                                div()
                                    .px_1()
                                    .flex()
                                    .gap(px(2.))
                                    .children(mono::COLORS_MONO.into_iter().enumerate().map(
                                        |(ix, color)| {
                                            swatch(
                                                ("mono-color", ix).into(),
                                                COLOR_NAMES[ix].into(),
                                                self.mono.color == color,
                                                div()
                                                    .size(px(14.))
                                                    .rounded_full()
                                                    .bg(rgb(color))
                                                    .into_any_element(),
                                            )
                                            .debug_selector(move || format!("mono-color-{ix}"))
                                            .on_click(
                                                cx.listener(move |this, _, _, cx| {
                                                    this.mono.color = color;
                                                    this.custom_open = false;
                                                    this.custom.update(cx, |picker, cx| {
                                                        picker.set_color(color, cx)
                                                    });
                                                    cx.emit(LookChanged {
                                                        mono: this.mono.clone(),
                                                    });
                                                    cx.notify();
                                                }),
                                            )
                                        },
                                    ))
                                    .child(
                                        swatch(
                                            "mono-custom-color".into(),
                                            "Custom color".into(),
                                            !preset || self.custom_open,
                                            div()
                                                .relative()
                                                .size(px(14.))
                                                .rounded_full()
                                                .overflow_hidden()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .map(|d| {
                                                    if preset {
                                                        d.child(div().absolute().inset_0().child(
                                                            super::super::color_picker::spectrum(),
                                                        ))
                                                        .child(
                                                            icon("pipette", 8.)
                                                                .relative()
                                                                .text_color(gpui::white()),
                                                        )
                                                    } else {
                                                        d.bg(rgb(self.mono.color))
                                                    }
                                                })
                                                .into_any_element(),
                                        )
                                        .debug_selector(|| "mono-custom-color".into())
                                        .aria_expanded(self.custom_open)
                                        .on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.custom_open = !this.custom_open;
                                                cx.notify();
                                            }),
                                        ),
                                    ),
                            )
                            .when(self.custom_open, |d| {
                                d.child(div().px_1().pt_2().child(self.custom.clone()))
                            }),
                    ),
            )
            .into_any_element()
    }
}

impl LookEditor {
    pub(super) fn settings_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        const COLOR_NAMES: [&str; 9] = [
            "Blue", "Coral", "Yellow", "Green", "Pink", "Purple", "Teal", "Orange", "Indigo",
        ];
        div()
            .id("mono-settings-look-picker")
            .debug_selector(|| "mono-settings-look-picker".into())
            .role(Role::Dialog)
            .aria_label("Mascot and color")
            .track_focus(&self.focus)
            .p_3()
            .w_full()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .id("mono-mascot-options")
                    .role(Role::RadioGroup)
                    .aria_label("Mascot")
                    .px_1()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(MASCOTS.chunks(5).enumerate().map(|(row, names)| {
                        div().flex().gap_1().children(names.iter().enumerate().map(
                            |(col, name)| {
                                let selected = self.mono.mascot == *name;
                                div()
                                    .id(("settings-mascot", row * 5 + col))
                                    .role(Role::RadioButton)
                                    .aria_label(*name)
                                    .aria_selected(selected)
                                    .h(px(48.))
                                    .flex_1()
                                    .min_w_0()
                                    .rounded_lg()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .when(selected, |d| {
                                        d.bg(content(0.1)).border_1().border_color(content(0.2))
                                    })
                                    .hover(|d| d.bg(content(0.06)))
                                    .child(crate::ui::avatar::avatar(
                                        name,
                                        self.mono.color,
                                        "idle",
                                        32.,
                                        !selected,
                                    ))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.mono.mascot = (*name).into();
                                        cx.emit(LookChanged {
                                            mono: this.mono.clone(),
                                        });
                                        cx.notify();
                                    }))
                            },
                        ))
                    })),
            )
            .child(div().h(px(1.)).bg(content(0.1)))
            .child(
                div()
                    .id("mono-color-options")
                    .role(Role::Group)
                    .aria_label("Color")
                    .px_1()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children((0..2).map(|row| {
                        div().flex().justify_between().children((0..5).map(|col| {
                            let index = row * 5 + col;
                            if index < 9 {
                                let color = mono::COLORS_MONO[index];
                                let selected = self.mono.color == color;
                                div()
                                    .id(("settings-color", index))
                                    .role(Role::Button)
                                    .aria_label(COLOR_NAMES[index])
                                    .aria_selected(selected)
                                    .size(px(36.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .child(
                                        div()
                                            .size(px(32.))
                                            .rounded_full()
                                            .border_2()
                                            .border_color(if selected {
                                                content(0.8)
                                            } else {
                                                content(0.)
                                            })
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(
                                                div().size(px(28.)).rounded_full().bg(rgb(color)),
                                            ),
                                    )
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.mono.color = color;
                                        this.custom_open = false;
                                        this.custom
                                            .update(cx, |picker, cx| picker.set_color(color, cx));
                                        this.color.update(cx, |f, cx| {
                                            f.set_text(format!("#{color:06x}"), cx)
                                        });
                                        cx.emit(LookChanged {
                                            mono: this.mono.clone(),
                                        });
                                        cx.notify();
                                    }))
                                    .into_any_element()
                            } else {
                                div()
                                    .id("settings-custom-color")
                                    .role(Role::Button)
                                    .aria_label("Custom color")
                                    .size(px(36.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .child(
                                        div()
                                            .size(px(28.))
                                            .rounded_full()
                                            .when(
                                                !mono::COLORS_MONO.contains(&self.mono.color),
                                                |d| d.bg(rgb(self.mono.color)),
                                            )
                                            .when(
                                                mono::COLORS_MONO.contains(&self.mono.color),
                                                |d| {
                                                    d.child(div().absolute().inset_0().child(
                                                        super::super::color_picker::spectrum(),
                                                    ))
                                                },
                                            )
                                            .relative()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(
                                                icon("pipette", 14.)
                                                    .relative()
                                                    .text_color(gpui::white()),
                                            ),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.custom_open = !this.custom_open;
                                        cx.notify();
                                    }))
                                    .into_any_element()
                            }
                        }))
                    })),
            )
            .when(self.custom_open, |d| {
                d.child(div().px_1().pt_2().child(self.custom.clone()))
            })
            .into_any_element()
    }
}
