mod dock;
mod drag;
mod view;
// The dock's tab strip (ProjectTerminalDock.tsx over SurfaceTabs.tsx) and the
// footer's terminal controls (UsageFooter.tsx's Terminal button and
// RunningTerminalChip).
use super::{DragSource, MenuTarget, Shell, TabShift, panes::drag::PaneDrag};
use crate::{
    app::workspace::{DockSide, TerminalDock},
    terminal::Terminal,
    ui::{
        popover::{Anchor, MenuAction, MenuItem, MenuStyle},
        project_badge::project_name,
        theme::*,
    },
};
use gpui::{
    AnyElement, App, Bounds, Context, Entity, EntityId, MouseButton, MouseDownEvent,
    MouseMoveEvent, Pixels, Point, Role, SharedString, StyleRefinement, Task, Window, div, point,
    prelude::*, px, rgb,
};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

/// useAnimatedReorder's drag of one dock tab along the strip.
pub(super) struct DockTabDrag {
    /// Where the press landed; pulling far enough off the strip turns the
    /// drag into a pane drag that can drop the terminal in a split or on
    /// the title strip.
    start: Point<Pixels>,
    from: usize,
    to: usize,
    origin: Pixels,
    offset: Pixels,
    active: bool,
    bounds: Vec<Bounds<Pixels>>,
}

/// How far off the dock's strip a tab must be pulled before it can leave
/// the dock.
const DOCK_PULL: Pixels = px(24.);

/// A terminal whose foreground process is not the shell (listRunningTerminals).
struct Running {
    ix: usize,
    process: String,
    label: String,
}

/// TerminalLiveMark: three bars that light in turn over 3.2s.
pub(super) struct LiveMark {
    epoch: Instant,
    _tick: Task<()>,
}
const LIVE_CYCLE: f32 = 3.2;
const LIVE_COLOR: u32 = 0xe39b4a;
impl LiveMark {
    fn new(cx: &mut Context<Self>) -> Self {
        let tick = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs_f32(LIVE_CYCLE / 4.))
                    .await;
                // Each step redraws the whole window; hold the mark while
                // nobody is looking at it or motion is reduced.
                let still =
                    cx.update(|cx| cx.reduce_motion() || !crate::ui::motion::pulse::app_active(cx));
                if !still && this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        Self {
            epoch: Instant::now(),
            _tick: tick,
        }
    }
}
impl Render for LiveMark {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let phase = (self.epoch.elapsed().as_secs_f32() / LIVE_CYCLE).fract();
        let reduced = cx.reduce_motion();
        div()
            .flex()
            .items_end()
            .gap(px(2.))
            .h(px(10.))
            .children((1..=3).map(|bar| {
                let opacity = if reduced {
                    0.7
                } else if phase >= bar as f32 * 0.25 {
                    0.85
                } else {
                    0.4
                };
                div()
                    .w(px(4.))
                    .h(px(8.))
                    .bg(rgb(LIVE_COLOR))
                    .opacity(opacity)
            }))
    }
}

