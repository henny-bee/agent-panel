use super::*;

#[gpui::test]
fn mobile_catalog_matches_sidebar_visibility_for_saved_and_open_sessions(
    cx: &mut gpui::TestAppContext,
) {
    let (window, saved, blank) = crate::app::shell::tests::window(cx);
    window
        .update(cx, |shell, _, cx| {
            let paths = shell.mobile_projects(cx);
            let ids = |shell: &Shell, cx: &App| {
                shell
                    .mobile_catalog_sessions(&paths, cx)
                    .into_iter()
                    .map(|entry| entry.id)
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                ids(shell, cx),
                vec![saved],
                "empty tabs must stay out of the phone's history"
            );
            let view = shell.sessions.remove(&saved).unwrap();
            let entries = shell.mobile_catalog_sessions(&paths, cx);
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].id, saved);
            assert!(
                entries[0].live.is_none(),
                "history must not need an open desktop view"
            );
            shell.sessions.insert(saved, view.clone());
            shell.sidebar_hidden_sessions.insert(saved);
            assert!(ids(shell, cx).is_empty());
            shell.sidebar_hidden_sessions.remove(&saved);
            view.update(cx, |chat, _| chat.summary.archived = true);
            assert!(ids(shell, cx).is_empty());
            shell.sessions[&blank].update(cx, |chat, _| chat.summary.has_messages = true);
            assert_eq!(
                ids(shell, cx),
                vec![blank],
                "a real conversation with a default title is still history"
            );
        })
        .unwrap();
}

