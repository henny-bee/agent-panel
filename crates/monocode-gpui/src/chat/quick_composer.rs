//! QuickComposer.tsx: the Spotlight-style panel opened by the global shortcut
//! and the menu bar. It floats over other apps without activating MonoCode,
//! keeps its draft across a dismiss, and opens its pickers below the toolbar
//! so the prompt never moves.
use crate::{
    chat::composer::pickers::{ModelList, ModelPicked, model_display_name, model_name},
    git::picker::{GitPicker, Mode as GitMode, Picked as GitPicked},
    ui::{
        input::{self, Command, Input},
        popover::{Anchor, AnchorSlot, Side, popover},
        theme::*,
    },
};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, ExternalPaths, FocusHandle, Focusable,
    MouseButton, Render, Role, SharedString, Subscription, Task, Window, WindowHandle, div,
    prelude::*, px,
};
use monocode_core::{
    project::appearance::ProjectAppearance, provider::Kind, session::agent::AccessMode,
    vcs::git::Snapshot,
};
use std::{collections::HashMap, path::PathBuf};
mod attachments;
#[cfg(all(target_os = "macos", not(test)))]
mod hotkey;
mod lists;
mod view;

pub(crate) const WIDTH: f32 = 680.;
pub(crate) const INITIAL_HEIGHT: f32 = 128.;
/// Down from the top of the screen's work area, like Spotlight.
pub(crate) const TOP_FRACTION: f32 = 0.22;
const MAX_HEIGHT: f32 = 520.;
/// Must match the window's corners so the blur and border share one outline.
const RADIUS: f32 = 16.;
/// Tallest the prompt grows before it scrolls.
const PROMPT_MAX_HEIGHT: f32 = 220.;
/// useQuickPickerMotion: the card's height tween, and the picker's fade-in.
const PICKER_RESIZE: f32 = 0.24;
const PICKER_FADE_DELAY: f32 = 0.04;
const PICKER_FADE: f32 = 0.18;
/// Commands the floating composer offers after a leading `/`.
const COMMANDS: [(Command, &str); 4] = [
    (
        Command::Plan,
        "Create a reviewable implementation plan before changing files.",
    ),
    (
        Command::Operator,
        "Give this thread access to MCode sessions, folders, and notes.",
    ),
    (Command::Orchestrator, "Plan and coordinate agent work."),
    (
        Command::Draft,
        "Save this message without starting the agent.",
    ),
];
/// RUNTIME_MODES, with their icons and hints.
const PERMISSIONS: [(AccessMode, &str, &str); 4] = [
    (
        AccessMode::Supervised,
        "lock",
        "Ask before commands and file changes.",
    ),
    (
        AccessMode::AutoAcceptEdits,
        "pencil",
        "Auto-approve edits, ask before other actions.",
    ),
    (
        AccessMode::Auto,
        "sparkles",
        "An AI reviewer can approve or deny actions.",
    ),
    (
        AccessMode::FullAccess,
        "shield",
        "Allow commands, edits, and supported MCP confirmations in non-plan turns without prompts.",
    ),
];

