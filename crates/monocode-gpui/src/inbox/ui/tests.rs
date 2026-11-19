use super::*;
use gpui::{
    AssetSource, Bounds, Render, TestAppContext, WindowBounds, WindowHandle, WindowOptions, point,
    size,
};
use std::sync::{Arc, Mutex};

struct Fixture {
    inbox: Entity<Inbox>,
    item: Item,
}
impl Render for Fixture {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.inbox.update(cx, |inbox, cx| {
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(inbox.pr_actions(&self.item, cx))
                .child(inbox.summary_view(&self.item, window, cx))
        })
    }
}
fn fixture(cx: &mut TestAppContext) -> WindowHandle<Fixture> {
    let window = cx.update(|cx| cx.open_window(WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(0.), px(0.)), size(px(1100.), px(700.))))),
        ..Default::default()
    }, |_, cx| cx.new(|cx| {
        let item = Item { provider: "github".into(), kind: "pr".into(), state: "open".into(), number: 7, ..Default::default() };
        let inbox = cx.new(|cx| {
            let store = Arc::new(Mutex::new(monocode_core::store::Store::open(std::path::Path::new(":memory:")).unwrap()));
            let mut view = Inbox::new_inner(store, vec![], PathBuf::from("/private/tmp/inbox-regression"), false, cx);
            view.snapshot.items = vec![item.clone()]; view.selected = Some(item.key());
            view.details = Some(Details { body: "Description".into(), ..Default::default() });
            let comments = (0..100).map(|ix| model::Comment {
                id: format!("comment-{ix}"), body: (0..30).map(|line| format!("### Finding {line}\n\nReview **retry handling** in `src/main.rs` before returning.\n\n")).collect(),
                created_at: format!("2026-10-22T08:{:02}:00Z", ix % 60), ..Default::default()
            }).collect::<Vec<_>>();
            view.comments = comments.iter().map(|comment| (comment.clone(), cx.new(|cx| document::InboxDocument::new(comment.body.clone(), PathBuf::from("/repo"), cx)))).collect();
            view.thread = Some(Thread { comments, ..Default::default() });
            view.rebuild_activity(); view
        });
        Fixture { inbox, item }
    })).unwrap());
    draw(window, cx);
    window
}
fn draw(window: WindowHandle<Fixture>, cx: &mut TestAppContext) {
    window.update(cx, |_, _, cx| cx.notify()).unwrap();
    cx.refresh().unwrap();
    cx.run_until_parked();
}
#[gpui::test]
fn the_inbox_opens_scoped_to_its_project_and_labels_projects_by_appearance(
    cx: &mut TestAppContext,
) {
    let window = fixture(cx);
    window
        .update(cx, |fixture, _, cx| {
            fixture.inbox.update(cx, |inbox, _| {
                let home = PathBuf::from("/work/home");
                let other = PathBuf::from("/work/other");
                assert_eq!(
                    super::super::other_projects(&[home.clone(), other.clone()], &home),
                    vec!["/work/other".to_string()]
                );
                assert_eq!(inbox.project_label("/work/other"), "other");
                inbox.appearances.insert(
                    "/work/other".into(),
                    monocode_core::project::appearance::ProjectAppearance {
                        name: "Other app".into(),
                        ..Default::default()
                    },
                );
                assert_eq!(inbox.project_label("/work/other"), "Other app");
            });
        })
        .unwrap();
}
#[gpui::test]
fn switching_projects_drops_the_previous_projects_selection_and_detail(cx: &mut TestAppContext) {
    let window = fixture(cx);
    window
        .update(cx, |fixture, _, cx| {
            fixture.inbox.update(cx, |inbox, cx| {
                let (a, b) = (PathBuf::from("/work/a"), PathBuf::from("/work/b"));
                let item = |number, project: &PathBuf| Item {
                    provider: "github".into(),
                    kind: "pr".into(),
                    state: "open".into(),
                    repo: format!("acme/app-{number}"),
                    number,
                    project_path: project.to_string_lossy().into_owned(),
                    ..Default::default()
                };
                // Keep the list fetch queued instead of reaching real providers.
                inbox.loading = true;
                inbox.snapshot.items = vec![item(1, &a), item(2, &b)];
                inbox.set_context(vec![a.clone(), b.clone()], a.clone(), cx);
                inbox.selected = Some(item(1, &a).key());
                inbox.details = Some(Details {
                    body: "From project A".into(),
                    ..Default::default()
                });
                inbox.detail_loading = true;
                let request = inbox.detail_generation;
                let scope = inbox.fetch_scope();

                inbox.set_context(vec![a.clone(), b.clone()], b.clone(), cx);

                assert_eq!(inbox.selected, None);
                assert!(inbox.details.is_none() && inbox.thread.is_none());
                assert!(inbox.document.is_none() && inbox.comments.is_empty());
                assert!(!inbox.detail_loading);
                assert_ne!(
                    inbox.detail_generation, request,
                    "project A's detail request must be discarded"
                );
                assert_ne!(inbox.fetch_scope(), scope, "a fetch made for A is stale");
                assert_eq!(
                    inbox
                        .visible_items(cx)
                        .iter()
                        .map(Item::key)
                        .collect::<Vec<_>>(),
                    vec![item(2, &b).key()]
                );
            });
        })
        .unwrap();
}
#[gpui::test]
fn long_discussion_materializes_only_viewport_comments_and_reply_focus_reaches_composer(
    cx: &mut TestAppContext,
) {
    let window = fixture(cx);
    window
        .update(cx, |fixture, _, cx| {
            fixture.inbox.update(cx, |inbox, cx| {
                let measured = inbox
                    .comments
                    .iter()
                    .filter(|(_, doc)| doc.read(cx).measured_rows() > 0)
                    .count();
                assert!(
                    measured > 0 && measured < 10,
                    "only viewport/overscan cards should be laid out, got {measured}"
                );
                assert_eq!(inbox.summary_scroll.item_count(), 103);
                inbox.summary_scroll.scroll_by(px(2500.));
            })
        })
        .unwrap();
    draw(window, cx);
    window
        .update(cx, |fixture, window, cx| {
            fixture.inbox.update(cx, |inbox, cx| {
                assert!(inbox.summary_scroll.logical_scroll_top().item_ix > 3);
                inbox.reply = Some(("thread-1".into(), "Reviewer".into()));
                inbox.summary_scroll.remeasure_items(102..103);
                inbox.summary_scroll.scroll_to_end();
                window.focus(&inbox.comment.focus_handle(cx), cx);
            })
        })
        .unwrap();
    draw(window, cx);
    window
        .update(cx, |fixture, window, cx| {
            fixture.inbox.update(cx, |inbox, cx| {
                assert!(inbox.summary_scroll.logical_scroll_top().item_ix > 90);
                assert!(inbox.comment.focus_handle(cx).is_focused(window));
                inbox
                    .comment
                    .update(cx, |input, cx| input.set_text("A reply\n".repeat(8), cx));
            })
        })
        .unwrap();
    draw(window, cx);
    window
        .update(cx, |fixture, window, cx| {
            fixture.inbox.update(cx, |inbox, cx| {
                assert!(inbox.comment.focus_handle(cx).is_focused(window));
                assert_eq!(inbox.summary_scroll.item_count(), 103);
            })
        })
        .unwrap();
}
#[gpui::test]
fn expanding_comment_invalidates_its_height_and_preserves_timeline_order(cx: &mut TestAppContext) {
    let window = fixture(cx);
    let (id, previous) = window
        .update(cx, |fixture, _, cx| {
            fixture.inbox.update(cx, |inbox, cx| {
                let Activity::Comment(comment) = &inbox.activity[0] else {
                    panic!()
                };
                let id = comment.id.clone();
                let doc = &inbox
                    .comments
                    .iter()
                    .find(|(comment, _)| comment.id == id)
                    .unwrap()
                    .1;
                let previous = doc.read(cx).measured_rows();
                inbox.expanded_comments.insert(id.clone());
                inbox.summary_scroll.remeasure();
                (id, previous)
            })
        })
        .unwrap();
    draw(window, cx);
    window
        .update(cx, |fixture, _, cx| {
            fixture.inbox.update(cx, |inbox, cx| {
                let doc = &inbox
                    .comments
                    .iter()
                    .find(|(comment, _)| comment.id == id)
                    .unwrap()
                    .1;
                assert!(doc.read(cx).measured_rows() > previous);
                let Activity::Comment(first) = &inbox.activity[0] else {
                    panic!()
                };
                assert_eq!(first.id, id);
                assert_eq!(inbox.summary_scroll.logical_scroll_top().item_ix, 0);
            })
        })
        .unwrap();
}
#[gpui::test]
fn linear_team_filters_hide_cached_issues_before_refresh(cx: &mut TestAppContext) {
    let window = fixture(cx);
    window
        .update(cx, |fixture, _, cx| {
            fixture.inbox.update(cx, |inbox, cx| {
                inbox.state.source = "linear".into();
                inbox.state.filters = model::state::Filters::default();
                inbox.snapshot.items = ["web", "mobile"]
                    .into_iter()
                    .map(|team| Item {
                        provider: "linear".into(),
                        identifier: format!("{team}-1"),
                        team_id: team.into(),
                        ..Default::default()
                    })
                    .collect();
                let visible_teams = |inbox: &Inbox, cx: &App| {
                    inbox
                        .visible_items(cx)
                        .into_iter()
                        .map(|item| item.team_id)
                        .collect::<Vec<_>>()
                };
                assert_eq!(visible_teams(inbox, cx), ["web", "mobile"]);
                toggle(&mut inbox.state.hidden_linear_teams, "web".into());
                assert_eq!(visible_teams(inbox, cx), ["mobile"]);
                let mobile = inbox.visible_items(cx).remove(0);
                inbox.selected = Some(mobile.key());
                inbox.filters_changed(false, cx);
                assert_eq!(inbox.item().map(|item| item.team_id), Some("mobile".into()));
                toggle(&mut inbox.state.hidden_linear_teams, "mobile".into());
                assert!(inbox.visible_items(cx).is_empty());
                inbox.filters_changed(false, cx);
                assert!(inbox.item().is_none());
                toggle(&mut inbox.state.hidden_linear_teams, "web".into());
                assert_eq!(visible_teams(inbox, cx), ["web"]);
                inbox.state.hidden_linear_teams.clear();
                assert_eq!(visible_teams(inbox, cx), ["web", "mobile"]);
            });
        })
        .unwrap();
}
#[gpui::test]
fn arriving_comment_preserves_the_readers_item_and_offset(cx: &mut TestAppContext) {
    let window = fixture(cx);
    window
        .update(cx, |fixture, _, cx| {
            fixture
                .inbox
                .update(cx, |inbox, _| inbox.summary_scroll.scroll_by(px(2500.)))
        })
        .unwrap();
    draw(window, cx);
    let before = window
        .update(cx, |fixture, _, cx| {
            fixture.inbox.update(cx, |inbox, cx| {
                let before = inbox.summary_scroll.logical_scroll_top();
                let comment = model::Comment {
                    id: "new-comment".into(),
                    body: "A new reply".into(),
                    created_at: "2026-10-22T09:00:00Z".into(),
                    ..Default::default()
                };
                let doc = cx.new(|cx| {
                    document::InboxDocument::new(comment.body.clone(), PathBuf::from("/repo"), cx)
                });
                inbox.comments.push((comment.clone(), doc));
                inbox.thread.as_mut().unwrap().comments.push(comment);
                inbox.rebuild_activity();
                before
            })
        })
        .unwrap();
    draw(window, cx);
    window
        .update(cx, |fixture, _, cx| {
            fixture.inbox.update(cx, |inbox, _| {
                let after = inbox.summary_scroll.logical_scroll_top();
                assert_eq!(after.item_ix, before.item_ix);
                assert!((after.offset_in_item - before.offset_in_item).abs() < px(1.));
                assert_eq!(inbox.summary_scroll.item_count(), 104);
            })
        })
        .unwrap();
}

#[test]
fn toolbar_icons_have_explicit_ink_and_the_original_distinct_assets() {
    for (name, ink) in [
        ("git-merge", shell_fill()),
        ("git-pull-request-draft", content(0.7)),
        ("git-pull-request-closed", content(0.7)),
        ("external-link", content(0.7)),
    ] {
        let mut icon = action_icon(name, ink);
        assert_eq!(icon.style().text.color, Some(ink));
        let bytes = crate::ui::assets::Assets.load(name).unwrap().unwrap();
        assert!(std::str::from_utf8(&bytes).unwrap().contains("<svg"));
    }
}
