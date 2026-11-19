use super::super::*;
use crate::chat::markdown::model::InlineStyle;
use gpui::{Context, TestAppContext};

struct Fixture;
impl gpui::Render for Fixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let runs = vec![
            InlineRun {
                text: "メモには".into(),
                style: InlineStyle::default(),
            },
            InlineRun {
                text: "example".into(),
                style: InlineStyle {
                    code: true,
                    ..Default::default()
                },
            },
            InlineRun {
                text: "午後は近所を散歩してから部屋に戻り夕方まで穏やかな時間を過ごしました"
                    .repeat(4),
                style: InlineStyle::default(),
            },
        ];
        div()
            .w(px(220.))
            .debug_selector(|| "rich-wrap".into())
            .child(rich_inline(
                &runs,
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
#[gpui::test]
fn cjk_prose_with_inline_code_wraps_within_column(cx: &mut TestAppContext) {
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| cx.new(|_| Fixture))
            .unwrap()
    });
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    visual.refresh().unwrap();
    let bounds = visual.debug_bounds("rich-wrap").unwrap();
    let prose = visual.debug_bounds("rich-prose-2-0").unwrap();
    assert!(
        prose.right() <= bounds.right(),
        "prose {prose:?} exceeds {bounds:?}"
    );
    assert!(
        prose.size.height > px(24.),
        "long prose did not wrap: {prose:?}"
    );
}
