use super::*;
use gpui::TestAppContext;
use monocode_core::{session::transcript::Transcript, store::Store};
use std::sync::Mutex;
#[gpui::test]
fn cancelled_recall_preserves_history_and_native_provider_binding(cx: &mut TestAppContext) {
    let mut store = Store::open(Path::new(":memory:")).unwrap();
    let mut session = store.create("pi", "/project").unwrap();
    store.provider_id(session.id, "thread").unwrap();
    session.provider_id = Some("thread".into());
    let mut transcript = Transcript::default();
    transcript.begin("user");
    transcript.append("Original");
    transcript.begin("assistant");
    transcript.append("Answer");
    store.write_rows(session.id, 0, &transcript.rows).unwrap();
    let edit = store.prepare_edit(session.id, &transcript.rows, 0).unwrap();
    let chat = cx.new(|cx| Conversation::new(session, Arc::new(Mutex::new(store)), cx));
    chat.update(cx, |chat, cx| {
        chat.loaded = true;
        chat.transcript
            .update(cx, |view, cx| view.patch(0, transcript.rows.clone(), cx));
        chat.editing_last_turn.prepared = Some(edit);
        assert!(chat.submit_edited(cx));
        assert!(!chat.editing_last_turn.pending());
        assert_eq!(chat.transcript.read(cx).source_rows(), transcript.rows);
        assert_eq!(chat.summary.provider_id.as_deref(), Some("thread"));
        assert_eq!(
            locked(&chat.store)
                .unwrap()
                .load(chat.summary.id)
                .unwrap()
                .rows,
            transcript.rows
        );
    });
}
#[gpui::test]
fn a_refused_cut_says_why_above_the_composer_and_cancel_stops_a_running_one(
    cx: &mut TestAppContext,
) {
    let store = Store::open(Path::new(":memory:")).unwrap();
    let session = store.create("claude", "/project").unwrap();
    let chat = cx.new(|cx| Conversation::new(session, Arc::new(Mutex::new(store)), cx));
    chat.update(cx, |chat, cx| {
        chat.loaded = true;
        chat.busy = true;
        assert!(chat.refuse_cut(Cut::Rewind, cx));
        let notice = chat.editing_last_turn.notice.clone().unwrap();
        assert!(notice.error);
        assert_eq!(
            notice.text.as_ref(),
            "Rewind is unavailable while Claude Code is working."
        );
        chat.busy = false;
        chat.input
            .update(cx, |input, cx| input.set_text("Draft".into(), cx));
        assert!(chat.refuse_cut(Cut::Edit, cx));
        assert!(
            chat.editing_last_turn
                .notice
                .as_ref()
                .unwrap()
                .text
                .starts_with("Edit needs an empty message box")
        );
        // A fork leaves the composer alone.
        assert!(!chat.refuse_cut(Cut::Fork, cx));
        // Sending ends the notice.
        chat.editing_last_turn.message_sent();
        assert!(chat.editing_last_turn.notice.is_none());
        // Cancel edit reaches a cut that is still running.
        let flag = Arc::new(AtomicBool::new(false));
        chat.editing_last_turn.active = true;
        chat.editing_last_turn.cancel = Some(flag.clone());
        assert!(chat.cancel_edit_last_turn(false, cx));
        assert!(flag.load(Ordering::SeqCst));
    });
}
#[test]
fn changed_files_counts_each_edited_path_once_from_the_message_on() {
    use monocode_core::session::{tool::ToolCall, transcript::Row};
    let tool = |name: &str, path: &str| {
        let mut call = ToolCall::new(path, name);
        call.input = serde_json::json!({"file_path":path}).to_string();
        Row {
            role: "tool".into(),
            text: serde_json::to_string(&call).unwrap(),
            first: true,
            sent_at: None,
        }
    };
    let rows = [
        tool("Edit", "a.rs"),
        tool("Write", "b.rs"),
        tool("Edit", "b.rs"),
        tool("Read", "c.rs"),
    ];
    assert_eq!(changed_files(&rows, 0), 2);
    assert_eq!(changed_files(&rows, 1), 1);
    assert_eq!(changed_files(&rows, 3), 0);
}
#[gpui::test]
fn newer_composer_and_saved_draft_survive_interrupted_edit_recovery(cx: &mut TestAppContext) {
    let store = Store::open(Path::new(":memory:")).unwrap();
    let session = store.create("codex", "/project").unwrap();
    let chat = cx.new(|cx| Conversation::new(session, Arc::new(Mutex::new(store)), cx));
    chat.update(cx, |chat, cx| {
        chat.loaded = true;
        chat.input
            .update(cx, |input, cx| input.set_text("New draft".into(), cx));
        chat.attachments.push("/tmp/new.png".into());
        chat.follow_ups.state.draft = Some(monocode_core::session::follow_up::Message {
            id: "saved".into(),
            text: "Existing draft".into(),
            ..Default::default()
        });
        chat.preserve_interrupted_edit("Edited prompt".into(), Vec::new(), 0, cx);
        assert_eq!(chat.input.read(cx).content, "New draft");
        assert_eq!(chat.attachments, vec![PathBuf::from("/tmp/new.png")]);
        assert_eq!(
            chat.follow_ups.state.draft.as_ref().unwrap().text,
            "Existing draft"
        );
        assert_eq!(chat.follow_ups.state.messages[0].text, "Edited prompt");
        assert!(chat.follow_ups.state.paused);
    });
}
#[gpui::test]
fn local_commands_and_cancel_never_remove_recalled_history(cx: &mut TestAppContext) {
    let mut store = Store::open(Path::new(":memory:")).unwrap();
    let mut session = store.create("pi", "/project").unwrap();
    store.provider_id(session.id, "thread").unwrap();
    session.provider_id = Some("thread".into());
    let mut transcript = Transcript::default();
    transcript.begin("user");
    transcript.append("Original");
    transcript.begin("assistant");
    transcript.append("Answer");
    store.write_rows(session.id, 0, &transcript.rows).unwrap();
    let edit = store.prepare_edit(session.id, &transcript.rows, 0).unwrap();
    let chat = cx.new(|cx| Conversation::new(session, Arc::new(Mutex::new(store)), cx));
    chat.update(cx, |chat, cx| {
        chat.loaded = true;
        chat.transcript
            .update(cx, |view, cx| view.patch(0, transcript.rows.clone(), cx));
        chat.editing_last_turn.prepared = Some(edit);
        chat.input
            .update(cx, |input, cx| input.set_text("/compact".into(), cx));
        assert!(chat.submit_edited(cx));
        assert!(chat.editing_last_turn.pending());
        assert_eq!(chat.input.read(cx).content, "/compact");
        assert!(chat.cancel_edit_last_turn(true, cx));
        assert!(!chat.editing_last_turn.pending());
        assert_eq!(chat.input.read(cx).content, "/compact");
        assert_eq!(
            locked(&chat.store)
                .unwrap()
                .load(chat.summary.id)
                .unwrap()
                .rows,
            transcript.rows
        );
    });
}
#[gpui::test]
fn persisted_recovery_guard_preserves_a_new_composer_until_cancel_lifts_it(
    cx: &mut TestAppContext,
) {
    let mut store = Store::open(Path::new(":memory:")).unwrap();
    let mut session = store.create("pi", "/project").unwrap();
    store.provider_id(session.id, "original").unwrap();
    session.provider_id = Some("original".into());
    let mut transcript = Transcript::default();
    transcript.begin("user");
    transcript.append("Original");
    store.write_rows(session.id, 0, &transcript.rows).unwrap();
    let edit = store.prepare_edit(session.id, &transcript.rows, 0).unwrap();
    store.provider_id(session.id, "changed").unwrap();
    // An in-place rewind whose local write fails leaves the guard.
    assert!(
        store
            .finish_edit(session.id, &edit, Some("original"))
            .is_err()
    );
    let chat = cx.new(|cx| Conversation::new(session, Arc::new(Mutex::new(store)), cx));
    chat.update(cx, |chat, cx| {
        chat.loaded = true;
        chat.input.update(cx, |input, cx| {
            input.set_text("Unsent edited prompt".into(), cx)
        });
        assert!(chat.submit_edited(cx));
        assert!(chat.editing_last_turn.blocked);
        // Escape leaves the guard in place.
        chat.stop_if_busy(cx);
        assert!(chat.editing_last_turn.blocked);
        assert_eq!(chat.input.read(cx).content, "Unsent edited prompt");
        // Cancel edit is the way out: it lifts the guard and keeps the draft.
        assert!(chat.cancel_edit_last_turn(true, cx));
        assert!(!chat.editing_last_turn.pending());
        assert_eq!(chat.input.read(cx).content, "Unsent edited prompt");
        let store = locked(&chat.store).unwrap();
        assert!(
            store
                .edit_recovery_reason(chat.summary.id)
                .unwrap()
                .is_none()
        );
    });
}
