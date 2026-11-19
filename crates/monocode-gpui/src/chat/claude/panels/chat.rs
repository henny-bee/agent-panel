//! The "In this chat" tab: one box per kind of thing the chat used, each
//! left out while it has nothing to show.
use super::rows::*;
use super::*;
use crate::chat::claude::activity::{Background, Todo};

impl ClaudePanel {
    pub(super) fn chat(
        &self,
        snapshot: &Snapshot,
        cwd: &str,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let boxes: Vec<AnyElement> = [
            (snapshot.context_used > 0)
                .then(|| context_box(snapshot.context_used, snapshot.context_window)),
            self.todo_box(&snapshot.todos, cx),
            self.mcp_box(&snapshot.mcp, cx),
            self.tools_box(&snapshot.tools, cx),
            self.background_box(&snapshot.background, cx),
            self.files_box(&snapshot.files, cwd, cx),
            self.context_files_box(&snapshot.context, cwd, cx),
            self.skills_box(&snapshot.skills, cx),
        ]
        .into_iter()
        .flatten()
        .collect();
        let mut out = vec![title()];
        if boxes.is_empty() {
            out.push(note("Nothing used in this chat yet."));
        }
        out.extend(boxes);
        out
    }

    fn todo_box(&self, todos: &[Todo], cx: &mut Context<Self>) -> Option<AnyElement> {
        if todos.is_empty() {
            return None;
        }
        let done = todos
            .iter()
            .filter(|t| t.status == TodoStatus::Done)
            .count();
        let rows = todos.iter().map(|todo| {
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
        Some(self.section(
            "todo",
            "lucide-list-todo",
            "To-do",
            format!("{done} of {}", todos.len()),
            rows.collect(),
            cx,
        ))
    }

    fn mcp_box(
        &self,
        mcp: &[(String, Vec<(String, u32)>)],
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if mcp.is_empty() {
            return None;
        }
        let mut rows = Vec::new();
        for (ix, (server, tools)) in mcp.iter().enumerate() {
            let key = format!("mcp:{server}");
            let open = self.open.contains(&key);
            let total = tools.iter().map(|(_, n)| n).sum::<u32>();
            rows.push(
                tool_row(
                    ix > 0,
                    None,
                    Some(open),
                    activity::server_name(server),
                    total,
                )
                .id(SharedString::from(key.clone()))
                .role(Role::Button)
                .aria_expanded(open)
                .rounded_md()
                .hover(|s| s.bg(content(0.05)))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.toggle(key.clone(), cx)))
                .into_any_element(),
            );
            if open {
                rows.extend(tools.iter().map(|(tool, n)| {
                    div()
                        .h(px(24.))
                        .pl(px(34.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family(crate::ui::fonts::mono())
                                .text_size(px(11.))
                                .text_color(content(0.75))
                                .child(tool.clone()),
                        )
                        .child(calls(*n))
                        .into_any_element()
                }));
            }
        }
        Some(self.section("mcp", "mcp", "MCP", mcp.len().to_string(), rows, cx))
    }

    fn tools_box(&self, tools: &[(String, u32)], cx: &mut Context<Self>) -> Option<AnyElement> {
        if tools.is_empty() {
            return None;
        }
        let rows = tools.iter().enumerate().map(|(ix, (name, calls))| {
            tool_row(ix > 0, Some(tool_icon(name)), None, name.clone(), *calls).into_any_element()
        });
        Some(self.section(
            "tools",
            "lucide-wrench",
            "Tools",
            tools.len().to_string(),
            rows.collect(),
            cx,
        ))
    }

    fn background_box(
        &self,
        background: &[Background],
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if background.is_empty() {
            return None;
        }
        let now = monocode_core::sys::time::now_ms() as i64;
        let rows = background.iter().enumerate().map(|(ix, task)| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .min_h(px(34.))
                .py(px(4.))
                .when(ix > 0, |d| d.border_t_1().border_color(stroke()))
                .child(icon(tool_icon(&task.tool), 14.).text_color(content(0.75)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(content(0.88))
                                .child(task.tool.clone()),
                        )
                        .child(
                            div()
                                .truncate()
                                .font_family(crate::ui::fonts::mono())
                                .text_size(px(10.5))
                                .text_color(content(0.45))
                                .child(task.detail.clone()),
                        ),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .font_features(tabular())
                        .text_size(px(11.))
                        .text_color(rgb(RED_400))
                        .child(duration(now - task.started)),
                )
                .into_any_element()
        });
        Some(self.section(
            "background",
            "lucide-activity",
            "Run in Background",
            background.len().to_string(),
            rows.collect(),
            cx,
        ))
    }

