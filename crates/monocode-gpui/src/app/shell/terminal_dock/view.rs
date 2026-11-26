use super::*;

/// Where a tab is drawn now, easing toward `target` (the title strip's
/// `eased_shift`).
pub(super) fn eased_shift(
    shifts: &mut HashMap<EntityId, TabShift>,
    id: EntityId,
    target: f32,
    follow: bool,
    reduced: bool,
) -> (f32, bool) {
    let now = Instant::now();
    if follow || reduced {
        shifts.remove(&id);
        return (target, false);
    }
    let current = shifts.get(&id).map(|s| s.at(now));
    if current.is_none_or(|_| shifts[&id].to != target) {
        let from = current.map_or(0., |(value, _)| value);
        if from == target {
            shifts.remove(&id);
            return (target, false);
        }
        shifts.insert(
            id,
            TabShift {
                from,
                to: target,
                start: now,
            },
        );
    }
    let (value, done) = shifts[&id].at(now);
    if done && target == 0. {
        shifts.remove(&id);
    }
    (value, !done)
}

impl Shell {
    /// SurfaceTabs for the dock's terminals, then New, Move and Hide.
    /// `in_panel` drops the controls that only make sense for the dock.
    pub(in crate::app::shell) fn dock_header(
        &mut self,
        dock: &TerminalDock,
        in_panel: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let live = self.live_terminals();
        let hide = match dock.side {
            DockSide::Bottom => "chevron-down",
            DockSide::Top => "chevron-up",
            DockSide::Left => "chevron-left",
            DockSide::Right => "chevron-right",
        };
        let side = match dock.side {
            DockSide::Bottom => "panel-bottom",
            DockSide::Top => "panel-top",
            DockSide::Left => "panel-left",
            DockSide::Right => "panel-right",
        };
        let reduced = cx.reduce_motion();
        let dragged = self.dock_drag.as_ref().filter(|d| d.active).map(|d| d.from);
        let mut moving = false;
        let shifts: Vec<f32> = live
            .iter()
            .enumerate()
            .map(|(ix, terminal)| {
                let target = self.dock_tab_shift(ix).as_f32();
                let (value, active) = eased_shift(
                    &mut self.dock_shifts,
                    terminal.entity_id(),
                    target,
                    dragged == Some(ix),
                    reduced,
                );
                moving |= active;
                value
            })
            .collect();
        let animate = self.workspace.preferences.tab_animations && !reduced && dragged.is_none();
        let keyed = live
            .iter()
            .map(|t| (t.entity_id(), SharedString::from(t.read(cx).title.clone())))
            .collect();
        let (slots, opening) = self
            .dock_tab_motion
            .sync(keyed, animate, std::time::Instant::now());
        if moving || opening {
            window.request_animation_frame();
        }
        let move_anchor = self.move_terminal_anchor.clone();
        div()
            .h(px(36.))
            .flex_shrink_0()
            .min_w_0()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(crate::ui::panel::stroke())
            .child(
                div()
                    .id("terminal-tabs")
                    .role(Role::TabList)
                    .aria_label("Terminals")
                    .track_scroll(&self.dock_tab_scroll)
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .pl(px(6.))
                    .pr(px(10.))
                    // Every tab fits, sharing the strip however many there are.
                    .overflow_hidden()
                    .children({
                        let mut tabs: Vec<Option<AnyElement>> = live
                            .iter()
                            .enumerate()
                            .map(|(ix, terminal)| {
                                let t = terminal.read(cx);
                                let label: SharedString = t.title.clone().into();
                                let agent = t.agent();
                                let description =
                                    SharedString::from(format!("{} — {}", t.title, t.cwd));
                                let active = ix == dock.active;
                                let held = dragged == Some(ix);
                                let hovered = self.dock_hover == Some(ix) && dragged.is_none();
                                div()
                                    .id(("terminal-tab", ix))
                                    .debug_selector(move || format!("terminal-tab-{ix}"))
                                    .relative()
                                    .left(px(shifts[ix]))
                                    .when(held, |d| d.cursor_grabbing())
                                    .h_full()
                                    .w(px(224.))
                                    .min_w_0()
                                    .flex_shrink(1.)
                                    .flex()
                                    .items_center()
                                    .on_hover(cx.listener(move |this, hovering: &bool, _, cx| {
                                        let next = if *hovering {
                                            Some(ix)
                                        } else if this.dock_hover == Some(ix) {
                                            None
                                        } else {
                                            this.dock_hover
                                        };
                                        if next != this.dock_hover {
                                            this.dock_hover = next;
                                            cx.notify();
                                        }
                                    }))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(
                                            move |this, event: &MouseDownEvent, window, cx| {
                                                this.start_dock_drag(ix, event, window, cx)
                                            },
                                        ),
                                    )
                                    .on_mouse_down(
                                        MouseButton::Middle,
                                        cx.listener(move |this, _, window, cx| {
                                            this.close_terminal(ix, window, cx)
                                        }),
                                    )
                                    .on_mouse_down(
                                        MouseButton::Right,
                                        cx.listener(
                                            move |this, event: &MouseDownEvent, window, cx| {
                                                cx.stop_propagation();
                                                this.select_terminal(ix, window, cx);
                                                this.open_menu(
                                                    MenuTarget::TerminalTab(ix),
                                                    Anchor::Point(event.position),
                                                    window,
                                                    cx,
                                                );
                                            },
                                        ),
                                    )
                                    .child(
                                        div()
                                            .id(("terminal-tab-button", ix))
                                            .role(Role::Tab)
                                            .aria_label(label.clone())
                                            .aria_description(description)
                                            .aria_selected(active)
                                            .h(px(TAB_PLATE_HEIGHT))
                                            .min_w_0()
                                            .flex_1()
                                            .flex()
                                            .items_center()
                                            .gap(px(crate::ui::tabs::tile::TAB_TILE_GAP))
                                            .pl(px(crate::ui::theme::tab_tile_inset()))
                                            .pr(px(28.))
                                            .rounded_md()
                                            .text_size(px(13.))
                                            .map(|d| {
                                                if active {
                                                    d.bg(crate::ui::panel::selection())
                                                        .text_color(content(1.))
                                                } else if hovered {
                                                    d.bg(content(0.05)).text_color(content(1.))
                                                } else {
                                                    d.text_color(content(0.5))
                                                }
                                            })
                                            .child(crate::ui::tabs::tile::terminal_tile(agent))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .child(label.clone()),
                                            )
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                if std::mem::take(&mut this.dock_skip_click) {
                                                    return;
                                                }
                                                this.focus_terminal(window, cx);
                                            })),
                                    )
                                    .child(
                                        div()
                                            .id(("terminal-tab-close", ix))
                                            .role(Role::Button)
                                            .aria_label(SharedString::from(format!(
                                                "Close {label}"
                                            )))
                                            .group("terminal-tab-close")
                                            .absolute()
                                            .right_1()
                                            .top(px(8.))
                                            .size(px(20.))
                                            .rounded(px(4.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .opacity(if (active || hovered) && !held {
                                                1.
                                            } else {
                                                0.
                                            })
                                            .hover(|s| s.bg(content(0.1)))
                                            .child(
                                                icon("x", 12.)
                                                    .text_color(content(0.5))
                                                    .group_hover("terminal-tab-close", |s| {
                                                        s.text_color(content(1.))
                                                    }),
                                            )
                                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                cx.stop_propagation()
                                            })
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                cx.stop_propagation();
                                                this.close_terminal(ix, window, cx);
                                            })),
                                    )
                                    .into_any_element()
                            })
                            .map(Some)
                            .collect();
                        let ids: Vec<_> = live.iter().map(|t| t.entity_id()).collect();
                        // TabWidthMotion: `w-56` eases in from nothing, or out to it.
                        let slot = |open: f32| {
                            div()
                                .h_full()
                                .w(px(224. * open))
                                .min_w_0()
                                .flex_shrink(1.)
                                .mr(px(-crate::ui::tabs::motion::GAP * (1. - open)))
                                .overflow_hidden()
                                .flex()
                        };
                        slots
                            .into_iter()
                            .filter_map(move |s| match s {
                                crate::ui::tabs::motion::Slot::Live { key, open } => {
                                    let ix = ids.iter().position(|id| *id == key)?;
                                    let tab = tabs.get_mut(ix)?.take()?;
                                    Some(if open >= 1. {
                                        tab
                                    } else {
                                        slot(open).child(tab).into_any_element()
                                    })
                                }
                                crate::ui::tabs::motion::Slot::Closing { ghost, open, .. } => Some(
                                    slot(open)
                                        .child(
                                            div()
                                                .w(px(224.))
                                                .h_full()
                                                .flex_shrink_0()
                                                .flex()
                                                .items_center()
                                                .child(
                                                    div()
                                                        .h(px(TAB_PLATE_HEIGHT))
                                                        .w_full()
                                                        .flex()
                                                        .items_center()
                                                        .gap(px(
                                                            crate::ui::tabs::tile::TAB_TILE_GAP,
                                                        ))
                                                        .pl(px(crate::ui::theme::tab_tile_inset()))
                                                        .pr_2()
                                                        .rounded_md()
                                                        .text_size(px(13.))
                                                        .text_color(content(0.5))
                                                        .child(
                                                            crate::ui::tabs::tile::terminal_tile(
                                                                crate::terminal::agent_cli(&ghost),
                                                            ),
                                                        )
                                                        .child(
                                                            div().min_w_0().truncate().child(ghost),
                                                        ),
                                                ),
                                        )
                                        .into_any_element(),
                                ),
                            })
                            .collect::<Vec<_>>()
                    })
                    // In the panel, New follows the tabs as the title bar's does.
                    .when(in_panel, |d| {
                        d.child(
                            div()
                                .id("new-terminal")
                                .debug_selector(|| "new-terminal".into())
                                .group("new-terminal")
                                .role(Role::Button)
                                .aria_label(NEW_TERMINAL_LABEL)
                                .cursor_pointer()
                                .size(px(TAB_PLATE_HEIGHT))
                                .flex_shrink_0()
                                .ml(px(2.))
                                .rounded_md()
                                .bg(content(0.04))
                                .opacity(0.65)
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(|s| s.opacity(1.).bg(content(0.07)))
                                .child(
                                    icon("plus", 14.)
                                        .group_hover("new-terminal", |s| s.text_color(content(1.))),
                                )
                                .on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.new_terminal(window, cx)
                                    }),
                                ),
                        )
                    }),
            )
            .when(!in_panel, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap(px(2.))
                        .pr(px(6.))
                        .child(
                            icon_button("new-terminal", NEW_TERMINAL_LABEL, "plus").on_click(
                                cx.listener(|this, _, window, cx| this.new_terminal(window, cx)),
                            ),
                        )
                        .child(
                            icon_button("move-terminal", "Move Terminal", side)
                                .child(move_anchor.probe())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    // ExplorerMenu hangs from the button's bottom-left corner.
                                    let at = this
                                        .move_terminal_anchor
                                        .get()
                                        .map(|b| point(b.left(), b.bottom() + px(4.)))
                                        .unwrap_or_default();
                                    this.open_menu(
                                        MenuTarget::TerminalDock,
                                        Anchor::Point(at),
                                        window,
                                        cx,
                                    );
                                })),
                        )
                        .child(
                            icon_button("hide-terminal", HIDE_TERMINAL_LABEL, hide).on_click(
                                cx.listener(|this, _, window, cx| this.toggle_terminal(window, cx)),
                            ),
                        ),
                )
            })
            .into_any_element()
    }

    /// Chat | Terminal for the active local Claude session: the same
    /// conversation shown as a chat or as the Claude Code CLI.
    fn claude_mode_switch(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let chat = self.sessions.get(&self.workspace.active?)?.clone();
        let (terminal, blocked) = {
            let view = chat.read(cx);
            (view.claude_mode(cx)?, view.claude_mode_blocked())
        };
        let classic = self.workspace.boxy_layout();
        let outer = if classic { 6. } else { 10. };
        let part = |id: &'static str, label: &'static str, on: bool, to_terminal: bool| {
            let chat = chat.clone();
            div()
                .id(id)
                .debug_selector(move || id.into())
                .role(Role::Button)
                .aria_label(label)
                .aria_toggled(if on {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .h_full()
                .px(px(if classic { 5. } else { 7. }))
                .flex()
                .items_center()
                .rounded(px(outer - 2.))
                .text_color(content(if on { 0.9 } else { 0.4 }))
                .when(on, |d| d.bg(content(0.1)))
                .when(!on && blocked.is_none(), |d| {
                    d.cursor_pointer()
                        .hover(|s| s.bg(content(0.06)).text_color(content(0.8)))
                        .on_click(move |_, _, cx| {
                            chat.update(cx, |chat, cx| chat.switch_claude_mode(to_terminal, cx))
                        })
                })
                .child(label)
        };
        let tip: SharedString = blocked
            .unwrap_or(if terminal {
                "Open this session as a chat. Stops Claude Code in this tab first."
            } else {
                "Open this session in Claude Code. The conversation carries over."
            })
            .into();
        Some(
            div()
                .id("footer-claude-mode")
                .role(Role::Group)
                .aria_label("Show Claude as")
                .h(px(20.))
                .p(px(2.))
                .gap(px(1.))
                .flex()
                .flex_shrink_0()
                .items_stretch()
                .whitespace_nowrap()
                .rounded(px(outer))
                .bg(content(0.04))
                .when(blocked.is_some(), |d| d.opacity(0.6))
                .tooltip(move |_, cx| {
                    cx.new(|_| super::super::sidebar::project_rail::ProjectTooltip(tip.to_string()))
                        .into()
                })
                .child(part("footer-claude-chat", "Chat", !terminal, false))
                .child(part("footer-claude-terminal", "Terminal", terminal, true))
                .into_any_element(),
        )
    }

    /// UsageFooter's right side: the running-terminal chip, else the
    /// Terminal button.
    pub(in crate::app::shell) fn terminal_footer(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.app_view_open() || self.workspace.project.is_empty() {
            self.live_mark = None;
            return None;
        }
        let running = self.running_terminals(cx);
        let open = self.dock_open(cx);
        let classic = self.workspace.boxy_layout();
        let actions = self.actions_button(cx);
        let mode = self.claude_mode_switch(cx);
        let has_actions = actions.is_some() || mode.is_some();
        // The classic footer's own `px-3` and border hold these in place.
        let group = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(FOOTER_GAP))
            .when(!classic, |d| d.mr(px(6.)).pb(px(2.)))
            .text_size(px(11.))
            .when_some(mode, |d, mode| {
                d.child(mode)
                    .when(actions.is_some(), |d| d.child(footer_separator()))
            })
            .children(actions);
        // The right panel's Terminal tab, or title bar terminal tabs, take
        // over the footer's terminal controls.
        if self.terminal_in_panel(cx) || self.terminal_in_title() {
            self.live_mark = None;
            return Some(group.into_any_element());
        }
        if running.is_empty() {
            self.live_mark = None;
            let has_terminals = self
                .workspace
                .docks
                .get(&self.workspace.project)
                .is_some_and(|d| !d.terminals.is_empty());
            let label: SharedString = if has_terminals {
                "Terminal".into()
            } else {
                NEW_TERMINAL_LABEL.into()
            };
            return Some(
                group
                    .when(has_actions, |d| d.child(footer_separator()))
                    .child(
                        div()
                            .id("footer-terminal")
                            .debug_selector(|| "footer-terminal".into())
                            .role(Role::Button)
                            .aria_label(label)
                            .aria_toggled(if open {
                                gpui::Toggled::True
                            } else {
                                gpui::Toggled::False
                            })
                            .h(px(20.))
                            .map(|d| {
                                if classic {
                                    d.px(px(6.)).rounded_md()
                                } else {
                                    d.px_2().rounded_full()
                                }
                            })
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap(px(6.))
                            .whitespace_nowrap()
                            .map(|d| {
                                if open {
                                    d.text_color(accent_color()).hover(|s| s.bg(content(0.1)))
                                } else {
                                    d.text_color(content(0.4))
                                        .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
                                }
                            })
                            .child(icon("terminal", 14.))
                            .child("Terminal")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.toggle_terminal(window, cx)),
                            ),
                    )
                    .into_any_element(),
            );
        }
        let mark = self
            .live_mark
            .get_or_insert_with(|| cx.new(LiveMark::new))
            .clone();
        let many = running.len() > 1;
        let aria = match (running.as_slice(), open) {
            ([one], true) => format!("Hide {}", one.process),
            ([one], false) => format!("Show {}", one.process),
            (_, true) => "Hide running terminals".into(),
            (all, false) => format!("{} terminals are running processes", all.len()),
        };
        let description = running
            .iter()
            .map(|r| format!("\"{}\" in {}", r.process, r.label))
            .collect::<Vec<_>>()
            .join("\n");
        let first = running[0].ix;
        let label = chip_label(running.iter().map(|r| r.process.as_str()));
        let anchor = self.terminal_chip_anchor.clone();
        Some(
            group
                .when(has_actions, |d| d.child(footer_separator()))
                .child(
                    div()
                        .id("footer-running-terminals")
                        .role(Role::Button)
                        .aria_label(aria)
                        .aria_description(description)
                        .aria_toggled(if open { gpui::Toggled::True } else { gpui::Toggled::False })
                        .when(many && !open, |d| {
                            d.aria_expanded(
                                matches!(&self.menu, Some(m) if m.target == MenuTarget::RunningTerminals),
                            )
                        })
                        .relative()
                        .map(|d| {
                            // `-mx-1` pulls the hover wash to the footer's edge;
                            // the left pull would close the gap after Actions.
                            if classic {
                                d.mr(px(-4.)).px(px(4.)).rounded_md()
                            } else {
                                d.h(px(20.)).px_2().rounded_full()
                            }
                        })
                        .max_w(px(256.))
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .whitespace_nowrap()
                        .text_color(content(0.55))
                        .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
                        .child(anchor.probe())
                        .child(
                            mark.cached(StyleRefinement::default().w(px(16.)).h(px(10.)).flex_shrink_0()),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_family(crate::ui::fonts::mono())
                                .text_size(px(10.))
                                .child(label),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if this.dock_open(cx) || !many {
                                this.toggle_running_terminal(first, window, cx);
                            } else if matches!(&this.menu, Some(m) if m.target == MenuTarget::RunningTerminals) {
                                this.close_menu(cx);
                            } else {
                                let anchor = Anchor::Slot(this.terminal_chip_anchor.clone());
                                this.open_menu(MenuTarget::RunningTerminals, anchor, window, cx);
                            }
                        })),
                )
                .into_any_element(),
        )
    }
}
