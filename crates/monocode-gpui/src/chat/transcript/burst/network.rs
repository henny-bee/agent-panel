use super::*;

pub(super) fn stroke_line(
    from: Point<Pixels>,
    to: Point<Pixels>,
    width: f32,
    color: Hsla,
    window: &mut Window,
) {
    let mut path = PathBuilder::stroke(px(width));
    path.move_to(from);
    path.line_to(to);
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

/// orchestrator-hub / orchestrator-node with its expanding ::after ring.
pub(super) fn paint_node(
    center: Point<Pixels>,
    diameter: f32,
    (t, delay, ring_delay, ring_ms): (f32, f32, f32, f32),
    fade: f32,
    colors: &Palette,
    window: &mut Window,
) {
    let p = progress(t, delay, 600.);
    let opacity = frames(p, &[(0., 0.), (0.3, 1.), (1., 1.)], EASE_OUT) * fade;
    let peak = if diameter >= 8. { 1.35 } else { 1.4 };
    let low = if diameter >= 8. { 0.3 } else { 0.2 };
    let scale = frames(p, &[(0., low), (0.3, peak), (1., 1.)], EASE_OUT);
    let ring = progress(t, ring_delay, ring_ms);
    let ring_opacity = frames(ring, &[(0., 0.8), (1., 0.)], EASE_OUT) * opacity;
    let ring_scale = scale * frames(ring, &[(0., 1.), (1., 3.5)], EASE_OUT);
    if opacity <= 0.01 {
        return;
    }
    paint_glow(
        center,
        diameter * scale,
        8.,
        mix(colors.glow, opacity),
        window,
    );
    paint_orb(
        center,
        diameter * scale,
        (mix(colors.spark, opacity), mix(colors.color, opacity)),
        window,
    );
    if ring_opacity > 0.01 {
        let d = diameter * ring_scale;
        window.paint_quad(
            fill(
                Bounds::new(center - point(px(d * 0.5), px(d * 0.5)), size(px(d), px(d))),
                gpui::transparent_black(),
            )
            .corner_radii(px(d * 0.5))
            .border_widths(px(1.5 * ring_scale))
            .border_color(mix(colors.color, ring_opacity)),
        );
    }
}

/// OrchestratorConstellation: the lead lights up at the bubble's base, edges
/// race out to agents that pop in and hand off again, then the whole network
/// rises beyond the top edge.
pub(super) fn paint_network(
    bubble: &Bubble,
    t: f32,
    colors: &Palette,
    rng: &mut Rng,
    window: &mut Window,
) {
    let (w, h) = (bubble.width(), bubble.height());
    let (bx, by) = bubble.origin();
    let (hub, nodes) = make_constellation(w, h, || rng.next());
    // orchestrator-network-rise
    let p = progress(t, 300., 2200.);
    let fade = frames(p, &[(0., 1.), (0.85, 1.), (1., 0.)], RISE);
    let lift = -(h + 12.) * ease(p, RISE);
    if fade <= 0.01 {
        return;
    }
    let at = |(x, y): (f32, f32)| point(px(bx + x), px(by + y + lift));
    for node in &nodes {
        let length = (node.x - node.from.0).hypot(node.y - node.from.1);
        if length <= 0. {
            continue;
        }
        let along = |s: f32| {
            let k = s / length;
            at((
                node.from.0 + (node.x - node.from.0) * k,
                node.from.1 + (node.y - node.from.1) * k,
            ))
        };
        let q = progress(t, node.delay, node.draw);
        let drawn = length * ease(q, RISE);
        if drawn > 0.01 {
            stroke_line(
                at(node.from),
                along(drawn),
                1.25,
                mix(colors.color, 0.6 * fade),
                window,
            );
        }
        // A bright head rides the tip of each edge as it draws out.
        let offset = frames(q, &[(0., 6.), (1., -length)], RISE);
        let head = frames(q, &[(0., 1.), (0.85, 1.), (1., 0.)], RISE) * fade;
        let (start, end) = ((-offset).max(0.), (6. - offset).min(length));
        if head > 0.01 && end > start && q > 0. {
            stroke_line(
                along(start),
                along(end),
                5.,
                mix(colors.glow, 0.35 * head),
                window,
            );
            stroke_line(
                along(start),
                along(end),
                2.5,
                mix(colors.spark, head),
                window,
            );
        }
    }
    paint_node(at(hub), 8., (t, 120., 220., 700.), fade, colors, window);
    for node in &nodes {
        let delay = node.delay + node.draw - 40.;
        let diameter = if node.child { 4.5 } else { 6. };
        paint_node(
            at((node.x, node.y)),
            diameter,
            (t, delay, delay, 500.),
            fade,
            colors,
            window,
        );
    }
}
