use super::super::*;
use crate::chat::markdown::model::InlineStyle;
use gpui::{Bounds, Context, Pixels, TestAppContext};

struct Fixture(Vec<InlineRun>);
impl gpui::Render for Fixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(220.))
            .debug_selector(|| "rich-wrap".into())
            .child(rich_inline(
                &self.0,
                "fixture".into(),
                Tone::Normal,
                None,
                Instant::now(),
                0,
                14.,
                None,
            ))
    }
}

fn code(text: &str) -> InlineRun {
    InlineRun {
        text: text.into(),
        style: InlineStyle {
            code: true,
            ..Default::default()
        },
    }
}

/// Lays out `runs` in a 220px column; returns the column and `part` bounds.
fn layout(
    cx: &mut TestAppContext,
    runs: Vec<InlineRun>,
    part: &'static str,
) -> [Bounds<Pixels>; 2] {
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| cx.new(|_| Fixture(runs)))
            .unwrap()
    });
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    visual.refresh().unwrap();
    [
        visual.debug_bounds("rich-wrap").unwrap(),
        visual.debug_bounds(part).unwrap(),
    ]
}

#[gpui::test]
fn cjk_prose_with_inline_code_wraps_within_column(cx: &mut TestAppContext) {
    let runs = vec![
        InlineRun {
            text: "メモには".into(),
            style: InlineStyle::default(),
        },
        code("example"),
        InlineRun {
            text: "午後は近所を散歩してから部屋に戻り夕方まで穏やかな時間を過ごしました".repeat(4),
            style: InlineStyle::default(),
        },
    ];
    let [bounds, prose] = layout(cx, runs, "rich-prose-2-0");
    assert!(
        prose.right() <= bounds.right(),
        "prose {prose:?} exceeds {bounds:?}"
    );
    assert!(
        prose.size.height > px(24.),
        "long prose did not wrap: {prose:?}"
    );
}

#[gpui::test]
fn long_code_path_wraps_within_column(cx: &mut TestAppContext) {
    let path =
        r"C:\Users\someone\AppData\Local\Temp\scratchpad\extract\Key_Enroll_Instructions.pdf";
    let [bounds, chip] = layout(cx, vec![code(path)], "rich-code-0");
    assert!(
        chip.right() <= bounds.right(),
        "code {chip:?} exceeds {bounds:?}"
    );
    assert!(
        chip.size.height > px(24.),
        "long code did not wrap: {chip:?}"
    );
}
