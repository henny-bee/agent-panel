use super::*;

/// `placePopover`: prefer `options.side`, flip when it can't hold the
/// popover and the other side has more room, then clamp into the viewport.
pub(crate) fn place(
    anchor: Rect,
    popover: (f32, f32),
    viewport: (f32, f32),
    o: &Options,
) -> Placement {
    let (vw, vh) = viewport;
    let max = o.max_height.unwrap_or(f32::INFINITY);
    let width = o
        .width
        .unwrap_or(popover.0)
        .min((vw - o.padding * 2.).max(0.));
    let wanted = max.min(popover.1.max(o.min_height.unwrap_or(0.)));
    let needed = if o.side.vertical() { wanted } else { width };
    let room = room_on(o.side, &anchor, viewport, o.gap, o.padding);
    let flipped = room_on(o.side.opposite(), &anchor, viewport, o.gap, o.padding);
    let side = if room >= needed || flipped <= room {
        o.side
    } else {
        o.side.opposite()
    };

    if side.vertical() {
        let left = clamp(
            cross_start(o.align, anchor.left, anchor.right, width),
            o.padding,
            vw - width - o.padding,
        );
        // A minimum height can outgrow the room beside the anchor, but never
        // the viewport: past that the overflow would be unreachable.
        let available = room_on(side, &anchor, viewport, o.gap, o.padding)
            .max(o.min_height.unwrap_or(0.))
            .min((vh - o.padding * 2.).max(0.));
        let max_height = max.min(available);
        if side == Side::Bottom {
            let top = (anchor.bottom + o.gap).min(vh - o.padding - max_height);
            return Placement {
                side,
                left,
                top: Some(top.max(o.padding)),
                bottom: None,
                width,
                max_height,
            };
        }
        let bottom = (vh - anchor.top + o.gap).min(vh - o.padding - max_height);
        return Placement {
            side,
            left,
            top: None,
            bottom: Some(bottom.max(o.padding)),
            width,
            max_height,
        };
    }

    let capped = max.min((vh - o.padding * 2.).max(0.));
    let height = popover.1.min(capped);
    let top = clamp(
        cross_start(o.align, anchor.top, anchor.bottom, height),
        o.padding,
        vh - height - o.padding,
    );
    let raw_left = if side == Side::Right {
        anchor.right + o.gap
    } else {
        anchor.left - o.gap - width
    };
    Placement {
        side,
        left: clamp(raw_left, o.padding, vw - width - o.padding),
        top: Some(top),
        bottom: None,
        width,
        max_height: capped.min(vh - top - o.padding),
    }
}

pub(super) struct Placed {
    pub(super) anchor: Anchor,
    pub(super) options: Options,
    pub(super) progress: f32,
    pub(super) frame: AnyElement,
    pub(super) track: Vec<AnchorSlot>,
    pub(super) ignore: Vec<AnchorSlot>,
    pub(super) on_dismiss: Option<Dismiss>,
    pub(super) consume_outside_press: bool,
}

impl IntoElement for Placed {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for Placed {
    type RequestLayoutState = LayoutId;
    type PrepaintState = Bounds<Pixels>;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, LayoutId) {
        let child = self.frame.request_layout(window, cx);
        let style = Style {
            position: CssPosition::Absolute,
            display: Display::Flex,
            ..Style::default()
        };
        (window.request_layout(style, [child], cx), child)
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        child: &mut LayoutId,
        window: &mut Window,
        cx: &mut App,
    ) -> Bounds<Pixels> {
        let surface = window.layout_bounds(*child).size;
        let viewport = window.viewport_size();
        let viewport = (f32::from(viewport.width), f32::from(viewport.height));
        let height = f32::from(surface.height);
        let placed = place(
            Rect::from_bounds(self.anchor.bounds()),
            (f32::from(surface.width), height),
            viewport,
            &self.options,
        );
        let top = placed.top.unwrap_or_else(|| {
            viewport.1 - placed.bottom.unwrap_or(0.) - height.min(placed.max_height)
        });
        let target = if matches!(self.anchor, Anchor::Center) {
            point(
                px(((viewport.0 - f32::from(surface.width)) / 2.).max(0.)).round(),
                px(((viewport.1 - height) / 2.).max(0.)).round(),
            )
        } else {
            point(px(placed.left).round(), px(top).round())
        };
        let moving = target + entrance_offset(placed.side, self.progress);
        // One offset moves the glass, frame, content, clipping and hitboxes.
        window.with_element_offset(moving - bounds.origin, |window| {
            self.frame.prepaint(window, cx)
        });
        // Dismissal and child menus follow the visible card during its entrance.
        let placed = Bounds::new(moving, surface);
        for slot in &self.track {
            slot.set(Some(placed));
        }
        placed
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut LayoutId,
        surface: &mut Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.frame.paint(window, cx);
        let Some(dismiss) = self.on_dismiss.clone() else {
            return;
        };
        let surface = *surface;
        let anchor = self.anchor.bounds();
        let ignore = self.ignore.clone();
        let consume = self.consume_outside_press;
        let outside = move |position: &Point<Pixels>| {
            !surface.contains(position)
                && !anchor.contains(position)
                && !ignore
                    .iter()
                    .any(|slot| slot.get().is_some_and(|b| b.contains(position)))
        };
        // Capture, so a trigger beneath that stops propagation still closes
        // the popover, and before that trigger reacts to the same press.
        // Then consume that press in bubble so the
        // content behind cannot act on it too; the next press reaches it.
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if !outside(&event.position) {
                return;
            }
            match phase {
                DispatchPhase::Capture => dismiss(window, cx),
                DispatchPhase::Bubble if consume => cx.stop_propagation(),
                DispatchPhase::Bubble => {}
            }
        });
    }
}
