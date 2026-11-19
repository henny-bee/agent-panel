//! BtwSheet.tsx's isolated side threads. Their transcript and cancellation never
//! share the parent provider session, turn queue, or main transcript rows.
mod actions;
mod view;
use super::*;
use crate::chat::transcript::burst::{self as fx, Bubble, Halo};
use gpui::{
    AnyElement, App, Bounds, Corners, Hsla, Pixels, Point, Render, Window, point, rgb, size,
};
use monocode_core::{
    provider::{self, Event, Request},
    session::transcript::{Row, Transcript},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const SNAPSHOT_CHARS: usize = 64_000;
/// BtwSheet's `max-w-4xl`.
const SHEET_WIDTH: f32 = 896.;
/// The composer box radius the morph starts and ends on.
const BOX_RADIUS: f32 = 12.;
/// `--radius-lg`, the sheet's resting top corners.
const SHEET_RADIUS: f32 = 8.;
const OPEN_MS: f32 = 360.;
const CLOSE_MS: f32 = 260.;
const OPEN_EASE: fx::Ease = (0.22, 1., 0.36, 1.);
const CLOSE_EASE: fx::Ease = (0.4, 0., 0.2, 1.);
const BURST_MS: f32 = 1900.;
const MODEL_NOTE: &str = "Answer side questions with another model.";
const BLOCK_CHARS: usize = 8_000;

fn supported(kind: Kind) -> bool {
    !matches!(kind, Kind::Fx | Kind::Hermes | Kind::Antigravity)
}
fn bounded(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}
fn snapshot(rows: &[Row]) -> String {
    let mut result = Vec::new();
    let mut remaining = SNAPSHOT_CHARS;
    let mut end = rows.len();
    while end > 0 && remaining > 0 {
        let start = rows[..end].iter().rposition(|row| row.first).unwrap_or(0);
        let role = &rows[start].role;
        if matches!(
            role.as_str(),
            "user" | "assistant" | "tool" | "tasks" | "plan" | "image"
        ) {
            let prefix = format!("{role}: ");
            let limit = BLOCK_CHARS.min(remaining);
            let text: String = prefix
                .chars()
                .chain(rows[start..end].iter().flat_map(|row| row.text.chars()))
                .take(limit)
                .collect();
            remaining = remaining.saturating_sub(text.chars().count() + 2);
            result.push(text);
        }
        end = start;
    }
    result.reverse();
    result.join("\n\n")
}

#[derive(Clone)]
pub(super) struct Target {
    source_turn: usize,
    snapshot: String,
    provider: Kind,
    model: Option<String>,
    cwd: String,
    settings: Settings,
}
#[derive(Clone, Serialize, Deserialize)]
struct Thread {
    id: String,
    source_turn: usize,
    snapshot: String,
    provider: String,
    model: Option<String>,
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    account: monocode_core::session::profile::Profile,
    #[serde(default)]
    effort: Option<String>,
    #[serde(default)]
    model_settings: HashMap<String, String>,
    #[serde(skip)]
    environment: HashMap<String, String>,
    rows: Vec<Row>,
    draft: String,
    status: String,
    error: Option<String>,
    #[serde(default)]
    desktop: Option<serde_json::Value>,
}
#[derive(Default, Serialize, Deserialize)]
struct Saved {
    revision: u64,
    threads: Vec<Thread>,
}
enum Stream {
    Text(String),
    Finished(Result<(), String>),
}
pub(super) struct Btw {
    session: i64,
    store: Database,
    parent_input: Entity<input::Input>,
    input: Entity<input::Input>,
    transcript: Entity<TranscriptView>,
    target: Target,
    threads: Vec<Thread>,
    active: Option<String>,
    open: bool,
    /// Still on screen: open, or folding back into the composer.
    rendered: bool,
    morph: Option<Morph>,
    /// When the panel last started opening or closing, for the body's
    /// entrance and exit.
    shown_at: Option<Instant>,
    question_burst: Option<(Instant, u64)>,
    /// The session composer's box, the sheet's own composer box, and the
    /// overlay, each in window coordinates.
    home: AnchorSlot,
    side_box: AnchorSlot,
    overlay: AnchorSlot,
    focus_pending: bool,
    loading: bool,
    error: Option<String>,
    revision: u64,
    pending_submit: Option<String>,
    cancel: HashMap<String, Arc<AtomicBool>>,
    /// The handoff-style model menu, while open.
    models: Option<Entity<Menu>>,
    model_subscription: Option<Subscription>,
    model_anchor: AnchorSlot,
    _submit: Subscription,
    _changed: Subscription,
    response_subscription: Option<Subscription>,
}
impl Drop for Btw {
    fn drop(&mut self) {
        for cancel in self.cancel.values() {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}
impl Btw {
    pub(super) fn new(
        session: i64,
        store: Database,
        parent_input: Entity<input::Input>,
        home: AnchorSlot,
        target: Target,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(input::Input::side_question);
        let submit = cx.subscribe(&input, |this, _, _: &input::Submitted, cx| {
            this.submit(cx);
        });
        let changed = cx.observe(&input, |this, input, cx| {
            if let Some(thread) = this.active_mut() {
                thread.draft = input.read(cx).content.to_string();
            }
        });
        let transcript = cx.new(|_| {
            let mut view = TranscriptView::new();
            view.side_question();
            view
        });
        let load_store = store.clone();
        let work = cx.background_executor().spawn(async move {
            let store = locked(&load_store)?;
            if let Some(value) = store.setting(&format!("btw:{session}"))? {
                return serde_json::from_str::<Saved>(&value)
                    .map(Some)
                    .map_err(|error| format!("Could not restore side questions: {error}"));
            }
            let imported = store.desktop_side_threads(session)?;
            let accounts = store.provider_accounts()?;
            let root = store.data_dir().map(std::path::Path::to_path_buf);
            let threads = imported
                .into_iter()
                .map(|thread| {
                    let kind = Kind::from_id(&thread.provider);
                    let account = monocode_core::session::profile::Profile {
                        id: accounts.selected(kind, &thread.cwd),
                        root: root.clone(),
                    };
                    let effort = [
                        "effort",
                        "reasoning",
                        "reasoningEffort",
                        "reasoning_effort",
                        "variant",
                    ]
                    .into_iter()
                    .find_map(|key| thread.model_settings.get(key).cloned());
                    Thread {
                        id: thread.id,
                        source_turn: thread.source_turn,
                        snapshot: thread.snapshot,
                        provider: thread.provider,
                        model: thread.model,
                        cwd: thread.cwd,
                        account,
                        effort,
                        model_settings: thread.model_settings,
                        environment: HashMap::new(),
                        rows: thread.rows,
                        draft: String::new(),
                        status: match thread.status.as_str() {
                            "ready" => "completed",
                            "running" => "stopped",
                            status => status,
                        }
                        .into(),
                        error: thread.error,
                        desktop: Some(thread.original),
                    }
                })
                .collect();
            Ok(Some(Saved {
                revision: 0,
                threads,
            }))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(Some(mut saved)) => {
                        this.revision = this.revision.max(saved.revision);
                        for thread in &mut saved.threads {
                            if thread.cwd.is_empty() {
                                thread.cwd = this.target.cwd.clone();
                            }
                            thread.environment = this.target.settings.environment.clone();
                            if thread.status == "running" {
                                thread.status = "stopped".into();
                            }
                        }
                        saved.threads.append(&mut this.threads);
                        this.threads = saved.threads;
                    }
                    Ok(None) => {}
                    Err(error) => this.error = Some(error),
                }
                if let Some(text) = this.pending_submit.take()
                    && this.input.read(cx).content == text
                {
                    this.submit(cx);
                }
                this.refresh(cx);
            });
        })
        .detach();
        Self {
            session,
            store,
            parent_input,
            input,
            transcript,
            target,
            threads: Vec::new(),
            active: None,
            open: false,
            rendered: false,
            morph: None,
            shown_at: None,
            question_burst: None,
            home,
            side_box: AnchorSlot::default(),
            overlay: AnchorSlot::default(),
            focus_pending: false,
            loading: true,
            error: None,
            revision: monocode_core::sys::time::now_ms(),
            pending_submit: None,
            cancel: HashMap::new(),
            models: None,
            model_subscription: None,
            model_anchor: AnchorSlot::default(),
            _submit: submit,
            _changed: changed,
            response_subscription: None,
        }
    }
    pub(super) fn is_open(&self) -> bool {
        self.open
    }
    /// Esc: close the model menu if it is up, else fold the sheet away.
    /// False leaves the key to others (closed sheet, or IME composition).
    pub(super) fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.open || self.input.read(cx).is_composing() {
            return false;
        }
        if self.models.take().is_some() {
            window.focus(&self.input.focus_handle(cx), cx);
            cx.notify();
        } else {
            self.close(window, cx);
        }
        true
    }
    /// How much of the session composer shows: it steps aside under an open
    /// sheet and returns as the sheet folds back into it.
    pub(super) fn home_opacity(&self, now: Instant) -> f32 {
        if !self.rendered {
            return 1.;
        }
        match self.morph {
            Some(morph) if morph.opening => {
                let e = morph.eased(now);
                (1. - e / 0.3).max(0.)
            }
            Some(morph) => ((morph.eased(now) - 0.55) / 0.45).max(0.),
            None => 0.,
        }
    }
    fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.open == open && self.rendered {
            return;
        }
        let was_rendered = self.rendered;
        self.open = open;
        if !open && !self.rendered {
            return;
        }
        self.rendered = true;
        if cx.reduce_motion() {
            self.morph = None;
            self.rendered = open;
            self.shown_at = None;
            return;
        }
        // A quick reopen or close reverses from where the panel is now.
        let reversing = was_rendered && self.morph.is_some();
        let from = match self.morph.filter(|_| was_rendered) {
            Some(morph) => {
                let ease = if open { OPEN_EASE } else { CLOSE_EASE };
                invert(ease, 1. - morph.eased(Instant::now()))
            }
            None => 0.,
        };
        self.morph = Some(Morph {
            opening: open,
            from,
            start: None,
            reversing,
        });
        self.shown_at = None;
    }
    /// Advance the open or close motion, starting it once the boxes it moves
    /// between have been measured.
    fn step_motion(&mut self, now: Instant, window: &mut Window, cx: &mut Context<Self>) {
        let before = (self.rendered, self.morph.is_some());
        if let Some(morph) = self.morph.as_mut()
            && morph.start.is_none()
        {
            if self.home.get().is_some() && self.side_box.get().is_some() {
                morph.start = Some(now);
                self.shown_at = Some(now);
                if morph.opening && !morph.reversing {
                    self.question_burst = Some((now, monocode_core::sys::time::now_ms()));
                }
            } else if self.overlay.get().is_some() {
                // Nothing to grow out of (the composer is not on screen).
                let opening = morph.opening;
                self.morph = None;
                self.shown_at = Some(now);
                if !opening {
                    self.rendered = false;
                }
            }
        }
        if let Some(morph) = self.morph
            && morph.start.is_some()
            && morph.progress(now) >= 1.
        {
            self.morph = None;
            if !morph.opening {
                self.rendered = false;
                self.shown_at = None;
            }
        }
        if self
            .question_burst
            .is_some_and(|(at, _)| now.duration_since(at).as_secs_f32() * 1000. >= BURST_MS)
        {
            self.question_burst = None;
        }
        // Notify rather than only redraw, so the conversation can fade its
        // composer in step, including on the frame the motion settles.
        if self.morph.is_some()
            || self.question_burst.is_some()
            || self.body_moving(now)
            || before != (self.rendered, self.morph.is_some())
        {
            let view = cx.entity().downgrade();
            window.on_next_frame(move |_, cx| {
                let _ = view.update(cx, |_, cx| cx.notify());
            });
        }
    }
    fn body_moving(&self, now: Instant) -> bool {
        self.shown_at.is_some_and(|at| {
            now.duration_since(at).as_secs_f32() * 1000. < if self.open { 340. } else { 120. }
        })
    }
    /// btw-body-in on open, btw-fade-out on close: (opacity, rise).
    fn body_motion(&self, now: Instant) -> (f32, f32) {
        let Some(at) = self.shown_at else {
            return (if self.open { 1. } else { 0. }, 0.);
        };
        let t = now.duration_since(at).as_secs_f32() * 1000.;
        if self.open {
            let e = fx::ease(fx::progress(t, 100., 240.), OPEN_EASE);
            (e, 10. * (1. - e))
        } else {
            let e = fx::ease(fx::progress(t, 0., 120.), (0.42, 0., 1., 1.));
            (1. - e, 0.)
        }
    }
    pub(super) fn is_running(&self) -> bool {
        !self.cancel.is_empty()
    }
    fn active(&self) -> Option<&Thread> {
        self.threads
            .iter()
            .find(|thread| Some(&thread.id) == self.active.as_ref())
    }
    fn active_mut(&mut self) -> Option<&mut Thread> {
        self.threads
            .iter_mut()
            .find(|thread| Some(&thread.id) == self.active.as_ref())
    }
    pub(super) fn open_with(
        &mut self,
        target: Target,
        text: String,
        draft: bool,
        cx: &mut Context<Self>,
    ) {
        self.target = target;
        self.new_thread(cx);
        self.set_open(true, cx);
        self.focus_pending = true;
        self.pending_submit =
            (!draft && !text.trim().is_empty() && self.loading).then(|| text.clone());
        self.input.update(cx, |input, cx| input.set_text(text, cx));
        if !draft && !self.loading {
            self.submit(cx);
        }
        cx.notify();
    }
}

