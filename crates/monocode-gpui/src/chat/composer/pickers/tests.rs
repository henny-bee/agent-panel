use super::*;

#[gpui::test]
fn remote_model_availability_comes_from_the_host(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let list = cx.new(|cx| {
            ModelList::new_for(
                Kind::Codex,
                None,
                "remote://env/home/code",
                Default::default(),
                cx,
            )
            .hide_providers(vec!["claude".into()])
        });
        list.update(cx, |list, cx| {
            list.remote_providers = Some(vec![Kind::Claude]);
            list.missing = vec![Kind::Codex];
            list.tab = ModelTab::Harness(Kind::Claude);
            assert!(list.hidden.is_empty());
            assert!(list.enabled(Kind::Claude));
            assert!(!list.enabled(Kind::Codex));
            assert!(!list.enabled(Kind::Cursor));
            // Until its host catalog arrives, no local Claude aliases
            // are advertised as available remote models.
            let entries = list.entries(cx);
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].id, None);
        });
    });
}

#[test]
fn account_metadata_never_becomes_a_model_label() {
    for id in [
        "account-de37caea9106642f26b59835a2e9aa4f",
        "fixture-account-de37caea-9106-642f-26b5-9835a2e9aa4f",
    ] {
        let label = format!("{id} model");
        assert_eq!(model_name(Kind::Codex, Some(id)).as_ref(), "Codex");
        assert_eq!(
            model_display_name(Kind::Codex, id, &label).as_ref(),
            "Codex"
        );
        assert_eq!(
            model_display_name(Kind::Codex, "gpt-6.1-sol", &label).as_ref(),
            "GPT-6.1-Sol"
        );
    }
    assert_eq!(
        model_display_name(Kind::Codex, "custom-model", "My custom model").as_ref(),
        "My custom model"
    );
    assert_eq!(
        model_name(Kind::Codex, Some("custom-model")).as_ref(),
        "custom-model"
    );
}

#[gpui::test]
fn remembered_favorites_and_recents_keep_provider_identity_and_busy_rules(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|cx| {
        let prefs = crate::ui::preferences::Preferences {
            model_picker_tab: Some("favorites".into()),
            favorite_models: vec![
                crate::ui::preferences::ModelChoice {
                    provider: "codex".into(),
                    model: Some("gpt-saved".into()),
                },
                crate::ui::preferences::ModelChoice {
                    provider: "claude".into(),
                    model: Some("opus".into()),
                },
            ],
            recent_models: vec![crate::ui::preferences::ModelChoice {
                provider: "codex".into(),
                model: Some("gpt-recent".into()),
            }],
            ..Default::default()
        };
        cx.set_global(prefs);
        let list = cx.new(|cx| {
            ModelList::new_for(
                Kind::Codex,
                Some("gpt-saved".into()),
                "/work",
                Default::default(),
                cx,
            )
            .allow_provider_switch(false)
        });
        let favorites = list.read(cx).entries(cx);
        assert_eq!(favorites.len(), 2);
        assert_eq!(favorites[0].kind, Kind::Codex);
        assert!(favorites[0].enabled);
        assert_eq!(favorites[1].kind, Kind::Claude);
        assert!(!favorites[1].enabled);
        list.update(cx, |list, _| list.tab = ModelTab::Harness(Kind::Codex));
        let rows = list.read(cx).entries(cx);
        assert_eq!(rows[1].id.as_deref(), Some("gpt-recent"));
        assert_eq!(rows[2].id.as_deref(), Some("gpt-saved"));
        assert_eq!(list.read(cx).current_index(cx), 2);
    });
}

type Picks = std::rc::Rc<std::cell::RefCell<Vec<(&'static str, Option<String>)>>>;

/// A rendered picker over `project`, with `models` as `kind`'s live catalog.
fn rendered_picker(
    cx: &mut gpui::TestAppContext,
    kind: Kind,
    project: &str,
    models: &[&str],
) -> (gpui::WindowHandle<ModelList>, Picks, Subscription) {
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| ModelList::new_for(kind, None, project, Default::default(), cx))
        })
        .unwrap()
    });
    let picks = Picks::default();
    let output = picks.clone();
    let events = window
        .update(cx, |list, _, cx| {
            list.catalogs = vec![(
                kind,
                models
                    .iter()
                    .map(|id| Model {
                        id: (*id).into(),
                        name: (*id).into(),
                        alias_target: None,
                        efforts: Vec::new(),
                        default_effort: None,
                        fast: false,
                        thinking: false,
                        default: false,
                        tiers: Vec::new(),
                        controls: Vec::new(),
                    })
                    .collect(),
            )];
            list.remote_providers = list.remote_project.is_some().then(|| vec![kind]);
            cx.notify();
            cx.subscribe(&cx.entity(), move |_, _, picked: &ModelPicked, _| {
                output.borrow_mut().push((picked.0.id(), picked.1.clone()))
            })
        })
        .unwrap();
    (window, picks, events)
}
/// Debug selectors are `'static`; leaking a few test strings is fine.
fn star(ix: usize) -> &'static str {
    Box::leak(format!("model-star-{ix}").into_boxed_str())
}
fn click_star(visual: &mut gpui::VisualTestContext, ix: usize) {
    visual.refresh().unwrap();
    let bounds = visual
        .debug_bounds(star(ix))
        .unwrap_or_else(|| panic!("{} is rendered", star(ix)));
    visual.simulate_click(bounds.center(), Default::default());
    visual.refresh().unwrap();
}
fn favorites(cx: &gpui::App) -> Vec<(String, Option<String>)> {
    cx.global::<crate::ui::preferences::Preferences>()
        .favorite_models
        .iter()
        .map(|choice| (choice.provider.clone(), choice.model.clone()))
        .collect()
}

