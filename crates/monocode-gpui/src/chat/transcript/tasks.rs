//! Native task snapshots and delegated runs, driven only by structured metadata.
mod agents;
use super::{
    TranscriptView,
    live_phase::{LiveScroll, live_phase},
};
use crate::ui::theme::*;
use crate::{
    chat::markdown::{self, model},
    ui::motion::reveal::{Disclosure, Motion},
};
use agents::agent;
use gpui::{
    AnyElement, Context, Pixels, Role, SharedString, Transformation, div, percentage, prelude::*,
    px, rgb,
};
use monocode_core::session::tool::{
    ToolCall, ToolState,
    tasks::{AgentRun, AgentStep, Panel, TaskList, TaskStatus},
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    time::{Duration, Instant},
};

pub(super) fn render(
    view: &mut TranscriptView,
    call: &ToolCall,
    start: usize,
    width: Option<Pixels>,
    cx: &mut Context<TranscriptView>,
) -> Option<AnyElement> {
    Some(match call.panel()? {
        Panel::Tasks(list) => task_list(&list, start),
        Panel::Agent(run) => agent(view, call, &run, start, width, cx),
    })
}
fn task_list(list: &TaskList, start: usize) -> AnyElement {
    div()
        .id(("task-progress", start))
        .role(Role::Group)
        .aria_label(format!("Task progress: {}", list.progress()))
        .w_full()
        .min_w_0()
        .my_1()
        .rounded(px(10.))
        .border_1()
        .border_color(content(0.1))
        .bg(content(0.035))
        .child(
            div()
                .px_2p5()
                .py_2()
                .border_b_1()
                .border_color(content(0.1))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(icon("list-bullet", 16.).text_color(content(0.45)))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(12.))
                                .text_color(content(0.85))
                                .child("Tasks"),
                        )
                        .child(
                            div()
                                .rounded_full()
                                .px_2()
                                .py_0p5()
                                .bg(content(0.07))
                                .text_size(px(10.))
                                .text_color(content(0.5))
                                .child(list.progress()),
                        ),
                )
                .when(!list.explanation.is_empty(), |d| {
                    d.child(
                        div()
                            .mt_1()
                            .text_size(px(12.))
                            .text_color(content(0.5))
                            .child(list.explanation.clone()),
                    )
                }),
        )
        .child(
            div()
                .id(("task-items", start))
                .max_h(px(320.))
                .overflow_y_scroll()
                .py_1()
                .when(list.items.is_empty(), |d| {
                    d.child(
                        div()
                            .px_2p5()
                            .py_1()
                            .text_size(px(12.))
                            .text_color(content(0.5))
                            .child("No tasks"),
                    )
                })
                .children(
                    list.items
                        .iter()
                        .take(100)
                        .enumerate()
                        .map(|(index, task)| {
                            let (glyph, color) = match task.status {
                                TaskStatus::Completed => ("check", rgb(0x6ee7b7).into()),
                                TaskStatus::InProgress => ("circle-dashed", rgb(0x7dd3fc).into()),
                                TaskStatus::Cancelled => ("minus", content(0.35)),
                                TaskStatus::Pending => ("context-ring", content(0.35)),
                            };
                            div()
                                .id(("task-item", index))
                                .role(Role::Label)
                                .aria_label(format!("{}: {}", task.status.label(), task.text))
                                .flex()
                                .items_start()
                                .gap_2p5()
                                .px_2p5()
                                .py_1p5()
                                .child(
                                    icon(glyph, 14.)
                                        .mt(px(2.))
                                        .text_color(color)
                                        .flex_shrink_0(),
                                )
                                .child(
                                    div()
                                        .min_w_0()
                                        .flex_1()
                                        .text_size(px(13.))
                                        .line_height(px(18.))
                                        .text_color(content(match task.status {
                                            TaskStatus::Completed | TaskStatus::Cancelled => 0.4,
                                            TaskStatus::InProgress => 0.85,
                                            _ => 0.6,
                                        }))
                                        .when(
                                            matches!(
                                                task.status,
                                                TaskStatus::Completed | TaskStatus::Cancelled
                                            ),
                                            |d| d.line_through(),
                                        )
                                        .child(task.text.clone()),
                                )
                        }),
                )
                .when(list.items.len() > 100, |d| {
                    d.child(
                        div()
                            .px_2p5()
                            .py_1()
                            .text_size(px(11.))
                            .text_color(content(0.45))
                            .child(format!("Showing 100 of {} tasks", list.items.len())),
                    )
                }),
        )
        .into_any_element()
}

