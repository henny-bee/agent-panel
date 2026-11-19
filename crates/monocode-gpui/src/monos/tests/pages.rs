use super::*;

#[gpui::test]
fn missing_document_uses_error_cards_without_moving_content(cx: &mut TestAppContext) {
    let s = setup(cx);
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    visual.refresh().unwrap();
    let body = visual.debug_bounds("main-pane-frame").unwrap();
    s.manager.update(visual, |m, cx| {
        m.failed_reports.insert("failed-report".into());
        m.open_document("deleted-document", cx);
    });
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert_eq!(
        s.manager
            .read_with(visual, |m, _| m.error.clone())
            .as_deref(),
        Some("Document unavailable. It may have been deleted.")
    );
    let error = visual.debug_bounds("mono-error").unwrap();
    let viewport = s
        .window
        .update(visual, |_, w, _| w.viewport_size())
        .unwrap();
    assert_eq!(error.top(), px(12.));
    assert_eq!(error.right(), viewport.width - px(12.));
    assert_eq!(error.size.width, px(360.));
    assert_eq!(visual.debug_bounds("main-pane-frame").unwrap(), body);

    let close = visual.debug_bounds("mono-error-close").unwrap();
    visual.simulate_click(close.center(), Default::default());
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("mono-error").is_none());
    s.manager.read_with(visual, |m, _| {
        assert!(m.error.is_none());
        assert!(m.failed_reports.is_empty());
        assert!(
            m.settings.is_some(),
            "dismissing an error keeps settings open"
        );
        assert_eq!(m.active.as_deref(), Some(s.id.as_str()));
    });
}

#[gpui::test]
fn saved_desktop_mono_opens_in_actual_manager_without_replacing_its_conversation(
    cx: &mut TestAppContext,
) {
    let db = Store::open_monocode(std::path::Path::new(":memory:")).unwrap();
    let session = db.create("codex", "/saved-project").unwrap();
    let original = db.persisted_session_id(session.id).unwrap();
    let settings = Settings {
        model: Some("gpt-6.1-sol".into()),
        access: AccessMode::Auto,
        ..Default::default()
    };
    db.configure_agent(session.id, "codex", &settings).unwrap();
    let mut roster = Roster::default();
    roster.monos.push(Mono {
        id: "desktop-original-mono".into(),
        session_id: Some(session.id),
        name: "Big Boss".into(),
        mascot: "ghost".into(),
        color: 0xe25a9e,
        projects: vec!["/saved-project".into()],
        ..Default::default()
    });
    roster.save(&db).unwrap();
    let summaries = db.sessions().unwrap();
    let store = Arc::new(Mutex::new(db));
    let database = store.clone();
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| Shell::new(database, summaries, "/saved-project".into(), None, cx))
        })
        .unwrap()
    });
    cx.run_until_parked();
    let manager = window
        .update(cx, |shell, _, _| shell.mono_manager())
        .unwrap();
    window
        .update(cx, |_, window, cx| {
            manager.update(cx, |m, cx| m.open("desktop-original-mono", window, cx));
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        manager.update(cx, |m, cx| {
            m.ensure_chat("desktop-original-mono", cx);
            let mono = m.current().unwrap();
            assert_eq!(mono.id, "desktop-original-mono");
            assert_eq!(mono.session_id, Some(session.id));
            assert_eq!(mono.name, "Big Boss");
            assert_eq!(mono.mascot, "ghost");
            let chat = m.chats[&mono.id].read(cx);
            assert_eq!(chat.summary.id, session.id);
            assert_eq!(chat.summary.provider, "codex");
            assert_eq!(chat.settings.model.as_deref(), Some("gpt-6.1-sol"));
            assert_eq!(m.chats.len(), 1);
        });
    });
    let db = locked(&store).unwrap();
    assert_eq!(db.sessions().unwrap().len(), 1);
    assert_eq!(db.persisted_session_id(session.id).unwrap(), original);
}

