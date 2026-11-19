use super::*;

impl Btw {
    pub(super) fn shape(&self, sheet: Bounds<Pixels>, now: Instant) -> Shape {
        let rest = |top: f32, bottom: f32| Corners {
            top_left: px(top),
            top_right: px(top),
            bottom_left: px(bottom),
            bottom_right: px(bottom),
        };
        let (Some(morph), Some(home)) = (self.morph, self.home.get()) else {
            return Shape {
                clip: sheet,
                corners: rest(SHEET_RADIUS, 0.),
                layer: if self.morph.is_some_and(|m| m.start.is_none()) {
                    0.
                } else {
                    1.
                },
                outline: None,
                composer: 1.,
            };
        };
        let e = morph.eased(now);
        // Openness: 0 is the composer box, 1 the full sheet.
        let open = if morph.opening { e } else { 1. - e };
        let layer = if morph.opening {
            (e / 0.3).min(1.)
        } else if e < 0.2 {
            1.
        } else {
            1. - (e - 0.2) / 0.8
        };
        let outline = self.side_box.get().map(|away| {
            (
                lerp_rect(home, away, open),
                if morph.start.is_some() {
                    outline_opacity(e)
                } else {
                    0.
                },
            )
        });
        Shape {
            clip: lerp_rect(home, sheet, open),
            corners: rest(
                lerp(BOX_RADIUS, SHEET_RADIUS, open),
                lerp(BOX_RADIUS, 0., open),
            ),
            layer,
            outline,
            composer: if !morph.opening {
                self.body_motion(now).0
            } else if e < 0.45 {
                0.
            } else {
                (e - 0.45) / 0.55
            },
        }
    }

