//! The Agents tab: what the chat cost, and a card per subagent grouped by
//! whether it still runs.
use super::rows::*;
use super::*;

impl ClaudePanel {
    fn header(&self, title: String, cx: &mut Context<Self>) -> AnyElement {
        div()
            .h(px(32.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(content(0.85))
                    .child(title),
            )
            .child(
                div()
                    .id("claude-panel-hide")
                    .role(Role::Button)
                    .aria_label("Hide and show the files")
                    .px_2()
                    .py(px(2.))
                    .rounded_md()
                    .bg(content(0.06))
                    .hover(|s| s.bg(content(0.12)))
                    .cursor_pointer()
                    .text_size(px(11.))
                    .text_color(content(0.7))
                    .child("Hide")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ClaudePanelEvent::Hide))),
            )
            .into_any_element()
    }

    pub(super) fn agents(&self, snapshot: &Snapshot, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let count = snapshot.agents.len();
        let mut out = vec![self.header(
            format!(
                "{count} subagent{} in this chat",
                if count == 1 { "" } else { "s" }
            ),
            cx,
        )];
        let time = (snapshot.last - snapshot.started).max(0);
        out.push(
            div()
                .px_3()
                .pb_2()
                .flex()
                .flex_wrap()
                .gap_1p5()
                .children([
                    stat("Est. chat cost", format!("${:.2}", snapshot.cost)),
                    stat("Total tokens", tokens(snapshot.tokens)),
                    stat("Subagent tokens", tokens(snapshot.agent_tokens)),
                    stat("Time", duration(time)),
                ])
                .into_any_element(),
        );
        if count == 0 {
            out.push(note("No subagents in this chat yet."));
            return out;
        }
        let now = monocode_core::time::now_ms() as i64;
        for (key, label, color, running) in [
            ("running", "Running", GREEN, true),
            ("finished", "Finished", AMBER_400, false),
        ] {
            // Colors follow start order, so an agent keeps its color as it finishes.
            let group: Vec<(usize, &Agent)> = snapshot
                .agents
                .iter()
                .enumerate()
                .filter(|(_, a)| (a.status == AgentStatus::Running) == running)
                .collect();
            if group.is_empty() {
                continue;
            }
            let collapsed = self.open.contains(key);
            out.push(
                div()
                    .id(SharedString::from(format!("claude-group-{key}")))
                    .role(Role::Button)
                    .aria_expanded(!collapsed)
                    .mx_3()
                    .mb_1p5()
                    .h(px(22.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .rounded_md()
                    .bg(Hsla::from(rgb(color)).alpha(0.16))
                    .cursor_pointer()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(color))
                    .child(
                        icon(
                            if collapsed {
                                "chevron-right"
                            } else {
                                "chevron-down"
                            },
                            12.,
                        )
                        .text_color(rgb(color)),
                    )
                    .child(format!("{label} · {}", group.len()))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(key.into(), cx)))
                    .into_any_element(),
            );
            if !collapsed {
                for (ix, agent) in group {
                    let color = AGENT_COLORS[ix % AGENT_COLORS.len()];
                    out.push(self.agent_card(agent, color, now, cx));
                }
            }
        }
        out
    }

    fn agent_card(
        &self,
        agent: &Agent,
        color: u32,
        now: i64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let running = agent.status == AgentStatus::Running;
        let tint = Hsla::from(rgb(color));
        let end = if running { now } else { agent.last };
        let steps = agent.steps.len();
        let (line, line_color) = match agent.status {
            AgentStatus::Running => (
                format!(
                    "Step {steps} · {}",
                    agent.steps.last().map_or("Starting", String::as_str)
                ),
                rgb(CLAUDE),
            ),
            AgentStatus::Done => (format!("Done · {steps} steps"), rgb(GREEN)),
            AgentStatus::Stopped => (format!("Stopped · {steps} steps"), rgb(RED_400)),
            AgentStatus::Failed => (format!("Failed · {steps} steps"), rgb(RED_400)),
        };
        // The two steps before the current one while running; the last two
        // once the agent is done.
        let skip = usize::from(running);
        let recent: Vec<String> = agent
            .steps
            .iter()
            .rev()
            .skip(skip)
            .take(2)
            .rev()
            .cloned()
            .collect();
        let key = format!("agent:{}", agent.id);
        let expanded = self.open.contains(&key);
        let status: AnyElement = match agent.status {
            AgentStatus::Running => div()
                .size(px(7.))
                .rounded_full()
                .bg(rgb(GREEN))
                .into_any_element(),
            AgentStatus::Done => icon("check", 13.).text_color(rgb(GREEN)).into_any_element(),
            AgentStatus::Stopped | AgentStatus::Failed => div()
                .text_size(px(10.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(RED_400))
                .child(if agent.status == AgentStatus::Failed {
                    "Failed"
                } else {
                    "Stopped"
                })
                .into_any_element(),
        };
        let small = || div().text_size(px(10.5)).text_color(content(0.5));
        let kind = match agent.model.as_str() {
            "" => agent.kind.clone(),
            model => format!("{} · {model}", agent.kind),
        };
        div()
            .id(SharedString::from(format!("claude-agent-{}", agent.id)))
            .role(Role::Button)
            .aria_expanded(expanded)
            .aria_label(SharedString::from(format!("{}: {line}", agent.title)))
            .mx_3()
            .mb_1p5()
            .p_2()
            .rounded_lg()
            .border_1()
            .border_color(if running { tint.alpha(0.6) } else { stroke() })
            .bg(content(0.03))
            .hover(|s| s.bg(content(0.05)))
            .cursor_pointer()
            .flex()
            .gap_2()
            .on_click(cx.listener(move |this, _, _, cx| this.toggle(key.clone(), cx)))
            .child(
                div()
                    .pt(px(6.))
                    .child(icon("agent-critter", 26.).text_color(tint)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(content(0.95))
                                    .child(agent.title.clone()),
                            )
                            .child(status),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1p5()
                            .child(small().flex_1().min_w_0().truncate().child(kind))
                            .child(small().flex_shrink_0().font_features(tabular()).child(
                                format!(
                                    "{} tokens · {}",
                                    tokens(agent.tokens),
                                    duration(end - agent.started)
                                ),
                            )),
                    )
                    .child(
                        div()
                            .pt(px(2.))
                            .truncate()
                            .text_size(px(11.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(line_color)
                            .child(line),
                    )
                    .children(
                        recent
                            .into_iter()
                            .map(|step| small().truncate().child(step)),
                    )
                    .when(expanded, |d| {
                        d.child(
                            div()
                                .mt_1()
                                .pt_1()
                                .border_t_1()
                                .border_color(stroke())
                                .flex()
                                .flex_col()
                                .children(agent.steps.iter().enumerate().map(|(ix, step)| {
                                    small()
                                        .flex()
                                        .gap_1p5()
                                        .child(
                                            div()
                                                .w(px(22.))
                                                .flex_shrink_0()
                                                .font_features(tabular())
                                                .text_color(content(0.35))
                                                .child(format!("{}", ix + 1)),
                                        )
                                        .child(div().min_w_0().child(step.clone()))
                                })),
                        )
                    })
                    .child(progress(&agent.id, agent.status, tint)),
            )
            .into_any_element()
    }
}

fn stat(label: &'static str, value: String) -> AnyElement {
    div()
        .flex_1()
        .min_w(px(110.))
        .px_2()
        .py_1p5()
        .rounded_lg()
        .border_1()
        .border_color(stroke())
        .bg(content(0.03))
        .child(
            div()
                .truncate()
                .text_size(px(10.5))
                .text_color(content(0.5))
                .child(label),
        )
        .child(
            div()
                .text_size(px(14.))
                .font_weight(FontWeight::SEMIBOLD)
                .font_features(tabular())
                .text_color(rgb(CLAUDE))
                .child(value),
        )
        .into_any_element()
}

/// Running: a segment sweeping the track; otherwise full, red when it ended badly.
fn progress(id: &str, status: AgentStatus, tint: Hsla) -> AnyElement {
    let track = div()
        .mt_1p5()
        .h(px(3.))
        .w_full()
        .relative()
        .overflow_hidden()
        .rounded_full()
        .bg(content(0.08));
    match status {
        AgentStatus::Running => track
            .child(crate::pulse::looping(
                SharedString::from(format!("claude-agent-bar-{id}")),
                Duration::from_millis(1600),
                crate::pulse::Rate::Smooth,
                move |phase| {
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .w(relative(0.35))
                        .left(relative(phase * 1.35 - 0.35))
                        .rounded_full()
                        .bg(tint)
                },
            ))
            .into_any_element(),
        AgentStatus::Done => track.child(div().size_full().bg(tint)).into_any_element(),
        AgentStatus::Stopped | AgentStatus::Failed => track
            .child(div().size_full().bg(Hsla::from(rgb(RED_400)).alpha(0.6)))
            .into_any_element(),
    }
}
