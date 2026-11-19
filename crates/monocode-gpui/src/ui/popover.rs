// Anchored popovers and menus, ported from src/shared/ui/Popover.tsx, its
// placement math in src/shared/lib/popover.ts, and the row/keyboard model of
// src/features/files/ui/ExplorerMenu.tsx.
mod menu;
pub(crate) mod placement;
use crate::ui::theme::*;
use gpui::{
    AnyElement, AnyView, App, Bounds, BoxShadow, Context, Corners, DispatchPhase, Display, Element,
    ElementId, Entity, EventEmitter, FocusHandle, Focusable, GlobalElementId, Hsla,
    InspectorElementId, KeyBinding, LayoutId, MouseDownEvent, Pixels, Point,
    Position as CssPosition, Role, SharedString, Style, Toggled, Window, actions, canvas, deferred,
    div, point, prelude::*, px, rgb, rgba, size,
};
use placement::Placed;
#[cfg(test)]
use placement::place;
use std::{cell::Cell, rc::Rc, time::Instant};

// Bound to escape by the shell; the name stays `workspace::DismissMenu`.
actions!(workspace, [DismissMenu]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Top,
    Bottom,
    Left,
    Right,
}
impl Side {
    fn opposite(self) -> Self {
        match self {
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
    fn vertical(self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Align {
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Options {
    /// Preferred side of the anchor; flips only when the space runs out.
    pub side: Side,
    pub align: Align,
    /// Space between anchor and popover. Negative overlaps, as flyouts like to.
    pub gap: f32,
    /// Breathing room kept against every viewport edge.
    pub padding: f32,
    pub width: Option<f32>,
    /// Height the popover wants before it is worth flipping to the other side.
    pub min_height: Option<f32>,
    pub max_height: Option<f32>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            side: Side::Bottom,
            align: Align::Start,
            gap: 6.,
            padding: 8.,
            width: None,
            min_height: None,
            max_height: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}
impl Rect {
    fn from_bounds(b: Bounds<Pixels>) -> Self {
        Self {
            left: f32::from(b.left()),
            top: f32::from(b.top()),
            right: f32::from(b.right()),
            bottom: f32::from(b.bottom()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placement {
    pub side: Side,
    pub left: f32,
    /// Set for every side except `Top`, which is pinned by its bottom edge.
    pub top: Option<f32>,
    /// Pinning a top-side popover here keeps it still while its content grows.
    pub bottom: Option<f32>,
    pub width: f32,
    pub max_height: f32,
}

fn clamp(value: f32, min: f32, max: f32) -> f32 {
    value.max(min).min(max.max(min))
}

fn room_on(side: Side, a: &Rect, viewport: (f32, f32), gap: f32, padding: f32) -> f32 {
    match side {
        Side::Top => a.top - gap - padding,
        Side::Bottom => viewport.1 - a.bottom - gap - padding,
        Side::Left => a.left - gap - padding,
        Side::Right => viewport.0 - a.right - gap - padding,
    }
}

fn cross_start(align: Align, start: f32, end: f32, length: f32) -> f32 {
    match align {
        Align::Start => start,
        Align::End => end - length,
        Align::Center => start + (end - start) / 2. - length / 2.,
    }
}

/// Window bounds of a trigger, written while it is laid out so a popover can
/// open against it. Put [`AnchorSlot::probe`] inside the trigger.
#[derive(Clone, Default)]
pub(crate) struct AnchorSlot(Rc<Cell<Option<Bounds<Pixels>>>>);
impl AnchorSlot {
    pub(crate) fn get(&self) -> Option<Bounds<Pixels>> {
        self.0.get()
    }
    pub(crate) fn set(&self, bounds: Option<Bounds<Pixels>>) {
        self.0.set(bounds)
    }
    pub(crate) fn probe(&self) -> impl IntoElement {
        let slot = self.clone();
        // Pinned to the corner: an absolute child left at its static
        // position starts inside the trigger's padding, which would shift
        // every popover by it.
        canvas(move |bounds, _, _| slot.set(Some(bounds)), |_, _, _, _| {})
            .absolute()
            .top_0()
            .left_0()
            .size_full()
    }
    /// A probe for sizes the next render lays out from: a change of size
    /// asks for another frame, so nothing waits on unrelated input to catch up.
    pub(crate) fn layout_probe(&self) -> impl IntoElement {
        let slot = self.clone();
        canvas(
            move |bounds, window, _| {
                let changed = slot.get().is_none_or(|old| old.size != bounds.size);
                slot.set(Some(bounds));
                if changed {
                    window.refresh();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }
}

/// A trigger, or a bare point for context menus.
#[derive(Clone)]
pub(crate) enum Anchor {
    Slot(AnchorSlot),
    Point(Point<Pixels>),
    /// Centre the measured card in the window, for modals.
    Center,
    /// Follow the row vertically, but always measure the gap from the parent
    /// card's edge, regardless of its row padding or menu style.
    Submenu {
        row: AnchorSlot,
        surface: AnchorSlot,
    },
}
impl Anchor {
    fn bounds(&self) -> Bounds<Pixels> {
        match self {
            Self::Slot(slot) => slot.get().unwrap_or_default(),
            Self::Point(p) => Bounds::new(*p, size(px(0.), px(0.))),
            Self::Center => Bounds::default(),
            Self::Submenu { row, surface } => {
                let row = row.get().unwrap_or_default();
                let surface = surface.get().unwrap_or(row);
                Bounds::new(
                    point(surface.left(), row.top()),
                    size(surface.size.width, row.size.height),
                )
            }
        }
    }
}

type Dismiss = Rc<dyn Fn(&mut Window, &mut App)>;

// Popover.tsx / .popover-open: 170ms, cubic-bezier(.16, 1, .3, 1),
// fading and travelling 8px from the resolved anchor side. The whole card
// moves together so content cannot slide inside its frame. Dismissal is immediate.
const MENU_IN: f32 = 0.17;
const MENU_TRAVEL: f32 = 8.;
const SUBMENU_GAP: f32 = 4.;
const RADIUS: f32 = 12.;
// GlassBackdrop's backdrop-blur-xl.
#[cfg(any(target_os = "macos", target_os = "windows"))]
const BLUR: f32 = 24.;

fn entrance(elapsed: f32, reduced: bool) -> f32 {
    if reduced {
        1.
    } else {
        crate::ui::motion::reveal::bezier((elapsed / MENU_IN).clamp(0., 1.), 0.16, 1., 0.3, 1.)
    }
}
fn entrance_offset(side: Side, progress: f32) -> Point<Pixels> {
    let remaining = px(MENU_TRAVEL * (1. - progress));
    match side {
        Side::Top => point(px(0.), remaining),
        Side::Bottom => point(px(0.), -remaining),
        Side::Left => point(remaining, px(0.)),
        Side::Right => point(-remaining, px(0.)),
    }
}

/// A menu, dropdown or flyout drawn above everything else in the window,
/// placed against its anchor with viewport flipping (Popover.tsx's
/// placement). The glass frame, 10% border and shadow follow the React
/// component; the whole card fades and slides from the anchored edge.
/// Pressing outside it (and outside its anchor)
/// dismisses it, and that press reaches nothing else.
#[derive(IntoElement)]
pub(crate) struct Popover {
    id: ElementId,
    anchor: Anchor,
    options: Options,
    padding: f32,
    priority: usize,
    track: Vec<AnchorSlot>,
    ignore: Vec<AnchorSlot>,
    on_dismiss: Option<Dismiss>,
    consume_outside_press: bool,
    modal_classic: Option<bool>,
    animate: bool,
    /// Without a fixed width, the card hugs its content between these.
    fit: (Option<f32>, Option<f32>),
    children: Vec<AnyElement>,
}
pub(crate) fn popover(id: impl Into<ElementId>, anchor: Anchor) -> Popover {
    Popover {
        id: id.into(),
        anchor,
        options: Options::default(),
        padding: 0.,
        priority: 40,
        track: Vec::new(),
        ignore: Vec::new(),
        on_dismiss: None,
        consume_outside_press: true,
        modal_classic: None,
        animate: true,
        fit: (None, None),
        children: Vec::new(),
    }
}
impl Popover {
    /// Dismiss without swallowing the next click. Text-selection toolbars and
    /// link menus let that press select text or open another link's menu.
    pub(crate) fn allow_outside_press(mut self) -> Self {
        self.consume_outside_press = false;
        self
    }
    pub(crate) fn without_animation(mut self) -> Self {
        self.animate = false;
        self
    }
    /// The standard modal card ([`crate::modal`]): 16px corners, the 10%
    /// rule and the dialog tint.
    pub(crate) fn modal_surface(mut self, classic: bool) -> Self {
        self.modal_classic = Some(classic);
        self
    }
    pub(crate) fn align(mut self, align: Align) -> Self {
        self.options.align = align;
        self
    }
    pub(crate) fn side(mut self, side: Side) -> Self {
        self.options.side = side;
        self
    }
    pub(crate) fn gap(mut self, gap: f32) -> Self {
        self.options.gap = gap;
        self
    }
    pub(crate) fn width(mut self, width: f32) -> Self {
        self.options.width = Some(width);
        self
    }
    /// Sizes the card to its content, wrapping text past `max` instead of
    /// leaving a fixed width's empty space beside short content.
    pub(crate) fn max_width(mut self, max: f32) -> Self {
        self.options.width = None;
        self.fit.1 = Some(max);
        self
    }
    pub(crate) fn min_width(mut self, min: f32) -> Self {
        self.fit.0 = Some(min);
        self
    }
    /// A fixed height, such as the model flyout's: `minHeight = maxHeight`.
    pub(crate) fn height(mut self, height: f32) -> Self {
        self.options.min_height = Some(height);
        self.options.max_height = Some(height);
        self
    }
    pub(crate) fn max_height(mut self, height: f32) -> Self {
        self.options.max_height = Some(height);
        self
    }
    /// The content's own padding, the caller's `className="p-…"`.
    pub(crate) fn padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }
    pub(crate) fn priority(mut self, priority: usize) -> Self {
        self.priority = priority;
        self
    }
    /// Records the surface's bounds, so another popover can ignore them.
    pub(crate) fn track(mut self, slot: AnchorSlot) -> Self {
        self.track.push(slot);
        self
    }
    /// A press inside this region is not "outside", like a submenu's surface.
    pub(crate) fn ignore(mut self, slot: AnchorSlot) -> Self {
        self.ignore.push(slot);
        self
    }
    pub(crate) fn on_dismiss(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(f));
        self
    }
}
impl ParentElement for Popover {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements)
    }
}
impl RenderOnce for Popover {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // Keyed like the surface, so it lives exactly as long as the popover
        // stays rendered and the entrance replays on every open.
        let opened = window.use_keyed_state(self.id.clone(), cx, |_, _| Instant::now());
        let enter = if self.animate {
            entrance(opened.read(cx).elapsed().as_secs_f32(), cx.reduce_motion())
        } else {
            1.
        };
        if enter < 1. {
            window.request_animation_frame();
        }
        self.render_at(enter, window.viewport_size())
    }
}
impl Popover {
    fn render_at(self, enter: f32, viewport: gpui::Size<Pixels>) -> impl IntoElement {
        let (viewport_width, viewport_height) = (viewport.width, viewport.height);
        let radius = if self.modal_classic.is_some() {
            16.
        } else {
            RADIUS
        };
        let border = if self.modal_classic.is_some() {
            crate::ui::modal::BORDER
        } else {
            0.1
        };
        let tint = if self.modal_classic.is_some() {
            let mut fill = crate::ui::theme::pane_fill();
            fill.a = 0.55;
            fill
        } else {
            tint()
        };
        let viewport = f32::from(viewport_height) - self.options.padding * 2.;
        let max_height = self
            .options
            .max_height
            .unwrap_or(f32::INFINITY)
            .min(viewport);
        let frame = div()
            .id(self.id)
            .occlude()
            .relative()
            .flex()
            .flex_col()
            .overflow_hidden()
            .max_h(px(max_height))
            .when_some(self.options.min_height, |d, h| d.h(px(h.min(max_height))))
            .when_some(self.options.width, |d, w| d.w(px(w)))
            .when_some(self.fit.0, |d, w| d.min_w(px(w)))
            .when_some(self.fit.1, |d, w| {
                d.max_w(px(
                    w.min(f32::from(viewport_width) - self.options.padding * 2.)
                ))
            })
            .rounded(px(radius))
            .opacity(enter)
            .shadow(shadow())
            .child(glass(tint, radius))
            .child(
                div()
                    .relative()
                    .m(px(1.))
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .p(px(self.padding))
                    .children(self.children),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(px(radius))
                    .border_1()
                    .border_color(content(border)),
            );
        deferred(Placed {
            anchor: self.anchor,
            options: self.options,
            progress: enter,
            frame: frame.into_any_element(),
            track: self.track,
            ignore: self.ignore,
            on_dismiss: self.on_dismiss,
            consume_outside_press: self.consume_outside_press,
        })
        .with_priority(self.priority)
    }
}
fn tint() -> Hsla {
    content(0.02)
}
// On macOS the renderer blurs what is already drawn behind this quad.
// Light themes, and platforms without the blur, paint an opaque card.
fn glass(tint: Hsla, radius: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| paint_glass(bounds, Corners::all(px(radius)), tint, window),
    )
    .absolute()
    .size_full()
}
/// The popover surface: `tint` over a backdrop blur, or the opaque card.
pub(crate) fn paint_glass(
    bounds: Bounds<Pixels>,
    corners: Corners<Pixels>,
    tint: Hsla,
    window: &mut Window,
) {
    let opaque = crate::ui::theme::floating_fill();
    let light = crate::ui::theme::is_light();
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let quad = if light {
        gpui::fill(bounds, opaque)
    } else {
        gpui::fill(bounds, tint)
            .border_color(gpui::backdrop_blur_marker(BLUR * window.scale_factor()))
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let quad = {
        let _ = (light, tint);
        gpui::fill(bounds, opaque)
    };
    window.paint_quad(quad.corner_radii(corners));
}
/// A whole popover surface painted directly, for cards drawn inside a
/// canvas such as chart readouts: shadow, glass (or the light theme's opaque
/// card) and the 10% border, at the popover's radius.
pub(crate) fn paint_surface(bounds: Bounds<Pixels>, window: &mut Window) {
    let corners = Corners::all(px(RADIUS));
    window.paint_drop_shadows(bounds, corners, &shadow());
    paint_glass(bounds, corners, tint(), window);
    window.paint_quad(
        gpui::fill(bounds, gpui::transparent_black())
            .corner_radii(corners)
            .border_widths(px(1.))
            .border_color(content(0.1)),
    );
}
/// The default popover glass tint.
pub(crate) fn glass_tint() -> Hsla {
    tint()
}
/// The popover's two-layer drop shadow.
pub(crate) fn shadow() -> Vec<BoxShadow> {
    vec![
        BoxShadow::new(px(0.), px(20.), rgba(0x0000001a).into())
            .blur_radius(px(25.))
            .spread_radius(px(-5.)),
        BoxShadow::new(px(0.), px(8.), rgba(0x0000001a).into())
            .blur_radius(px(10.))
            .spread_radius(px(-6.)),
    ]
}

actions!(
    popover_menu,
    [
        SelectPrevious,
        SelectNext,
        Confirm,
        Cancel,
        OpenSubmenu,
        CloseSubmenu
    ]
);
pub(crate) fn bind(cx: &mut App) {
    let context = Some("PopoverMenu");
    cx.bind_keys([
        KeyBinding::new("up", SelectPrevious, context),
        KeyBinding::new("down", SelectNext, context),
        KeyBinding::new("enter", Confirm, context),
        KeyBinding::new("escape", Cancel, context),
        KeyBinding::new("right", OpenSubmenu, context),
        KeyBinding::new("left", CloseSubmenu, context),
    ]);
}

/// Which React menu a [`Menu`] draws like. Each has its own row metrics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MenuStyle {
    /// ExplorerMenu: tab and session context menus, and every submenu.
    Explorer,
    /// TabGroupMenu: the project context menu. Icons at 55%, hover only.
    Group,
    /// SidebarWorktreeSwitcher: the sidebar project header's menu.
    Switcher,
    /// The composer's plus menu: an icon, title and one-line hint per row.
    Composer,
    /// AccessPicker: like Composer, with a medium title, wrapping hint and
    /// the selected row filled instead of checked.
    Access,
    /// ModelPicker's main menu: a setting per row, its value on the right.
    Settings,
    /// GitChangesPanel's branch and commit popovers: `text-[12px]` rows at
    /// a 10% hover, disabled rows at 40%.
    Git,
    /// SessionFiltersMenu's 228px card and compact checkbox rows.
    Filter,
    /// ModelPicker's recent models: 40px rows of harness icon, model name
    /// over its harness, and a muted check on the current one.
    Recent,
}
impl MenuStyle {
    fn padding(self) -> f32 {
        match self {
            Self::Group => 8.,
            Self::Composer => 6.,
            _ => 4.,
        }
    }
    /// Rows follow the keyboard highlight; the rest only react to hover.
    fn highlights(self) -> bool {
        matches!(
            self,
            Self::Explorer | Self::Access | Self::Settings | Self::Filter | Self::Recent
        )
    }
}

/// Where a row's flyout comes from: a whole view, like the model list.
#[derive(Clone)]
pub(crate) struct Flyout {
    pub view: AnyView,
    pub width: f32,
    /// Card inset for content that does not provide its own padding.
    pub padding: f32,
    pub height: Option<f32>,
    pub max_height: Option<f32>,
    /// Focused when the flyout opens, such as its search field.
    pub focus: Option<FocusHandle>,
    /// A nested Menu uses its host card as the parent of further submenus.
    pub surface: Option<AnchorSlot>,
}

#[derive(Clone, Default)]
pub(crate) struct MenuAction {
    pub id: SharedString,
    pub label: SharedString,
    pub description: Option<SharedString>,
    pub shortcut: Option<SharedString>,
    pub icon: Option<&'static str>,
    pub icon_artwork: Option<&'static str>,
    pub image: Option<&'static str>,
    pub icon_color: Option<Hsla>,
    /// A provider logo shown with `value`, as the model row's harness icon.
    pub value_image: Option<&'static str>,
    pub value: Option<SharedString>,
    pub badge: Option<SharedString>,
    pub disabled: bool,
    pub danger: bool,
    pub checked: bool,
    /// A toggle row uses a switch instead of the selected-item checkmark.
    pub switch: bool,
    /// Checkbox-style actions can update their owner without dismissing the menu.
    pub keep_open: bool,
    pub submenu: Vec<MenuAction>,
    pub flyout: Option<Flyout>,
}
#[derive(Clone)]
pub(crate) enum MenuItem {
    /// Interactive appearance controls above a project's action rows.
    Custom(AnyView, FocusHandle),
    /// A quiet uppercase caption, like the composer's "Add to message".
    Header(SharedString),
    Separator,
    Action(Box<MenuAction>),
    /// A footnote paragraph, like AccessPicker's busy note.
    Note(SharedString),
}
impl From<MenuAction> for MenuItem {
    fn from(action: MenuAction) -> Self {
        Self::Action(Box::new(action))
    }
}

pub(crate) enum MenuEvent {
    /// The picked action, and where its row was, for menus opened from it.
    Picked(SharedString, Option<Bounds<Pixels>>),
    Dismissed,
}
/// A separate event so lazy flyouts can load without dismissing menu owners.
pub(crate) struct MenuOpened(pub SharedString);

const MENU_WIDTH: f32 = 228.;
/// Content-sized menus never shrink below this.
const MENU_MIN_WIDTH: f32 = 160.;
// Tailwind v4 reds.
const RED_300: u32 = 0xffa2a2;
const RED_500: u32 = 0xfb2c36;

/// ExplorerMenu and its siblings: takes focus on open, highlights under the
/// pointer and with the arrow keys, and opens one level of submenu or flyout.
/// Picking or dismissing returns focus to wherever it was before.
pub(crate) struct Menu {
    label: SharedString,
    items: Vec<MenuItem>,
    style: MenuStyle,
    width: f32,
    row_metrics: Option<(f32, f32)>,
    search_navigation: bool,
    submenu_width: Option<f32>,
    active: Option<usize>,
    submenu: Option<usize>,
    sub_active: Option<usize>,
    rows: Vec<AnchorSlot>,
    surface: AnchorSlot,
    sub_region: AnchorSlot,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
}
impl EventEmitter<MenuEvent> for Menu {}
impl EventEmitter<MenuOpened> for Menu {}
impl Focusable for Menu {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
fn separator(ix: usize, style: MenuStyle) -> AnyElement {
    div()
        .id(("menu-separator", ix))
        .role(Role::Splitter)
        .flex_shrink_0()
        .my_1()
        .h(px(1.))
        .map(|d| {
            if style == MenuStyle::Switcher {
                // `mx-2 bg-stroke`.
                d.mx_2().bg(content(0.07))
            } else if style == MenuStyle::Git {
                // `mx-1 border-t border-content/10`.
                d.mx_1().bg(content(0.1))
            } else {
                d.bg(content(0.1))
            }
        })
        .into_any()
}

/// All child menus and picker flyouts share the same card-edge gap, flipping
/// and layer. Their content only controls size and padding.
fn submenu_popover(
    id: impl Into<ElementId>,
    row: AnchorSlot,
    surface: AnchorSlot,
    region: AnchorSlot,
) -> Popover {
    popover(id, Anchor::Submenu { row, surface })
        .side(Side::Right)
        .gap(SUBMENU_GAP)
        .priority(41)
        .track(region)
}

/// Wraps `menu` in its popover. Pressing outside it, or outside its open
/// submenu or flyout, dismisses it like Escape does.
pub(crate) fn menu_popover(menu: &Entity<Menu>, anchor: Anchor, cx: &App) -> Popover {
    let m = menu.read(cx);
    let weak = menu.downgrade();
    // Plain menus hug their rows up to their width; embedded views keep it.
    let custom = m
        .items
        .iter()
        .any(|item| matches!(item, MenuItem::Custom(..)));
    popover(("menu", menu.entity_id().as_u64() as usize), anchor)
        .map(|p| {
            if custom || m.style == MenuStyle::Filter {
                p.width(m.width)
            } else {
                p.max_width(m.width).min_width(MENU_MIN_WIDTH.min(m.width))
            }
        })
        .when(m.style == MenuStyle::Filter, |p| p.max_height(480.))
        .padding(m.style.padding())
        .track(m.surface.clone())
        .ignore(m.sub_region.clone())
        .on_dismiss(move |window, cx| {
            let _ = weak.update(cx, |m, cx| m.finish(MenuEvent::Dismissed, window, cx));
        })
        .child(menu.clone())
}

#[cfg(test)]
mod tests;
