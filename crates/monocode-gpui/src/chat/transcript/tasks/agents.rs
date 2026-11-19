use super::*;

pub(super) fn panel_width(view: &TranscriptView, start: usize) -> Pixels {
    let inset = view
        .rows
        .iter()
        .find(|row| row.key == (start, 2))
        .map_or(0., |row| {
            (if row.work { 20. } else { 0. }) + if row.in_phase() { 20. } else { 0. }
        });
    px((f32::from(view.viewport.size.width).min(COLUMN)
        - 2. * crate::ui::theme::COLUMN_PAD
        - inset)
        .max(1.))
}

pub(super) fn disclosure(
    view: &TranscriptView,
    key: &str,
    open: bool,
    width: Pixels,
    body: AnyElement,
) -> AnyElement {
    let (height, opacity, _) = view.agent_panels.progress(key, open);
    if height >= 1. {
        return body;
    }
    div()
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .opacity(opacity)
        .child(Disclosure::new(body, width, height))
        .into_any_element()
}

pub(super) fn agent(
    view: &mut TranscriptView,
    call: &ToolCall,
    run: &AgentRun,
    start: usize,
    available_width: Option<Pixels>,
    cx: &mut Context<TranscriptView>,
) -> AnyElement {
    let name = run.display_name();
    let failed = matches!(call.state, ToolState::Failed | ToolState::Cancelled);
    let active = view.streaming
        && call.state == ToolState::Running
        && view
            .source
            .iter()
            .rposition(monocode_core::session::transcript::Row::is_prompt)
            .is_none_or(|prompt| start > prompt);
    let report = call.agent_report();
    let omitted = call
        .display
        .as_ref()
        .is_some_and(|meta| meta["detailOmitted"] == true);
    let has_detail = !run.steps.is_empty() || report.is_some() || omitted;
    let key = format!("agent:{start}:{}", call.id);
    let open = view.agent_panels.open(&key, failed);
    if !open {
        // Work arriving behind the row is history when it opens again.
        for phase in agent_phases(run) {
            let phase_key = format!("agent-phase:{start}:{}:{}", call.id, phase.id);
            view.agent_panels
                .observe_steps(&phase_key, &phase.steps, start, false);
        }
    }
    let rotation = view.agent_panels.progress(&key, open).2;
    let toggle_key = key.clone();
    let status = run.status_line(&call.state);
    let model = run.model_name(
        monocode_core::provider::Kind::parse(&view.provider)
            .unwrap_or(monocode_core::provider::Kind::Codex),
    );
    let header = div()
        .id(SharedString::from(key.clone()))
        .debug_selector(move || format!("subagent-{start}"))
        .role(if has_detail {
            Role::Button
        } else {
            Role::Label
        })
        .aria_label(if has_detail {
            format!("{} {name}'s work", if open { "Hide" } else { "Show" })
        } else {
            format!("Subagent: {name}")
        })
        .when(has_detail, |d| d.aria_expanded(open))
        .tooltip({
            let brief = run.display_brief();
            move |_, cx| cx.new(|_| AgentBrief(brief.clone())).into()
        })
        .group("subagent-row")
        .flex()
        .items_center()
        .gap_2()
        .w_full()
        .min_w_0()
        .mx(px(-6.))
        .px_1p5()
        .py_1()
        .rounded(px(6.))
        .text_size(px(14.))
        .line_height(px(20.))
        .when(has_detail, |d| {
            d.cursor_pointer().hover(|s| s.bg(content(0.08)))
        })
        .when(has_detail && open, |d| d.bg(content(0.08)))
        .child(super::super::mascot::subagent_mascot(
            &name,
            &call.state,
            active && view.visible,
        ))
        .child(
            div()
                .flex()
                .items_baseline()
                .gap_2()
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(if failed {
                            rgb(0xf87171).into()
                        } else {
                            content(0.75)
                        })
                        .group_hover("subagent-row", |s| {
                            s.text_color(if failed {
                                rgb(0xf87171).into()
                            } else {
                                content(1.)
                            })
                        })
                        .child(if active && view.visible && !cx.reduce_motion() {
                            crate::chat::transcript::activity::shimmer(
                                name.clone(),
                                view.started_at.elapsed().as_secs_f32(),
                            )
                        } else {
                            div().child(name).into_any_element()
                        }),
                )
                .when(model.is_some() || !status.is_empty(), |d| {
                    d.child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap_2()
                            .min_w_0()
                            .max_w(gpui::relative(0.55))
                            .flex_shrink_0()
                            .text_size(px(12.))
                            .text_color(content(0.4))
                            .when_some(model, |d, model| {
                                d.child(div().min_w_0().truncate().child(model))
                            })
                            .when(!status.is_empty(), |d| {
                                d.child(div().flex_shrink_0().child(status))
                            }),
                    )
                }),
        )
        .child(if has_detail {
            icon("chevron-right", 14.)
                .flex_shrink_0()
                .text_color(content(0.35))
                .with_transformation(Transformation::rotate(percentage(rotation * 0.25)))
                .into_any_element()
        } else {
            div().size(px(14.)).flex_shrink_0().into_any_element()
        })
        .on_click(cx.listener(move |this, _, _, cx| {
            if has_detail {
                this.toggle_agent_part(toggle_key.clone(), !open, start, cx);
            }
        }));
    let mut panel = div().w_full().min_w_0().flex().flex_col().child(header);
    if has_detail && view.agent_panels.visible(&key, open) {
        let width = available_width.unwrap_or_else(|| panel_width(view, start));
        let trail = agent_trail(view, call, run, start, active, width, cx);
        let mut body = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .pb_1()
            .child(trail);
        if let Some(report) = report {
            let report = if failed {
                div()
                    .font_family(crate::ui::fonts::mono())
                    .text_size(px(12.))
                    .line_height(px(20.))
                    .text_color({
                        let mut color: gpui::Hsla = rgb(0xf87171).into();
                        color.a = 0.8;
                        color
                    })
                    .child(report.to_owned())
                    .into_any_element()
            } else {
                markdown_text(
                    view,
                    report,
                    &format!("{}:report", call.id),
                    markdown::Tone::Normal,
                    cx,
                )
            };
            body = body.child(
                div()
                    .relative()
                    .pl(px(20.))
                    .py_1()
                    .child(crate::chat::transcript::activity::rail(true))
                    .child(report),
            );
        }
        panel = panel.child(disclosure(view, &key, open, width, body.into_any_element()));
    }
    panel.into_any_element()
}

