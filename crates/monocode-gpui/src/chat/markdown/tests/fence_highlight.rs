use super::super::Fence;
use crate::ui::syntax_palette::{HighlightRequest, highlight};

#[test]
fn prose_fences_do_not_carry_regex_or_string_colors_after_paths() {
    let source = "Don't open ~/Library, or docs/ROADMAP.md.\nThe owner's next step.\n";
    for tag in ["", "text", "plaintext", "txt"] {
        let fence = Fence::new(tag, source);
        assert_eq!(fence.language, "text");
        let result = highlight(HighlightRequest {
            source,
            path: None,
            fence_tag: Some(&fence.language),
        })
        .unwrap();
        assert_eq!(result.lines.len(), 2);
        for line in result.lines {
            assert!(!line.is_empty());
            assert!(line.iter().all(|span| span.color == 0xD4D4D4));
        }
    }
    assert_eq!(Fence::new("javascript", source).language, "javascript");
}
