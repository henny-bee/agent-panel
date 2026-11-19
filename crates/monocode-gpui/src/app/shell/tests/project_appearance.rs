use super::*;

#[gpui::test]
fn header_menu_switches_projects_from_its_picker_flyout(cx: &mut TestAppContext) {
    let (window, _, _) = window(cx);
    cx.update(|cx| {
        input::bind(cx);
        crate::ui::popover::bind(cx);
    });
    let before = window
        .update(cx, |shell, w, cx| {
            let anchor = Anchor::Slot(shell.project_anchor.clone());
            shell.open_menu(MenuTarget::Header, anchor, w, cx);
            shell.workspace.project.clone()
        })
        .unwrap();
    cx.run_until_parked();
    // Switch worktree is first; skip the separator to Switch project, open
    // its picker (which focuses the search field), and take the next project.
    cx.simulate_keystrokes(window.into(), "down right down enter");
    cx.run_until_parked();
    window
        .update(cx, |shell, _, _| {
            assert!(shell.menu.is_none());
            assert_ne!(shell.workspace.project, before);
        })
        .unwrap();
}

#[gpui::test]
fn project_appearance_menu_edits_and_background_dialog_persist_per_project(
    cx: &mut TestAppContext,
) {
    use monocode_core::project::appearance::{Background, COLORS, Effect, Scope};
    let (window, _, active) = window(cx);
    cx.update(|cx| {
        input::bind(cx);
        crate::ui::popover::bind(cx);
        cx.set_reduce_motion(true);
    });
    window
        .update(cx, |shell, w, cx| {
            let anchor = Anchor::Slot(shell.project_anchor.clone());
            shell.open_menu(MenuTarget::Header, anchor, w, cx);
        })
        .unwrap();
    cx.run_until_parked();
    // The settings flyout opens beside the header menu and focuses its name.
    cx.simulate_keystrokes(window.into(), "down down right");
    cx.run_until_parked();
    cx.simulate_input(window.into(), "Small");
    cx.simulate_keystrokes(window.into(), "space");
    cx.simulate_input(window.into(), "Studio");
    cx.run_until_parked();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    let flyout = window
        .update(visual, |shell, _, cx| {
            shell
                .menu
                .as_ref()
                .unwrap()
                .menu
                .read(cx)
                .submenu_region()
                .get()
                .unwrap()
        })
        .unwrap();
    let controls = visual.debug_bounds("project-appearance-controls").unwrap();
    let name = visual.debug_bounds("project-appearance-name").unwrap();
    let logo = visual.debug_bounds("choose-project-logo").unwrap();
    // TabGroupMenu's p-2 belongs to the whole card, including its actions.
    for inset in [
        controls.left() - flyout.left(),
        flyout.right() - controls.right(),
        name.top() - flyout.top(),
    ] {
        assert_eq!(inset, px(9.), "8px of padding inside the card's 1px frame");
    }
    assert_eq!(name.left(), controls.left());
    assert_eq!(logo.left() - controls.left(), px(2.));
    assert_eq!(logo.top() - name.bottom(), px(8.));
    assert_eq!(logo.size, gpui::size(px(36.), px(36.)));
    let background = visual.debug_bounds("menu-action-background").unwrap();
    assert_eq!(background.left() - flyout.left(), px(9.));
    assert_eq!(flyout.right() - background.right(), px(9.));
    let color = visual
        .debug_bounds("project-color-4")
        .expect("appearance controls in the flyout");
    visual.simulate_click(color.center(), Default::default());
    visual.run_until_parked();
    let mascot = visual.debug_bounds("project-mascot-3").unwrap();
    visual.simulate_click(mascot.center(), Default::default());
    visual.run_until_parked();
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    window
        .update(visual, |shell, w, cx| {
            assert!(shell.menu.is_none());
            assert_eq!(shell.project_label("/work/monocode"), "Small Studio");
            let appearance = shell
                .workspace
                .project_appearance
                .get_mut("/work/monocode")
                .unwrap();
            assert_eq!(appearance.color, Some(COLORS[4]));
            assert_eq!(appearance.mascot.as_deref(), Some("cat"));
            appearance.background = Some(Background {
                path: concat!(env!("CARGO_MANIFEST_DIR"), "/assets/monocode.png").into(),
                ..Default::default()
            });
            assert!(
                !shell
                    .workspace
                    .project_appearance
                    .contains_key("/work/another-project")
            );
            shell.open_menu(
                MenuTarget::Project {
                    path: "/work/monocode".into(),
                    rail: false,
                },
                Anchor::Point(gpui::point(px(300.), px(60.))),
                w,
                cx,
            );
        })
        .unwrap();
    visual.run_until_parked();
    let card = window
        .update(visual, |shell, _, cx| {
            let menu = shell.menu.as_ref().unwrap().menu.read(cx);
            menu.surface_region().get().unwrap()
        })
        .unwrap();
    let background_row = visual.debug_bounds("menu-action-background").unwrap();
    let last_row = visual.debug_bounds("menu-action-delete").unwrap();
    assert_eq!(background_row.left() - card.left(), px(9.));
    assert_eq!(card.right() - background_row.right(), px(9.));
    assert_eq!(background_row.size.height, px(32.));
    assert_eq!(card.bottom() - last_row.bottom(), px(9.));
    visual.simulate_click(background_row.center(), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("project-background-dialog").is_some());
    let dialog = visual.debug_bounds("project-background-dialog").unwrap();
    let header = visual.debug_bounds("background-dialog-header").unwrap();
    let preview = visual.debug_bounds("background-dialog-preview").unwrap();
    let change = visual
        .debug_bounds("project-settings-Change image")
        .unwrap();
    let effect = visual.debug_bounds("background-effect-row").unwrap();
    let blur_row = visual.debug_bounds("background-blur-row").unwrap();
    let scope = visual.debug_bounds("background-scope-row").unwrap();
    let scope_control = visual.debug_bounds("background-scope-control").unwrap();
    let empty_row = visual.debug_bounds("background-empty-row").unwrap();
    let session_row = visual.debug_bounds("background-session-row").unwrap();
    let remove = visual
        .debug_bounds("project-settings-Remove background image")
        .unwrap();
    // Modal.tsx + ProjectBackgroundDialog.tsx: match the React box metrics.
    assert_eq!(dialog.size.width, px(418.));
    assert_eq!(preview.size, gpui::size(px(386.), px(162.)));
    assert_eq!(preview.left() - dialog.left(), px(16.));
    assert_eq!(preview.top() - header.bottom(), px(16.));
    assert_eq!(change.top() - preview.bottom(), px(8.));
    assert_eq!(change.size.height, px(32.));
    assert_eq!(effect.size.height, px(57.5));
    assert_eq!(scope_control.size, gpui::size(px(176.), px(32.)));
    for (before, after) in [
        (effect, blur_row),
        (blur_row, scope),
        (scope, empty_row),
        (empty_row, session_row),
        (session_row, remove),
    ] {
        assert_eq!(after.top() - before.bottom(), px(20.));
    }
    assert_eq!(dialog.bottom() - remove.bottom(), px(16.));
    window
        .update(visual, |_, w, _| {
            assert_eq!(
                w.painted_quads()
                    .iter()
                    .filter(|q| q.background == content(0.15).into()
                        && q.bounds.size.width.0 == 172. * w.scale_factor()
                        && q.bounds.size.height.0 == 4. * w.scale_factor())
                    .count(),
                3
            );
        })
        .unwrap();
    let effect_trigger = visual.debug_bounds("project-background-effect").unwrap();
    assert_eq!(effect_trigger.size, gpui::size(px(144.), px(36.)));
    assert_eq!(effect_trigger.right(), preview.right());
    visual.simulate_click(effect_trigger.center(), Default::default());
    visual.run_until_parked();
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(visual.debug_bounds("project-background-dialog").is_some());
    assert!(visual.debug_bounds("project-settings-Haze").is_none());
    visual.simulate_click(effect_trigger.center(), Default::default());
    visual.run_until_parked();
    for selector in ["project-settings-Haze", "project-settings-Empty only"] {
        let bounds = visual.debug_bounds(selector).unwrap();
        visual.simulate_click(bounds.center(), Default::default());
        visual.run_until_parked();
    }
    let opacity = visual.debug_bounds("empty-background-opacity").unwrap();
    visual.simulate_click(opacity.center(), Default::default());
    visual.simulate_keystrokes("right");
    visual.run_until_parked();
    // Frosted glass: the middle of the track, then one step by keyboard.
    let blur = visual.debug_bounds("background-blur").unwrap();
    visual.simulate_click(blur.center(), Default::default());
    visual.simulate_keystrokes("right");
    visual.run_until_parked();
    window
        .update(visual, |shell, _, _| {
            let background = shell.workspace.project_appearance["/work/monocode"]
                .background
                .as_ref()
                .unwrap();
            assert_eq!(background.effect, Effect::GradientBlur);
            assert_eq!(background.scope, Scope::Empty);
            assert!((background.empty_opacity - 0.36).abs() < 0.001);
            assert_eq!(background.blur, 51);
            let restored = Workspace::restore(
                Some(&serde_json::to_string(&shell.workspace).unwrap()),
                &[(active, "/work/monocode".into())],
                "/work/monocode",
            );
            assert_eq!(
                restored.project_appearance,
                shell.workspace.project_appearance
            );
        })
        .unwrap();
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    window
        .update(visual, |shell, _, _| {
            assert!(shell.background_dialog.is_none())
        })
        .unwrap();
    assert!(visual.debug_bounds("project-chat-background").is_some());
    // Empty-only backgrounds disappear once a conversation has messages.
    window
        .update(visual, |shell, _, cx| {
            shell.sessions[&active].update(cx, |session, cx| {
                session.summary.has_messages = true;
                cx.notify();
            });
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("project-chat-background").is_none());
    visual.simulate_resize(gpui::size(px(820.), px(520.)));
    window
        .update(visual, |shell, w, cx| {
            shell.open_background_dialog("/work/monocode".into(), w, cx);
        })
        .unwrap();
    visual.run_until_parked();
    let compact = visual.debug_bounds("project-background-dialog").unwrap();
    let body = visual.debug_bounds("background-dialog-body").unwrap();
    assert!(compact.top() >= px(16.));
    assert!(compact.bottom() <= px(504.));
    assert!(body.bottom() <= compact.bottom());
}

#[gpui::test]
fn theme_colors_dialog_saves_a_project_theme_and_can_return_to_app_colors(cx: &mut TestAppContext) {
    let (window, _, _) = window(cx);
    cx.update(|cx| {
        input::bind(cx);
        crate::ui::popover::bind(cx);
        cx.set_reduce_motion(true);
    });
    let project = window
        .update(cx, |shell, w, cx| {
            let project = shell.workspace.project.clone();
            shell.open_project_theme(project.clone(), w, cx);
            project
        })
        .unwrap();
    cx.run_until_parked();
    let theme = |cx: &mut TestAppContext| {
        window
            .update(cx, |shell, _, _| shell.appearance(&project).theme)
            .unwrap()
    };
    assert_eq!(theme(cx), None, "opening the dialog alone changes nothing");
    // The modal centers itself once its height is measured.
    cx.refresh().unwrap();
    cx.run_until_parked();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    let dialog = visual.debug_bounds("project-theme-dialog").unwrap();
    assert!(dialog.top() >= px(16.));
    let harbor = visual.debug_bounds("project-theme-palette-harbor").unwrap();
    visual.simulate_click(harbor.center(), gpui::Modifiers::none());
    let red = visual.debug_bounds("project-theme-accent-4").unwrap();
    visual.simulate_click(red.center(), gpui::Modifiers::none());
    visual.run_until_parked();
    let saved = theme(visual).expect("the first pick gives the project its colors");
    assert_eq!(saved.workspace_theme, "harbor");
    assert_eq!(saved.accent_color.as_deref(), Some("#ef4444"));
    assert_eq!(crate::ui::theme::accent_color(), gpui::rgb(0xef4444).into());
    let reset = visual
        .debug_bounds("project-settings-Use app colors")
        .unwrap();
    visual.simulate_click(reset.center(), gpui::Modifiers::none());
    visual.run_until_parked();
    assert_eq!(theme(visual), None);
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    window
        .update(visual, |shell, _, _| {
            assert!(shell.background_dialog.is_none())
        })
        .unwrap();
}

#[gpui::test]
fn backgrounds_survive_tab_and_notes_switches_until_the_tab_closes(cx: &mut TestAppContext) {
    use monocode_core::project::appearance::{Background, Effect, ProjectAppearance};
    let (window, a, b) = window(cx);
    window
        .update(cx, |shell, _, cx| {
            for path in ["/work/another-project", "/work/monocode"] {
                shell.workspace.project_appearance.insert(
                    path.into(),
                    ProjectAppearance {
                        background: Some(Background {
                            path: concat!(env!("CARGO_MANIFEST_DIR"), "/assets/monocode.png")
                                .into(),
                            effect: Effect::Dither,
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                );
            }
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    cx.refresh().unwrap();
    cx.run_until_parked();
    let background_b = window
        .update(cx, |shell, _, _| shell.backgrounds[&b].clone())
        .unwrap();
    let image_b = background_b.read_with(cx, |background, _| background.prepared_image().unwrap());
    window
        .update(cx, |shell, w, cx| shell.open(a, w, cx))
        .unwrap();
    cx.run_until_parked();
    cx.refresh().unwrap();
    cx.run_until_parked();
    let background_a = window
        .update(cx, |shell, _, _| shell.backgrounds[&a].clone())
        .unwrap();
    let image_a = background_a.read_with(cx, |background, _| background.prepared_image().unwrap());
    assert!(
        Arc::ptr_eq(&image_a, &image_b),
        "projects using the same artwork share its texture"
    );
    for _ in 0..3 {
        window
            .update(cx, |shell, w, cx| {
                shell.open(b, w, cx);
                shell.create(Kind::Codex, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, w, cx| {
                let new = shell.workspace.active.unwrap();
                assert_ne!(new, b);
                let prepared = shell.backgrounds[&new].read(cx).prepared_image().unwrap();
                assert!(
                    Arc::ptr_eq(&prepared, &image_b),
                    "new tabs must reuse the already decoded artwork"
                );
                shell.close(new, w, cx);
            })
            .unwrap();
        cx.run_until_parked();
    }
    for id in [b, a, b, a, b] {
        window
            .update(cx, |shell, w, cx| shell.open(id, w, cx))
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |shell, _, cx| {
                for (id, background, image) in
                    [(a, &background_a, &image_a), (b, &background_b, &image_b)]
                {
                    assert_eq!(
                        shell.backgrounds[&id].entity_id(),
                        background.entity_id(),
                        "hidden tabs retain the original background view"
                    );
                    let prepared = shell.backgrounds[&id].read(cx).prepared_image().unwrap();
                    assert!(
                        Arc::ptr_eq(&prepared, image),
                        "switching tabs must not regenerate the texture"
                    );
                }
            })
            .unwrap();
    }
    window
        .update(cx, |shell, w, cx| shell.open_notes(w, cx))
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, w, cx| {
            assert_eq!(shell.backgrounds.len(), 2);
            shell.open(b, w, cx);
            shell.close(a, w, cx);
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |shell, _, cx| {
            assert!(
                !shell.backgrounds.contains_key(&a),
                "closing a tab releases its background"
            );
            assert_eq!(shell.backgrounds[&b].entity_id(), background_b.entity_id());
            assert!(Arc::ptr_eq(
                &shell.backgrounds[&b].read(cx).prepared_image().unwrap(),
                &image_b
            ));
        })
        .unwrap();
}
