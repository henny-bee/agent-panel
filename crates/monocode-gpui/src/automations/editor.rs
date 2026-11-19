mod history;
mod settings;
use super::*;
use chrono::{Local, TimeZone};

pub(super) fn toggle(id: impl Into<gpui::ElementId>, on: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(Role::Switch)
        .aria_label(if on {
            "Pause automation"
        } else {
            "Enable automation"
        })
        .w(px(28.))
        .h(px(16.))
        .flex_shrink_0()
        .rounded_full()
        .px(px(2.))
        .flex()
        .items_center()
        .when(on, |d| d.bg(accent_color()).justify_end())
        .when(!on, |d| d.bg(content(0.2)))
        .child(div().size(px(12.)).rounded_full().bg(rgb(0xffffff)))
}
fn options(values: &[(&str, &str)]) -> Vec<(String, String)> {
    values
        .iter()
        .map(|(v, l)| (v.to_string(), l.to_string()))
        .collect()
}
pub(super) fn events(kind: &str) -> Vec<(String, String)> {
    options(match kind {
        "time" => &[
            ("hourly", "Hourly"),
            ("daily", "Daily"),
            ("weekdays", "Weekdays"),
            ("weekly", "Weekly"),
        ],
        "github" => &[
            ("draft_opened", "Draft opened"),
            ("pull_request_opened", "Pull request opened"),
            ("issue_opened", "Issue opened"),
        ],
        "gitlab" => &[
            ("merge_request_opened", "Merge request opened"),
            ("issue_opened", "Issue opened"),
        ],
        "linear" => &[("issue_created", "Issue created")],
        "jira" => &[("issue_created", "Issue appeared")],
        "azuredevops" => &[
            ("pull_request_appeared", "Pull request appeared"),
            ("work_item_appeared", "Work item appeared"),
        ],
        _ => &[],
    })
}
fn section(label: &'static str) -> gpui::Div {
    div()
        .px(px(4.))
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(content(0.5))
        .child(label)
}
impl Automations {
    fn select_control(
        &self,
        id: impl Into<gpui::ElementId>,
        label: String,
        field: Choice,
        values: Vec<(String, String)>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let anchor = AnchorSlot::default();
        let captured = anchor.clone();
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label.clone())
            .relative()
            .h(px(28.))
            .px(px(8.))
            .rounded(px(6.))
            .bg(content(0.1))
            .flex()
            .items_center()
            .gap(px(4.))
            .text_size(px(12.))
            .text_color(content(1.))
            .hover(|s| s.bg(content(0.14)))
            .child(anchor.probe())
            .when(matches!(field, Choice::Access), |d| {
                let mode = self
                    .editor
                    .as_ref()
                    .map(|e| e.draft.runtime_mode.as_str())
                    .unwrap_or("auto");
                d.child(icon(
                    match mode {
                        "supervised" => "lock",
                        "auto-accept-edits" => "pencil",
                        "full-access" => "shield",
                        _ => "sparkles",
                    },
                    14.,
                ))
            })
            .child(label)
            .child(icon("chevron-down", 12.).text_color(content(0.45)))
            .on_click(cx.listener(move |this, _, _, cx| {
                if matches!(field, Choice::Conversation)
                    && this
                        .editor
                        .as_ref()
                        .is_some_and(|e| e.draft.workspace_mode == "worktree")
                {
                    return;
                }
                this.choice = Some(ChoiceMenu {
                    field: field.clone(),
                    options: values.clone(),
                    anchor: captured.clone(),
                });
                this.trigger_menu = None;
                this.models = None;
                this.project_picker = None;
                cx.notify();
            }))
    }
    pub(super) fn apply_choice(&mut self, field: Choice, value: String, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        match field {
            Choice::Access => editor.draft.runtime_mode = value,
            Choice::Workspace => {
                editor.draft.workspace_mode = value;
                editor.draft.worktree_cwd.clear();
                if editor.draft.workspace_mode == "worktree" {
                    editor.draft.reuse_session = false;
                }
            }
            Choice::Conversation => editor.draft.reuse_session = value == "reuse",
            Choice::Folder => editor.draft.session_folder_id = value,
            Choice::Grace => editor.draft.missed_run_grace_minutes = value.parse().unwrap_or(720),
            Choice::ModelSetting(key) => {
                set_control_value(&mut editor.draft, key, value);
            }
            Choice::AddTrigger => {
                if let Some((kind, event)) = value.split_once(':') {
                    let triggers = editor.draft.triggers.get_or_insert_with(Vec::new);
                    if triggers.len() < 20 {
                        triggers.push(AutomationTrigger::new(kind, event));
                    }
                }
            }
            Choice::TriggerEvent(id) | Choice::Schedule(id) => {
                if let Some(trigger) = editor
                    .draft
                    .triggers
                    .as_mut()
                    .and_then(|ts| ts.iter_mut().find(|t| t.id == id))
                {
                    trigger.event = value.clone();
                    if trigger.kind == "time" {
                        trigger.schedule_kind = value;
                    }
                }
            }
            Choice::Minute(id) => {
                if let Some(trigger) = editor
                    .draft
                    .triggers
                    .as_mut()
                    .and_then(|ts| ts.iter_mut().find(|t| t.id == id))
                {
                    trigger.minute = value.parse().unwrap_or(0);
                }
            }
            Choice::Time(id) => {
                if let Some(trigger) = editor
                    .draft
                    .triggers
                    .as_mut()
                    .and_then(|ts| ts.iter_mut().find(|t| t.id == id))
                {
                    trigger.time = value.clone();
                }
                if let Some(fields) = editor.fields.iter().find(|f| f.id == id) {
                    fields
                        .time
                        .update(cx, |input, cx| input.set_text(value, cx));
                }
            }
            Choice::Weekday(id) => {
                if let Some(trigger) = editor
                    .draft
                    .triggers
                    .as_mut()
                    .and_then(|ts| ts.iter_mut().find(|t| t.id == id))
                {
                    trigger.day_of_week = value.parse().unwrap_or(1);
                }
            }
        }
        editor.dirty = true;
        self.choice = None;
        self.rebuild_trigger_fields(cx);
        cx.notify();
    }
    pub(super) fn render_model_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(editor) = &self.editor else {
            return div().into_any_element();
        };
        if cx
            .try_global::<crate::ui::preferences::Preferences>()
            .is_some_and(|p| p.model_controls == "beside")
            || self.model_controls.is_empty()
        {
            return div().into_any_element();
        }
        div()
            .p(px(8.))
            .border_b_1()
            .border_color(content(0.08))
            .flex()
            .flex_col()
            .gap(px(4.))
            .children(
                self.model_controls
                    .iter()
                    .enumerate()
                    .map(|(index, control)| {
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(12.))
                            .text_size(px(12.))
                            .text_color(content(0.7))
                            .child(control.name.clone())
                            .child(self.select_control(
                                ("automation-model-menu-setting", index),
                                control_label(control, &editor.draft),
                                Choice::ModelSetting(control.id.clone()),
                                control.choices.clone(),
                                cx,
                            ))
                    }),
            )
            .into_any_element()
    }
    pub(super) fn render_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let editor = self.editor.as_ref().expect("editor");
        let draft = &editor.draft;
        let valid = !editor.name.read(cx).content.trim().is_empty()
            && !editor.prompt.read(cx).content.trim().is_empty()
            && !draft.cwd.trim().is_empty();
        let mut actions = div().flex_shrink_0().flex().items_center().gap(px(8.));
        if !editor.saved || editor.dirty {
            actions = actions.child(
                button(
                    "automation-reset",
                    if editor.saved && editor.dirty {
                        "Reset"
                    } else {
                        "Cancel"
                    },
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(id) = this.selected.clone() {
                        this.select(id, cx);
                    } else {
                        this.show_gallery(cx);
                    }
                })),
            );
        }
        if editor.saved {
            actions = actions.child(
                button(
                    "automation-run",
                    if self.running {
                        "Starting…"
                    } else {
                        "Run now"
                    },
                )
                .child(icon("queue-play", 13.))
                .on_click(cx.listener(|this, _, _, cx| this.run(cx))),
            );
        }
        let disabled = !valid || self.saving || (editor.saved && !editor.dirty);
        actions = actions.child(
            button(
                "automation-save",
                if self.saving {
                    "Saving…"
                } else if editor.saved {
                    "Save"
                } else {
                    "Create"
                },
            )
            .h(px(26.))
            .bg(content(1.))
            .text_color(shell_fill())
            .font_weight(gpui::FontWeight::MEDIUM)
            .when(disabled, |d| d.opacity(0.4))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !disabled {
                    this.save(cx);
                }
            })),
        );
        let header = div()
            .flex_shrink_0()
            .border_b_1()
            .border_color(content(0.07))
            .child(
                div()
                    .w_full()
                    .max_w(px(1024.))
                    .mx_auto()
                    .px(px(32.))
                    .pt(px(20.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(px(24.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child(editor.name.clone()),
                                    ),
                            )
                            .child(actions),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .text_size(px(12.))
                            .text_color(content(0.5))
                            .child(toggle("automation-enabled", draft.enabled).on_click(
                                cx.listener(|this, _, _, cx| {
                                    if let Some(e) = &mut this.editor {
                                        e.draft.enabled = !e.draft.enabled;
                                        e.dirty = true;
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(if draft.enabled { "Active" } else { "Inactive" })
                            .child(div().ml(px(8.)).h(px(12.)).w(px(1.)).bg(content(0.15)))
                            .child(
                                div()
                                    .id("automation-project")
                                    .relative()
                                    .min_w_0()
                                    .role(Role::Button)
                                    .aria_label("Choose automation project")
                                    .px(px(8.))
                                    .h(px(26.))
                                    .rounded(px(6.))
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .text_size(px(12.))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(content(0.9))
                                    .hover(|s| s.bg(content(0.05)))
                                    .child(self.project_anchor.probe())
                                    .child(self.project_mark(&draft.cwd, 14.))
                                    .child(div().min_w_0().truncate().child(
                                        if draft.cwd.is_empty() {
                                            "Choose project".into()
                                        } else {
                                            self.project_label(&draft.cwd)
                                        },
                                    ))
                                    .child(icon("chevron-down", 12.).text_color(content(0.45)))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.choose_project(window, cx)
                                    })),
                            )
                            .when(editor.saved, |d| {
                                d.child(
                                    icon_button(
                                        "automation-delete-menu",
                                        "Delete automation",
                                        "trash",
                                    )
                                    .on_click(cx.listener(
                                        |this, _, window, cx| this.ask_delete(window, cx),
                                    )),
                                )
                            }),
                    )
                    .child(if editor.saved {
                        div().h(px(36.)).flex().gap(px(16.)).children(
                            [("Settings", false), ("Run history", true)].map(|(label, history)| {
                                div()
                                    .id(if history {
                                        "automation-history-tab"
                                    } else {
                                        "automation-settings-tab"
                                    })
                                    .role(Role::Tab)
                                    .aria_label(label)
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .border_b_2()
                                    .border_color(if self.history == history {
                                        content(0.8)
                                    } else {
                                        content(0.)
                                    })
                                    .text_size(px(12.))
                                    .text_color(content(if self.history == history {
                                        0.9
                                    } else {
                                        0.5
                                    }))
                                    .child(label)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.history = history;
                                        this.choice = None;
                                        cx.notify();
                                    }))
                            }),
                        )
                    } else {
                        div().h(px(20.))
                    }),
            );
        let panel = if self.history && editor.saved {
            self.render_history(cx)
        } else {
            self.render_settings(cx)
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(header)
            .child(panel)
            .into_any_element()
    }
}
fn control_label(
    control: &monocode_core::provider::catalog::ModelControl,
    draft: &AutomationUpsert,
) -> String {
    let value = control_value(control, draft);
    let label = control
        .choices
        .iter()
        .find(|(v, _)| v == value)
        .map(|(_, l)| l)
        .unwrap_or(value);
    match label.as_str() {
        "low" => "Low".into(),
        "medium" => "Medium".into(),
        "high" => "High".into(),
        "xhigh" => "Extra high".into(),
        "minimal" => "Minimal".into(),
        "none" => "None".into(),
        _ => label.clone(),
    }
}
fn control_value<'a>(
    control: &'a monocode_core::provider::catalog::ModelControl,
    draft: &'a AutomationUpsert,
) -> &'a String {
    let aliases: &[&str] = match control.id.as_str() {
        "reasoningEffort" | "reasoning_effort" | "effort" | "reasoning" => {
            &["reasoningEffort", "reasoning_effort", "effort", "reasoning"]
        }
        "serviceTier" | "service_tier" => &["serviceTier", "service_tier"],
        _ => &[],
    };
    aliases
        .iter()
        .find_map(|key| draft.model_settings.get(*key))
        .or_else(|| draft.model_settings.get(&control.id))
        .unwrap_or(&control.current)
}
fn set_control_value(draft: &mut AutomationUpsert, key: String, value: String) {
    let (canonical, aliases): (&str, &[&str]) = match key.as_str() {
        "reasoningEffort" | "reasoning_effort" | "effort" | "reasoning" => (
            "reasoningEffort",
            &["reasoningEffort", "reasoning_effort", "effort", "reasoning"],
        ),
        "serviceTier" | "service_tier" => ("serviceTier", &["serviceTier", "service_tier"]),
        _ => (key.as_str(), &[]),
    };
    for alias in aliases {
        draft.model_settings.remove(*alias);
    }
    draft.model_settings.insert(canonical.to_string(), value);
}
fn gmt_offset() -> String {
    let minutes = Local::now().offset().local_minus_utc() / 60;
    let sign = if minutes >= 0 { '+' } else { '-' };
    let absolute = minutes.unsigned_abs();
    if absolute.is_multiple_of(60) {
        format!("GMT{sign}{}", absolute / 60)
    } else {
        format!("GMT{sign}{}:{:02}", absolute / 60, absolute % 60)
    }
}
fn at(timestamp: i64) -> String {
    if timestamp <= 0 {
        return "—".into();
    }
    Local
        .timestamp_millis_opt(timestamp)
        .single()
        .map(|d| d.format("%-d %b, %H:%M").to_string())
        .unwrap_or_else(|| "—".into())
}
