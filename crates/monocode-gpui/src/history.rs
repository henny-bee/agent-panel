//! GitHistoryGraph.tsx's standalone History tab: its toolbar, search and
//! 44px rows with the swimlane graph painted per row.
use crate::{
    git_graph::{self, GraphCommit, NodeKind, Row, RowGraph, SWIMLANE_WIDTH},
    git_status::{GitChanged, ProjectGit},
    input::{self, Input},
    popover::{Anchor, popover},
    right_panel::*,
    theme::*,
};
use gpui::{
    AnyElement, App, Bounds, ClipboardItem, Context, Entity, FocusHandle, Focusable, FontWeight,
    Hsla, KeyBinding, MouseButton, MouseDownEvent, Pixels, Point, Role, ScrollStrategy,
    SharedString, Subscription, UniformListScrollHandle, Window, actions, canvas, div, fill, point,
    prelude::*, px, rgb, size, uniform_list,
};
use monocode_core::{
    languages::Share,
    scm::{self, CommitDetails, HistoryCommit},
};
use std::{collections::HashMap, path::PathBuf};

mod languages;

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
        let work = cx
            .background_executor()
            .spawn(async move { monocode_core::languages::languages(&cwd, LANGUAGE_ROWS) });
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
        let rows = git_graph::layout(&graph);
        self.items = commits
            .into_iter()
            .zip(rows)
            .map(|(commit, row)| {
                let graph = git_graph::row_graph(&row, ROW);
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

    /// VS Code's Graph hover: author, age, message, stat, refs, SHA and link.
    fn card(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let card = self.card.as_ref()?;
        let item = self.items.iter().find(|i| i.commit.sha == card.sha)?;
        let commit = &item.commit;
        let details = card.details.as_ref();
        let message = details
            .map(|d| d.message.clone())
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| commit.subject.clone());
        let when = {
            use chrono::TimeZone;
            let ago = chrono::DateTime::from_timestamp(commit.timestamp, 0)
                .map(|at| crate::inbox::ui::relative_time(&at.to_rfc3339()))
                .unwrap_or_default();
            match chrono::Local.timestamp_opt(commit.timestamp, 0).single() {
                Some(at) => format!("{ago} ({})", at.format("%B %-d, %Y at %-I:%M %p")),
                None => ago,
            }
        };
        let rule = || div().h(px(1.)).flex_shrink_0().bg(content(0.1));
        let row = || div().px_3().py_2().flex().items_center().gap_1();
        let stat = details.filter(|d| d.files > 0).map(|d| {
            let plural =
                |n: i64, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
            row()
                .child(format!("{} changed", plural(d.files, "file")))
                .when(d.additions > 0, |r| {
                    r.child(
                        div()
                            .text_color(rgb(DIFF_ADD))
                            .child(format!(", {}(+)", plural(d.additions, "insertion"))),
                    )
                })
                .when(d.deletions > 0, |r| {
                    r.child(
                        div()
                            .text_color(rgb(DIFF_DEL))
                            .child(format!(", {}(-)", plural(d.deletions, "deletion"))),
                    )
                })
        });
        let url = details.and_then(|d| d.url.clone());
        let github = url
            .as_deref()
            .is_some_and(|u| u.starts_with("https://github.com/"));
        let sha = commit.sha.clone();
        let weak = cx.entity().downgrade();
        Some(
            popover("history-card", Anchor::Point(card.at))
                .width(360.)
                .on_dismiss(move |_, cx| {
                    let _ = weak.update(cx, |this, cx| this.close_card(cx));
                })
                .child(
                    div()
                        .id("history-card-body")
                        .role(Role::Dialog)
                        .aria_label("Commit details")
                        .flex()
                        .flex_col()
                        .text_size(px(12.))
                        .text_color(content(0.85))
                        .child(
                            row()
                                .flex_wrap()
                                .child(icon("account-user", 14.).text_color(content(0.55)))
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(accent_color())
                                        .child(format!("{},", commit.author)),
                                )
                                .child(icon("clock", 12.).ml_1().text_color(content(0.55)))
                                .child(div().text_color(content(0.55)).child(when)),
                        )
                        .child(div().px_3().pb_2().child(message))
                        .when_some(stat, |d, stat| d.child(rule()).child(stat))
                        .when(!item.row.refs.is_empty(), |d| {
                            d.child(rule()).child(
                                row()
                                    .flex_wrap()
                                    .children(item.row.refs.iter().map(ref_pill)),
                            )
                        })
                        .child(rule())
                        .child(
                            row()
                                .gap_2()
                                .child(icon("git-commit", 14.).text_color(content(0.55)))
                                .child(
                                    div()
                                        .font_family(crate::fonts::mono())
                                        .text_color(accent_color())
                                        .child(commit.short_sha.clone()),
                                )
                                .child(
                                    icon_action(
                                        "history-card-copy",
                                        "Copy commit SHA",
                                        "copy",
                                        true,
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                sha.clone(),
                                            ));
                                            this.close_card(cx);
                                        },
                                    )),
                                )
                                .when_some(url, |d, url| {
                                    d.child(div().text_color(content(0.3)).child("|")).child(
                                        div()
                                            .id("history-card-open")
                                            .role(Role::Link)
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .cursor_pointer()
                                            .text_color(accent_color())
                                            .hover(|s| s.underline())
                                            .child(
                                                icon(
                                                    if github {
                                                        "inbox-github"
                                                    } else {
                                                        "external-link"
                                                    },
                                                    14.,
                                                )
                                                .text_color(accent_color()),
                                            )
                                            .child(if github {
                                                "Open on GitHub"
                                            } else {
                                                "Open in browser"
                                            })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.open_url(&url);
                                                this.close_card(cx);
                                            })),
                                    )
                                }),
                        ),
                )
                .into_any_element(),
        )
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let head = self.items.iter().find(|i| i.commit.head);
        let branch = head.and_then(|h| {
            h.commit
                .refs
                .iter()
                .find(|r| r.kind == "local")
                .map(|r| r.name.clone())
        });
        let label: SharedString = branch
            .clone()
            .or_else(|| head.map(|h| h.commit.short_sha.clone()))
            .unwrap_or_else(|| "History".into())
            .into();
        let can_refresh = self.available() && !self.loading;
        div()
            .flex_shrink_0()
            .child(pane_header("history-toolbar", "History").map(|d| {
                if self.search_open {
                    d.key_context("HistorySearch")
                        .on_action(cx.listener(|this, _: &CloseSearch, window, cx| {
                            this.close_search(window, cx)
                        }))
                        .child(icon("search", 14.).text_color(content(0.45)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .font_weight(FontWeight::NORMAL)
                                .text_color(content(1.))
                                .child(self.search.clone()),
                        )
                        .child(
                            header_action(
                                "history-close-search",
                                "Close history search",
                                "x",
                                12.,
                                true,
                                false,
                            )
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_search(window, cx)),
                            ),
                        )
                } else if self.section {
                    let open = self.open;
                    let code = self.code;
                    d.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .id("history-section")
                                    .role(Role::Button)
                                    .aria_label(if code { "Code" } else { "Graph" })
                                    .aria_expanded(open)
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .child(
                                        icon(
                                            if open {
                                                "chevron-down"
                                            } else {
                                                "chevron-right"
                                            },
                                            14.,
                                        )
                                        .text_color(content(0.5)),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open = !this.open;
                                        if this.open && this.code {
                                            this.load_languages(cx);
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                section_tab("graph-tab", "GRAPH", !code)
                                    .on_click(cx.listener(|this, _, _, cx| this.choose(false, cx))),
                            )
                            .child(div().w(px(1.)).h(px(10.)).mx(px(4.)).bg(content(0.15)))
                            .child(
                                section_tab("code-tab", "CODE", code)
                                    .on_click(cx.listener(|this, _, _, cx| this.choose(true, cx))),
                            ),
                    )
                    .when(open && !code, |d| d.child(self.actions(can_refresh, cx)))
                } else {
                    d.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(icon("git-branch-hi", 14.).text_color(content(0.45)))
                            .child(div().min_w_0().truncate().child(label)),
                    )
                    .child(self.actions(can_refresh, cx))
                }
            }))
            .when_some(self.error.clone(), |d, error| {
                d.child(
                    div()
                        .id("history-error")
                        .role(Role::Alert)
                        .border_b_1()
                        .border_color(stroke())
                        .px_3()
                        .py_2()
                        .text_size(px(11.))
                        .text_color(rgb(RED_400))
                        .child(error),
                )
            })
    }

    /// Search and refresh, shared by the tab's toolbar and the section header.
    fn actions(&self, can_refresh: bool, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(2.))
            .child(
                header_action(
                    "history-search",
                    "Search history",
                    "search",
                    14.,
                    true,
                    false,
                )
                .on_click(cx.listener(|this, _, window, cx| this.open_search(window, cx))),
            )
            .child(
                header_action(
                    "history-refresh",
                    "Refresh history",
                    "refresh-cw",
                    14.,
                    can_refresh,
                    self.loading,
                )
                .when(can_refresh, |d| {
                    d.on_click(cx.listener(|this, _, _, cx| this.load(cx)))
                }),
            )
    }

    fn row(&self, ix: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let item = &self.items[ix];
        let commit = &item.commit;
        let active = self.selected.as_deref() == Some(commit.sha.as_str());
        let hovered = self.hovered == Some(ix);
        let filtered = !self.query.trim().is_empty();
        let head = item.row.kind == NodeKind::Head;
        let badge = item
            .row
            .refs
            .iter()
            .find(|r| r.color.is_some())
            .or(item.row.refs.first())
            .cloned();
        let date = short_date(commit.timestamp);
        let graph = item.graph.clone();
        let width = if filtered {
            SWIMLANE_WIDTH * 2.
        } else {
            graph.width
        };
        let title: SharedString = {
            let mut t = format!("{} {}", commit.short_sha, commit.subject);
            if !commit.author.is_empty() {
                t.push_str(&format!(" — {}", commit.author));
            }
            t.into()
        };
        div()
            .id(("history-row", ix))
            .role(Role::Button)
            .aria_label(title)
            .aria_selected(active)
            .h(px(ROW))
            .w_full()
            .min_w_0()
            .flex()
            .items_stretch()
            .pl_1()
            .pr_3()
            .text_color(content(1.))
            .when(active, |d| d.bg(selection()))
            .when(!active, |d| d.hover(|s| s.bg(content(0.05))))
            .on_hover(cx.listener(move |this, hovering: &bool, _, cx| {
                let next = if *hovering {
                    Some(ix)
                } else if this.hovered == Some(ix) {
                    None
                } else {
                    this.hovered
                };
                if next != this.hovered {
                    this.hovered = next;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.select(ix, cx)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_card(ix, event.position, cx);
                }),
            )
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        paint_graph(&graph, bounds, filtered, head, active, hovered, window)
                    },
                )
                .w(px(width))
                .h(px(ROW))
                .flex_shrink_0(),
            )
            .child(
                div()
                    .ml_1()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .gap(px(2.))
                    .overflow_hidden()
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.))
                                    .line_height(px(16.))
                                    .when(head, |d| d.font_weight(FontWeight::MEDIUM))
                                    .child(if commit.subject.is_empty() {
                                        commit.short_sha.clone()
                                    } else {
                                        commit.subject.clone()
                                    }),
                            )
                            .when_some(badge, |d, badge| d.child(ref_pill(&badge))),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(10.))
                            .line_height(px(14.))
                            .text_color(content(0.4))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(commit.author.clone()),
                            )
                            .when_some(date, |d, date| d.child(div().flex_shrink_0().child(date)))
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .font_family(crate::fonts::mono())
                                    .text_color(content(0.3))
                                    .child(commit.short_sha.clone()),
                            ),
                    ),
            )
    }
}

