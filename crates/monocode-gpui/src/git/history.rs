//! GitHistoryGraph.tsx's standalone History tab: its toolbar, search and
//! 44px rows with the swimlane graph painted per row.
use crate::{
    git::graph::{self, GraphCommit, NodeKind, Row, RowGraph, SWIMLANE_WIDTH},
    runtime::git_status::{GitChanged, ProjectGit},
    ui::panel::*,
    ui::{
        input::{self, Input},
        popover::{Anchor, popover},
        theme::*,
    },
};
use gpui::{
    AnyElement, App, Bounds, ClipboardItem, Context, Entity, FocusHandle, Focusable, FontWeight,
    Hsla, KeyBinding, MouseButton, MouseDownEvent, Pixels, Point, Role, ScrollStrategy,
    SharedString, Subscription, UniformListScrollHandle, Window, actions, canvas, div, fill, point,
    prelude::*, px, rgb, size, uniform_list,
};
use monocode_core::{
    project::languages::Share,
    vcs::git::scm::{self, CommitDetails, HistoryCommit},
};
use std::{collections::HashMap, path::PathBuf};

mod languages;
mod view;

const ROW: f32 = 44.;
/// The Code tab lists this many languages, then "Other".
const LANGUAGE_ROWS: usize = 6;
actions!(history, [CloseSearch]);

pub(crate) fn bind(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        "escape",
        CloseSearch,
        Some("HistorySearch"),
    )]);
}

pub(crate) enum HistoryEvent {
    OpenCommit {
        cwd: String,
        sha: String,
        short_sha: String,
        subject: String,
    },
    /// The section switched between GRAPH (false) and CODE (true).
    Code(bool),
}
impl gpui::EventEmitter<HistoryEvent> for History {}

struct Item {
    commit: HistoryCommit,
    row: Row,
    graph: RowGraph,
}

/// The right-click card, keyed by SHA so a reload keeps it on its commit.
struct Card {
    sha: String,
    at: Point<Pixels>,
    details: Option<CommitDetails>,
}

