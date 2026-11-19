//! Sidebar.tsx's session actions: pins, renames, GitHub links, reminders,
//! folders and deletion, and the grouped list they shape.
mod rename;
mod rows;
use super::super::*;
use chrono::{Local, NaiveDateTime, TimeZone};
use gpui::EventEmitter;
use monocode_core::{
    session::groups::{
        Entry, Groups, build_list, format_reminder, navigation_ids, parse_github_work_item,
        reminder_at, reminder_presets,
    },
    store::Reminder,
};

actions!(
    session_groups,
    [CancelRename, RenameSidebarItem, DeleteSidebarSelection]
);

pub(crate) fn bind(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", CancelRename, Some("SessionRename")),
        KeyBinding::new("f2", RenameSidebarItem, Some("Workspace && !SessionRename")),
        KeyBinding::new(
            "backspace",
            DeleteSidebarSelection,
            Some("SessionSidebar && !Composer"),
        ),
        KeyBinding::new(
            "delete",
            DeleteSidebarSelection,
            Some("SessionSidebar && !Composer"),
        ),
    ]);
}

/// tabGroups.ts's TAB_GROUP_COLORS; 0 is the default wash.
const FOLDER_COLORS: [(f32, f32, f32); 9] = [
    (210., 0.08, 0.58),
    (211., 0.92, 0.62),
    (12., 0.80, 0.58),
    (45., 0.90, 0.55),
    (142., 0.55, 0.50),
    (330., 0.70, 0.62),
    (280., 0.55, 0.62),
    (175., 0.55, 0.48),
    (25., 0.85, 0.58),
];
fn folder_color(ix: usize) -> Option<gpui::Hsla> {
    FOLDER_COLORS
        .get(ix)
        .map(|&(h, s, l)| gpui::hsla(h / 360., s, l, 1.))
}
/// Sidebar.tsx's REMINDERS_COLOR, amber-500.
const REMINDERS_COLOR: u32 = 0xf59e0b;

fn local_ms(at: NaiveDateTime) -> Option<u64> {
    Local
        .from_local_datetime(&at)
        .earliest()
        .map(|at| at.timestamp_millis().max(0) as u64)
}
fn local_time(ms: u64) -> Option<NaiveDateTime> {
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .map(|at| at.with_timezone(&Local).naive_local())
}
pub(crate) fn local_reminder_label(ms: u64) -> String {
    local_time(ms).map(format_reminder).unwrap_or_default()
}
pub(crate) fn reminder_caption(ms: u64) -> String {
    let Some(at) = local_time(ms) else {
        return String::new();
    };
    let now = Local::now().naive_local();
    if at <= now {
        "Now".into()
    } else if at.date() == now.date() {
        at.format("%-H:%M").to_string()
    } else {
        at.format("%a").to_string()
    }
}

#[derive(Clone, PartialEq)]
pub(crate) enum ListDrop {
    Folder(String),
    Session(i64),
}
pub(crate) struct FolderDrag {
    id: String,
    start: Point<Pixels>,
    active: bool,
    over: Option<String>,
}

#[derive(Clone, PartialEq)]
pub(crate) enum RenameTarget {
    Session(i64),
    Folder(String),
}
pub(crate) struct Renaming {
    pub(crate) target: RenameTarget,
    input: Entity<input::Input>,
    _events: Vec<Subscription>,
}
pub(crate) struct LinkDialog {
    id: i64,
    input: Entity<input::Input>,
    error: Option<SharedString>,
    editing: bool,
    _events: Vec<Subscription>,
}

/// FolderColorSwatches: the folder menu's palette row.
pub(crate) struct FolderSwatches {
    focus: FocusHandle,
    selected: usize,
}
pub(crate) struct SwatchPicked(Option<usize>);
impl EventEmitter<SwatchPicked> for FolderSwatches {}
impl Render for FolderSwatches {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus)
            .px_2()
            .py_1p5()
            .flex()
            .items_center()
            .gap_1p5()
            .children((0..FOLDER_COLORS.len()).map(|ix| {
                let selected = ix == self.selected;
                div()
                    .id(("folder-swatch", ix))
                    .debug_selector(move || format!("folder-color-{ix}"))
                    .role(Role::Button)
                    .aria_label(if ix == 0 {
                        "Default color".to_string()
                    } else {
                        format!("Color {ix}")
                    })
                    .size(px(18.))
                    .rounded_full()
                    .border_2()
                    .border_color(if selected { content(0.9) } else { content(0.) })
                    .hover(|s| s.border_color(content(0.5)))
                    .child(
                        div()
                            .size_full()
                            .rounded_full()
                            .border_2()
                            .border_color(content(0.))
                            .bg(folder_color(ix).unwrap()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = ix;
                        cx.emit(SwatchPicked((ix > 0).then_some(ix)));
                        cx.notify();
                    }))
            }))
    }
}