    pub(super) fn header(&self, cx: &mut Context<Self>) -> gpui::Div {
        let tabs: Vec<AnyElement> = self
            .threads
            .iter()
            .map(|thread| {
                let id = thread.id.clone();
                let remove_id = id.clone();
                let group: SharedString = format!("btw-tab-group-{id}").into();
                let selected = self.active.as_ref() == Some(&id);
                let question = thread
                    .rows
                    .iter()
                    .find(|row| row.role == "user")
                    .map(|row| row.text.as_str());
                let label = compact_question(question);
                let persisted = !thread.rows.is_empty();
                div()
                    .id(group.clone())
                    .group(group.clone())
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .h(px(28.))
                    .max_w(px(240.))
                    .rounded_md()
                    .map(|d| {
                        if selected {
                            d.bg(content(0.1)).text_color(content(1.))
                        } else {
                            d.text_color(content(0.5))
                                .hover(|s| s.bg(content(0.05)).text_color(content(0.8)))
                        }
                    })
                    .child(
                        div()
                            .id(format!("btw-tab-{id}"))
                            .role(Role::Tab)
                            .aria_label(question.unwrap_or("New question").to_owned())
                            .aria_selected(selected)
                            .cursor_pointer()
                            .h_full()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .rounded_md()
                            .pl(px(10.))
                            .pr(px(4.))
                            .text_size(px(12.))
                            .children(status_color(&thread.status).map(|color| {
                                div().size(px(6.)).flex_shrink_0().rounded_full().bg(color)
                            }))
                            .child(div().min_w_0().truncate().child(label.clone()))
                            .on_click(cx.listener(move |this, _, _, cx| this.select(&id, cx))),
                    )
                    .child(
                        div()
                            .id(format!("btw-remove-{remove_id}"))
                            .role(Role::Button)
                            .aria_label(if persisted {
                                format!("Delete “{label}”")
                            } else {
                                "Discard new question".into()
                            })
                            .cursor_pointer()
                            .mr(px(4.))
                            .size(px(20.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .text_color(content(0.4))
                            .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
                            .when(!selected, |d| {
                                d.opacity(0.).group_hover(group, |s| s.opacity(1.))
                            })
                            .child(icon("close", 12.))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.remove(&remove_id, cx)),
                            ),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .h(px(44.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .pl(px(10.))
            .pr_2()
            .border_b_1()
            .border_color(content(0.08))
            .child(
                div()
                    .id("btw-tabs")
                    .role(Role::TabList)
                    .aria_label("Side questions")
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .overflow_x_scroll()
                    .children(tabs),
            )
            .child(
                header_button("btw-new", "New side question", "plus", 14.)
                    .on_click(cx.listener(|this, _, _, cx| this.new_thread(cx))),
            )
            .child(
                header_button(
                    "btw-close",
                    "Back to the conversation (Esc)",
                    "chevron-down",
                    16.,
                )
                .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
            )
    }

    /// `.btw-error`: the red "Couldn't finish" card with its Retry.
    pub(super) fn error_card(&self, running: bool, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let thread_error = self
            .active()
            .filter(|thread| thread.status == "error")
            .map(|thread| {
                thread
                    .error
                    .clone()
                    .filter(|e| !e.is_empty())
                    .unwrap_or_else(|| {
                        format!(
                            "{} could not answer this side question.",
                            Kind::from_id(&thread.provider).label()
                        )
                    })
            });
        let retry = thread_error.is_some();
        let message = thread_error.or_else(|| self.error.clone())?;
        Some(
            div().flex_shrink_0().px_5().pb_3().child(
                div()
                    .id("btw-error")
                    .role(Role::Alert)
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap_3()
                    .px(px(11.))
                    .py(px(10.))
                    .border_1()
                    .border_color(red(0xf87171, 0.24))
                    .rounded(px(10.))
                    .bg(red(0xf87171, 0.07))
                    .child(
                        div()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(red(0xfecaca, 0.7))
                                    .child("COULDN’T FINISH"),
                            )
                            .child(
                                div()
                                    .mt_1()
                                    .text_size(px(12.))
                                    .line_height(px(18.))
                                    .text_color(red(0xfee2e2, 0.75))
                                    .child(message),
                            ),
                    )
                    .when(retry, |d| {
                        d.child(
                            div()
                                .id("btw-retry")
                                .role(Role::Button)
                                .aria_label("Retry side question")
                                .cursor_pointer()
                                .flex_shrink_0()
                                .h(px(24.))
                                .px_2()
                                .flex()
                                .items_center()
                                .gap_1()
                                .rounded_md()
                                .text_size(px(11.))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(red(0xfee2e2, 0.8))
                                .hover(|s| s.bg(red(0xfecaca, 0.1)).text_color(red(0xfef2f2, 1.)))
                                .when(running, |d| d.opacity(0.5))
                                .child(icon("refresh-cw", 12.))
                                .child("Retry")
                                .on_click(cx.listener(|this, _, _, cx| this.retry(cx))),
                        )
                    }),
            ),
        )
    }

    /// The side composer: the session composer's box with only its model
    /// picker and send button (Composer `compact` and `hideTopBar`).
    pub(super) fn composer_box(
        &self,
        running: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let can_submit =
            !self.loading && !running && !self.input.read(cx).content.trim().is_empty();
        let (provider, model) = self
            .active()
            .map(|thread| {
                (
                    thread.provider.clone(),
                    crate::chat::composer::pickers::model_name(
                        Kind::from_id(&thread.provider),
                        thread.model.as_deref(),
                    ),
                )
            })
            .unwrap_or_else(|| (self.target.provider.id().into(), "Model".into()));
        let focused = self.input.focus_handle(cx).is_focused(window);
        div().flex_shrink_0().px(px(6.)).pb(px(6.)).child(
            div()
                .id("btw-composer-box")
                .relative()
                .rounded(px(BOX_RADIUS))
                .border_1()
                .border_color(composer_border(focused))
                .shadow(composer_shadow())
                .flex()
                .flex_col()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        window.focus(&this.input.focus_handle(cx), cx)
                    }),
                )
                .child(self.side_box.probe())
                .child(composer_surface(BOX_RADIUS))
                .child(
                    div()
                        .px_3()
                        .pt(px(14.))
                        .pb(px(14.))
                        .child(self.input.clone()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_2()
                        .pb(px(10.))
                        .child(
                            tool_pill("btw-model", "Model")
                                .min_w_0()
                                .max_w(px(220.))
                                .flex_shrink(1.)
                                .aria_label(format!(
                                    "{}, {}",
                                    Kind::from_id(&provider).label(),
                                    model
                                ))
                                .aria_expanded(self.models.is_some())
                                .when(running, |d| d.opacity(0.5))
                                .child(self.model_anchor.probe())
                                .child(provider_icon(&provider, 16.))
                                .child(pill_label(model))
                                .when_some(self.effort_label(), |d, effort| {
                                    d.child(
                                        div()
                                            .flex_shrink_0()
                                            .text_size(px(11.))
                                            .text_color(content(0.5))
                                            .child(effort),
                                    )
                                })
                                .child(icon("chevron-down", 12.))
                                .on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.model_picker(window, cx)
                                    }),
                                ),
                        )
                        .child(div().flex_1())
                        .child(
                            send_button(
                                "btw-send",
                                if running { "Stop" } else { "Send" },
                                running,
                                "send",
                                running || can_submit,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this
                                    .active()
                                    .is_some_and(|thread| thread.status == "running")
                                {
                                    this.stop(cx);
                                } else {
                                    this.submit(cx);
                                }
                            })),
                        ),
                ),
        )
    }
}

