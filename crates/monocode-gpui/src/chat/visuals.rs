//! Session visuals are a bounded native component tree, with no browser surface.
use crate::ui::theme::{accent_color, content};
use gpui::{
    AnyElement, Bounds, Context, Div, EventEmitter, FontWeight, Hsla, IntoElement, MouseButton,
    PathBuilder, Pixels, Point, Render, Role, SharedString, Stateful, Window, canvas, div, fill,
    point, prelude::*, px, relative, size,
};
use monocode_core::session::visual::{
    Camera, Node, SceneObject, Shape, TextSize, ThemeColor, VisualDocument, VisualRuntime,
};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
};

mod charts;
pub(crate) mod dither;
mod node;
mod scene;
use drawing::drawing;
pub(crate) mod drawing;
#[cfg(test)]
mod tests;

/// The transcript must remeasure this row when its content changes.
pub(crate) struct VisualChanged;
pub(crate) struct VisualView {
    document: VisualDocument,
    runtime: Option<VisualRuntime>,
    tree: Option<Node>,
    error: Option<String>,
    width: Rc<Cell<f32>>,
    tracks: HashMap<String, Rc<Cell<Option<Bounds<Pixels>>>>>,
    slider_drag: Option<SliderDrag>,
    orbits: HashMap<String, scene::Orbit>,
    scene_drag: Option<(String, Point<Pixels>)>,
    inputs: HashMap<String, InputSlot>,
    active: HashSet<String>,
    revision: u64,
    scene_cache: HashMap<String, Rc<RefCell<Option<SceneCache>>>>,
    /// Placed in an answer's prose: no title bar, its caption underneath.
    inline: Option<String>,
    /// When the visual first appeared, for its charts' entrances.
    motion: dither::Motion,
}
impl EventEmitter<VisualChanged> for VisualView {}
struct SliderDrag {
    key: String,
    handler: String,
    min: f64,
    max: f64,
    step: f64,
}
struct InputSlot {
    input: gpui::Entity<crate::ui::input::Input>,
    handler: String,
    source_value: String,
    _subscription: gpui::Subscription,
}
struct SceneCache {
    revision: u64,
    dimensions: [f32; 2],
    orbit: scene::Orbit,
    projection: Rc<scene::Projection>,
}

