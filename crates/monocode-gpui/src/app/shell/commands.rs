//! Actions, app commands and keyboard shortcuts.

pub(crate) mod actions;
pub(crate) mod app_commands;
pub(crate) mod shortcuts;
use super::*;

/// The workspace's keys, after TitleBar/App's `tabCommand` and the settings
/// keybinding table: tabs, history, sessions, projects and the sidebar.
pub(crate) fn bind_keys(cx: &mut App) {
    sessions::groups::bind(cx);
    sidebar::folders::bind(cx);
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    // Scoped to the shell, so the focused composer, menus and pickers outrank
    // these: GPUI ranks an unscoped binding above every context.
    let context = Some("Workspace");
    cx.bind_keys([
        KeyBinding::new(&format!("{modifier}-n"), NewCodex, context),
        KeyBinding::new(&format!("{modifier}-t"), NewCodex, context),
        KeyBinding::new(&format!("{modifier}-shift-n"), NewClaude, context),
        KeyBinding::new(&format!("{modifier}-o"), OpenProject, context),
        KeyBinding::new(&format!("{modifier}-p"), GoToFile, context),
        KeyBinding::new(&format!("{modifier}-shift-p"), OpenCommandPalette, context),
        KeyBinding::new(&format!("{modifier}-,"), OpenSettings, context),
        KeyBinding::new(&format!("{modifier}-w"), CloseTab, context),
        KeyBinding::new(&format!("{modifier}-shift-w"), CloseAllTabs, context),
        KeyBinding::new(
            &format!("{modifier}-shift-a"),
            ArchiveSession,
            Some("Session > (Composer || TranscriptSelection) && !CodeEditor && !Completion && !PopoverMenu && !Prompt"),
        ),
        KeyBinding::new(&format!("{modifier}-["), VisitBack, context),
        KeyBinding::new(&format!("{modifier}-]"), VisitForward, context),
        KeyBinding::new("escape", DismissMenu, context),
        KeyBinding::new(&format!("{modifier}-b"), ToggleSidebar, context),
        KeyBinding::new(&format!("{modifier}-k"), FocusSearch, context),
        KeyBinding::new(&format!("{modifier}-shift-f"), SearchFiles, context),
        KeyBinding::new(&format!("{modifier}-alt-t"), CloseOtherTabs, context),
        KeyBinding::new(&format!("{modifier}-shift-]"), NextTab, context),
        KeyBinding::new(&format!("{modifier}-}}"), NextTab, context),
        KeyBinding::new(&format!("{modifier}-shift-["), PreviousTab, context),
        KeyBinding::new(&format!("{modifier}-{{"), PreviousTab, context),
        KeyBinding::new(&format!("{modifier}-1"), ActivateTab(0), context),
        KeyBinding::new(&format!("{modifier}-2"), ActivateTab(1), context),
        KeyBinding::new(&format!("{modifier}-3"), ActivateTab(2), context),
        KeyBinding::new(&format!("{modifier}-4"), ActivateTab(3), context),
        KeyBinding::new(&format!("{modifier}-5"), ActivateTab(4), context),
        KeyBinding::new(&format!("{modifier}-6"), ActivateTab(5), context),
        KeyBinding::new(&format!("{modifier}-7"), ActivateTab(6), context),
        KeyBinding::new(&format!("{modifier}-8"), ActivateTab(7), context),
        KeyBinding::new(&format!("{modifier}-9"), ActivateTab(-1), context),
        KeyBinding::new(&format!("{modifier}-shift-up"), PreviousSession, context),
        KeyBinding::new(&format!("{modifier}-shift-down"), NextSession, context),
        KeyBinding::new(&format!("{modifier}-up"), PreviousSessionInTab, context),
        KeyBinding::new(&format!("{modifier}-down"), NextSessionInTab, context),
        KeyBinding::new(&format!("{modifier}-shift-left"), PreviousProject, context),
        KeyBinding::new(&format!("{modifier}-shift-right"), NextProject, context),
        KeyBinding::new(&format!("{modifier}-alt-shift-left"), MoveProjectBack, context),
        KeyBinding::new(&format!("{modifier}-alt-shift-right"), MoveProjectForward, context),
        KeyBinding::new(&format!("{modifier}-."), SwitchModel, context),
        KeyBinding::new(&format!("{modifier}-shift-b"), ToggleRightPanel, context),
        KeyBinding::new(&format!("{modifier}-d"), SplitRight, context),
        KeyBinding::new(&format!("{modifier}-j"), ToggleTerminal, context),
        KeyBinding::new(&format!("{modifier}-shift-j"), SplitTerminal, context),
        KeyBinding::new(&format!("{modifier}-`"), NewTerminal, context),
        // Switch between the original two-sidebar layout and the new layout.
        KeyBinding::new(&format!("{modifier}-shift-l"), ToggleLayout, context),
        // Switch the Focused layout between its rounded and boxy workspace.
        KeyBinding::new(&format!("{modifier}-alt-l"), ToggleWorkspaceStyle, context),
        KeyBinding::new(&format!("{modifier}-shift-d"), SplitDown, context),
        KeyBinding::new(&format!("{modifier}-="), ZoomIn, context),
        KeyBinding::new(&format!("{modifier}-+"), ZoomIn, context),
        KeyBinding::new(&format!("{modifier}--"), ZoomOut, context),
        KeyBinding::new(&format!("{modifier}-0"), ResetZoom, context),
        KeyBinding::new(&format!("{modifier}-shift-r"), Reload, context),
        KeyBinding::new(&format!("{modifier}-alt-left"), FocusPaneLeft, context),
        KeyBinding::new(&format!("{modifier}-alt-right"), FocusPaneRight, context),
        KeyBinding::new(&format!("{modifier}-alt-up"), FocusPaneUp, context),
        KeyBinding::new(&format!("{modifier}-alt-down"), FocusPaneDown, context),
        KeyBinding::new("ctrl-tab", NextTab, context),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, context),
        KeyBinding::new("tab", FocusNext, context),
        KeyBinding::new("shift-tab", FocusPrevious, context),
    ]);
}