impl Render for Btw {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        self.step_motion(now, window, cx);
        if !self.rendered {
            return div().into_any_element();
        }
        if self.response_subscription.is_none() {
            self.response_subscription = Some(cx.subscribe_in(
                &self.transcript,
                window,
                |this, _, request: &ResponseRequest, window, cx| {
                    this.response_request(request, window, cx)
                },
            ));
        }
        if self.focus_pending && self.open {
            window.focus(&self.input.focus_handle(cx), cx);
            self.focus_pending = false;
        }
        let open = self.open;
        let running = self
            .active()
            .is_some_and(|thread| thread.status == "running");
        let (body, rise) = self.body_motion(now);
        let body_in = |d: gpui::Div| d.relative().top(px(rise)).opacity(body);
        let overlay = self.overlay.get();
        // `top: clamp(2.5rem, 18%, 12rem)` under BtwSheet's max width.
        let sheet = overlay.map(|o| {
            let (w, h) = (f32::from(o.size.width), f32::from(o.size.height));
            let width = w.min(SHEET_WIDTH);
            let top = (h * 0.18).clamp(40., 192.);
            Bounds::new(
                point(o.origin.x + px((w - width) * 0.5), o.origin.y + px(top)),
                size(px(width), px(h - top)),
            )
        });
        let shape = sheet.map(|sheet| self.shape(sheet, now));
        let sheet_content = div()
            .id("btw-sheet")
            .size_full()
            .flex()
            .flex_col()
            .role(Role::Dialog)
            .aria_label("By-the-way conversations")
            .when(open, |d| {
                d.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            })
            .child(body_in(self.header(cx)))
            .child(
                body_in(div().flex_1().min_h_0()).child(
                    self.transcript
                        .clone()
                        .cached(StyleRefinement::default().size_full()),
                ),
            )
            .children(self.error_card(running, cx).map(body_in))
            .child(
                self.composer_box(running, window, cx)
                    .opacity(shape.as_ref().map_or(1., |shape| shape.composer)),
            );
        let models = self.models.as_ref().map(|menu| {
            menu_popover(menu, Anchor::Slot(self.model_anchor.clone()), cx).side(Side::Top)
        });
        let mut root = div()
            .absolute()
            .inset_0()
            .child(self.overlay.layout_probe())
            .when(open, |d| {
                d.child(div().absolute().inset_0().on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.close(window, cx)),
                ))
            });
        match (overlay, sheet, shape) {
            (Some(overlay), Some(sheet), Some(shape)) => {
                let clip = shape.clip;
                let corners = shape.corners;
                let local = |at: Point<Pixels>, from: Point<Pixels>| at - from;
                root = root
                    .child(
                        div().absolute().inset_0().opacity(shape.layer).child(
                            canvas(
                                |_, _, _| {},
                                move |_, _, window, _| {
                                    window.paint_drop_shadows(
                                        clip,
                                        corners,
                                        &crate::ui::popover::shadow(),
                                    );
                                    crate::ui::popover::paint_glass(
                                        clip,
                                        corners,
                                        crate::ui::popover::glass_tint(),
                                        window,
                                    );
                                    // `.btw-sheet-glass`: no bottom border.
                                    window.paint_quad(
                                        gpui::fill(clip, gpui::transparent_black())
                                            .corner_radii(corners)
                                            .border_widths(gpui::Edges {
                                                top: px(1.),
                                                right: px(1.),
                                                bottom: px(0.),
                                                left: px(1.),
                                            })
                                            .border_color(content(0.1)),
                                    );
                                },
                            )
                            .size_full(),
                        ),
                    )
                    .child({
                        let at = local(clip.origin, overlay.origin);
                        let inner = local(sheet.origin, clip.origin);
                        div()
                            .absolute()
                            .left(at.x)
                            .top(at.y)
                            .w(clip.size.width)
                            .h(clip.size.height)
                            .overflow_hidden()
                            .opacity(shape.layer)
                            .child(
                                div()
                                    .absolute()
                                    .left(inner.x)
                                    .top(inner.y)
                                    .w(sheet.size.width)
                                    .h(sheet.size.height)
                                    .child(sheet_content),
                            )
                    })
                    .when_some(shape.outline, |d, (outline, opacity)| {
                        d.child(
                            canvas(
                                |_, _, _| {},
                                move |_, _, window, _| paint_outline(outline, opacity, window),
                            )
                            .absolute()
                            .size_full(),
                        )
                    });
            }
            // The first frame, before the overlay has been measured.
            _ => {
                root = root.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .top(gpui::relative(0.18))
                        .mx_auto()
                        .w_full()
                        .max_w(px(SHEET_WIDTH))
                        .opacity(if self.morph.is_some() { 0. } else { 1. })
                        .child(sheet_content),
                );
            }
        }
        if let (Some((born, seed)), Some(away)) = (self.question_burst, self.side_box.get()) {
            root = root.child(
                canvas(
                    |_, _, _| {},
                    move |_, _, window, cx| paint_question_burst(away, born, seed, window, cx),
                )
                .absolute()
                .size_full(),
            );
        }
        root.children(models).into_any_element()
    }
}

