use super::*;

impl VisualView {
    pub(super) fn node(&mut self, node: &Node, key: String, cx: &mut Context<Self>) -> AnyElement {
        self.active.insert(key.clone());
        match node {
            Node::Column { children, gap }
            | Node::Row { children, gap }
            | Node::Grid { children, gap, .. }
            | Node::Card { children, gap } => {
                let mut layout = div().w_full().min_w_0().gap(px(*gap));
                if matches!(node, Node::Card { .. }) {
                    layout = layout
                        .p_4()
                        .rounded_xl()
                        .bg(content(0.03))
                        .border_1()
                        .border_color(content(0.08));
                }
                if let Node::Grid { columns, .. } = node {
                    let responsive =
                        ((*columns).min((self.width.get() / 180.).floor().max(1.) as usize)).max(1);
                    layout = layout.grid().grid_cols(responsive as u16);
                } else if matches!(node, Node::Row { .. }) {
                    layout = layout.flex().flex_wrap().items_start();
                } else {
                    layout = layout.flex().flex_col();
                }
                for (i, child) in children.iter().enumerate() {
                    let element = self.node(child, format!("{key}/{i}"), cx);
                    layout = layout.child(
                        div()
                            .min_w_0()
                            .when(matches!(node, Node::Row { .. }), |d| {
                                d.flex_1().min_w(px(120.))
                            })
                            .child(element),
                    );
                }
                layout.into_any_element()
            }
            Node::Text {
                text,
                size,
                color: token,
            } => {
                let font_size = match size {
                    TextSize::Caption => 11.,
                    TextSize::Small => 12.,
                    TextSize::Body => 14.,
                    TextSize::Subtitle => 17.,
                    TextSize::Title => 22.,
                    TextSize::Hero => 34.,
                };
                div()
                    .min_w_0()
                    .text_size(px(font_size))
                    .line_height(px(font_size * 1.45))
                    .text_color(color(token))
                    .when(
                        matches!(size, TextSize::Subtitle | TextSize::Title | TextSize::Hero),
                        |d| d.font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(text.clone())
                    .into_any_element()
            }
            Node::Stat {
                label,
                value,
                detail,
                color: token,
                delta,
                trend,
                lower_is_better,
            } => {
                let (delta, detail) = match (delta, detail) {
                    (Some(delta), detail) => (Some(delta.clone()), detail.clone()),
                    (None, Some(detail)) => match charts::split_delta(detail) {
                        Some((delta, rest)) => (
                            Some(delta.to_owned()),
                            (!rest.is_empty()).then(|| rest.to_owned()),
                        ),
                        None => (None, Some(detail.clone())),
                    },
                    (None, None) => (None, None),
                };
                let tint = if matches!(
                    token,
                    ThemeColor::Ink | ThemeColor::Muted | ThemeColor::Faint
                ) {
                    accent_color()
                } else {
                    color(token)
                };
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(content(0.6))
                            .truncate()
                            .child(label.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(30.))
                            .line_height(px(38.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .font_features(crate::ui::panel::tabular())
                            .text_color(color(token))
                            .child(value.clone()),
                    )
                    .when(delta.is_some() || detail.is_some(), |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap_x_2()
                                .children(
                                    delta.as_deref().map(|d| charts::delta(d, *lower_is_better)),
                                )
                                .children(detail.map(|detail| {
                                    div()
                                        .text_size(px(12.))
                                        .text_color(content(0.55))
                                        .child(detail)
                                })),
                        )
                    })
                    .when(trend.len() > 1, |d| {
                        d.child(
                            div()
                                .pt_2()
                                .child(charts::trend(trend.clone(), tint, self.motion)),
                        )
                    })
                    .into_any_element()
            }
            Node::Table { columns, rows } => {
                let mut table = div()
                    .id(SharedString::from(format!("visual-table-{key}")))
                    .w_full()
                    .overflow_x_scroll()
                    .flex()
                    .flex_col()
                    .text_size(px(12.));
                let count = columns.len().max(1);
                let width = (self.width.get() / count as f32).max(100.);
                let row = |cells: &[String], header: bool| {
                    div()
                        .flex()
                        .min_w(px(width * count as f32))
                        .border_b_1()
                        .border_color(content(0.08))
                        .when(header, |d| {
                            d.font_weight(FontWeight::SEMIBOLD)
                                .text_color(content(0.65))
                        })
                        .children(cells.iter().map(|cell| {
                            div()
                                .w(px(width))
                                .flex_shrink_0()
                                .px_2()
                                .py_2()
                                .child(cell.clone())
                        }))
                };
                table = table.child(row(columns, true));
                for cells in rows {
                    table = table.child(row(cells, false));
                }
                table.into_any_element()
            }
            Node::Button {
                label,
                handler,
                disabled,
            } => {
                let handler = handler.clone();
                let keyboard = handler.clone();
                button(SharedString::from(format!("visual-button-{key}")))
                    .debug_selector(move || format!("visual-button-{key}"))
                    .aria_label(label.clone())
                    .child(label.clone())
                    .when(*disabled, |d| d.opacity(0.4).cursor_default())
                    .when(!disabled, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            this.dispatch(&handler, Value::Null, cx)
                        }))
                        .on_key_down(cx.listener(
                            move |this, event, _, cx| {
                                if activated(event) {
                                    this.dispatch(&keyboard, Value::Null, cx);
                                    cx.stop_propagation();
                                }
                            },
                        ))
                    })
                    .into_any_element()
            }
            Node::Toggle {
                label,
                value,
                handler,
            } => {
                let next = !*value;
                let handler = handler.clone();
                let keyboard = handler.clone();
                div()
                    .id(SharedString::from(format!("visual-toggle-{key}")))
                    .debug_selector(move || format!("visual-toggle-{key}"))
                    .role(Role::Switch)
                    .aria_label(label.clone())
                    .aria_toggled(if *value {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .focusable()
                    .tab_stop(true)
                    .flex()
                    .items_center()
                    .gap_2()
                    .py_1()
                    .cursor_pointer()
                    .child(
                        div()
                            .w(px(30.))
                            .h(px(18.))
                            .rounded_full()
                            .bg(if *value {
                                accent_color()
                            } else {
                                content(0.18)
                            })
                            .relative()
                            .child(
                                div()
                                    .absolute()
                                    .top(px(3.))
                                    .left(px(if *value { 15. } else { 3. }))
                                    .size(px(12.))
                                    .rounded_full()
                                    .bg(content(1.)),
                            ),
                    )
                    .child(label.clone())
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.dispatch(&handler, json!(next), cx)),
                    )
                    .on_key_down(cx.listener(move |this, event, _, cx| {
                        if activated(event) {
                            this.dispatch(&keyboard, json!(next), cx);
                            cx.stop_propagation();
                        }
                    }))
                    .into_any_element()
            }
            Node::Slider {
                label,
                value,
                min,
                max,
                step,
                handler,
            } => {
                let fraction =
                    ((*value - *min) / (*max - *min).max(f64::EPSILON)).clamp(0., 1.) as f32;
                let bounds = self
                    .tracks
                    .entry(key.clone())
                    .or_insert_with(|| Rc::new(Cell::new(None)))
                    .clone();
                let min = *min;
                let max = *max;
                let step = *step;
                let current = *value;
                let handler = handler.clone();
                let keyboard = handler.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .min_w(px(120.))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .gap_3()
                            .text_size(px(12.))
                            .child(label.clone())
                            .child(format_number(*value)),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("visual-slider-{key}")))
                            .debug_selector({
                                let key = key.clone();
                                move || format!("visual-slider-{key}")
                            })
                            .role(Role::Slider)
                            .aria_label(label.clone())
                            .focusable()
                            .tab_stop(true)
                            .w_full()
                            .h(px(20.))
                            .flex()
                            .items_center()
                            .relative()
                            .cursor_pointer()
                            .child(
                                canvas(move |rect, _, _| bounds.set(Some(rect)), |_, _, _, _| {})
                                    .absolute()
                                    .size_full(),
                            )
                            .child(div().w_full().h(px(4.)).rounded_full().bg(content(0.12)))
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top(px(8.))
                                    .h(px(4.))
                                    .w(relative(fraction))
                                    .rounded_full()
                                    .bg(accent_color()),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top(px(4.))
                                    .left(relative(fraction))
                                    .ml(px(-6.))
                                    .size(px(12.))
                                    .rounded_full()
                                    .bg(accent_color()),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                                    this.slider_drag = Some(SliderDrag {
                                        key: key.clone(),
                                        handler: handler.clone(),
                                        min,
                                        max,
                                        step,
                                    });
                                    this.slide(event.position.x, cx);
                                    cx.stop_propagation();
                                }),
                            )
                            .on_key_down(cx.listener(
                                move |this, event: &gpui::KeyDownEvent, _, cx| {
                                    let next = match event.keystroke.key.as_str() {
                                        "left" | "down" => current - step,
                                        "right" | "up" => current + step,
                                        "home" => min,
                                        "end" => max,
                                        _ => return,
                                    };
                                    this.dispatch(&keyboard, json!(next.clamp(min, max)), cx);
                                    cx.stop_propagation();
                                },
                            )),
                    )
                    .into_any_element()
            }
            Node::Tabs {
                labels,
                selected,
                handler,
            } => {
                let mut tabs = div().flex().flex_wrap().gap_1();
                for (i, label) in labels.iter().enumerate() {
                    let handler = handler.clone();
                    let keyboard = handler.clone();
                    tabs =
                        tabs.child(
                            tab_button(
                                SharedString::from(format!("visual-tab-{key}-{i}")),
                                i == *selected,
                            )
                            .debug_selector({
                                let key = key.clone();
                                move || format!("visual-tab-{key}-{i}")
                            })
                            .aria_label(label.clone())
                            .aria_toggled(if i == *selected {
                                gpui::Toggled::True
                            } else {
                                gpui::Toggled::False
                            })
                            .child(label.clone())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.dispatch(&handler, json!(i), cx)
                            }))
                            .on_key_down(cx.listener(
                                move |this, event, _, cx| {
                                    if activated(event) {
                                        this.dispatch(&keyboard, json!(i), cx);
                                        cx.stop_propagation();
                                    }
                                },
                            )),
                        );
                }
                tabs.into_any_element()
            }
            Node::Input {
                label,
                value,
                placeholder,
                handler,
            } => {
                if !self.inputs.contains_key(&key) {
                    let input = cx.new(|cx| {
                        let mut input = crate::ui::input::Input::field(
                            label.clone(),
                            placeholder.clone().unwrap_or_default(),
                            content(0.4),
                            cx,
                        );
                        input.set_max_chars(4096);
                        input.set_text(value.clone(), cx);
                        input
                    });
                    let changed_key = key.clone();
                    let subscription = cx.subscribe(
                        &input,
                        move |this, input, _: &crate::ui::input::Changed, cx| {
                            let value = input.read(cx).content.to_string();
                            let Some(slot) = this.inputs.get_mut(&changed_key) else {
                                return;
                            };
                            if value == slot.source_value {
                                return;
                            }
                            slot.source_value = value.clone();
                            let handler = slot.handler.clone();
                            this.dispatch(&handler, json!(value), cx);
                        },
                    );
                    self.inputs.insert(
                        key.clone(),
                        InputSlot {
                            input,
                            handler: handler.clone(),
                            source_value: value.clone(),
                            _subscription: subscription,
                        },
                    );
                }
                let slot = self.inputs.get_mut(&key).unwrap();
                slot.handler = handler.clone();
                slot.source_value = value.clone();
                if slot.input.read(cx).content.as_ref() != value {
                    slot.input
                        .update(cx, |input, cx| input.set_text(value.clone(), cx));
                }
                // The placeholder says what to type; the label stays for
                // assistive technology only.
                div()
                    .id(SharedString::from(format!("visual-input-{key}")))
                    .debug_selector({
                        let key = key.clone();
                        move || format!("visual-input-{key}")
                    })
                    .aria_label(label.clone())
                    .min_w_0()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(content(0.14))
                    .child(slot.input.clone())
                    .into_any_element()
            }
            Node::Select {
                label,
                options,
                selected,
                handler,
            } => {
                // The options name themselves; the label stays for assistive
                // technology only.
                let mut choices = div()
                    .id(SharedString::from(format!("visual-select-{key}")))
                    .role(Role::Group)
                    .aria_label(label.clone())
                    .flex()
                    .flex_wrap()
                    .gap_1();
                for (i, option) in options.iter().enumerate() {
                    let handler = handler.clone();
                    let keyboard = handler.clone();
                    choices = choices.child(
                        button(SharedString::from(format!("visual-option-{key}-{i}")))
                            .debug_selector({
                                let key = key.clone();
                                move || format!("visual-option-{key}-{i}")
                            })
                            .aria_label(option.clone())
                            .child(option.clone())
                            .when(i == *selected, |d| {
                                d.bg(content(0.09))
                                    .border_color(accent_color())
                                    .text_color(content(1.))
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.dispatch(&handler, json!(i), cx)
                            }))
                            .on_key_down(cx.listener(move |this, event, _, cx| {
                                if activated(event) {
                                    this.dispatch(&keyboard, json!(i), cx);
                                    cx.stop_propagation();
                                }
                            })),
                    );
                }
                choices.into_any_element()
            }
            Node::Chart {
                chart_type,
                series,
                labels,
                height,
            } => charts::chart(
                *chart_type,
                series.clone(),
                labels.clone(),
                *height,
                self.motion,
                format!("visual-chart-{key}").into(),
            ),
            Node::Gauge {
                label,
                value,
                min,
                max,
                unit,
                color: token,
            } => charts::radial::gauge(
                label.clone(),
                *value,
                *min,
                *max,
                unit.clone(),
                *token,
                self.motion,
            ),
            Node::Donut {
                slices,
                label,
                value,
            } => charts::radial::donut(
                slices.clone(),
                label.clone(),
                value.clone(),
                self.motion,
                format!("visual-donut-{key}").into(),
            ),
            Node::Heatmap {
                values,
                rows,
                columns,
                color: token,
            } => charts::heatmap(
                values.clone(),
                rows.clone(),
                columns.clone(),
                *token,
                self.motion,
                format!("visual-heatmap-{key}").into(),
            ),
            Node::Progress {
                label,
                value,
                max,
                detail,
                color: token,
            } => charts::radial::progress(
                label.clone(),
                *value,
                *max,
                detail.clone(),
                *token,
                self.motion,
            ),
            Node::Canvas {
                width,
                height,
                shapes,
            } => drawing(*width, *height, shapes.clone()),
            Node::Scene {
                width: _,
                height,
                objects,
                camera,
            } => self.scene(key, *height, objects.clone(), camera.clone(), cx),
        }
    }
}
