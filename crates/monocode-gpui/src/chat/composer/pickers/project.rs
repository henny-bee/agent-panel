use super::*;

impl ProjectPicker {
    pub(crate) fn new(projects: &[String], current: &str, cx: &mut Context<Self>) -> Self {
        let search = search_field("Search projects", "Search projects...", 0.35, cx);
        let changed = cx.subscribe(&search, |this, _, _: &input::Changed, cx| {
            this.active = 0;
            cx.notify();
        });
        let mut projects: Vec<String> = projects.to_vec();
        if let Some(ix) = projects.iter().position(|p| p == current) {
            let first = projects.remove(ix);
            projects.insert(0, first);
        }
        Self {
            appearance: Default::default(),
            projects,
            monos: Vec::new(),
            active_mono: None,
            current: current.into(),
            active: 0,
            allow_remote: false,
            search,
            _changed: changed,
        }
    }
    pub(crate) fn search_focus(&self, cx: &gpui::App) -> FocusHandle {
        self.search.focus_handle(cx)
    }
    pub(crate) fn allow_remote(mut self) -> Self {
        self.allow_remote = true;
        self
    }
    pub(crate) fn appearance(
        mut self,
        appearance: std::collections::HashMap<
            String,
            monocode_core::project::appearance::ProjectAppearance,
        >,
    ) -> Self {
        self.appearance = appearance;
        self
    }
    pub(crate) fn monos(
        mut self,
        monos: Vec<PickerMono>,
        active: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        self.active_mono = active.filter(|id| monos.iter().any(|mono| &mono.id == id));
        if !monos.is_empty() {
            self.search.update(cx, |search, cx| {
                search.set_placeholder("Search monos and projects...", cx)
            });
        }
        self.monos = monos;
        self
    }
    pub(super) fn matching_monos(&self, cx: &gpui::App) -> Vec<&PickerMono> {
        let query = self.search.read(cx).content.trim().to_lowercase();
        self.monos
            .iter()
            .filter(|mono| mono.name.to_lowercase().contains(&query))
            .collect()
    }
    pub(super) fn label(&self, path: &str) -> String {
        self.appearance
            .get(path)
            .map_or_else(|| project_name(path), |a| a.label(path))
    }
    pub(super) fn matches(&self, cx: &gpui::App) -> Vec<String> {
        let query = self.search.read(cx).content.to_lowercase();
        self.projects
            .iter()
            .filter(|path| {
                query.is_empty()
                    || format!("{}\n{}", self.label(path), path)
                        .to_lowercase()
                        .contains(&query)
            })
            .cloned()
            .collect()
    }
    pub(super) fn step(&mut self, direction: isize, cx: &mut Context<Self>) {
        let count = self.matching_monos(cx).len() + self.matches(cx).len();
        if count > 0 {
            self.active = (self.active as isize + direction).clamp(0, count as isize - 1) as usize;
            cx.notify();
        }
    }
    pub(super) fn pick(&mut self, ix: usize, cx: &mut Context<Self>) {
        let monos = self.matching_monos(cx);
        if let Some(mono) = monos.get(ix) {
            cx.emit(ProjectPicked::Mono(mono.id.clone()));
        } else if let Some(path) = self
            .matches(cx)
            .into_iter()
            .nth(ix.saturating_sub(monos.len()))
        {
            cx.emit(ProjectPicked::Project(path));
        }
    }
}

