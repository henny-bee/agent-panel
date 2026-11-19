//! The desktop Notes split view, backed by the same notes and image assets.
mod preview;
mod view;

use crate::{
    runtime::{Database, locked},
    ui::{
        files::DraggedFile,
        input::{self, Input},
        popover::{Anchor, AnchorSlot, popover},
        theme::*,
    },
};
use gpui::{
    AnyElement, App, ClipboardItem, Context, CursorStyle, Entity, EventEmitter, ExternalPaths,
    FocusHandle, Focusable, FontWeight, MouseButton, Role, Subscription, Task, Window, div,
    prelude::*, px,
};
use monocode_core::session::notes::{self, Note};
use std::{collections::HashMap, path::PathBuf, time::Duration};

pub(crate) enum Event {
    Close,
    OpenSource(i64),
    AddToChat(Note),
}

pub(crate) struct Notes {
    store: Database,
    notes: Vec<Note>,
    selected: Option<String>,
    query: Entity<Input>,
    title: Entity<Input>,
    body: Entity<Input>,
    tag: Entity<Input>,
    project_query: Entity<Input>,
    project_anchor: AnchorSlot,
    project_open: bool,
    cwd: String,
    projects: Vec<String>,
    appearances: HashMap<String, monocode_core::project::appearance::ProjectAppearance>,
    source: bool,
    modes: HashMap<String, bool>,
    document: Option<Entity<preview::Preview>>,
    dirty: bool,
    revision: u64,
    loading: bool,
    error: Option<String>,
    debounce: Option<Task<()>>,
    image_busy: bool,
    image_drag: bool,
    list_width: f32,
    resize: Option<(f32, f32)>,
    blurs_bound: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<Event> for Notes {}
impl Focusable for Notes {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.query.focus_handle(cx)
    }
}