fn color(token: &ThemeColor) -> Hsla {
    // Semantic hues need darker variants when the interface uses dark ink.
    let light_surface = content(1.).l < 0.5;
    let hue = |dark, light| gpui::rgb(if light_surface { light } else { dark }).into();
    // Series 1 is the user's accent. A later series of the same hue would be
    // indistinguishable, so it takes the first spare hue that differs.
    let series = |dark, light| {
        let accent = accent_color();
        let near = |c: Hsla| {
            let d = (c.h - accent.h).abs();
            accent.s > 0.25 && d.min(1. - d) < 30. / 360.
        };
        let own: Hsla = hue(dark, light);
        if !near(own) {
            return own;
        }
        [
            (0x2dd4bf, 0x0d7f73),
            (0xe2c541, 0x8a6d00),
            (0xfc5855, 0xd01d21),
        ]
        .into_iter()
        .map(|(dark, light)| hue(dark, light))
        .find(|c| !near(*c))
        .unwrap_or(own)
    };
    match token {
        ThemeColor::Ink => content(1.),
        ThemeColor::Muted => content(0.6),
        ThemeColor::Faint => content(0.2),
        ThemeColor::Accent | ThemeColor::Series1 => accent_color(),
        // Series hues, tuned separately for dark and light surfaces.
        // Semantic colours keep their meaning even when close to the accent.
        ThemeColor::Positive => hue(0x4fd57f, 0x0a924b),
        ThemeColor::Negative => hue(0xfc5855, 0xd01d21),
        ThemeColor::Warning => hue(0xfea247, 0xd96700),
        ThemeColor::Series2 => series(0x4fd57f, 0x0a924b),
        ThemeColor::Series3 => series(0xb77df8, 0x863ad2),
        ThemeColor::Series4 => series(0xfea247, 0xd96700),
        ThemeColor::Series5 => series(0xfa78b2, 0xd12c80),
        ThemeColor::Series6 => series(0x3faff3, 0x0077d7),
    }
}
fn control(id: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id.into())
        .role(Role::Button)
        .focusable()
        .tab_stop(true)
        .h(px(26.))
        .px(px(9.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .text_size(px(12.))
        .line_height(px(16.))
        .cursor_pointer()
}
fn button(id: impl Into<SharedString>) -> Stateful<Div> {
    control(id)
        .border_1()
        .border_color(content(0.12))
        .text_color(content(0.8))
        .hover(|s| s.bg(content(0.06)).text_color(content(1.)))
        .focus(|s| s.border_color(accent_color()))
}
fn tab_button(id: impl Into<SharedString>, selected: bool) -> Stateful<Div> {
    let light = content(1.).l < 0.5;
    let selected_fill: Hsla = gpui::rgb(if light { 0x000000 } else { 0xffffff }).into();
    let selected_text: Hsla = gpui::rgb(if light { 0xffffff } else { 0x141414 }).into();
    control(id)
        .text_color(if selected {
            selected_text
        } else {
            content(0.6)
        })
        .when(selected, |d| d.bg(selected_fill))
        .hover(move |s| {
            if selected {
                s.bg(selected_fill).text_color(selected_text)
            } else {
                s.bg(content(0.06)).text_color(content(1.))
            }
        })
        .focus_visible(move |s| {
            s.bg(if selected {
                selected_fill
            } else {
                content(0.08)
            })
        })
}

fn activated(event: &gpui::KeyDownEvent) -> bool {
    matches!(event.keystroke.key.as_str(), "enter" | "space" | " ")
}

impl VisualView {
    pub(crate) fn new(document: VisualDocument, _cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            document,
            runtime: None,
            tree: None,
            error: None,
            width: Rc::new(Cell::new(640.)),
            tracks: HashMap::new(),
            slider_drag: None,
            orbits: HashMap::new(),
            scene_drag: None,
            inputs: HashMap::new(),
            active: HashSet::new(),
            revision: 0,
            scene_cache: HashMap::new(),
            inline: None,
            motion: dither::Motion(std::time::Instant::now()),
        };
        this.initialize();
        this
    }
    /// Draws the visual inside an answer, captioned, or back in its own row.
    pub(crate) fn set_inline(&mut self, caption: Option<String>, cx: &mut Context<Self>) {
        if self.inline != caption {
            self.inline = caption;
            self.changed(cx);
        }
    }
    fn initialize(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.error = None;
        self.tree = None;
        self.runtime = None;
        match self
            .document
            .validate()
            .and_then(|()| VisualRuntime::new(&self.document.source))
            .and_then(|mut runtime| {
                let tree = runtime.render()?;
                Ok((runtime, tree))
            }) {
            Ok((runtime, tree)) => {
                self.runtime = Some(runtime);
                self.tree = Some(tree);
            }
            Err(error) => self.error = Some(error),
        }
    }
    fn changed(&self, cx: &mut Context<Self>) {
        cx.emit(VisualChanged);
        cx.notify();
    }
    fn dispatch(&mut self, handler: &str, value: Value, cx: &mut Context<Self>) {
        let Some(runtime) = self.runtime.as_mut() else {
            return;
        };
        match runtime.dispatch(handler, value) {
            Ok(tree) => {
                self.revision = self.revision.wrapping_add(1);
                // A render refreshes callback capabilities. A pointer drag and a
                // focused input survive that render, so refresh their bindings too.
                if let Some(drag) = self.slider_drag.as_mut() {
                    if let Some(Node::Slider {
                        handler,
                        min,
                        max,
                        step,
                        ..
                    }) = node_at(&tree, &drag.key)
                    {
                        drag.handler = handler.clone();
                        drag.min = *min;
                        drag.max = *max;
                        drag.step = *step;
                    } else {
                        self.slider_drag = None;
                    }
                }
                for (key, slot) in &mut self.inputs {
                    if let Some(Node::Input { handler, .. }) = node_at(&tree, key) {
                        slot.handler = handler.clone();
                    }
                }
                self.tree = Some(tree);
                self.error = None;
            }
            Err(error) => {
                self.error = Some(error);
                self.slider_drag = None;
                self.scene_drag = None;
            }
        }
        self.changed(cx);
    }
    fn slide(&mut self, x: Pixels, cx: &mut Context<Self>) {
        let Some(drag) = &self.slider_drag else {
            return;
        };
        let Some(bounds) = self.tracks.get(&drag.key).and_then(|b| b.get()) else {
            return;
        };
        let fraction = ((x - bounds.left()).as_f32() / bounds.size.width.as_f32().max(1.))
            .clamp(0., 1.) as f64;
        let raw = drag.min + (drag.max - drag.min) * fraction;
        let value = (drag.min + ((raw - drag.min) / drag.step).round() * drag.step)
            .clamp(drag.min, drag.max);
        let handler = drag.handler.clone();
        self.dispatch(&handler, json!(value), cx);
    }
}

