mod detail;
mod dialogs;
mod discussion;
mod list;
use super::*;
use crate::ui::popover::{Align, Anchor, popover};
use gpui::{
    Animation, AnimationExt, AnyElement, ClipboardItem, CursorStyle, Div, FontWeight, Hsla,
    MouseButton, Pixels, Role, SharedString, Stateful, Window, canvas, div, list, prelude::*, px,
    rgb,
};

fn action(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Stateful<Div> {
    plain_action(id, label).hover(|s| s.bg(content(0.07)).text_color(content(1.)))
}
/// An [`action`] without its hover, for callers that set their own.
fn plain_action(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Stateful<Div> {
    let id: SharedString = id.into();
    let label = label.into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .h(px(28.))
        .px(px(10.))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .rounded(px(6.))
        .text_size(px(12.))
        .text_color(content(0.7))
        .cursor_pointer()
        .child(label)
}
/// The green of the tick that confirms a copy.
const COPIED: Hsla = Hsla {
    h: 0.396,
    s: 0.72,
    l: 0.56,
    a: 1.,
};
const COPIED_POP: Duration = Duration::from_millis(260);

fn icon_action(id: impl Into<SharedString>, label: &str, name: &'static str) -> Stateful<Div> {
    icon_action_ink(id, label, name, content(0.7))
}
fn icon_action_ink(
    id: impl Into<SharedString>,
    label: &str,
    name: &'static str,
    ink: Hsla,
) -> Stateful<Div> {
    icon_action_with(id, label, action_icon(name, ink), ink)
}
fn icon_action_with(
    id: impl Into<SharedString>,
    label: &str,
    glyph: impl IntoElement,
    ink: Hsla,
) -> Stateful<Div> {
    let id: SharedString = id.into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.to_owned())
        .h(px(28.))
        .px(px(10.))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .rounded(px(6.))
        .text_size(px(12.))
        .text_color(content(0.7))
        .cursor_pointer()
        .hover(move |s| {
            if ink == shell_fill() {
                s.bg(shell_fill().opacity(0.1)).text_color(ink)
            } else {
                s.bg(content(0.07)).text_color(content(1.))
            }
        })
        .child(glyph)
        .child(label.to_owned())
}
fn action_icon(name: &'static str, ink: Hsla) -> gpui::Svg {
    icon(name, 14.).text_color(ink)
}
pub(crate) fn relative_time(iso: &str) -> String {
    if model::timestamp_ms(iso) == 0 {
        return String::new();
    }
    let mut value =
        (((model::timestamp_ms(iso) - chrono::Utc::now().timestamp_millis()) as f64 / 1000.) + 0.5)
            .floor();
    let mut unit = "second";
    for (step, name) in [
        (60., "second"),
        (60., "minute"),
        (24., "hour"),
        (7., "day"),
        (4.34524, "week"),
        (12., "month"),
        (f64::INFINITY, "year"),
    ] {
        unit = name;
        if value.abs() < step {
            break;
        }
        value = ((value / step) + 0.5).floor();
    }
    let amount = value as i64;
    match (unit, amount) {
        ("second", 0) => "now".into(),
        ("day", -1) => "yesterday".into(),
        ("day", 1) => "tomorrow".into(),
        ("week" | "month" | "year", -1) => format!("last {unit}"),
        ("week" | "month" | "year", 1) => format!("next {unit}"),
        _ => {
            let count = amount.unsigned_abs();
            let label = format!("{count} {unit}{}", if count == 1 { "" } else { "s" });
            if amount > 0 {
                format!("in {label}")
            } else {
                format!("{label} ago")
            }
        }
    }
}
fn review_label(state: &str) -> String {
    match state.trim().to_uppercase().as_str() {
        "APPROVED" => "Approved".into(),
        "CHANGES_REQUESTED" => "Changes requested".into(),
        "COMMENTED" => "Commented".into(),
        "DISMISSED" => "Dismissed".into(),
        "PENDING" => "Pending".into(),
        "REVIEW_REQUIRED" => "Review required".into(),
        _ => state.replace('_', " ").to_lowercase(),
    }
}
fn failed_check(state: &str) -> bool {
    matches!(
        state.to_lowercase().as_str(),
        "failure" | "failed" | "error" | "timed_out" | "cancelled" | "action_required"
    )
}
fn review_event(comment: &model::Comment) -> bool {
    comment.kind == "review" && comment.body.trim().is_empty() && comment.replies.is_empty()
}
#[derive(Clone)]
pub(super) enum Activity {
    Comment(std::sync::Arc<model::Comment>),
    Commits(Vec<model::Commit>),
}
#[derive(Default)]
pub(super) struct SummaryRows {
    revision: u64,
    discussion: bool,
    keys: Vec<SharedString>,
}
fn menu_row(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    checked: bool,
) -> Stateful<Div> {
    let id: SharedString = id.into();
    let label = label.into();
    div()
        .id(id)
        .role(Role::MenuItemCheckBox)
        .aria_label(label.clone())
        .h(px(28.))
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(8.))
        .rounded(px(6.))
        .text_size(px(12.))
        .text_color(content(0.8))
        .cursor_pointer()
        .hover(|s| s.bg(content(0.06)))
        .child(
            div()
                .size(px(14.))
                .when(checked, |d| d.child(icon("check", 14.))),
        )
        .child(div().flex_1().min_w_0().truncate().child(label))
}
fn caption(text: impl Into<SharedString>) -> Div {
    div()
        .px(px(8.))
        .pt(px(10.))
        .pb(px(4.))
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(content(0.4))
        .child(text.into())
}
fn message(text: impl Into<SharedString>) -> Div {
    div()
        .p(px(12.))
        .text_size(px(12.))
        .line_height(px(19.))
        .text_color(content(0.5))
        .child(text.into())
}
fn error(text: impl Into<SharedString>) -> Div {
    message(text).text_color(rgb(0xf87171))
}
fn provider_mark(provider: &str) -> gpui::Svg {
    let (path, color) = match provider {
        "linear" => ("inbox-linear", content(0.8)),
        "jira" => ("inbox-jira", rgb(0x2684ff).into()),
        "gitlab" => ("inbox-gitlab", rgb(0xfc6d26).into()),
        "azuredevops" => ("inbox-azuredevops", content(0.8)),
        _ => ("inbox-github", content(0.8)),
    };
    icon(path, 14.).text_color(color)
}
fn status_color(item: &Item) -> Hsla {
    match item.status() {
        "Merged" => rgb(0xa78bfa).into(),
        "Closed" => rgb(0xf87171).into(),
        "Draft" => content(0.4),
        _ => rgb(0x4ade80).into(),
    }
}
fn status_icon(item: &Item) -> &'static str {
    if item.kind == "pr" {
        "git-pull-request"
    } else if item.status() == "Closed" {
        "check-circle"
    } else {
        "circle-dot"
    }
}
fn attention_label(reason: &str) -> String {
    let reason = reason.trim().to_lowercase();
    match reason.as_str() {
        "assigned" => "Assigned to you",
        "mentioned" | "directly_addressed" => "Mentioned you",
        "review_requested" => "Review requested",
        "review_submitted" => "Review submitted",
        "approval_required" => "Approval required",
        "build_failed" => "Pipeline failed",
        "unmergeable" => "Cannot be merged",
        "merge_train_removed" => "Removed from merge train",
        "member_access_requested" => "Access requested",
        "marked" => "Added to your to-dos",
        _ => {
            return reason
                .split('_')
                .filter(|word| !word.is_empty())
                .enumerate()
                .map(|(ix, word)| {
                    if ix == 0 {
                        let mut chars = word.chars();
                        chars
                            .next()
                            .map(|c| c.to_uppercase().to_string() + chars.as_str())
                            .unwrap_or_default()
                    } else {
                        word.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
        }
    }
    .into()
}
fn toggle(list: &mut Vec<String>, key: String) {
    if list.contains(&key) {
        list.retain(|value| value != &key);
    } else {
        list.push(key);
    }
}
fn project_name(path: &std::path::Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

fn label_badge(label: &model::Label, compact: bool) -> Div {
    let color = u32::from_str_radix(label.color.trim().trim_start_matches('#'), 16)
        .ok()
        .filter(|_| label.color.trim().trim_start_matches('#').len() == 6);
    div()
        .max_w(px(if compact { 80. } else { 200. }))
        .rounded(px(3.))
        .px(px(6.))
        .py(px(1.))
        .flex()
        .items_center()
        .gap(px(4.))
        .bg(content(0.08))
        .text_size(px(if compact { 10. } else { 11. }))
        .text_color(content(0.5))
        .when_some(color, |d, color| {
            d.child(div().size(px(6.)).rounded_full().bg(rgb(color)))
        })
        .child(div().min_w_0().truncate().child(label.name.clone()))
}

#[derive(Clone, Copy)]
enum CommentStyle {
    Plain,
    Timeline,
    Reply,
}

impl Inbox {}

impl gpui::Render for Inbox {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.focus_comment {
            self.focus_comment = false;
            window.focus(&self.comment.focus_handle(cx), cx);
        }
        let list = self.list_view(cx);
        let detail = self.detail_view(window, cx);
        div()
            .id("inbox-view")
            .role(Role::Group)
            .aria_label("Inbox")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    if this.connect_provider.is_some() && !this.connecting {
                        this.connect_provider = None;
                    } else if this.confirm_action.is_some() && !this.acting {
                        this.confirm_action = None;
                    } else if this.filters_open
                        || this.connections_open
                        || this.project_picker
                        || this.action_menu
                    {
                        this.filters_open = false;
                        this.connections_open = false;
                        this.project_picker = false;
                        this.action_menu = false;
                    } else {
                        cx.emit(Event::Close);
                    }
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_mouse_move(
                cx.listener(|this, event: &gpui::MouseMoveEvent, window, cx| {
                    if let Some((start, width)) = this.resize_start {
                        let max = (f32::from(window.viewport_size().width) * 0.5).clamp(240., 420.);
                        this.state.list_width =
                            (width + f32::from(event.position.x) - start).clamp(240., max);
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.resize_start.take().is_some() {
                        this.save();
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.resize_start.take().is_some() {
                        this.save();
                        cx.notify();
                    }
                }),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .child(list)
                    .child(div().flex_1().min_w_0().min_h_0().child(detail)),
            )
            .when_some(self.error.clone(), |d, text| d.child(error(text)))
            .when(self.filters_open, |d| d.child(self.filters_menu(cx)))
            .when(self.connections_open, |d| d.child(self.connection_menu(cx)))
            .when(self.project_picker, |d| d.child(self.project_menu(cx)))
            .when(self.action_menu, |d| d.child(self.action_menu_view(cx)))
            .when(self.connect_provider.is_some(), |d| {
                d.child(crate::ui::modal::window_overlay(
                    window,
                    self.connection_dialog(cx),
                ))
            })
            .when(self.confirm_action.is_some(), |d| {
                d.child(self.action_confirmation(cx))
            })
    }
}

#[cfg(test)]
mod tests;