/// Direct port of buildActivityPhases: prose titles the work that follows it;
/// private reasoning remains a step, and mixed tool kinds stay in one phase.
pub(super) struct AgentPhase<'a> {
    pub(super) id: &'a str,
    pub(super) headline: Option<&'a AgentStep>,
    pub(super) steps: Vec<&'a AgentStep>,
    pub(super) kind: &'static str,
    pub(super) counts: [usize; 5],
}

/// The phase kind a tool step counts toward, as an index into the kinds
/// `agent_phases` names. Classifying builds the step's full tool call and runs
/// its presentation, and agent rows group their steps on every frame, so a
/// finished step, which no longer changes, keeps its answer.
pub(super) fn step_kind(run: &AgentRun, step: &AgentStep) -> usize {
    thread_local! {
        static SETTLED: RefCell<HashMap<u64, usize>> = RefCell::default();
    }
    let settled = matches!(
        step.state,
        Some(ToolState::Completed | ToolState::Failed | ToolState::Cancelled)
    );
    let key = {
        let mut hasher = std::hash::DefaultHasher::new();
        (&run.ids, &step.id, &step.text).hash(&mut hasher);
        hasher.finish()
    };
    if settled && let Some(kind) = SETTLED.with_borrow(|cache| cache.get(&key).copied()) {
        return kind;
    }
    let kind = match step.to_tool_call().summary().0.as_str() {
        "Edit" | "Write" => 0,
        "Run" => 1,
        "Agent" => 2,
        "Read" | "List" | "Search" | "Find" => 3,
        _ => 4,
    };
    if settled {
        SETTLED.with_borrow_mut(|cache| {
            if cache.len() >= 4096 {
                cache.clear();
            }
            cache.insert(key, kind);
        });
    }
    kind
}

pub(super) fn agent_phases(run: &AgentRun) -> Vec<AgentPhase<'_>> {
    let mut phases: Vec<AgentPhase<'_>> = Vec::new();
    for step in &run.steps {
        if step.kind != "tool" && step.text.trim().is_empty() {
            continue;
        }
        let prose = step.kind != "tool" && step.kind != "reasoning";
        let narrating = phases
            .last()
            .is_some_and(|phase| matches!(phase.kind, "think" | "note"));
        if phases.is_empty() || (prose && !narrating) {
            phases.push(AgentPhase {
                id: &step.id,
                headline: None,
                steps: Vec::new(),
                kind: if prose { "note" } else { "think" },
                counts: [0; 5],
            });
        }
        let current = phases.last_mut().unwrap();
        if prose && current.headline.is_none() {
            current.headline = Some(step);
            current.kind = "note";
            continue;
        }
        current.steps.push(step);
        if step.kind == "tool" {
            current.counts[step_kind(run, step)] += 1;
            let dominant = (0..5)
                .max_by_key(|&kind| (current.counts[kind], 5 - kind))
                .unwrap();
            current.kind = ["edit", "run", "agent", "research", "other"][dominant];
        }
    }
    phases
}

