use super::*;

impl Automations {
    pub(super) fn render_history(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut table = div()
            .mt(px(12.))
            .rounded(px(6.))
            .overflow_hidden()
            .border_1()
            .border_color(content(0.1));
        if self.runs.is_empty() {
            table = table.child(
                div()
                    .px(px(16.))
                    .py(px(64.))
                    .text_center()
                    .text_size(px(12.))
                    .text_color(content(0.4))
                    .child("This automation has not run yet."),
            );
        } else {
            table = table.child(
                div()
                    .h(px(40.))
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .text_size(px(11.))
                    .text_color(content(0.4))
                    .child(div().flex_1().child("Trigger"))
                    .child(div().w(px(152.)).child("Triggered"))
                    .child(div().w(px(108.)).child("Status"))
                    .child(div().w(px(56.)).text_right().child("Duration")),
            );
            for (index, run) in self.runs.iter().take(100).enumerate() {
                let session = run.session_id.clone();
                let trigger = if run.trigger == "manual" {
                    "Test run".into()
                } else if run.trigger == "event" {
                    automations::model::event_label(
                        run.event_kind.as_deref().unwrap_or(""),
                        run.event.as_deref().unwrap_or(""),
                    )
                } else {
                    self.editor
                        .as_ref()
                        .and_then(|e| e.draft.triggers.as_ref())
                        .and_then(|ts| ts.iter().find(|t| t.kind == "time"))
                        .map(|t| format!("Scheduled · {}", t.label()))
                        .unwrap_or_else(|| "Scheduled".into())
                };
                let (label, color, fill): (&str, gpui::Hsla, gpui::Hsla) = match run.status.as_str()
                {
                    "succeeded" => (
                        "Succeeded",
                        rgb(0x34d399).into(),
                        rgb(0x10b981).alpha(0.12).into(),
                    ),
                    "failed" => (
                        "Failed",
                        rgb(0xfb7185).into(),
                        rgb(0xf43f5e).alpha(0.12).into(),
                    ),
                    "skipped" => (
                        "Skipped",
                        rgb(0xfbbf24).into(),
                        rgb(0xf59e0b).alpha(0.12).into(),
                    ),
                    "cancelled" => ("Cancelled", content(0.5), content(0.08)),
                    "running" => (
                        "Running",
                        rgb(0x60a5fa).into(),
                        rgb(0x3b82f6).alpha(0.12).into(),
                    ),
                    _ => (
                        "Pending",
                        rgb(0x60a5fa).into(),
                        rgb(0x3b82f6).alpha(0.12).into(),
                    ),
                };
                table = table.child(
                    div()
                        .id(("automation-run", index))
                        .role(Role::Button)
                        .aria_label(format!("{trigger}, {label}"))
                        .h(px(44.))
                        .px(px(16.))
                        .border_t_1()
                        .border_color(content(0.08))
                        .flex()
                        .items_center()
                        .gap(px(16.))
                        .text_size(px(13.))
                        .when(session.is_some(), |d| d.hover(|s| s.bg(content(0.05))))
                        .child(div().flex_1().min_w_0().truncate().child(trigger))
                        .child(div().w(px(152.)).text_color(content(0.7)).child(at(
                            if run.scheduled_for == 0 {
                                run.created_at
                            } else {
                                run.scheduled_for
                            },
                        )))
                        .child(
                            div().w(px(108.)).flex().items_center().child(
                                div()
                                    .h(px(20.))
                                    .max_w_full()
                                    .flex_shrink_0()
                                    .px(px(8.))
                                    .flex()
                                    .items_center()
                                    .gap(px(4.))
                                    .rounded_full()
                                    .text_size(px(11.))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(color)
                                    .bg(fill)
                                    .when(run.status == "running", |d| {
                                        d.child(crate::ui::motion::pulse::looping(
                                            SharedString::from(format!(
                                                "automation-run-spinner-{index}"
                                            )),
                                            std::time::Duration::from_secs(1),
                                            crate::ui::motion::pulse::Rate::Smooth,
                                            move |delta| {
                                                icon("loader", 10.)
                                                    .text_color(color)
                                                    .with_transformation(
                                                        gpui::Transformation::rotate(
                                                            gpui::percentage(delta),
                                                        ),
                                                    )
                                            },
                                        ))
                                    })
                                    .child(label),
                            ),
                        )
                        .child(
                            div()
                                .w(px(56.))
                                .text_right()
                                .text_color(content(0.55))
                                .child(duration(run)),
                        )
                        .on_click(cx.listener(move |_, _, _, cx| {
                            if let Some(id) = &session {
                                cx.emit(Event::OpenSession(id.clone()));
                            }
                        })),
                );
                if let Some(error) = &run.error {
                    table = table.child(
                        div()
                            .px(px(16.))
                            .pb(px(8.))
                            .text_size(px(11.))
                            .text_color(content(0.5))
                            .child(error.clone()),
                    );
                }
            }
        }
        div()
            .id("automation-history-scroll")
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
                    .child(section("Run history"))
                    .child(table),
            )
            .into_any_element()
    }
}

pub(super) fn duration(run: &AutomationRun) -> String {
    let live = matches!(run.status.as_str(), "pending" | "running");
    let start = run
        .started_at
        .or_else(|| (run.status != "pending").then_some(run.created_at));
    let end = run
        .completed_at
        .or_else(|| (live && start.is_some()).then(|| monocode_core::sys::time::now_ms() as i64));
    let (Some(start), Some(end)) = (start, end) else {
        return "—".into();
    };
    if end < start {
        return "—".into();
    }
    let minutes = end.saturating_sub(start) / 60_000;
    if minutes < 1 {
        "< 1m".into()
    } else if minutes < 60 {
        format!("{minutes}m")
    } else if minutes % 60 == 0 {
        format!("{}h", minutes / 60)
    } else {
        format!("{}h {}m", minutes / 60, minutes % 60)
    }
}
