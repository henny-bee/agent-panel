//! The slash/@ layer lives beside Input, so typing never rebuilds the transcript
//! or shell. Its popover keeps native text/IME focus in the composer.
mod rank;
mod view;
use crate::{
    chat::conversation::git_actions::GitAction,
    ui::{
        input,
        popover::{Anchor, AnchorSlot, Side, popover},
        theme::*,
    },
};
use gpui::{
    App, Context, Entity, EventEmitter, Focusable, KeyBinding, MouseButton, Role, SharedString,
    Subscription, Window, actions, div, prelude::*, px,
};
use monocode_core::{
    files::ProjectFile,
    provider::skills::DiscoveredSkill,
    session::composer::{self, Token, Trigger},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

actions!(completion, [Accept, Dismiss]);
pub(crate) fn bind(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", Accept, Some("Completion")),
        KeyBinding::new("escape", Dismiss, Some("Completion")),
    ]);
}
pub(crate) struct ModePicked(pub &'static str);
#[derive(Clone)]
struct Choice {
    label: String,
    detail: String,
    insertion: String,
    builtin: Option<&'static str>,
    mode: Option<&'static str>,
    file: bool,
    server: Option<monocode_core::provider::mcp::McpConnection>,
}
impl Choice {
    fn icon(&self) -> (&'static str, gpui::Hsla) {
        if let Some(command) = self.builtin.and_then(input::Command::from_name) {
            return (command.icon(), command.color());
        }
        let (path, dark, light) = match self.builtin {
            Some("btw") => ("command-btw", 0x99f6e4, 0x0f766e),
            Some("compact") => ("command-compact", 0xc4b5fd, 0x6d28d9),
            Some("stage") => (GitAction::Stage.icon(), 0xbbf7d0, 0x15803d),
            Some("commit") => (GitAction::Commit.icon(), 0xfed7aa, 0xc2410c),
            Some("visualize") => return ("command-visualize", accent_color()),
            _ => {
                return (
                    if self.insertion.starts_with("@note/") {
                        "note"
                    } else {
                        "sparkles"
                    },
                    accent_color(),
                );
            }
        };
        let dark_theme = content(1.).l > 0.5;
        let mut color: gpui::Hsla = gpui::rgb(if dark_theme { dark } else { light }).into();
        if dark_theme {
            color.a = 0.9;
        }
        (path, color)
    }
}
pub(crate) struct Completion {
    input: Entity<input::Input>,
    store: Option<crate::runtime::Database>,
    notes: Vec<monocode_core::session::notes::Note>,
    cwd: String,
    token: Option<Token>,
    dismissed: Option<Token>,
    skills: Vec<DiscoveredSkill>,
    provider: monocode_core::provider::Kind,
    native_commands: Vec<monocode_core::provider::NativeCommand>,
    native_loading: bool,
    native_revision: usize,
    mcp: Vec<monocode_core::provider::mcp::McpConnection>,
    selected_mcp: Vec<(String, monocode_core::provider::mcp::McpConnection)>,
    mcp_open: bool,
    disabled_paths: Vec<String>,
    known: std::collections::HashSet<String>,
    files: Vec<ProjectFile>,
    /// `@name` → `@relative/path` for file names unique in the project; the
    /// picker writes the short label and sending expands it back.
    short_mentions: std::collections::HashMap<String, String>,
    choices: Vec<Choice>,
    active: usize,
    scroll: gpui::ScrollHandle,
    loading: bool,
    loaded: Option<Instant>,
    error: Option<String>,
    generation: usize,
    cancel: Arc<AtomicBool>,
    anchor: AnchorSlot,
    composer_anchor: AnchorSlot,
    container_anchor: AnchorSlot,
    inline_field: bool,
    _observe: Subscription,
}
impl EventEmitter<ModePicked> for Completion {}
/// Composer.tsx's `buildMentionIndex`: a name is a label only when it is
/// unique and needs no quoting.
fn short_mentions(files: &[ProjectFile]) -> std::collections::HashMap<String, String> {
    let mut counts = std::collections::HashMap::<&str, usize>::new();
    for file in files {
        *counts.entry(file.name.as_str()).or_default() += 1;
    }
    files
        .iter()
        .filter(|file| {
            file.name != file.relative
                && counts[file.name.as_str()] == 1
                && !file.name.chars().any(char::is_whitespace)
        })
        .map(|file| {
            (
                composer::mention_text(&file.name),
                composer::mention_text(&file.relative),
            )
        })
        .collect()
}
impl Drop for Completion {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl Completion {
    pub(crate) fn new(input: Entity<input::Input>, cwd: String, cx: &mut Context<Self>) -> Self {
        input.update(cx, |input, cx| input.enable_inline_commands(cx));
        let observe = cx.observe(&input, |this, _, cx| this.sync(cx));
        Self {
            input,
            store: None,
            notes: Vec::new(),
            cwd,
            token: None,
            dismissed: None,
            skills: Vec::new(),
            provider: monocode_core::provider::Kind::Codex,
            native_commands: Vec::new(),
            native_loading: false,
            native_revision: 0,
            mcp: Vec::new(),
            selected_mcp: Vec::new(),
            mcp_open: false,
            disabled_paths: Vec::new(),
            known: [
                "/plan",
                "/operator",
                "/orchestrator",
                "/compact",
                "/mcp",
                "/btw",
                "/stage",
                "/commit",
                "/visualize",
            ]
            .map(str::to_owned)
            .into_iter()
            .collect(),
            files: Vec::new(),
            short_mentions: Default::default(),
            choices: Vec::new(),
            active: 0,
            scroll: gpui::ScrollHandle::new(),
            loading: false,
            loaded: None,
            error: None,
            generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            anchor: AnchorSlot::default(),
            composer_anchor: AnchorSlot::default(),
            container_anchor: AnchorSlot::default(),
            inline_field: false,
            _observe: observe,
        }
    }
    pub(crate) fn with_store(mut self, store: crate::runtime::Database) -> Self {
        self.store = Some(store);
        self
    }
    /// Growing document fields anchor suggestions beside the visible caret.
    pub(crate) fn inline_field(mut self) -> Self {
        self.inline_field = true;
        self
    }
    /// Keep composer suggestions inside the conversation, below its tabs.
    pub(crate) fn container_probe(&self) -> impl IntoElement {
        self.container_anchor.layout_probe()
    }
    pub(crate) fn composer_probe(&self) -> impl IntoElement {
        // Include the composer's 1px border instead of measuring its padded
        // text area, so the menu's edges align with the whole input box.
        div()
            .absolute()
            .inset(px(-1.))
            .child(self.composer_anchor.layout_probe())
    }
    pub(crate) fn set_cwd(&mut self, cwd: &str, cx: &mut Context<Self>) {
        if self.cwd == cwd {
            return;
        }
        self.cancel.store(true, Ordering::Relaxed);
        self.generation += 1;
        self.cwd = cwd.into();
        self.skills.clear();
        self.native_commands.clear();
        self.native_loading = false;
        self.native_revision = self.native_revision.wrapping_add(1);
        self.mcp.clear();
        self.selected_mcp.clear();
        self.mcp_open = false;
        self.files.clear();
        self.short_mentions.clear();
        self.notes.clear();
        self.known = [
            "/plan",
            "/operator",
            "/orchestrator",
            "/compact",
            "/mcp",
            "/btw",
            "/stage",
            "/commit",
            "/visualize",
        ]
        .map(str::to_owned)
        .into_iter()
        .collect();
        self.loaded = None;
        self.loading = false;
        self.token = None;
        self.dismissed = None;
        self.sync(cx);
    }
    pub(crate) fn set_provider(&mut self, provider: &str, cx: &mut Context<Self>) {
        let provider = monocode_core::provider::Kind::from_id(provider);
        if self.provider != provider {
            for command in &self.native_commands {
                self.known.remove(&format!("/{}", command.invocation));
            }
            for (token, _) in &self.selected_mcp {
                self.known.remove(token);
            }
            self.provider = provider;
            self.native_commands.clear();
            self.native_loading = false;
            self.native_revision = self.native_revision.wrapping_add(1);
            self.selected_mcp.clear();
            if self.token.is_some() {
                self.load_native(cx);
            }
            self.highlight(cx);
            self.rank();
            cx.notify();
        }
    }
    pub(crate) fn set_native_commands(
        &mut self,
        commands: Vec<monocode_core::provider::NativeCommand>,
        cx: &mut Context<Self>,
    ) {
        self.native_revision = self.native_revision.wrapping_add(1);
        for command in &self.native_commands {
            self.known.remove(&format!("/{}", command.invocation));
        }
        self.native_commands = commands
            .into_iter()
            .filter(|command| command.source == self.provider)
            .collect();
        self.known.extend(
            self.native_commands
                .iter()
                .map(|command| format!("/{}", command.invocation)),
        );
        self.highlight(cx);
        self.rank();
        cx.notify();
    }
    /// A skill or provider command already named `/{name}`, which wins over
    /// MonoCode's own command of the same name.
    pub(crate) fn has_user_command(&self, name: &str) -> bool {
        self.skills
            .iter()
            .map(|skill| skill.name.as_str())
            .chain(self.native_commands.iter().map(|c| c.invocation.as_str()))
            .any(|command| command.trim_start_matches('/').eq_ignore_ascii_case(name))
    }
    pub(crate) fn open_mcp(&mut self, cx: &mut Context<Self>) {
        let content = &self.input.read(cx).content;
        self.token = composer::token_at(content, content.trim_end().len());
        self.dismissed = None;
        self.mcp_open = true;
        if !self.loading {
            self.load(cx);
        }
        self.rank();
        cx.notify();
    }
    pub(crate) fn prepare_prompt(&self, text: &str) -> String {
        if self.cwd.starts_with("remote://") {
            return text.to_owned();
        }
        let text = self.expand_mentions(text);
        let text = monocode_core::provider::native_prompt(self.provider, &text);
        let text = self
            .store
            .as_ref()
            .and_then(|store| crate::runtime::locked(store).ok())
            .and_then(|store| store.apply_notes_to_turn(&text).ok())
            .unwrap_or(text);
        let servers = self
            .selected_mcp
            .iter()
            .filter(|(token, _)| {
                text.match_indices(token.as_str()).any(|(start, _)| {
                    let line_start = text[..start].rfind('\n').map_or(0, |index| index + 1);
                    if text[line_start..start].trim_start().starts_with('>') {
                        return false;
                    }
                    let end = start + token.len();
                    let before = text[..start]
                        .chars()
                        .next_back()
                        .is_none_or(|c| c.is_whitespace() || "([{ ".contains(c));
                    let after = text[end..]
                        .chars()
                        .next()
                        .is_none_or(|c| c.is_whitespace() || ")]}.!?;,".contains(c));
                    before && after
                })
            })
            .map(|(_, server)| {
                format!(
                    "{} ({})",
                    serde_json::to_string(&server.name).unwrap_or_default(),
                    server.provider
                )
            })
            .collect::<Vec<_>>();
        if servers.is_empty() {
            text
        } else {
            format!(
                "MCP context: Use the configured server{} {} when relevant to this request.\n\n{text}",
                if servers.len() == 1 { "" } else { "s" },
                servers.join(", ")
            )
        }
    }
    /// The bare file name when no other project file shares it.
    fn mention_label(&self, file: &ProjectFile) -> String {
        let full = composer::mention_text(&file.relative);
        let short = composer::mention_text(&file.name);
        if self.short_mentions.get(&short) == Some(&full) {
            short
        } else {
            full
        }
    }
    /// Short `@name` labels back to the project-relative paths the prompt
    /// resolves.
    pub(crate) fn expand_mentions(&self, text: &str) -> String {
        if self.short_mentions.is_empty() {
            return text.to_owned();
        }
        let labels = self.short_mentions.keys().cloned().collect();
        let mut text = text.to_owned();
        for range in composer::highlight_ranges(&text, &labels).into_iter().rev() {
            let full = self.short_mentions[&text[range.clone()]].clone();
            text.replace_range(range, &full);
        }
        text
    }
    pub(crate) fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.mcp_open = false;
        self.dismissed = self.token.take();
        cx.notify();
    }
    fn sync(&mut self, cx: &mut Context<Self>) {
        if self.cwd.starts_with("remote://") {
            self.choices.clear();
            self.token = None;
            self.input.update(cx, |input, cx| {
                input.highlight_composer_tokens(Vec::new(), cx)
            });
            return;
        }
        let disabled = cx
            .try_global::<crate::ui::preferences::Preferences>()
            .map(|p| p.disabled_skill_paths.clone())
            .unwrap_or_default();
        if self.disabled_paths != disabled {
            self.disabled_paths = disabled;
            self.loaded = None;
            self.token = None;
        }
        self.highlight(cx);
        let token = self.input.read(cx).completion_token();
        if token.is_some() && token == self.dismissed {
            return;
        }
        // An empty caret token closes the picker and clears an earlier Escape
        // dismissal, so deleting and retyping the same command can reopen it.
        self.dismissed = None;
        let close_mcp = self.mcp_open && token.as_ref().is_none_or(|token| token.query != "mcp");
        if close_mcp {
            self.mcp_open = false;
        }
        if token == self.token {
            if close_mcp {
                cx.notify();
            }
            return;
        }
        self.token = token;
        self.active = 0;
        if self.token.is_some()
            && !self.loading
            && self
                .loaded
                .is_none_or(|at| at.elapsed() > Duration::from_secs(30))
        {
            self.load(cx);
        }
        self.rank();
        cx.notify();
    }
    fn load_native(&mut self, cx: &mut Context<Self>) {
        if self.native_loading
            || !matches!(
                self.provider,
                monocode_core::provider::Kind::Pi | monocode_core::provider::Kind::Omp
            )
        {
            return;
        }
        self.native_loading = true;
        let native_revision = self.native_revision;
        let provider = self.provider;
        let cwd = self.cwd.clone();
        let generation = self.generation;
        let cancel = self.cancel.clone();
        let work = cx.background_executor().spawn(async move {
            if cancel.load(Ordering::Relaxed) {
                return Err("Stopped".into());
            }
            let Some(binary) = monocode_core::provider::resolve(provider) else {
                return Ok(Vec::new());
            };
            monocode_core::provider::discover_commands(
                monocode_core::provider::Request {
                    kind: provider,
                    binary,
                    cwd: cwd.into(),
                    prompt: String::new(),
                    provider_id: None,
                    model: None,
                    settings: Default::default(),
                },
                cancel,
            )
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || this.provider != provider {
                    return;
                }
                this.native_loading = false;
                if this.native_revision != native_revision {
                    return;
                }
                match result {
                    Ok(commands) => this.set_native_commands(commands, cx),
                    Err(error) => {
                        this.error = Some(error);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
    fn load(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        self.cancel = Arc::new(AtomicBool::new(false));
        self.load_native(cx);
        let cancel = self.cancel.clone();
        let cwd = self.cwd.clone();
        let generation = self.generation;
        let disabled = self.disabled_paths.clone();
        let store = self.store.clone();
        let work = cx.background_executor().spawn(async move {
            let notes = store
                .and_then(|s| {
                    crate::runtime::locked(&s)
                        .ok()
                        .and_then(|s| s.list_notes().ok())
                })
                .unwrap_or_default();
            let skills = monocode_core::provider::skills::list_skills(cwd.clone(), Some(disabled))
                .unwrap_or_default();
            let files =
                monocode_core::files::list_project_files_sync_cancellable(&cwd, Some(&cancel));
            let mcp = monocode_core::provider::mcp::mcp_discover(cwd).unwrap_or_default();
            (skills, files, notes, mcp)
        });
        cx.spawn(async move |this, cx| {
            let (skills, files, notes, mcp) = work.await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.generation {
                    return;
                }
                this.loading = false;
                this.loaded = Some(Instant::now());
                this.skills = skills;
                this.mcp = mcp;
                this.notes = notes;
                match files {
                    Ok(files) => this.files = files,
                    Err(error) => this.error = Some(error),
                }
                this.short_mentions = short_mentions(&this.files);
                this.known = [
                    "/plan",
                    "/operator",
                    "/orchestrator",
                    "/compact",
                    "/mcp",
                    "/btw",
                    "/stage",
                    "/commit",
                    "/visualize",
                ]
                .map(str::to_owned)
                .into_iter()
                .chain(
                    this.native_commands
                        .iter()
                        .map(|c| format!("/{}", c.invocation)),
                )
                .chain(this.selected_mcp.iter().map(|(token, _)| token.clone()))
                .chain(this.skills.iter().map(|s| format!("/{}", s.name)))
                .chain(
                    this.files
                        .iter()
                        .map(|f| composer::mention_text(&f.relative)),
                )
                .chain(this.short_mentions.keys().cloned())
                .chain(this.notes.iter().map(|n| format!("@note/{}", n.slug)))
                .collect();
                this.highlight(cx);
                this.rank();
                cx.notify();
            });
        })
        .detach();
    }
    fn highlight(&self, cx: &mut Context<Self>) {
        let ranges = composer::highlight_ranges(&self.input.read(cx).content, &self.known);
        self.input
            .update(cx, |input, cx| input.highlight_composer_tokens(ranges, cx));
    }
    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if !self.choices.is_empty() {
            self.active =
                (self.active as isize + delta).rem_euclid(self.choices.len() as isize) as usize;
            self.scroll.scroll_to_item(self.active);
            cx.notify();
        }
    }
}
#[cfg(test)]
mod tests;
