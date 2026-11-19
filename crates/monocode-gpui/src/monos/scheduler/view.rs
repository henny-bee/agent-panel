use super::*;

pub(super) fn property(label: &'static str, value: impl IntoElement) -> gpui::Div {
    div()
        .w_full()
        .min_w_0()
        .flex_shrink_0()
        .flex()
        .items_start()
        .gap_2()
        .text_size(px(12.))
        .line_height(px(28.))
        .child(
            div()
                .w(px(80.))
                .flex_shrink_0()
                .text_color(content(0.45))
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(content(0.85))
                .child(value),
        )
}

impl Monos {
    pub(in crate::monos) fn attention_views(
        &self,
        id: &str,
        cx: &App,
    ) -> Vec<Entity<Conversation>> {
        let mut views = Vec::new();
        for (&session, run) in self.running.iter().filter(|(_, r)| r.mono == id) {
            if run.view.read(cx).needs_approval() {
                views.push(run.view.clone());
            }
            if let Some(owner) = self.owner.upgrade() {
                for launch in self
                    .roster
                    .monos
                    .iter()
                    .flat_map(|m| m.launches.iter())
                    .filter(|l| {
                        (l.origin_turn >> 32) as i64 == session && !l.completed && !l.dismissed
                    })
                {
                    if let Some(view) = owner
                        .read(cx)
                        .mono_session_entity(launch.session_id)
                        .filter(|v| v.read(cx).needs_approval())
                    {
                        views.push(view);
                    }
                }
            }
        }
        views
    }

    pub(in crate::monos) fn habits_list(
        &mut self,
        mono: &Mono,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let filter = self.field(
            "mono-habit-search",
            "Search habits…",
            String::new(),
            false,
            cx,
        );
        let query = filter.read(cx).content.to_string();
        let groups =
            grouped_habits(&self.habits, now_ms(), |h| self.habit_running_since(h)).map(|items| {
                items
                    .into_iter()
                    .filter(|ix| {
                        let h = &self.habits[*ix];
                        matches_query(
                            &query,
                            &format!("{} {} {}", h.name, h.instructions, h.schedule.label()),
                        )
                    })
                    .collect::<Vec<_>>()
            });
        let no_matches = !self.habits.is_empty() && groups.iter().all(|items| items.is_empty());
        let list = div().id("mono-habits-list").debug_selector(|| "mono-habits-list".into())
            .flex_1().min_h_0().min_w_0().overflow_y_scroll().p_2().flex().flex_col().gap_4()
            .when(no_matches, |d| d.child(empty_state("search-01", "No habits match this search.")))
            .when(self.habits.is_empty(), |d| d.child(
                div().flex_1().min_h(px(160.)).flex().flex_col().items_center().justify_center()
                    .gap_3().px_6().py_10().text_center().text_size(px(12.)).line_height(px(18.))
                    .text_color(content(0.55)).child(icon("clock", 18.))
                    .child(format!("Ask {} to put recurring work on a schedule, like a morning digest or a weekly report.", mono.label()))))
            .children(["Up next today", "Done today", "Later"].into_iter().zip(groups).filter(|(_, items)| !items.is_empty()).map(|(title, items)| {
                div().flex_shrink_0().w_full().min_w_0().flex().flex_col().gap_1p5()
                    .child(div().px_2().text_size(px(11.)).text_color(content(0.4)).child(title))
                    .children(items.into_iter().map(|ix| self.habit_row(&self.habits[ix], mono, cx)))
            }));
        div()
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .when(!self.habits.is_empty(), |d| d.child(search_bar(filter)))
            .child(list)
            .into_any_element()
    }