pub(crate) struct History {
    git: Entity<ProjectGit>,
    /// The collapsible GRAPH section under Changes, rather than the tab.
    section: bool,
    pub(crate) open: bool,
    /// The section shows the CODE tab's languages instead of the graph.
    code: bool,
    languages: Option<Result<Vec<Share>, String>>,
    languages_request: u64,
    card: Option<Card>,
    cwd: String,
    enabled: bool,
    items: Vec<Item>,
    /// historyByCwd: a revisit opens on the last read.
    cache: HashMap<String, Vec<HistoryCommit>>,
    request: u64,
    loading: bool,
    error: Option<String>,
    search_open: bool,
    search: Entity<Input>,
    query: String,
    visible: Vec<usize>,
    selected: Option<String>,
    hovered: Option<usize>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl History {
    pub(crate) fn new(
        git: Entity<ProjectGit>,
        section: bool,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| {
            let mut input = Input::field("Search commits", "Search history…", content(0.35), cx);
            input.set_font_size(12.);
            input
        });
        let subscriptions = vec![
            cx.subscribe(&search, |this, search, _: &input::Changed, cx| {
                this.query = search.read(cx).content.to_string();
                this.filter();
                cx.notify();
            }),
            cx.subscribe(&git, |this, _, _: &GitChanged, cx| this.git_changed(cx)),
        ];
        Self {
            git,
            section,
            open: true,
            code: false,
            languages: None,
            languages_request: 0,
            card: None,
            cwd: String::new(),
            enabled: false,
            items: Vec::new(),
            cache: HashMap::new(),
            request: 0,
            loading: false,
            error: None,
            search_open: false,
            search,
            query: String::new(),
            visible: Vec::new(),
            selected: None,
            hovered: None,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }
    pub(crate) fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let was = self.enabled;
        self.enabled = enabled;
        if enabled && !was {
            self.sync_project(cx);
            self.refresh(cx);
        }
    }
    /// The window came back into focus.
    pub(crate) fn resume(&mut self, cx: &mut Context<Self>) {
        if self.enabled {
            self.refresh(cx);
        }
    }
    fn git_changed(&mut self, cx: &mut Context<Self>) {
        if !self.enabled {
            return;
        }
        self.sync_project(cx);
        self.refresh(cx);
    }
    /// The saved tab choice, applied when the panel opens.
    pub(crate) fn set_code(&mut self, code: bool, cx: &mut Context<Self>) {
        if self.section && self.code != code {
            self.code = code;
            self.refresh(cx);
            cx.notify();
        }
    }
    /// Reread whatever the section shows: the languages or the commits.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.code {
            if self.open {
                self.load_languages(cx);
            }
        } else {
            self.load(cx);
        }
    }
    /// A label click switches the tab and unfolds the section.
    fn choose(&mut self, code: bool, cx: &mut Context<Self>) {
        let changed = self.code != code || !self.open;
        self.open = true;
        if self.code != code {
            self.code = code;
            cx.emit(HistoryEvent::Code(code));
        }
        if changed {
            self.refresh(cx);
        }
        cx.notify();
    }
    fn load_languages(&mut self, cx: &mut Context<Self>) {
        if !self.enabled || !self.available() {
            return;
        }
        self.languages_request += 1;
        let request = self.languages_request;
        let cwd = self.cwd.clone();
        let work = cx.background_executor().spawn(async move {
            monocode_core::project::languages::languages(&cwd, LANGUAGE_ROWS)
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if this.languages_request == request {
                    this.languages = Some(result);
                    cx.notify();
                }
            });
        })
        .detach();
    }
    /// A new checkout clears the search and opens on its cached history.
    fn sync_project(&mut self, cx: &mut Context<Self>) -> bool {
        let root = self.git.read(cx).root.clone();
        if root == self.cwd {
            return false;
        }
        self.cwd = root;
        self.request += 1;
        self.languages_request += 1;
        self.languages = None;
        self.loading = false;
        self.error = None;
        self.search_open = false;
        self.query.clear();
        self.search
            .update(cx, |s, cx| s.set_text(String::new(), cx));
        let cached = self.cache.get(&self.cwd).cloned().unwrap_or_default();
        self.set_commits(cached);
        cx.notify();
        true
    }
    fn available(&self) -> bool {
        !self.cwd.is_empty() && self.cwd != "~"
    }
    fn load(&mut self, cx: &mut Context<Self>) {
        if !self.enabled || !self.available() {
            return;
        }
        self.request += 1;
        let request = self.request;
        self.loading = true;
        self.error = None;
        let cwd = self.cwd.clone();
        let path = PathBuf::from(&cwd);
        let work = cx
            .background_executor()
            .spawn(async move { scm::history(&path, Some(200)) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if this.request != request {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(history) => {
                        let same = this.items.len() == history.commits.len()
                            && this
                                .items
                                .iter()
                                .zip(&history.commits)
                                .all(|(a, b)| &a.commit == b);
                        if !same {
                            this.cache.insert(cwd, history.commits.clone());
                            this.set_commits(history.commits);
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
    fn set_commits(&mut self, commits: Vec<HistoryCommit>) {
        let graph: Vec<GraphCommit> = commits
            .iter()
            .map(|c| GraphCommit {
                sha: c.sha.clone(),
                parents: c.parents.clone(),
                head: c.head,
                refs: c
                    .refs
                    .iter()
                    .map(|r| (r.name.clone(), r.kind.clone()))
                    .collect(),
            })
            .collect();
        let rows = graph::layout(&graph);
        self.items = commits
            .into_iter()
            .zip(rows)
            .map(|(commit, row)| {
                let graph = graph::row_graph(&row, ROW);
                Item { commit, row, graph }
            })
            .collect();
        self.filter();
    }
    fn filter(&mut self) {
        let needle = self.query.trim().to_lowercase();
        self.visible = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                needle.is_empty() || {
                    let c = &item.commit;
                    [&c.subject, &c.author, &c.sha]
                        .into_iter()
                        .chain(c.refs.iter().map(|r| &r.name))
                        .any(|v| v.to_lowercase().contains(&needle))
                }
            })
            .map(|(ix, _)| ix)
            .collect();
        self.hovered = None;
    }
    fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = true;
        let focus = self.search.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }
    fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = false;
        self.query.clear();
        self.search
            .update(cx, |s, cx| s.set_text(String::new(), cx));
        self.filter();
        window.focus(&self.focus, cx);
        cx.notify();
    }
    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        let commit = &self.items[ix].commit;
        self.selected = Some(commit.sha.clone());
        cx.emit(HistoryEvent::OpenCommit {
            cwd: self.cwd.clone(),
            sha: commit.sha.clone(),
            short_sha: commit.short_sha.clone(),
            subject: commit.subject.clone(),
        });
        cx.notify();
    }
    fn open_card(&mut self, ix: usize, at: Point<Pixels>, cx: &mut Context<Self>) {
        let sha = self.items[ix].commit.sha.clone();
        self.card = Some(Card {
            sha: sha.clone(),
            at,
            details: None,
        });
        let path = PathBuf::from(&self.cwd);
        let work = cx.background_executor().spawn({
            let sha = sha.clone();
            async move { scm::commit_details(&path, &sha) }
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                // A failed read leaves the card with the row's subject.
                if let (Ok(details), Some(card)) = (result, this.card.as_mut())
                    && card.sha == sha
                {
                    card.details = Some(details);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }
    fn close_card(&mut self, cx: &mut Context<Self>) {
        self.card = None;
        cx.notify();
    }
}

/// `Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" })` in
/// an English locale.
fn short_date(timestamp: i64) -> Option<String> {
    use chrono::{Datelike, Local, TimeZone};
    let date = Local.timestamp_opt(timestamp, 0).single()?;
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    Some(format!("{} {}", MONTHS[date.month0() as usize], date.day()))
}

/// `color-mix(in oklab, color 70%, content)`: the detailed rows' softened ink.
fn soft(color: u32) -> Hsla {
    fn to_linear(c: f32) -> f32 {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    fn to_srgb(c: f32) -> f32 {
        if c <= 0.003_130_8 {
            c * 12.92
        } else {
            1.055 * c.powf(1. / 2.4) - 0.055
        }
    }
    fn oklab(hex: u32) -> [f32; 3] {
        let [r, g, b] = [16, 8, 0].map(|s| to_linear(((hex >> s) & 0xff) as f32 / 255.));
        let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
        let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
        let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
        [
            0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
            1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
            0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
        ]
    }
    let (a, b) = (oklab(color), oklab(TEXT));
    let [l, x, y] = [0, 1, 2].map(|i| a[i] * 0.7 + b[i] * 0.3);
    let l_ = (l + 0.396_337_78 * x + 0.215_803_76 * y).powi(3);
    let m_ = (l - 0.105_561_346 * x - 0.063_854_17 * y).powi(3);
    let s_ = (l - 0.089_484_18 * x - 1.291_485_5 * y).powi(3);
    let rgb_ = [
        4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_,
        -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_,
        -0.004_196_086_3 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_,
    ]
    .map(|c| (to_srgb(c).clamp(0., 1.) * 255.).round() as u32);
    rgb((rgb_[0] << 16) | (rgb_[1] << 8) | rgb_[2]).into()
}
/// `color-mix(in srgb, content N%, background-base)`.
fn over_base(alpha: f32) -> Hsla {
    let mix = |s: u32| {
        let c = ((TEXT >> s) & 0xff) as f32;
        let b = ((BACKGROUND_BASE >> s) & 0xff) as f32;
        (c * alpha + b * (1. - alpha)).round() as u32
    };
    rgb((mix(16) << 16) | (mix(8) << 8) | mix(0)).into()
}

impl Focusable for History {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl History {
    #[allow(dead_code)]
    fn reveal(&self, ix: usize) {
        self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
    }
}

#[cfg(test)]
mod tests;
