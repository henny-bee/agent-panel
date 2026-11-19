use super::super::*;

impl Shell {
    /// newTerminalCwd: the focused session's worktree, else its folder, else
    /// the project.
    pub(in crate::app::shell) fn terminal_cwd(&self, cx: &App) -> String {
        self.workspace
            .active
            .and_then(|id| self.sessions.get(&id))
            .map(|view| {
                let view = view.read(cx);
                view.settings
                    .worktree_cwd
                    .clone()
                    .unwrap_or_else(|| view.summary.cwd.clone())
            })
            .unwrap_or_else(|| self.working_copy().to_owned())
    }

    pub(in crate::app::shell) fn spawn_terminal(
        &mut self,
        cwd: String,
        cx: &mut Context<Self>,
    ) -> Entity<Terminal> {
        let terminal = cx.new(|cx| Terminal::new(cwd, cx));
        self.observations
            .push(cx.subscribe(&terminal, |_, _, _: &TerminalEvent, cx| cx.notify()));
        terminal
    }

    /// A restored dock reopens its tabs as fresh shells once it is shown.
    pub(in crate::app::shell) fn ensure_terminals(
        &mut self,
        project: &str,
        cx: &mut Context<Self>,
    ) {
        if project.starts_with("remote://") {
            return;
        }
        let Some(dock) = self.workspace.docks.get(project) else {
            return;
        };
        let wanted = dock.terminals.clone();
        let have = self.terminals.get(project).map_or(0, Vec::len);
        for cwd in wanted.into_iter().skip(have) {
            let terminal = self.spawn_terminal(cwd, cx);
            self.terminals
                .entry(project.to_owned())
                .or_default()
                .push(terminal);
        }
    }

    /// The focused terminal of the current project's dock.
    pub(in crate::app::shell) fn dock_focused(&self, window: &Window, cx: &App) -> Option<usize> {
        self.terminals
            .get(&self.workspace.project)?
            .iter()
            .position(|t| t.focus_handle(cx).contains_focused(window, cx))
    }

    pub(in crate::app::shell) fn focus_terminal(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project = &self.workspace.project;
        if let Some(dock) = self.workspace.docks.get(project)
            && let Some(terminal) = self.terminals.get(project).and_then(|t| t.get(dock.active))
        {
            let focus = terminal.focus_handle(cx);
            window.focus(&focus, cx);
        }
    }

    pub(in crate::app::shell) fn slide_dock(&mut self, open: bool, cx: &mut Context<Self>) {
        let project = self.workspace.project.clone();
        let Some(size) = self.workspace.docks.get(&project).map(|d| d.size) else {
            return;
        };
        if cx.reduce_motion() {
            self.dock_slide = None;
            return;
        }
        let to = if open { 1. } else { 0. };
        self.dock_slide = Some((
            project.clone(),
            match self.dock_slide.take() {
                Some((p, slide)) if p == project => slide.retarget(to, std::time::Instant::now()),
                _ => PaneSlide::new(1. - to, to, size),
            },
        ));
    }