impl Render for VisualView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.document.title.clone();
        let tree = self.tree.clone();
        let width = self.width.clone();
        let entity = cx.entity().downgrade();
        self.active.clear();
        let body = if let Some(tree) = tree {
            self.node(&tree, "root".into(), cx)
        } else {
            div().into_any_element()
        };
        self.inputs.retain(|key, _| self.active.contains(key));
        self.tracks.retain(|key, _| self.active.contains(key));
        self.orbits.retain(|key, _| self.active.contains(key));
        self.scene_cache.retain(|key, _| self.active.contains(key));
        let inline = self.inline.clone();
        // A visual in its own row is titled; placed in prose, it is captioned.
        let header = inline.is_none().then(|| {
            div()
                .min_w_0()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title)
        });
        div()
            .id("native-session-visual")
            .debug_selector(|| "native-session-visual".into())
            .relative()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_3()
            .text_size(px(13.))
            .text_color(content(1.))
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let old = (width.get() / 180.).floor();
                        width.set(bounds.size.width.as_f32());
                        if old != (width.get() / 180.).floor() {
                            let _ = entity.update(cx, |this, cx| this.changed(cx));
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .w_full()
                .h(px(1.)),
            )
            .children(header)
            .children(self.error.iter().map(|error| {
                div()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .border_1()
                    .border_color(color(&ThemeColor::Negative))
                    .text_size(px(12.))
                    .text_color(color(&ThemeColor::Negative))
                    .child(format!("Visual could not update: {error}"))
            }))
            .child(
                div()
                    .id("visual-content")
                    .debug_selector(|| "visual-content".into())
                    .w_full()
                    .min_w_0()
                    .flex_shrink_0()
                    .child(body),
            )
            .children(inline.as_ref().filter(|c| !c.is_empty()).map(|caption| {
                div()
                    .text_size(px(12.))
                    .text_color(content(0.5))
                    .child(caption.clone())
            }))
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                if !event.dragging() {
                    this.slider_drag = None;
                    this.scene_drag = None;
                    return;
                }
                if this.slider_drag.is_some() {
                    this.slide(event.position.x, cx);
                    cx.stop_propagation();
                }
                if let Some((key, last)) = &mut this.scene_drag {
                    let delta = event.position - *last;
                    *last = event.position;
                    let orbit = this.orbits.entry(key.clone()).or_default();
                    orbit.yaw += delta.x.as_f32() * 0.01;
                    orbit.pitch = (orbit.pitch + delta.y.as_f32() * 0.01).clamp(-1.45, 1.45);
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.slider_drag = None;
                    this.scene_drag = None;
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.slider_drag = None;
                    this.scene_drag = None;
                }),
            )
    }
}

fn node_at<'a>(mut node: &'a Node, key: &str) -> Option<&'a Node> {
    let mut parts = key.split('/');
    if parts.next() != Some("root") {
        return None;
    }
    for part in parts {
        let index = part.parse::<usize>().ok()?;
        node = match node {
            Node::Column { children, .. }
            | Node::Row { children, .. }
            | Node::Grid { children, .. }
            | Node::Card { children, .. } => children.get(index)?,
            _ => return None,
        };
    }
    Some(node)
}

fn format_number(value: f64) -> String {
    if value.abs() >= 1_000_000. {
        format!("{:.1}m", value / 1_000_000.)
    } else if value.abs() >= 10_000. {
        format!("{:.1}k", value / 1_000.)
    } else if value.fract().abs() < 0.00001 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}
fn paint_path(
    points: &[[f32; 2]],
    origin: Point<Pixels>,
    scale: f32,
    width: f32,
    closed: bool,
    color: Hsla,
    window: &mut Window,
) {
    if points.is_empty() {
        return;
    }
    let mut path = if closed {
        PathBuilder::fill()
    } else {
        PathBuilder::stroke(px(width))
    };
    for (i, p) in points.iter().enumerate() {
        let p = origin + point(px(p[0] * scale), px(p[1] * scale));
        if i == 0 {
            path.move_to(p);
        } else {
            path.line_to(p);
        }
    }
    if closed {
        path.close();
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}
