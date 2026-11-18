//! Split panes each carry their own project panel beside them. While a tab is
//! split, the title bar's change count, file search and panel toggle move
//! into every pane's header, and the window-wide panel steps aside.
use super::*;
use crate::{git_status::ProjectGit, right_panel::RightTab};

/// Room the session keeps beside its pane's panel.
const PANE_MIN: f32 = 200.;

impl Shell {
    /// Whether the active tab is split with a panel per pane: the default
    /// layout only, and not over a Mono or an app view.
    pub(super) fn split_panels(&self, cx: &App) -> bool {
        !self.workspace.classic_layout
            && !self.workspace.boxy_layout()
            && !self.app_view_open()
            && !self.mono_active(cx)
            && self.workspace.active.is_some_and(|id| {
                let tab = self.workspace.tab_of(id).unwrap_or(id);
                self.workspace.layout(tab).is_split()
            })
    }

    fn pane_panel_open(&self, id: i64) -> bool {
        self.workspace.pane_panels.get(&id).is_some_and(|p| p.open)
    }

    fn ensure_pane_panel(
        &mut self,
        id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<RightPanel> {
        if let Some(panel) = self.pane_panels.get(&id) {
            return panel.clone();
        }
        let tab = self
            .workspace
            .pane_panels
            .get(&id)
            .map_or(RightTab::Files, |p| p.tab);
        let git = self.pane_git(id, cx);
        let panel = self.new_project_panel(tab, Some(id), git, window, cx);
        self.pane_panels.insert(id, panel.clone());
        panel
    }

    /// What a pane's panel shows: a folder picked there, else its chat's
    /// working copy.
    fn pane_folder(&self, id: i64, cx: &App) -> String {
        if let Some(folder) = self.workspace.panel_folders.get(&id) {
            return folder.clone();
        }
        let session = self.workspace.session_for_pane(id);
        if self.sessions.contains_key(&session) || self.session_summaries.contains_key(&session) {
            self.session_working_copy(id, cx).to_owned()
        } else {
            self.working_copy().to_owned()
        }
    }

    /// Each pane reads Git for its own folder, so two panes can show two
    /// repositories side by side.
    fn pane_git(&mut self, id: i64, cx: &mut Context<Self>) -> Entity<ProjectGit> {
        let folder = self.pane_folder(id, cx);
        let git = self
            .pane_gits
            .entry(id)
            .or_insert_with(|| cx.new(|cx| ProjectGit::new(folder.clone(), cx)))
            .clone();
        git.update(cx, |git, cx| git.set_project(folder, cx));
        git
    }

    pub(super) fn toggle_pane_panel(
        &mut self,
        id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let open = !self.pane_panel_open(id);
        self.workspace.pane_panels.entry(id).or_default().open = open;
        if open {
            let panel = self.ensure_pane_panel(id, window, cx);
            panel.update(cx, |p, cx| p.focus_pane(window, cx));
        } else {
            self.focus_pane(id, window, cx);
        }
        self.persist(cx);
        cx.notify();
    }

    /// A pane's change count opens its panel on Changes.
    fn show_pane_changes(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.ensure_pane_panel(id, window, cx);
        self.workspace.pane_panels.entry(id).or_default().open = true;
        panel.update(cx, |p, cx| {
            p.select(RightTab::Changes, cx);
            p.focus_pane(window, cx);
        });
        self.persist(cx);
        cx.notify();
    }

    /// Panels read and poll only while on screen; the window panel's
    /// Explorer follows the active session.
    pub(super) fn sync_pane_panels(&mut self, cx: &mut Context<Self>) {
        let split = self.split_panels(cx);
        let leaves: Vec<i64> = self
            .workspace
            .active
            .map(|id| {
                let tab = self.workspace.tab_of(id).unwrap_or(id);
                self.workspace
                    .layout(tab)
                    .leaves()
                    .iter()
                    .map(|l| l.id)
                    .collect()
            })
            .unwrap_or_default();
        for (id, panel) in &self.pane_panels {
            let shown = split && self.pane_panel_open(*id) && leaves.contains(id);
            panel.update(cx, |p, cx| p.set_shown(shown, cx));
        }
        // Every split pane's header counts its own folder's changes.
        if split {
            for id in &leaves {
                if !self.workspace.mono_panes.contains_key(id) {
                    self.pane_git(*id, cx);
                }
            }
        }
        let active = self.window_active;
        for (id, git) in &self.pane_gits {
            let polling = active && split && leaves.contains(id);
            git.update(cx, |git, cx| {
                git.set_polling(polling && git.available(), cx)
            });
        }
        if let Some(panel) = &self.right_panel {
            let activity = self.claude_activity(self.workspace.active, cx);
            let custom = self
                .workspace
                .active
                .is_some_and(|id| self.workspace.panel_folders.contains_key(&id));
            let explorer = panel.read(cx).explorer.clone();
            explorer.update(cx, |e, cx| {
                e.set_claude(activity, cx);
                e.set_custom_folder(custom, cx);
            });
        }
    }

    fn claude_activity(
        &self,
        id: Option<i64>,
        cx: &mut Context<Self>,
    ) -> Option<Entity<crate::claude_panels::ClaudeActivity>> {
        let session = id.and_then(|id| self.sessions.get(&id)).cloned()?;
        session.update(cx, |session, cx| session.claude_activity(cx))
    }

    /// The pane's width on screen, while the tab is split.
    fn pane_width(&self, id: i64) -> Option<f32> {
        let tab = self.workspace.tab_of(id).unwrap_or(id);
        let leaf = self
            .workspace
            .layout(tab)
            .leaves()
            .into_iter()
            .find(|l| l.id == id)?;
        Some(leaf.rect.w * self.pane_bounds.get()?.size.width.as_f32())
    }

    fn pane_panel_width(&self, id: i64) -> f32 {
        let saved = self
            .workspace
            .pane_panels
            .get(&id)
            .map_or(crate::right_panel::WIDTH_DEFAULT, |p| p.width);
        let room = self.pane_width(id).map_or(saved, |w| w - PANE_MIN);
        saved
            .min(room)
            .max(crate::right_panel::WIDTH_MIN.min(room))
            .max(0.)
    }

    /// The panel beside a split pane, when it is open.
    pub(super) fn pane_panel_frame(
        &mut self,
        id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.split_panels(cx)
            || self.workspace.mono_panes.contains_key(&id)
            || !self.pane_panel_open(id)
            || !self.pane_git(id, cx).read(cx).available()
        {
            return None;
        }
        let custom = self.workspace.panel_folders.contains_key(&id);
        let width = self.pane_panel_width(id);
        let panel = self.ensure_pane_panel(id, window, cx);
        let activity = self.claude_activity(Some(id), cx);
        let show_excluded = self.workspace.preferences.show_excluded_files;
        panel.update(cx, |p, cx| {
            p.set_tab_width(width - 12., cx);
            p.set_show_excluded(show_excluded, cx);
            p.explorer.update(cx, |e, cx| {
                e.set_claude(activity, cx);
                e.set_custom_folder(custom, cx);
            });
        });
        let tabs = panel.read(cx).tabs.clone();
        let resizing = self
            .pane_panel_resize
            .is_some_and(|(pane, _, _)| pane == id);
        Some(
            div()
                .id(("pane-panel", id as usize))
                .debug_selector(move || format!("pane-panel-{id}"))
                .relative()
                .h_full()
                .w(px(width))
                .flex_shrink_0()
                .flex()
                .flex_col()
                .border_l_1()
                .border_color(crate::right_panel::stroke())
                .child(
                    // Level with the pane's own header.
                    div()
                        .h(px(36.))
                        .flex_shrink_0()
                        .px_1p5()
                        .overflow_hidden()
                        .flex()
                        .items_center()
                        .border_b_1()
                        .border_color(crate::right_panel::stroke())
                        .child(
                            tabs.cached(
                                StyleRefinement::default().w_full().h(px(TITLE_BAR_HEIGHT)),
                            ),
                        ),
                )
                .child(panel.cached(StyleRefinement::default().flex_1().min_h_0().w_full()))
                .child(
                    div()
                        .id(("pane-panel-resize", id as usize))
                        .role(Role::Splitter)
                        .aria_label("Resize panel")
                        .group("resize-handle")
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(px(-4.))
                        .w(px(8.))
                        .cursor(CursorStyle::ResizeLeftRight)
                        .child(resize_border(true, resizing))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                let width = this.pane_panel_width(id);
                                this.pane_panel_resize = Some((id, event.position.x, width));
                                cx.notify();
                            }),
                        ),
                )
                .into_any_element(),
        )
    }

    pub(super) fn drag_pane_panel(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some((id, start, width)) = self.pane_panel_resize else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.release_resizes(cx);
            return;
        }
        let max = self
            .pane_width(id)
            .map_or(crate::right_panel::WIDTH_MAX, |w| w - PANE_MIN)
            .min(crate::right_panel::WIDTH_MAX);
        let to = (width - (event.position.x - start).as_f32())
            .clamp(
                crate::right_panel::WIDTH_MIN,
                max.max(crate::right_panel::WIDTH_MIN),
            )
            .round();
        self.workspace.pane_panels.entry(id).or_default().width = to;
        cx.notify();
    }

    /// The title bar's change count, file search and panel toggle, for one
    /// pane's header while the tab is split.
    pub(super) fn pane_header_controls(&self, id: i64, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if !self.split_panels(cx) || self.workspace.mono_panes.contains_key(&id) {
            return Vec::new();
        }
        let mut controls = Vec::new();
        // The pane's own Git once the tab has drawn split; the window's before.
        let git = self.pane_gits.get(&id).unwrap_or(&self.git);
        let available = git.read(cx).available();
        let open = self.pane_panel_open(id);
        if available {
            let stats = git.read(cx).stats.clone();
            let showing_changes = open
                && self
                    .pane_panels
                    .get(&id)
                    .is_some_and(|p| p.read(cx).tab() == RightTab::Changes);
            controls.push(
                crate::right_panel::diff_stat(
                    stats.as_ref(),
                    crate::right_panel::diff_stat_width(stats.as_ref()),
                )
                .when(showing_changes, |d| d.bg(content(0.1)))
                .cursor_pointer()
                .hover(|s| s.bg(content(0.1)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.focus_pane(id, window, cx);
                    this.show_pane_changes(id, window, cx);
                }))
                .into_any_element(),
            );
        }
        let search = if cfg!(target_os = "macos") {
            "Search files (⌘P)"
        } else {
            "Search files (Ctrl+P)"
        };
        controls.push(
            header_button(format!("pane-file-search-{id}"), search, "search", false)
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.focus_pane(id, window, cx);
                    this.open_quick_open(false, window, cx);
                }))
                .into_any_element(),
        );
        if available {
            controls.push(
                header_button(
                    format!("pane-panel-toggle-{id}"),
                    "Toggle panel",
                    "sidebar-right",
                    open,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.toggle_pane_panel(id, window, cx);
                }))
                .into_any_element(),
            );
        }
        controls
    }
}

