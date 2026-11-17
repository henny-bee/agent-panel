//! Many windows, one app. Windows onto the same database show the same live
//! view of a session, so a Claude CLI runs once however many windows show it.
//! The first window runs the app's background services and saves the
//! workspace; the window used last takes them over when it closes. Settings,
//! projects and their look are the app's, so its windows keep them in step.
use super::*;
use gpui::{EntityId, WeakEntity};

#[derive(Default)]
struct Windows {
    /// Each window's shell, keyed by its database, the one used last first.
    shells: Vec<(usize, WeakEntity<Shell>)>,
    /// The live view of every open session, whichever window opened it.
    sessions: HashMap<(usize, i64), WeakEntity<Conversation>>,
    /// The workspace saved when the app quits, written by the services window.
    states: HashMap<usize, Arc<Mutex<String>>>,
}
impl gpui::Global for Windows {}

/// Windows share an app when they share a database.
fn app_key(store: &Database) -> usize {
    Arc::as_ptr(store) as usize
}

/// The other windows onto `store`, the one used last first. Never this one:
/// it is mid-update.
fn others(me: EntityId, store: &Database, cx: &App) -> Vec<Entity<Shell>> {
    let key = app_key(store);
    cx.try_global::<Windows>()
        .map(|windows| {
            windows
                .shells
                .iter()
                .filter(|(app, shell)| *app == key && shell.entity_id() != me)
                .filter_map(|(_, shell)| shell.upgrade())
                .collect()
        })
        .unwrap_or_default()
}

impl Shell {
    /// Join the app's windows. True for the first, which runs the services.
    pub(super) fn join_windows(store: &Database, cx: &mut Context<Self>) -> bool {
        let key = app_key(store);
        let shell = cx.weak_entity();
        let windows = cx.default_global::<Windows>();
        windows
            .shells
            .retain(|(_, shell)| shell.upgrade().is_some());
        let first = !windows.shells.iter().any(|(app, _)| *app == key);
        windows.shells.push((key, shell));
        first
    }
    /// The app's quit-time workspace, once the services window made it.
    pub(super) fn app_state(store: &Database, cx: &App) -> Option<Arc<Mutex<String>>> {
        cx.try_global::<Windows>()?
            .states
            .get(&app_key(store))
            .cloned()
    }
    pub(super) fn set_app_state(store: &Database, state: Arc<Mutex<String>>, cx: &mut App) {
        cx.default_global::<Windows>()
            .states
            .insert(app_key(store), state);
    }
    /// This window became the one in use.
    pub(super) fn window_used(&self, cx: &mut Context<Self>) {
        let key = app_key(&self.store);
        let shell = cx.weak_entity();
        let me = shell.entity_id();
        let windows = cx.default_global::<Windows>();
        windows
            .shells
            .retain(|(_, shell)| shell.entity_id() != me && shell.upgrade().is_some());
        windows.shells.insert(0, (key, shell));
    }