/// `.btw-morph-frame`: the composer's outline between its two boxes.
pub(super) fn paint_outline(bounds: Bounds<Pixels>, opacity: f32, window: &mut Window) {
    if opacity <= 0. {
        return;
    }
    let corners = Corners::all(px(BOX_RADIUS));
    if crate::ui::theme::is_light() {
        let shadows: Vec<_> = composer_shadow()
            .into_iter()
            .map(|mut shadow| {
                shadow.color.a *= opacity;
                shadow
            })
            .collect();
        window.paint_drop_shadows(bounds, corners, &shadows);
    }
    let mut fill = composer_fill();
    fill.a *= opacity;
    let mut border = content(0.12);
    border.a *= opacity;
    window.paint_quad(
        gpui::fill(bounds, fill)
            .corner_radii(corners)
            .border_widths(px(1.))
            .border_color(border),
    );
}

/// BtwQuestionBurst: question marks drift up out of the side composer while
/// its box glows and a sheen crosses it, in the picked accent.
pub(super) fn paint_question_burst(
    away: Bounds<Pixels>,
    born: Instant,
    seed: u64,
    window: &mut Window,
    cx: &mut App,
) {
    const MARKS: usize = 16;
    let t = born.elapsed().as_secs_f32() * 1000.;
    let accent = crate::ui::theme::picked_accent();
    let (color, glow) = if crate::ui::theme::is_light() {
        (accent, fx::mix(accent, 0.45))
    } else {
        // `color-mix(in srgb, accent 70%, white)`.
        let rgb = accent.to_rgb();
        let tint = |v: f32| v * 0.7 + 0.3;
        let mixed = gpui::Rgba {
            r: tint(rgb.r),
            g: tint(rgb.g),
            b: tint(rgb.b),
            a: 1.,
        };
        (mixed.into(), fx::mix(accent, 0.75))
    };
    let bubble = Bubble {
        bounds: away,
        corners: Corners::all(px(BOX_RADIUS)),
    };
    Halo {
        delay: 60.,
        duration: 1100.,
        peak: 0.3,
        blur: 28.,
        spread: 4.,
    }
    .paint(&bubble, t, glow, fx::mix(glow, 0.45), window);
    window.with_content_mask(Some(gpui::ContentMask { bounds: away }), |window| {
        fx::paint_sweep(
            &bubble,
            fx::progress(t, 140., 900.),
            &[
                (0.3, fx::mix(color, 0.)),
                (0.5, fx::mix(color, 0.3)),
                (0.7, fx::mix(color, 0.)),
            ],
            window,
        );
    });
    let mut rng = seed | 1;
    let mut next = move || {
        rng ^= rng >> 12;
        rng ^= rng << 25;
        rng ^= rng >> 27;
        (rng.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32 / (1u64 << 24) as f32
    };
    let (x0, y0) = (f32::from(away.origin.x), f32::from(away.origin.y));
    let (w, h) = (f32::from(away.size.width), f32::from(away.size.height));
    let mut font = window.text_style().font();
    font.weight = gpui::FontWeight::BOLD;
    for i in 0..MARKS {
        let glyph = i % 4 != 3;
        let x = (((i as f32 + next()) / MARKS as f32) * 100.).clamp(5., 95.) / 100. * w;
        let size = if glyph {
            11. + next() * 9.
        } else {
            4. + next() * 3.
        };
        let delay = 120. + next() * 520.;
        let duration = 900. + next() * 500.;
        let drift = (next() - 0.5) * 36.;
        let rise = 110. + next() * 130.;
        let p = fx::progress(t, delay, duration);
        let lift = fx::ease(p, (0.2, 0.7, 0.3, 1.));
        let opacity = fx::frames(
            p,
            &[(0., 0.), (0.15, 1.), (0.7, 0.9), (1., 0.)],
            fx::EASE_IN_OUT,
        );
        if opacity <= 0.01 {
            continue;
        }
        let scale = fx::frames(
            p,
            &[(0., 0.3), (0.15, 1.15), (0.45, 0.9), (0.7, 1.), (1., 0.6)],
            fx::EASE_IN_OUT,
        );
        // `.btw-burst-mark` sits 45% up the box; its glyph stands on it and
        // scales about a point 80% down the glyph.
        let (ax, ay) = (x0 + x + drift * lift, y0 + h * 0.55 - rise * lift);
        let pivot = ay - size * 0.2;
        let center = point(px(ax), px(pivot - size * 0.3 * scale));
        if glyph {
            let font_size = size * scale;
            fx::paint_glow(center, font_size * 0.6, 6., fx::mix(glow, opacity), window);
            let line = window.text_system().shape_line(
                "?".into(),
                px(font_size),
                &[gpui::TextRun {
                    len: 1,
                    font: font.clone(),
                    color: fx::mix(color, opacity),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            );
            let origin = center - point(line.width * 0.5, px(font_size * 0.5));
            let _ = line.paint(
                origin,
                px(font_size),
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            );
        } else {
            let diameter = size * scale;
            fx::paint_glow(center, diameter, 6., fx::mix(glow, opacity), window);
            fx::paint_orb(
                center,
                diameter,
                (fx::mix(gpui::white(), opacity), fx::mix(color, opacity)),
                window,
            );
        }
    }
}
