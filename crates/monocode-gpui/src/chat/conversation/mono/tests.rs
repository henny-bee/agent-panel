use super::*;
use gpui::TestAppContext;
use monocode_core::{session::transcript::Transcript, store::Store};
use serde_json::json;
use std::sync::Mutex;

#[gpui::test]
fn long_reports_preserve_card_and_follow_up_positions(cx: &mut TestAppContext) {
    for report in [
        "x".repeat(monocode_core::session::transcript::ROW_BYTES + 1),
        "A finding\n".repeat(26),
        "**Habit report**\n\n🙂 café 漢字\n\n".repeat(100),
    ] {
        let mut store = Store::open_monocode(Path::new(":memory:")).unwrap();
        let session = store.create("codex", "/project").unwrap();
        let mut history = Transcript::default();
        history.begin("user");
        history.append("Earlier question");
        history.begin("assistant");
        history.append("Earlier reply");
        store.write_rows(session.id, 0, &history.rows).unwrap();
        let chat = cx.new(|cx| Conversation::new(session, Arc::new(Mutex::new(store)), cx));
        chat.update(cx, |chat, cx| {
            chat.loaded = true;
            chat.transcript
                .update(cx, |view, cx| view.patch(0, history.rows.clone(), cx));
            chat.mono_post("assistant", report.clone(), cx).unwrap();
            let card_index = chat.mono_rows(cx).len();
            let mut card = json!({
                "id":"report-card", "type":"document", "documentId":"report-document",
                "title":"Bug triage", "summary":"Detailed findings 🙂. ".repeat(200),
            });
            chat.mono_post("mono-card", card.to_string(), cx).unwrap();
            card["title"] = json!("Updated bug triage");
            chat.mono_patch_card(card_index, "report-card", card.clone(), cx)
                .unwrap();
            chat.mono_post("assistant", "Another report".into(), cx)
                .unwrap();

            let rows = chat.mono_rows(cx);
            assert_eq!(&rows[..history.rows.len()], history.rows);
            assert_eq!(rows[history.rows.len()].role, "mono-system");
            assert_eq!(
                rows[history.rows.len()..card_index]
                    .iter()
                    .filter(|row| row.role == "assistant")
                    .map(|row| row.text.as_str())
                    .collect::<String>(),
                report,
            );
            assert!(
                rows.iter()
                    .filter(|row| row.role == "assistant")
                    .all(|row| {
                        row.text.len() <= monocode_core::session::transcript::ROW_BYTES
                            && row.text.matches('\n').count() <= 12
                    })
            );
            assert_eq!(rows.iter().filter(|row| row.role == "mono-card").count(), 1);
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&rows[card_index].text).unwrap(),
                card,
            );
            assert_eq!(rows.last().unwrap().text, "Another report");
            let mut reloaded = locked(&chat.store)
                .unwrap()
                .load(chat.summary.id)
                .unwrap()
                .rows;
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&reloaded[card_index].text).unwrap(),
                card,
            );
            // Restoring the card id can change JSON object field order.
            reloaded[card_index].text = rows[card_index].text.clone();
            assert_eq!(
                reloaded, rows,
                "reloading keeps the displayed rows and their message boundaries",
            );
        });
    }
}
