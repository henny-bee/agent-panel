mod look_editor;
mod pages;
mod pickers;
mod settings_modal;
use super::*;
use crate::{
    chat::composer::pickers::{ModelList, ModelPicked},
    project::settings::{BackgroundDialog, Event as BackgroundEvent},
    ui::popover::{MenuAction, MenuItem, MenuStyle, Side},
};
use gpui::{AnyView, Div, FontWeight, Stateful};
use look_editor::{LookChanged, LookEditor, LookSubmitted};
use monocode_core::mono::heartbeat;
use monocode_core::project::appearance::{MASCOTS, ProjectAppearance};
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum OverviewTab {
    Overview,
    Sessions,
    Habits,
    Documents,
    Changes,
    Activity,
}
#[derive(Clone, Copy, PartialEq)]
pub(super) enum SettingsSection {
    General,
    Soul,
    Memory,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Route {
    Review,
    Session(i64),
    Document(String),
    Habit(String),
    NewHabit,
}
impl Route {
    pub(super) fn key(&self) -> String {
        match self {
            Self::Review => "review".into(),
            Self::Session(id) => format!("session:{id}"),
            Self::Document(id) => format!("document:{id}"),
            Self::Habit(id) => format!("habit:{id}"),
            Self::NewHabit => "new-habit".into(),
        }
    }
}
/// What a Mono's details panel showed when another Mono was opened.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PanelView {
    pub(super) tab: OverviewTab,
    pub(super) previous: OverviewTab,
    pub(super) route: Option<Route>,
}
impl Default for PanelView {
    fn default() -> Self {
        Self {
            tab: OverviewTab::Overview,
            previous: OverviewTab::Overview,
            route: None,
        }
    }
}
impl PanelView {
    /// The view to show again, keeping only a page whose subject still exists.
    /// Reviews and habit pages depend on state reloaded per Mono, so they
    /// return to their tab's list.
    pub(super) fn restored(mut self, exists: impl Fn(&Route) -> bool) -> Self {
        self.route = self.route.filter(|route| {
            matches!(route, Route::Session(_) | Route::Document(_)) && exists(route)
        });
        self
    }
}
impl Monos {
    pub(super) fn remember_panel_view(&mut self) {
        if let Some(id) = self.active.clone() {
            let view = PanelView {
                tab: self.overview_tab,
                previous: self.overview_previous,
                route: self.route.clone(),
            };
            self.panel_views.insert(id, view);
        }
    }
    /// Restores `id`'s remembered view. Callers may be inside the Shell's own
    /// update, so whether a session still exists unarchived is checked after.
    /// A document listed by the chat may have been deleted from the shared
    /// store while `id` was inactive, so its record is checked there too.
    pub(super) fn restore_panel_view(&mut self, id: &str, cx: &mut Context<Self>) {
        let saved = self.panel_views.get(id).cloned().unwrap_or_default();
        let mono = self.roster.get(id);
        let view = saved.restored(|route: &Route| match route {
            Route::Session(_) => true,
            Route::Document(document) => {
                mono.is_some_and(|m| m.documents.iter().any(|d| &d.id == document))
            }
            _ => false,
        });
        self.panel_check = None;
        if let Some(Route::Document(document)) = view.route.clone()
            && !mono.is_some_and(|m| super::work::inline_document(m, &document))
        {
            let store = self.store.clone();
            let target = document.clone();
            let exists = cx
                .background_executor()
                .spawn(async move { locked(&store)?.mono_document(&target).map(|d| d.is_some()) });
            self.panel_check = Some(cx.spawn(async move |this, cx| {
                if exists.await == Ok(false) {
                    let _ = this.update(cx, |this, cx| this.forget_document(&document, cx));
                }
            }));
        }
        if let Some(Route::Session(session)) = view.route {
            self.panel_check = Some(cx.spawn(async move |this, cx| {
                let _ = this.update(cx, |this, cx| {
                    let live = this.owner.upgrade().is_some_and(|owner| {
                        owner
                            .read(cx)
                            .mono_session_views(cx)
                            .iter()
                            .any(|v| v.id == session && !v.archived)
                    });
                    if !live && this.route == Some(Route::Session(session)) {
                        this.route = None;
                        cx.notify();
                    }
                });
            }));
        }
        self.overview_tab = view.tab;
        self.overview_previous = view.previous;
        self.overview_reveal = None;
        self.route = view.route;
    }
}
pub(super) struct OpenPicker {
    pub(super) menu: Entity<Menu>,
    pub(super) anchor: Anchor,
    pub(super) width: f32,
    pub(super) align: crate::ui::popover::Align,
    pub(super) _events: Vec<Subscription>,
}
pub(super) fn button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
    let label = label.into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_1p5()
        .px_2p5()
        .py_1p5()
        .rounded_md()
        .text_size(px(12.))
        .text_color(content(0.8))
        .hover(|s| s.bg(content(0.08)).text_color(content(1.)))
        .child(label)
}
fn heading(title: &str, description: Option<&str>) -> Div {
    div()
        .pt_7()
        .pb_2()
        .px_1()
        .text_size(px(12.))
        .text_color(content(0.7))
        .font_weight(FontWeight::MEDIUM)
        .child(title.to_string())
        .when_some(description, |d, text| {
            d.child(
                div()
                    .line_height(px(20.))
                    .text_color(content(0.4))
                    .font_weight(FontWeight::NORMAL)
                    .child(text.to_string()),
            )
        })
}
/// The outlined, rounded card each Sub-agents, Documents and Habits row sits in.
pub(super) fn outlined<E: Styled + InteractiveElement>(element: E) -> E {
    element
        .rounded_xl()
        .border_1()
        .border_color(content(0.07))
        .bg(content(0.025))
        .hover(|s| s.bg(content(0.05)).border_color(content(0.14)))
}
enum AgentState {
    Working,
    NeedsInput,
    Idle(u64),
    Unavailable,
}
struct SubAgent<'a> {
    id: i64,
    title: String,
    harness: &'a str,
    /// The live session's model label, when it is loaded.
    model: Option<SharedString>,
    model_id: Option<&'a str>,
    place: String,
    branch: bool,
    state: AgentState,
}
/// A sub-agent the Mono started: who it is (model and provider) first, then
/// what it was asked to do and where.
fn sub_agent_card(agent: SubAgent) -> Stateful<Div> {
    let kind = Kind::from_id(agent.harness);
    let model = agent
        .model
        .unwrap_or_else(|| crate::chat::composer::pickers::cached_model_name(kind, agent.model_id));
    // Claude models already say who made them.
    let model: SharedString = model
        .strip_prefix("Claude ")
        .map_or(model.clone(), |short| short.to_owned().into());
    let amber: gpui::Hsla = rgb(0xfbbf24).into();
    let (label, color, live): (String, gpui::Hsla, bool) = match agent.state {
        AgentState::Working => ("Working".into(), accent_color(), true),
        AgentState::NeedsInput => ("Needs approval".into(), amber, true),
        AgentState::Idle(at) => (
            monocode_core::sys::time::relative(at, monocode_core::sys::time::now_ms()),
            content(0.45),
            false,
        ),
        AgentState::Unavailable => ("Unavailable".into(), content(0.45), false),
    };
    let needs_input = matches!(agent.state, AgentState::NeedsInput);
    let id = agent.id;
    outlined(
        div()
            .id(("mono-launch", id as usize))
            .debug_selector(move || format!("mono-launch-{id}"))
            .role(Role::Button)
            .aria_label(format!("{}, {model}", agent.title)),
    )
    .when(needs_input, |d| {
        let mut c = amber;
        c.a = 0.45;
        d.border_color(c)
    })
    .when(matches!(agent.state, AgentState::Unavailable), |d| {
        d.opacity(0.5)
    })
    .flex_shrink_0()
    .px(px(10.))
    .py_2()
    .flex()
    .items_center()
    .gap_2p5()
    .child(
        div()
            .size(px(28.))
            .flex_shrink_0()
            .rounded_md()
            .bg(content(0.06))
            .flex()
            .items_center()
            .justify_center()
            .child(provider_icon(agent.harness, 16.)),
    )
    .child(
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            // What it was asked to do, and how it is doing.
            .child(
                div()
                    .flex()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .line_height(px(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(content(1.))
                            .child(agent.title),
                    )
                    .child(
                        div()
                            .ml_auto()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .text_size(px(11.))
                            .text_color(color)
                            .when(live, |d| {
                                d.child(div().size(px(6.)).rounded_full().bg(color))
                            })
                            .child(label),
                    ),
            )
            // Which model runs it, and where.
            .child(
                div()
                    .flex()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .text_size(px(11.))
                    .line_height(px(16.))
                    .text_color(content(0.5))
                    .child(div().min_w_0().truncate().child(model))
                    .child(
                        div()
                            .ml_auto()
                            .min_w_0()
                            .max_w(px(160.))
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_color(content(0.4))
                            .child(
                                icon(
                                    if agent.branch {
                                        "git-branch"
                                    } else {
                                        "folder-01"
                                    },
                                    11.,
                                )
                                .text_color(content(0.4)),
                            )
                            .child(div().min_w_0().truncate().child(agent.place)),
                    ),
            ),
    )
}
fn card() -> Div {
    div()
        .rounded_xl()
        .border_1()
        .border_color(content(0.04))
        .overflow_hidden()
        .bg(content(0.02))
        .flex()
        .flex_col()
}
/// MonoOverview's search row above the Sessions, Habits and Documents lists.
pub(super) fn search_bar(field: Entity<Input>) -> Div {
    div()
        .h(px(36.))
        .flex_shrink_0()
        .px_2()
        .border_b_1()
        .border_color(crate::ui::panel::stroke())
        .flex()
        .items_center()
        .gap_2()
        .child(icon("search", 12.))
        .child(div().flex_1().min_w_0().child(field))
}
/// Whether `haystack` contains `query`, ignoring case and surrounding spaces.
pub(super) fn matches_query(query: &str, haystack: &str) -> bool {
    haystack
        .to_lowercase()
        .contains(&query.trim().to_lowercase())
}
/// monoPanelParts' EmptyState: an icon over a short, centred note.
pub(super) fn empty_state(glyph: &'static str, message: &str) -> AnyElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .px_6()
        .py_10()
        .text_center()
        .child(icon(glyph, 18.).text_color(content(0.55)))
        .child(
            div()
                .max_w(px(288.))
                .text_size(px(12.))
                .line_height(px(18.))
                .text_color(content(0.55))
                .child(message.to_string()),
        )
        .into_any_element()
}
fn empty(message: &str) -> AnyElement {
    div()
        .px_5()
        .py_8()
        .text_size(px(12.))
        .line_height(px(20.))
        .text_color(content(0.45))
        .child(message.to_string())
        .into_any_element()
}
impl EventEmitter<LookChanged> for LookEditor {}
impl EventEmitter<LookSubmitted> for LookEditor {}
impl Monos {}

/// The model and effort for sessions a Mono starts, as its settings show it.
struct SubsessionChoice {
    /// Set only when a provider is picked; inherited shows no logo.
    provider: Option<Kind>,
    kind: Kind,
    profile: monocode_core::session::profile::Profile,
    current_model: Option<String>,
    model_label: SharedString,
    efforts: Vec<(String, String)>,
    effort_label: SharedString,
}