/// runningTerminalChipLabel: `vite`, or `vite · jest`, or `vite ×2`.
fn chip_label<'a>(processes: impl IntoIterator<Item = &'a str>) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for process in processes {
        match counts.iter_mut().find(|(name, _)| *name == process) {
            Some((_, n)) => *n += 1,
            None => counts.push((process, 1)),
        }
    }
    counts
        .into_iter()
        .map(|(name, n)| {
            if n > 1 {
                format!("{name} ×{n}")
            } else {
                name.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

impl Shell {
    fn live_terminals(&self) -> Vec<Entity<Terminal>> {
        self.terminals
            .get(&self.workspace.project)
            .cloned()
            .unwrap_or_default()
    }
    fn running_terminals(&self, cx: &App) -> Vec<Running> {
        self.live_terminals()
            .iter()
            .enumerate()
            .filter_map(|(ix, terminal)| {
                let t = terminal.read(cx);
                let process = t.foreground.as_deref()?.trim();
                (!process.is_empty()).then(|| Running {
                    ix,
                    process: process.to_owned(),
                    label: project_name(&t.cwd),
                })
            })
            .collect()
    }
    fn dock_open(&self, cx: &App) -> bool {
        if self.terminal_in_panel(cx) {
            return self.panel_terminal_shown(cx);
        }
        self.workspace
            .docks
            .get(&self.workspace.project)
            .is_some_and(|d| d.open && !d.terminals.is_empty())
    }
    /// The new layout can host the terminal as the right panel's last tab
    /// instead of the dock, wherever that panel can open.
    pub(super) fn terminal_in_panel(&self, cx: &App) -> bool {
        self.workspace.preferences.terminal_placement() == "panel"
            && !self.workspace.classic_layout
            && !self.workspace.project.starts_with("remote://")
            && !self.mono_active(cx)
            && self.git.read(cx).available()
    }
    /// Terminals open as their own title bar tabs; the dock stays for
    /// actions and the footer button.
    pub(super) fn terminal_in_title(&self) -> bool {
        self.workspace.preferences.terminal_placement() == "title"
            && !self.workspace.project.starts_with("remote://")
    }
    pub(super) fn panel_terminal_shown(&self, cx: &App) -> bool {
        self.workspace.right_panel_open
            && !self.app_view_open()
            && self
                .right_panel
                .as_ref()
                .is_some_and(|p| p.read(cx).tab() == crate::ui::panel::RightTab::Terminal)
    }
    /// The panel's Terminal tab always has a shell to show.
    pub(super) fn ensure_panel_terminal(&mut self, cx: &mut Context<Self>) {
        let project = self.workspace.project.clone();
        self.ensure_terminals(&project, cx);
        if self
            .workspace
            .docks
            .get(&project)
            .is_some_and(|d| !d.terminals.is_empty())
        {
            return;
        }
        let cwd = self.terminal_cwd(cx);
        let dock = self.workspace.docks.entry(project.clone()).or_default();
        dock.add(cwd.clone());
        // The dock stays shut until the terminal moves back out of the panel.
        dock.open = false;
        let terminal = self.spawn_terminal(cwd, cx);
        self.terminals.entry(project).or_default().push(terminal);
        self.persist(cx);
    }
    /// Opens the right panel on its Terminal tab and focuses the shell.
    pub(super) fn show_panel_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_right_panel(window, cx);
        self.ensure_panel_terminal(cx);
        if let Some(panel) = self.right_panel.clone() {
            panel.update(cx, |p, cx| {
                p.set_terminal_tab(true, cx);
                p.select(crate::ui::panel::RightTab::Terminal, cx);
            });
        }
        self.set_right_panel_open(true, window, cx);
        self.focus_terminal(window, cx);
        self.invalidate_chrome(cx);
        cx.notify();
    }
    /// The panel's Terminal tab: the dock's header and its active terminal.
    pub(super) fn panel_terminal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let project = self.workspace.project.clone();
        // Never render the tab without a shell, whatever emptied the dock.
        self.ensure_panel_terminal(cx);
        let Some(dock) = self.workspace.docks.get(&project).cloned() else {
            return div().into_any_element();
        };
        // The boxy frame's title bar sits over the panel; clear it as the
        // other tabs do.
        let top_inset = self
            .right_panel
            .as_ref()
            .map_or(0., |p| p.read(cx).top_inset());
        div()
            .id("panel-terminal")
            .debug_selector(|| "panel-terminal".into())
            .role(Role::Region)
            .aria_label("Terminal")
            .size_full()
            .flex()
            .flex_col()
            .when(top_inset > 0., |d| {
                d.child(
                    div()
                        .h(px(top_inset))
                        .flex_shrink_0()
                        .border_b_1()
                        .border_color(crate::ui::panel::stroke()),
                )
            })
            .child(self.dock_header(&dock, true, window, cx))
            .child(
                div().flex_1().min_h_0().min_w_0().children(
                    self.terminals
                        .get(&project)
                        .and_then(|t| t.get(dock.active))
                        // The grid shapes every line; redraw it only when the
                        // terminal itself changes.
                        .map(|t| t.clone().cached(StyleRefinement::default().size_full())),
                ),
            )
            .into_any_element()
    }
    /// onToggleRunningTerminal: hide the open dock, else show it on `ix`.
    fn toggle_running_terminal(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.dock_open(cx)
            && let Some(dock) = self.workspace.docks.get_mut(&self.workspace.project)
        {
            dock.active = ix.min(dock.terminals.len().saturating_sub(1));
            self.dock_tab_scroll.scroll_to_item(dock.active);
        }
        self.toggle_terminal(window, cx);
    }
    pub(super) fn running_terminals_menu(&self, cx: &App) -> Vec<MenuItem> {
        self.running_terminals(cx)
            .into_iter()
            .map(|r| {
                MenuAction::new(SharedString::from(r.ix.to_string()), r.process)
                    .value(r.label)
                    .into()
            })
            .collect()
    }
    pub(super) fn run_running_terminals_menu(
        &mut self,
        action: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Ok(ix) = action.parse() {
            self.toggle_running_terminal(ix, window, cx);
        }
    }
    pub(super) fn terminal_tab_menu(&self, ix: usize) -> Vec<MenuItem> {
        let count = self
            .workspace
            .docks
            .get(&self.workspace.project)
            .map_or(0, |d| d.terminals.len());
        vec![
            MenuAction::new("close", "Close").into(),
            MenuAction::new("close-others", "Close Others")
                .disabled(count < 2 || ix >= count)
                .into(),
        ]
    }
    pub(super) fn run_terminal_tab_menu(
        &mut self,
        ix: usize,
        action: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            "close" => self.close_terminal(ix, window, cx),
            "close-others" => self.close_other_terminals(ix, window, cx),
            _ => {}
        }
    }
    /// confirmCloseTerminals: the terminals still running something ask first.
    fn close_other_terminals(&mut self, keep: usize, window: &mut Window, cx: &mut Context<Self>) {
        let project = self.workspace.project.clone();
        let live = self.live_terminals();
        let Some(kept) = live.get(keep).cloned() else {
            return;
        };
        let running: Vec<(String, String)> = live
            .iter()
            .enumerate()
            .filter(|(ix, _)| *ix != keep)
            .filter_map(|(_, terminal)| {
                let t = terminal.read(cx);
                let process = t.foreground.as_deref()?.trim().to_owned();
                (!process.is_empty()).then(|| (t.title.clone(), process))
            })
            .collect();
        let message = match running.as_slice() {
            [] => None,
            [(label, process)] => Some(format!(
                "\"{process}\" is still running in {label}. Close this terminal anyway?"
            )),
            many => Some(format!(
                "These terminals are still running:\n{}\n\nClose them anyway?",
                many.iter()
                    .map(|(label, process)| format!("• {label} ({process})"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )),
        };
        let Some(message) = message else {
            self.keep_terminal(&project, &kept, window, cx);
            return;
        };
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            &message,
            None,
            &["Close", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if matches!(answer.await, Ok(0)) {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.keep_terminal(&project, &kept, window, cx)
                });
            }
        })
        .detach();
    }
    fn keep_terminal(
        &mut self,
        project: &str,
        kept: &Entity<Terminal>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self
            .terminals
            .get(project)
            .and_then(|t| t.iter().position(|e| e == kept))
        else {
            return;
        };
        let had_focus = self.dock_focused(window, cx).is_some();
        if let Some(live) = self.terminals.get_mut(project) {
            live.retain(|e| e == kept);
        }
        if let Some(dock) = self.workspace.docks.get_mut(project) {
            dock.keep_only(ix);
        }
        self.dock_shifts.clear();
        if had_focus {
            self.focus_terminal(window, cx);
        }
        self.persist(cx);
        cx.notify();
    }
    pub(super) fn terminal_menu_items(
        &self,
        target: &MenuTarget,
        cx: &App,
    ) -> Option<(SharedString, Vec<MenuItem>, f32, MenuStyle)> {
        match target {
            MenuTarget::TerminalTab(ix) => Some((
                "File tab actions".into(),
                self.terminal_tab_menu(*ix),
                160.,
                MenuStyle::Explorer,
            )),
            MenuTarget::RunningTerminals => Some((
                "Running terminals".into(),
                self.running_terminals_menu(cx),
                192.,
                MenuStyle::Explorer,
            )),
            _ => None,
        }
    }
}

/// The space between the footer's right-hand buttons: the resource chip,
/// Actions, and Terminal or the running-terminals chip.
pub(super) const FOOTER_GAP: f32 = 4.;

/// A short hairline between footer controls: High usage, actions, Terminal.
pub(super) fn footer_separator() -> gpui::Div {
    div()
        .debug_selector(|| "footer-separator".into())
        .flex_shrink_0()
        .w(px(1.))
        .h(px(12.))
        .mx(px(3.))
        .rounded_full()
        .bg(content(0.12))
}
const NEW_TERMINAL_LABEL: &str = if cfg!(target_os = "macos") {
    "New Terminal (⌘`)"
} else {
    "New Terminal (Ctrl+`)"
};
const HIDE_TERMINAL_LABEL: &str = if cfg!(target_os = "macos") {
    "Hide Terminal (⌘J)"
} else {
    "Hide Terminal (Ctrl+J)"
};

#[cfg(test)]
mod tests;
