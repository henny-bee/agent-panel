//! Exact project identity and timestamp semantics of notificationPreferences.ts.
mod mute;
mod projects;
use crate::{
    notifications::{
        delivery::{Permission, ProjectRule},
        platform,
    },
    ui::{
        input,
        popover::{Align, Anchor, AnchorSlot, Menu, MenuAction, MenuEvent, menu_popover, popover},
        theme::*,
    },
};
use crate::{
    settings::SettingsView,
    ui::{preferences::Preferences, widgets::secondary_button},
};
use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, TimeZone};
use gpui::{AnyElement, Context, Entity, Role, Subscription, Task, Window, div, prelude::*, px};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    sync::mpsc::{Receiver, TryRecvError},
    time::Duration,
};

pub(crate) const CATEGORIES: [(&str, &str); 5] = [
    ("pullRequests", "Pull requests / Merge requests"),
    ("issues", "Issues and Linear tasks"),
    ("agentFinished", "Agent finished"),
    ("agentInput", "Agent approvals and questions"),
    ("reminders", "Reminders"),
];
fn path_key(path: &str) -> String {
    let windows =
        path.as_bytes().get(1) == Some(&b':') || path.starts_with("\\\\") || path.starts_with("//");
    let normalized = if windows || cfg!(target_os = "windows") {
        path.replace('\\', "/")
    } else {
        path.to_owned()
    };
    let normalized = normalized.trim_end_matches('/');
    if normalized.is_empty() {
        return "/".into();
    }
    if windows {
        normalized.to_lowercase()
    } else {
        normalized.into()
    }
}
fn looks_like_project(path: &str) -> bool {
    let key = path_key(path);
    !path.is_empty()
        && key != "/"
        && key != "~"
        && !(key.len() == 2 && key.ends_with(':'))
        && !path.contains(".app/")
        && !path.contains(".app\\")
        && std::env::var("HOME")
            .ok()
            .is_none_or(|home| path_key(&home) != key)
}
#[derive(Clone)]
pub(super) struct Project {
    id: String,
    name: String,
    detail: String,
    kind: String,
}
impl Preferences {
    pub(crate) fn known_notification_project_id(&self, cwd: &str) -> Option<String> {
        if !looks_like_project(cwd) {
            return None;
        }
        let key = path_key(cwd);
        self.notification_projects
            .as_array()
            .into_iter()
            .flatten()
            .find(|project| {
                valid_project(project)
                    && project["paths"].as_array().is_some_and(|paths| {
                        paths
                            .iter()
                            .any(|p| p.as_str().is_some_and(|p| path_key(p) == key))
                    })
            })
            .and_then(|project| project["id"].as_str())
            .map(str::to_owned)
            .or_else(|| Some(format!("local:{key}")))
    }
    /// None means muted until manually resumed. Time is milliseconds since epoch.
    pub(crate) fn mute_notification_project(&mut self, cwd: &str, until: Option<u64>) {
        if let Some(id) = self.known_notification_project_id(cwd) {
            self.mute_projects(
                &[id],
                Some(until.map_or(Value::Null, |at| json!(at))),
                monocode_core::sys::time::now_ms(),
            );
        }
    }
    pub(crate) fn resume_notification_project(&mut self, cwd: &str) {
        if let Some(id) = self.known_notification_project_id(cwd) {
            self.mute_projects(&[id], None, monocode_core::sys::time::now_ms());
        }
    }
    pub(crate) fn allows_project_notification(
        &self,
        cwd: &str,
        category: &str,
        occurred_at: Option<u64>,
        now: u64,
    ) -> bool {
        self.known_notification_project_id(cwd)
            .is_some_and(|id| self.allows_notification_project(&id, category, occurred_at, now))
    }
    pub(crate) fn allows_notification_project(
        &self,
        id: &str,
        category: &str,
        occurred_at: Option<u64>,
        now: u64,
    ) -> bool {
        let rule = ProjectRule::from_preferences(&self.project_notifications, id, category);
        rule.allows(occurred_at.unwrap_or(now), now)
    }
    /// How many of the times notification rules switch at (mutes ending,
    /// resumes, categories enabled) are at or before `now`. A change means
    /// some rule now answers differently.
    pub(crate) fn notification_times_passed(&self, now: u64) -> usize {
        let passed = |value: &Value| value.as_f64().is_some_and(|at| at <= now as f64);
        self.project_notifications
            .as_object()
            .into_iter()
            .flat_map(|map| map.values())
            .map(|p| {
                let enabled = p["enabledAfter"]
                    .as_object()
                    .map_or(0, |map| map.values().filter(|at| passed(at)).count());
                usize::from(passed(&p["mutedUntil"]))
                    + usize::from(passed(&p["resumedAt"]))
                    + enabled
            })
            .sum()
    }
    fn project_muted(&self, id: &str, now: u64) -> bool {
        let p = &self.project_notifications[id];
        p.get("mutedUntil") == Some(&Value::Null)
            || p["mutedUntil"].as_u64().is_some_and(|at| at > now)
    }
    fn mute_projects(&mut self, ids: &[String], until: Option<Value>, now: u64) {
        if !self.project_notifications.is_object() {
            self.project_notifications = json!({});
        }
        for id in ids {
            let p = &mut self.project_notifications[id];
            if !p.is_object() {
                *p = json!({"disabled":[]});
            }
            if let Some(until) = &until {
                p["mutedUntil"] = until.clone();
            } else if p.get("mutedUntil").is_some() {
                p.as_object_mut().unwrap().remove("mutedUntil");
                p["resumedAt"] = json!(now);
            }
        }
    }
    fn set_category(&mut self, id: &str, category: &str, enabled: bool, now: u64) {
        if !self.project_notifications.is_object() {
            self.project_notifications = json!({});
        }
        let p = &mut self.project_notifications[id];
        if !p.is_object() {
            *p = json!({"disabled":[]});
        }
        let mut disabled: Vec<_> = p["disabled"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|id| CATEGORIES.iter().any(|(cat, _)| id == cat))
            .map(str::to_owned)
            .collect();
        let was_disabled = disabled.iter().any(|id| id == category);
        disabled.retain(|id| id != category);
        if !enabled {
            disabled.push(category.to_owned());
        }
        if enabled && was_disabled {
            if !p["enabledAfter"].is_object() {
                p["enabledAfter"] = json!({});
            }
            p["enabledAfter"][category] = json!(now);
        }
        p["disabled"] = json!(disabled);
    }
}
fn valid_project(p: &Value) -> bool {
    ["id", "name", "detail"]
        .iter()
        .all(|key| p[*key].is_string())
        && p["kind"]
            .as_str()
            .is_some_and(|kind| ["repository", "local", "linear", "jira"].contains(&kind))
        && p["paths"]
            .as_array()
            .is_some_and(|paths| paths.iter().all(Value::is_string))
}
struct MuteMenu {
    menu: Entity<Menu>,
    anchor: AnchorSlot,
    _events: Subscription,
}
struct CustomMute {
    ids: Vec<String>,
    input: Entity<input::Input>,
    date: NaiveDate,
    focused_date: NaiveDate,
    focus_pending: bool,
    focus: gpui::FocusHandle,
    anchor: AnchorSlot,
    month: NaiveDate,
    error: Option<String>,
}
pub(super) struct State {
    permission: Permission,
    active: bool,
    _deadline: Option<Task<()>>,
    receiver: Option<Receiver<Permission>>,
    _poll: Option<Task<()>>,
    selecting: bool,
    selected: HashSet<String>,
    expanded: Option<String>,
    menu: Option<MuteMenu>,
    custom: Option<CustomMute>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            permission: Permission::Prompt,
            active: false,
            _deadline: None,
            receiver: None,
            _poll: None,
            selecting: false,
            selected: HashSet::new(),
            expanded: None,
            menu: None,
            custom: None,
        }
    }
}
impl SettingsView {
    pub(super) fn notification_window_state(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = window.is_window_active();
        if active && !self.notification_state.active && self.prefs.notifications {
            self.probe_notification_permission(false, cx);
        }
        self.notification_state.active = active;
        if let Some(custom) = &mut self.notification_state.custom
            && custom.focus_pending
        {
            custom.focus_pending = false;
            window.focus(&custom.focus, cx);
        }
    }
    pub(super) fn dismiss_notification_transient(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.notification_state.custom.take().is_some()
            || self.notification_state.menu.take().is_some()
        {
            window.focus(&self.focus, cx);
            cx.notify();
            return true;
        }
        false
    }
    pub(super) fn notification_deadline(&mut self, cx: &mut Context<Self>) {
        let now = monocode_core::sys::time::now_ms();
        let deadline = self
            .prefs
            .project_notifications
            .as_object()
            .into_iter()
            .flat_map(|map| map.values())
            .filter_map(|p| p["mutedUntil"].as_u64())
            .filter(|at| *at > now)
            .min();
        self.notification_state._deadline = deadline.map(|at| {
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(at.saturating_sub(now)))
                    .await;
                let _ = this.update(cx, |this, cx| {
                    this.notification_deadline(cx);
                    cx.notify();
                });
            })
        });
    }
    pub(super) fn probe_notification_permission(&mut self, request: bool, cx: &mut Context<Self>) {
        self.notification_state.receiver = Some(if request {
            platform::request_permission()
        } else {
            platform::permission()
        });
        self.notification_state._poll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let done = this
                    .update(cx, |this, cx| {
                        let result = this
                            .notification_state
                            .receiver
                            .as_ref()
                            .map(Receiver::try_recv);
                        match result {
                            Some(Ok(permission)) => {
                                this.notification_state.permission = permission;
                                this.notification_state.receiver = None;
                                cx.notify();
                                true
                            }
                            Some(Err(TryRecvError::Empty)) => false,
                            _ => {
                                this.notification_state.receiver = None;
                                true
                            }
                        }
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
        }));
    }
    pub(super) fn notification_permission_controls(
        &self,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        if !self.prefs.notifications {
            return Vec::new();
        }
        match self.notification_state.permission {
            Permission::Denied => vec![
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.))
                    .text_color(content(0.45))
                    .child("Permission needed")
                    .child(
                        secondary_button(
                            "notifications-system-settings",
                            "Open System Settings",
                            false,
                            false,
                        )
                        .child("Open System Settings")
                        .on_click(cx.listener(|this, _, _, cx| {
                            let _ = platform::open_settings();
                            this.probe_notification_permission(false, cx);
                        })),
                    )
                    .into_any_element(),
            ],
            Permission::Unsupported => vec![
                div()
                    .text_size(px(12.))
                    .text_color(content(0.45))
                    .child("Not available on this platform")
                    .into_any_element(),
            ],
            _ => Vec::new(),
        }
    }
    pub(super) fn notification_overlay(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut overlays = Vec::new();
        if let Some(menu) = &self.notification_state.menu {
            overlays.push(
                menu_popover(&menu.menu, Anchor::Slot(menu.anchor.clone()), cx)
                    .gap(4.)
                    .into_any_element(),
            );
        }
        if let Some(custom) = &self.notification_state.custom {
            overlays.push(
                popover(
                    "notification-custom-mute",
                    Anchor::Slot(custom.anchor.clone()),
                )
                .align(Align::End)
                .width(280.)
                .gap(6.)
                .on_dismiss({
                    let weak = cx.weak_entity();
                    move |_, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.notification_state.custom = None;
                            cx.notify();
                        });
                    }
                })
                .child(
                    div()
                        .id("project-notification-accessible-4")
                        .role(Role::Dialog)
                        .aria_label("Mute project notifications")
                        .w(px(280.))
                        .p_3()
                        .rounded_lg()
                        .border_1()
                        .border_color(content(0.15))
                        .bg(pane_fill())
                        .shadow_xl()
                        .child(
                            div()
                                .mb_3()
                                .text_size(px(11.))
                                .text_color(content(0.45))
                                .child("Mute all notifications until"),
                        )
                        .child(self.mute_calendar(custom, cx))
                        .child(
                            div()
                                .mt_3()
                                .text_size(px(10.))
                                .text_color(content(0.4))
                                .child("Time · Local time, 24-hour"),
                        )
                        .child(
                            div()
                                .h(px(34.))
                                .px_2()
                                .border_1()
                                .border_color(content(0.15))
                                .rounded_md()
                                .child(custom.input.clone()),
                        )
                        .when(custom.error.is_some(), |d| {
                            d.child(
                                div()
                                    .id("project-notification-accessible-5")
                                    .role(Role::Alert)
                                    .mt_3()
                                    .text_size(px(12.))
                                    .text_color(gpui::rgb(0xff6467))
                                    .child(custom.error.clone().unwrap_or_default()),
                            )
                        })
                        .child(
                            div()
                                .mt_3()
                                .pt(px(10.))
                                .border_t_1()
                                .border_color(content(0.1))
                                .flex()
                                .justify_between()
                                .child(
                                    secondary_button(
                                        "cancel-custom-notification-mute",
                                        "Cancel",
                                        false,
                                        false,
                                    )
                                    .child("Cancel")
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            this.notification_state.custom = None;
                                            cx.notify();
                                        },
                                    )),
                                )
                                .child(
                                    secondary_button(
                                        "save-custom-notification-mute",
                                        "Mute until then",
                                        false,
                                        false,
                                    )
                                    .child("Mute until then")
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            let Some(custom) = &this.notification_state.custom
                                            else {
                                                return;
                                            };
                                            let parsed = NaiveDateTime::parse_from_str(
                                                &format!(
                                                    "{} {}",
                                                    custom.date,
                                                    custom.input.read(cx).content.trim()
                                                ),
                                                "%Y-%m-%d %H:%M",
                                            )
                                            .ok()
                                            .and_then(|at| {
                                                Local.from_local_datetime(&at).earliest()
                                            })
                                            .map(|at| at.timestamp_millis().max(0) as u64);
                                            let now = monocode_core::sys::time::now_ms();
                                            match parsed {
                                                Some(at) if at > now => {
                                                    let ids = custom.ids.clone();
                                                    this.update(cx, |p| {
                                                        p.mute_projects(&ids, Some(json!(at)), now)
                                                    });
                                                    this.notification_state.custom = None;
                                                }
                                                _ => {
                                                    this.notification_state
                                                        .custom
                                                        .as_mut()
                                                        .unwrap()
                                                        .error = Some(
                                                        if parsed.is_some() {
                                                            "Choose a date and time in the future."
                                                        } else {
                                                            "Choose a valid date and time."
                                                        }
                                                        .into(),
                                                    );
                                                }
                                            }
                                            cx.notify();
                                        },
                                    )),
                                ),
                        ),
                )
                .into_any_element(),
            );
        }
        overlays
    }
}
impl SettingsView {}