#[test]
fn favorite_star_names_say_what_the_click_does() {
    assert_eq!(favorite_label(false, "Fast"), "Add Fast to favorites");
    assert_eq!(favorite_label(true, "Fast"), "Remove Fast from favorites");
}

#[gpui::test]
fn star_toggles_a_favorite_without_picking_the_row(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| cx.set_global(crate::ui::preferences::Preferences::default()));
    let (window, picks, _events) = rendered_picker(cx, Kind::Codex, "/work", &["example-model"]);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    // Default (0) and the catalog's model (1) both get a star.
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("model-star-0").is_some());
    click_star(visual, 1);
    visual.update(|_, cx| {
        assert_eq!(
            favorites(cx),
            vec![("codex".into(), Some("example-model".into()))]
        )
    });
    assert!(picks.borrow().is_empty(), "the star never picks the row");
    window
        .update(visual, |list, _, cx| {
            assert_eq!(list.current, None);
            assert!(list.favorite(Kind::Codex, Some("example-model"), cx));
            assert!(
                !cx.global::<crate::ui::preferences::Preferences>()
                    .recent_models
                    .iter()
                    .any(|c| c.model.is_some())
            );
            // The Favorites tab now lists it.
            list.tab = ModelTab::Favorites;
            let entries = list.entries(cx);
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].id.as_deref(), Some("example-model"));
            assert_eq!(entries[0].kind, Kind::Codex);
            cx.notify();
        })
        .unwrap();
    // Un-starring from the Favorites tab removes it again.
    click_star(visual, 0);
    visual.update(|_, cx| assert!(favorites(cx).is_empty()));
    assert!(picks.borrow().is_empty());
    window
        .update(visual, |list, _, cx| assert!(list.entries(cx).is_empty()))
        .unwrap();
}

#[gpui::test]
fn picking_the_option_still_picks_and_leaves_favorites_alone(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| cx.set_global(crate::ui::preferences::Preferences::default()));
    let (window, picks, _events) = rendered_picker(cx, Kind::Codex, "/work", &["example-model"]);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.refresh().unwrap();
    // The option sits left of its star, so aim at the row's left edge.
    let star = visual.debug_bounds("model-star-1").unwrap();
    visual.simulate_click(
        gpui::point(star.left() - gpui::px(80.), star.center().y),
        Default::default(),
    );
    assert_eq!(
        *picks.borrow(),
        vec![("codex", Some("example-model".to_owned()))]
    );
    visual.update(|_, cx| assert!(favorites(cx).is_empty()));
}

#[gpui::test]
fn every_favorite_row_has_a_star(cx: &mut gpui::TestAppContext) {
    let prefs = crate::ui::preferences::Preferences {
        favorite_models: vec![
            crate::ui::preferences::ModelChoice {
                provider: "codex".into(),
                model: Some("example-model".into()),
            },
            crate::ui::preferences::ModelChoice {
                provider: "claude".into(),
                model: None,
            },
        ],
        ..Default::default()
    };
    cx.update(|cx| cx.set_global(prefs));
    let (window, _picks, _events) = rendered_picker(cx, Kind::Codex, "/work", &[]);
    window
        .update(cx, |list, _, cx| {
            list.tab = ModelTab::Favorites;
            assert_eq!(list.entries(cx).len(), 2);
            cx.notify();
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.refresh().unwrap();
    assert!(visual.debug_bounds(star(0)).is_some());
    assert!(visual.debug_bounds(star(1)).is_some());
    assert!(visual.debug_bounds(star(2)).is_none());
}

#[gpui::test]
fn remote_prefixed_model_is_favorited_and_picked_by_its_full_id(cx: &mut gpui::TestAppContext) {
    const REMOTE: &str = "hermes:example-vendor:example-model";
    cx.update(|cx| cx.set_global(crate::ui::preferences::Preferences::default()));
    let (window, picks, _events) =
        rendered_picker(cx, Kind::Hermes, "remote://env/home/code", &[REMOTE]);
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    // Default (0), then the host's model (1).
    click_star(visual, 1);
    visual.update(|_, cx| assert_eq!(favorites(cx), vec![("hermes".into(), Some(REMOTE.into()))]));
    window
        .update(visual, |list, _, cx| {
            list.tab = ModelTab::Favorites;
            let entries = list.entries(cx);
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].kind, Kind::Hermes);
            assert_eq!(entries[0].id.as_deref(), Some(REMOTE));
            assert!(entries[0].enabled);
            list.pick(0, cx);
            cx.notify();
        })
        .unwrap();
    assert_eq!(*picks.borrow(), vec![("hermes", Some(REMOTE.to_owned()))]);
    // Toggling again from the Favorites row removes it.
    click_star(visual, 0);
    visual.update(|_, cx| assert!(favorites(cx).is_empty()));
    window
        .update(visual, |list, _, cx| {
            assert!(list.entries(cx).is_empty());
            assert!(!list.favorite(Kind::Hermes, Some(REMOTE), cx));
        })
        .unwrap();
}