    /// The view another window already has for this session.
    pub(super) fn shared_session(&self, id: i64, cx: &App) -> Option<Entity<Conversation>> {
        cx.try_global::<Windows>()?
            .sessions
            .get(&(app_key(&self.store), id))?
            .upgrade()
    }
    /// Offer a new live view to the other windows. The services window watches
    /// it too, so a turn finishing here still notifies and automation runs
    /// still complete; the others list the session in their sidebars.
    pub(super) fn share_session(
        &self,
        id: i64,
        view: &Entity<Conversation>,
        cx: &mut Context<Self>,
    ) {
        let sessions = &mut cx.default_global::<Windows>().sessions;
        sessions.retain(|_, view| view.upgrade().is_some());
        sessions.insert((app_key(&self.store), id), view.downgrade());
        let others = others(cx.entity_id(), &self.store, cx);
        if others.is_empty() {
            return;
        }
        let summary = view.read(cx).summary.clone();
        // After this update: the others may look back at this window.
        cx.defer(move |cx| {
            for other in others {
                other.update(cx, |other, cx| {
                    if other.main {
                        other.add_loaded(summary.clone(), cx);
                    } else if let std::collections::hash_map::Entry::Vacant(entry) =
                        other.session_summaries.entry(id)
                    {
                        entry.insert(summary.clone());
                        other.order.push(id);
                    }
                    other.invalidate_chrome(cx);
                });
            }
        });
    }
    /// Of the windows holding a session, the one used last acts on what the
    /// session asks for, so a click acts once and in the window it came from.
    pub(super) fn acts_for(&self, id: i64, cx: &Context<Self>) -> bool {
        self.first_used(cx, |shell| shell.sessions.contains_key(&id))
    }
    /// Of the windows showing a session, the one used last sets its layout
    /// (split, frame), so two windows never undo each other's every frame.
    pub(super) fn lays_out(&self, id: i64, cx: &Context<Self>) -> bool {
        self.first_used(cx, |shell| shell.shown.contains(&id))
    }
    /// True unless a window used more recently than this one also `has` it.
    fn first_used(&self, cx: &Context<Self>, has: impl Fn(&Shell) -> bool) -> bool {
        let me = cx.entity_id();
        let key = app_key(&self.store);
        let Some(windows) = cx.try_global::<Windows>() else {
            return true;
        };
        for (app, shell) in &windows.shells {
            if *app != key {
                continue;
            }
            if shell.entity_id() == me {
                return true;
            }
            if shell.upgrade().is_some_and(|shell| has(shell.read(cx))) {
                return false;
            }
        }
        true
    }
    /// A new window starts from the app-wide settings the others already use.
    pub(super) fn adopt_app_workspace(
        workspace: &mut Workspace,
        store: &Database,
        cx: &Context<Self>,
    ) {
        if let Some(other) = others(cx.entity_id(), store, cx).first() {
            workspace.adopt_shared(&other.read(cx).workspace);
        }
    }
    /// Sessions on screen in any window. One hidden here may show in another,
    /// and must keep its history and stay live there.
    pub(super) fn shown_anywhere(
        &self,
        mut shown: HashSet<i64>,
        cx: &Context<Self>,
    ) -> HashSet<i64> {
        for other in others(cx.entity_id(), &self.store, cx) {
            shown.extend(other.read(cx).shown.iter().copied());
        }
        shown
    }
    /// The services window's Monos, for a window that runs none of its own.
    pub(super) fn app_monos(&self, cx: &Context<Self>) -> Option<Entity<crate::monos::Monos>> {
        self.monos.clone().or_else(|| {
            others(cx.entity_id(), &self.store, cx)
                .into_iter()
                .find_map(|shell| shell.read(cx).monos.clone())
        })
    }
    /// The app bridge settings a session's tools reach this app through.
    pub(super) fn app_environment(
        &self,
        id: i64,
        cx: &Context<Self>,
    ) -> Option<HashMap<String, String>> {
        match &self.app_bridge {
            Some(bridge) => Some(bridge.environment(id)),
            None => others(cx.entity_id(), &self.store, cx)
                .into_iter()
                .find_map(|shell| {
                    shell
                        .read(cx)
                        .app_bridge
                        .as_ref()
                        .map(|bridge| bridge.environment(id))
                }),
        }
    }

