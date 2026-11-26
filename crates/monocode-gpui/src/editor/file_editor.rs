//! Native FileEditor/FilePane previews. Disk work and syntax parsing stay off
//! the UI thread; drafts, selections and scrolling belong to each live tab.
use crate::{
    chat::transcript::TranscriptView,
    runtime::git_status::{GitChanged, ProjectGit},
    ui::{
        input::{self, Input},
        panel::{icon_action, stroke},
        theme::*,
    },
};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Image, ImageCacheError,
    ImageFormat, ImageSource, KeyBinding, ObjectFit, PromptLevel, RenderImage, Role, SharedString,
    Subscription, Task, Window, actions, div, img, prelude::*, px, rgb,
};
use monocode_core::files::document::{self, Document, TextDocument};
use std::{path::PathBuf, sync::Arc, time::Duration};

mod chrome;
mod diff;
mod media;
mod search;
mod selection;
mod view;

actions!(
    file_editor,
    [
        ToggleAutosave,
        ToggleWordWrap,
        ReloadFile,
        Save,
        Find,
        Replace,
        FindNext,
        FindPrevious,
        DismissSearch,
        MatchCase,
        WholeWord,
        Regex,
        Format,
        PreviousChange,
        NextChange,
        RevertChange
    ]
);
pub(crate) fn bind(cx: &mut App) {
    let m = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    cx.bind_keys([
        KeyBinding::new(&format!("{m}-s"), Save, Some("FileEditor")),
        KeyBinding::new(&format!("{m}-f"), Find, Some("FileEditor")),
        KeyBinding::new(&format!("{m}-alt-f"), Replace, Some("FileEditor")),
        KeyBinding::new("alt-shift-f", Format, Some("FileEditor")),
        KeyBinding::new("alt-z", ToggleWordWrap, Some("FileEditor")),
        KeyBinding::new("alt-c", MatchCase, Some("FileEditor")),
        KeyBinding::new("alt-w", WholeWord, Some("FileEditor")),
        KeyBinding::new("alt-r", Regex, Some("FileEditor")),
        KeyBinding::new("f3", FindNext, Some("FileEditor")),
        KeyBinding::new("shift-f3", FindPrevious, Some("FileEditor")),
        KeyBinding::new("alt-up", PreviousChange, Some("FileEditor")),
        KeyBinding::new("alt-down", NextChange, Some("FileEditor")),
        KeyBinding::new(&format!("{m}-g"), FindNext, Some("FileEditor")),
        KeyBinding::new(&format!("{m}-shift-g"), FindPrevious, Some("FileEditor")),
        KeyBinding::new("escape", DismissSearch, Some("FileEditor")),
    ]);
}

pub(crate) enum FileEvent {
    Dirty,
    Saved,
    SaveFailed,
    WordWrap(bool),
    AddToChat(String),
}

