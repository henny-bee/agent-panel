//! What a tool row shows when opened, as in Claude Desktop's Code tab: a
//! command in its own box with the output under it, an edit as its diff, and
//! anything else as its output.
use crate::ui::theme::*;
use gpui::{AnyElement, App, Div, div, prelude::*, px, rgb};
use monocode_core::session::tool::{ToolCall, ToolState};
use serde_json::Value;

/// Lines an opened row shows before it says how many it left out.
const LINES: usize = 40;

/// One line of an edit: `+`, `-` or ` ` (unchanged context), and its text.
pub(crate) type DiffLine = (char, String);

fn input(call: &ToolCall) -> Value {
    serde_json::from_str(&call.input).unwrap_or_default()
}

/// The edit a call made, as diff lines, for Edit, MultiEdit, Write and
/// patches carried as unified diff text. None for anything else.
pub(crate) fn edit_lines(call: &ToolCall) -> Option<Vec<DiffLine>> {
    let input = input(call);
    let pair = |edit: &Value| {
        Some((
            edit["old_string"].as_str()?.to_owned(),
            edit["new_string"].as_str()?.to_owned(),
        ))
    };
    let mut lines = Vec::new();
    let mut push = |old: &str, new: &str| {
        for change in similar::TextDiff::from_lines(old, new).iter_all_changes() {
            let sign = match change.tag() {
                similar::ChangeTag::Delete => '-',
                similar::ChangeTag::Insert => '+',
                similar::ChangeTag::Equal => ' ',
            };
            lines.push((
                sign,
                change.value().trim_end_matches(['\n', '\r']).to_owned(),
            ));
        }
    };
    if let Some((old, new)) = pair(&input) {
        push(&old, &new);
    } else if let Some(edits) = input["edits"].as_array() {
        for (old, new) in edits.iter().filter_map(pair) {
            push(&old, &new);
        }
    } else if let Some(content) = input["content"].as_str() {
        push("", content);
    } else if let Some(patch) = ["patch", "diff", "input"]
        .iter()
        .find_map(|key| input[*key].as_str())
        .filter(|text| {
            text.lines()
                .any(|line| line.starts_with("@@") || line.starts_with("+++"))
        })
    {
        for line in patch.lines() {
            match line.chars().next() {
                Some('+') if !line.starts_with("+++") => lines.push(('+', line[1..].to_owned())),
                Some('-') if !line.starts_with("---") => lines.push(('-', line[1..].to_owned())),
                Some(' ') => lines.push((' ', line[1..].to_owned())),
                _ => {}
            }
        }
    } else {
        return None;
    }
    Some(lines)
}

/// Lines an edit added and removed: the `+12 −3` beside its file.
pub(crate) fn edit_counts(call: &ToolCall) -> Option<(usize, usize)> {
    let lines = edit_lines(call)?;
    Some((
        lines.iter().filter(|(sign, _)| *sign == '+').count(),
        lines.iter().filter(|(sign, _)| *sign == '-').count(),
    ))
}

/// `+12 −3`, in the diff colours, for a summary line.
pub(crate) fn counts_label((added, removed): (usize, usize), cx: &App) -> Div {
    let (add, del) = crate::editor::scrollbar::diff_colors(cx);
    div()
        .flex()
        .flex_shrink_0()
        .gap_1()
        .font_family(crate::ui::fonts::mono())
        .text_size(px(12.))
        .when(added > 0, |d| {
            d.child(div().text_color(rgb(add)).child(format!("+{added}")))
        })
        .when(removed > 0, |d| {
            d.child(div().text_color(rgb(del)).child(format!("−{removed}")))
        })
}

fn code_box() -> Div {
    div()
        .w_full()
        .min_w_0()
        .px(px(10.))
        .py(px(8.))
        .rounded(px(8.))
        .border_1()
        .border_color(content(0.08))
        .bg(content(0.03))
        .font_family(crate::ui::fonts::mono())
        .text_size(px(12.))
        .line_height(px(17.))
}

/// The first `LINES` lines of `text`, and how many there were.
fn head(text: &str) -> (String, usize) {
    let total = text.lines().count();
    (
        text.lines().take(LINES).collect::<Vec<_>>().join("\n"),
        total,
    )
}

fn more(shown: usize, total: usize) -> Option<Div> {
    (total > shown).then(|| {
        div()
            .pt_1()
            .text_size(px(11.))
            .text_color(content(0.45))
            .child(format!("Showing the first {shown} of {total} lines."))
    })
}

