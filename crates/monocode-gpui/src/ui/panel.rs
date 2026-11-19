//! Panel helpers shared by side panels: strokes, tabular digits, spinners, diff stats.
use crate::ui::theme::*;
use crate::ui::theme::{DIFF_ADD, DIFF_DEL};
use gpui::{
    AnyElement, FontFeatures, FontWeight, Hsla, Role, SharedString, Stateful, div, prelude::*, px,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub(crate) const WIDTH_MIN: f32 = 220.;

pub(crate) const WIDTH_MAX: f32 = 640.;

pub(crate) const WIDTH_DEFAULT: f32 = 288.;

pub(crate) fn tab_icons_width(count: usize) -> f32 {
    count as f32 * 26. + count.saturating_sub(1) as f32 * 2.
}

// Separators inside the inset card use its 4% hairline; dark selection is 10%.
pub(crate) fn stroke() -> Hsla {
    content(0.04)
}

pub(crate) fn selection() -> Hsla {
    content(0.1)
}

/// The GRAPH section under Changes keeps two history rows, and leaves the
/// Changes header, commit box and a row above it.
pub(crate) fn clamp_graph(height: f32, total: f32) -> f32 {
    height.min(total - 160.).max(PANE_HEADER_HEIGHT + 88.)
}

pub(crate) fn tabular() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".into(), 1)]))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RightTab {
    #[default]
    Files,
    Changes,
    Graph,
    /// The project's terminal, when it opens in the panel. Always last and
    /// never part of the saved order.
    Terminal,
}

impl RightTab {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Files => "Explorer",
            Self::Changes => "Changes",
            Self::Graph => "History",
            Self::Terminal => "Terminal",
        }
    }
    /// loadRightPanelTabOrder: unique known tabs, the missing ones appended.
    /// Changes has no tab; the title bar's diff stat opens it.
    pub(crate) fn normalize(order: &[RightTab]) -> Vec<RightTab> {
        let mut out = Vec::new();
        for tab in order.iter().chain(&[Self::Files, Self::Graph]) {
            if !matches!(tab, Self::Terminal | Self::Changes) && !out.contains(tab) {
                out.push(*tab);
            }
        }
        out
    }
}

/// Tool headers share the open-file tab bar's height and bottom rule.
pub(crate) fn pane_header(id: &'static str, label: &'static str) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(Role::Toolbar)
        .aria_label(label)
        .h(px(PANE_HEADER_HEIGHT))
        .flex_shrink_0()
        .min_w_0()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .border_b_1()
        .border_color(stroke())
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(content(0.85))
}

/// GitChangesPanel's `IconAction`: a 20px square, ink at 55%.
pub(crate) fn icon_action(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    path: &'static str,
    enabled: bool,
) -> Stateful<gpui::Div> {
    let id = id.into();
    let group: SharedString = format!("icon-action-{id:?}").into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.into())
        .group(group.clone())
        .size(px(20.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .when(!enabled, |d| d.opacity(0.4))
        .when(enabled, |d| d.hover(|s| s.bg(content(0.1))))
        .child(
            icon(path, 14.)
                .text_color(content(0.55))
                .when(enabled, |i| {
                    i.group_hover(group, |s| s.text_color(content(1.)))
                }),
        )
}

/// The `animate-spin` Loader.
pub(crate) fn spinner(id: impl Into<gpui::ElementId>, path: &'static str, side: f32) -> AnyElement {
    use gpui::{Transformation, percentage};
    crate::ui::motion::pulse::looping(
        id,
        std::time::Duration::from_secs(1),
        crate::ui::motion::pulse::Rate::Smooth,
        move |delta| {
            icon(path, side).with_transformation(Transformation::rotate(percentage(delta)))
        },
    )
    .into_any_element()
}

pub(crate) fn format_integer(value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if value < 0 { format!("-{out}") } else { out }
}

pub(crate) fn diff_stat_width(stats: Option<&monocode_core::vcs::git::scm::DiffStats>) -> f32 {
    let added = format_integer(stats.map_or(0, |s| s.additions.max(0)));
    let deleted = format_integer(stats.map_or(0, |s| s.deletions.max(0)));
    // Tabular digits at 10px, the signs, a 6px gap and 6px padding per side.
    (added.len() + deleted.len() + 2) as f32 * 6.5 + 18.
}

/// Fit the complete diff label together so one number cannot push the other
/// outside its slot. Callers include any gap that scales with their text.
pub(crate) fn diff_stat_scale(available: f32, natural: f32) -> f32 {
    (available.max(0.) / natural.max(f32::EPSILON)).clamp(0.1, 1.)
}

pub(crate) fn diff_stat(
    stats: Option<&monocode_core::vcs::git::scm::DiffStats>,
    width: f32,
) -> Stateful<gpui::Div> {
    let added = format_integer(stats.map_or(0, |s| s.additions.max(0)));
    let deleted = format_integer(stats.map_or(0, |s| s.deletions.max(0)));
    let scale = diff_stat_scale(width - 18., diff_stat_width(stats) - 18.);
    div()
        .id("project-diff-stat")
        .debug_selector(|| "project-diff-stat".into())
        .role(Role::Button)
        .aria_label(format!(
            "Open Changes: {added} lines added, {deleted} lines deleted"
        ))
        .w(px(width))
        .h(px(24.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_end()
        .gap(px(6.))
        .px_1p5()
        .rounded_md()
        .whitespace_nowrap()
        .text_size(px(10. * scale))
        .font_weight(FontWeight::MEDIUM)
        .font_features(tabular())
        .child(
            div()
                .text_color(gpui::rgb(DIFF_ADD))
                .child(format!("+{added}")),
        )
        .child(
            div()
                .text_color(gpui::rgb(DIFF_DEL))
                .child(format!("-{deleted}")),
        )
}
