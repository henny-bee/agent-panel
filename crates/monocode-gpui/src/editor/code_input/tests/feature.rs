use super::super::*;
use gpui::{TestAppContext, WindowBounds, WindowOptions};
fn editor(cx: &mut TestAppContext) -> gpui::WindowHandle<Input> {
    cx.update(|cx| {
        bind(cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(500.), px(300.)),
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(Input::editor),
        )
        .unwrap()
    })
}
#[gpui::test]
fn folds_keep_source_offsets_and_expand_when_search_reveals_hidden_text(cx: &mut TestAppContext) {
    let window = editor(cx);
    let source = "function hello() {\n  const emoji = '🙂';\n  return emoji;\n}\n";
    window
        .update(cx, |i, _, cx| {
            i.set_text(source.into(), cx);
            i.set_editor_analysis(
                crate::editor::analysis::analyze("a.js", source, None).unwrap(),
                cx,
            );
            i.toggle_fold_row(0, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |i, _, cx| {
            let code = i.code.as_ref().unwrap();
            let fold = code.folded[0].clone();
            assert_eq!(i.content.as_ref(), source);
            assert!(code.hidden(1) && code.hidden(2) && code.hidden(3));
            assert_eq!(i.next(fold.from), fold.to);
            let p = i.code_position(fold.to);
            assert_eq!(i.code_index(p), fold.to);
            i.replace_bytes(0..0, "// header\n", 10, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |i, _, cx| {
            assert_eq!(
                i.code.as_ref().unwrap().folded.len(),
                1,
                "editing before a fold keeps it collapsed"
            );
            let at = i.content.find("emoji").unwrap();
            i.select_range(at..at + 5, cx);
            assert!(i.code.as_ref().unwrap().folded.is_empty());
        })
        .unwrap();
}
#[gpui::test]
fn autocomplete_accepts_with_tab_and_deleted_diff_rows_remain_outside_the_document(
    cx: &mut TestAppContext,
) {
    let window = editor(cx);
    window
        .update(cx, |i, _, cx| {
            i.set_editor_path("a.ts".into());
            i.set_text("cons".into(), cx);
            i.schedule_completion(true, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |i, w, cx| {
            let code = i.code.as_mut().unwrap();
            code.completion_index = code
                .completions
                .iter()
                .position(|c| c.label == "const")
                .unwrap();
            i.indent(&Indent, w, cx);
            assert_eq!(i.content.as_ref(), "const");
            i.undo(&Undo, w, cx);
            assert_eq!(i.content.as_ref(), "cons");
            i.set_text("const value = 2;\n".into(), cx);
            i.set_editor_diff(true, true);
            let analysis =
                crate::editor::analysis::analyze("a.ts", &i.content, Some("const value = 1;\n"))
                    .unwrap();
            i.set_editor_analysis(analysis, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |i, _, _| {
            let code = i.code.as_ref().unwrap();
            assert_eq!(code.deleted_rows.len(), 1);
            assert_eq!(code.deleted_rows[0].2, "const value = 1;");
            assert_eq!(code.tops[0], TOP_PADDING + i.line_height);
            assert_eq!(i.content.as_ref(), "const value = 2;\n");
        })
        .unwrap();
}