#[gpui::test]
fn settings_reset_restores_only_mono_soul_and_name(cx: &mut TestAppContext) {
    let s = setup(cx);
    mono::files::save(&s.root, &s.id, None, "SOUL.md", "# Custom soul", None).unwrap();
    mono::files::save(&s.root, &s.id, None, "MEMORY.md", "Keep this memory", None).unwrap();
    let before = cx.update(|cx| {
        s.manager.update(cx, |m, cx| {
            m.roster.get_mut(&s.id).unwrap().name = "Custom name".into();
            m.save(cx);
            m.roster.get(&s.id).unwrap().clone()
        })
    });
    cx.run_until_parked();
    let _reset = cx.update(|cx| s.manager.update(cx, |m, cx| m.reset_defaults(&s.id, cx)));
    cx.run_until_parked();
    let roster = Roster::load(&locked(&s.store).unwrap()).unwrap();
    let after = roster.get(&s.id).unwrap();
    assert!(after.name.is_empty());
    assert_eq!(after.session_id, before.session_id);
    assert_eq!(after.projects, before.projects);
    assert_eq!(after.habit_sessions, before.habit_sessions);
    let files = mono::files::load(&s.root, &s.id, None).unwrap();
    assert_eq!(files.soul.as_deref(), Some(mono::DEFAULT_SOUL));
    assert_eq!(files.memory, "Keep this memory");
}

