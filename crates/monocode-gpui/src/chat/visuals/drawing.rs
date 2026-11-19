use super::*;

impl VisualView {
    pub(super) fn scene(
        &mut self,
        key: String,
        height: f32,
        objects: Vec<SceneObject>,
        camera: Camera,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let orbit = *self.orbits.entry(key.clone()).or_default();
        let wheel_key = key.clone();
        let reset_key = key.clone();
        let keyboard_key = key.clone();
        let reset_keyboard = key.clone();
        let cache = self.scene_cache.entry(key.clone()).or_default().clone();
        let revision = self.revision;
        div()
            .flex()
            .flex_col()
            .gap_1()
            .w_full()
            .child(
                div()
                    .id(SharedString::from(format!("visual-scene-{key}")))
                    .debug_selector({
                        let key = key.clone();
                        move || format!("visual-scene-{key}")
                    })
                    .w_full()
                    .focusable()
                    .tab_stop(true)
                    .aria_label(
                        "3D scene; arrow keys orbit, plus and minus zoom, zero resets camera",
                    )
                    .h(px(height))
                    .overflow_hidden()
                    .rounded_lg()
                    .border_1()
                    .border_color(content(0.08))
                    .cursor(gpui::CursorStyle::OpenHand)
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, _, window, _| {
                                let dimensions =
                                    [bounds.size.width.as_f32(), bounds.size.height.as_f32()];
                                let mut cached = cache.borrow_mut();
                                if cached.as_ref().is_none_or(|c| {
                                    c.revision != revision
                                        || c.dimensions != dimensions
                                        || c.orbit != orbit
                                }) {
                                    *cached = Some(SceneCache {
                                        revision,
                                        dimensions,
                                        orbit,
                                        projection: Rc::new(scene::project(
                                            &objects,
                                            &camera,
                                            orbit,
                                            dimensions[0],
                                            dimensions[1],
                                        )),
                                    });
                                }
                                let projection = cached.as_ref().unwrap().projection.clone();
                                drop(cached);
                                window.with_content_mask(
                                    Some(gpui::ContentMask { bounds }),
                                    |window| {
                                        for surface in &projection.order {
                                            match surface {
                                                scene::Surface::Face(index) => {
                                                    let face = &projection.faces[*index];
                                                    let mut fill = color(&face.color);
                                                    fill.l *= face.light;
                                                    paint_path(
                                                        &face.points,
                                                        bounds.origin,
                                                        1.,
                                                        0.,
                                                        true,
                                                        fill,
                                                        window,
                                                    );
                                                }
                                                scene::Surface::Stroke(index) => {
                                                    let stroke = &projection.strokes[*index];
                                                    paint_path(
                                                        &stroke.points,
                                                        bounds.origin,
                                                        1.,
                                                        stroke.width,
                                                        false,
                                                        color(&stroke.color),
                                                        window,
                                                    );
                                                }
                                            }
                                        }
                                    },
                                );
                            },
                        )
                        .size_full(),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                            this.scene_drag = Some((key.clone(), event.position));
                            cx.stop_propagation();
                        }),
                    )
                    .on_scroll_wheel(cx.listener(
                        move |this, event: &gpui::ScrollWheelEvent, _, cx| {
                            // Modifier zoom leaves ordinary transcript scrolling available.
                            if event.modifiers.control || event.modifiers.platform {
                                let orbit = this.orbits.entry(wheel_key.clone()).or_default();
                                orbit.zoom = (orbit.zoom
                                    * (event.delta.pixel_delta(px(16.)).y.as_f32() * 0.008).exp())
                                .clamp(0.15, 8.);
                                cx.notify();
                                cx.stop_propagation();
                            }
                        },
                    ))
                    .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                        let orbit = this.orbits.entry(keyboard_key.clone()).or_default();
                        match event.keystroke.key.as_str() {
                            "left" => orbit.yaw -= 0.12,
                            "right" => orbit.yaw += 0.12,
                            "up" => orbit.pitch = (orbit.pitch - 0.12).max(-1.45),
                            "down" => orbit.pitch = (orbit.pitch + 0.12).min(1.45),
                            "+" | "=" => orbit.zoom = (orbit.zoom / 1.2).max(0.15),
                            "-" => orbit.zoom = (orbit.zoom * 1.2).min(8.),
                            "0" => *orbit = scene::Orbit::default(),
                            _ => return,
                        }
                        cx.notify();
                        cx.stop_propagation();
                    })),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(11.))
                    .text_color(content(0.45))
                    .child(
                        div()
                            .flex_1()
                            .child("Drag to orbit · ⌘/Ctrl + scroll to zoom"),
                    )
                    .child(
                        button(SharedString::from(format!(
                            "visual-camera-reset-{reset_key}"
                        )))
                        .aria_label("Reset scene camera")
                        .child("Reset camera")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.orbits.remove(&reset_key);
                            cx.notify();
                        }))
                        .on_key_down(cx.listener(
                            move |this, event, _, cx| {
                                if activated(event) {
                                    this.orbits.remove(&reset_keyboard);
                                    cx.notify();
                                    cx.stop_propagation();
                                }
                            },
                        )),
                    ),
            )
            .into_any_element()
    }
}