    pub(super) fn habit_row(&self, h: &Habit, mono: &Mono, cx: &mut Context<Self>) -> AnyElement {
        let id = h.id.clone();
        let since = self.habit_running_since(h);
        let subtitle = if let Some(since) = since {
            format!("Running now · {}", duration(now_ms().saturating_sub(since)))
        } else if self.habit_busy(h) {
            "Starting…".into()
        } else if h.enabled {
            h.schedule.label()
        } else {
            format!("Paused · {}", h.schedule.label())
        };
        super::super::panels::outlined(div().id(SharedString::from(format!("mono-habit-{id}"))))
            .debug_selector({
                let id = id.clone();
                move || format!("mono-habit-{id}")
            })
            .group("mono-habit")
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .flex()
            .items_center()
            .child(
                div()
                    .id(SharedString::from(format!("mono-habit-open-{id}")))
                    .role(Role::Button)
                    .aria_label(h.name.clone())
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .px(px(10.))
                    .py_2()
                    .child(
                        div()
                            .size(px(28.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .bg(content(0.06))
                            .child(if since.is_some() {
                                div()
                                    .size(px(8.))
                                    .rounded_full()
                                    .bg(rgb(mono.color))
                                    .into_any_element()
                            } else {
                                icon("clock", 14.)
                                    .text_color(content(if h.enabled { 0.6 } else { 0.3 }))
                                    .into_any_element()
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .line_height(px(20.))
                                    .truncate()
                                    .text_color(content(if h.enabled || since.is_some() {
                                        0.85
                                    } else {
                                        0.45
                                    }))
                                    .child(h.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .line_height(px(16.))
                                    .truncate()
                                    .text_color(content(0.4))
                                    .child(subtitle),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.route = Some(Route::Habit(id.clone()));
                        this.clear_habit_fields();
                        cx.notify();
                    })),
            )
            .child(self.habit_actions(h, false, cx).pr_2())
            .into_any_element()
    }

    pub(super) fn habit_actions(&self, h: &Habit, page: bool, cx: &mut Context<Self>) -> gpui::Div {
        let busy = self.habit_busy(h);
        let running = self.habit_running_since(h).is_some();
        let run = h.id.clone();
        let toggle = h.id.clone();
        let remove = h.id.clone();
        let suffix = if page {
            String::new()
        } else {
            format!("-{}", h.id)
        };
        div()
            .when(!page, |d| {
                d.opacity(0.).group_hover("mono-habit", |s| s.opacity(1.))
            })
            .flex_shrink_0()
            .flex()
            .items_center()
            .child(
                habit_icon_button(
                    SharedString::from(format!("mono-habit-run{suffix}")),
                    if running {
                        "Already running"
                    } else if busy {
                        "Starting"
                    } else {
                        "Run now"
                    },
                    "zap",
                )
                .debug_selector({
                    let suffix = suffix.clone();
                    move || format!("mono-habit-run{suffix}")
                })
                .when(busy, |d| d.opacity(0.35))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !busy {
                        this.request_habit(&run, cx);
                    }
                })),
            )
            .child(
                habit_icon_button(
                    SharedString::from(format!("mono-habit-toggle{suffix}")),
                    if h.enabled { "Pause" } else { "Resume" },
                    if h.enabled {
                        "queue-pause"
                    } else {
                        "queue-play"
                    },
                )
                .debug_selector({
                    let suffix = suffix.clone();
                    move || format!("mono-habit-toggle{suffix}")
                })
                .on_click(cx.listener(move |this, _, _, cx| this.habit_toggle(&toggle, cx))),
            )
            .child(
                habit_icon_button(
                    SharedString::from(format!("mono-habit-delete{suffix}")),
                    "Remove",
                    "trash",
                )
                .debug_selector(move || format!("mono-habit-delete{suffix}"))
                .on_click(cx.listener(move |this, _, _, cx| this.remove_habit(&remove, cx))),
            )
    }

    pub(in crate::monos) fn habit_toggle(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(mono) = self.active.clone() else {
            return;
        };
        let root = self.root.clone();
        let target = mono.clone();
        let id = id.to_string();
        self.file_work(
            mono,
            move || {
                habits::update(&root, &target, |all| {
                    let h = all
                        .iter_mut()
                        .find(|h| h.id == id)
                        .ok_or("Habit not found")?;
                    h.enabled = !h.enabled;
                    if h.enabled {
                        h.next_run_at = h.schedule.next(now_ms())?;
                    }
                    Ok(())
                })
            },
            cx,
        );
    }

    pub(in crate::monos) fn habit_page(
        &mut self,
        mono: &Mono,
        id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(id) = id {
            let Some(h) = self.habits.iter().find(|h| h.id == id).cloned() else {
                return div()
                    .size_full()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(self.page_header("Habit", cx))
                    .child(
                        div()
                            .p_4()
                            .text_size(px(12.))
                            .text_color(content(0.45))
                            .child("This habit is no longer available."),
                    )
                    .into_any_element();
            };
            return self.habit_detail(mono, h, cx);
        }
        self.new_habit_page(window, cx)
    }

