use super::*;

/// A radial dial of dithered blocks.
pub(in crate::chat::visuals) fn gauge(
    label: String,
    value: f64,
    min: f64,
    max: f64,
    unit: Option<String>,
    token: ThemeColor,
    motion: Motion,
) -> AnyElement {
    const SEGMENTS: usize = 36;
    const SPAN: f32 = 270.;
    const START: f32 = -135.;
    let ratio = ((value - min) / (max - min)).clamp(0., 1.) as f32;
    let format = move |value: f64| match unit.as_deref() {
        Some("%") => format!("{}%", compact(value, 1.)),
        Some(unit) => format!("{} {unit}", compact(value, 0.1)),
        None => compact(value, 0.1),
    };
    let size = 220f32;
    let outer = size / 2. - 1. - CELL;
    let height = size / 2. + (outer + CELL) * std::f32::consts::FRAC_1_SQRT_2 + 18.;
    canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            let tint = color(&token);
            let size = size.min(bounds.size.width.as_f32());
            let left = ((bounds.size.width.as_f32() - size) / 2.).round();
            let grid = Grid::new(bounds.origin + point(px(left), px(0.)), size, height);
            let center = (size / 2., size / 2.);
            let outer = size / 2. - 1. - CELL;
            let thickness = ((size * 0.12) / CELL).round().max(2.) * CELL;
            let inner = outer - thickness;
            let step = SPAN / SEGMENTS as f32;
            let middle = outer - thickness / 2.;
            let gap = (2. / middle).to_degrees();
            // The blocks fill in one by one as the reading counts up.
            let grow = motion.at(0., 1000., window, cx);
            let filled = ratio * grow * SEGMENTS as f32;
            let reading = format(min + (value - min) * grow as f64);
            let active = (filled - 1e-6).ceil().max(0.) as usize;
            let track = faint(tint, 0.14);
            grid.paint(window, |col, row| {
                let (x, y) = Grid::center(col, row);
                let (dx, dy) = (x - center.0, y - center.1);
                let r = (dx * dx + dy * dy).sqrt();
                if r < inner || r > outer + CELL {
                    return None;
                }
                let angle = dx.atan2(-dy).to_degrees();
                let along = angle - START;
                if !(0. ..SPAN).contains(&along) {
                    return None;
                }
                let index = (along / step) as usize;
                let within = along - index as f32 * step;
                if within < gap / 2. || within > step - gap / 2. {
                    return None;
                }
                let head = active > 0 && index == active - 1;
                // The block at the value steps out a cell, like a needle.
                if r > outer && !head {
                    return None;
                }
                if index >= active {
                    return Some(track);
                }
                if head {
                    let part = (filled - index as f32).clamp(0., 1.);
                    if within > gap / 2. + (step - gap) * part.max(0.15) {
                        return (r <= outer).then_some(track);
                    }
                    return Some(tint);
                }
                let tone = (1. - (active - 1 - index) as f32 / 16.).max(9. / 16.);
                inked(col, row, tone).then_some(tint)
            });
            let at = |dx: f32, dy: f32| grid.origin + point(px(center.0 + dx), px(center.1 + dy));
            Label::sans((size * 0.15).round(), content(1.), FontWeight::SEMIBOLD).paint(
                &reading,
                at(0., -4.),
                Align::Center,
                window,
                cx,
            );
            Label::sans(12., content(0.6), FontWeight::NORMAL).paint(
                &label,
                at(0., size * 0.1 + 6.),
                Align::Center,
                window,
                cx,
            );
            let ends = middle * std::f32::consts::FRAC_1_SQRT_2;
            for (dx, value) in [(-ends, min), (ends, max)] {
                axis().paint(
                    &compact(value, 1.),
                    at(dx, ends + thickness / 2. + 12.),
                    Align::Center,
                    window,
                    cx,
                );
            }
        },
    )
    .w_full()
    .h(px(height))
    .into_any_element()
}

