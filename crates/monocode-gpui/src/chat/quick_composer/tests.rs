use super::*;
use gpui::TestAppContext;

fn open(cx: &mut TestAppContext) -> WindowHandle<QuickComposer> {
    cx.update(|cx| cx.set_global(crate::ui::preferences::Preferences::default()));
    let window = cx.add_window(|_, cx| {
        QuickComposer::new(
            vec!["/work/app".into(), "/work/site".into()],
            HashMap::new(),
            Launch::choice(
                "/work/app".into(),
                Kind::Codex,
                None,
                AccessMode::Supervised,
            ),
            cx,
        )
    });
    cx.run_until_parked();
    window
}

#[gpui::test]
fn every_picker_opens_below_the_toolbar_and_escape_closes_it(cx: &mut TestAppContext) {
    let window = open(cx);
    for picker in [
        Picker::Project,
        Picker::Model,
        Picker::Permissions,
        Picker::Attachments,
        Picker::Workspace,
        Picker::Branch,
    ] {
        window
            .update(cx, |view, window, cx| {
                view.open(picker, window, cx);
                assert!(view.picker == Some(picker));
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |view, window, cx| {
                // Opening the same picker again closes it, as a toggle.
                view.open(picker, window, cx);
                assert!(view.picker.is_none());
            })
            .unwrap();
    }
}

#[gpui::test]
fn a_leading_slash_lists_modes_and_picking_one_keeps_it_in_the_prompt(cx: &mut TestAppContext) {
    let window = open(cx);
    window
        .update(cx, |view, _, cx| {
            view.input.update(cx, |i, cx| i.set_text("/pl".into(), cx));
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, _, cx| {
            assert!(view.picker == Some(Picker::Commands));
            assert_eq!(view.command_options(cx).len(), 1);
            view.choose_command(0, cx);
            assert_eq!(view.input.read(cx).content.as_ref(), "/plan ");
            assert!(view.picker.is_none());
            // A bare mode is not a message yet.
            assert!(!view.can_submit(cx));
        })
        .unwrap();
}

#[gpui::test]
fn launch_carries_the_workspace_choice_and_clears_the_draft(cx: &mut TestAppContext) {
    let window = open(cx);
    let launched = std::rc::Rc::new(std::cell::RefCell::new(None));
    window
        .update(cx, |view, window, cx| {
            let sink = launched.clone();
            cx.subscribe_in(&cx.entity(), window, move |_, _, launch: &Launch, _, _| {
                *sink.borrow_mut() = Some(launch.clone());
            })
            .detach();
            view.workspace.new_worktree = true;
            view.workspace.base = Some("main".into());
            view.access = AccessMode::FullAccess;
            view.input
                .update(cx, |i, cx| i.set_text("/draft fix the build".into(), cx));
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(cx, |view, _, cx| {
            assert_eq!(view.message(cx), "fix the build");
            view.launch(true, cx);
            assert!(view.input.read(cx).content.is_empty());
        })
        .unwrap();
    let launch = launched.borrow_mut().take().expect("launched");
    assert_eq!(launch.prompt, "/draft fix the build");
    assert!(launch.new_worktree);
    assert_eq!(launch.worktree_base.as_deref(), Some("main"));
    assert_eq!(launch.worktree_cwd, None);
    assert!(launch.foreground);
    assert_eq!(launch.access, AccessMode::FullAccess);
}
