//! Native counterpart of AutomationsView: persisted list, example gallery,
//! trigger editor and run history. All scheduling lives in monocode-core.
use crate::{
    chat::composer::pickers::{
        ModelList, ModelPicked, ProjectPicked, ProjectPicker, cached_model_name,
    },
    runtime::{Database, locked},
    ui::{
        input::{self, Input},
        popover::{Anchor, AnchorSlot, popover},
        theme::*,
    },
};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Role, SharedString,
    Subscription, Window, div, prelude::*, px, rgb,
};
use monocode_core::{
    automations::{
        self, Service,
        model::{Automation, AutomationRun, AutomationTrigger, AutomationUpsert, DueAutomationRun},
    },
    provider::Kind,
    session::groups::Folder,
};
use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

mod editor;
mod gallery;
mod trigger_menu;
mod view;

/// Event providers and schedules are separate from agent harness branding.
fn trigger_mark(kind: &str, side: f32) -> AnyElement {
    if kind == "gitlab" {
        return gpui::img("inbox-gitlab-color.svg")
            .size(px(side))
            .flex_shrink_0()
            .object_fit(gpui::ObjectFit::Contain)
            .into_any_element();
    }
    let (path, color) = match kind {
        "time" => ("clock", content(0.45)),
        "github" => ("inbox-github", content(0.65)),
        "linear" => ("inbox-linear", content(0.65)),
        "jira" => ("inbox-jira", rgb(0x2684ff).into()),
        "azuredevops" => ("inbox-azuredevops", content(0.65)),
        _ => ("zap", content(0.45)),
    };
    icon(path, side).text_color(color).into_any_element()
}