pub(super) fn paint_label(
    text: String,
    at: Point<Pixels>,
    token: &ThemeColor,
    font_size: f32,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    let style = window.text_style();
    let run = gpui::TextRun {
        len: text.len(),
        font: style.font(),
        color: color(token),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(text.into(), px(font_size), &[run], None);
    // A label's y is its baseline, as in SVG text and canvas fillText. A line
    // box of ascent + descent has no padding, so its top is one ascent above
    // the baseline.
    let _ = line.paint(
        at - point(px(0.), line.ascent),
        line.ascent + line.descent,
        gpui::TextAlign::Left,
        None,
        window,
        cx,
    );
}

pub(super) fn drawing(width: f32, height: f32, shapes: Vec<Shape>) -> AnyElement {
    div()
        .w_full()
        .aspect_ratio(width / height)
        .max_h(px(height))
        .overflow_hidden()
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let scale = (bounds.size.width.as_f32() / width)
                        .min(bounds.size.height.as_f32() / height);
                    let origin = bounds.origin
                        + point(
                            px((bounds.size.width.as_f32() - width * scale) / 2.),
                            px(0.),
                        );
                    window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                        for shape in &shapes {
                            match shape {
                                Shape::Rect {
                                    x,
                                    y,
                                    width,
                                    height,
                                    color: token,
                                } => window.paint_quad(fill(
                                    Bounds::new(
                                        origin + point(px(x * scale), px(y * scale)),
                                        size(px(width * scale), px(height * scale)),
                                    ),
                                    color(token),
                                )),
                                Shape::Circle {
                                    x,
                                    y,
                                    radius,
                                    color: token,
                                } => {
                                    let points = (0..48)
                                        .map(|i| {
                                            let a = i as f32 * std::f32::consts::TAU / 48.;
                                            [x + radius * a.cos(), y + radius * a.sin()]
                                        })
                                        .collect::<Vec<_>>();
                                    paint_path(
                                        &points,
                                        origin,
                                        scale,
                                        0.,
                                        true,
                                        color(token),
                                        window,
                                    );
                                }
                                Shape::Line {
                                    x1,
                                    y1,
                                    x2,
                                    y2,
                                    width,
                                    color: token,
                                } => paint_path(
                                    &[[*x1, *y1], [*x2, *y2]],
                                    origin,
                                    scale,
                                    width * scale,
                                    false,
                                    color(token),
                                    window,
                                ),
                                Shape::Polyline {
                                    points,
                                    width,
                                    color: token,
                                } => paint_path(
                                    points,
                                    origin,
                                    scale,
                                    width * scale,
                                    false,
                                    color(token),
                                    window,
                                ),
                                Shape::Label {
                                    x,
                                    y,
                                    text,
                                    color: token,
                                } => paint_label(
                                    text.clone(),
                                    origin + point(px(x * scale), px(y * scale)),
                                    token,
                                    12. * scale.max(0.7),
                                    window,
                                    cx,
                                ),
                            }
                        }
                    });
                },
            )
            .size_full(),
        )
        .into_any_element()
}
