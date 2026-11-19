mod native_input;
use super::*;
#[::core::prelude::v1::test]
fn mention_tokens_pick_their_file_icon() {
    let asset = |token| match MentionIcon::of(token) {
        Some(MentionIcon::File(asset)) => asset.to_string(),
        Some(MentionIcon::Note) => "note".into(),
        None => String::new(),
    };
    assert_eq!(asset("@src/main.rs"), "mi/rust.svg");
    assert_eq!(asset("@\"docs/My Notes.md\""), "mi/markdown.svg");
    assert_eq!(asset("@note/todo"), "note");
    assert_eq!(asset("/plan"), "");
}
#[::core::prelude::v1::test]
fn mention_slots_map_offsets_both_ways() {
    let source = "see @a.rs and @b.md";
    let layout = DisplayLayout {
        command: None,
        mentions: vec![(4, MentionIcon::Note), (14, MentionIcon::Note)],
    };
    let display = layout.apply(source.into());
    assert_eq!(
        display.as_ref(),
        "see \u{2003}\u{2009}a.rs and \u{2003}\u{2009}b.md"
    );
    for at in 0..=source.len() {
        assert_eq!(layout.to_source(layout.to_display(at)), at);
    }
    // Clicking inside a blank lands on the nearer side of its `@`.
    assert_eq!(layout.to_source(4 + 3), 4);
    assert_eq!(layout.to_source(4 + 6), 5);
}
#[::core::prelude::v1::test]
fn utf16_and_bounded_paste_respect_unicode() {
    assert_eq!(from16("a🙂漢", 1), 1);
    assert_eq!(from16("a🙂漢", 2), 1);
    assert_eq!(from16("a🙂漢", 3), 5);
    assert_eq!(bounded("🙂a", 3), "");
}
