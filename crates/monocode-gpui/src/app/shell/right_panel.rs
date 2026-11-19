//! Sidebar.tsx's inset `RightPanel`: the project tools beside the card, with
//! reorderable Explorer / Changes / History tabs in the title bar, and a
//! trailing Terminal tab when the terminal lives in the panel.
mod frame;
use crate::ui::panel::{RightTab, clamp_graph, stroke, tab_icons_width};
use crate::{
    git::{changes::Changes, history::History},
    project::{
        explorer::Explorer,
        tabs::{TabLabel, TabReveal, tab_button},
    },
    runtime::git_status::{GitChanged, ProjectGit},
    ui::theme::*,
};
use gpui::{
    AnyElement, App, Bounds, Context, CursorStyle, Entity, EventEmitter, FocusHandle, Focusable,
    MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Role, StyleRefinement, Subscription,
    WeakEntity, Window, div, prelude::*, px,
};

/// `count` 26px icon tabs, one unfolded label with 4px extra padding, and the gaps.
pub(crate) fn tabs_width(count: usize) -> f32 {
    tab_icons_width(count) + 64. + 4.
}

pub(crate) fn tab_label_width(width: f32, count: usize) -> f32 {
    // The selected tab adds 2px of padding per side. Below 30px there is
    // insufficient room for a useful abbreviated label and its 6px gap.
    let available = (width - tab_icons_width(count) - 4.).max(0.);
    if available < 30. { 0. } else { available }
}

pub(crate) enum PanelEvent {
    Tab(RightTab),
    Reordered(Vec<RightTab>),
}

struct TabDrag {
    tab: RightTab,
    origin: Pixels,
    offset: Pixels,
    active: bool,
}

/// The title-bar strip observes the panel so selection and drag motion
/// repaint independently of the cached pane and session tabs.
pub(crate) struct ProjectTabs {
    panel: WeakEntity<RightPanel>,
    _subscription: Subscription,
}