#[derive(Clone, Copy)]
struct Morph {
    opening: bool,
    /// Where the motion starts, so a reversal continues from the current frame.
    from: f32,
    /// `None` holds the first frame until the boxes have been measured.
    start: Option<Instant>,
    reversing: bool,
}
impl Morph {
    fn progress(&self, now: Instant) -> f32 {
        let ms = if self.opening { OPEN_MS } else { CLOSE_MS };
        self.start.map_or(self.from, |start| {
            (self.from + now.duration_since(start).as_secs_f32() * 1000. / ms).min(1.)
        })
    }
    fn eased(&self, now: Instant) -> f32 {
        fx::ease(
            self.progress(now),
            if self.opening { OPEN_EASE } else { CLOSE_EASE },
        )
    }
}
/// The progress at which `ease` reaches `value`.
fn invert(ease: fx::Ease, value: f32) -> f32 {
    let (mut low, mut high) = (0f32, 1f32);
    for _ in 0..24 {
        let middle = (low + high) * 0.5;
        if fx::ease(middle, ease) < value {
            low = middle;
        } else {
            high = middle;
        }
    }
    (low + high) * 0.5
}
fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}
fn lerp_rect(from: Bounds<Pixels>, to: Bounds<Pixels>, t: f32) -> Bounds<Pixels> {
    let at = |a: Pixels, b: Pixels| px(lerp(f32::from(a), f32::from(b), t));
    Bounds::new(
        point(
            at(from.origin.x, to.origin.x),
            at(from.origin.y, to.origin.y),
        ),
        size(
            at(from.size.width, to.size.width),
            at(from.size.height, to.size.height),
        ),
    )
}
/// The keyframe opacity of the composer-shaped frame carried between boxes.
fn outline_opacity(e: f32) -> f32 {
    if e < 0.15 {
        e / 0.15
    } else if e < 0.8 {
        1.
    } else {
        (1. - e) / 0.2
    }
}
/// BtwSheet's `compactQuestion`.
fn compact_question(text: Option<&str>) -> String {
    let compact = text
        .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "New question".into());
    if compact.chars().count() > 48 {
        format!("{}…", bounded(&compact, 45))
    } else {
        compact
    }
}
fn status_color(status: &str) -> Option<Hsla> {
    match status {
        "draft" => None,
        "running" => Some(rgb(0xfcd34d).into()),
        "error" => Some(rgb(0xfca5a5).into()),
        _ => Some(rgb(0x6ee7b7).into()),
    }
}
fn red(hex: u32, alpha: f32) -> Hsla {
    let mut color: Hsla = rgb(hex).into();
    color.a = alpha;
    color
}
fn header_button(
    id: &'static str,
    label: &'static str,
    glyph: &'static str,
    side: f32,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .cursor_pointer()
        .flex_shrink_0()
        .size(px(28.))
        .rounded_md()
        .flex()
        .items_center()
        .justify_center()
        .text_color(content(0.45))
        .hover(|s| s.bg(content(0.08)).text_color(content(1.)))
        .child(icon(glyph, side))
}