pub(crate) enum Event {
    Close,
    Launch(Box<DueAutomationRun>),
    OpenSession(String),
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Template {
    id: String,
    category: String,
    #[serde(default)]
    popular: bool,
    icon: String,
    name: String,
    description: String,
    prompt: String,
    trigger: serde_json::Value,
    trigger_label: String,
}
struct TriggerFields {
    id: String,
    time: Entity<Input>,
    repos: Entity<Input>,
    actor: Entity<Input>,
}
struct Editor {
    draft: AutomationUpsert,
    name: Entity<Input>,
    prompt: Entity<Input>,
    completion: Entity<crate::chat::composer::completion::Completion>,
    fields: Vec<TriggerFields>,
    saved: bool,
    dirty: bool,
    _subscriptions: Vec<Subscription>,
}
#[derive(Clone)]
enum Choice {
    Access,
    Workspace,
    Conversation,
    Folder,
    Grace,
    AddTrigger,
    TriggerEvent(String),
    Schedule(String),
    Time(String),
    Minute(String),
    Weekday(String),
    ModelSetting(String),
}
struct ChoiceMenu {
    field: Choice,
    options: Vec<(String, String)>,
    anchor: AnchorSlot,
}

pub(crate) struct Automations {
    service: Arc<Mutex<Service>>,
    database: Option<Database>,
    cwd: String,
    projects: Vec<String>,
    appearances: HashMap<String, monocode_core::project::appearance::ProjectAppearance>,
    automations: Vec<Automation>,
    runs: Vec<AutomationRun>,
    selected: Option<String>,
    editor: Option<Editor>,
    loading: bool,
    error: Option<String>,
    saving: bool,
    running: bool,
    gallery: bool,
    category: String,
    templates: Vec<Template>,
    filter: Entity<Input>,
    focus: FocusHandle,
    _filter_subscription: Subscription,
    history: bool,
    advanced: bool,
    confirm_delete: bool,
    folders: Vec<Folder>,
    choice: Option<ChoiceMenu>,
    models: Option<Entity<ModelList>>,
    model_controls: Vec<monocode_core::provider::catalog::ModelControl>,
    project_picker: Option<Entity<ProjectPicker>>,
    picker_subscription: Option<Subscription>,
    project_anchor: AnchorSlot,
    model_anchor: AnchorSlot,
    trigger_anchor: AnchorSlot,
    trigger_menu: Option<Entity<trigger_menu::TriggerMenu>>,
    trigger_subscription: Option<Subscription>,
    trigger_connections: HashMap<String, bool>,
    request_generation: u64,
}
impl EventEmitter<Event> for Automations {}
impl Focusable for Automations {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Automations {
    pub(crate) fn new(
        service: Arc<Mutex<Service>>,
        cwd: String,
        projects: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter =
            cx.new(|cx| Input::field("Filter automations", "Filter automations", content(0.4), cx));
        let subscription = cx.subscribe(&filter, |_, _, _: &input::Changed, cx| cx.notify());
        let mut this = Self {
            service,
            database: None,
            cwd,
            projects,
            appearances: HashMap::new(),
            automations: Vec::new(),
            runs: Vec::new(),
            selected: None,
            editor: None,
            loading: true,
            error: None,
            saving: false,
            running: false,
            gallery: true,
            category: "popular".into(),
            templates: serde_json::from_str(include_str!("automations/templates.json"))
                .unwrap_or_default(),
            filter,
            focus: cx.focus_handle(),
            _filter_subscription: subscription,
            history: false,
            advanced: false,
            confirm_delete: false,
            folders: Vec::new(),
            choice: None,
            models: None,
            model_controls: Vec::new(),
            project_picker: None,
            picker_subscription: None,
            project_anchor: AnchorSlot::default(),
            model_anchor: AnchorSlot::default(),
            trigger_anchor: AnchorSlot::default(),
            trigger_menu: None,
            trigger_subscription: None,
            trigger_connections: HashMap::new(),
            request_generation: 0,
        };
        this.refresh(cx);
        this
    }
    pub(crate) fn set_database(&mut self, database: Database, cx: &mut Context<Self>) {
        self.database = Some(database);
        self.load_folders(cx);
    }
    pub(crate) fn set_appearances(
        &mut self,
        appearances: HashMap<String, monocode_core::project::appearance::ProjectAppearance>,
        cx: &mut Context<Self>,
    ) {
        self.appearances = appearances;
        cx.notify();
    }
    fn project_label(&self, path: &str) -> String {
        self.appearances
            .get(path)
            .map_or_else(|| project_name(path), |appearance| appearance.label(path))
    }
    fn project_mark(&self, path: &str, side: f32) -> AnyElement {
        crate::ui::project_badge::project_icon(
            &self.appearances.get(path).cloned().unwrap_or_default(),
            path,
            side,
        )
    }
    pub(crate) fn set_context(
        &mut self,
        cwd: String,
        projects: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        self.cwd = cwd;
        self.projects = projects;
        self.load_folders(cx);
        cx.notify();
    }
    fn show_gallery(&mut self, cx: &mut Context<Self>) {
        self.editor = None;
        self.selected = None;
        self.runs.clear();
        self.gallery = true;
        self.history = false;
        self.confirm_delete = false;
        self.choice = None;
        self.trigger_menu = None;
        self.models = None;
        self.project_picker = None;
        self.error = None;
        self.request_generation += 1;
        cx.notify();
    }
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        let service = self.service.clone();
        let selected = self.selected.clone();
        let selected_for_work = selected.clone();
        let work = cx.background_executor().spawn(async move {
            let mut service = service
                .lock()
                .map_err(|_| "Automations unavailable".to_string())?;
            let list = service.list()?;
            let runs = selected_for_work
                .as_deref()
                .map(|id| service.runs(id))
                .transpose()?
                .unwrap_or_default();
            Ok::<_, String>((list, runs))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok((list, runs)) => {
                        this.automations = list;
                        if this.selected == selected {
                            this.runs = runs;
                        }
                        if this
                            .selected
                            .as_ref()
                            .is_some_and(|id| !this.automations.iter().any(|a| &a.id == id))
                        {
                            this.selected = None;
                            if !this.editor.as_ref().is_some_and(|e| e.dirty) {
                                this.editor = None;
                                this.gallery = true;
                            }
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn load_folders(&mut self, cx: &mut Context<Self>) {
        let Some(database) = self.database.clone() else {
            return;
        };
        let cwd = self
            .editor
            .as_ref()
            .map(|e| e.draft.cwd.clone())
            .unwrap_or_else(|| self.cwd.clone());
        let expected = cwd.clone();
        let work = cx
            .background_executor()
            .spawn(async move { locked(&database)?.groups(&cwd) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if this
                    .editor
                    .as_ref()
                    .map(|e| e.draft.cwd.as_str())
                    .unwrap_or(&this.cwd)
                    == expected
                {
                    this.folders = result.map(|g| g.folders).unwrap_or_default();
                    cx.notify();
                }
            });
        })
        .detach();
    }
    fn begin(&mut self, template: Option<Template>, cx: &mut Context<Self>) {
        let preferences = cx.try_global::<crate::ui::preferences::Preferences>();
        let kind = preferences
            .and_then(|p| p.project_providers.get(&self.cwd))
            .and_then(|p| p.provider.as_deref())
            .or_else(|| preferences.and_then(|p| p.default_provider.as_deref()))
            .map(Kind::from_id)
            .unwrap_or(Kind::Codex);
        let model = preferences
            .and_then(|p| p.project_providers.get(&self.cwd))
            .and_then(|p| p.models.get(kind.id()))
            .or_else(|| preferences.and_then(|p| p.default_models.get(kind.id())))
            .cloned()
            .unwrap_or_default();
        let mut draft = AutomationUpsert::new(self.cwd.clone(), kind.id().into(), model);
        if let Some(template) = template {
            draft.name = template.name;
            draft.prompt = template.prompt;
            let kind = template.trigger["kind"].as_str().unwrap_or("time");
            let event = template.trigger["event"].as_str().unwrap_or("weekdays");
            let mut trigger = AutomationTrigger::new(kind, event);
            if let Some(value) = template.trigger["time"].as_str() {
                trigger.time = value.into();
            }
            if let Some(value) = template.trigger["dayOfWeek"].as_i64() {
                trigger.day_of_week = value;
            }
            if let Some(value) = template.trigger["minute"].as_i64() {
                trigger.minute = value;
            }
            draft.triggers = Some(vec![trigger]);
        }
        self.selected = None;
        self.runs.clear();
        self.set_editor(draft, false, cx);
    }
    fn select(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(automation) = self.automations.iter().find(|a| a.id == id) else {
            return;
        };
        let draft = AutomationUpsert::from_automation(automation);
        self.selected = Some(id);
        self.runs.clear();
        self.set_editor(draft, true, cx);
        self.refresh(cx);
    }
    fn set_editor(&mut self, draft: AutomationUpsert, saved: bool, cx: &mut Context<Self>) {
        let name = cx.new(|cx| {
            let mut input = Input::automation_name(cx);
            input.set_text(draft.name.clone(), cx);
            input
        });
        let prompt = cx.new(|cx| {
            let mut input = Input::automation_instructions(cx);
            input.set_text(draft.prompt.clone(), cx);
            input
        });
        let completion = cx.new(|cx| {
            let completion = crate::chat::composer::completion::Completion::new(
                prompt.clone(),
                draft.cwd.clone(),
                cx,
            )
            .inline_field();
            if let Some(store) = &self.database {
                completion.with_store(store.clone())
            } else {
                completion
            }
        });
        completion.update(cx, |completion, cx| {
            completion.set_provider(&draft.harness, cx)
        });
        let mut subscriptions = Vec::new();
        for field in [&name, &prompt] {
            subscriptions.push(cx.subscribe(field, |this, _, _: &input::Changed, cx| {
                if let Some(editor) = &mut this.editor {
                    editor.dirty = true;
                }
                cx.notify();
            }));
        }
        self.editor = Some(Editor {
            draft,
            name,
            prompt,
            completion,
            fields: Vec::new(),
            saved,
            dirty: !saved,
            _subscriptions: subscriptions,
        });
        self.rebuild_trigger_fields(cx);
        self.gallery = false;
        self.history = false;
        self.confirm_delete = false;
        self.choice = None;
        self.trigger_menu = None;
        self.models = None;
        self.project_picker = None;
        self.error = None;
        self.load_folders(cx);
        self.load_controls(cx);
        cx.notify();
    }
    fn rebuild_trigger_fields(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        let triggers = editor.draft.triggers.clone().unwrap_or_default();
        editor
            .fields
            .retain(|fields| triggers.iter().any(|t| t.id == fields.id));
        for trigger in triggers {
            if editor.fields.iter().any(|f| f.id == trigger.id) {
                continue;
            }
            let field = |label: &str, placeholder: &str, text: String, cx: &mut Context<Self>| {
                cx.new(|cx| {
                    let mut input = Input::field(
                        label.to_string(),
                        placeholder.to_string(),
                        content(0.35),
                        cx,
                    );
                    input.set_text(text, cx);
                    input
                })
            };
            let time = field("Trigger time", "09:00", trigger.time, cx);
            let repos = field(
                "Trigger repositories",
                "Any repository",
                trigger
                    .repos
                    .iter()
                    .chain(std::iter::once(&trigger.repo))
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", "),
                cx,
            );
            let actor = field("Trigger actor", "Anyone", trigger.actor, cx);
            for input in [&time, &repos, &actor] {
                editor._subscriptions.push(cx.subscribe(
                    input,
                    |this, _, _: &input::Changed, cx| {
                        if let Some(e) = &mut this.editor {
                            e.dirty = true;
                        }
                        cx.notify();
                    },
                ));
            }
            editor.fields.push(TriggerFields {
                id: trigger.id,
                time,
                repos,
                actor,
            });
        }
    }
    fn collected(&self, cx: &App) -> Option<AutomationUpsert> {
        let editor = self.editor.as_ref()?;
        let mut draft = editor.draft.clone();
        draft.name = editor.name.read(cx).content.trim().to_string();
        // Runs later without the picker's index, so save full mention paths.
        draft.prompt = editor
            .completion
            .read(cx)
            .expand_mentions(&editor.prompt.read(cx).content);
        for trigger in draft.triggers.get_or_insert_with(Vec::new) {
            if let Some(fields) = editor.fields.iter().find(|f| f.id == trigger.id) {
                trigger.time = fields.time.read(cx).content.trim().to_string();
                trigger.repo.clear();
                trigger.repos = fields
                    .repos
                    .read(cx)
                    .content
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
                trigger.actor = fields.actor.read(cx).content.trim().to_string();
            }
        }
        draft.sync_triggers(monocode_core::sys::time::now_ms() as i64);
        Some(draft)
    }
    fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let Some(draft) = self.collected(cx) else {
            return;
        };
        self.saving = true;
        self.error = None;
        let service = self.service.clone();
        let work = cx.background_executor().spawn(async move {
            service
                .lock()
                .map_err(|_| "Automations unavailable".to_string())?
                .upsert(draft)
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.saving = false;
                match result {
                    Ok(saved) => {
                        this.selected = Some(saved.id.clone());
                        this.set_editor(AutomationUpsert::from_automation(&saved), true, cx);
                        this.refresh(cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn toggle(&mut self, id: String, enabled: bool, cx: &mut Context<Self>) {
        let service = self.service.clone();
        let work = cx.background_executor().spawn(async move {
            service
                .lock()
                .map_err(|_| "Automations unavailable".to_string())?
                .set_enabled(&id, enabled)
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(saved) => {
                        if let Some(editor) = &mut this.editor
                            && editor.draft.id == saved.id
                            && !editor.dirty
                        {
                            editor.draft.enabled = saved.enabled;
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                this.refresh(cx);
            });
        })
        .detach();
    }
    fn run(&mut self, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        let Some(automation) = self
            .selected
            .as_ref()
            .and_then(|id| self.automations.iter().find(|a| &a.id == id))
            .cloned()
        else {
            return;
        };
        self.running = true;
        let service = self.service.clone();
        let work = cx.background_executor().spawn(async move {
            let run = service
                .lock()
                .map_err(|_| "Automations unavailable".to_string())?
                .run_now(&automation.id, monocode_core::sys::time::now_ms() as i64)?;
            Ok::<_, String>(DueAutomationRun { automation, run })
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.running = false;
                match result {
                    Ok(run) => cx.emit(Event::Launch(Box::new(run))),
                    Err(error) => this.error = Some(error),
                }
                this.refresh(cx);
            });
        })
        .detach();
        cx.notify();
    }
    /// Asks with the standard confirmation before deleting the open automation.
    fn ask_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = window.prompt(
            gpui::PromptLevel::Warning,
            "Delete this automation?",
            Some("Its run history will be deleted too."),
            &["Delete", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if prompt.await == Ok(0) {
                let _ = this.update(cx, |this, cx| this.delete(cx));
            }
        })
        .detach();
    }
    fn delete(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        let service = self.service.clone();
        let work = cx.background_executor().spawn(async move {
            service
                .lock()
                .map_err(|_| "Automations unavailable".to_string())?
                .delete(&id)
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.confirm_delete = false;
                match result {
                    Ok(()) => {
                        this.editor = None;
                        this.selected = None;
                        this.gallery = true;
                        this.runs.clear();
                    }
                    Err(error) => this.error = Some(error),
                }
                this.refresh(cx);
            });
        })
        .detach();
    }
    fn load_controls(&mut self, cx: &mut Context<Self>) {
        self.model_controls.clear();
        self.request_generation += 1;
        let generation = self.request_generation;
        let Some(editor) = &self.editor else {
            return;
        };
        let kind = Kind::from_id(&editor.draft.harness);
        let model = monocode_core::store::native_model(&editor.draft.model, &editor.draft.harness)
            .unwrap_or_default();
        let profile = crate::accounts::get(cx)
            .map(|a| a.read(cx).selected(kind, &editor.draft.cwd))
            .unwrap_or_default();
        let work = cx
            .background_executor()
            .spawn(async move { monocode_core::provider::catalog::models_for(kind, &profile) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if this.request_generation != generation {
                    return;
                }
                if let Ok(models) = result
                    && let Some(model) = models
                        .iter()
                        .find(|m| m.id == model)
                        .or_else(|| models.iter().find(|m| m.default))
                {
                    this.model_controls = model.controls.clone();
                    if !model.efforts.is_empty()
                        && !this.model_controls.iter().any(|c| {
                            matches!(
                                c.id.as_str(),
                                "reasoningEffort" | "reasoning_effort" | "effort" | "reasoning"
                            )
                        })
                    {
                        this.model_controls
                            .push(monocode_core::provider::catalog::ModelControl {
                                id: "reasoningEffort".into(),
                                name: "Reasoning".into(),
                                current: model
                                    .default_effort
                                    .clone()
                                    .unwrap_or_else(|| "medium".into()),
                                choices: model
                                    .efforts
                                    .iter()
                                    .map(|e| (e.clone(), e.clone()))
                                    .collect(),
                            });
                    }
                    if !model.tiers.is_empty()
                        && !this
                            .model_controls
                            .iter()
                            .any(|c| c.id == "serviceTier" || c.id == "service_tier")
                    {
                        let mut choices = vec![("default".into(), "Standard".into())];
                        choices.extend(model.tiers.clone());
                        this.model_controls
                            .push(monocode_core::provider::catalog::ModelControl {
                                id: "serviceTier".into(),
                                name: "Speed".into(),
                                current: "default".into(),
                                choices,
                            });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn choose_model(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &self.editor else {
            return;
        };
        let kind = Kind::from_id(&editor.draft.harness);
        let profile = crate::accounts::get(cx)
            .map(|a| a.read(cx).selected(kind, &editor.draft.cwd))
            .unwrap_or_default();
        let picker = cx.new(|cx| {
            ModelList::new_for(
                kind,
                monocode_core::store::native_model(&editor.draft.model, &editor.draft.harness),
                &editor.draft.cwd,
                profile,
                cx,
            )
        });
        self.picker_subscription = Some(cx.subscribe_in(
            &picker,
            window,
            |this, _, picked: &ModelPicked, window, cx| {
                if let Some(editor) = &mut this.editor {
                    let current = monocode_core::store::native_model(
                        &editor.draft.model,
                        &editor.draft.harness,
                    );
                    let picked_model = picked
                        .1
                        .as_deref()
                        .and_then(|model| monocode_core::store::native_model(model, picked.0.id()));
                    if Kind::from_id(&editor.draft.harness) != picked.0 || current != picked_model {
                        editor.draft.harness = picked.0.id().into();
                        editor.draft.model = picked.1.clone().unwrap_or_default();
                        editor.completion.update(cx, |completion, cx| {
                            completion.set_provider(picked.0.id(), cx)
                        });
                        editor.dirty = true;
                    }
                }
                this.models = None;
                this.load_controls(cx);
                window.focus(&this.focus, cx);
                cx.notify();
            },
        ));
        window.focus(&picker.read(cx).search_focus(cx), cx);
        self.models = Some(picker);
        self.choice = None;
        self.trigger_menu = None;
        cx.notify();
    }
    fn choose_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &self.editor else {
            return;
        };
        let picker = cx.new(|cx| {
            ProjectPicker::new(&self.projects, &editor.draft.cwd, cx)
                .appearance(self.appearances.clone())
        });
        self.picker_subscription = Some(cx.subscribe_in(
            &picker,
            window,
            |this, _, picked: &ProjectPicked, window, cx| {
                this.project_picker = None;
                match picked {
                    ProjectPicked::Project(path) => {
                        if let Some(editor) = &mut this.editor {
                            editor.draft.cwd = path.clone();
                            editor.draft.session_folder_id.clear();
                            editor
                                .completion
                                .update(cx, |completion, cx| completion.set_cwd(path, cx));
                            editor.dirty = true;
                        }
                        this.load_folders(cx);
                        this.load_controls(cx);
                    }
                    ProjectPicked::New => this.browse_project(cx),
                    ProjectPicked::Remote | ProjectPicked::Mono(_) => {}
                }
                window.focus(&this.focus, cx);
                cx.notify();
            },
        ));
        window.focus(&picker.read(cx).search_focus(cx), cx);
        self.project_picker = Some(picker);
        self.choice = None;
        self.trigger_menu = None;
        cx.notify();
    }
    fn browse_project(&mut self, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose project".into()),
        });
        cx.spawn(async move |this, cx| {
            let result = prompt.await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(Ok(Some(paths))) = result
                    && let Some(path) = paths.first()
                {
                    let path = path.to_string_lossy().to_string();
                    if !this.projects.contains(&path) {
                        this.projects.push(path.clone());
                    }
                    if let Some(e) = &mut this.editor {
                        e.draft.cwd = path;
                        e.completion
                            .update(cx, |completion, cx| completion.set_cwd(&e.draft.cwd, cx));
                        e.draft.session_folder_id.clear();
                        e.dirty = true;
                    }
                    this.load_folders(cx);
                    this.load_controls(cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

fn project_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.into())
}
fn button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
) -> gpui::Stateful<gpui::Div> {
    let label: SharedString = label.into();
    div()
        .id(id)
        .role(Role::Button)
        .px(px(12.))
        .h(px(28.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .text_size(px(12.))
        .text_color(content(0.8))
        .border_1()
        .border_color(content(0.15))
        .hover(|s| s.bg(content(0.1)).border_color(content(0.3)))
        .child(label)
}
