// The pickers that open as flyouts beside a menu row: ModelPicker.tsx's
// ModelFlyout and SearchableProjectPicker.tsx's ProjectPickerPopover. Each
// owns a search field; arrows and Enter in it move and pick in the list.
mod model_list;
mod model_list_view;
mod project;
use crate::ui::{input, project_badge::project_name, theme::*};
use gpui::{
    AnyElement, Context, Entity, EventEmitter, FocusHandle, Focusable, Role, SharedString,
    Subscription, Window, div, prelude::*, px,
};
use monocode_core::provider::catalog::{self, Model};
use monocode_core::provider::{self, Kind};

/// Friendly names used before a provider's live catalog arrives.
pub(crate) const CLAUDE_MODELS: &[(&str, &str)] = &[
    ("claude-sonnet-5", "Claude Sonnet 5"),
    ("claude-opus-5", "Claude Opus 5"),
    ("claude-opus-5-5", "Claude Opus 5.5"),
    ("claude-fable-5", "Claude Fable 5"),
    ("claude-opus-4-6", "Opus 4.6"),
    ("claude-sonnet-4-6", "Sonnet 4.6"),
    ("claude-haiku-4-5", "Haiku 4.5"),
    ("claude-opus-4-5", "Opus 4.5"),
];

/// The display name for a model id; `None` is the provider's default.
pub(crate) fn model_name(kind: Kind, model: Option<&str>) -> SharedString {
    if model.is_some_and(contains_account_id) {
        return kind.label().into();
    }
    match model {
        None => kind.label().into(),
        Some("sonnet") => "Sonnet".into(),
        Some("opus") => "Opus".into(),
        Some("haiku") => "Haiku".into(),
        Some(id) => CLAUDE_MODELS
            .iter()
            .find(|(model, _)| *model == id)
            .map_or_else(
                || {
                    if let Some(name) = id.strip_prefix("gpt-") {
                        let parts = name
                            .split('-')
                            .map(|part| match part {
                                "codex" => "Codex".into(),
                                "sol" => "Sol".into(),
                                "astra" => "Astra".into(),
                                "mini" => "Mini".into(),
                                "max" => "Max".into(),
                                other => other.to_owned(),
                            })
                            .collect::<Vec<String>>()
                            .join("-");
                        format!("GPT-{parts}").into()
                    } else {
                        id.to_owned().into()
                    }
                },
                |(_, name)| (*name).into(),
            ),
    }
}

/// Account metadata is never a model label, including stale fixture responses.
pub(crate) fn model_display_name(kind: Kind, id: &str, name: &str) -> SharedString {
    if name.trim().is_empty() || contains_account_id(name) {
        model_name(kind, Some(id))
    } else {
        name.to_owned().into()
    }
}
/// A model's name from a cached catalog, else its friendly fallback.
pub(crate) fn cached_model_name(kind: Kind, id: Option<&str>) -> SharedString {
    id.and_then(|id| catalog::cached_name(kind, id).map(|name| model_display_name(kind, id, &name)))
        .unwrap_or_else(|| model_name(kind, id))
}
/// saveRecentModelChoice: the pick moves to the front of the six recents.
pub(crate) fn remember_model(kind: Kind, model: Option<String>, cx: &mut gpui::App) {
    if cx.has_global::<crate::ui::preferences::Preferences>() {
        cx.update_global::<crate::ui::preferences::Preferences, _>(|prefs, _| {
            let choice = crate::ui::preferences::ModelChoice {
                provider: kind.id().into(),
                model,
            };
            prefs.recent_models.retain(|old| old != &choice);
            prefs.recent_models.insert(0, choice);
            prefs.recent_models.truncate(6);
        });
    }
}
/// The accessible name of a row's favorite star.
fn favorite_label(starred: bool, name: &str) -> String {
    if starred {
        format!("Remove {name} from favorites")
    } else {
        format!("Add {name} to favorites")
    }
}
fn contains_account_id(value: &str) -> bool {
    value.match_indices("account-").any(|(offset, _)| {
        let tail = &value[offset + "account-".len()..];
        let end = tail
            .find(|c: char| !c.is_ascii_hexdigit() && c != '-')
            .unwrap_or(tail.len());
        let token = tail[..end].trim_end_matches('-');
        let canonical = token.len() == 36
            && token.bytes().enumerate().all(|(i, b)| {
                if [8, 13, 18, 23].contains(&i) {
                    b == b'-'
                } else {
                    b.is_ascii_hexdigit()
                }
            });
        (token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit())) || canonical
    })
}

#[cfg(test)]
mod tests;

fn search_field<T: 'static>(
    label: &'static str,
    placeholder: &'static str,
    placeholder_alpha: f32,
    cx: &mut Context<T>,
) -> Entity<input::Input> {
    cx.new(|cx| input::Input::field(label, placeholder, content(placeholder_alpha), cx))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ModelTab {
    Favorites,
    Harness(Kind),
}

struct ModelEntry {
    kind: Kind,
    id: Option<String>,
    name: SharedString,
    enabled: bool,
}

/// The model the next turn uses; `None` is the provider's default.
pub(crate) struct ModelPicked(pub Kind, pub Option<String>);

/// ModelFlyout: providers down a rail on the left, a search field over the
/// selected provider's models on the right. Switching providers is available
/// between turns.
pub(crate) struct ModelList {
    provider: Kind,
    remote_project: Option<String>,
    remote_providers: Option<Vec<Kind>>,
    current: Option<String>,
    profiles: std::collections::HashMap<Kind, monocode_core::session::profile::Profile>,
    catalogs: Vec<(Kind, Vec<Model>)>,
    errors: Vec<(Kind, String)>,
    loading: Vec<Kind>,
    missing: Vec<Kind>,
    switch_allowed: bool,
    hidden: Vec<String>,
    /// QuickModelSelector.tsx: providers as tabs above the search and list.
    quick: bool,
    tab: ModelTab,
    highlighted: usize,
    search: Entity<input::Input>,
    _changed: Subscription,
}
impl EventEmitter<ModelPicked> for ModelList {}
impl ModelList {}
impl ModelList {}

pub(crate) enum ProjectPicked {
    Project(String),
    Mono(String),
    New,
    Remote,
}

pub(crate) struct PickerMono {
    pub id: String,
    pub name: String,
    pub mascot: String,
    pub color: u32,
}

/// ProjectPickerPopover: a search field over the projects, the current one
/// first and checked, and "New project" at the foot.
pub(crate) struct ProjectPicker {
    appearance:
        std::collections::HashMap<String, monocode_core::project::appearance::ProjectAppearance>,
    projects: Vec<String>,
    monos: Vec<PickerMono>,
    active_mono: Option<String>,
    current: String,
    active: usize,
    allow_remote: bool,
    search: Entity<input::Input>,
    _changed: Subscription,
}
impl EventEmitter<ProjectPicked> for ProjectPicker {}
/// `prettyParent`: the folder holding a project, with the home folder as `~`.
pub(crate) fn pretty_parent(path: &str) -> String {
    let parent = std::path::Path::new(path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && parent.starts_with(&home) => {
            format!("~{}", &parent[home.len()..])
        }
        _ => parent,
    }
}