impl Render for ProjectTabs {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.panel
            .update(cx, |panel, cx| panel.tab_strip(window, cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}

pub(crate) struct RightPanel {
    pub tabs: Entity<ProjectTabs>,
    pub explorer: Entity<Explorer>,
    pub changes: Entity<Changes>,
    pub history: Entity<History>,
    /// The commit graph under Changes, VS Code's Source Control "Graph".
    pub graph: Entity<History>,
    graph_open: bool,
    graph_height: f32,
    /// The pointer's y and the height when the divider was pressed.
    graph_drag: Option<(f32, f32)>,
    split: crate::ui::popover::AnchorSlot,
    focus: FocusHandle,
    tab: RightTab,
    order: Vec<RightTab>,
    shown: bool,
    sidebar_tab: Option<RightTab>,
    /// Whether the Terminal tab follows the others; the shell draws its pane.
    terminal_tab: bool,
    /// Classic reserves the title bar's height; rounded content starts at
    /// the card's top edge, below the window-wide title bar.
    top_inset: f32,
    tab_width: f32,
    drag: Option<TabDrag>,
    reveal: Option<TabReveal>,
    skip_click: bool,
    tab_bounds: Vec<crate::ui::popover::AnchorSlot>,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<PanelEvent> for RightPanel {}
impl Focusable for RightPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl RightPanel {
    pub(crate) fn new(
        git: Entity<ProjectGit>,
        tab: RightTab,
        order: Vec<RightTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let explorer = cx.new(|cx| Explorer::new(git.clone(), window, cx));
        let changes = cx.new(|cx| Changes::new(git.clone(), window, cx));
        let history = cx.new(|cx| History::new(git.clone(), false, window, cx));
        let graph = cx.new(|cx| History::new(git.clone(), true, window, cx));
        let subscriptions = vec![
            cx.observe(&git, |_, _, cx| cx.notify()),
            cx.subscribe(&git, |_, _, _: &GitChanged, cx| cx.notify()),
            // Only a collapse changes the split; hovers stay in the cached pane.
            cx.observe(&graph, |this, graph, cx| {
                let open = graph.read(cx).open;
                if this.graph_open != open {
                    this.graph_open = open;
                    cx.notify();
                }
            }),
        ];
        let panel = cx.entity();
        let tabs = cx.new(|cx| ProjectTabs {
            panel: panel.downgrade(),
            _subscription: cx.observe(&panel, |_, _, cx| cx.notify()),
        });
        Self {
            tabs,
            focus: cx.focus_handle(),
            explorer,
            changes,
            history,
            graph,
            graph_open: true,
            graph_height: 280.,
            graph_drag: None,
            split: Default::default(),
            tab,
            order: RightTab::normalize(&order),
            shown: false,
            sidebar_tab: None,
            terminal_tab: false,
            top_inset: 0.,
            tab_width: tabs_width(3),
            drag: None,
            reveal: None,
            skip_click: false,
            tab_bounds: (0..4).map(|_| Default::default()).collect(),
            _subscriptions: subscriptions,
        }
    }
    pub(crate) fn tab(&self) -> RightTab {
        self.tab
    }
    /// The tabs in strip order: the saved two, with Terminal between them
    /// when enabled.
    pub(crate) fn tabs(&self) -> Vec<RightTab> {
        let mut tabs = self.order.clone();
        if self.terminal_tab {
            tabs.insert(1.min(tabs.len()), RightTab::Terminal);
        }
        tabs
    }
    /// Adds or removes the Terminal tab; Explorer takes over when it goes.
    pub(crate) fn set_terminal_tab(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.terminal_tab == enabled {
            return;
        }
        self.terminal_tab = enabled;
        if !enabled && self.tab == RightTab::Terminal {
            self.select(RightTab::Files, cx);
        }
        cx.notify();
    }
    pub(crate) fn set_show_excluded(&mut self, show: bool, cx: &mut Context<Self>) {
        self.explorer
            .update(cx, |explorer, cx| explorer.set_show_excluded(show, cx));
    }
    pub(crate) fn top_inset(&self) -> f32 {
        self.top_inset
    }
    pub(crate) fn set_top_inset(&mut self, height: f32, cx: &mut Context<Self>) {
        if self.top_inset != height {
            self.top_inset = height;
            cx.notify();
        }
    }
    pub(crate) fn set_tab_width(&mut self, width: f32, cx: &mut Context<Self>) {
        if self.tab_width != width {
            self.tab_width = width;
            cx.notify();
        }
    }
    /// The panes read and poll only while their tab is on screen.
    pub(crate) fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.shown == shown {
            return;
        }
        self.shown = shown;
        self.sync_panes(cx);
    }
    pub(crate) fn select(&mut self, tab: RightTab, cx: &mut Context<Self>) {
        if self.tab != tab {
            self.reveal = Some(TabReveal::new(tab, self.tab));
        }
        self.tab = tab;
        self.sync_panes(cx);
        cx.emit(PanelEvent::Tab(tab));
        cx.notify();
    }
    pub(crate) fn set_sidebar_tab(&mut self, tab: Option<RightTab>, cx: &mut Context<Self>) {
        if self.sidebar_tab != tab {
            self.sidebar_tab = tab;
            self.sync_panes(cx);
        }
    }
    fn sync_panes(&mut self, cx: &mut Context<Self>) {
        let (shown, tab) = match self.sidebar_tab {
            Some(tab) => (true, tab),
            None => (self.shown, self.tab),
        };
        self.explorer.update(cx, |e, cx| {
            e.set_enabled(shown && tab == RightTab::Files, cx)
        });
        self.changes.update(cx, |c, cx| {
            c.set_enabled(shown && tab == RightTab::Changes, cx)
        });
        self.graph.update(cx, |h, cx| {
            h.set_enabled(shown && tab == RightTab::Changes, cx)
        });
        self.history.update(cx, |h, cx| {
            h.set_enabled(shown && tab == RightTab::Graph, cx)
        });
    }
    pub(crate) fn focus_pane(&self, window: &mut Window, cx: &mut App) {
        let focus = match (self.shown, self.tab) {
            (true, RightTab::Files) => self.explorer.focus_handle(cx),
            (true, RightTab::Graph) => self.history.focus_handle(cx),
            _ => self.focus.clone(),
        };
        window.focus(&focus, cx);
    }
    pub(crate) fn finish_opening(&self, window: &mut Window, cx: &mut App) {
        if self.focus.is_focused(window) {
            self.focus_pane(window, cx);
        }
    }

    fn start_drag(&mut self, tab: RightTab, event: &MouseDownEvent) {
        self.drag = Some(TabDrag {
            tab,
            origin: event.position.x,
            offset: px(0.),
            active: false,
        });
    }
    fn move_drag(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let tabs = self.tabs();
        let Some(drag) = &mut self.drag else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.finish_drag(cx);
            return;
        }
        drag.offset = event.position.x - drag.origin;
        if !drag.active && drag.offset.abs() < px(4.) {
            return;
        }
        drag.active = true;
        // Swap with a neighbour once the dragged tab's centre crosses its centre.
        // Terminal holds its slot, outside the reorderable tabs.
        let Some(from) = tabs.iter().position(|t| *t == drag.tab) else {
            return;
        };
        let bounds: Vec<Option<Bounds<Pixels>>> = tabs
            .iter()
            .zip(&self.tab_bounds)
            .map(|(tab, slot)| slot.get().filter(|_| *tab != RightTab::Terminal))
            .collect();
        if let Some(Some(own)) = bounds.get(from) {
            let centre = own.center().x + drag.offset;
            let target = bounds
                .iter()
                .enumerate()
                .filter(|(ix, _)| *ix != from)
                .find(|(ix, b)| {
                    b.is_some_and(|b| {
                        if *ix > from {
                            centre > b.center().x
                        } else {
                            centre < b.center().x
                        }
                    })
                })
                .map(|(ix, _)| ix);
            if let Some(to) = target
                && let Some(Some(other)) = bounds.get(to)
            {
                // A selected tab includes its label, so moving right crosses
                // the other tabs' widths rather than the selected tab's width.
                let shift = if to > from {
                    other.right() - own.right()
                } else {
                    other.left() - own.left()
                };
                let (a, b) = (
                    self.order.iter().position(|t| *t == tabs[from]),
                    self.order.iter().position(|t| *t == tabs[to]),
                );
                if let (Some(a), Some(b)) = (a, b) {
                    self.order.swap(a, b);
                }
                drag.origin += shift;
                drag.offset -= shift;
            }
        }
        cx.notify();
    }
    fn finish_drag(&mut self, cx: &mut Context<Self>) {
        if let Some(drag) = self.drag.take()
            && drag.active
        {
            self.skip_click = true;
            cx.emit(PanelEvent::Reordered(self.order.clone()));
            cx.notify();
        }
    }

    fn tab_strip(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let progress = self
            .reveal
            .as_ref()
            .and_then(|reveal| reveal.progress(self.tab, window, cx));
        let dragging = self
            .drag
            .as_ref()
            .filter(|d| d.active)
            .map(|d| (d.tab, d.offset));
        let count = self.tabs().len();
        let tabs: Vec<AnyElement> = self
            .tabs()
            .into_iter()
            .enumerate()
            .map(|(ix, tab)| {
                let active = self.tab == tab;
                let (id, path) = match tab {
                    RightTab::Files => ("right-panel-tab-Explorer", "file-script"),
                    RightTab::Changes => ("right-panel-tab-Changes", "git-branch"),
                    RightTab::Graph => ("right-panel-tab-History", "clock"),
                    RightTab::Terminal => ("right-panel-tab-Terminal", "terminal"),
                };
                let unfolded = match progress {
                    Some((_, p)) if active => p,
                    Some((closing, p)) if tab == closing => 1. - p,
                    _ if active => 1.,
                    _ => 0.,
                };
                let shift = dragging.filter(|(t, _)| *t == tab).map(|(_, o)| o);
                div()
                    .relative()
                    .flex_shrink_0()
                    .flex()
                    .items_stretch()
                    // Keep the measured slot still while its button follows
                    // the pointer; drag offsets are relative to the slot.
                    .child(self.tab_bounds[ix].probe())
                    .child(
                        tab_button(
                            id,
                            TabLabel::from(tab.label())
                                .max_width(tab_label_width(self.tab_width, count)),
                            path,
                            active,
                            false,
                            unfolded,
                            window,
                        )
                        .relative()
                        .when_some(shift, |d, o| d.left(o))
                        .when(!active && shift.is_some(), |d| {
                            d.opacity(1.).text_color(content(1.))
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, _, _| {
                                this.start_drag(tab, event)
                            }),
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                if std::mem::take(&mut this.skip_click) {
                                    return;
                                }
                                this.select(tab, cx);
                                this.focus_pane(window, cx);
                            },
                        )),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .id("right-panel-tabs")
            .debug_selector(|| "right-panel-tabs".into())
            .role(Role::TabList)
            .aria_label("Project")
            .h(px(TITLE_BAR_HEIGHT))
            .w_full()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(2.))
            .on_mouse_move(
                cx.listener(|this, event: &MouseMoveEvent, _, cx| this.move_drag(event, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_drag(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_drag(cx)),
            )
            .children(tabs)
            .into_any_element()
    }
}

impl RightPanel {
    fn drag_graph(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some((start, height)) = self.graph_drag else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.graph_drag = None;
            cx.notify();
            return;
        }
        let total = self
            .split
            .get()
            .map_or(f32::INFINITY, |b| b.size.height.as_f32());
        self.graph_height = clamp_graph(height + start - event.position.y.as_f32(), total);
        cx.notify();
    }
    fn end_graph_drag(&mut self, cx: &mut Context<Self>) {
        if self.graph_drag.take().is_some() {
            cx.notify();
        }
    }
    /// Changes above, the GRAPH section below, a divider between them.
    fn changes_split(&self, cx: &mut Context<Self>) -> AnyElement {
        let total = self
            .split
            .get()
            .map_or(f32::INFINITY, |b| b.size.height.as_f32());
        let height = if self.graph_open {
            clamp_graph(self.graph_height, total)
        } else {
            PANE_HEADER_HEIGHT
        };
        let sash = div()
            .id("graph-sash")
            .group("resize-handle")
            .role(Role::Splitter)
            .aria_label("Resize graph")
            .absolute()
            .top(px(-3.5))
            .left_0()
            .right_0()
            .h(px(6.))
            .cursor(CursorStyle::ResizeUpDown)
            .child(crate::app::shell::panes::drag::resize_border(
                false,
                self.graph_drag.is_some(),
            ))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.graph_drag = Some((event.position.y.as_f32(), this.graph_height));
                    cx.notify();
                }),
            );
        div()
            .id("right-panel-changes-split")
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_col()
            .child(self.split.layout_probe())
            .child(
                self.changes
                    .clone()
                    .cached(StyleRefinement::default().flex_1().min_h_0().w_full()),
            )
            .child(
                div()
                    .relative()
                    .flex_shrink_0()
                    .w_full()
                    .h(px(height))
                    .border_t_1()
                    .border_color(stroke())
                    .child(
                        self.graph
                            .clone()
                            .cached(StyleRefinement::default().size_full()),
                    )
                    .when(self.graph_open, |d| d.child(sash)),
            )
            .on_mouse_move(
                cx.listener(|this, event: &MouseMoveEvent, _, cx| this.drag_graph(event, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.end_graph_drag(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.end_graph_drag(cx)),
            )
            .into_any_element()
    }
}

impl Render for RightPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pane: AnyElement = match self.tab {
            RightTab::Files => self
                .explorer
                .clone()
                .cached(StyleRefinement::default().flex_1().min_h_0().w_full())
                .into_any_element(),
            RightTab::Changes => self.changes_split(cx),
            RightTab::Graph => self
                .history
                .clone()
                .cached(StyleRefinement::default().flex_1().min_h_0().w_full())
                .into_any_element(),
            // The shell paints the terminal over this pane.
            RightTab::Terminal => div().into_any_element(),
        };
        div()
            .id("right-panel")
            .track_focus(&self.focus)
            .role(Role::Group)
            .aria_label("Project tools")
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .when(self.top_inset > 0., |d| {
                d.child(
                    div()
                        .h(px(self.top_inset))
                        .flex_shrink_0()
                        .border_b_1()
                        .border_color(stroke()),
                )
            })
            .child(pane)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::panel::format_integer;
    #[test]
    fn tab_order_keeps_known_tabs_once() {
        assert_eq!(
            RightTab::normalize(&[RightTab::Graph, RightTab::Graph]),
            [RightTab::Graph, RightTab::Files]
        );
        assert_eq!(
            RightTab::normalize(&[RightTab::Changes, RightTab::Graph]),
            [RightTab::Graph, RightTab::Files]
        );
        assert_eq!(format_integer(1234567), "1,234,567");
        assert_eq!(format_integer(12), "12");
    }
}