/// The body under an opened tool row, or None when there is nothing more to
/// show than its line already does.
pub(crate) fn render(call: &ToolCall, cx: &App) -> Option<AnyElement> {
    let failed = matches!(call.state, ToolState::Failed | ToolState::Cancelled);
    let shown = call.presentation();
    let output = call.output.trim_end();
    if shown.action == "Run" {
        let command = input(call)["command"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| shown.target.clone());
        let (text, total) = head(output);
        let body = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                code_box().child(
                    div()
                        .flex()
                        .gap_2()
                        .child(div().flex_shrink_0().text_color(content(0.4)).child("$"))
                        .child(div().min_w_0().text_color(content(0.85)).child(command)),
                ),
            )
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .font_family(crate::ui::fonts::mono())
                    .text_size(px(12.))
                    .line_height(px(17.))
                    .text_color(if failed {
                        rgb(0xf87171).into()
                    } else {
                        content(0.65)
                    })
                    .child(if text.is_empty() {
                        if call.state == ToolState::Running {
                            "Running…".to_owned()
                        } else {
                            "No output".to_owned()
                        }
                    } else {
                        text
                    })
                    .children(more(LINES, total)),
            );
        return Some(body.into_any_element());
    }
    if let Some(lines) = edit_lines(call).filter(|lines| !lines.is_empty()) {
        let (add, del) = crate::editor::scrollbar::diff_colors(cx);
        let total = lines.len();
        let body = code_box()
            .px_0()
            .flex()
            .flex_col()
            .children(lines.into_iter().take(LINES).map(|(sign, text)| {
                let (fill, ink): (gpui::Hsla, gpui::Hsla) = match sign {
                    '+' => (with_alpha(add, 0.14), rgb(add).into()),
                    '-' => (with_alpha(del, 0.14), rgb(del).into()),
                    _ => (gpui::transparent_black(), content(0.6)),
                };
                div()
                    .w_full()
                    .min_w_0()
                    .px(px(10.))
                    .flex()
                    .gap_2()
                    .bg(fill)
                    .child(
                        div()
                            .w(px(8.))
                            .flex_shrink_0()
                            .text_color(ink)
                            .child(sign.to_string()),
                    )
                    .child(div().min_w_0().text_color(content(0.8)).child(text))
            }))
            .children(more(LINES, total).map(|d| d.px(px(10.))));
        let failure = (failed && !output.is_empty()).then(|| {
            div()
                .pt_2()
                .font_family(crate::ui::fonts::mono())
                .text_size(px(12.))
                .text_color(rgb(0xf87171))
                .child(head(output).0)
        });
        return Some(
            div()
                .w_full()
                .min_w_0()
                .flex()
                .flex_col()
                .child(body)
                .children(failure)
                .into_any_element(),
        );
    }
    // A read's text is the file itself, which the row's link opens.
    if output.is_empty() || (shown.action == "Read" && !failed) {
        return None;
    }
    let (text, total) = head(output);
    Some(
        code_box()
            .text_color(if failed {
                rgb(0xf87171).into()
            } else {
                content(0.65)
            })
            .child(text)
            .children(more(LINES, total))
            .into_any_element(),
    )
}

fn with_alpha(color: u32, alpha: f32) -> gpui::Hsla {
    let mut color: gpui::Hsla = rgb(color).into();
    color.a = alpha;
    color
}

#[cfg(test)]
mod tests {
    use super::*;
    fn call(name: &str, input: Value) -> ToolCall {
        let mut call = ToolCall::new("t", name);
        call.input = input.to_string();
        call
    }
    #[test]
    fn edits_count_the_lines_they_add_and_remove() {
        let edit = call(
            "Edit",
            serde_json::json!({"file_path":"a.rs","old_string":"a\nb\nc\n","new_string":"a\nB\nc\nd\n"}),
        );
        assert_eq!(edit_counts(&edit), Some((2, 1)));
        let lines = edit_lines(&edit).unwrap();
        assert_eq!(lines[0], (' ', "a".into()));
        assert!(lines.contains(&('-', "b".into())) && lines.contains(&('+', "d".into())));
        let write = call(
            "Write",
            serde_json::json!({"file_path":"b.rs","content":"one\ntwo"}),
        );
        assert_eq!(edit_counts(&write), Some((2, 0)));
        let multi = call(
            "MultiEdit",
            serde_json::json!({"file_path":"c.rs","edits":[
                {"old_string":"x","new_string":"y"},{"old_string":"p","new_string":"q"}]}),
        );
        assert_eq!(edit_counts(&multi), Some((2, 2)));
        let patch = call(
            "apply_patch",
            serde_json::json!({"input":"--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n same"}),
        );
        assert_eq!(edit_counts(&patch), Some((1, 1)));
        assert_eq!(
            edit_counts(&call("Bash", serde_json::json!({"command":"ls"}))),
            None
        );
    }
    #[test]
    fn head_keeps_the_first_lines_and_counts_them_all() {
        let text = (0..50)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let (shown, total) = head(&text);
        assert_eq!(total, 50);
        assert_eq!(shown.lines().count(), LINES);
        assert_eq!(shown.lines().next(), Some("0"));
    }
}