/// Parts of a whole as a dithered ring with a legend.
pub(in crate::chat::visuals) fn donut(
    slices: Vec<Slice>,
    label: Option<String>,
    value: Option<String>,
    motion: Motion,
    id: SharedString,
) -> AnyElement {
    const SIZE: f32 = 184.;
    const FLOOR: f32 = 3. / 16.;
    let total: f64 = slices.iter().map(|s| s.value.max(0.)).sum();
    let total_reading = value.unwrap_or_else(|| {
        let s = format!("{total:.0}");
        if total.abs() >= 1e4 {
            // 106000 → 106,000: the hole has room for the full figure.
            let digits: Vec<char> = s.chars().collect();
            let mut out = String::new();
            for (i, c) in digits.iter().enumerate() {
                if i > 0 && (digits.len() - i).is_multiple_of(3) && c.is_ascii_digit() {
                    out.push(',');
                }
                out.push(*c);
            }
            out
        } else {
            compact(total, 0.1)
        }
    });
    let legend = div()
        .flex_1()
        .min_w(px(140.))
        .max_w(px(320.))
        .flex()
        .flex_col()
        .gap_2()
        .children(slices.iter().map(|s| {
            let share = if total > 0. {
                s.value / total * 100.
            } else {
                0.
            };
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(12.))
                .child(swatch(color(&s.color)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(content(0.8))
                        .child(s.label.clone()),
                )
                .child(
                    div()
                        .font_family(crate::ui::fonts::mono())
                        .text_color(content(0.5))
                        .child(format!("{} · {share:.0}%", compact(s.value, 0.1))),
                )
        }));
    let ring = canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            let grid = Grid::new(bounds.origin, SIZE, SIZE);
            let half = (grid.cols as f32 * CELL) / 2.;
            let outer = half - 2. * CELL;
            let inner = outer - ((SIZE * 0.17) / CELL).round() * CELL;
            // The ring sweeps round from twelve o'clock.
            let sweep = motion.at(0., 900., window, cx) * std::f32::consts::TAU;
            let angle_of = |dx: f32, dy: f32| {
                let a = dx.atan2(-dy);
                if a < 0. { a + std::f32::consts::TAU } else { a }
            };
            let mut edges = Vec::new();
            let mut running = 0.;
            for s in &slices {
                edges.push((running / total.max(1e-9)) as f32 * std::f32::consts::TAU);
                running += s.value.max(0.);
            }
            let shown = slices.iter().filter(|s| s.value > 0.).count();
            // The hovered slice stays bright and the others dim.
            let hovered = pointer(bounds, window).and_then(|(x, y)| {
                let (dx, dy) = (x - half, y - half);
                let r = (dx * dx + dy * dy).sqrt();
                (r >= inner - CELL && r <= outer + CELL)
                    .then(|| edges.iter().rposition(|e| angle_of(dx, dy) >= *e))
                    .flatten()
            });
            let colors: Vec<Hsla> = slices
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let c = color(&s.color);
                    if hovered.is_some_and(|h| h != i) {
                        faint(c, 0.25)
                    } else {
                        c
                    }
                })
                .collect();
            grid.paint(window, |col, row| {
                let (x, y) = Grid::center(col, row);
                let (dx, dy) = (x - half, y - half);
                let r = (dx * dx + dy * dy).sqrt();
                if r < inner || r > outer {
                    return None;
                }
                let angle = angle_of(dx, dy);
                if angle > sweep {
                    return None;
                }
                let index = edges.iter().rposition(|e| angle >= *e)?;
                let mut outline = r > outer - CELL || r < inner + CELL;
                if shown > 1 {
                    // Distance in cells from the nearest slice edge.
                    let near = edges
                        .iter()
                        .zip(&slices)
                        .filter(|(_, s)| s.value > 0.)
                        .map(|(e, _)| {
                            let d = (angle - e).abs();
                            let d = d.min(std::f32::consts::TAU - d);
                            if d > std::f32::consts::FRAC_PI_2 {
                                f32::MAX
                            } else {
                                r * d.sin() / CELL
                            }
                        })
                        .fold(f32::MAX, f32::min);
                    if near < 0.75 {
                        return None;
                    }
                    outline |= near < 1.75;
                }
                let depth = (outer - r) / (outer - inner);
                let tone = FLOOR + (1. - FLOOR) * (1. - depth).powf(1.4);
                (outline || inked(col, row, tone)).then_some(colors[index])
            });
            let center = grid.origin + point(px(half), px(half));
            // The middle names the hovered slice, else the whole.
            let (caption, figure, share) = match hovered.and_then(|h| slices.get(h)) {
                Some(s) => (
                    Some(s.label.clone()),
                    reading(s.value),
                    Some(format!("{:.0}%", s.value / total.max(1e-9) * 100.)),
                ),
                None => (label.clone(), total_reading.clone(), None),
            };
            if let Some(caption) = &caption {
                let caption: String = caption.to_uppercase().chars().take(18).collect();
                axis().paint(
                    &caption,
                    center - point(px(0.), px(16.)),
                    Align::Center,
                    window,
                    cx,
                );
            }
            let fit = ((inner * 2. - 16.) / (figure.chars().count().max(1) as f32 * 0.62)).min(26.);
            Label::sans(fit.floor(), content(1.), FontWeight::SEMIBOLD).paint(
                &figure,
                center + point(px(0.), px(if caption.is_some() { 4. } else { 0. })),
                Align::Center,
                window,
                cx,
            );
            if let Some(share) = share {
                axis().paint(
                    &share,
                    center + point(px(0.), px(24.)),
                    Align::Center,
                    window,
                    cx,
                );
            }
        },
    )
    .size_full();
    div()
        .w_full()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_6()
        .child(hoverable(id).size(px(SIZE)).flex_shrink_0().child(ring))
        .child(legend)
        .into_any_element()
}

/// A dithered progress bar with a solid head.
pub(in crate::chat::visuals) fn progress(
    label: String,
    value: f64,
    max: f64,
    detail: Option<String>,
    token: ThemeColor,
    motion: Motion,
) -> AnyElement {
    let ratio = (value / max).clamp(0., 1.) as f32;
    let bar = canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            let tint = color(&token);
            let track = faint(tint, 0.14);
            let grid = Grid::new(bounds.origin, bounds.size.width.as_f32(), 3. * CELL);
            let grow = motion.at(0., 800., window, cx);
            let head = (grid.cols as f32 * ratio * grow).round() as i32;
            if head > 0 {
                glow(
                    window,
                    Bounds::new(
                        grid.origin,
                        gpui::size(px(head as f32 * CELL), px(3. * CELL)),
                    ),
                    tint,
                    0.3,
                );
            }
            grid.paint(window, |col, row| {
                if col >= head {
                    return Some(track);
                }
                // A comet: sparse at the start, solid at the head.
                let tone = 0.3 + 0.7 * (col as f32 / head.max(1) as f32).powf(1.5);
                Some(if col >= head - 3 || inked(col, row, tone) {
                    tint
                } else {
                    track
                })
            });
        },
    )
    .w_full()
    .h(px(3. * CELL));
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_baseline()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(13.))
                        .child(label),
                )
                .child(
                    div()
                        .font_family(crate::ui::fonts::mono())
                        .text_size(px(12.))
                        .text_color(content(0.55))
                        .child(format!("{:.0}%", ratio * 100.)),
                ),
        )
        .child(bar)
        .children(detail.map(|d| div().text_size(px(12.)).text_color(content(0.55)).child(d)))
        .into_any_element()
}
