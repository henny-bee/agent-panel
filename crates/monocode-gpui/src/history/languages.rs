//! The section's CODE tab: GitHub's Languages bar, then one row per language.
use crate::{activity::file_icon, right_panel::tabular, theme::*};
use gpui::{AnyElement, FontWeight, div, prelude::*, px, relative, rgb};
use monocode_core::languages::Share;

pub(super) fn view(languages: Option<&Result<Vec<Share>, String>>) -> AnyElement {
    let shares = match languages {
        None => return note("Counting languages…".into()),
        Some(Err(error)) => return note(error.clone()),
        Some(Ok(shares)) if shares.is_empty() => return note("No code found".into()),
        Some(Ok(shares)) => shares,
    };
    let total = shares.iter().map(|s| s.bytes).sum::<u64>().max(1) as f32;
    let fraction = |share: &Share| share.bytes as f32 / total;
    div()
        .id("history-languages")
        .debug_selector(|| "history-languages".into())
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px_3()
        .py_2()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(content(0.85))
                .child("Languages"),
        )
        .child(
            div()
                .flex_shrink_0()
                .h(px(8.))
                .w_full()
                .rounded_full()
                .overflow_hidden()
                .bg(content(0.08))
                .flex()
                .children(
                    shares
                        .iter()
                        .map(|s| div().h_full().w(relative(fraction(s))).bg(rgb(s.color))),
                ),
        )
        .child(div().flex_shrink_0().h(px(1.)).my(px(2.)).bg(content(0.07)))
        .children(shares.iter().map(|s| row(s, fraction(s))))
        .into_any_element()
}

fn row(share: &Share, fraction: f32) -> impl IntoElement {
    div()
        .flex_shrink_0()
        .h(px(26.))
        .px_2()
        .rounded(px(6.))
        .bg(content(0.03))
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .size(px(16.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .child(if share.sample.is_empty() {
                    icon("more-horizontal", 14.)
                        .text_color(content(0.5))
                        .into_any_element()
                } else {
                    file_icon(share.sample, 14.)
                }),
        )
        .child(
            div()
                .w(px(84.))
                .flex_shrink_0()
                .truncate()
                .text_size(px(11.))
                .text_color(content(0.8))
                .child(share.name),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .h(px(4.))
                .rounded_full()
                .bg(content(0.08))
                .child(
                    div()
                        .h_full()
                        .rounded_full()
                        .w(relative(fraction))
                        .bg(rgb(share.color)),
                ),
        )
        .child(
            div()
                .w(px(40.))
                .flex_shrink_0()
                .text_right()
                .font_features(tabular())
                .text_size(px(11.))
                .text_color(content(0.55))
                .child(percent(fraction)),
        )
}

fn note(text: String) -> AnyElement {
    div()
        .px_3()
        .py_2()
        .text_size(px(12.))
        .text_color(content(0.45))
        .child(text)
        .into_any_element()
}

/// One decimal, like GitHub; a sliver that rounds to nothing still shows.
fn percent(fraction: f32) -> String {
    let percent = fraction * 100.;
    if percent > 0. && percent < 0.05 {
        "<0.1%".into()
    } else {
        format!("{percent:.1}%")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentages_keep_one_decimal_and_never_show_a_used_language_as_zero() {
        assert_eq!(percent(0.717), "71.7%");
        assert_eq!(percent(1.), "100.0%");
        assert_eq!(percent(0.0001), "<0.1%");
        assert_eq!(percent(0.), "0.0%");
    }
}
