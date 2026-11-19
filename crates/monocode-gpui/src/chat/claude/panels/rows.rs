//! Small pieces both tabs share: rows, tool icons and how counts, times and
//! paths read.
use super::*;

pub(super) fn note(text: &'static str) -> AnyElement {
    div()
        .px_3()
        .py_2()
        .text_size(px(12.))
        .text_color(content(0.5))
        .child(text)
        .into_any_element()
}

/// An MCP server (letter badge and a fold arrow, `open`) or a built-in tool
/// (its icon), with a line above every row but the first.
pub(super) fn tool_row(
    divider: bool,
    glyph: Option<&'static str>,
    open: Option<bool>,
    name: String,
    count: u32,
) -> gpui::Div {
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
        .when_some(open, |d, open| {
            d.child(
                icon(
                    if open {
                        "chevron-down"
                    } else {
                        "chevron-right"
                    },
                    12.,
                )
                .text_color(content(0.6)),
            )
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(12.))
                .text_color(content(0.88))
                .when(open.is_some(), |d| d.font_weight(FontWeight::SEMIBOLD))
                .child(name),
        )
        .child(calls(count))
}

/// "12 Calls", the count in red.
pub(super) fn calls(count: u32) -> gpui::Div {
    div()
        .flex_shrink_0()
        .flex()
        .gap_1()
        .text_size(px(11.))
        .font_features(tabular())
        .child(div().text_color(rgb(RED_400)).child(count.to_string()))
        .child(
            div()
                .text_color(content(0.5))
                .child(if count == 1 { "Call" } else { "Calls" }),
        )
}

/// The Lucide icon for a built-in Claude Code tool; a wrench for the rest.
pub(super) fn tool_icon(name: &str) -> &'static str {
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

pub(super) fn file_label(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Read => "Read",
        FileKind::Edited => "Edited",
        FileKind::Created => "Created",
        FileKind::Written => "Written over",
    }
}

pub(super) fn file_name(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_owned()
}

/// The folder holding `path`: inside the project relative to it, elsewhere
/// with the home folder written `~`.
pub(super) fn folder(path: &str, cwd: &str) -> String {
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
pub(super) fn tokens(count: u64) -> String {
    match count {
        0..1_000 => count.to_string(),
        1_000..10_000 => format!("{:.1}k", count as f64 / 1e3),
        10_000..1_000_000 => format!("{}k", count / 1_000),
        _ => format!("{:.1}M", count as f64 / 1e6),
    }
}

/// 2:30, 1:02:30 past an hour, 3d 23h past a day.
pub(super) fn duration(ms: i64) -> String {
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
                crate::ui::assets::Assets.load(path).unwrap().is_some(),
                "{path}"
            );
        }
    }
}