#[gpui::test]
fn detail_pages_slide_over_the_overview_and_back_without_recreating_content(
    cx: &mut TestAppContext,
) {
    for kind in ["markdown", "html", "session"] {
        cx.update(|cx| cx.set_reduce_motion(true));
        let s = setup(cx);
        let session = s
            .window
            .update(cx, |shell, _, _| shell.workspace.active.unwrap())
            .unwrap();
        s.window
            .update(cx, |_, _, cx| {
                s.manager.update(cx, |m, cx| {
                    m.settings = None;
                    m.overview_tab = OverviewTab::Documents;
                    m.roster.get_mut(&s.id).unwrap().documents.push(
                        monocode_core::session::mono_model::Document {
                            id: "slide-document".into(),
                            kind: kind.into(),
                            title: "Slide document".into(),
                            summary: String::new(),
                            body: "# Document\n\nKeep this reader alive.".into(),
                            created_at: 1,
                            updated_at: 1,
                        },
                    );
                    cx.notify();
                });
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
        visual.run_until_parked();
        visual.update(|_, cx| cx.set_reduce_motion(false));
        let action = if kind == "session" {
            json!({"type":"session", "sessionId":session})
        } else {
            json!({"type":"document", "documentId":"slide-document"})
        };
        s.window
            .update(visual, |_, _, cx| {
                s.manager
                    .update(cx, |m, cx| m.card_action(&s.id, &action, cx))
            })
            .unwrap();
        visual.run_until_parked();
        let page: &'static str = Box::leak(
            format!(
                "mono-detail-page-{}",
                if kind == "session" {
                    format!("session:{session}")
                } else {
                    "document:slide-document".into()
                }
            )
            .into_boxed_str(),
        );
        let advance = |visual: &mut gpui::VisualTestContext, ms| {
            let details = s.manager.read_with(visual, |m, _| m.details.clone());
            details.update(visual, |d, cx| d.elapsed(Duration::from_millis(ms), cx));
            visual
                .update_window(s.window.into(), |_, w, cx| {
                    w.simulate_next_frame(cx);
                    w.draw(cx).clear(cx);
                })
                .unwrap();
        };
        advance(visual, 70);
        let stack = visual.debug_bounds("mono-detail-stack").unwrap();
        let incoming = visual.debug_bounds(page).unwrap();
        assert!(incoming.left() > stack.left() && incoming.left() < stack.right());
        assert_eq!(
            incoming.size, stack.size,
            "sliding pages keep their full layout size"
        );
        assert!(visual.debug_bounds("mono-detail-base").unwrap().left() < stack.left());
        advance(visual, 1000);
        assert_eq!(visual.debug_bounds(page).unwrap().left(), stack.left());
        let reader = s.manager.read_with(visual, |m, cx| match kind {
            "html" => m.html_documents["slide-document"].entity_id(),
            "session" => m
                .owner
                .upgrade()
                .unwrap()
                .read(cx)
                .mono_session_entity(session)
                .unwrap()
                .entity_id(),
            _ => m.documents["slide-document"].entity_id(),
        });
        assert_eq!(
            s.manager
                .read_with(visual, |m, cx| m.native_detail_content(cx)),
            kind == "html"
        );
        let back = visual.debug_bounds("mono-page-back").unwrap().center();
        visual.simulate_click(back, Default::default());
        visual.run_until_parked();
        advance(visual, 70);
        let outgoing = visual
            .debug_bounds(page)
            .expect("Back keeps the outgoing page mounted");
        assert!(outgoing.left() > stack.left() && outgoing.left() < stack.right());
        advance(visual, 1000);
        assert!(visual.debug_bounds(page).is_none());
        assert_eq!(
            visual.debug_bounds("mono-detail-base").unwrap().left(),
            stack.left()
        );
        assert!(
            !s.manager
                .read_with(visual, |m, cx| m.native_detail_content(cx))
        );

        visual.update(|_, cx| cx.set_reduce_motion(true));
        s.window
            .update(visual, |_, _, cx| {
                s.manager
                    .update(cx, |m, cx| m.card_action(&s.id, &action, cx))
            })
            .unwrap();
        visual.run_until_parked();
        assert_eq!(visual.debug_bounds(page).unwrap().left(), stack.left());
        s.manager.read_with(visual, |m, cx| {
            let reopened = match kind {
                "html" => m.html_documents["slide-document"].entity_id(),
                "session" => m
                    .owner
                    .upgrade()
                    .unwrap()
                    .read(cx)
                    .mono_session_entity(session)
                    .unwrap()
                    .entity_id(),
                _ => m.documents["slide-document"].entity_id(),
            };
            assert_eq!(reader, reopened);
        });
        visual.update(|_, cx| cx.set_reduce_motion(false));
        s.window
            .update(visual, |_, w, cx| {
                s.manager.update(cx, |m, cx| {
                    m.close(cx);
                    m.open(&s.id, w, cx);
                });
            })
            .unwrap();
        visual.run_until_parked();
        let tab = if kind == "session" {
            OverviewTab::Sessions
        } else {
            OverviewTab::Documents
        };
        assert_eq!(
            visual.debug_bounds(page).unwrap().left(),
            stack.left(),
            "returning to a Mono shows its remembered page in place, without sliding"
        );
        assert_eq!(s.manager.read_with(visual, |m, _| m.overview_tab), tab);
    }
}

#[gpui::test]
fn each_mono_remembers_its_details_tab_and_open_session(cx: &mut TestAppContext) {
    let s = setup(cx);
    let session = s
        .window
        .update(cx, |shell, _, _| shell.workspace.active.unwrap())
        .unwrap();
    let action = json!({"type":"session", "sessionId":session});
    let other = s
        .window
        .update(cx, |_, w, cx| {
            s.manager.update(cx, |m, cx| {
                m.card_action(&s.id, &action, cx);
                m.create(vec![s.project.clone()], w, cx);
                m.active.clone().unwrap()
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert_ne!(other, s.id);
    let view = |cx: &mut TestAppContext| {
        s.manager.read_with(cx, |m, _| {
            (m.active.clone().unwrap(), m.overview_tab, m.route.clone())
        })
    };
    assert_eq!(view(cx), (other.clone(), OverviewTab::Overview, None));
    s.window
        .update(cx, |_, w, cx| {
            s.manager.update(cx, |m, cx| {
                m.select_overview(OverviewTab::Habits, cx);
                m.open(&s.id, w, cx);
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        view(cx),
        (
            s.id.clone(),
            OverviewTab::Sessions,
            Some(Route::Session(session))
        )
    );
    // An archived session falls back to the list; the other Mono keeps its tab.
    s.window
        .update(cx, |shell, w, cx| {
            s.manager.update(cx, |m, cx| m.open(&other, w, cx));
            shell.mono_remove(session, false, w, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(view(cx), (other.clone(), OverviewTab::Habits, None));
    s.window
        .update(cx, |_, w, cx| {
            s.manager.update(cx, |m, cx| m.open(&s.id, w, cx))
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(view(cx), (s.id.clone(), OverviewTab::Sessions, None));
    // A card from another Mono's chat switches to that Mono's own view and
    // leaves the one it came from intact.
    let pr = json!({"type":"pr", "repo":"o/r", "number":1});
    s.manager.update(cx, |m, cx| m.card_action(&other, &pr, cx));
    assert_eq!(view(cx), (other.clone(), OverviewTab::Habits, None));
    s.window
        .update(cx, |_, w, cx| {
            s.manager.update(cx, |m, cx| m.open(&s.id, w, cx))
        })
        .unwrap();
    assert_eq!(view(cx), (s.id.clone(), OverviewTab::Sessions, None));
}

#[gpui::test]
fn a_card_switching_monos_shows_the_target_monos_habits(cx: &mut TestAppContext) {
    let s = setup(cx);
    let mut h = Habit::new(
        "Daily review".into(),
        "Review changes.".into(),
        habits::Schedule::default(),
        monocode_core::sys::time::now_ms(),
    )
    .unwrap();
    h.id = "target-habit".into();
    habits::update(&s.root, &s.id, |all| {
        all.push(h.clone());
        Ok(())
    })
    .unwrap();
    let other = s
        .window
        .update(cx, |_, w, cx| {
            s.manager.update(cx, |m, cx| {
                m.open(&s.id, w, cx);
                m.create(vec![s.project.clone()], w, cx);
                m.active.clone().unwrap()
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert!(s.manager.read_with(cx, |m, _| m.habits.is_empty()));
    let ids = |cx: &mut TestAppContext| {
        s.manager.read_with(cx, |m, _| {
            m.habits.iter().map(|h| h.id.clone()).collect::<Vec<_>>()
        })
    };
    // Back on the first Mono, then a card from the second's chat switches over.
    s.window
        .update(cx, |_, w, cx| {
            s.manager.update(cx, |m, cx| m.open(&s.id, w, cx))
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(ids(cx), vec!["target-habit".to_string()]);
    let pr = json!({"type":"pr", "repo":"o/r", "number":1});
    s.manager.update(cx, |m, cx| m.card_action(&other, &pr, cx));
    assert!(
        ids(cx).is_empty(),
        "the previous Mono's habits must not linger"
    );
    cx.run_until_parked();
    assert_eq!(
        s.manager.read_with(cx, |m, _| m.active.clone()),
        Some(other)
    );
    assert!(ids(cx).is_empty());
}

#[gpui::test]
fn a_document_deleted_by_the_desktop_while_its_mono_is_inactive_returns_to_documents(
    cx: &mut TestAppContext,
) {
    let s = setup_shared(cx);
    let doc = "saved-document";
    // The desktop app writes the same SQLite file through its own connection.
    let desktop =
        rusqlite::Connection::open(std::path::Path::new(&s.project).join("native.sqlite")).unwrap();
    let original = locked(&s.store)
        .unwrap()
        .persisted_session_id(s.session)
        .unwrap();
    desktop
        .execute(
            "INSERT INTO notes(id,slug,title,body,tags_json,created_at,updated_at,content_kind,artifact_kind)
             VALUES(?1,?1,'Saved document','# Saved\n\nBody.','[]',10,20,'artifact','document')",
            [doc],
        )
        .unwrap();
    let blocks = json!([{"id":"reply","role":"assistant","text":"Saved it.",
        "artifactCards":[{"id":doc,"kind":"document","title":"Saved document","summary":""}]}]);
    desktop
        .execute(
            "UPDATE sessions SET blocks_json=?2 WHERE id=?1",
            rusqlite::params![original, blocks.to_string()],
        )
        .unwrap();
    s.manager.update(cx, |m, cx| m.refresh_work(&s.id, cx));
    cx.run_until_parked();
    let listed = |cx: &mut TestAppContext| {
        s.manager.read_with(cx, |m, _| {
            m.roster
                .get(&s.id)
                .unwrap()
                .documents
                .iter()
                .any(|d| d.id == doc)
        })
    };
    assert!(listed(cx), "the chat's card lists the saved document");

    // Open it on this Mono, then switch to another.
    let action = json!({"type":"document", "documentId":doc});
    s.manager
        .update(cx, |m, cx| m.card_action(&s.id, &action, cx));
    cx.run_until_parked();
    s.manager.read_with(cx, |m, _| {
        assert_eq!(m.route, Some(Route::Document(doc.into())));
        assert!(m.work.documents.contains_key(doc));
    });
    let other = s
        .window
        .update(cx, |_, w, cx| {
            s.manager.update(cx, |m, cx| {
                m.create(vec![s.project.clone()], w, cx);
                m.active.clone().unwrap()
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert_ne!(other, s.id);

    // The desktop deletes the artifact (src-tauri/src/artifacts.rs). Its card
    // stays in the transcript, so the Mono keeps a placeholder entry.
    desktop
        .execute(
            "DELETE FROM notes WHERE id = ?1 AND content_kind = 'artifact'",
            [doc],
        )
        .unwrap();
    s.window
        .update(cx, |_, w, cx| {
            s.manager.update(cx, |m, cx| m.open(&s.id, w, cx))
        })
        .unwrap();
    cx.run_until_parked();
    assert!(listed(cx), "the placeholder entry survives the deletion");
    s.manager.read_with(cx, |m, _| {
        assert_eq!(m.active.as_deref(), Some(s.id.as_str()));
        assert_eq!(m.overview_tab, OverviewTab::Documents);
        assert_eq!(m.route, None, "the deleted document's page is not restored");
        assert!(!m.work.documents.contains_key(doc));
        assert!(!m.documents.contains_key(doc));
        assert!(
            m.panel_views
                .values()
                .all(|v| v.route != Some(Route::Document(doc.into())))
        );
    });
}

#[gpui::test]
fn an_unavailable_document_page_keeps_its_back_control(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let s = setup(cx);
    s.manager.update(cx, |m, cx| {
        m.settings = None;
        m.overview_tab = OverviewTab::Documents;
        m.route = Some(Route::Document("never-loaded".into()));
        cx.notify();
    });
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    visual.refresh().unwrap();
    let back = visual
        .debug_bounds("mono-page-back")
        .expect("an unavailable document still offers Back");
    visual.simulate_click(back.center(), Default::default());
    visual.run_until_parked();
    assert_eq!(s.manager.read_with(visual, |m, _| m.route.clone()), None);
}

#[test]
fn restored_panel_views_keep_only_pages_whose_subject_exists() {
    let view = |route| PanelView {
        tab: OverviewTab::Habits,
        previous: OverviewTab::Overview,
        route: Some(route),
    };
    let missing = view(Route::Session(7)).restored(|_| false);
    assert_eq!((missing.tab, missing.route), (OverviewTab::Habits, None));
    let kept = view(Route::Document("d".into())).restored(|_| true);
    assert_eq!(kept.route, Some(Route::Document("d".into())));
    assert_eq!(
        view(Route::Habit("h".into())).restored(|_| true).route,
        None
    );
}