    /// Hand this window's app-wide settings to the others. Saving them stays
    /// with the services window.
    pub(super) fn share_workspace(&self, cx: &mut Context<Self>) {
        let others = others(cx.entity_id(), &self.store, cx);
        if others.is_empty() {
            return;
        }
        let shared = self.workspace.shared_part();
        // After this update: applying them may look back at this window.
        cx.defer(move |cx| {
            for other in others {
                other.update(cx, |other, cx| {
                    let preferences = other.workspace.preferences != shared.preferences;
                    if other.workspace.adopt_shared(&shared) {
                        if preferences {
                            other.apply_preferences(cx);
                        }
                        other.write_state(cx);
                        other.invalidate_chrome(cx);
                        cx.notify();
                    }
                });
            }
        });
    }
    /// The closing services window hands them to the window used last.
    pub(super) fn hand_over_services(store: &Database, cx: &mut App) {
        let key = app_key(store);
        let next = cx.try_global::<Windows>().and_then(|windows| {
            windows
                .shells
                .iter()
                .filter(|(app, _)| *app == key)
                .find_map(|(_, shell)| shell.upgrade())
        });
        if let Some(next) = next {
            next.update(cx, |shell, cx| shell.start_services(cx));
        }
    }
    /// Background work the app runs once: Monos, automations, reminders,
    /// notifications, phone access and the bridge sessions reach the app by.
    pub(super) fn start_services(&mut self, cx: &mut Context<Self>) {
        if self.main {
            return;
        }
        self.main = true;
        self.start_monos(cx);
        self.init_app_bridge(cx);
        self.init_mobile_host(cx);
        self.poll_reminders(cx);
        self.initialize_automations(cx);
        if self.window_observed {
            self.start_native_notifications(cx);
        }
        self.write_state(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn windows_share_live_sessions_and_settings_and_hand_over_services(cx: &mut TestAppContext) {
        let (first, a, _) = super::super::tests::window(cx);
        let store = first.update(cx, |shell, _, _| shell.store.clone()).unwrap();
        let summaries = locked(&store).unwrap().sessions().unwrap();
        let second = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| Shell::new(store, summaries, "/work/monocode".into(), None, cx))
            })
            .unwrap()
        });
        let view = first
            .update(cx, |shell, _, cx| {
                assert!(shell.main, "the first window runs the services");
                shell.ensure_session(a, cx).unwrap()
            })
            .unwrap();
        second
            .update(cx, |shell, _, cx| {
                assert!(!shell.main && shell.monos.is_none());
                let shared = shell.ensure_session(a, cx).unwrap();
                assert_eq!(shared.entity_id(), view.entity_id(), "one live view");
                assert!(
                    !shell.acts_for(a, cx),
                    "the first window acts until another is used"
                );
                shell.window_used(cx);
                assert!(shell.acts_for(a, cx));
                shell.workspace.preferences.sounds = !shell.workspace.preferences.sounds;
                shell.save_state(cx);
            })
            .unwrap();
        let sounds = second
            .update(cx, |shell, _, _| shell.workspace.preferences.sounds)
            .unwrap();
        let fresh = second
            .update(cx, |shell, _, cx| {
                let summary = locked(&shell.store)
                    .unwrap()
                    .create("codex", "/work/monocode")
                    .unwrap();
                let id = summary.id;
                shell.add(summary, cx);
                id
            })
            .unwrap();
        first
            .update(cx, |shell, _, cx| {
                assert_eq!(shell.workspace.preferences.sounds, sounds, "settings sync");
                assert!(!shell.acts_for(a, cx), "a request acts once");
                assert!(
                    shell.sessions.contains_key(&fresh),
                    "the services window watches a session opened elsewhere"
                );
            })
            .unwrap();
        // A window onto another database is another app: nothing is shared.
        let other = cx.update(|cx| {
            let store = Arc::new(Mutex::new(
                monocode_core::store::Store::open(std::path::Path::new(":memory:")).unwrap(),
            ));
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| Shell::new(store, Vec::new(), "/work/other".into(), None, cx))
            })
            .unwrap()
        });
        other.update(cx, |shell, _, _| assert!(shell.main)).unwrap();
        first
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();
        cx.run_until_parked();
        second
            .update(cx, |shell, _, _| {
                assert!(shell.main && shell.monos.is_some(), "services move on");
            })
            .unwrap();
        assert!(view.read_with(cx, |view, _| view.summary.id == a));
    }
}