/// GRAPH and CODE in the section header: the chosen one bright and underlined.
fn section_tab(id: &'static str, label: &'static str, active: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(Role::Tab)
        .aria_label(label)
        .aria_selected(active)
        .group(id)
        .h_full()
        .flex()
        .items_center()
        .child(
            div()
                .py(px(1.))
                .border_b_1()
                .text_size(px(10.))
                .font_weight(FontWeight::SEMIBOLD)
                .map(|d| {
                    if active {
                        d.border_color(content(0.75)).text_color(content(0.85))
                    } else {
                        d.border_color(gpui::transparent_black())
                            .text_color(content(0.4))
                            .group_hover(id, |s| s.text_color(content(0.7)))
                    }
                })
                .child(label),
        )
}

/// HistoryToolbar's 24px actions: 50% ink, 5% hover.
fn header_action(
    id: &'static str,
    label: &'static str,
    path: &'static str,
    side: f32,
    enabled: bool,
    spinning: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .group(id)
        .size(px(24.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .when(!enabled, |d| d.opacity(0.4))
        .when(enabled, |d| d.hover(|s| s.bg(content(0.05))))
        .child(if spinning {
            spinner(id, path, side)
        } else {
            icon(path, side)
                .text_color(content(0.5))
                .when(enabled, |i| {
                    i.group_hover(id, |s| s.text_color(content(1.)))
                })
                .into_any_element()
        })
}

fn ref_pill(r: &git_graph::GraphRef) -> impl IntoElement {
    let local = r.kind == "local";
    div()
        .ml_1()
        .h(px(14.))
        .min_w_0()
        .max_w(px(104.))
        .flex_shrink_0()
        .self_center()
        .flex()
        .items_center()
        .gap(px(2.))
        .px(px(6.))
        .rounded_full()
        .text_size(px(10.))
        .line_height(px(10.))
        .map(|d| match r.color {
            Some(color) => d.bg(rgb(color)).text_color(rgb(BACKGROUND_BASE)),
            None => d.bg(content(0.1)).text_color(content(0.55)),
        })
        .when(local, |d| {
            d.child(icon("git-branch-2", 10.).text_color(match r.color {
                Some(_) => rgb(BACKGROUND_BASE).into(),
                None => content(0.55),
            }))
        })
        .child(div().min_w_0().truncate().child(r.name.clone()))
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

/// The row SVG and index.css's `.git-history-item` cutouts: strokes in the
/// background colour, cleared or tinted on hover and selection.
fn paint_graph(
    graph: &RowGraph,
    bounds: Bounds<gpui::Pixels>,
    filtered: bool,
    head: bool,
    active: bool,
    hovered: bool,
    window: &mut Window,
) {
    let origin = bounds.origin;
    if !filtered {
        for path in &graph.paths {
            let color = soft(path.color).opacity(0.8);
            git_graph::stroke_path(&path.d, origin, path.stroke_width, color, window);
        }
    }
    let count = graph.circles.len();
    for (ix, circle) in graph.circles.iter().enumerate() {
        let cx = if filtered { SWIMLANE_WIDTH } else { circle.cx };
        let centre = origin + point(px(cx), px(circle.cy));
        let base: Hsla = rgb(BACKGROUND_BASE).into();
        let tint = if active {
            over_base(0.1)
        } else if hovered {
            over_base(0.05)
        } else {
            base
        };
        let stroke = match ix {
            0 if hovered => None,
            1 if active || hovered => Some(tint),
            _ => Some(base),
        };
        let fill_color = if head && ix + 1 == count {
            Some(tint)
        } else {
            circle.fill.map(soft)
        };
        let disc = |r: f32, color: Hsla, window: &mut Window| {
            window.paint_quad(
                fill(
                    Bounds::new(centre - point(px(r), px(r)), size(px(r * 2.), px(r * 2.))),
                    color,
                )
                .corner_radii(px(r)),
            );
        };
        if let Some(color) = fill_color {
            disc(circle.r, color, window);
        }
        if let Some(stroke) = stroke {
            let outer = circle.r + circle.stroke_width / 2.;
            window.paint_quad(gpui::quad(
                Bounds::new(
                    centre - point(px(outer), px(outer)),
                    size(px(outer * 2.), px(outer * 2.)),
                ),
                px(outer),
                gpui::transparent_black(),
                px(circle.stroke_width),
                stroke,
                gpui::BorderStyle::Solid,
            ));
        }
    }
}

impl Focusable for History {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for History {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let empty: Option<(&'static str, bool)> = if !self.available() {
            Some(("No project folder", false))
        } else if self.error.is_some() && self.items.is_empty() {
            None
        } else if self.items.is_empty() {
            Some((
                if self.loading {
                    "Loading history…"
                } else {
                    "No commits yet"
                },
                false,
            ))
        } else if self.visible.is_empty() {
            Some(("No matching commits", true))
        } else {
            None
        };
        let list_len = self.visible.len();
        let entity = cx.entity();
        div()
            .id("history")
            .role(if self.section {
                Role::Group
            } else {
                Role::TabPanel
            })
            .aria_label(if self.section { "Graph" } else { "History" })
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(self.toolbar(cx))
            .children(self.card(cx))
            .when(self.code && self.open, |d| {
                d.child(languages::view(self.languages.as_ref()))
            })
            .when(
                (!self.section || self.open) && !self.code,
                |d| match empty {
                    Some((text, centred)) => d.child(
                        div()
                            .px_3()
                            .when(centred, |d| d.py_4().text_center())
                            .when(!centred, |d| d.py_2())
                            .text_size(px(12.))
                            .text_color(content(0.45))
                            .child(text),
                    ),
                    None if self.items.is_empty() => d,
                    None => d.child(
                        uniform_list("history-rows", list_len, move |range, _, cx| {
                            entity.update(cx, |this, cx| {
                                range
                                    .filter_map(|i| this.visible.get(i).copied())
                                    .map(|ix| this.row(ix, cx).into_any_element())
                                    .collect::<Vec<_>>()
                            })
                        })
                        .track_scroll(&self.scroll)
                        .flex_1()
                        .min_h_0(),
                    ),
                },
            )
    }
}

impl History {
    #[allow(dead_code)]
    fn reveal(&self, ix: usize) {
        self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    #[gpui::test]
    fn code_tab_swaps_the_graph_for_languages_and_back(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("mcode-code-tab-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.rs"), "fn main() {}\n".repeat(10)).unwrap();
        std::fs::write(dir.join("app.ts"), "export {};\n").unwrap();
        std::fs::write(dir.join("README.md"), "# Not code\n".repeat(50)).unwrap();
        let root = dir.to_string_lossy().into_owned();
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |window, cx| {
                let git = cx.new(|cx| ProjectGit::new(root, cx));
                cx.new(|cx| {
                    let mut graph = History::new(git, true, window, cx);
                    graph.set_enabled(true, cx);
                    graph
                })
            })
            .unwrap()
        });
        let chosen = Rc::new(RefCell::new(Vec::new()));
        let entity = window.update(cx, |_, _, cx| cx.entity()).unwrap();
        let log = chosen.clone();
        let _events = cx.update(|cx| {
            cx.subscribe(&entity, move |_, event: &HistoryEvent, _| {
                if let HistoryEvent::Code(code) = event {
                    log.borrow_mut().push(*code);
                }
            })
        });
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        visual.refresh().unwrap();
        assert!(visual.debug_bounds("history-languages").is_none());

        let code = visual.debug_bounds("code-tab").unwrap();
        visual.simulate_click(code.center(), Default::default());
        visual.run_until_parked();
        visual.refresh().unwrap();
        assert!(
            visual.debug_bounds("history-languages").is_some(),
            "CODE shows the languages"
        );
        window
            .update(visual, |graph, _, _| {
                let shares = graph.languages.clone().unwrap().unwrap();
                let names: Vec<_> = shares.iter().map(|s| s.name).collect();
                assert_eq!(names, ["Rust", "TypeScript"], "Markdown isn't code");
            })
            .unwrap();
        assert_eq!(*chosen.borrow(), [true]);

        let graph = visual.debug_bounds("graph-tab").unwrap();
        visual.simulate_click(graph.center(), Default::default());
        visual.refresh().unwrap();
        assert!(
            visual.debug_bounds("history-languages").is_none(),
            "GRAPH brings the graph back"
        );
        assert_eq!(*chosen.borrow(), [true, false]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn softened_lane_ink_mixes_toward_content() {
        let mixed = soft(0xFFB000);
        let pure: Hsla = rgb(0xFFB000).into();
        assert!(mixed.s < pure.s);
        assert!((over_base(0.).l - Hsla::from(rgb(BACKGROUND_BASE)).l).abs() < 0.01);
    }
}