impl Render for ProjectPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let matches = self.matches(cx);
        let focused = self.search.focus_handle(cx).is_focused(window);
        let matching_monos = self.matching_monos(cx);
        let mono_count = matching_monos.len();
        let mut rows: Vec<AnyElement> = matching_monos
            .into_iter()
            .enumerate()
            .map(|(ix, mono)| {
                let active = self.active == ix;
                let current = self.active_mono.as_ref() == Some(&mono.id);
                div()
                    .id(("project-picker-mono", ix))
                    .debug_selector({
                        let id = mono.id.clone();
                        move || format!("project-picker-mono-{id}")
                    })
                    .role(Role::ListBoxOption)
                    .aria_label(format!("{}, Mono", mono.name))
                    .aria_selected(current)
                    .cursor_pointer()
                    .flex_shrink_0()
                    .h(px(36.))
                    .px(px(10.))
                    .gap(px(10.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .text_color(content(if active { 1. } else { 0.75 }))
                    .when(active, |d| d.bg(content(0.1)))
                    .when(!active, |d| {
                        d.hover(|s| s.bg(content(0.05)).text_color(content(1.)))
                    })
                    .child(
                        div()
                            .size(px(16.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(if current {
                                icon("check-2", 14.)
                                    .text_color(content(1.))
                                    .into_any_element()
                            } else {
                                crate::ui::avatar::avatar(
                                    &mono.mascot,
                                    mono.color,
                                    "idle",
                                    16.,
                                    true,
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(mono.name.clone()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(11.))
                            .text_color(content(0.4))
                            .child("Mono"),
                    )
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered {
                            this.active = ix;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| this.pick(ix, cx)))
                    .into_any_element()
            })
            .collect();
        rows.extend(matches.into_iter().enumerate().map(|(project_ix, path)| {
            let ix = mono_count + project_ix;
            let active = self.active == ix;
            let current = self.active_mono.is_none() && path == self.current;
            div()
                .id(("project-option", ix))
                .debug_selector(move || format!("project-picker-project-{project_ix}"))
                .role(Role::ListBoxOption)
                .aria_label(self.label(&path))
                .aria_description(path.clone())
                .aria_selected(current)
                .cursor_pointer()
                .flex_shrink_0()
                .h(px(36.))
                .px(px(10.))
                .gap(px(10.))
                .rounded(px(8.))
                .flex()
                .items_center()
                .text_color(content(if active { 1. } else { 0.75 }))
                .when(active, |d| d.bg(content(0.1)))
                .when(!active, |d| {
                    d.hover(|s| s.bg(content(0.05)).text_color(content(1.)))
                })
                .child(
                    div()
                        .size(px(16.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(if current {
                            icon("check-2", 14.)
                                .text_color(content(1.))
                                .into_any_element()
                        } else {
                            if path.starts_with("remote://") {
                                icon("globe", 14.)
                                    .text_color(content(0.6))
                                    .into_any_element()
                            } else {
                                crate::ui::project_badge::project_icon(
                                    &self.appearance.get(&path).cloned().unwrap_or_default(),
                                    &path,
                                    14.,
                                )
                            }
                        }),
                )
                .child(
                    div()
                        .min_w_0()
                        .flex_shrink_0()
                        .max_w(px(286. - 12. - 20. - 36.))
                        .truncate()
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(self.label(&path)),
                )
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .max_w(px(112.))
                        .truncate()
                        .font_family("Menlo")
                        .text_size(px(11.))
                        .text_color(content(0.4))
                        .child(pretty_parent(&path)),
                )
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered {
                        this.active = ix;
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, _, _, cx| this.pick(ix, cx)))
                .into_any()
        }));
        let empty = rows.is_empty();
        div()
            .id("project-picker")
            .debug_selector(|| "project-picker".into())
            .role(Role::Dialog)
            .aria_label("Project picker")
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .capture_action(cx.listener(|this, _: &input::Up, _, cx| {
                this.step(-1, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &input::Down, _, cx| {
                this.step(1, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &input::Submit, _, cx| {
                this.pick(this.active, cx);
                cx.stop_propagation();
            }))
            .child(
                div()
                    .h(px(44.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px_3()
                    .border_b_1()
                    .border_color(content(0.07))
                    .text_color(content(1.))
                    .child(icon("search", 16.).text_color(content(if focused {
                        0.7
                    } else {
                        0.45
                    })))
                    .child(div().flex_1().min_w_0().child(self.search.clone())),
            )
            .child(
                div()
                    .id("project-list")
                    .role(Role::ListBox)
                    .aria_label(if self.monos.is_empty() {
                        "Projects"
                    } else {
                        "Monos and projects"
                    })
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(6.))
                    .flex()
                    .flex_col()
                    .when(empty, |d| {
                        d.child(
                            div()
                                .px(px(10.))
                                .py(px(20.))
                                .flex()
                                .justify_center()
                                .text_size(px(12.))
                                .text_color(content(0.45))
                                .child(if self.monos.is_empty() {
                                    "No projects found"
                                } else {
                                    "No monos or projects found"
                                }),
                        )
                    })
                    .children(rows),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(content(0.07))
                    .p(px(6.))
                    .child(
                        div()
                            .id("project-new")
                            .role(Role::Button)
                            .aria_label("New project")
                            .group("project-new")
                            .cursor_pointer()
                            .h(px(36.))
                            .px(px(10.))
                            .gap(px(10.))
                            .rounded(px(8.))
                            .flex()
                            .items_center()
                            .text_size(px(13.))
                            .text_color(content(0.75))
                            .hover(|s| s.bg(content(0.08)).text_color(content(1.)))
                            .child(
                                icon("plus", 16.)
                                    .text_color(content(0.75))
                                    .group_hover("project-new", |s| s.text_color(content(1.))),
                            )
                            .child("New project")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ProjectPicked::New))),
                    )
                    .when(self.allow_remote, |d| {
                        d.child(
                            div()
                                .id("project-remote")
                                .role(Role::Button)
                                .aria_label("Open folder on a machine")
                                .cursor_pointer()
                                .h(px(36.))
                                .px(px(10.))
                                .gap(px(10.))
                                .rounded(px(8.))
                                .flex()
                                .items_center()
                                .text_size(px(13.))
                                .text_color(content(0.75))
                                .hover(|s| s.bg(content(0.08)).text_color(content(1.)))
                                .child(icon("globe", 16.).text_color(content(0.75)))
                                .child("Open folder on a machine…")
                                .on_click(
                                    cx.listener(|_, _, _, cx| cx.emit(ProjectPicked::Remote)),
                                ),
                        )
                    }),
            )
    }
}
