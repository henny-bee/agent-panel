use super::super::*;

#[test]
fn notes_preserve_prose_breaks_without_changing_chat_or_code() {
    fn prose(tree: &BlockTree) -> String {
        let Block::Paragraph { runs } = &tree.blocks[0].block else {
            panic!("expected paragraph")
        };
        runs.iter().map(|run| run.text.as_str()).collect()
    }
    assert_eq!(prose(&parse_full("one\ntwo")), "one two");
    assert_eq!(prose(&parse_full_hard_breaks("one\ntwo")), "one\ntwo");
    let tree = parse_full_hard_breaks("```text\none\ntwo\n```");
    let Block::CodeBlock { code, .. } = &tree.blocks[0].block else {
        panic!("expected code")
    };
    assert_eq!(code, "one\ntwo\n");
}

#[test]
fn a_reply_places_a_local_image_but_leaves_web_images_as_prose() {
    let first = |text: &str| parse_full(text).blocks[0].block.clone();
    let Block::Visual { id, caption } = first("![The chart](out/chart.png)") else {
        panic!("expected a placed image")
    };
    assert_eq!(
        (id.as_str(), caption.as_str()),
        ("image:out/chart.png", "The chart")
    );
    let Block::Visual { id, .. } = first("![](file:///tmp/a.JPG)") else {
        panic!("expected a placed image")
    };
    assert_eq!(id, "image:/tmp/a.JPG");
    assert!(!matches!(
        first("![x](https://example.com/a.png)"),
        Block::Visual { .. }
    ));
    assert!(!matches!(first("![x](notes.txt)"), Block::Visual { .. }));
}
