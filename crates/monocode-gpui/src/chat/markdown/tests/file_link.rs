use super::super::*;
use crate::chat::markdown::model::InlineStyle;
#[test]
fn preview_links_keep_their_document_base_and_display_text() {
    let runs = vec![
        InlineRun {
            text: "source".into(),
            style: InlineStyle {
                link: Some("../main.rs:5".into()),
                ..Default::default()
            },
        },
        InlineRun {
            text: "src/lib.rs".into(),
            style: InlineStyle {
                code: true,
                ..Default::default()
            },
        },
        InlineRun {
            text: "web".into(),
            style: InlineStyle {
                link: Some("https://example.com".into()),
                ..Default::default()
            },
        },
    ];
    let mut row = rows(&Block::Paragraph { runs }).remove(0);
    resolve_file_links(&mut row, "/project/docs");
    let Part::Text { runs, .. } = row.part else {
        panic!()
    };
    // Resolved paths use the platform's separator.
    let native = |path: &str| path.replace('/', std::path::MAIN_SEPARATOR_STR);
    assert_eq!(runs[0].text, "source");
    assert_eq!(
        runs[0].style.link.as_deref(),
        Some(native("/project/main.rs:5:1").as_str())
    );
    assert_eq!(runs[1].text, "src/lib.rs");
    assert_eq!(
        runs[1].style.link.as_deref(),
        Some(native("/project/docs/src/lib.rs").as_str())
    );
    assert_eq!(runs[2].style.link.as_deref(), Some("https://example.com"));
}