impl Shell {
    pub(crate) fn rename_sidebar_item(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = if self.sidebar_focus.is_focused(window) {
            self.sidebar_target.clone()
        } else {
            self.session_hover
                .or(self.workspace.active)
                .map(RenameTarget::Session)
        };
        if let Some(target) = target {
            self.start_rename(target, window, cx);
        }
    }
    pub(crate) fn delete_sidebar_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<_> = self.selected_sessions.iter().copied().collect();
        for id in ids {
            self.delete_session(id, window, cx);
        }
    }
    pub(crate) fn sidebar_drop_at(
        &self,
        position: Point<Pixels>,
        source: i64,
        cx: &App,
    ) -> Option<ListDrop> {
        if !self.workspace.sidebar_visible || self.reminders.iter().any(|r| r.session_id == source)
        {
            return None;
        }
        let groups = self.project_groups();
        for folder in &groups.folders {
            if self
                .folder_bounds
                .get(&folder.id)
                .and_then(AnchorSlot::get)
                .is_some_and(|b| b.contains(&position))
            {
                return Some(ListDrop::Folder(folder.id.clone()));
            }
        }
        for id in self.sidebar_navigation_ids(cx) {
            if id == source || self.reminders.iter().any(|r| r.session_id == id) {
                continue;
            }
            if self
                .session_row_bounds
                .get(&id)
                .and_then(AnchorSlot::get)
                .is_some_and(|b| b.contains(&position))
            {
                return Some(match groups.folder_of(id) {
                    Some(folder) => ListDrop::Folder(folder.id.clone()),
                    None => ListDrop::Session(id),
                });
            }
        }
        None
    }
    pub(crate) fn apply_sidebar_drop(
        &mut self,
        source: i64,
        target: ListDrop,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            ListDrop::Folder(folder) => {
                self.update_groups(cx, |g| g.add_to_folder(&folder, source))
            }
            ListDrop::Session(target) if source != target => {
                let folder = format!("{:x}-{source}-{target}", monocode_core::sys::time::now_ms());
                self.update_groups(cx, |g| {
                    g.create_folder(&[source, target], folder.clone());
                });
                self.start_rename(RenameTarget::Folder(folder), window, cx);
            }
            _ => {}
        }
    }
    pub(crate) fn move_folder_drag(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(drag) = &mut self.folder_drag else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.folder_drag = None;
            return;
        }
        let delta = event.position - drag.start;
        if !drag.active && delta.x.as_f32().hypot(delta.y.as_f32()) < 5. {
            return;
        }
        drag.active = true;
        self.skip_folder_click = true;
        drag.over = self.folder_bounds.iter().find_map(|(id, slot)| {
            slot.get()
                .filter(|b| b.contains(&event.position))
                .map(|_| id.clone())
        });
        self.notify_sidebar(cx);
    }
    pub(crate) fn finish_folder_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.folder_drag.take() else {
            return;
        };
        if !drag.active {
            return;
        }
        if let Some(over) = drag.over {
            self.update_groups(cx, |g| {
                if let Some((from, to)) = g
                    .folders
                    .iter()
                    .position(|f| f.id == drag.id)
                    .zip(g.folders.iter().position(|f| f.id == over))
                {
                    let folder = g.folders.remove(from);
                    g.folders.insert(to, folder);
                }
            });
        }
        self.notify_sidebar(cx);
    }
    pub(crate) fn project_groups(&self) -> Groups {
        self.groups
            .get(&self.workspace.project)
            .cloned()
            .unwrap_or_default()
    }
    pub(crate) fn update_groups(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut Groups),
    ) {
        let project = self.workspace.project.clone();
        let groups = self.groups.entry(project.clone()).or_default();
        change(groups);
        let groups = groups.clone();
        self.store_write("save folders", cx, move |store| {
            store.save_groups(&project, &groups)
        });
        self.invalidate_chrome(cx);
        cx.notify();
    }
    pub(crate) fn store_write(
        &mut self,
        what: &'static str,
        cx: &mut Context<Self>,
        write: impl FnOnce(&mut monocode_core::store::Store) -> Result<(), String> + Send + 'static,
    ) {
        let store = self.store.clone();
        let previous = self.group_writes.take();
        self.group_writes = Some(cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let work = cx
                .background_executor()
                .spawn(async move { write(&mut *locked(&store)?) });
            if let Err(error) = work.await {
                let _ = this.update(cx, |this, cx| {
                    this.error = Some(format!("Could not {what}: {error}"));
                    cx.notify();
                });
            }
        }));
    }

    /// The project's sidebar entries, reminders first.
    pub(crate) fn session_entries(&self, ids: &[i64], cx: &App) -> Vec<Entry> {
        let reminders: Vec<_> = self
            .reminders
            .iter()
            .map(|r| (r.session_id, r.due_at))
            .collect();
        build_list(
            ids,
            |id| self.session_metadata(id, cx).summary.pinned,
            &self.project_groups(),
            &reminders,
        )
    }
    /// The sessions in the order the sidebar shows them, for stepping.
    pub(crate) fn sidebar_navigation_ids(&self, cx: &App) -> Vec<i64> {
        let ids = self.sidebar_session_ids(cx);
        let groups = self.project_groups();
        let entries: Vec<_> = self
            .session_entries(&ids, cx)
            .into_iter()
            .filter(|entry| {
                !self.query.is_empty()
                    || match entry {
                        Entry::Folder { id, .. } => groups.folder(id).is_none_or(|f| !f.collapsed),
                        Entry::Pinned { .. } if self.workspace.classic_layout => {
                            !groups.pinned_collapsed
                        }
                        _ => true,
                    }
            })
            .collect();
        navigation_ids(&entries, !self.query.is_empty())
    }

    fn session_menu_ids(&self, id: i64, cx: &App) -> Vec<i64> {
        if !self.selected_sessions.contains(&id) {
            return vec![id];
        }
        self.sidebar_session_ids(cx)
            .into_iter()
            .filter(|id| self.selected_sessions.contains(id))
            .collect()
    }
    pub(crate) fn clear_context_selection(&mut self) {
        if std::mem::take(&mut self.context_selection) {
            self.selected_sessions.clear();
        }
    }
    pub(crate) fn select_sidebar_session(
        &mut self,
        id: i64,
        modifiers: gpui::Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.context_selection = false;
        self.close_menu(cx);
        self.sidebar_target = Some(RenameTarget::Session(id));
        let additive = modifiers.control || modifiers.platform;
        if modifiers.shift {
            let visible = self.sidebar_navigation_ids(cx);
            let anchor = self
                .selection_anchor
                .filter(|id| visible.contains(id))
                .or(self.workspace.active)
                .unwrap_or(id);
            let range = visible
                .iter()
                .position(|s| *s == anchor)
                .zip(visible.iter().position(|s| *s == id));
            if !additive {
                self.selected_sessions.clear();
            }
            if let Some((a, b)) = range {
                self.selected_sessions.extend(&visible[a.min(b)..=a.max(b)]);
                self.selection_anchor = Some(anchor);
            } else {
                self.selected_sessions.insert(id);
                self.selection_anchor = Some(id);
            }
        } else if additive {
            self.selection_anchor = Some(id);
            if !self.selected_sessions.insert(id) {
                self.selected_sessions.remove(&id);
            }
        } else {
            self.selected_sessions.clear();
            self.selection_anchor = Some(id);
            self.open(id, window, cx);
        }
        if modifiers.shift || additive {
            window.focus(&self.sidebar_focus, cx);
        }
        self.invalidate_chrome(cx);
        cx.notify();
    }
    pub(crate) fn session_menu_items(&self, id: i64, cx: &App) -> Vec<MenuItem> {
        let summary = self.session_metadata(id, cx).summary;
        let ids = self.session_menu_ids(id, cx);
        let multiple = ids.len() > 1;
        let groups = self.project_groups();
        let all_pinned = ids
            .iter()
            .all(|id| self.session_metadata(*id, cx).summary.pinned);
        let all_archived = ids
            .iter()
            .all(|id| self.session_metadata(*id, cx).summary.archived);
        let mut items = Vec::new();
        if self.reminders.iter().any(|r| ids.contains(&r.session_id)) {
            items.push(MenuAction::new("reminder:cancel", "Cancel reminder").into());
            items.push(MenuItem::Separator);
        }
        items.push(MenuAction::new("pin", if all_pinned { "Unpin" } else { "Pin" }).into());
        if !multiple {
            items.extend([
                MenuAction::new("rename", "Rename").shortcut("F2").into(),
                MenuAction::new("copy-session-id", "Copy session ID")
                    .submenu(vec![
                        MenuAction::new("copy-harness-session-id", "Harness session ID")
                            .disabled(summary.provider_id.is_none()),
                        MenuAction::new("copy-monocode-session-id", "MCode session ID"),
                    ])
                    .into(),
                MenuAction::new(
                    "link-work-item",
                    if summary.work_item.is_some() {
                        "Edit GitHub issue or PR link…"
                    } else {
                        "Link GitHub issue or PR…"
                    },
                )
                .into(),
            ]);
        }
        items.extend([
            MenuAction::new("reminder", "Remind me")
                .submenu(
                    reminder_presets(Local::now().naive_local())
                        .into_iter()
                        .map(|(id, label, enabled)| MenuAction::new(id, label).disabled(!enabled))
                        .collect(),
                )
                .into(),
            MenuItem::Separator,
            MenuAction::new("folder-new", "New folder").into(),
        ]);
        if !groups.folders.is_empty() {
            items.push(MenuItem::Separator);
            items.extend(groups.folders.iter().map(|folder| {
                MenuAction::new(
                    format!("folder-add:{}", folder.id),
                    format!("Add to {}", folder.name),
                )
                .checked(ids.iter().all(|id| folder.session_ids.contains(id)))
                .into()
            }));
        }
        if ids.iter().any(|id| groups.folder_of(*id).is_some()) {
            items.push(
                MenuAction::new(
                    "folder-remove",
                    if multiple {
                        "Remove from folders"
                    } else {
                        "Remove from folder"
                    },
                )
                .into(),
            );
        }
        items.extend([
            MenuItem::Separator,
            MenuAction::new(
                "archive",
                if all_archived { "Unarchive" } else { "Archive" },
            )
            .into(),
            MenuAction::new("delete", "Delete")
                .shortcut("⌫")
                .danger()
                .into(),
        ]);
        items
    }
    pub(crate) fn run_session_menu(
        &mut self,
        id: i64,
        action: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.session_summaries.contains_key(&id) {
            return;
        }
        let summary = self.session_metadata(id, cx).summary.clone();
        let ids = self.session_menu_ids(id, cx);
        match action {
            "copy-harness-session-id" | "copy-monocode-session-id" => {
                let text = if action == "copy-harness-session-id" {
                    summary.provider_id
                } else {
                    locked(&self.store)
                        .and_then(|store| store.persisted_session_id(summary.id))
                        .ok()
                };
                if let Some(text) = text {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
            "pin" => {
                let pinned = !ids
                    .iter()
                    .all(|id| self.session_metadata(*id, cx).summary.pinned);
                for id in &ids {
                    if let Some(summary) = self.session_summaries.get_mut(id) {
                        summary.pinned = pinned;
                    }
                    if let Some(view) = self.sessions.get(id) {
                        view.update(cx, |s, _| s.summary.pinned = pinned);
                    }
                }
                self.store_write("pin the session", cx, move |s| s.set_pinned(&ids, pinned));
                self.invalidate_chrome(cx);
            }
            "rename" => self.start_rename(RenameTarget::Session(id), window, cx),
            "link-work-item" => self.open_link_dialog(id, window, cx),
            "reminder:cancel" => {
                let occurrences: Vec<_> = self
                    .reminders
                    .iter()
                    .filter(|reminder| ids.contains(&reminder.session_id))
                    .map(|reminder| (reminder.session_id, reminder.due_at))
                    .collect();
                for (id, due_at) in occurrences {
                    self.clear_notification_reminder(id, due_at, cx);
                }
                self.invalidate_chrome(cx);
            }
            preset if preset.starts_with("reminder:") => {
                let Some(due_at) =
                    reminder_at(preset, Local::now().naive_local()).and_then(local_ms)
                else {
                    return;
                };
                self.reminders.retain(|r| !ids.contains(&r.session_id));
                self.reminders.extend(ids.iter().map(|id| Reminder {
                    session_id: *id,
                    due_at,
                    fired_at: None,
                }));
                let project = self.workspace.project.clone();
                if let Some(groups) = self.groups.get_mut(&project) {
                    groups.reminders_collapsed = false;
                }
                self.update_groups(cx, |_| {});
                self.store_write("set the reminder", cx, move |s| {
                    s.set_reminders(&ids, due_at)
                });
            }
            "folder-new" => {
                let folder = format!(
                    "{:x}",
                    monocode_core::sys::time::now_ms() ^ ((id as u64) << 40)
                );
                self.update_groups(cx, |groups| {
                    groups.create_folder(&ids, folder.clone());
                });
                self.start_rename(RenameTarget::Folder(folder), window, cx);
            }
            "folder-remove" => self.update_groups(cx, |groups| groups.remove_sessions(&ids)),
            add if add.starts_with("folder-add:") => {
                let folder = add["folder-add:".len()..].to_string();
                self.update_groups(cx, |groups| {
                    for id in &ids {
                        groups.add_to_folder(&folder, *id);
                    }
                });
            }
            "archive" => {
                let archived = !ids
                    .iter()
                    .all(|id| self.session_metadata(*id, cx).summary.archived);
                for id in ids {
                    if self.session_metadata(id, cx).summary.archived != archived {
                        self.toggle_archive(id, cx);
                    }
                }
            }
            "delete" => {
                for id in ids {
                    self.delete_session(id, window, cx);
                }
            }
            _ => {}
        }
    }
    pub(crate) fn folder_menu_items(
        &self,
        folder: &str,
        events: &mut Vec<Subscription>,
        cx: &mut Context<Self>,
    ) -> Vec<MenuItem> {
        let selected = self
            .project_groups()
            .folder(folder)
            .and_then(|f| f.color_index)
            .unwrap_or(0);
        let swatches = cx.new(|cx| FolderSwatches {
            focus: cx.focus_handle(),
            selected,
        });
        let id = folder.to_string();
        events.push(
            cx.subscribe(&swatches, move |this, _, SwatchPicked(color), cx| {
                let color = *color;
                this.update_groups(cx, |groups| groups.set_color(&id, color));
            }),
        );
        let focus = swatches.read(cx).focus.clone();
        vec![
            MenuItem::Custom(swatches.into(), focus),
            MenuAction::new("rename", "Rename").shortcut("F2").into(),
            MenuItem::Separator,
            MenuAction::new("ungroup", "Ungroup").into(),
        ]
    }
    pub(crate) fn run_folder_menu(
        &mut self,
        folder: String,
        action: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            "rename" => self.start_rename(RenameTarget::Folder(folder), window, cx),
            "ungroup" => self.update_groups(cx, |groups| groups.dissolve(&folder)),
            _ => {}
        }
    }

    /// Opening a session whose reminder is due dismisses it.
    pub(crate) fn dismiss_due_reminder(&mut self, id: i64, cx: &mut Context<Self>) {
        let now = monocode_core::sys::time::now_ms();
        let Some(due) = self
            .reminders
            .iter()
            .find(|r| r.session_id == id && r.due_at <= now)
            .map(|r| r.due_at)
        else {
            return;
        };
        self.clear_notification_reminder(id, due, cx);
    }

    pub(crate) fn poll_reminders(&mut self, cx: &mut Context<Self>) {
        if self.notification_state.reminder_error.is_some() {
            return;
        }
        let now = monocode_core::sys::time::now_ms();
        let due: Vec<_> = self
            .reminders
            .iter_mut()
            .filter_map(|reminder| {
                if reminder.due_at > now || reminder.fired_at.is_some() {
                    return None;
                }
                reminder.fired_at = Some(now);
                Some((reminder.session_id, reminder.due_at))
            })
            .collect();
        if due.is_empty() {
            return;
        }
        let store = self.store.clone();
        let previous = self.group_writes.take();
        self.group_writes = Some(cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let work = cx
                .background_executor()
                .spawn(async move { locked(&store)?.take_due_reminders(now) });
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(claimed) => {
                        this.notification_state.reminder_error = None;
                        for reminder in claimed {
                            // A cancel/reschedule while the store operation was
                            // pending must never deliver an obsolete occurrence.
                            if this.reminders.iter().any(|current| {
                                current.session_id == reminder.session_id
                                    && current.due_at == reminder.due_at
                            }) {
                                this.deliver_reminder_notice(
                                    reminder.session_id,
                                    reminder.due_at,
                                    cx,
                                );
                            }
                        }
                    }
                    Err(error) => {
                        // Re-arm only the occurrences claimed by this request;
                        // a newer schedule has its own independent fired state.
                        for reminder in &mut this.reminders {
                            if due.contains(&(reminder.session_id, reminder.due_at))
                                && reminder.fired_at == Some(now)
                            {
                                reminder.fired_at = None;
                            }
                        }
                        this.notification_state.reminder_error =
                            Some(format!("Could not deliver reminders: {error}"));
                    }
                }
                this.invalidate_chrome(cx);
                cx.notify();
            });
        }));
        self.invalidate_chrome(cx);
    }

    pub(crate) fn delete_session(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace.review_owners.iter().any(|(pane, owner)| {
            *owner == id
                && (self.workspace.tab_of(*pane).is_some()
                    || self
                        .reviews
                        .get(pane)
                        .is_some_and(|r| r.read(cx).unsaved_files(cx)))
        }) {
            self.error = Some("Close this session's detached file panes before deleting it".into());
            cx.notify();
            return;
        }
        if self
            .workspace
            .terminal_panes
            .iter()
            .any(|(pane, t)| t.owner == id && self.workspace.tab_of(*pane).is_some())
        {
            self.error = Some("Close this session's terminal panes before deleting it".into());
            cx.notify();
            return;
        }
        if !self.session_summaries.contains_key(&id) || !self.deleting.insert(id) {
            return;
        }
        self.close_preview(cx);
        if let Some(view) = self.sessions.get(&id) {
            view.update(cx, |session, cx| session.stop_if_busy(cx));
        }
        if self.workspace.tab_of(id).is_some() {
            self.close_pane(id, window, cx);
        }
        if self.workspace.tab_of(id).is_some() {
            self.deleting.remove(&id);
            self.error = Some("Could not delete a session that is still open".into());
            cx.notify();
            return;
        }
        self.invalidate_chrome(cx);
        let store = self.store.clone();
        let previous = self.group_writes.take();
        self.group_writes = Some(cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let work = cx.background_executor().spawn(async move {
                locked(&store)?.delete(id)?;
                // Drop the session's recorded turns and their Git refs.
                if let Some(checkpoints) = monocode_core::vcs::checkpoint::store() {
                    let _ = checkpoints.exclusive(|c| c.forget(&id.to_string()));
                }
                Ok::<_, String>(())
            });
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.deleting.remove(&id);
                match result {
                    Ok(()) => this.forget_session(id, cx),
                    Err(error) => {
                        this.error = Some(format!("Could not delete the session: {error}"))
                    }
                }
                this.invalidate_chrome(cx);
                this.sync_settings(cx);
                cx.notify();
            });
        }));
    }
    fn forget_session(&mut self, id: i64, cx: &mut Context<Self>) {
        // A tab opened on it while the store was deleting keeps it alive.
        if self.workspace.tab_of(id).is_some() {
            return;
        }
        if self
            .session_preview
            .as_ref()
            .is_some_and(|preview| preview.id == id)
        {
            self.session_preview = None;
            self.preview_close = None;
        }
        self.sessions.remove(&id);
        self.session_summaries.remove(&id);
        self.draft_sessions.remove(&id);
        self.history_recency.remove(&id);
        self.session_marks.remove(&id);
        self.tab_marks.remove(&id);
        self.order.retain(|s| *s != id);
        self.busy_sessions.remove(&id);
        self.unseen_finished.remove(&id);
        self.selected_sessions.remove(&id);
        self.reminders.retain(|r| r.session_id != id);
        self.workspace.project_focus.retain(|_, s| *s != id);
        self.history
            .prune(&self.workspace.open_sessions(), self.workspace.active);
        if self.session_hover == Some(id) {
            self.session_hover = None;
        }
        if self
            .renaming
            .as_ref()
            .is_some_and(|r| r.target == RenameTarget::Session(id))
        {
            self.renaming = None;
        }
        let projects: Vec<String> = self
            .groups
            .iter()
            .filter(|(_, g)| g.folder_of(id).is_some())
            .map(|(p, _)| p.clone())
            .collect();
        for project in projects {
            let groups = self.groups.get_mut(&project).unwrap();
            groups.remove_sessions(&[id]);
            let groups = groups.clone();
            self.store_write("save folders", cx, move |s| {
                s.save_groups(&project, &groups)
            });
        }
        self.persist(cx);
    }
}

#[cfg(test)]
mod tests;