pub(crate) struct FileEditor {
    pub path: String,
    git: Entity<ProjectGit>,
    cwd: String,
    git_original: Option<String>,
    git_staged: bool,
    git_line_ending: &'static str,
    git_task: Option<Task<()>>,
    analysis: Option<Task<()>>,
    /// The Tree-sitter parser and last tree, reused by each incremental pass.
    syntax: Arc<std::sync::Mutex<Option<crate::editor::syntax::Session>>>,
    focus: FocusHandle,
    pub input: Entity<Input>,
    selection: selection::Selection,
    search: Entity<Input>,
    search_open: bool,
    replacement: Entity<Input>,
    replace_open: bool,
    match_case: bool,
    whole_word: bool,
    regex: bool,
    search_invalid: bool,
    search_task: Option<Task<()>>,
    matches: Vec<crate::editor::analysis::SearchMatch>,
    autosave: Option<Task<()>>,
    formatting: Option<Task<()>>,
    pub show_diff: bool,
    change_index: usize,
    match_index: usize,
    document: Option<TextDocument>,
    image: Option<Arc<Image>>,
    media: Option<Entity<media::MediaPreview>>,
    media_events: Option<Subscription>,
    /// Each edit renders a fresh sprite; the one it replaces is given back.
    svg_preview: Option<Arc<RenderImage>>,
    svg_failed: bool,
    svg_render: Option<Task<()>>,
    unsupported: Option<String>,
    error: Option<String>,
    disk_changed: bool,
    /// The file's size and modified time at the last poll that reread it.
    disk_stamp: Option<(u64, Option<std::time::SystemTime>)>,
    loading: bool,
    pub saving: bool,
    pub dirty: bool,
    visible: bool,
    preview: bool,
    markdown: Option<Entity<TranscriptView>>,
    markdown_source: SharedString,
    navigation: Option<(usize, usize)>,
    _subscriptions: Vec<Subscription>,
    _poll: Task<()>,
}
impl EventEmitter<FileEvent> for FileEditor {}
impl Focusable for FileEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl FileEditor {
    pub(crate) fn new(path: String, git: Entity<ProjectGit>, cx: &mut Context<Self>) -> Self {
        let mut this = Self::new_deferred(path, git, cx);
        this.load(false, cx);
        this
    }
    pub(crate) fn new_deferred(
        path: String,
        git: Entity<ProjectGit>,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(Input::editor);
        let search = cx.new(|cx| {
            let mut field = Input::field("Find", "Find", content(0.35), cx);
            field.set_font_size(12.);
            field
        });
        let replacement = cx.new(|cx| {
            let mut field = Input::field("Replace", "Replace", content(0.35), cx);
            field.set_font_size(12.);
            field
        });
        input.update(cx, |input, _| input.set_editor_path(path.clone()));
        let subscriptions = vec![
            cx.subscribe(&input, |this, _, _: &input::Changed, cx| this.edited(cx)),
            cx.subscribe(&input, |this, _, action: &input::CodeAction, cx| {
                let (index, stage) = match action {
                    input::CodeAction::Stage(index) => (*index, true),
                    input::CodeAction::Revert(index) => (*index, false),
                };
                this.change_index = index;
                this.apply_change(stage, cx);
            }),
            cx.observe(&input, |this, _, cx| {
                this.observe_code_selection(cx);
                cx.notify();
            }),
            cx.subscribe(&input, |this, _, event: &input::Rejected, cx| {
                this.error = Some(event.0.into());
                cx.notify();
            }),
            cx.subscribe(&search, |this, _, _: &input::Changed, cx| {
                this.find_matches(cx)
            }),
            cx.subscribe(&search, |this, _, _: &input::Submitted, cx| {
                this.step_match(false, cx)
            }),
            cx.subscribe(&replacement, |this, _, _: &input::Changed, cx| {
                this.find_matches(cx)
            }),
            cx.subscribe(&replacement, |this, _, _: &input::Submitted, cx| {
                this.replace_matches(false, cx)
            }),
            cx.subscribe(&git, |this, _, _: &GitChanged, cx| {
                if this.visible {
                    this.refresh_git(cx);
                }
            }),
            cx.observe_global::<crate::ui::preferences::Preferences>(|this, cx| {
                let wrap = cx
                    .global::<crate::ui::preferences::Preferences>()
                    .file_word_wrap;
                this.input
                    .update(cx, |input, cx| input.set_word_wrap(wrap, cx));
                this.schedule_autosave(cx);
                cx.notify();
            }),
        ];
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                if this
                    .update(cx, |this, cx| {
                        if this.visible && !this.saving {
                            this.poll_disk(cx);
                            // ProjectGit announces index and HEAD changes in
                            // its own checkout (GitChanged above).
                            if this.cwd != this.git.read(cx).project() {
                                this.refresh_git(cx);
                            }
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        cx.on_release(|this, cx| {
            if let Some(image) = this.svg_preview.take() {
                cx.drop_image(image, None);
            }
        })
        .detach();
        Self {
            path,
            cwd: git.read(cx).project().to_owned(),
            git,
            git_original: None,
            git_staged: false,
            git_line_ending: "\n",
            git_task: None,
            analysis: None,
            syntax: Default::default(),
            focus: cx.focus_handle(),
            input,
            selection: selection::Selection::default(),
            search,
            search_open: false,
            replacement,
            replace_open: false,
            match_case: false,
            whole_word: false,
            regex: false,
            search_invalid: false,
            search_task: None,
            autosave: None,
            formatting: None,
            show_diff: false,
            change_index: 0,
            matches: Vec::new(),
            match_index: 0,
            document: None,
            image: None,
            media: None,
            media_events: None,
            svg_preview: None,
            svg_failed: false,
            svg_render: None,
            unsupported: None,
            error: None,
            disk_changed: false,
            disk_stamp: None,
            loading: false,
            saving: false,
            dirty: false,
            visible: false,
            preview: false,
            markdown: None,
            markdown_source: "".into(),
            navigation: None,
            _subscriptions: subscriptions,
            _poll: poll,
        }
    }
    pub(crate) fn navigate(&mut self, line: usize, column: usize, cx: &mut Context<Self>) {
        self.navigation = Some((line, column));
        self.apply_navigation(cx);
    }
    fn apply_navigation(&mut self, cx: &mut Context<Self>) {
        if self.document.is_none() {
            return;
        }
        let Some((line, column)) = self.navigation.take() else {
            return;
        };
        self.preview = false;
        self.input.update(cx, |input, cx| {
            let mut start = 0;
            for _ in 1..line.min(input.content.len() + 1) {
                if let Some(n) = input.content[start..].find('\n') {
                    start += n + 1;
                } else {
                    start = input.content.len();
                    break;
                }
            }
            let end = input.content[start..]
                .find('\n')
                .map_or(input.content.len(), |n| start + n);
            let at = start
                + input.content[start..end]
                    .char_indices()
                    .nth(column.saturating_sub(1))
                    .map_or(end - start, |(at, _)| at);
            input.select_range(at..at, cx);
        });
        cx.notify();
    }
    pub(crate) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        let was = self.visible;
        self.visible = visible;
        if !visible {
            self.selection.dismiss();
        }
        if let Some(media) = &self.media {
            media.update(cx, |m, _| m.set_visible(visible));
        }
        if visible && !was {
            self.load(false, cx);
            self.refresh_git(cx);
        }
    }
    fn is_markdown(&self) -> bool {
        matches!(
            PathBuf::from(&self.path)
                .extension()
                .and_then(|s| s.to_str()),
            Some("md" | "mdx" | "markdown")
        )
    }
    fn load(&mut self, force: bool, cx: &mut Context<Self>) {
        if self.loading || self.saving {
            return;
        }
        self.loading = true;
        let path = PathBuf::from(&self.path);
        let work = cx
            .background_executor()
            .spawn(async move { document::read_document(&path).and_then(media::prepare_document) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(Document::Text(doc)) => {
                        if !force
                            && this
                                .document
                                .as_ref()
                                .is_some_and(|old| old.disk == doc.disk)
                        {
                            return;
                        }
                        if this.dirty && !force {
                            this.disk_changed = true;
                            cx.notify();
                            return;
                        }
                        this.error = None;
                        this.disk_changed = false;
                        this.image = None;
                        this.media = None;
                        this.unsupported = None;
                        let first = this.document.is_none();
                        let text = doc.text.clone();
                        this.document = Some(doc);
                        this.input.update(cx, |input, cx| {
                            input.reload_file(text, !first && !force, cx);
                        });
                        this.dirty = false;
                        if first && !this.show_diff && (this.is_markdown() || this.is_svg()) {
                            this.preview = cx
                                .try_global::<crate::ui::preferences::Preferences>()
                                .is_none_or(|prefs| prefs.file_view_mode != "source");
                        }
                        this.apply_navigation(cx);
                        this.update_preview(cx);
                        this.schedule_analysis(cx);
                        cx.emit(FileEvent::Dirty);
                    }
                    Ok(Document::Image {
                        bytes,
                        format,
                        width,
                        height,
                        original_size,
                    }) => {
                        if this.dirty && !force {
                            this.disk_changed = true;
                            cx.notify();
                            return;
                        }
                        let byte_size = original_size;
                        let file_format = if format == "tiff" { "avif" } else { format };
                        let format = match format {
                            "jpeg" => ImageFormat::Jpeg,
                            "webp" => ImageFormat::Webp,
                            "gif" => ImageFormat::Gif,
                            "bmp" => ImageFormat::Bmp,
                            "ico" => ImageFormat::Ico,
                            "tiff" => ImageFormat::Tiff,
                            _ => ImageFormat::Png,
                        };
                        let image = Arc::new(Image::from_bytes(format, bytes));
                        if this
                            .image
                            .as_ref()
                            .is_some_and(|old| old.id() == image.id())
                        {
                            return;
                        }
                        this.media = Some(cx.new(|cx| {
                            media::MediaPreview::image(
                                this.path.clone(),
                                image.clone(),
                                width,
                                height,
                                byte_size,
                                file_format,
                                cx,
                            )
                        }));
                        this.image = Some(image);
                        this.dirty = false;
                        this.disk_changed = false;
                        cx.emit(FileEvent::Dirty);
                        this.document = None;
                        this.unsupported = None;
                        this.error = None;
                    }
                    Ok(Document::Video {
                        size,
                        modified,
                        source,
                    }) => {
                        if this.dirty && !force {
                            this.disk_changed = true;
                            cx.notify();
                            return;
                        }
                        let same = this
                            .media
                            .as_ref()
                            .is_some_and(|m| m.read(cx).same_video(size, modified));
                        if !same || force {
                            this.media = Some(cx.new(|cx| {
                                media::MediaPreview::video(
                                    this.path.clone(),
                                    size,
                                    modified,
                                    source,
                                    cx,
                                )
                            }));
                        }
                        if let Some(media) = &this.media {
                            media.update(cx, |m, _| m.set_visible(this.visible));
                            this.media_events =
                                Some(cx.subscribe(media, |this, _, _: &media::MediaEvent, cx| {
                                    this.load(true, cx)
                                }));
                        }
                        this.document = None;
                        this.image = None;
                        this.unsupported = None;
                        this.error = None;
                        this.dirty = false;
                        this.disk_changed = false;
                        cx.emit(FileEvent::Dirty);
                    }
                    Ok(Document::Unsupported(reason)) => {
                        if this.dirty && !force {
                            this.disk_changed = true;
                            cx.notify();
                            return;
                        }
                        this.media = None;
                        this.unsupported = Some(reason);
                        this.disk_changed = false;
                        this.document = None;
                        this.image = None;
                        this.error = None;
                        this.dirty = false;
                        cx.emit(FileEvent::Dirty);
                    }
                    Err(error) => {
                        this.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn edited(&mut self, cx: &mut Context<Self>) {
        self.selection.dismiss();
        let Some(doc) = &self.document else { return };
        let dirty = self.input.read(cx).content.as_ref() != doc.text;
        if self.dirty != dirty {
            self.dirty = dirty;
            cx.emit(FileEvent::Dirty);
        }
        self.schedule_analysis(cx);
        self.schedule_autosave(cx);
        if self.dirty {
            self.input
                .update(cx, |input, cx| input.schedule_completion(false, cx));
        }
        if self.search_open {
            self.find_matches(cx);
        }
        cx.notify();
    }
    /// Rereads the file only when its size or modified time moved, instead
    /// of reading it whole every poll.
    // ponytail: a same-size write within one mtime tick (2 s on FAT drives)
    // waits for the next change; hash the file if that ever matters.
    fn poll_disk(&mut self, cx: &mut Context<Self>) {
        let path = PathBuf::from(&self.path);
        let stat = cx.background_executor().spawn(async move {
            std::fs::metadata(&path)
                .ok()
                .map(|meta| (meta.len(), meta.modified().ok()))
        });
        cx.spawn(async move |this, cx| {
            let stamp = stat.await;
            let _ = this.update(cx, |this, cx| {
                // A busy editor tries again on the next poll.
                if this.disk_stamp != stamp && !this.loading && !this.saving {
                    this.disk_stamp = stamp;
                    this.load(false, cx);
                }
            });
        })
        .detach();
    }
    fn refresh_git(&mut self, cx: &mut Context<Self>) {
        // A read can outlast the 2 second poll where starting git is slow
        // (Windows). Restarting it then cancels every read before it lands,
        // so a running one finishes and the next poll picks up later changes.
        if self.git_task.is_some() {
            return;
        }
        let cwd = PathBuf::from(&self.cwd);
        let Ok(relative) = PathBuf::from(&self.path)
            .strip_prefix(&cwd)
            .map(|p| p.to_string_lossy().into_owned())
        else {
            return;
        };
        let work = cx.background_executor().spawn(async move {
            let staged = monocode_core::vcs::git::scm::staged_only(&cwd, &relative).ok()?;
            let diff = monocode_core::vcs::git::scm::file_diff(&cwd, &relative, staged).ok()?;
            (!diff.binary && !diff.too_large).then(|| {
                (
                    diff.original.replace("\r\n", "\n").replace('\r', "\n"),
                    staged,
                    if diff.original.contains("\r\n") {
                        "\r\n"
                    } else if diff.original.contains('\r') {
                        "\r"
                    } else {
                        "\n"
                    },
                )
            })
        });
        self.git_task = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.git_task = None;
                let original = result.map(|(original, staged, line_ending)| {
                    this.git_staged = staged;
                    this.git_line_ending = line_ending;
                    original
                });
                if this.git_original != original {
                    this.git_original = original;
                    this.schedule_analysis(cx);
                }
            });
        }));
    }
    fn schedule_analysis(&mut self, cx: &mut Context<Self>) {
        let text = self.input.read(cx).content.clone();
        let original = self.git_original.clone();
        let path = self.path.clone();
        let syntax = crate::editor::syntax::supports(&path).then(|| self.syntax.clone());
        self.analysis = Some(cx.spawn(async move |this, cx| {
            // An incremental parse costs about a frame, so native analysis
            // runs per edit. The QuickJS fallback reparses everything, so it
            // waits for a pause in typing.
            if syntax.is_none() {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
            }
            let source = text.clone();
            let base = original.clone();
            let work = cx.background_executor().spawn(async move {
                match syntax {
                    Some(syntax) => {
                        crate::editor::syntax::analyze(&syntax, &path, &source, base.as_deref())
                    }
                    None => crate::editor::analysis::analyze(&path, &source, base.as_deref()),
                }
            });
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if this.input.read(cx).content == text && this.git_original == original {
                    match result {
                        Ok(analysis) => this
                            .input
                            .update(cx, |input, cx| input.set_editor_analysis(analysis, cx)),
                        Err(error) => eprintln!("monocode: editor analysis failed: {error}"),
                    }
                }
            });
        }));
    }
    fn toggle_word_wrap(&mut self, cx: &mut Context<Self>) {
        let wrap = !self.input.read(cx).word_wrap();
        self.input
            .update(cx, |input, cx| input.set_word_wrap(wrap, cx));
        cx.emit(FileEvent::WordWrap(wrap));
        cx.notify();
    }
    fn schedule_autosave(&mut self, cx: &mut Context<Self>) {
        self.autosave = None;
        if !self.dirty
            || self.disk_changed
            || !cx
                .try_global::<crate::ui::preferences::Preferences>()
                .is_some_and(|p| p.autosave)
        {
            return;
        }
        self.autosave = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let _ = this.update(cx, |this, cx| {
                if this.dirty
                    && !this.disk_changed
                    && cx
                        .try_global::<crate::ui::preferences::Preferences>()
                        .is_some_and(|p| p.autosave)
                {
                    if this.input.read(cx).is_composing() || this.saving {
                        this.schedule_autosave(cx);
                    } else {
                        this.save(cx);
                    }
                }
            });
        }));
    }
    fn format_document(&mut self, cx: &mut Context<Self>) {
        if self.document.is_none() || self.saving || self.input.read(cx).is_composing() {
            return;
        }
        let source = self.input.read(cx).content.clone();
        let at = self.input.read(cx).selection_range().end;
        let path = self.path.clone();
        self.formatting = Some(cx.spawn(async move |this, cx| {
            let text = source.clone();
            let result = cx
                .background_executor()
                .spawn(async move { crate::editor::analysis::format(&path, &text, at) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.input.read(cx).content == source
                    && !this.input.read(cx).is_composing()
                    && let Ok(Some(result)) = result
                    && result.formatted != source.as_ref()
                {
                    this.input.update(cx, |input, cx| {
                        input.replace_bytes(0..source.len(), &result.formatted, result.cursor, cx)
                    });
                }
            });
        }));
    }
    pub(crate) fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving || self.input.read(cx).is_composing() || self.document.is_none() {
            return;
        }
        let source = self.input.read(cx).content.clone();
        let at = self.input.read(cx).selection_range().end;
        let path = self.path.clone();
        let format = cx
            .try_global::<crate::ui::preferences::Preferences>()
            .is_none_or(|p| p.format_on_save);
        self.saving = true;
        self.error = None;
        cx.spawn(async move |this, cx| {
            if format {
                let text = source.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move { crate::editor::analysis::format(&path, &text, at) })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if this.input.read(cx).content == source
                        && !this.input.read(cx).is_composing()
                        && let Ok(Some(result)) = result
                        && result.formatted != source.as_ref()
                    {
                        this.input.update(cx, |input, cx| {
                            input.replace_bytes(
                                0..source.len(),
                                &result.formatted,
                                result.cursor,
                                cx,
                            )
                        });
                    }
                });
            }
            let Ok(Some((path, doc, saved))) = this.update(cx, |this, cx| {
                if this.input.read(cx).is_composing() {
                    this.saving = false;
                    this.schedule_autosave(cx);
                    cx.emit(FileEvent::SaveFailed);
                    return None;
                }
                Some((
                    PathBuf::from(&this.path),
                    this.document.clone()?,
                    this.input.read(cx).content.to_string(),
                ))
            }) else {
                return;
            };
            let text = saved.clone();
            let result = cx
                .background_executor()
                .spawn(async move { document::save_document(&path, &doc, &text) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.saving = false;
                match result {
                    Ok(disk) => {
                        if let Some(doc) = &mut this.document {
                            doc.disk = disk;
                            doc.text = saved;
                        }
                        this.disk_changed = false;
                        this.edited(cx);
                        this.git.update(cx, |git, cx| git.changed(cx));
                        cx.emit(FileEvent::Saved);
                    }
                    Err(error) => {
                        this.error = Some(error);
                        cx.emit(FileEvent::SaveFailed);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.dirty {
            self.load(true, cx);
            return;
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            "Discard your edits and reload this file?",
            None,
            &["Reload", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if matches!(answer.await, Ok(0)) {
                let _ = this.update(cx, |this, cx| this.load(true, cx));
            }
        })
        .detach();
    }
    fn is_svg(&self) -> bool {
        PathBuf::from(&self.path)
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("svg"))
    }
    fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.document.is_none() {
            return;
        }
        self.preview = false;
        if !self.search_open {
            let input = self.input.read(cx);
            let selected = input.content[input.selection_range()].to_owned();
            if !selected.is_empty() && !selected.contains('\n') {
                self.search.update(cx, |i, cx| i.set_text(selected, cx));
            }
        }
        self.search_open = true;
        window.focus(&self.search.focus_handle(cx), cx);
        cx.notify();
    }
    fn find_matches(&mut self, cx: &mut Context<Self>) {
        let source = self.input.read(cx).content.clone();
        let query = self.search.read(cx).content.clone();
        let replacement = self.replacement.read(cx).content.clone();
        let (case, word, regex) = (self.match_case, self.whole_word, self.regex);
        let previous = self
            .matches
            .get(self.match_index)
            .map(|m| m.from)
            .unwrap_or(0);
        self.matches.clear();
        self.search_task = Some(cx.spawn(async move |this, cx| {
            let text = source.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    crate::editor::analysis::search(&text, &query, &replacement, case, word, regex)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.input.read(cx).content != source {
                    return;
                }
                match result {
                    Ok(result) => {
                        this.matches = result.matches;
                        this.search_invalid = result.invalid;
                    }
                    Err(_) => {
                        this.matches.clear();
                        this.search_invalid = true;
                    }
                }
                this.match_index = this
                    .matches
                    .iter()
                    .position(|m| m.from >= previous)
                    .unwrap_or(0);
                let ranges = this.matches.iter().map(|m| m.from..m.to).collect();
                this.input
                    .update(cx, |i, cx| i.set_search_matches(ranges, cx));
                if let Some(m) = this.matches.get(this.match_index) {
                    this.input
                        .update(cx, |i, cx| i.select_range(m.from..m.to, cx));
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn step_match(&mut self, backwards: bool, cx: &mut Context<Self>) {
        if self.matches.is_empty() {
            return;
        }
        self.match_index = (self.match_index as isize + if backwards { -1 } else { 1 })
            .rem_euclid(self.matches.len() as isize) as usize;
        let m = &self.matches[self.match_index];
        self.input
            .update(cx, |i, cx| i.select_range(m.from..m.to, cx));
        cx.notify();
    }
    fn replace_matches(&mut self, all: bool, cx: &mut Context<Self>) {
        if self.input.read(cx).is_composing() || self.matches.is_empty() {
            return;
        }
        let mut text = self.input.read(cx).content.to_string();
        let cursor = if all {
            for m in self.matches.iter().rev() {
                text.replace_range(m.from..m.to, &m.insert);
            }
            0
        } else {
            let m = &self.matches[self.match_index];
            text.replace_range(m.from..m.to, &m.insert);
            m.from + m.insert.len()
        };
        self.input.update(cx, |i, cx| {
            i.replace_bytes(0..i.content.len(), &text, cursor, cx)
        });
    }
    fn step_change(&mut self, backwards: bool, cx: &mut Context<Self>) {
        let changes = &self.input.read(cx).editor_analysis().changes;
        if changes.is_empty() {
            return;
        }
        self.change_index = if backwards {
            self.change_index.saturating_sub(1)
        } else {
            (self.change_index + 1).min(changes.len() - 1)
        };
        let from = changes[self.change_index].from;
        self.input
            .update(cx, |i, cx| i.select_range(from..from, cx));
        cx.notify();
    }
    fn revert_change(&mut self, cx: &mut Context<Self>) {
        self.apply_change(false, cx);
    }
    fn dismiss_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search_open {
            self.search_open = false;
            self.search_task = None;
            self.input
                .update(cx, |i, cx| i.set_search_matches(Vec::new(), cx));
            window.focus(&self.input.focus_handle(cx), cx);
            cx.notify();
        } else {
            cx.propagate();
        }
    }
}
#[cfg(test)]
mod tests;
