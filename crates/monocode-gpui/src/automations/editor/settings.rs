use super::*;

pub(super) fn settings_row(
    label: &'static str,
    hint: &'static str,
    control: impl IntoElement,
) -> gpui::Div {
    div()
        .min_h(px(54.))
        .px(px(16.))
        .py(px(10.))
        .border_b_1()
        .border_color(content(0.07))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(content(0.75))
                        .child(label),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(content(0.4))
                        .child(hint),
                ),
        )
        .child(control)
}

impl Automations {
    pub(super) fn render_trigger(
        &self,
        trigger: &AutomationTrigger,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fields = self
            .editor
            .as_ref()
            .and_then(|e| e.fields.iter().find(|f| f.id == trigger.id));
        let id = trigger.id.clone();
        let mut sentence = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(px(6.))
            .gap_y(px(4.))
            .text_size(px(13.))
            .text_color(content(0.7));
        if trigger.kind == "time" {
            let prefix = match trigger.schedule_kind.as_str() {
                "hourly" => "Every hour at",
                "daily" => "Every day at",
                "weekly" => "Every week on",
                _ => "Every weekday at",
            };
            let frequency_anchor = AnchorSlot::default();
            let captured = frequency_anchor.clone();
            let schedule_id = id.clone();
            // The schedule reads as one sentence. Keep frequency editable
            // without adding an extra pill to the sentence.
            sentence = sentence.child(
                div()
                    .id(("trigger-frequency", index))
                    .relative()
                    .role(Role::Button)
                    .aria_label(format!("Schedule: {prefix}"))
                    .rounded(px(3.))
                    .hover(|s| s.bg(content(0.05)))
                    .child(frequency_anchor.probe())
                    .child(prefix)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.choice = Some(ChoiceMenu {
                            field: Choice::Schedule(schedule_id.clone()),
                            options: events("time"),
                            anchor: captured.clone(),
                        });
                        cx.notify();
                    })),
            );
            if trigger.schedule_kind == "weekly" {
                sentence = sentence
                    .child(
                        self.select_control(
                            ("trigger-day", index),
                            automations::model::WEEKDAYS
                                .get(trigger.day_of_week as usize)
                                .unwrap_or(&"Monday")
                                .to_string(),
                            Choice::Weekday(id.clone()),
                            automations::model::WEEKDAYS
                                .iter()
                                .enumerate()
                                .map(|(i, s)| (i.to_string(), s.to_string()))
                                .collect(),
                            cx,
                        ),
                    )
                    .child("at");
            }
            let mut preview_trigger = trigger.clone();
            if let Some(fields) = fields {
                preview_trigger.time = fields.time.read(cx).content.to_string();
            }
            if trigger.schedule_kind == "hourly" {
                sentence = sentence.child(
                    self.select_control(
                        ("trigger-minute", index),
                        format!(":{:02}", trigger.minute),
                        Choice::Minute(id.clone()),
                        (0..60)
                            .map(|m| (m.to_string(), format!(":{m:02}")))
                            .collect(),
                        cx,
                    ),
                );
            } else {
                let time = preview_trigger.time.clone();
                let mut times = (0..24)
                    .flat_map(|hour| {
                        [0, 30].map(move |minute| {
                            let value = format!("{hour:02}:{minute:02}");
                            (value.clone(), value)
                        })
                    })
                    .collect::<Vec<_>>();
                if !times.iter().any(|(value, _)| value == &time) {
                    times.insert(0, (time.clone(), time.clone()));
                }
                sentence = sentence.child(self.select_control(
                    ("trigger-time", index),
                    time,
                    Choice::Time(id.clone()),
                    times,
                    cx,
                ));
            }
            sentence = sentence
                .child(div().text_color(content(0.45)).child(gmt_offset()))
                .child(
                    div()
                        .ml(px(4.))
                        .text_color(content(0.35))
                        .child(next_run_preview(
                            preview_trigger.next_at(monocode_core::sys::time::now_ms() as i64),
                        )),
                );
        } else {
            sentence = sentence
                .child(self.select_control(
                    ("trigger-event", index),
                    automations::model::event_label(&trigger.kind, &trigger.event),
                    Choice::TriggerEvent(id.clone()),
                    events(&trigger.kind),
                    cx,
                ))
                .child("by");
            if let Some(fields) = fields {
                sentence = sentence.child(
                    div()
                        .w(px(80.))
                        .h(px(28.))
                        .rounded(px(6.))
                        .bg(content(0.1))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .child(fields.actor.clone()),
                );
            }
        }
        let row = div()
            .group("automation-trigger")
            .min_h(px(40.))
            .py(px(4.))
            .flex()
            .items_center()
            .gap(px(10.))
            .child(trigger_mark(&trigger.kind, 14.))
            .child(sentence)
            .child(
                div()
                    .id(SharedString::from(format!("remove-trigger-{id}")))
                    .role(Role::Button)
                    .aria_label("Remove trigger")
                    .size(px(28.))
                    .flex_shrink_0()
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(content(0.35))
                    .opacity(0.)
                    .group_hover("automation-trigger", |s| s.opacity(1.))
                    .hover(|s| s.bg(content(0.08)).text_color(content(1.)))
                    .child(icon("close", 14.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(editor) = &mut this.editor {
                            editor
                                .draft
                                .triggers
                                .get_or_insert_with(Vec::new)
                                .retain(|t| t.id != id);
                            editor.dirty = true;
                        }
                        this.rebuild_trigger_fields(cx);
                        cx.notify();
                    })),
            );
        let mut result = div().child(row);
        if trigger.kind != "time"
            && let Some(fields) = fields
        {
            result = result.child(
                div()
                    .pl(px(24.))
                    .pb(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(11.))
                    .text_color(content(0.4))
                    .child("Repositories")
                    .child(div().flex_1().max_w(px(340.)).child(fields.repos.clone())),
            );
        }
        result.into_any_element()
    }

    pub(super) fn render_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let editor = self.editor.as_ref().expect("editor");
        let draft = &editor.draft;
        let triggers = draft.triggers.clone().unwrap_or_default();
        let triggers_section = div().child(section("Triggers")).child(
            div()
                .mt(px(12.))
                .rounded(px(6.))
                .border_1()
                .border_color(content(0.1))
                .when(!triggers.is_empty(), |d| {
                    d.child(
                        div().px(px(12.)).py(px(6.)).children(
                            triggers
                                .iter()
                                .enumerate()
                                .map(|(i, t)| self.render_trigger(t, i, cx)),
                        ),
                    )
                    .child(div().h(px(1.)).bg(content(0.08)))
                })
                .child(
                    div()
                        .id("automation-add-trigger")
                        .relative()
                        .role(Role::Button)
                        .aria_label("Add Trigger")
                        .h(px(48.))
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(px(13.))
                        .text_color(content(0.55))
                        .hover(|s| s.bg(content(0.04)).text_color(content(1.)))
                        .child(self.trigger_anchor.probe())
                        .child(icon("plus", 14.))
                        .child("Add Trigger")
                        .when(triggers.len() >= 20, |d| d.opacity(0.4))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_trigger_menu(window, cx);
                        })),
                ),
        );
        let kind = Kind::from_id(&draft.harness);
        let model = monocode_core::store::native_model(&draft.model, &draft.harness);
        let access = match draft.runtime_mode.as_str() {
            "supervised" => "Supervised",
            "auto-accept-edits" => "Auto-accept edits",
            "full-access" => "Full access",
            _ => "Auto",
        };
        let beside = cx
            .try_global::<crate::ui::preferences::Preferences>()
            .is_some_and(|p| p.model_controls == "beside");
        let effort_label = (!beside)
            .then(|| {
                self.model_controls
                    .iter()
                    .find(|control| {
                        matches!(
                            control.id.as_str(),
                            "effort" | "reasoning_effort" | "reasoningEffort" | "reasoning"
                        )
                    })
                    .map(|control| control_label(control, draft))
            })
            .flatten();
        let mut controls = div()
            .px(px(8.))
            .pb(px(8.))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(4.))
            .child(
                div()
                    .id("automation-model")
                    .relative()
                    .role(Role::Button)
                    .aria_label("Choose automation model")
                    .px(px(6.))
                    .h(px(26.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_size(px(11.))
                    .text_color(content(1.))
                    .bg(content(0.1))
                    .hover(|s| s.bg(content(0.14)))
                    .child(self.model_anchor.probe())
                    .child(provider_icon(kind.id(), 16.))
                    .child(cached_model_name(kind, model.as_deref()))
                    .when_some(effort_label, |d, effort| {
                        d.child(div().text_color(content(0.5)).child(effort))
                    })
                    .child(icon("chevron-down", 12.).text_color(content(0.5)))
                    .on_click(cx.listener(|this, _, window, cx| this.choose_model(window, cx))),
            );
        for (index, control) in self.model_controls.iter().enumerate().filter(|_| beside) {
            controls = controls.child(
                self.select_control(
                    ("automation-model-setting", index),
                    control_label(control, draft),
                    Choice::ModelSetting(control.id.clone()),
                    control.choices.clone(),
                    cx,
                )
                .h(px(26.))
                .px(px(6.))
                .text_size(px(11.)),
            );
        }
        if draft.harness != "fx" {
            controls = controls.child(
                self.select_control(
                    "automation-access",
                    access.into(),
                    Choice::Access,
                    options(&[
                        ("supervised", "Supervised"),
                        ("auto-accept-edits", "Auto-accept edits"),
                        ("auto", "Auto"),
                        ("full-access", "Full access"),
                    ]),
                    cx,
                )
                .h(px(26.))
                .px(px(6.))
                .text_size(px(11.)),
            );
        }
        let instructions = div()
            .child(section("Instructions"))
            .child(
                div()
                    .relative()
                    .mt(px(12.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(content(0.1))
                    .bg(content(0.03))
                    .child(editor.completion.read(cx).composer_probe())
                    .child(
                        div()
                            .px(px(12.))
                            .py(px(12.))
                            .child(editor.completion.clone()),
                    )
                    .child(controls),
            )
            .child(
                div()
                    .mt(px(8.))
                    .px(px(4.))
                    .text_size(px(11.))
                    .text_color(content(0.35))
                    .child("Skills, @file references, and built-in commands work here."),
            );
        let mut folder_options = vec![(String::new(), "No folder".into())];
        folder_options.extend(self.folders.iter().map(|f| (f.id.clone(), f.name.clone())));
        if !draft.session_folder_id.is_empty()
            && !folder_options
                .iter()
                .any(|(id, _)| id == &draft.session_folder_id)
        {
            folder_options.push((
                draft.session_folder_id.clone(),
                draft.session_folder_id.clone(),
            ));
        }
        let folder_label = folder_options
            .iter()
            .find(|(id, _)| id == &draft.session_folder_id)
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| "No folder".into());
        let workspace = self.select_control(
            "automation-workspace",
            if draft.workspace_mode == "worktree" {
                "Fresh worktree"
            } else if draft.workspace_mode == "existing" {
                "Existing worktree"
            } else {
                "Current"
            }
            .into(),
            Choice::Workspace,
            options(&[("current", "Current"), ("worktree", "Fresh worktree")]),
            cx,
        );
        let conversation = self
            .select_control(
                "automation-conversation",
                if draft.reuse_session {
                    "Continue last"
                } else {
                    "Start fresh"
                }
                .into(),
                Choice::Conversation,
                if draft.workspace_mode == "worktree" {
                    vec![("fresh".into(), "Start fresh".into())]
                } else {
                    options(&[("fresh", "Start fresh"), ("reuse", "Continue last")])
                },
                cx,
            )
            .when(draft.workspace_mode == "worktree", |d| d.opacity(0.5));
        let session = div().child(section("Session")).child(
            div()
                .mt(px(12.))
                .rounded(px(6.))
                .overflow_hidden()
                .border_1()
                .border_color(content(0.1))
                .child(settings_row(
                    "Working copy",
                    "This repo, or a fresh worktree",
                    workspace,
                ))
                .child(settings_row(
                    "Conversation",
                    "New chat, or continue the last run",
                    conversation,
                ))
                .child(
                    settings_row(
                        "Session folder",
                        "Where runs appear in the sidebar",
                        self.select_control(
                            "automation-folder",
                            folder_label,
                            Choice::Folder,
                            folder_options,
                            cx,
                        ),
                    )
                    .border_b_0(),
                ),
        );
        let grace_options = options(&[
            ("0", "Do not catch up"),
            ("30", "30 minutes"),
            ("120", "2 hours"),
            ("720", "12 hours"),
            ("1440", "24 hours"),
        ]);
        let grace = grace_options
            .iter()
            .find(|(id, _)| *id == draft.missed_run_grace_minutes.to_string())
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| format!("{} minutes", draft.missed_run_grace_minutes));
        let advanced = div()
            .rounded(px(6.))
            .border_1()
            .border_color(content(0.1))
            .child(
                div()
                    .id("automation-advanced")
                    .role(Role::Button)
                    .aria_label("Advanced")
                    .min_h(px(56.))
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(content(0.75))
                                    .child("Advanced"),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(content(0.4))
                                    .child("Catch-up window for missed runs"),
                            ),
                    )
                    .child(icon(
                        if self.advanced {
                            "chevron-up"
                        } else {
                            "chevron-down"
                        },
                        14.,
                    ))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.advanced = !this.advanced;
                        cx.notify();
                    })),
            )
            .when(self.advanced, |d| {
                d.child(settings_row(
                    "Missed-run grace",
                    "Catch up if a scheduled run was missed",
                    self.select_control(
                        "automation-grace",
                        grace,
                        Choice::Grace,
                        grace_options,
                        cx,
                    ),
                ))
            });
        div()
            .id("automation-settings-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                div()
                    .w_full()
                    .max_w(px(1024.))
                    .mx_auto()
                    .px(px(32.))
                    .pt(px(20.))
                    .pb(px(40.))
                    .flex()
                    .flex_col()
                    .gap(px(32.))
                    .child(triggers_section)
                    .child(instructions)
                    .child(session)
                    .child(advanced),
            )
            .into_any_element()
    }
}

pub(super) fn next_run_preview(timestamp: i64) -> String {
    let Some(date) = Local
        .timestamp_millis_opt(timestamp)
        .single()
        .filter(|_| timestamp > 0)
    else {
        return "Next run —".into();
    };
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut zone = gmt_offset();
    #[cfg(unix)]
    {
        let seconds = (timestamp / 1000) as libc::time_t;
        let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
        // Use the system abbreviation (BST, PDT, etc.) for the scheduled date.
        if !unsafe { libc::localtime_r(&seconds, local.as_mut_ptr()) }.is_null() {
            let local = unsafe { local.assume_init() };
            let mut name = [0u8; 64];
            let length = unsafe {
                libc::strftime(name.as_mut_ptr().cast(), name.len(), c"%Z".as_ptr(), &local)
            };
            if length > 0 {
                zone = String::from_utf8_lossy(&name[..length]).into_owned();
            }
        }
    }
    format!("Next run {} {zone}", date.format("%a %-d %b, %H:%M"))
}