#[gpui::test]
fn saved_and_live_usage_tokens_match_and_shell_routes_continuation_controls(
    cx: &mut gpui::TestAppContext,
) {
    let (window, id, _) = crate::app::shell::tests::window(cx);
    window
        .update(cx, |shell, _, cx| {
            shell.sessions[&id].update(cx, |v, cx| v.load(cx))
        })
        .unwrap();
    cx.run_until_parked();
    let conversation = window
        .update(cx, |shell, _, cx| {
            let conversation = shell.mobile_session_id(id);
            shell.sessions[&id].update(cx, |v, cx| {
                v.mobile_configure(
                    Some((Kind::Codex, Some("fixture-model".into()))),
                    &HashMap::new(),
                    None,
                    cx,
                );
                v.settings.usage_limit = Some(monocode_core::session::agent::Limit {
                    resets_at: Some(monocode_core::provider::usage::now_ms() + 60_000),
                    resume_at_reset: false,
                });
                let token = v.mobile_usage_limit(cx)["token"].clone();
                v.mobile_resume_usage(&token, Some(true), cx).unwrap();
            });
            conversation
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, window, cx| {
            let store = locked(&shell.store).unwrap();
            let summary = shell.session_summaries[&id].clone();
            let saved = Saved {
                id,
                settings: store.agent_settings(id).unwrap().unwrap(),
                rows: store.load(id).unwrap().rows,
                harness: summary.provider,
                cwd: summary.cwd,
                history: None,
                mono: None,
                launches: HashMap::new(),
            };
            drop(store);
            let from_disk = saved_snapshot(&saved).usage_limit;
            let live = shell
                .with_live_snapshot(&Target::Session(id), cx, |s| s.usage_limit.clone())
                .unwrap();
            assert_eq!(from_disk["token"], live["token"]);
            let params =
                json!({"conversationId":conversation, "token":from_disk["token"], "enabled":false});
            shell
                .mobile_session_command("session.resumeUsageAtReset", &params, window, cx)
                .unwrap();
            assert!(
                !shell.sessions[&id]
                    .read(cx)
                    .settings
                    .usage_limit
                    .as_ref()
                    .unwrap()
                    .resume_at_reset
            );
            assert!(
                shell
                    .mobile_session_command("session.resumeUsageAtReset", &params, window, cx)
                    .unwrap_err()
                    .contains("changed")
            );
            shell.sessions[&id].update(cx, |v, _| {
                v.settings.usage_limit.as_mut().unwrap().resets_at = None;
                v.settings.mono = Some(monocode_core::session::agent::MonoContext {
                    mono: Default::default(),
                    root: "/fixture".into(),
                    executable: "/fixture/binary".into(),
                });
            });
            let token = shell.sessions[&id].read(cx).mobile_usage_limit(cx)["token"].clone();
            let params = json!({"conversationId":conversation, "token":token});
            shell
                .mobile_session_command("session.resumeUsage", &params, window, cx)
                .unwrap();
            assert!(shell.sessions[&id].read(cx).settings.usage_limit.is_none());
            assert!(
                shell
                    .mobile_session_command("session.resumeUsage", &params, window, cx)
                    .unwrap_err()
                    .contains("no longer paused")
            );
            shell.session_summaries.remove(&id);
            assert!(
                shell
                    .mobile_session_command("session.resumeUsage", &params, window, cx)
                    .unwrap_err()
                    .contains("no longer available")
            );
        })
        .unwrap();
}

#[test]
fn saved_usage_state_carries_provider_reset_and_recomputes_eligibility() {
    let reset = monocode_core::provider::usage::now_ms() + 7 * 24 * 3_600_000;
    let mut saved = Saved {
        id: 42,
        settings: Settings {
            usage_limit: Some(monocode_core::session::agent::Limit {
                resets_at: Some(reset),
                resume_at_reset: true,
            }),
            ..Default::default()
        },
        rows: Vec::new(),
        harness: "codex".into(),
        cwd: "/fixture".into(),
        history: None,
        mono: None,
        launches: HashMap::new(),
    };
    let state = saved_snapshot(&saved).usage_limit;
    assert_eq!(state["resetsAt"], reset);
    assert_eq!(state["canResume"], false);
    assert_eq!(state["resumeAtReset"], true);
    saved.settings.usage_limit.as_mut().unwrap().resets_at =
        Some(monocode_core::provider::usage::now_ms() - 1);
    let state = saved_snapshot(&saved).usage_limit;
    assert_eq!(state["canResume"], true);
    assert_eq!(state["resetLabel"], "Limit has reset");
    saved.cwd = "remote://host/fixture".into();
    let remote = saved_snapshot(&saved).usage_limit;
    assert_eq!(remote["canResume"], false);
    assert_eq!(remote["canAutoResume"], false);
    saved.settings.usage_limit = None;
    assert!(saved_snapshot(&saved).usage_limit.is_null());
}

#[test]
fn model_choices_use_the_same_ids_as_session_cards_and_normalize_back_for_the_cli() {
    let model = monocode_core::provider::catalog::Model {
        alias_target: None,
        id: "claude-opus-5-5".into(),
        name: "Claude Opus 5.5".into(),
        efforts: vec!["high".into()],
        default_effort: Some("high".into()),
        fast: true,
        thinking: false,
        default: true,
        tiers: vec![],
        controls: vec![],
    };
    assert_eq!(
        mobile_catalog::current_model_id(Kind::Claude, None, std::slice::from_ref(&model)),
        "claude:opus-5-5"
    );
    assert_eq!(
        mobile_catalog::current_model_id(Kind::Claude, Some("opus"), std::slice::from_ref(&model)),
        "claude:opus"
    );
    let choice = catalog_model(Kind::Claude, model);
    assert_eq!(choice["id"], "claude:opus-5-5");
    assert_eq!(choice["name"], "Claude Opus 5.5");
    assert_eq!(choice["settings"][0]["value"], "high");
    assert_eq!(
        monocode_core::store::native_model(choice["id"].as_str().unwrap(), "claude").as_deref(),
        Some("claude-opus-5-5")
    );
}

#[test]
fn remote_projects_keep_hidden_providers_in_the_model_sheet() {
    let prefs = crate::ui::preferences::Preferences {
        hidden_providers: vec!["claude".into()],
        ..Default::default()
    };
    assert_eq!(hidden_providers(&prefs, "/work/project"), ["claude"]);
    assert!(hidden_providers(&prefs, "remote://env/work/project").is_empty());
}

#[test]
fn a_project_hidden_provider_is_left_out_of_the_model_sheet_for_local_projects() {
    let mut prefs = crate::ui::preferences::Preferences {
        hidden_providers: vec!["claude".into()],
        ..Default::default()
    };
    prefs
        .project_providers
        .entry("/work/project".into())
        .or_default()
        .hidden = vec!["codex".into()];
    assert_eq!(
        hidden_providers(&prefs, "/work/project"),
        ["claude", "codex"]
    );
    assert_eq!(hidden_providers(&prefs, "/work/other"), ["claude"]);
    assert!(hidden_providers(&prefs, "remote://env/work/project").is_empty());
}

#[test]
fn an_explicit_phone_provider_without_a_model_uses_its_configured_model() {
    let mut prefs = crate::ui::preferences::Preferences {
        default_models: [("claude".into(), "claude-opus-5-5".into())].into(),
        ..Default::default()
    };
    assert_eq!(
        explicit_model(&prefs, "/work/project", Kind::Claude).as_deref(),
        Some("claude-opus-5-5")
    );
    let project = prefs
        .project_providers
        .entry("/work/project".into())
        .or_default();
    project.models = [("claude".into(), "claude-sonnet-5-5".into())].into();
    assert_eq!(
        explicit_model(&prefs, "/work/project", Kind::Claude).as_deref(),
        Some("claude-sonnet-5-5")
    );
    assert_eq!(explicit_model(&prefs, "/work/project", Kind::Codex), None);
}

#[test]
fn an_explicit_phone_provider_that_is_the_default_ignores_the_recent_model() {
    let mut prefs = crate::ui::preferences::Preferences {
        recent_models: vec![crate::ui::preferences::ModelChoice {
            provider: "claude".into(),
            model: Some("claude-haiku-5-5".into()),
        }],
        ..Default::default()
    };
    let choice = glue::new_chat_choice::resolve(&prefs, "/work/project");
    assert_eq!(choice.kind, Kind::Claude);
    let recent = choice
        .model
        .and_then(|model| monocode_core::store::native_model(&model, "claude"));
    assert_eq!(recent.as_deref(), Some("claude-haiku-5-5"));
    let start = |prefs: &crate::ui::preferences::Preferences, chosen, requested: Option<&str>| {
        start_model(
            prefs,
            "/work/project",
            chosen,
            requested.map(Into::into),
            recent.clone(),
        )
    };
    // Omitted: the new-chat default, recent model included.
    assert_eq!(start(&prefs, None, None).as_deref(), recent.as_deref());
    // Explicit without a model: the harness default, not the recent model.
    assert_eq!(start(&prefs, Some(Kind::Claude), None), None);
    prefs
        .default_models
        .insert("claude".into(), "claude-opus-5-5".into());
    assert_eq!(
        start(&prefs, Some(Kind::Claude), None).as_deref(),
        Some("claude-opus-5-5")
    );
    // Explicit with a model: that model.
    assert_eq!(
        start(&prefs, Some(Kind::Claude), Some("claude:sonnet-5-5")).as_deref(),
        Some("claude-sonnet-5-5")
    );
}

#[test]
fn project_ids_match_the_earlier_desktop_encoding() {
    assert_eq!(project_id("/Users/me/My App"), "%2FUsers%2Fme%2FMy%20App");
    assert_eq!(project_id("C:\\code\\é"), "C%3A%5Ccode%5C%C3%A9");
    assert_eq!(project_id("a-b_c.d!~*'()"), "a-b_c.d!~*'()");
}

#[test]
fn older_page_turns_name_their_page_and_window_turns_do_not() {
    assert_eq!(page_turn("p42t7"), Some((42, 7)));
    assert_eq!(page_turn("p-3t0"), Some((-3, 0)));
    // The loaded window's own turns, and anything malformed, are not pages.
    assert_eq!(page_turn("t7"), None);
    assert_eq!(page_turn("p42"), None);
    assert_eq!(page_turn("pxtn"), None);
}

#[test]
fn messages_need_text_or_files_and_settings_drop_malformed_values() {
    assert!(message_text(&json!({"text": "  "})).is_err());
    assert_eq!(message_text(&json!({"text": " hi "})).unwrap(), "hi");
    assert!(message_text(&json!({"text": "", "attachments": [{}]})).is_ok());
    let settings = model_settings(&json!({"modelSettings": {"effort": "high", "bad": 3}}));
    assert_eq!(settings.len(), 1);
    assert_eq!(settings["effort"], "high");
}

#[test]
fn only_a_claude_session_shown_in_the_cli_is_read_from_the_cli_log() {
    let cli = Settings {
        terminal: true,
        ..Settings::default()
    };
    assert_eq!(
        requests::cli_binding(&cli, "claude", Some("abc")),
        Some("abc")
    );
    assert_eq!(requests::cli_binding(&cli, "claude", None), None);
    assert_eq!(requests::cli_binding(&cli, "codex", Some("abc")), None);
    assert_eq!(
        requests::cli_binding(&Settings::default(), "claude", Some("abc")),
        None
    );
}