/// A pane header's 20px icon button, as its split buttons draw.
fn header_button(
    key: String,
    label: &'static str,
    glyph: &'static str,
    active: bool,
) -> gpui::Stateful<gpui::Div> {
    let group: SharedString = key.clone().into();
    div()
        .id(SharedString::from(key.clone()))
        .debug_selector(move || key.clone())
        .role(Role::Button)
        .aria_label(label)
        .when(glyph == "sidebar-right", |d| {
            d.aria_toggled(if active {
                gpui::Toggled::True
            } else {
                gpui::Toggled::False
            })
        })
        .group(group.clone())
        .size(px(20.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .cursor_pointer()
        .hover(|s| s.bg(content(0.1)))
        .when(active, |d| d.bg(content(0.08)))
        .child(
            icon(glyph, 13.)
                .text_color(content(if active { 1. } else { 0.5 }))
                .group_hover(group, |s| s.text_color(content(1.))),
        )
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{pane_layout::SplitDir, shell::tests::window};
    use gpui::{TestAppContext, VisualTestContext};

    #[gpui::test]
    fn split_panes_carry_the_corner_controls_and_their_own_panels(cx: &mut TestAppContext) {
        let (window, _, claude) = window(cx);
        cx.update(|cx| cx.set_reduce_motion(true));
        let other = window
            .update(cx, |shell, w, cx| {
                shell.open(claude, w, cx);
                shell.toggle_right_panel(w, cx);
                assert!(shell.workspace.right_panel_open);
                let summary = locked(&shell.store)
                    .unwrap()
                    .create("codex", "/work/monocode")
                    .unwrap();
                let other = summary.id;
                shell.add(summary, cx);
                shell.workspace.split(other, SplitDir::Right);
                shell.open(claude, w, cx);
                assert!(shell.split_panels(cx));
                // While split, the panel shortcut serves the focused pane.
                shell.toggle_right_panel(w, cx);
                assert!(shell.workspace.right_panel_open);
                assert!(shell.workspace.pane_panels[&claude].open);
                assert!(!shell.pane_panel_open(other));
                shell
                    .pane_panels
                    .get(&claude)
                    .unwrap()
                    .read(cx)
                    .explorer
                    .clone()
                    .update(cx, |e, cx| {
                        e.set_claude_tab(Some(crate::claude_panels::ClaudeTab::Agents), cx)
                    });
                shell.workspace.pane_panels.get_mut(&claude).unwrap().claude =
                    Some(crate::claude_panels::ClaudeTab::Agents);
                other
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        })
        .unwrap();
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        // The title bar's corner steps aside; each pane header carries it.
        assert!(visual.debug_bounds("toggle-right-panel").is_none());
        assert!(visual.debug_bounds("right-panel-frame").is_none());
        for pane in [claude, other] {
            let toggle = Box::leak(format!("pane-panel-toggle-{pane}").into_boxed_str());
            assert!(visual.debug_bounds(toggle).is_some());
        }
        let own = Box::leak(format!("pane-panel-{claude}").into_boxed_str());
        let closed = Box::leak(format!("pane-panel-{other}").into_boxed_str());
        assert!(visual.debug_bounds(own).is_some());
        assert!(visual.debug_bounds(closed).is_none());
        // Only the Claude pane's Explorer offers the agent views.
        assert!(visual.debug_bounds("explorer-claude-agents").is_some());
        assert!(visual.debug_bounds("explorer-claude-chat").is_some());
        // Collapse All over an agent view brings the tree (and its folder box) back.
        assert!(visual.debug_bounds("explorer-folder-pick").is_none());
        assert!(visual.debug_bounds("explorer-new-file").is_none());
        let collapse = visual.debug_bounds("explorer-collapse").unwrap().center();
        visual.simulate_click(collapse, gpui::Modifiers::none());
        visual
            .update_window(window.into(), |_, window, cx| {
                window.simulate_next_frame(cx);
                window.draw(cx).clear(cx);
            })
            .unwrap();
        assert!(visual.debug_bounds("explorer-folder-pick").is_some());
        window
            .update(visual, |shell, _, _| {
                assert_eq!(shell.workspace.pane_panels[&claude].claude, None)
            })
            .unwrap();
        // Back to one pane, the corner returns.
        window
            .update(visual, |shell, w, cx| {
                shell.close_pane(other, w, cx);
                assert!(!shell.split_panels(cx));
            })
            .unwrap();
        visual.run_until_parked();
        visual
            .update_window(window.into(), |_, window, cx| {
                window.simulate_next_frame(cx);
                window.draw(cx).clear(cx);
            })
            .unwrap();
        assert!(visual.debug_bounds("toggle-right-panel").is_some());
    }
}

#[cfg(test)]
mod folder_tests {
    use super::*;
    use crate::{explorer::FolderChosen, pane_layout::SplitDir, shell::tests::window};
    use gpui::TestAppContext;

    fn choose(explorer: &Entity<crate::explorer::Explorer>, folder: Option<&str>, cx: &mut App) {
        explorer.update(cx, |_, cx| cx.emit(FolderChosen(folder.map(str::to_owned))));
    }

    #[gpui::test]
    fn a_picked_folder_drives_its_panel_and_is_remembered_per_chat(cx: &mut TestAppContext) {
        let (window, codex, claude) = window(cx);
        cx.update(|cx| cx.set_reduce_motion(true));
        window
            .update(cx, |shell, w, cx| {
                shell.open(claude, w, cx);
                shell.ensure_right_panel(w, cx);
                let explorer = shell
                    .right_panel
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .explorer
                    .clone();
                choose(&explorer, Some("/work/picked"), cx);
            })
            .unwrap();
        cx.run_until_parked();
        let other = window
            .update(cx, |shell, w, cx| {
                assert_eq!(shell.workspace.panel_folders[&claude], "/work/picked");
                assert_eq!(shell.git.read(cx).project(), "/work/picked");
                // Another chat shows its own folder; coming back restores the pick.
                shell.open(codex, w, cx);
                assert_eq!(shell.git.read(cx).project(), "/work/another-project");
                shell.open(claude, w, cx);
                assert_eq!(shell.git.read(cx).project(), "/work/picked");
                // Split, each pane reads its own folder.
                let summary = locked(&shell.store)
                    .unwrap()
                    .create("codex", "/work/monocode")
                    .unwrap();
                let other = summary.id;
                shell.add(summary, cx);
                shell.workspace.split(other, SplitDir::Right);
                shell.open(claude, w, cx);
                shell.sync_pane_panels(cx);
                assert_eq!(shell.pane_gits[&claude].read(cx).project(), "/work/picked");
                assert_eq!(shell.pane_gits[&other].read(cx).project(), "/work/monocode");
                let explorer = shell
                    .ensure_pane_panel(other, w, cx)
                    .read(cx)
                    .explorer
                    .clone();
                choose(&explorer, Some("/work/second"), cx);
                let explorer = shell
                    .ensure_pane_panel(claude, w, cx)
                    .read(cx)
                    .explorer
                    .clone();
                choose(&explorer, None, cx);
                other
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                shell.sync_pane_panels(cx);
                assert!(!shell.workspace.panel_folders.contains_key(&claude));
                assert_eq!(
                    shell.pane_gits[&claude].read(cx).project(),
                    "/work/monocode"
                );
                assert_eq!(shell.pane_gits[&other].read(cx).project(), "/work/second");
            })
            .unwrap();
    }
}
