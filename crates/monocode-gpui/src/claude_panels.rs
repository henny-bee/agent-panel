//! The B (agents) and B2 (in this chat) views of a Claude session, shown in
//! place of the Explorer's file tree.
use crate::claude_activity::{
    self as activity, Agent, AgentStatus, FILE_KINDS, FileKind, Snapshot, TodoStatus,
};
use crate::conversation::Conversation;
use crate::right_panel::{AMBER_400, RED_400, stroke, tabular};
use crate::theme::*;
use gpui::{
    AnyElement, Context, Entity, EventEmitter, FontWeight, Hsla, MouseButton, Role, ScrollHandle,
    SharedString, Subscription, Task, WeakEntity, Window, div, prelude::*, px, relative, rgb,
};
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::collections::HashSet;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ClaudeTab {
    Agents,
    Chat,
}

/// Claude's own orange, for the numbers worth reading first.
const CLAUDE: u32 = 0xe3876a;
const GREEN: u32 = 0x34d399;
/// One color per agent, in the order they started.
const AGENT_COLORS: [u32; 8] = [
    0x60a5fa, 0xf472b6, 0xfb923c, 0xf87171, 0x4ade80, 0xa78bfa, 0x2dd4bf, 0xfacc15,
];

/// One session's activity, read once a second while a panel shows it.
pub(crate) struct ClaudeActivity {
    pub(crate) snapshot: Snapshot,
    cwd: String,
    seen: Cell<Option<Instant>>,
    _poll: Task<()>,
}

impl ClaudeActivity {
    pub(crate) fn new(conversation: WeakEntity<Conversation>, cx: &mut Context<Self>) -> Self {
        let poll = cx.spawn(async move |this, cx| {
            let mut tracker = activity::Tracker::default();
            loop {
                let source = this.update(cx, |this, cx| {
                    let seen = this.seen.get();
                    let watched = seen.is_some_and(|at| at.elapsed() < Duration::from_secs(3));
                    watched
                        .then(|| {
                            conversation
                                .upgrade()
                                .and_then(|c| c.read(cx).claude_source())
                        })
                        .flatten()
                });
                let Ok(source) = source else {
                    break;
                };
                if let Some((account, session, cwd)) = source {
                    let (back, snapshot) = cx
                        .background_executor()
                        .spawn(async move {
                            tracker.refresh(&account, &session);
                            let now = monocode_core::time::now_ms() as i64;
                            let snapshot = tracker.snapshot(&account, now);
                            (tracker, snapshot)
                        })
                        .await;
                    tracker = back;
                    let updated = this.update(cx, |this, cx| {
                        // A running agent's clock moves even when nothing is written.
                        let running = snapshot
                            .agents
                            .iter()
                            .any(|agent| agent.status == AgentStatus::Running);
                        if running || this.snapshot != snapshot || this.cwd != cwd {
                            this.snapshot = snapshot;
                            this.cwd = cwd;
                            cx.notify();
                        }
                    });
                    if updated.is_err() {
                        break;
                    }
                }
                cx.background_executor().timer(Duration::from_secs(1)).await;
            }
        });
        Self {
            snapshot: Snapshot::default(),
            cwd: String::new(),
            seen: Cell::new(None),
            _poll: poll,
        }
    }
}

pub(crate) enum ClaudePanelEvent {
    Hide,
    Open(String),
}

/// B or B2 for one Explorer: which view, and what is expanded in it.
pub(crate) struct ClaudePanel {
    pub(crate) activity: Entity<ClaudeActivity>,
    pub(crate) tab: ClaudeTab,
    /// Expanded agent cards and file lists, and collapsed agent groups.
    open: HashSet<String>,
    scroll: ScrollHandle,
    _observe: Subscription,
}
impl EventEmitter<ClaudePanelEvent> for ClaudePanel {}

