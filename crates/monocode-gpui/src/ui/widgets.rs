//! Settings-style building blocks reused across views: buttons, notes, page headers.
use crate::ui::theme::*;
use gpui::{
    AnyElement, Div, ElementId, FontWeight, Hsla, Role, SharedString, Stateful, div, prelude::*,
    px, rgb,
};

pub(crate) const RED: u32 = 0xf87171;

/// `text-[12px] leading-relaxed` and `text-[13px]` line boxes.
pub(crate) const RELAXED_12: f32 = 19.5;

pub(crate) fn red(alpha: f32) -> Hsla {
    let mut color: Hsla = rgb(RED).into();
    color.a = alpha;
    color
}

pub(crate) fn accent(alpha: f32) -> Hsla {
    let mut color: Hsla = accent_color();
    color.a = alpha;
    color
}

/// PageHeader: the page's title and what it is for.
pub(crate) fn page_header(title: &'static str, description: &'static str) -> Div {
    div()
        .debug_selector(|| "settings-page-header".into())
        .pb_4()
        .child(
            div()
                .text_size(px(20.))
                .line_height(px(25.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(content(1.))
                .child(title),
        )
        .child(
            div()
                .mt(px(6.))
                .max_w(px(576.))
                .text_size(px(13.))
                .line_height(px(21.125))
                .text_color(content(0.45))
                .child(description),
        )
}

/// A muted line inside a group card: an empty state or a hint.
pub(crate) fn note(text: impl Into<SharedString>) -> AnyElement {
    div()
        .px_4()
        .py(px(14.))
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(content(0.45))
        .child(text.into())
        .into_any_element()
}

/// SecondaryButton.tsx.
pub(crate) fn secondary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    danger: bool,
    disabled: bool,
) -> Stateful<Div> {
    button_base(id, label).map(|d| {
        if disabled {
            d.opacity(0.4).text_color(if danger {
                rgb(RED).into()
            } else {
                content(0.7)
            })
        } else if danger {
            d.cursor_pointer()
                .text_color(rgb(RED))
                .hover(|s| s.border_color(red(0.4)).bg(red(0.1)))
        } else {
            d.cursor_pointer()
                .text_color(content(0.7))
                .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
        }
    })
}

pub(crate) fn button_base(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
    let label = label.into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(px(6.))
        .rounded_md()
        .border_1()
        .border_color(content(0.1))
        .px(px(10.))
        .py_1()
        .text_size(px(12.))
        .line_height(px(16.))
        .whitespace_nowrap()
}

/// Borderless, square icon actions used by SkillsPage and MonoRow.
pub(crate) fn icon_action(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    path: &'static str,
    side: f32,
    icon_side: f32,
    alpha: f32,
    disabled: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.into())
        .group("settings-icon-action")
        .size(px(side))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(if side == 20. { 4. } else { 6. }))
        .text_color(content(alpha))
        .when(disabled, |d| d.opacity(0.4))
        .when(!disabled, |d| {
            d.cursor_pointer()
                .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
        })
        .child(
            icon(path, icon_side)
                .text_color(content(alpha))
                .when(!disabled, |i| {
                    i.group_hover("settings-icon-action", |s| s.text_color(content(1.)))
                }),
        )
}

pub(crate) fn primary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    disabled: bool,
) -> Stateful<Div> {
    let mut ink = shell_fill();
    ink.a = 1.;
    button_base(id, label)
        .border_0()
        .bg(content(1.))
        .text_color(ink)
        .font_weight(gpui::FontWeight::MEDIUM)
        .map(|d| {
            if disabled {
                d.opacity(0.4)
            } else {
                d.cursor_pointer()
                    .hover(move |s| s.bg(content(0.85)).text_color(ink))
            }
        })
}