    pub(super) fn habit_detail(&self, mono: &Mono, h: Habit, cx: &mut Context<Self>) -> AnyElement {
        let since = self.habit_running_since(&h);
        let mut info = div()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .px_4()
            .py_3()
            .flex()
            .flex_col()
            .gap_0p5()
            .child(property(
                "Runs",
                if h.enabled {
                    h.schedule.label()
                } else {
                    "Paused".into()
                },
            ));
        if since.is_none() && self.habit_busy(&h) {
            info = info.child(property("Next", "Starting…"));
        } else if h.enabled {
            info = info.child(property("Next", when(h.next_run_at, now_ms())));
        }
        let mut history = section("Recent runs");
        if let Some(since) = since {
            history = history.child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .px_2()
                    .py_2()
                    .text_size(px(12.))
                    .child(
                        div()
                            .size(px(8.))
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(rgb(mono.color)),
                    )
                    .child(div().flex_1().min_w_0().child("Running now"))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(content(0.4))
                            .child(duration(now_ms().saturating_sub(since))),
                    ),
            );
        }
        if h.runs.is_empty() && since.is_none() {
            history = history.child(
                div()
                    .px_2()
                    .py_3()
                    .text_size(px(12.))
                    .text_color(content(0.4))
                    .child("It hasn't run yet."),
            );
        }
        for run in &h.runs {
            history = history.child(self.habit_run_row(mono, &h, run, cx));
        }
        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(
                self.page_header(&h.name, cx)
                    .child(self.habit_actions(&h, true, cx)),
            )
            .child(
                div()
                    .id("mono-habit-editor")
                    .debug_selector(|| "mono-habit-editor".into())
                    .w_full()
                    .min_w_0()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .child(info)
                    .child(
                        section("What it does").child(
                            div()
                                .id("mono-habit-instructions")
                                .debug_selector(|| "mono-habit-instructions".into())
                                .w_full()
                                .min_w_0()
                                .flex_shrink_0()
                                .px_2()
                                .text_size(px(12.))
                                .line_height(px(20.))
                                .text_color(content(0.75))
                                .child(h.instructions.clone()),
                        ),
                    )
                    .child(history),
            )
            .into_any_element()
    }

    pub(super) fn habit_run_row(
        &self,
        mono: &Mono,
        h: &Habit,
        run: &Run,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = (h.id.clone(), run.at);
        let expanded = self.habit_ui.expanded.contains(&key);
        let detail = run
            .report
            .as_ref()
            .or(run.error.as_ref())
            .is_some_and(|text| !text.is_empty());
        let (label, color) = match run.outcome.as_str() {
            "posted" => ("Messaged you", rgb(mono.color).into()),
            "quiet" => ("Nothing to report", content(0.25)),
            _ => ("Couldn't finish", rgb(0xf87171).into()),
        };
        let time = format!(
            "{}{}",
            when(run.at, now_ms()),
            if run.duration_ms > 0 {
                format!(" · {}", duration(run.duration_ms))
            } else {
                String::new()
            }
        );
        let mut card = div()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .rounded_md()
            .when(expanded && detail, |d| d.bg(content(0.05)))
            .child(
                div()
                    .id(SharedString::from(format!(
                        "mono-habit-history-{}-{}",
                        h.id, run.at
                    )))
                    .debug_selector({
                        let id = h.id.clone();
                        let at = run.at;
                        move || format!("mono-habit-history-{id}-{at}")
                    })
                    .role(Role::Button)
                    .aria_label(format!("{label}, {time}"))
                    .w_full()
                    .min_w_0()
                    .px_2()
                    .py_2()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2p5()
                    .text_size(px(12.))
                    .when(detail, |d| {
                        d.aria_expanded(expanded).hover(|s| s.bg(content(0.05)))
                    })
                    .child(div().size(px(8.)).flex_shrink_0().rounded_full().bg(color))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(72.))
                            .truncate()
                            .text_color(content(0.85))
                            .child(label),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(content(0.4))
                            .child(time),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if detail {
                            if !this.habit_ui.expanded.remove(&key) {
                                this.habit_ui.expanded.insert(key.clone());
                            }
                            cx.notify();
                        }
                    })),
            );
        if expanded && detail {
            let mut body = div()
                .id(SharedString::from(format!(
                    "mono-habit-report-{}-{}",
                    h.id, run.at
                )))
                .debug_selector({
                    let id = h.id.clone();
                    let at = run.at;
                    move || format!("mono-habit-report-{id}-{at}")
                })
                .w_full()
                .min_w_0()
                .flex_shrink_0()
                .overflow_hidden()
                .pl(px(26.))
                .pr_3()
                .pb_3()
                .text_size(px(12.))
                .line_height(px(20.));
            if let Some(report) = &run.report {
                for (ix, mut row) in crate::chat::markdown::model::parse_full(report)
                    .blocks
                    .iter()
                    .flat_map(|b| crate::chat::markdown::rows(&b.block))
                    .enumerate()
                {
                    crate::chat::markdown::resolve_file_links(
                        &mut row,
                        &mono.projects.first().cloned().unwrap_or_default(),
                    );
                    body = body.child(div().w_full().min_w_0().flex_shrink_0().child(
                        crate::chat::markdown::body(
                            &row,
                            format!("habit-report-{}-{}-{ix}", h.id, run.at).into(),
                            crate::chat::markdown::Tone::Normal,
                            None,
                            std::time::Instant::now(),
                        ),
                    ));
                }
            } else if let Some(error) = &run.error {
                body = body.child(div().text_color(rgb(0xf87171)).child(error.clone()));
            }
            card = card.child(body);
        }
        card.into_any_element()
    }
}