#[derive(Clone)]
pub(crate) struct Launch {
    pub cwd: String,
    pub kind: Kind,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub fast: bool,
    pub access: AccessMode,
    pub prompt: String,
    pub attachments: Vec<PathBuf>,
    pub staged_attachments: Vec<PathBuf>,
    pub new_worktree: bool,
    pub worktree_base: Option<String>,
    pub worktree_cwd: Option<String>,
    pub foreground: bool,
}
/// Pasted images wait here until the launch creates their conversation, which
/// then adopts them into its own folder.
pub(crate) fn paste_staging_dir() -> PathBuf {
    std::env::temp_dir().join("monocode-quick-composer")
}
impl Launch {
    pub(crate) fn choice(
        cwd: String,
        kind: Kind,
        model: Option<String>,
        access: AccessMode,
    ) -> Self {
        Self {
            cwd,
            kind,
            model,
            effort: None,
            fast: false,
            access,
            prompt: String::new(),
            attachments: Vec::new(),
            staged_attachments: Vec::new(),
            new_worktree: false,
            worktree_base: None,
            worktree_cwd: None,
            foreground: false,
        }
    }
}
#[derive(Default)]
pub(crate) struct Controller {
    pub window: Option<WindowHandle<QuickComposer>>,
    pub last: Option<Launch>,
    pub error: Option<String>,
    pub task: Option<Task<()>>,
    pub events: Option<Subscription>,
    config: String,
    #[cfg(all(target_os = "macos", not(test)))]
    registration: Option<hotkey::Registration>,
}
impl Controller {
    pub fn configure(
        &mut self,
        prefs: &crate::ui::preferences::Preferences,
    ) -> Option<async_channel::Receiver<()>> {
        let shortcut = match prefs.keybindings.get("App: Quick Composer") {
            Some(Some(v)) => v.clone(),
            Some(None) => String::new(),
            None => "cmd-shift-space".into(),
        };
        let config = format!("{}:{shortcut}", prefs.quick_composer);
        if self.config == config {
            return None;
        }
        self.config = config;
        self.error = None;
        self.task = None;
        #[cfg(all(target_os = "macos", not(test)))]
        {
            self.registration = None;
            if prefs.quick_composer && !shortcut.is_empty() {
                match hotkey::Registration::new(&shortcut) {
                    Ok((registration, rx)) => {
                        self.registration = Some(registration);
                        return Some(rx);
                    }
                    Err(e) => self.error = Some(e),
                }
            }
        }
        None
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Picker {
    Project,
    Model,
    Permissions,
    Attachments,
    Commands,
    Workspace,
    Branch,
}

/// quickWorkspace.ts: the working copy a launch starts in.
#[derive(Clone, Default)]
struct Workspace {
    new_worktree: bool,
    base: Option<String>,
    /// An existing linked worktree, rather than the project folder.
    tree: Option<String>,
}

pub(crate) struct QuickComposer {
    input: Entity<Input>,
    query: Entity<Input>,
    projects: Vec<String>,
    appearance: HashMap<String, ProjectAppearance>,
    cwd: String,
    kind: Kind,
    model: Option<String>,
    effort: Option<String>,
    fast: bool,
    access: AccessMode,
    workspace: Workspace,
    git: Option<Snapshot>,
    git_settled: bool,
    git_epoch: usize,
    attachments: Vec<PathBuf>,
    staged_attachments: Vec<PathBuf>,
    photos: HashMap<PathBuf, Entity<crate::chat::transcript::attachments::Photo>>,
    attaching: bool,
    dragging: bool,
    picker: Option<Picker>,
    highlight: usize,
    models: Option<Entity<ModelList>>,
    git_picker: Option<Entity<GitPicker>>,
    permissions_focus: FocusHandle,
    error: Option<String>,
    busy: bool,
    shown: bool,
    card: AnchorSlot,
    /// useQuickPickerMotion: the card's painted height, and the picker and
    /// start of the tween that carries it to the newly measured height.
    height: f32,
    motion_picker: Option<Picker>,
    motion: Option<(std::time::Instant, f32)>,
    plus: AnchorSlot,
    effort_track: AnchorSlot,
    effort_drag: bool,
    _subscriptions: Vec<Subscription>,
    picker_events: Option<Subscription>,
}
impl EventEmitter<Launch> for QuickComposer {}
impl QuickComposer {
    pub fn new(
        projects: Vec<String>,
        appearance: HashMap<String, ProjectAppearance>,
        choice: Launch,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            let mut input = Input::new(cx);
            input.set_font_size(16.);
            input.set_line_height(24.);
            input.set_min_height(48.);
            input.set_max_height(PROMPT_MAX_HEIGHT);
            input.enable_inline_commands(cx);
            input
        });
        let query =
            cx.new(|cx| input::Input::field("Find a project", "Find a project", content(0.35), cx));
        let subscriptions = vec![
            cx.subscribe(&input, |this, _, _: &input::Submitted, cx| {
                this.launch(false, cx)
            }),
            cx.observe(&input, |this, _, cx| this.sync_commands(cx)),
            cx.subscribe(&query, |this, _, _: &input::Changed, cx| {
                this.highlight = 0;
                cx.notify();
            }),
        ];
        let mut this = Self {
            input,
            query,
            projects,
            appearance,
            cwd: String::new(),
            kind: choice.kind,
            model: None,
            effort: None,
            fast: false,
            access: choice.access,
            workspace: Workspace::default(),
            git: None,
            git_settled: false,
            git_epoch: 0,
            attachments: Vec::new(),
            staged_attachments: Vec::new(),
            photos: HashMap::new(),
            attaching: false,
            dragging: false,
            picker: None,
            highlight: 0,
            models: None,
            git_picker: None,
            permissions_focus: cx.focus_handle(),
            error: None,
            busy: false,
            shown: true,
            card: AnchorSlot::default(),
            height: 0.,
            motion_picker: None,
            motion: None,
            plus: AnchorSlot::default(),
            effort_track: AnchorSlot::default(),
            effort_drag: false,
            _subscriptions: subscriptions,
            picker_events: None,
        };
        this.apply_choice(choice, cx);
        this
    }
    /// Each show re-reads projects and defaults, which can change in the
    /// workspace between shows. The draft survives, like Spotlight's query.
    pub fn reopen(
        &mut self,
        projects: Vec<String>,
        appearance: HashMap<String, ProjectAppearance>,
        choice: Launch,
        cx: &mut Context<Self>,
    ) {
        self.projects = projects;
        self.appearance = appearance;
        let keep = self.projects.contains(&self.cwd);
        let cwd = self.cwd.clone();
        self.apply_choice(choice, cx);
        if keep && cwd != self.cwd {
            self.set_project(cwd, cx);
        }
        self.close_picker_state(cx);
        self.error = None;
        self.shown = true;
        cx.notify();
    }
    pub fn shown(&self) -> bool {
        self.shown
    }
    fn apply_choice(&mut self, choice: Launch, cx: &mut Context<Self>) {
        self.kind = choice.kind;
        self.model = choice.model;
        self.effort = choice
            .effort
            .or_else(|| crate::chat::conversation::preferred_effort(self.kind.id(), cx));
        self.fast = choice.fast;
        if self.cwd != choice.cwd {
            self.cwd = choice.cwd;
            self.workspace = Workspace::default();
            self.load_git(cx);
        } else if self.git.is_none() {
            self.load_git(cx);
        }
        self.update_placeholder(cx);
    }
    fn update_placeholder(&mut self, cx: &mut Context<Self>) {
        let text = if self.cwd.is_empty() {
            "Open a project in MCode first".to_owned()
        } else {
            format!(
                "Start a {} session in {}…",
                self.kind.label(),
                crate::ui::project_badge::project_name(&self.cwd)
            )
        };
        self.input
            .update(cx, |input, cx| input.set_placeholder(text, cx));
    }
    fn set_project(&mut self, path: String, cx: &mut Context<Self>) {
        self.cwd = path;
        self.workspace = Workspace::default();
        let p = cx.global::<crate::ui::preferences::Preferences>();
        let kind = p
            .project_providers
            .get(&self.cwd)
            .and_then(|v| v.provider.as_deref())
            .or(p.default_provider.as_deref())
            .map(Kind::from_id)
            .unwrap_or(self.kind);
        let model = p
            .project_providers
            .get(&self.cwd)
            .and_then(|v| v.models.get(kind.id()))
            .or(p.default_models.get(kind.id()))
            .cloned();
        self.set_model(kind, model, cx);
        self.load_git(cx);
        self.update_placeholder(cx);
    }
    fn set_model(&mut self, kind: Kind, model: Option<String>, cx: &mut Context<Self>) {
        if kind != self.kind {
            self.effort = crate::chat::conversation::preferred_effort(kind.id(), cx);
            self.fast = false;
        }
        self.kind = kind;
        self.model = model;
        if !self.attachments_supported() {
            self.error = None;
        }
        self.update_placeholder(cx);
    }
    fn load_git(&mut self, cx: &mut Context<Self>) {
        self.git_epoch += 1;
        let epoch = self.git_epoch;
        let cwd = self
            .workspace
            .tree
            .clone()
            .unwrap_or_else(|| self.cwd.clone());
        self.git = crate::runtime::git_cache::get(&cwd);
        self.git_settled = false;
        if cwd.is_empty() || cfg!(test) {
            self.git_settled = true;
            return;
        }
        let work = cx
            .background_executor()
            .spawn(async move { crate::runtime::git_cache::load(std::path::Path::new(&cwd)) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if this.git_epoch != epoch {
                    return;
                }
                this.git = result.ok();
                this.git_settled = true;
                cx.notify();
            });
        })
        .detach();
    }
    fn attachments_supported(&self) -> bool {
        self.kind != Kind::Fx
    }
    fn prompt(&self, cx: &App) -> String {
        self.input.read(cx).content.trim().to_owned()
    }
    fn leading(&self, cx: &App) -> Option<Command> {
        Command::leading(&self.input.read(cx).content)
    }
    /// The prompt without a leading mode command; Operator is part of it.
    fn message(&self, cx: &App) -> String {
        let prompt = self.prompt(cx);
        match self.leading(cx) {
            Some(Command::Operator) | None => prompt,
            Some(command) => prompt[command.name().len() + 1..].trim().to_owned(),
        }
    }
    fn can_submit(&self, cx: &App) -> bool {
        (!self.message(cx).is_empty() || !self.attachments.is_empty())
            && !self.cwd.is_empty()
            && !self.busy
            && !self.attaching
            && !matches!(self.picker, Some(Picker::Workspace | Picker::Branch))
            && (self.attachments_supported() || self.attachments.is_empty())
    }
    fn launch(&mut self, foreground: bool, cx: &mut Context<Self>) {
        if self.picker == Some(Picker::Commands) {
            self.choose_command(self.highlight, cx);
            return;
        }
        if self.input.read(cx).is_composing() || !self.can_submit(cx) {
            return;
        }
        let new_worktree = self.workspace.new_worktree;
        let base = self
            .workspace
            .base
            .clone()
            .or_else(|| self.git.as_ref().and_then(|g| g.current.clone()))
            .unwrap_or_else(|| "HEAD".into());
        cx.emit(Launch {
            cwd: self.cwd.clone(),
            kind: self.kind,
            model: self.model.clone(),
            effort: self.effort.clone(),
            fast: self.fast,
            access: self.access,
            prompt: self.prompt(cx),
            attachments: std::mem::take(&mut self.attachments),
            staged_attachments: std::mem::take(&mut self.staged_attachments),
            new_worktree,
            worktree_base: new_worktree.then_some(base),
            worktree_cwd: self
                .workspace
                .tree
                .clone()
                .filter(|_| !new_worktree)
                .filter(|tree| {
                    monocode_core::files::path_key(tree)
                        != monocode_core::files::path_key(&self.cwd)
                }),
            foreground,
        });
        self.input.update(cx, |input, cx| {
            input.take(cx);
        });
        self.close_picker_state(cx);
        self.error = None;
        cx.notify();
    }
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.input.focus_handle(cx), cx);
        window.activate_window();
    }
    pub fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_picker_state(cx);
        self.shown = false;
        #[cfg(all(target_os = "macos", not(test)))]
        crate::ui::traffic_lights::hide(window);
        #[cfg(any(not(target_os = "macos"), test))]
        window.remove_window();
        cx.notify();
    }
    fn close_picker_state(&mut self, _cx: &mut Context<Self>) {
        self.picker = None;
        self.models = None;
        self.git_picker = None;
        self.picker_events = None;
        self.effort_drag = false;
    }
    fn close_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_picker_state(cx);
        window.focus(&self.input.focus_handle(cx), cx);
        cx.notify();
    }
    fn open(&mut self, picker: Picker, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker == Some(picker) {
            self.close_picker(window, cx);
            return;
        }
        self.close_picker_state(cx);
        self.picker = Some(picker);
        match picker {
            Picker::Project => {
                self.query.update(cx, |q, cx| {
                    q.take(cx);
                });
                self.highlight = self
                    .projects
                    .iter()
                    .position(|p| *p == self.cwd)
                    .unwrap_or(0);
                window.focus(&self.query.focus_handle(cx), cx);
            }
            Picker::Model => {
                let profile = crate::accounts::get(cx)
                    .map(|a| a.read(cx).selected(self.kind, &self.cwd))
                    .unwrap_or_default();
                let hidden = cx
                    .global::<crate::ui::preferences::Preferences>()
                    .hidden_for(&self.cwd);
                let list = cx.new(|cx| {
                    ModelList::new_for(self.kind, self.model.clone(), &self.cwd, profile, cx)
                        .hide_providers(hidden)
                        .quick(cx)
                });
                self.picker_events =
                    Some(
                        cx.subscribe_in(&list, window, |this, _, picked: &ModelPicked, _, cx| {
                            // As Tauri's selector: picking keeps it open, so the
                            // effort beneath can follow the new model.
                            this.set_model(picked.0, picked.1.clone(), cx);
                            if let Some(list) = this.models.clone() {
                                let (kind, model) = (this.kind, this.model.clone());
                                list.update(cx, |list, cx| list.set_current(kind, model, cx));
                            }
                            cx.notify();
                        }),
                    );
                window.focus(&list.read(cx).search_focus(cx), cx);
                self.models = Some(list);
            }
            Picker::Permissions => {
                self.highlight = PERMISSIONS
                    .iter()
                    .position(|(mode, ..)| *mode == self.access)
                    .unwrap_or(0);
                window.focus(&self.permissions_focus, cx);
            }
            Picker::Workspace | Picker::Branch => {
                let new = self.workspace.new_worktree;
                let (mode, selected) = match picker {
                    Picker::Workspace => (GitMode::Worktree, self.workspace.tree.clone()),
                    _ if new => (GitMode::Base, self.workspace.base.clone()),
                    _ => (GitMode::Branch, None),
                };
                let cwd = self
                    .workspace
                    .tree
                    .clone()
                    .unwrap_or_else(|| self.cwd.clone());
                let picker_view = cx.new(|cx| {
                    let picker = GitPicker::new(cwd.into(), mode, selected, cx);
                    if mode == GitMode::Worktree {
                        picker.existing_worktrees()
                    } else {
                        picker
                    }
                });
                self.picker_events = Some(cx.subscribe_in(
                    &picker_view,
                    window,
                    |this, _, picked: &GitPicked, window, cx| {
                        match picked {
                            GitPicked::Changed => this.load_git(cx),
                            GitPicked::Base(base) => this.workspace.base = Some(base.clone()),
                            GitPicked::Worktree(path) => {
                                this.workspace = Workspace {
                                    tree: Some(path.clone()),
                                    ..Default::default()
                                };
                                this.load_git(cx);
                            }
                            GitPicked::Dismissed => {}
                        }
                        this.close_picker(window, cx);
                    },
                ));
                window.focus(&picker_view.read(cx).focus(cx), cx);
                self.git_picker = Some(picker_view);
            }
            Picker::Attachments | Picker::Commands => {}
        }
        cx.notify();
    }
    /// A leading `/word` opens the command list; Operator only activates at
    /// the start of a prompt, so every mode does.
    fn sync_commands(&mut self, cx: &mut Context<Self>) {
        let text = self.input.read(cx).content.clone();
        let token = text
            .strip_prefix('/')
            .filter(|rest| !rest.contains(char::is_whitespace));
        let busy = matches!(self.picker, Some(Picker::Workspace | Picker::Branch));
        if token.is_some() && !self.busy && !busy {
            if self.picker != Some(Picker::Commands) {
                self.close_picker_state(cx);
                self.picker = Some(Picker::Commands);
                self.highlight = 0;
            }
            let count = self.command_options(cx).len();
            self.highlight = self.highlight.min(count.saturating_sub(1));
        } else if self.picker == Some(Picker::Commands) {
            self.picker = None;
        }
        cx.notify();
    }
    fn command_options(&self, cx: &App) -> Vec<(Command, &'static str)> {
        let content = self.input.read(cx).content.clone();
        let query = content.strip_prefix('/').unwrap_or("").to_lowercase();
        COMMANDS
            .into_iter()
            .filter(|(command, _)| command.name().contains(&query))
            .collect()
    }
    fn choose_command(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some((command, _)) = self.command_options(cx).get(ix).copied() else {
            return;
        };
        self.input.update(cx, |input, cx| {
            input.set_text(format!("/{} ", command.name()), cx)
        });
        self.picker = None;
        cx.notify();
    }
    fn project_options(&self, cx: &App) -> Vec<String> {
        let query = self.query.read(cx).content.trim().to_lowercase();
        self.projects
            .iter()
            .filter(|path| {
                query.is_empty()
                    || format!("{}\n{path}", self.project_label(path))
                        .to_lowercase()
                        .contains(&query)
            })
            .cloned()
            .collect()
    }
    fn project_label(&self, path: &str) -> String {
        self.appearance.get(path).map_or_else(
            || crate::ui::project_badge::project_name(path),
            |a| a.label(path),
        )
    }
    fn step(&mut self, direction: isize, cx: &mut Context<Self>) {
        let count = match self.picker {
            Some(Picker::Commands) => self.command_options(cx).len(),
            Some(Picker::Project) => self.project_options(cx).len(),
            Some(Picker::Permissions) => PERMISSIONS.len(),
            _ => return,
        };
        if count > 0 {
            self.highlight =
                (self.highlight as isize + direction).rem_euclid(count as isize) as usize;
            cx.notify();
        }
    }
    fn choose_project(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.project_options(cx).into_iter().nth(ix) {
            self.set_project(path, cx);
        }
        self.close_picker(window, cx);
    }
    fn choose_permission(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((mode, ..)) = PERMISSIONS.get(ix) {
            self.access = *mode;
        }
        self.close_picker(window, cx);
    }
    fn choose_workspace(&mut self, new: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace = Workspace {
            new_worktree: new,
            base: None,
            tree: None,
        };
        self.load_git(cx);
        self.close_picker(window, cx);
    }

    /// Returns the card's height this frame, the picker's fade, and whether
    /// the card is mid-tween and must be held to that height.
    fn picker_motion(&mut self, window: &mut Window, cx: &App) -> (f32, f32, bool) {
        // The probe sits inside the 1px border on each side.
        let natural = self.card.get().map(|b| f32::from(b.size.height) + 2.);
        if self.motion_picker != self.picker {
            self.motion_picker = self.picker;
            self.motion = (!cx.reduce_motion() && self.height > 0.)
                .then(|| (std::time::Instant::now(), self.height));
        }
        let (Some((start, from)), Some(to)) = (self.motion, natural) else {
            self.motion = None;
            self.height = natural.unwrap_or(0.);
            return (self.height, 1., false);
        };
        let elapsed = start.elapsed().as_secs_f32();
        if elapsed >= PICKER_RESIZE {
            self.motion = None;
            self.height = to;
            return (to, 1., false);
        }
        window.request_animation_frame();
        let grow = crate::ui::motion::reveal::bezier(elapsed / PICKER_RESIZE, 0.22, 1., 0.36, 1.);
        // `ease-out`, held at zero through the 40ms delay.
        let fade = crate::ui::motion::reveal::bezier(
            ((elapsed - PICKER_FADE_DELAY) / PICKER_FADE).clamp(0., 1.),
            0.,
            0.,
            0.58,
            1.,
        );
        self.height = from + (to - from) * grow;
        (self.height, fade, true)
    }

    fn fit(&mut self, height: f32, window: &mut Window, cx: &App) {
        if height <= 0. {
            return;
        }
        let height = height.clamp(1., MAX_HEIGHT);
        #[cfg(all(target_os = "macos", not(test)))]
        crate::ui::traffic_lights::fit_height(window, cx, height as f64);
        #[cfg(any(not(target_os = "macos"), test))]
        let _ = cx;
        #[cfg(any(not(target_os = "macos"), test))]
        {
            let size = window.bounds().size;
            if (f32::from(size.height) - height).abs() > 0.5 {
                window.resize(gpui::size(size.width, px(height)));
            }
        }
    }
}
impl Focusable for QuickComposer {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

fn kbd(text: &'static str) -> impl IntoElement {
    div()
        .px_1()
        .rounded(px(4.))
        .border_1()
        .border_color(content(0.12))
        .text_size(px(10.))
        .text_color(content(0.55))
        .child(text)
}
/// Tailwind's amber-400, which marks full access and fast mode.
fn amber(alpha: f32) -> gpui::Hsla {
    gpui::Hsla::from(gpui::rgb(0xfbbf24)).opacity(alpha)
}
fn effort_name(value: &str) -> String {
    crate::chat::conversation::effort_name(value)
}
impl QuickComposer {}
#[cfg(test)]
mod tests;