impl ClaudePanel {
    pub(crate) fn new(
        activity: Entity<ClaudeActivity>,
        tab: ClaudeTab,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            _observe: cx.observe(&activity, |_, _, cx| cx.notify()),
            activity,
            tab,
            open: HashSet::new(),
            scroll: ScrollHandle::new(),
        }
    }

    fn toggle(&mut self, key: String, cx: &mut Context<Self>) {
        if !self.open.remove(&key) {
            self.open.insert(key);
        }
        cx.notify();
    }

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

    fn agents(&self, snapshot: &Snapshot, cx: &mut Context<Self>) -> Vec<AnyElement> {
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

    fn chat(&self, snapshot: &Snapshot, cwd: &str, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = vec![
            div()
                .h(px(40.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .child(icon("lucide-message-circle", 16.).text_color(content(0.85)))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(content(0.92))
                        .child("In this chat"),
                )
                .into_any_element(),
        ];
        if !snapshot.todos.is_empty() {
            let done = snapshot
                .todos
                .iter()
                .filter(|t| t.status == TodoStatus::Done)
                .count();
            let rows = snapshot.todos.iter().map(|todo| {
                let mark: AnyElement = match todo.status {
                    TodoStatus::Done => icon("check-circle", 14.)
                        .text_color(rgb(GREEN))
                        .into_any_element(),
                    TodoStatus::Active => icon("circle-dot", 14.)
                        .text_color(rgb(CLAUDE))
                        .into_any_element(),
                    TodoStatus::Pending => div()
                        .size(px(12.))
                        .m(px(1.))
                        .rounded_full()
                        .border_1()
                        .border_color(content(0.35))
                        .into_any_element(),
                };
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .py(px(3.))
                    .child(div().pt(px(1.)).flex_shrink_0().child(mark))
                    .child(
                        div()
                            .min_w_0()
                            .text_size(px(12.))
                            .map(|d| match todo.status {
                                TodoStatus::Done => d.text_color(content(0.4)).line_through(),
                                TodoStatus::Active => d
                                    .text_color(content(0.95))
                                    .font_weight(FontWeight::SEMIBOLD),
                                TodoStatus::Pending => d.text_color(content(0.75)),
                            })
                            .child(todo.text.clone()),
                    )
                    .into_any_element()
            });
            out.push(self.section(
                "todo",
                "lucide-list-todo",
                "To-do",
                format!("{done} of {}", snapshot.todos.len()),
                rows.collect(),
                cx,
            ));
        }
        if !snapshot.mcp.is_empty() {
            let rows = snapshot
                .mcp
                .iter()
                .enumerate()
                .map(|(ix, (server, tools))| {
                    let name = activity::server_name(server);
                    let calls = tools.iter().map(|(_, n)| n).sum::<u32>();
                    let names = tools
                        .iter()
                        .map(|(t, _)| t.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    tool_row(ix > 0, None, name, Some(names), calls)
                });
            out.push(self.section(
                "mcp",
                "mcp",
                "MCP",
                snapshot.mcp.len().to_string(),
                rows.collect(),
                cx,
            ));
        }
        if !snapshot.tools.is_empty() {
            let rows = snapshot
                .tools
                .iter()
                .enumerate()
                .map(|(ix, (name, calls))| {
                    tool_row(ix > 0, Some(tool_icon(name)), name.clone(), None, *calls)
                });
            out.push(self.section(
                "tools",
                "lucide-wrench",
                "Tools",
                snapshot.tools.len().to_string(),
                rows.collect(),
                cx,
            ));
        }
        let touched: usize = snapshot.files.iter().map(Vec::len).sum();
        if touched > 0 {
            let mut rows = Vec::new();
            for (kind, paths) in FILE_KINDS.iter().zip(&snapshot.files) {
                if paths.is_empty() {
                    continue;
                }
                let key = format!("files:{kind:?}");
                let open = self.open.contains(&key);
                rows.push(
                    div()
                        .id(SharedString::from(key.clone()))
                        .role(Role::Button)
                        .aria_expanded(open)
                        .h(px(26.))
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .rounded_md()
                        .hover(|s| s.bg(content(0.05)))
                        .cursor_pointer()
                        .text_size(px(12.))
                        .child(icon(
                            if open {
                                "chevron-down"
                            } else {
                                "chevron-right"
                            },
                            12.,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .text_color(content(0.85))
                                .child(file_label(*kind)),
                        )
                        .child(
                            div()
                                .font_features(tabular())
                                .text_size(px(11.))
                                .text_color(rgb(RED_400))
                                .child(paths.len().to_string()),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle(key.clone(), cx)))
                        .into_any_element(),
                );
                if open {
                    for path in paths {
                        rows.push(self.file_row(path, cwd, cx));
                    }
                }
            }
            out.push(self.section(
                "files",
                "lucide-folder",
                "Files",
                touched.to_string(),
                rows,
                cx,
            ));
        }
        if !snapshot.context.is_empty() {
            let rows = snapshot.context.iter().map(|path| {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .py(px(4.))
                    .text_size(px(12.))
                    .child(icon("lucide-file", 14.).text_color(content(0.7)))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(content(0.88))
                            .child(file_name(path)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(content(0.45))
                            .child(folder(path, cwd)),
                    )
                    .child(
                        div()
                            .px_1()
                            .rounded(px(3.))
                            .bg(content(0.08))
                            .text_size(px(10.))
                            .text_color(content(0.55))
                            .child("auto"),
                    )
                    .into_any_element()
            });
            out.push(self.section(
                "context",
                "lucide-file-text",
                "Context files",
                snapshot.context.len().to_string(),
                rows.collect(),
                cx,
            ));
        }
        if !snapshot.skills.is_empty() {
            let rows = snapshot.skills.iter().map(|(name, uses)| {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .py(px(4.))
                    .text_size(px(12.))
                    .child(icon("lucide-code", 14.).text_color(content(0.7)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(content(0.88))
                            .child(name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(content(0.5))
                            .child(format!("{uses} use{}", if *uses == 1 { "" } else { "s" })),
                    )
                    .into_any_element()
            });
            out.push(self.section(
                "skills",
                "lucide-layers",
                "Skills",
                snapshot.skills.len().to_string(),
                rows.collect(),
                cx,
            ));
        }
        if out.len() == 1 {
            out.push(note("Nothing used in this chat yet."));
        }
        out
    }

    /// A card with an icon, title, count and fold arrow over its rows.
    fn section(
        &self,
        key: &'static str,
        path: &'static str,
        title: &'static str,
        count: String,
        rows: Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fold = format!("fold:{key}");
        let folded = self.open.contains(&fold);
        div()
            .mx_3()
            .mb_2()
            .rounded_lg()
            .border_1()
            .border_color(stroke())
            .bg(content(0.03))
            .flex()
            .flex_col()
            .child(
                div()
                    .id(SharedString::from(format!("claude-section-{key}")))
                    .role(Role::Button)
                    .aria_expanded(!folded)
                    .h(px(36.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .child(icon(path, 15.).text_color(content(0.8)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(content(0.92))
                            .child(title),
                    )
                    .child(
                        div()
                            .font_features(tabular())
                            .text_size(px(11.))
                            .text_color(content(0.5))
                            .child(count),
                    )
                    .child(
                        icon(
                            if folded {
                                "chevron-right"
                            } else {
                                "chevron-down"
                            },
                            14.,
                        )
                        .text_color(content(0.6)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(fold.clone(), cx))),
            )
            .when(!folded, |d| {
                d.child(
                    div()
                        .mx(px(10.))
                        .py_1()
                        .border_t_1()
                        .border_color(stroke())
                        .flex()
                        .flex_col()
                        .children(rows),
                )
            })
            .into_any_element()
    }

    fn file_row(&self, path: &str, cwd: &str, cx: &mut Context<Self>) -> AnyElement {
        let open = path.to_owned();
        div()
            .id(SharedString::from(format!("claude-file-{path}")))
            .role(Role::Button)
            .aria_label(SharedString::from(format!("Open {path}")))
            .pl(px(18.))
            .h(px(22.))
            .flex()
            .items_center()
            .gap_1p5()
            .rounded_md()
            .hover(|s| s.bg(content(0.05)))
            .cursor_pointer()
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(11.5))
                    .text_color(content(0.85))
                    .child(file_name(path)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(11.))
                    .text_color(content(0.4))
                    .child(folder(path, cwd)),
            )
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(ClaudePanelEvent::Open(open.clone()))))
            .into_any_element()
    }
}

impl Render for ClaudePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let activity = self.activity.read(cx);
        // Showing the panel is what keeps the session's files being read.
        activity.seen.set(Some(Instant::now()));
        let snapshot = activity.snapshot.clone();
        let cwd = activity.cwd.clone();
        let body = if !snapshot.loaded {
            vec![note("Reading this chat…")]
        } else {
            match self.tab {
                ClaudeTab::Agents => self.agents(&snapshot, cx),
                ClaudeTab::Chat => self.chat(&snapshot, &cwd, cx),
            }
        };
        div()
            .id("claude-panel")
            .role(Role::Group)
            .aria_label(match self.tab {
                ClaudeTab::Agents => "Agents in this chat",
                ClaudeTab::Chat => "In this chat",
            })
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .pb_3()
            // The file tree's context menu has nothing to act on here.
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .children(body)
    }
}

fn note(text: &'static str) -> AnyElement {
    div()
        .px_3()
        .py_2()
        .text_size(px(12.))
        .text_color(content(0.5))
        .child(text)
        .into_any_element()
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

/// An MCP server (letter badge and the tools it ran) or a built-in tool (its
/// icon), with a line above every row but the first.
fn tool_row(
    divider: bool,
    glyph: Option<&'static str>,
    name: String,
    tools: Option<String>,
    calls: u32,
) -> AnyElement {
    let letter = name
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string();
    div()
        .flex()
        .items_center()
        .gap_2()
        .min_h(px(30.))
        .py(px(4.))
        .when(divider, |d| d.border_t_1().border_color(stroke()))
        .map(|d| match glyph {
            Some(path) => d.child(icon(path, 14.).text_color(content(0.75))),
            None => d.child(
                div()
                    .size(px(26.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .bg(content(0.08))
                    .text_size(px(11.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(content(0.8))
                    .child(letter),
            ),
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_size(px(12.))
                        .text_color(content(0.88))
                        .when(tools.is_some(), |d| d.font_weight(FontWeight::SEMIBOLD))
                        .child(name),
                )
                .when_some(tools, |d, tools| {
                    d.child(
                        div()
                            .truncate()
                            .font_family(crate::fonts::mono())
                            .text_size(px(10.5))
                            .text_color(content(0.45))
                            .child(tools),
                    )
                }),
        )
        .child(
            div()
                .flex_shrink_0()
                .flex()
                .gap_1()
                .text_size(px(11.))
                .font_features(tabular())
                .child(div().text_color(rgb(RED_400)).child(calls.to_string()))
                .child(div().text_color(content(0.5)).child(if calls == 1 {
                    "Call"
                } else {
                    "Calls"
                })),
        )
        .into_any_element()
}

/// The Lucide icon for a built-in Claude Code tool; a wrench for the rest.
fn tool_icon(name: &str) -> &'static str {
    match name {
        "Bash" | "BashOutput" => "lucide-terminal",
        "Read" => "lucide-book-open",
        "Write" => "lucide-square-pen",
        "Edit" | "MultiEdit" => "lucide-pencil",
        "NotebookEdit" => "lucide-notebook-pen",
        "Glob" => "lucide-folder-search",
        "Grep" => "lucide-text-search",
        "ToolSearch" => "lucide-search",
        "WebSearch" => "lucide-globe",
        "WebFetch" => "lucide-link",
        "Agent" | "Task" => "lucide-bot",
        "SendMessage" => "lucide-send",
        "ListAgents" => "lucide-users",
        "SubagentHandback" => "lucide-reply",
        "StructuredOutput" => "lucide-braces",
        "TaskStop" | "KillShell" | "KillBash" => "lucide-circle-stop",
        "TaskCreate" => "lucide-list-plus",
        "TaskUpdate" => "lucide-list-checks",
        "TodoWrite" | "TaskList" | "TaskGet" | "TaskOutput" => "lucide-list-todo",
        "Monitor" => "lucide-activity",
        "AskUserQuestion" => "lucide-message-circle-question-mark",
        "Skill" => "lucide-layers",
        "EnterPlanMode" | "ExitPlanMode" => "lucide-map",
        "EnterWorktree" | "ExitWorktree" => "lucide-git-branch",
        "ScheduleWakeup" => "lucide-alarm-clock",
        "LSP" => "lucide-code",
        n if n.starts_with("Cron") => "lucide-calendar-clock",
        n if n.starts_with("Artifact") => "lucide-app-window",
        _ => "lucide-wrench",
    }
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

fn file_label(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Read => "Read",
        FileKind::Edited => "Edited",
        FileKind::Created => "Created",
        FileKind::Written => "Written over",
    }
}

fn file_name(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_owned()
}

/// The folder holding `path`: inside the project relative to it, elsewhere
/// with the home folder written `~`.
fn folder(path: &str, cwd: &str) -> String {
    let parent = path.rsplit_once(['/', '\\']).map_or("", |(dir, _)| dir);
    if !cwd.is_empty()
        && let Some(inner) = parent.strip_prefix(cwd)
        && (inner.is_empty() || inner.starts_with('/'))
    {
        return format!(".{inner}");
    }
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && parent.starts_with(&home) => {
            format!("~{}", &parent[home.len()..])
        }
        _ => parent.to_owned(),
    }
}

/// 950, 4.2k, 312k, 1.0M.
fn tokens(count: u64) -> String {
    match count {
        0..1_000 => count.to_string(),
        1_000..10_000 => format!("{:.1}k", count as f64 / 1e3),
        10_000..1_000_000 => format!("{}k", count / 1_000),
        _ => format!("{:.1}M", count as f64 / 1e6),
    }
}

/// 2:30, 1:02:30 past an hour, 3d 23h past a day.
fn duration(ms: i64) -> String {
    let seconds = ms.max(0) / 1000;
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours >= 24 {
        format!("{}d {}h", hours / 24, hours % 24)
    } else if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_counts_times_and_folders() {
        assert_eq!(tokens(950), "950");
        assert_eq!(tokens(12_400), "12k");
        assert_eq!(tokens(1_049_000), "1.0M");
        assert_eq!(duration(150_000), "2:30");
        assert_eq!(duration(3_750_000), "1:02:30");
        assert_eq!(duration(343_555_000), "3d 23h");
        assert_eq!(folder("/p/src/a.rs", "/p"), "./src");
        assert_eq!(folder("/p/a.rs", "/p"), ".");
        assert_eq!(folder("/pq/a.rs", "/p"), "/pq");
    }

    #[test]
    fn every_panel_icon_is_bundled() {
        use gpui::AssetSource;
        let tools = [
            "Bash",
            "Read",
            "Write",
            "Edit",
            "NotebookEdit",
            "Glob",
            "Grep",
            "ToolSearch",
            "WebSearch",
            "WebFetch",
            "Agent",
            "SendMessage",
            "ListAgents",
            "SubagentHandback",
            "StructuredOutput",
            "TaskStop",
            "TaskCreate",
            "TaskUpdate",
            "TodoWrite",
            "Monitor",
            "AskUserQuestion",
            "Skill",
            "ExitPlanMode",
            "EnterWorktree",
            "ScheduleWakeup",
            "LSP",
            "CronCreate",
            "Artifact",
            "SomethingNew",
        ];
        let sections = [
            "lucide-message-circle",
            "lucide-list-todo",
            "mcp",
            "lucide-wrench",
            "lucide-folder",
            "lucide-file-text",
            "lucide-layers",
            "lucide-file",
            "lucide-code",
        ];
        for path in tools.into_iter().map(tool_icon).chain(sections) {
            assert!(
                crate::assets::Assets.load(path).unwrap().is_some(),
                "{path}"
            );
        }
    }
}