impl Notes {
    pub(crate) fn new(store: Database, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| Input::field("Filter notes", "Filter notes", content(0.4), cx));
        query.update(cx, |input, _| input.set_font_size(12.));
        let title = cx.new(|cx| {
            let mut input = Input::field("Note title", "Untitled", content(0.35), cx);
            input.set_font_size(20.);
            input.set_max_chars(notes::MAX_TITLE);
            input
        });
        let body = cx.new(|cx| {
            let mut input = Input::editor(cx);
            input.set_font_size(13.);
            input.set_editor_path("note.md".into());
            input.set_max_chars(notes::MAX_BODY);
            input
        });
        let tag = cx.new(|cx| {
            let mut input = Input::field("Add note tag", "Add tag…", content(0.35), cx);
            input.set_font_size(11.);
            input.set_max_chars(notes::MAX_TAG_LENGTH + 1);
            input
        });
        let project_query =
            cx.new(|cx| Input::field("Find project", "Find project…", content(0.4), cx));
        let subscriptions = vec![
            cx.subscribe(&query, |_, _, _: &input::Changed, cx| cx.notify()),
            cx.subscribe(&title, |this, _, _: &input::Changed, cx| this.edited(cx)),
            cx.subscribe(&body, |this, _, _: &input::Changed, cx| {
                this.highlight_source(cx);
                this.edited(cx);
            }),
            cx.subscribe(&body, |this, _, event: &input::Rejected, cx| {
                this.error = Some(event.0.into());
                cx.notify();
            }),
            cx.subscribe(&tag, |this, _, _: &input::Changed, cx| {
                if this.tag.read(cx).content.ends_with(',') {
                    this.add_tag(cx);
                }
            }),
            cx.subscribe(&tag, |this, _, _: &input::Submitted, cx| this.add_tag(cx)),
            cx.subscribe(&title, |this, _, _: &input::Submitted, cx| this.flush(cx)),
            cx.subscribe(&project_query, |_, _, _: &input::Changed, cx| cx.notify()),
        ];
        let mut this = Self {
            store,
            notes: Vec::new(),
            selected: None,
            query,
            title,
            body,
            tag,
            project_query,
            project_anchor: AnchorSlot::default(),
            project_open: false,
            cwd: String::new(),
            projects: Vec::new(),
            appearances: HashMap::new(),
            source: false,
            modes: HashMap::new(),
            document: None,
            dirty: false,
            revision: 0,
            loading: true,
            error: None,
            debounce: None,
            image_busy: false,
            image_drag: false,
            list_width: 280.,
            resize: None,
            blurs_bound: false,
            _subscriptions: subscriptions,
        };
        this.refresh(cx);
        this
    }

    pub(crate) fn set_appearances(
        &mut self,
        appearances: HashMap<String, monocode_core::project::appearance::ProjectAppearance>,
        cx: &mut Context<Self>,
    ) {
        self.appearances = appearances;
        cx.notify();
    }

    fn project_mark(&self, path: &str, side: f32) -> AnyElement {
        crate::ui::project_badge::project_icon(
            &self.appearances.get(path).cloned().unwrap_or_default(),
            &project_name(path),
            side,
        )
    }

    fn highlight_source(&self, cx: &mut Context<Self>) {
        let text = self.body.read(cx).content.clone();
        let mut at = 0;
        let mut analysis = crate::editor::analysis::Analysis::default();
        for line in text.split_inclusive('\n') {
            let start = at;
            at += line.len();
            let trimmed = line.trim_start_matches(' ');
            let count = trimmed.bytes().take_while(|byte| *byte == b'#').count();
            if line.len() - trimmed.len() <= 3
                && (1..=6).contains(&count)
                && trimmed[count..].starts_with(char::is_whitespace)
            {
                analysis.tokens.push(crate::editor::analysis::Token {
                    from: start,
                    to: at - usize::from(line.ends_with('\n')),
                    kind: "var(--color-markdown-heading)".into(),
                });
            }
        }
        self.body
            .update(cx, |input, cx| input.set_editor_analysis(analysis, cx));
    }

    pub(crate) fn set_context(
        &mut self,
        cwd: String,
        projects: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        self.cwd = cwd;
        self.projects = projects;
        cx.notify();
    }

    /// How many notes the list holds, once it has loaded.
    pub(crate) fn count(&self) -> Option<usize> {
        (!self.loading).then_some(self.notes.len())
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.flush(cx);
        if self.dirty {
            return;
        }
        let revision = self.revision;
        let store = self.store.clone();
        let work = cx
            .background_executor()
            .spawn(async move { locked(&store)?.list_notes() });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                if this.revision != revision {
                    return;
                }
                match result {
                    Ok(mut notes) => {
                        // Preserve edits made while the refresh was in flight.
                        if this.dirty
                            && let Some(draft) = this.draft(cx)
                        {
                            notes.retain(|note| note.id != draft.id);
                            notes.insert(0, draft);
                        }
                        this.notes = notes;
                        let next = this
                            .selected
                            .clone()
                            .filter(|id| this.notes.iter().any(|note| &note.id == id))
                            .or_else(|| this.notes.first().map(|note| note.id.clone()));
                        if !this.dirty || next != this.selected || this.document.is_none() {
                            this.load_selection(next, cx);
                        }
                        this.error = None;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn draft(&self, cx: &App) -> Option<Note> {
        let mut note = self
            .notes
            .iter()
            .find(|note| Some(&note.id) == self.selected.as_ref())?
            .clone();
        note.title = self.title.read(cx).content.to_string();
        note.body = self.body.read(cx).content.to_string();
        Some(note)
    }

    fn edited(&mut self, cx: &mut Context<Self>) {
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
        self.error = None;
        self.debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(400))
                .await;
            let _ = this.update(cx, |this, cx| this.persist(false, cx));
        }));
        cx.notify();
    }

    /// Serializing at the editor boundary means a delayed save never overwrites
    /// a newer edit or resurrects a deleted note.
    pub(crate) fn flush(&mut self, cx: &mut Context<Self>) {
        self.add_tag(cx);
        self.persist(true, cx);
    }

    fn persist(&mut self, finalize: bool, cx: &mut Context<Self>) {
        self.debounce = None;
        let Some(mut draft) = self.draft(cx) else {
            return;
        };
        let saved_title = self
            .notes
            .iter()
            .find(|note| note.id == draft.id)
            .map(|note| note.title.clone())
            .unwrap_or_default();
        if !self.dirty && !(finalize && (draft.slug_pending || draft.title != saved_title)) {
            return;
        }
        self.dirty = true;
        if draft.title.trim().is_empty() {
            draft.title = if finalize {
                notes::title(&draft.body)
            } else {
                saved_title
            };
        }
        let result =
            locked(&self.store).and_then(|store| store.upsert_note(&draft.upsert(finalize)));
        match result {
            Ok(saved) => {
                self.revision = self.revision.wrapping_add(1);
                if finalize {
                    self.title
                        .update(cx, |input, cx| input.set_text(saved.title.clone(), cx));
                }
                self.notes.retain(|note| note.id != saved.id);
                self.notes.push(saved);
                self.notes
                    .sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.id.cmp(&b.id)));
                self.dirty = false;
                self.error = None;
            }
            Err(error) => self.error = Some(format!("Could not save note: {error}")),
        }
        cx.notify();
    }

    fn select(&mut self, id: String, cx: &mut Context<Self>) {
        if self.selected.as_ref() == Some(&id) {
            return;
        }
        self.flush(cx);
        if self.dirty {
            return;
        }
        self.load_selection(Some(id), cx);
    }

    fn load_selection(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.project_open = false;
        self.selected = id;
        self.dirty = false;
        let note = self
            .notes
            .iter()
            .find(|note| Some(&note.id) == self.selected.as_ref())
            .cloned();
        self.title.update(cx, |input, cx| {
            input.set_text(
                note.as_ref()
                    .map(|note| note.title.clone())
                    .unwrap_or_default(),
                cx,
            )
        });
        self.body.update(cx, |input, cx| {
            input.set_text(
                note.as_ref()
                    .map(|note| note.body.clone())
                    .unwrap_or_default(),
                cx,
            )
        });
        self.highlight_source(cx);
        self.body
            .update(cx, |input, cx| input.set_word_wrap(true, cx));
        self.tag
            .update(cx, |input, cx| input.set_text(String::new(), cx));
        self.source = note.as_ref().is_some_and(|note| {
            self.modes
                .get(&note.id)
                .copied()
                .unwrap_or(note.body.trim().is_empty() && note.title == "Untitled")
        });
        self.update_preview(cx);
        cx.notify();
    }

    fn update_preview(&mut self, cx: &mut Context<Self>) {
        self.document = self.draft(cx).map(|note| {
            cx.new(|_| {
                preview::Preview::new(
                    &note.body,
                    note.source_cwd.as_deref().unwrap_or(&self.cwd),
                    &self.store,
                )
            })
        });
    }

    fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flush(cx);
        if self.dirty {
            return;
        }
        let result =
            locked(&self.store).and_then(|store| store.create_note(Some(self.cwd.clone())));
        match result {
            Ok(note) => {
                self.revision = self.revision.wrapping_add(1);
                let id = note.id.clone();
                self.notes.insert(0, note);
                self.query
                    .update(cx, |input, cx| input.set_text(String::new(), cx));
                self.load_selection(Some(id), cx);
                window.focus(&self.body.focus_handle(cx), cx);
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        self.debounce = None;
        let result = locked(&self.store).and_then(|store| store.delete_note(&id));
        match result {
            Ok(()) => {
                self.revision = self.revision.wrapping_add(1);
                self.notes.retain(|note| note.id != id);
                self.modes.remove(&id);
                self.dirty = false;
                self.load_selection(self.notes.first().map(|note| note.id.clone()), cx);
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn add_tag(&mut self, cx: &mut Context<Self>) {
        let text = self
            .tag
            .read(cx)
            .content
            .trim()
            .trim_end_matches(',')
            .to_owned();
        if text.is_empty() {
            return;
        }
        self.tag
            .update(cx, |input, cx| input.set_text(String::new(), cx));
        if let Some(note) = self
            .notes
            .iter_mut()
            .find(|note| Some(&note.id) == self.selected.as_ref())
        {
            let tags = notes::normalize_tags(&[note.tags.clone(), vec![text]].concat());
            if tags != note.tags {
                note.tags = tags;
                self.edited(cx);
            }
        }
    }

    fn remove_tag(&mut self, tag: &str, cx: &mut Context<Self>) {
        if let Some(note) = self
            .notes
            .iter_mut()
            .find(|note| Some(&note.id) == self.selected.as_ref())
        {
            note.tags.retain(|saved| saved != tag);
            self.edited(cx);
        }
    }

    fn move_project(&mut self, path: String, cx: &mut Context<Self>) {
        if let Some(note) = self
            .notes
            .iter_mut()
            .find(|note| Some(&note.id) == self.selected.as_ref())
        {
            note.source_cwd = Some(path);
            self.revision = self.revision.wrapping_add(1);
            self.dirty = true;
            self.persist(false, cx);
            self.update_preview(cx);
        }
        self.project_open = false;
        cx.notify();
    }

    fn add_images(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if paths.is_empty() || self.image_busy {
            return;
        }
        let Some(id) = self.selected.clone() else {
            return;
        };
        let range = if self.source {
            self.body.read(cx).selection_range()
        } else {
            let length = self.body.read(cx).content.len();
            length..length
        };
        let store = self.store.clone();
        self.image_busy = true;
        self.image_drag = false;
        let work = cx.background_executor().spawn(async move {
            let store = locked(&store)?;
            let mut images = Vec::new();
            let mut failure = None;
            for path in paths {
                match store.save_note_image(&id, &path) {
                    Ok(image) => images.push(image),
                    Err(error) => failure = Some(error),
                }
            }
            if images.is_empty() {
                return Err(
                    failure.unwrap_or_else(|| "Drop a PNG, JPG, GIF, WebP, or SVG image.".into())
                );
            }
            Ok((id, images))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.image_busy = false;
                match result {
                    Ok((id, images)) => {
                        let block = images
                            .iter()
                            .map(notes::image_markdown)
                            .collect::<Vec<_>>()
                            .join("\n\n");
                        if this.selected.as_deref() == Some(&id) {
                            let text = this.body.read(cx).content.to_string();
                            let from = range.start.min(text.len());
                            let to = range.end.max(from).min(text.len());
                            let range = if text.is_char_boundary(from) && text.is_char_boundary(to)
                            {
                                from..to
                            } else {
                                text.len()..text.len()
                            };
                            let before = &text[..range.start];
                            let after = &text[range.end..];
                            let leading = if before.is_empty() || before.ends_with("\n\n") {
                                ""
                            } else if before.ends_with('\n') {
                                "\n"
                            } else {
                                "\n\n"
                            };
                            let trailing = if after.is_empty() || after.starts_with("\n\n") {
                                ""
                            } else if after.starts_with('\n') {
                                "\n"
                            } else {
                                "\n\n"
                            };
                            let insert = format!("{leading}{block}{trailing}");
                            let cursor = leading.len() + block.len();
                            this.body.update(cx, |input, cx| {
                                input.replace_bytes(range, &insert, cursor, cx)
                            });
                            this.edited(cx);
                            this.update_preview(cx);
                            if this.source {
                                window.focus(&this.body.focus_handle(cx), cx);
                            }
                        } else if let Some(note) = this.notes.iter_mut().find(|note| note.id == id)
                        {
                            this.revision = this.revision.wrapping_add(1);
                            // Finish a drop on its original note after navigation.
                            note.body = format!(
                                "{}{}{}",
                                note.body,
                                if note.body.is_empty() || note.body.ends_with("\n\n") {
                                    ""
                                } else {
                                    "\n\n"
                                },
                                block
                            );
                            let update = note.upsert(false);
                            if let Err(error) =
                                locked(&this.store).and_then(|store| store.upsert_note(&update))
                            {
                                this.error = Some(error);
                            }
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

fn project_name(path: &str) -> String {
    monocode_core::files::leaf_name(path)
}
fn relative_time(at: i64) -> String {
    let seconds = (monocode_core::sys::time::now_ms() as i64 - at).max(0) / 1000;
    match seconds {
        0..60 => "now".into(),
        60..3600 => format!("{}m", seconds / 60),
        3600..86400 => format!("{}h", seconds / 3600),
        86400..604800 => format!("{}d", seconds / 86400),
        _ => chrono::DateTime::from_timestamp_millis(at)
            .map(|date| date.format("%b %-d").to_string())
            .unwrap_or_default(),
    }
}