    fn files_box(
        &self,
        files: &[Vec<String>; 4],
        cwd: &str,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let touched: usize = files.iter().map(Vec::len).sum();
        if touched == 0 {
            return None;
        }
        let mut rows = Vec::new();
        for (kind, paths) in FILE_KINDS.iter().zip(files) {
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
        Some(self.section(
            "files",
            "lucide-folder",
            "Files",
            touched.to_string(),
            rows,
            cx,
        ))
    }

    fn context_files_box(
        &self,
        context: &[String],
        cwd: &str,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if context.is_empty() {
            return None;
        }
        let rows = context.iter().map(|path| {
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
        Some(self.section(
            "context",
            "lucide-file-text",
            "Context files",
            context.len().to_string(),
            rows.collect(),
            cx,
        ))
    }

    fn skills_box(&self, skills: &[(String, u32)], cx: &mut Context<Self>) -> Option<AnyElement> {
        if skills.is_empty() {
            return None;
        }
        let rows = skills.iter().map(|(name, uses)| {
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
        Some(self.section(
            "skills",
            "lucide-layers",
            "Skills",
            skills.len().to_string(),
            rows.collect(),
            cx,
        ))
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
            .debug_selector(|| format!("claude-box-{key}"))
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

/// The tab's own heading.
fn title() -> AnyElement {
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
        .into_any_element()
}

/// How full the main chat's context is: white, amber past 70%, red past 90%.
fn context_box(used: u64, window: u64) -> AnyElement {
    let ratio = (used as f32 / window.max(1) as f32).min(1.);
    let fill: Hsla = if ratio > 0.9 {
        rgb(RED_400).into()
    } else if ratio > 0.7 {
        rgb(AMBER_400).into()
    } else {
        content(0.9)
    };
    let format = monocode_core::session::context::format_tokens;
    div()
        .mx_3()
        .mb_2()
        .px(px(10.))
        .pt(px(10.))
        .pb(px(12.))
        .rounded_lg()
        .border_1()
        .border_color(stroke())
        .bg(content(0.03))
        .debug_selector(|| "claude-box-context-window".into())
        .id("claude-context-window")
        .role(Role::Meter)
        .aria_label(SharedString::from(format!(
            "Context window, {} of {} tokens",
            format(used),
            format(window)
        )))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(icon("lucide-layers", 15.).text_color(content(0.8)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(content(0.92))
                        .child("Context Window"),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .font_features(tabular())
                        .text_size(px(11.))
                        .text_color(content(0.5))
                        .child(format!("{} / {} tokens", format(used), format(window))),
                ),
        )
        .child(
            div()
                .mt_2()
                .h(px(6.))
                .w_full()
                .overflow_hidden()
                .rounded_full()
                .bg(content(0.08))
                .child(div().h_full().w(relative(ratio)).rounded_full().bg(fill)),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOXES: [&str; 8] = [
        "claude-box-context-window",
        "claude-box-todo",
        "claude-box-mcp",
        "claude-box-tools",
        "claude-box-background",
        "claude-box-files",
        "claude-box-context",
        "claude-box-skills",
    ];

    fn shown(
        cx: &mut gpui::TestAppContext,
        window: gpui::WindowHandle<ClaudePanel>,
    ) -> Vec<&'static str> {
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        })
        .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        BOXES
            .into_iter()
            .filter(|key| visual.debug_bounds(key).is_some())
            .collect()
    }

    #[gpui::test]
    fn every_box_shows_while_it_has_something_and_hides_when_empty(cx: &mut gpui::TestAppContext) {
        let full = Snapshot {
            loaded: true,
            context_used: 116_000,
            context_window: 200_000,
            todos: vec![Todo {
                text: "Split the file".into(),
                status: TodoStatus::Active,
            }],
            mcp: vec![("claude_ai_Gmail".into(), vec![("get_thread".into(), 2)])],
            tools: vec![("Bash".into(), 3)],
            background: vec![Background {
                tool: "Bash".into(),
                detail: "cargo test".into(),
                started: 0,
            }],
            files: [vec!["/p/a.rs".into()], vec![], vec![], vec![]],
            context: vec!["/p/CLAUDE.md".into()],
            skills: vec![("ponytail".into(), 1)],
            ..Snapshot::default()
        };
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                let activity = cx.new(|_| ClaudeActivity::fixed(full));
                cx.new(|cx| ClaudePanel::new(activity, ClaudeTab::Chat, cx))
            })
            .unwrap()
        });
        assert_eq!(shown(cx, window), BOXES);

        // Nothing left running: its box goes, the rest stay.
        let activity = window
            .update(cx, |panel, _, _| panel.activity.clone())
            .unwrap();
        cx.update(|cx| {
            activity.update(cx, |activity, cx| {
                activity.snapshot.background.clear();
                cx.notify();
            })
        });
        let mut left = BOXES.to_vec();
        left.retain(|key| *key != "claude-box-background");
        assert_eq!(shown(cx, window), left);

        cx.update(|cx| {
            activity.update(cx, |activity, cx| {
                activity.snapshot = Snapshot {
                    loaded: true,
                    ..Snapshot::default()
                };
                cx.notify();
            })
        });
        assert!(shown(cx, window).is_empty());
    }
}