/// Disclosure overrides belong to the run/step identity, independently of the
/// surrounding work fold. Updating a streamed run never resets the reader's choice.
#[derive(Default)]
pub(super) struct AgentPanels {
    parts: HashMap<String, PanelPart>,
    scrolls: HashMap<String, LiveScroll>,
    groups: HashMap<String, StepGroup>,
    arrivals: HashMap<String, StepArrival>,
    returned_at: Option<Instant>,
}
struct StepGroup {
    seen: HashSet<String>,
    next: Instant,
}
struct StepArrival {
    motion: Motion,
    source: usize,
}
#[derive(Default)]
struct PanelPart {
    open: Option<bool>,
    motion: Option<Motion>,
    source: usize,
}
impl AgentPanels {
    pub(super) fn animating(&self) -> bool {
        !self.arrivals.is_empty() || self.parts.values().any(|part| part.motion.is_some())
    }
    pub(super) fn animate(&mut self) -> Vec<usize> {
        let now = Instant::now();
        let mut changed = Vec::new();
        for part in self.parts.values_mut() {
            if let Some(motion) = &part.motion {
                changed.push(part.source);
                if motion.finished(now) {
                    part.motion = None;
                }
            }
        }
        self.arrivals.retain(|_, arrival| {
            changed.push(arrival.source);
            !arrival.motion.finished(now)
        });
        changed.sort_unstable();
        changed.dedup();
        changed
    }
    pub(super) fn settle(&mut self) {
        self.arrivals.clear();
        for part in self.parts.values_mut() {
            part.motion = None;
        }
        self.returned_at = Some(Instant::now());
        for scroll in self.scrolls.values_mut() {
            scroll.enable(false);
        }
    }
    fn observe_steps(&mut self, key: &str, steps: &[&AgentStep], source: usize, animate: bool) {
        let now = Instant::now();
        let returning = self
            .returned_at
            .is_some_and(|returned| now.duration_since(returned) < Duration::from_millis(400));
        let mounted = self.groups.contains_key(key);
        let group = self
            .groups
            .entry(key.to_owned())
            .or_insert_with(|| StepGroup {
                seen: HashSet::new(),
                next: now,
            });
        for step in steps {
            if !group.seen.insert(step.id.clone()) {
                continue;
            }
            if !mounted || !animate || returning {
                continue;
            }
            let queued = group.next.max(now);
            let wait = queued.duration_since(now).as_secs_f32();
            let pace = (0.48 * ((2. - wait) / (2. - 0.96)).min(1.)).max(0.16);
            let mut motion = Motion::new(true, true, now);
            motion.start = queued;
            motion.duration = Duration::from_secs_f32(pace);
            group.next = queued + motion.duration;
            self.arrivals
                .insert(format!("{key}:{}", step.id), StepArrival { motion, source });
        }
    }
    pub(super) fn part_open(&self, key: &str, default: bool) -> bool {
        self.parts
            .get(key)
            .and_then(|part| part.open)
            .unwrap_or(default)
    }
    fn open(&self, key: &str, default: bool) -> bool {
        self.part_open(key, default)
    }
    fn override_open(&self, key: &str) -> bool {
        self.parts.get(key).and_then(|part| part.open) == Some(true)
    }
    fn visible(&self, key: &str, open: bool) -> bool {
        open || self
            .parts
            .get(key)
            .is_some_and(|part| part.motion.is_some())
    }
    fn progress(&self, key: &str, open: bool) -> (f32, f32, f32) {
        self.parts
            .get(key)
            .and_then(|part| part.motion.as_ref())
            .map_or(if open { (1., 1., 1.) } else { (0., 0., 0.) }, |motion| {
                let now = Instant::now();
                let (height, opacity) = motion.value(now);
                (height, opacity, motion.rotation(now))
            })
    }
}

impl TranscriptView {
    pub(super) fn toggle_agent_part(
        &mut self,
        key: String,
        open: bool,
        start: usize,
        cx: &mut Context<Self>,
    ) {
        let part = self.agent_panels.parts.entry(key).or_default();
        part.open = Some(open);
        part.source = start;
        if self.visible && !cx.reduce_motion() {
            let now = Instant::now();
            part.motion = Some(part.motion.as_ref().map_or_else(
                || Motion::phase(start, open, now),
                |motion| motion.redirect(open, now),
            ));
        } else {
            part.motion = None;
        }
        self.list.pause_following_tail();
        let phase = self
            .rows
            .iter()
            .find(|row| row.key.0 == start && row.in_phase())
            .and_then(|row| row.phase_group());
        for (ix, row) in self.rows.iter().enumerate() {
            if row.key.0 == start
                || matches!(row.body, super::Body::Phase { group, .. } if Some(group) == phase && self.windowed_phase(group))
            {
                self.list.remeasure_items(ix..ix + 1);
            }
        }
        cx.notify();
    }
}

pub(crate) fn prose_summary(text: &str) -> String {
    model::parse_full(text)
        .blocks
        .iter()
        .flat_map(|block| markdown::rows(&block.block))
        .map(|row| markdown::row_text(&row))
        .find(|text| !text.trim().is_empty())
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn markdown_text(
    view: &mut TranscriptView,
    text: &str,
    identity: &str,
    tone: markdown::Tone,
    cx: &mut Context<TranscriptView>,
) -> AnyElement {
    let tree = model::parse_full(text);
    let mut body = div().w_full().min_w_0().flex().flex_col();
    for (index, mut row) in tree
        .blocks
        .iter()
        .flat_map(|block| markdown::rows(&block.block))
        .enumerate()
    {
        if let Some(base) = &view.file_link_base {
            markdown::resolve_file_links(&mut row, base);
        }
        let rendered = if let markdown::Part::Code { fence, lines } = &row.part {
            view.code(fence.clone(), lines.clone(), fence.key as usize, cx)
        } else {
            markdown::body(
                &row,
                format!("agent-md:{identity}:{index}").into(),
                tone,
                None,
                Instant::now(),
            )
        };
        body = body.child(
            div()
                .w_full()
                .min_w_0()
                .flex()
                .pt(px(if index == 0 { 0. } else { row.gap }))
                .pl(px(row.indent.saturating_sub(1) as f32 * 24.))
                .when(row.quote > 0, |d| {
                    d.ml_1().pl_4().border_l_4().border_color(content(0.2))
                })
                .when(row.indent > 0, |d| {
                    d.child(
                        div()
                            .w(px(24.))
                            .flex_shrink_0()
                            .text_color(content(0.78))
                            .child(row.marker.clone().unwrap_or_default()),
                    )
                })
                .child(rendered),
        );
    }
    body.into_any_element()
}