    /// Terminal: New (cmd-`): another terminal in the project's dock, which
    /// opens if it was hidden.
    pub(in crate::app::shell) fn new_terminal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace.project.starts_with("remote://") {
            self.error = Some("Terminals are not available on remote projects.".into());
            cx.notify();
            return;
        }
        if self.app_view_open() {
            return;
        }
        let project = self.workspace.project.clone();
        let cwd = self.terminal_cwd(cx);
        let was_open = self
            .workspace
            .docks
            .get(&project)
            .is_some_and(|d| d.open && !d.terminals.is_empty());
        self.ensure_terminals(&project, cx);
        let viewport = self.viewport();
        let dock = self.workspace.docks.entry(project.clone()).or_default();
        dock.size = dock.side.clamp(dock.size, viewport);
        let added = dock.add(cwd.clone());
        self.dock_tab_scroll.scroll_to_item(added);
        let terminal = self.spawn_terminal(cwd, cx);
        self.terminals.entry(project).or_default().push(terminal);
        if self.terminal_in_panel(cx) {
            if let Some(dock) = self.workspace.docks.get_mut(&self.workspace.project) {
                dock.open = false;
            }
            self.show_panel_terminal(window, cx);
            self.persist(cx);
            return;
        }
        if !was_open {
            self.slide_dock(true, cx);
        }
        self.focus_terminal(window, cx);
        self.fit_sidebar(cx);
        self.persist(cx);
        cx.notify();
    }

    /// Terminal: Toggle (cmd-j).
    pub(in crate::app::shell) fn toggle_terminal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace.project.starts_with("remote://") {
            self.error = Some("Terminals are not available on remote projects.".into());
            cx.notify();
            return;
        }
        if self.terminal_in_panel(cx) {
            if self.panel_terminal_shown(cx) {
                self.set_right_panel_open(false, window, cx);
            } else {
                self.show_panel_terminal(window, cx);
            }
            return;
        }
        let project = self.workspace.project.clone();
        let Some(dock) = self
            .workspace
            .docks
            .get_mut(&project)
            .filter(|d| !d.terminals.is_empty())
        else {
            self.new_terminal(window, cx);
            return;
        };
        dock.open = !dock.open;
        let open = dock.open;
        self.slide_dock(open, cx);
        if open {
            self.ensure_terminals(&project, cx);
            self.focus_terminal(window, cx);
        } else if self.dock_focused(window, cx).is_some() {
            self.focus_active(window, cx);
        }
        self.fit_sidebar(cx);
        self.persist(cx);
        cx.notify();
    }

    pub(in crate::app::shell) fn select_terminal(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(dock) = self.workspace.docks.get_mut(&self.workspace.project) {
            dock.active = ix.min(dock.terminals.len().saturating_sub(1));
            self.dock_tab_scroll.scroll_to_item(dock.active);
        }
        self.focus_terminal(window, cx);
        self.persist(cx);
        cx.notify();
    }

    /// confirmCloseTerminal: a terminal running something asks first.
    pub(in crate::app::shell) fn close_terminal(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project = self.workspace.project.clone();
        let Some(terminal) = self
            .terminals
            .get(&project)
            .and_then(|t| t.get(ix))
            .cloned()
        else {
            return;
        };
        let (process, label) = {
            let t = terminal.read(cx);
            (t.foreground.clone(), project_name(&t.cwd))
        };
        // The panel's last terminal stays: closing it hides the panel, and
        // stops whatever it was running.
        if self.terminal_in_panel(cx) && self.terminals.get(&project).is_some_and(|t| t.len() == 1)
        {
            let Some(process) = process else {
                self.set_right_panel_open(false, window, cx);
                return;
            };
            let answer = window.prompt(
                gpui::PromptLevel::Warning,
                &format!("\"{process}\" is still running in {label}. Stop it and close the panel?"),
                None,
                &["Stop", "Cancel"],
                cx,
            );
            cx.spawn_in(window, async move |this, cx| {
                if matches!(answer.await, Ok(0)) {
                    let _ = this.update_in(cx, |this, window, cx| {
                        terminal.update(cx, |t, cx| t.stop_foreground(cx));
                        this.set_right_panel_open(false, window, cx);
                    });
                }
            })
            .detach();
            return;
        }
        let Some(process) = process else {
            self.remove_terminal(&project, ix, window, cx);
            return;
        };
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            &format!("\"{process}\" is still running in {label}. Close this terminal anyway?"),
            None,
            &["Close", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if matches!(answer.await, Ok(0)) {
                let _ = this.update_in(cx, |this, window, cx| {
                    if let Some(ix) = this
                        .terminals
                        .get(&project)
                        .and_then(|t| t.iter().position(|e| *e == terminal))
                    {
                        this.remove_terminal(&project, ix, window, cx);
                    }
                });
            }
        })
        .detach();
    }

    pub(in crate::app::shell) fn remove_terminal(
        &mut self,
        project: &str,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let had_focus = self.dock_focused(window, cx).is_some();
        if let Some(live) = self.terminals.get_mut(project)
            && ix < live.len()
        {
            live.remove(ix);
        }
        let Some(dock) = self.workspace.docks.get_mut(project) else {
            return;
        };
        if !dock.close(ix) {
            // The last terminal closes the dock; it slides shut empty.
            dock.open = false;
            self.slide_dock(false, cx);
            if had_focus {
                self.focus_active(window, cx);
            }
            self.fit_sidebar(cx);
        } else if had_focus {
            self.focus_terminal(window, cx);
        }
        self.persist(cx);
        cx.notify();
    }

    pub(in crate::app::shell) fn set_dock_side(&mut self, side: DockSide, cx: &mut Context<Self>) {
        let viewport = self.viewport();
        if let Some(dock) = self.workspace.docks.get_mut(&self.workspace.project) {
            dock.set_side(side);
            dock.size = side.clamp(dock.size, viewport);
        }
        self.dock_slide = None;
        self.fit_sidebar(cx);
        self.persist(cx);
        cx.notify();
    }

    pub(in crate::app::shell) fn viewport(&self) -> (f32, f32) {
        (self.viewport_width, self.viewport_height)
    }

    pub(in crate::app::shell) fn drag_dock(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        let Some((start, size)) = self.dock_resize else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.release_resizes(cx);
            return;
        }
        let Some(dock) = self.workspace.docks.get(&self.workspace.project) else {
            return;
        };
        let at = if dock.side.vertical() {
            event.position.y.as_f32()
        } else {
            event.position.x.as_f32()
        };
        let delta = at - start;
        let signed = if matches!(dock.side, DockSide::Bottom | DockSide::Right) {
            -delta
        } else {
            delta
        };
        let target = ResizeTarget::Dock(self.workspace.project.clone());
        self.drag_resize(target, size + signed, cx);
    }

    /// App.tsx's dock grid: the project's terminal dock on its side of the
    /// workspace. Opening or closing slides the track while the terminal
    /// keeps its full size and rides the moving edge, so it fits once.
    pub(in crate::app::shell) fn with_dock(
        &mut self,
        body: AnyElement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let project = self.workspace.project.clone();
        // Other projects' docks stop polling for their foreground commands.
        // A hidden dock keeps polling for the footer's running-terminal chip.
        for (path, live) in &self.terminals {
            let shown = *path == project && !self.app_view_open();
            for terminal in live {
                terminal.update(cx, |t, _| t.visible = shown);
            }
        }
        // The right panel's Terminal tab hosts the shells instead.
        if self.terminal_in_panel(cx) {
            return body;
        }
        let Some(dock) = self.workspace.docks.get(&project).cloned() else {
            return body;
        };
        let progress = match self.dock_slide.as_ref().filter(|(p, _)| *p == project) {
            Some((_, slide)) => {
                let (t, done) = slide.at(std::time::Instant::now());
                if done {
                    self.dock_slide = None;
                    None
                } else {
                    window.request_animation_frame();
                    Some(t)
                }
            }
            None => None,
        };
        let shown = (dock.open || progress.is_some()) && !self.app_view_open();
        if !shown {
            return body;
        }
        self.ensure_terminals(&project, cx);
        let size = dock.size;
        let track = size * progress.unwrap_or(1.);
        let vertical = dock.side.vertical();
        let leading = matches!(dock.side, DockSide::Top | DockSide::Left);
        let surface = div()
            .absolute()
            .flex()
            .flex_col()
            .map(|d| {
                if vertical {
                    d.left_0().w_full().h(px(size))
                } else {
                    d.top_0().h_full().w(px(size))
                }
            })
            .map(|d| match dock.side {
                DockSide::Top => d.bottom_0(),
                DockSide::Left => d.right_0(),
                DockSide::Bottom => d.top_0(),
                DockSide::Right => d.left_0(),
            })
            .child(self.dock_header(&dock, false, window, cx))
            .child(
                div().flex_1().min_h_0().min_w_0().children(
                    self.terminals
                        .get(&project)
                        .and_then(|t| t.get(dock.active))
                        // The grid shapes every line; redraw it only when the
                        // terminal itself changes.
                        .map(|t| t.clone().cached(StyleRefinement::default().size_full())),
                ),
            );
        let sash = div()
            .id("terminal-dock-sash")
            .debug_selector(|| "terminal-dock-sash".into())
            .group("resize-handle")
            .role(Role::Splitter)
            .aria_label("Resize terminal")
            .absolute()
            .child(resize_border(!vertical, self.dock_resize.is_some()))
            .map(|d| match dock.side {
                DockSide::Bottom => d.bottom(px(track - 3.5)).left_0().right_0().h(px(6.)),
                DockSide::Top => d.top(px(track - 3.5)).left_0().right_0().h(px(6.)),
                DockSide::Left => d.left(px(track - 3.5)).top_0().bottom_0().w(px(6.)),
                DockSide::Right => d.right(px(track - 3.5)).top_0().bottom_0().w(px(6.)),
            })
            .cursor(if vertical {
                CursorStyle::ResizeUpDown
            } else {
                CursorStyle::ResizeLeftRight
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    // A double click restores the side's default size.
                    if event.click_count == 2 {
                        if let Some(dock) = this.workspace.docks.get_mut(&this.workspace.project) {
                            dock.size = dock.side.default_size();
                        }
                        this.persist(cx);
                        cx.notify();
                        return;
                    }
                    let at = if vertical {
                        event.position.y.as_f32()
                    } else {
                        event.position.x.as_f32()
                    };
                    this.dock_resize = Some((at, size));
                }),
            );
        let frame = div()
            .id("terminal-dock")
            .debug_selector(|| "terminal-dock".into())
            .role(Role::Region)
            .aria_label("Terminal")
            .relative()
            .flex_shrink_0()
            .overflow_hidden()
            .border_color(crate::ui::panel::stroke())
            .map(|d| match dock.side {
                DockSide::Bottom => d.border_t_1(),
                DockSide::Top => d.border_b_1(),
                DockSide::Left => d.border_r_1(),
                DockSide::Right => d.border_l_1(),
            })
            .map(|d| {
                if vertical {
                    d.w_full().h(px(track))
                } else {
                    d.h_full().w(px(track))
                }
            })
            .child(surface);
        let main = div().flex_1().min_w_0().min_h_0().child(body);
        div()
            .size_full()
            .relative()
            .flex()
            .when(vertical, |d| d.flex_col())
            .map(|d| {
                if leading {
                    d.child(frame).child(main)
                } else {
                    d.child(main).child(frame)
                }
            })
            // Paint the full hit area above both surfaces, outside the dock's clip.
            .when(progress.is_none(), |d| d.child(sash))
            .into_any_element()
    }
}