pub(super) fn phase_icon(kind: &str) -> &'static str {
    match kind {
        "edit" => "pencil-line",
        "run" => "terminal",
        "agent" => "bot",
        "research" => "search",
        "other" => "wrench",
        _ => "minus",
    }
}

pub(super) fn agent_trail(
    view: &mut TranscriptView,
    parent: &ToolCall,
    run: &AgentRun,
    start: usize,
    active: bool,
    width: Pixels,
    cx: &mut Context<TranscriptView>,
) -> AnyElement {
    let phases = agent_phases(run);
    let count = phases.len();
    let mut trail = div().w_full().min_w_0().flex().flex_col().gap_1();
    for (index, phase) in phases.into_iter().enumerate() {
        let live = active && index + 1 == count;
        let key = format!("agent-phase:{start}:{}:{}", parent.id, phase.id);
        let open = view.agent_panels.open(&key, live);
        if let Some(scroll) = view.agent_panels.scrolls.get_mut(&key) {
            scroll.enable(live && open && view.visible);
        }
        view.agent_panels.observe_steps(
            &key,
            &phase.steps,
            start,
            live && open && view.visible && !cx.reduce_motion(),
        );
        let calls: Vec<_> = phase
            .steps
            .iter()
            .filter(|step| step.kind == "tool")
            .map(|step| step.to_tool_call())
            .collect();
        let failed_agents = calls
            .iter()
            .filter(|call| {
                matches!(call.state, ToolState::Failed | ToolState::Cancelled)
                    && matches!(call.panel(), Some(Panel::Agent(_)))
            })
            .count();
        let title = if failed_agents > 0 {
            if failed_agents == 1 {
                "Subagent failed".to_owned()
            } else {
                format!("{failed_agents} subagents failed")
            }
        } else {
            phase
                .headline
                .map(|step| prose_summary(&step.text))
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| {
                    if calls.is_empty() {
                        if live { "Thinking" } else { "Thought" }.to_owned()
                    } else {
                        crate::chat::transcript::activity::summary(&calls, live).0
                    }
                })
        };
        let glyph = phase_icon(phase.kind);
        if phase.headline.is_none() && phase.steps.len() == 1 {
            let inset = if phase.kind == "think" { 0. } else { 20. };
            let row = agent_step(
                view,
                parent,
                phase.steps[0],
                start,
                true,
                (width - px(inset)).max(px(1.)),
                cx,
            );
            trail = trail.child(
                div()
                    .flex()
                    .items_start()
                    .gap_1p5()
                    .when(phase.kind != "think", |d| {
                        d.child(icon(glyph, 14.).mt(px(7.)).text_color(content(0.45)))
                    })
                    .child(div().flex_1().min_w_0().child(row)),
            );
            continue;
        }
        let has_headline_detail = phase
            .headline
            .is_some_and(|step| step.text.trim().lines().any(|line| line.trim().is_empty()));
        let inert = phase.steps.is_empty() && !has_headline_detail;
        let toggle_key = key.clone();
        let rotation = view.agent_panels.progress(&key, open).2;
        let mut group = div()
            .id(SharedString::from(key.clone()))
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("header")
                    .role(if inert { Role::Label } else { Role::Button })
                    .aria_label(format!(
                        "{} the steps for {title}",
                        if open { "Hide" } else { "Show" }
                    ))
                    .when(!inert, |d| d.aria_expanded(open).cursor_pointer())
                    .group("agent-phase")
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .py_1()
                    .text_size(px(14.))
                    .line_height(px(20.))
                    .text_color(content(0.5))
                    .child(
                        div()
                            .relative()
                            .size(px(14.))
                            .flex_shrink_0()
                            .when(phase.kind != "think", |d| {
                                d.child(
                                    icon(glyph, 14.).group_hover("agent-phase", |s| s.opacity(0.)),
                                )
                            })
                            .when(!inert, |d| {
                                d.child(
                                    icon("chevron-right", 14.)
                                        .absolute()
                                        .top_0()
                                        .left_0()
                                        .opacity(0.)
                                        .with_transformation(Transformation::rotate(percentage(
                                            rotation * 0.25,
                                        )))
                                        .group_hover("agent-phase", |s| s.opacity(1.)),
                                )
                            }),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(
                        if live && view.visible && !cx.reduce_motion() {
                            crate::chat::transcript::activity::shimmer(
                                title,
                                view.started_at.elapsed().as_secs_f32(),
                            )
                        } else {
                            div().child(title).into_any_element()
                        },
                    ))
                    .hover(|s| s.text_color(content(0.8)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !inert {
                            this.toggle_agent_part(toggle_key.clone(), !open, start, cx);
                        }
                    })),
            );
        if !inert && view.agent_panels.visible(&key, open) {
            let show_headline = has_headline_detail && view.agent_panels.override_open(&key);
            let mut steps = div().w_full().min_w_0().flex().flex_col();
            if show_headline {
                let headline = phase.headline.unwrap();
                let body = markdown_text(
                    view,
                    &headline.text,
                    &format!("{}:{}:headline", parent.id, phase.id),
                    markdown::Tone::Normal,
                    cx,
                );
                steps = steps.child(
                    div()
                        .relative()
                        .pl(px(20.))
                        .py_1()
                        .child(crate::chat::transcript::activity::rail(
                            phase.steps.is_empty(),
                        ))
                        .child(body),
                );
            }
            for (index, step) in phase.steps.iter().enumerate() {
                let row = agent_step(
                    view,
                    parent,
                    step,
                    start,
                    true,
                    (width - px(20.)).max(px(1.)),
                    cx,
                );
                let arrival = view
                    .agent_panels
                    .arrivals
                    .get(&format!("{key}:{}", step.id))
                    .map(|arrival| {
                        let now = Instant::now();
                        if now < arrival.motion.start {
                            (0., 0.)
                        } else {
                            arrival.motion.value(now)
                        }
                    });
                let step_body = div()
                    .relative()
                    .pl(px(20.))
                    .child(crate::chat::transcript::activity::rail(
                        index + 1 == phase.steps.len(),
                    ))
                    .child(
                        div()
                            .opacity(arrival.map_or(1., |(_, opacity)| opacity))
                            .child(row),
                    );
                let step_body = if let Some((height, _)) = arrival {
                    div()
                        .w_full()
                        .min_w_0()
                        .overflow_hidden()
                        .child(Disclosure::new(step_body, width, height))
                        .into_any_element()
                } else {
                    step_body.into_any_element()
                };
                steps = steps.child(step_body);
            }
            let body = if live || !open {
                live_phase(
                    steps.into_any_element(),
                    view.agent_panels.scrolls.entry(key.clone()).or_default(),
                    view.phase_max_height,
                    live && open && view.visible,
                    cx,
                )
            } else {
                if let Some(scroll) = view.agent_panels.scrolls.get_mut(&key) {
                    scroll.enable(false);
                }
                steps.into_any_element()
            };
            group = group.child(disclosure(view, &key, open, width, body));
        }
        trail = trail.child(group);
    }
    trail.into_any_element()
}

