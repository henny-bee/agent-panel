//! Original project navigation. Its list and shared image cache stay independent
//! from streamed conversations; Git counters use one visible-project batch.
mod drag;
mod header;
mod rows;
use super::super::*;
use crate::{
    app::workspace,
    ui::{
        panel::{format_integer, tabular},
        theme::{DIFF_ADD, DIFF_DEL},
    },
};
use gpui::{ListAlignment, ListState, list};
use monocode_core::vcs::git::scm::DiffStats;

const ICON_RAIL_WIDTH: f32 = 48.;

#[derive(Clone, Debug, PartialEq, Eq)]
enum ProjectRailRow {
    Monos,
    Heading(&'static str),
    Group(String),
    Project { path: String, grouped: bool },
    Gap,
    Empty,
}
pub(crate) struct ProjectRailList {
    state: ListState,
    rows: Vec<ProjectRailRow>,
    compact: bool,
    bounds: HashMap<String, AnchorSlot>,
    group_bounds: HashMap<String, AnchorSlot>,
    shifts: HashMap<String, TabShift>,
    settling: Option<String>,
    skip_click: bool,
}
impl ProjectRailList {
    pub(crate) fn new(cx: &mut Context<Shell>) -> Self {
        let state = ListState::new(0, ListAlignment::Top, px(96.));
        let owner = cx.weak_entity();
        state.set_scroll_handler(move |_, _, cx| {
            let _ = owner.update(cx, |this, cx| {
                for bounds in this
                    .project_rail_list
                    .bounds
                    .values()
                    .chain(this.project_rail_list.group_bounds.values())
                {
                    bounds.set(None);
                }
                this.projects_frame.update(cx, |_, cx| cx.notify());
            });
        });
        Self {
            state,
            rows: Vec::new(),
            compact: false,
            bounds: HashMap::new(),
            group_bounds: HashMap::new(),
            shifts: HashMap::new(),
            settling: None,
            skip_click: false,
        }
    }
    fn update(&mut self, rows: Vec<ProjectRailRow>, compact: bool) {
        if compact != self.compact {
            self.compact = compact;
            self.rows = rows;
            self.state.reset(self.rows.len());
        } else if self.rows != rows {
            let prefix = self
                .rows
                .iter()
                .zip(&rows)
                .take_while(|(a, b)| a == b)
                .count();
            let suffix = self.rows[prefix..]
                .iter()
                .rev()
                .zip(rows[prefix..].iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
            self.state.splice(
                prefix..self.rows.len() - suffix,
                rows.len() - prefix - suffix,
            );
            self.rows = rows;
        }
        // The global Mono roster is one naturally sized item, not a project
        // card. Its row count can change without the project list changing.
        if self.rows.first() == Some(&ProjectRailRow::Monos) {
            self.state.remeasure_items(0..1);
        }
    }
}
#[derive(Default)]
pub(crate) struct ProjectRailStats {
    values: HashMap<String, Option<DiffStats>>,
    queued: HashSet<String>,
    loading: bool,
    deferred: bool,
}
pub(crate) struct ProjectRailDrag {
    path: String,
    group: bool,
    order: Vec<String>,
    bounds: Vec<Bounds<Pixels>>,
    from: usize,
    to: usize,
    origin: Point<Pixels>,
    offset: Pixels,
    active: bool,
}
impl ProjectRailDrag {
    /// Variable-height groups keep their full measured block when they move.
    fn layout(&self, to: usize) -> Vec<Pixels> {
        let mut order: Vec<_> = (0..self.order.len()).collect();
        let moved = order.remove(self.from);
        order.insert(to, moved);
        let gap = self
            .bounds
            .get(1)
            .map_or(px(0.), |b| b.top() - self.bounds[0].bottom());
        let mut top = self.bounds[0].top();
        let mut starts = vec![px(0.); self.order.len()];
        for ix in order {
            starts[ix] = top;
            top += self.bounds[ix].size.height + gap;
        }
        starts
    }
}
fn rail_shift_key(id: &str, group: bool) -> String {
    format!("{}:{id}", if group { "group" } else { "project" })
}
pub(in crate::app::shell) struct ProjectTooltip(pub String);
impl Render for ProjectTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(pane_fill())
            .border_1()
            .border_color(content(0.15))
            .text_size(px(12.))
            .text_color(content(1.))
            .child(self.0.clone())
    }
}
impl Shell {
    pub(crate) fn project_rail_drawn_width(&self) -> f32 {
        if !self.workspace.classic_layout {
            0.
        } else if self.workspace.project_rail_open || self.settings.is_some() {
            self.workspace.project_rail_width
        } else {
            ICON_RAIL_WIDTH
        }
    }
    pub(crate) fn left_sidebar_width(&self, cx: &App) -> f32 {
        // The classic workspace sidebar sits on the right (`right_sidebar_width`).
        self.project_rail_drawn_width()
            + if self.workspace.classic_layout || !self.session_sidebar_available(cx) {
                0.
            } else {
                self.workspace.sidebar_width * self.sidebar_fraction
            }
    }
    pub(crate) fn toggle_primary_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The classic layout's settings stand in for the project rail and
        // can't be hidden.
        if self.settings.is_some() && self.workspace.classic_layout {
            return;
        }
        if self.workspace.classic_layout && self.settings.is_none() {
            self.workspace.project_rail_open = !self.workspace.project_rail_open;
            self.project_resize = None;
            self.fit_sidebar(cx);
            self.persist(cx);
            self.invalidate_chrome(cx);
            cx.notify();
        } else {
            self.toggle_sidebar(window, cx);
        }
    }
    pub(crate) fn project_rail_frame(&self, cx: &mut Context<Self>) -> AnyElement {
        let width = self.project_rail_drawn_width();
        let frame = div()
            .id("project-rail-frame")
            .debug_selector(|| "project-rail-frame".into())
            .relative()
            .w(px(width))
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(content(0.07))
            .child(
                self.projects_frame
                    .clone()
                    .cached(StyleRefinement::default().w(px(width - 1.)).h_full()),
            )
            .when(
                self.workspace.project_rail_open && self.settings.is_none(),
                |d| {
                    d.child(
                        div()
                            .id("project-rail-resize")
                            .debug_selector(|| "project-rail-resize".into())
                            .group("resize-handle")
                            .role(Role::Splitter)
                            .aria_label("Resize project sidebar")
                            .absolute()
                            .right(px(-3.5))
                            .top_0()
                            .bottom_0()
                            .w(px(6.))
                            .cursor(CursorStyle::ResizeLeftRight)
                            .child(resize_border(true, self.project_resize.is_some()))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, e: &MouseDownEvent, _, cx| {
                                    this.project_resize =
                                        Some((e.position.x, this.workspace.project_rail_width));
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            )
                            .on_click(cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                                if event.click_count() == 2 {
                                    this.workspace.project_rail_width = 220.;
                                    this.fit_sidebar(cx);
                                    this.persist(cx);
                                    this.invalidate_chrome(cx);
                                    cx.notify();
                                }
                            })),
                    )
                },
            )
            .into_any_element();
        if cfg!(target_os = "macos")
            && !self.workspace.project_rail_open
            && self.settings.is_none()
            && self.classic_surface_open(cx)
        {
            // Destination headers live beside the rail. Keep its divider below
            // the traffic lights, at the same height as the sessions view.
            div()
                .w(px(width))
                .h_full()
                .flex_shrink_0()
                .flex()
                .flex_col()
                .child(
                    titlebar_drag_area()
                        .w_full()
                        .h(px(TITLE_BAR_HEIGHT))
                        .flex_shrink_0()
                        .border_b_1()
                        .border_color(content(0.07)),
                )
                .child(div().flex_1().min_h_0().child(frame))
                .into_any_element()
        } else {
            frame
        }
    }
    pub(crate) fn project_rail(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let compact = !self.workspace.project_rail_open && self.settings.is_none();
        let header = titlebar_drag_area().h(px(TITLE_BAR_HEIGHT)).flex_shrink_0();
        if self.settings.is_some() {
            let nav = self.sidebar(cx);
            return div()
                .size_full()
                .flex()
                .flex_col()
                .child(header)
                .child(nav)
                .into_any();
        }
        if compact {
            return div()
                .id("project-rail")
                .debug_selector(|| "project-rail".into())
                .role(Role::Navigation)
                .aria_label("Project shortcuts")
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .overflow_hidden()
                .when(!cfg!(target_os = "macos"), |d| d.child(header))
                .child(self.project_rail_compact_actions(cx))
                .child(div().min_h(px(8.)).flex_1())
                .child(
                    div().w_full().py(px(6.)).flex().justify_center().child(
                        icon_button("rail-settings", "Settings", "settings")
                            .size(px(32.))
                            .on_click(cx.listener(|this, _, w, cx| {
                                this.open_settings(crate::settings::Section::General, w, cx)
                            })),
                    ),
                )
                .into_any();
        }
        self.prepare_project_rail(compact);
        let owner = cx.weak_entity();
        let rows = list(self.project_rail_list.state.clone(), move |ix, _, cx| {
            owner
                .update(cx, |this, cx| this.project_rail_list_row(ix, compact, cx))
                .unwrap_or_else(|_| div().into_any())
        })
        .size_full();
        div()
            .id("project-rail")
            .debug_selector(|| "project-rail".into())
            .role(Role::Navigation)
            .aria_label("Projects")
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .when(!self.classic_titlebar_spans_window(cx), |d| d.child(header))
            .when(!compact, |d| d.child(self.original_rail_navigation(cx)))
            .child(
                div()
                    .id("project-rail-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(rows),
            )
            .children(self.live_agents_preview(cx))
            .child(self.app_update_footer(cx))
            .child(self.project_rail_footer(compact, cx))
            .into_any_element()
    }
    fn prepare_project_rail(&mut self, compact: bool) {
        let mut rows = Vec::new();
        if self.workspace.preferences.monos {
            rows.push(ProjectRailRow::Monos);
        }
        let projects = &self.workspace.projects;
        let pinned: Vec<_> = projects
            .iter()
            .filter(|p| self.workspace.project_rail_pinned.contains(*p))
            .cloned()
            .collect();
        if !pinned.is_empty() {
            if !compact {
                rows.push(ProjectRailRow::Heading("Pinned"));
            }
            rows.extend(pinned.into_iter().map(|path| ProjectRailRow::Project {
                path,
                grouped: false,
            }));
            rows.push(ProjectRailRow::Gap);
        }
        if !self.workspace.project_rail_groups.is_empty() {
            if !compact {
                rows.push(ProjectRailRow::Heading("Groups"));
            }
            for group in &self.workspace.project_rail_groups {
                let items: Vec<_> = projects
                    .iter()
                    .filter(|p| {
                        !self.workspace.project_rail_pinned.contains(*p)
                            && self.workspace.project_rail_assignments.get(*p) == Some(&group.id)
                    })
                    .cloned()
                    .collect();
                if !compact {
                    rows.push(ProjectRailRow::Group(group.id.clone()));
                }
                if compact || !group.collapsed {
                    rows.extend(items.into_iter().map(|path| ProjectRailRow::Project {
                        path,
                        grouped: !compact,
                    }));
                }
            }
            rows.push(ProjectRailRow::Gap);
        }
        rows.push(ProjectRailRow::Heading("Projects"));
        let mut count = 0;
        for path in projects {
            let grouped = self
                .workspace
                .project_rail_assignments
                .get(path)
                .is_some_and(|id| {
                    self.workspace
                        .project_rail_groups
                        .iter()
                        .any(|g| &g.id == id)
                });
            if !self.workspace.project_rail_pinned.contains(path) && !grouped {
                count += 1;
                rows.push(ProjectRailRow::Project {
                    path: path.clone(),
                    grouped: false,
                });
            }
        }
        if count == 0 && self.workspace.project_rail_groups.is_empty() {
            rows.push(ProjectRailRow::Empty);
        }
        rows.push(ProjectRailRow::Gap);
        self.project_rail_list.update(rows, compact);
    }
    pub(crate) fn reveal_active_project_rail(&self) {
        if let Some(ix) = self.project_rail_list.rows.iter().position(
            |row| matches!(row,ProjectRailRow::Project {path,..} if path==&self.workspace.project),
        ) {
            self.project_rail_list.state.scroll_to_reveal_item(ix);
        }
    }
    pub(crate) fn toggle_project_rail_pin(&mut self, path: &str, cx: &mut Context<Self>) {
        if !self.workspace.project_rail_pinned.remove(path) {
            self.workspace.project_rail_pinned.insert(path.into());
        }
        self.persist(cx);
        self.projects_frame.update(cx, |_, cx| cx.notify());
    }
}

#[cfg(test)]
mod tests;