/// The /btw panel's frame for one moment of its open or close motion.
struct Shape {
    clip: Bounds<Pixels>,
    corners: Corners<Pixels>,
    /// The glass and content layers' opacity.
    layer: f32,
    /// The composer outline carried between the two composer boxes.
    outline: Option<(Bounds<Pixels>, f32)>,
    /// The side composer's own fade.
    composer: f32,
}

impl Btw {}

impl Conversation {
    fn btw_target(&self, cx: &App) -> Option<Target> {
        if !self.history_ready()
            || self.settings.mono.is_some()
            || self.settings.worktree_removed
            || self.editing_last_turn.pending()
        {
            return None;
        }
        let rows = self.transcript.read(cx).source_rows();
        let (turn, info) = self
            .settings
            .turns
            .iter()
            .filter(|(_, info)| {
                info.completed_at.is_some()
                    && supported(
                        Kind::parse(&info.provider)
                            .unwrap_or_else(|| Kind::from_id(&self.summary.provider)),
                    )
            })
            .max_by_key(|(turn, _)| **turn)?;
        let user = rows[..(*turn).min(rows.len())]
            .iter()
            .rposition(|row| row.first && row.role == "user")?;
        let end = rows[user + 1..]
            .iter()
            .position(|row| row.first && row.role == "user")
            .map_or(rows.len(), |offset| user + 1 + offset);
        let provider =
            Kind::parse(&info.provider).unwrap_or_else(|| Kind::from_id(&self.summary.provider));
        if !supported(provider) {
            return None;
        }
        let mut settings = self.settings.clone();
        settings.environment.extend(self.app_environment.clone());
        if provider != Kind::from_id(&self.summary.provider) {
            settings.account = crate::accounts::get(cx)
                .map(|center| center.read(cx).selected(provider, self.execution_cwd()))
                .unwrap_or_default();
        }
        Some(Target {
            source_turn: user,
            snapshot: snapshot(&rows[..end]),
            provider,
            model: info.model_id.clone().or_else(|| self.model.clone()),
            cwd: self.execution_cwd().into(),
            settings,
        })
    }
    pub(super) fn open_btw(&mut self, text: String, draft: bool, cx: &mut Context<Self>) -> bool {
        if self.attaching || !self.attachments.is_empty() || self.input.read(cx).is_composing() {
            return false;
        }
        self.attach_account(cx);
        let Some(target) = self.btw_target(cx) else {
            self.status =
                "Finish a supported conversation turn before asking a side question.".into();
            cx.notify();
            return false;
        };
        if self.btw.is_none() {
            let btw = cx.new(|cx| {
                Btw::new(
                    self.summary.id,
                    self.store.clone(),
                    self.input.clone(),
                    self.btw_home.0.clone(),
                    target.clone(),
                    cx,
                )
            });
            // Only the composer's fade needs this view to redraw with the sheet.
            self._btw_observer = Some(cx.observe(&btw, |this, btw, cx| {
                let home = btw.read(cx).home_opacity(Instant::now());
                if home != this.btw_home.1 {
                    this.btw_home.1 = home;
                    cx.notify();
                }
            }));
            self.btw = Some(btw);
        }
        if let Some(btw) = &self.btw {
            btw.update(cx, |btw, cx| btw.open_with(target, text, draft, cx));
        }
        self.completion
            .update(cx, |completion, cx| completion.dismiss(cx));
        self.input.update(cx, |input, cx| {
            input.take(cx);
        });
        cx.notify();
        true
    }
    pub(super) fn open_typed_btw(&mut self, cx: &mut Context<Self>) {
        if self.input.read(cx).is_composing()
            || !self.attachments.is_empty()
            || self.btw_target(cx).is_none()
        {
            return;
        }
        let content = self.input.read(cx).content.to_string();
        let text = content.trim_start();
        let Some(prefix) = text.get(..4) else {
            return;
        };
        if prefix.eq_ignore_ascii_case("/btw") && text[4..].starts_with(char::is_whitespace) {
            self.open_btw(text[4..].trim_start().into(), true, cx);
        }
    }
}

#[cfg(test)]
mod tests;