pub(super) fn agent_step(
    view: &mut TranscriptView,
    parent: &ToolCall,
    step: &AgentStep,
    start: usize,
    bare: bool,
    width: Pixels,
    cx: &mut Context<TranscriptView>,
) -> AnyElement {
    let identity = format!("{}:{}", parent.id, step.id);
    if step.kind == "tool" {
        let mut call = step.to_tool_call();
        call.id = identity.clone();
        return view.tool_in(call, start, Some(&identity), Some(width), cx);
    }
    let reasoning = step.kind == "reasoning";
    let key = format!("agent-note:{start}:{identity}");
    let open = view.agent_panels.open(&key, false);
    let toggle_key = key.clone();
    let summary = prose_summary(&step.text);
    let mut row = div()
        .id(SharedString::from(key))
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .child(
            div()
                .id("toggle")
                .role(Role::Button)
                .aria_expanded(open)
                .aria_label(format!(
                    "{} {}: {summary}",
                    if open { "Hide" } else { "Show" },
                    if reasoning { "thinking" } else { "note" }
                ))
                .flex()
                .items_center()
                .gap_1p5()
                .py_1()
                .text_size(px(14.))
                .line_height(px(20.))
                .text_color(content(if reasoning { 0.5 } else { 0.7 }))
                .cursor_pointer()
                .hover(|s| s.text_color(content(0.8)))
                .when(!bare, |d| d.child(icon("minus", 14.)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(if open && reasoning {
                            "Thinking".to_owned()
                        } else {
                            summary
                        }),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_agent_part(toggle_key.clone(), !open, start, cx)
                })),
        );
    if open {
        row = row.child(div().min_w_0().pb_2().when(!bare, |d| d.pl(px(20.))).child(
            markdown_text(
                view,
                &step.text,
                &identity,
                if reasoning {
                    markdown::Tone::Reasoning
                } else {
                    markdown::Tone::Normal
                },
                cx,
            ),
        ));
    }
    row.into_any_element()
}

pub(super) struct AgentBrief(pub(super) String);

impl gpui::Render for AgentBrief {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(px(480.))
            .px_2()
            .py_1p5()
            .rounded(px(6.))
            .bg(rgb(0x292929))
            .border_1()
            .border_color(content(0.12))
            .text_size(px(12.))
            .line_height(px(18.))
            .text_color(content(0.85))
            .child(self.0.clone())
    }
}
