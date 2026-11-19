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
