//! GitChangesPanel.tsx in the inset layout: the Changes header with branch
//! actions, the commit box, sync and pull-request actions, and the staged
//! and unstaged files as a list or a tree. Every Git call runs off-thread,
//! one mutation at a time per checkout.
pub(crate) mod actions;
mod view;
use crate::{
    git::picker::{GitPicker, Mode as GitMode, Picked as GitPicked},
    runtime::git_status::{GitChanged, ProjectGit},
    ui::panel::*,
    ui::{
        file_icons::file_type_icon,
        input::{self, Input},
        popover::{
            Align, Anchor, AnchorSlot, Menu, MenuAction, MenuEvent, MenuItem, MenuStyle, Side,
            menu_popover, popover,
        },
        theme::*,
    },
};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FontWeight, Hsla, PromptLevel, Role,
    SharedString, Subscription, Task, Window, div, prelude::*, px, rgb, uniform_list,
};
use monocode_core::{
    provider::{self, Event, Kind, Request},
    session::agent::{AccessMode, Settings},
    vcs::git::scm::{self, ChangedFile, DiffIndex, Pr},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const MOD: &str = if cfg!(target_os = "macos") {
    "⌘"
} else {
    "Ctrl+"
};
const ROW: f32 = 28.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ChangesView {
    #[default]
    List,
    Tree,
}
pub(crate) enum ChangesEvent {
    View(ChangesView),
    /// A file's review, or all of one side's when `path` is None.
    OpenDiff {
        cwd: String,
        path: Option<String>,
        staged: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
enum Kind2 {
    Staged,
    Unstaged,
}
impl Kind2 {
    fn key(self) -> &'static str {
        match self {
            Self::Staged => "staged",
            Self::Unstaged => "unstaged",
        }
    }
}
#[derive(Clone, Copy, PartialEq)]
enum FileAction {
    Stage,
    Unstage,
    Discard,
}

/// One 28px line of the scrolling list.
#[derive(Clone)]
enum Line {
    Section(Kind2),
    File {
        kind: Kind2,
        file: usize,
        depth: Option<usize>,
    },
    Dir {
        kind: Kind2,
        path: String,
        name: String,
        depth: usize,
        status: Option<String>,
        open: bool,
    },
}

#[derive(Default)]
struct ChangeDir {
    name: String,
    path: String,
    dirs: Vec<ChangeDir>,
    files: Vec<usize>,
    status: Option<String>,
}
/// buildChangeTree / sortChangeDir: folders first, names compared as
/// `localeCompare` roughly does, and a status shared by every descendant.
fn build_tree(files: &[ChangedFile], members: &[usize]) -> ChangeDir {
    let mut root = ChangeDir::default();
    for &ix in members {
        let segments: Vec<&str> = files[ix].relative.split('/').collect();
        let mut node = &mut root;
        for segment in &segments[..segments.len() - 1] {
            let path = if node.path.is_empty() {
                segment.to_string()
            } else {
                format!("{}/{segment}", node.path)
            };
            let pos = match node.dirs.iter().position(|d| d.path == path) {
                Some(pos) => pos,
                None => {
                    node.dirs.push(ChangeDir {
                        name: segment.to_string(),
                        path,
                        ..Default::default()
                    });
                    node.dirs.len() - 1
                }
            };
            node = &mut node.dirs[pos];
        }
        node.files.push(ix);
    }
    sort_dir(&mut root, files);
    root
}
fn locale_key(s: &str) -> (String, String) {
    (s.to_lowercase(), s.to_owned())
}
fn sort_dir(dir: &mut ChangeDir, files: &[ChangedFile]) -> Option<String> {
    dir.dirs.sort_by_key(|d| locale_key(&d.name));
    dir.files
        .sort_by_key(|&ix| locale_key(basename(&files[ix].relative)));
    let mut status: Option<String> = None;
    let mut mixed = false;
    let mut merge = |next: Option<String>| match (next, &status) {
        (None, _) => mixed = true,
        (Some(n), None) => status = Some(n),
        (Some(n), Some(s)) if &n != s => mixed = true,
        _ => {}
    };
    for child in &mut dir.dirs {
        merge(sort_dir(child, files));
    }
    for &ix in &dir.files {
        merge(Some(files[ix].status.clone()));
    }
    dir.status = if mixed { None } else { status };
    dir.status.clone()
}
fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
fn dirname(relative: &str) -> &str {
    match relative.rfind('/') {
        Some(i) if i > 0 => &relative[..i],
        _ => "",
    }
}
fn status_letter(status: &str) -> &'static str {
    match status {
        "untracked" => "U",
        "added" => "A",
        "deleted" => "D",
        _ => "M",
    }
}
pub(crate) fn status_color(status: &str) -> Hsla {
    rgb(match status {
        "untracked" => SKY_400,
        "added" => DIFF_ADD,
        "deleted" => DIFF_DEL,
        _ => AMBER_400,
    })
    .into()
}
fn sync_status_label(index: &DiffIndex) -> String {
    let plural = |n: i64| if n == 1 { "" } else { "s" };
    if index.ahead > 0 && index.behind > 0 {
        format!(
            "Diverged from {}",
            index.upstream.as_deref().unwrap_or("upstream")
        )
    } else if index.ahead > 0 {
        format!("{} unpushed commit{}", index.ahead, plural(index.ahead))
    } else if index.behind > 0 {
        format!("{} incoming commit{}", index.behind, plural(index.behind))
    } else {
        "No files".into()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum MenuKind {
    Branch,
    Commit,
}
struct OpenMenu {
    kind: MenuKind,
    menu: Entity<Menu>,
    _events: Subscription,
}

pub(crate) struct Changes {
    git: Entity<ProjectGit>,
    cwd: String,
    enabled: bool,
    pub(crate) text_provider: Kind,
    message: Entity<Input>,
    amend: Option<(Option<String>, Option<String>)>,
    busy: Option<String>,
    status: Option<String>,
    status_clear: Option<Task<()>>,
    pr: Option<Pr>,
    pr_branch: Option<String>,
    pr_request: u64,
    generate_cancel: Option<Arc<AtomicBool>>,
    staged_open: bool,
    changes_open: bool,
    view: ChangesView,
    collapsed: HashSet<String>,
    selected: Option<(String, Kind2)>,
    /// The row under the pointer, which shows its actions. Kept as state
    /// rather than a group-hover `display`, so layout and paint always agree.
    hovered: Option<usize>,
    lines: Vec<Line>,
    menu: Option<OpenMenu>,
    branch_anchor: AnchorSlot,
    commit_anchor: AnchorSlot,
    /// The composer's branch picker, dropped down from the branch name.
    branch_picker: Option<(Entity<GitPicker>, Subscription)>,
    name_anchor: AnchorSlot,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<ChangesEvent> for Changes {}

impl Changes {
    pub(crate) fn new(git: Entity<ProjectGit>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let message = cx.new(Input::commit_message);
        let subscriptions = vec![
            cx.observe(&git, |this, _, cx| this.index_changed(cx)),
            cx.subscribe(&git, |this, _, _: &GitChanged, cx| this.index_changed(cx)),
            cx.subscribe(&message, |_, _, _: &input::Changed, cx| cx.notify()),
        ];
        let mut this = Self {
            git,
            cwd: String::new(),
            enabled: false,
            text_provider: Kind::Claude,
            message,
            amend: None,
            busy: None,
            status: None,
            status_clear: None,
            pr: None,
            pr_branch: None,
            pr_request: 0,
            generate_cancel: None,
            staged_open: true,
            changes_open: true,
            view: ChangesView::List,
            collapsed: HashSet::new(),
            selected: None,
            hovered: None,
            lines: Vec::new(),
            menu: None,
            branch_anchor: AnchorSlot::default(),
            commit_anchor: AnchorSlot::default(),
            branch_picker: None,
            name_anchor: AnchorSlot::default(),
            _subscriptions: subscriptions,
        };
        this.rebuild(cx);
        this
    }
    fn toggle_branch_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.branch_picker.take().is_some() {
            cx.notify();
            return;
        }
        self.menu = None;
        let cwd = PathBuf::from(&self.cwd);
        let picker = cx.new(|cx| GitPicker::new(cwd, GitMode::Branch, None, cx));
        let events = cx.subscribe(&picker, |this, _, event: &GitPicked, cx| {
            if matches!(event, GitPicked::Changed) {
                this.git.update(cx, |git, cx| git.changed(cx));
            }
            this.branch_picker = None;
            cx.notify();
        });
        window.focus(&picker.read(cx).focus(cx), cx);
        self.branch_picker = Some((picker, events));
        cx.notify();
    }
    pub(crate) fn set_view(&mut self, view: ChangesView, cx: &mut Context<Self>) {
        self.view = view;
        self.rebuild(cx);
    }
    pub(crate) fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.enabled == enabled {
            return;
        }
        self.enabled = enabled;
        if enabled {
            let request = self.pr_request;
            self.index_changed(cx);
            if self.pr_request == request {
                self.load_pr(cx);
            }
        }
    }
    pub(crate) fn resume(&mut self, cx: &mut Context<Self>) {
        if self.enabled {
            self.load_pr(cx);
        }
    }
    fn index(&self, cx: &App) -> Option<DiffIndex> {
        self.git.read(cx).index.clone()
    }
    fn index_changed(&mut self, cx: &mut Context<Self>) {
        let git = self.git.read(cx);
        let project = git.project().to_owned();
        if project != self.cwd {
            // Checkouts don't share drafts or in-flight generation.
            self.cwd = project;
            self.cancel_generate();
            self.amend = None;
            self.pr = None;
            self.pr_branch = None;
            self.selected = None;
            self.branch_picker = None;
            self.message
                .update(cx, |m, cx| m.set_text(String::new(), cx));
        }
        let index = self.git.read(cx).index.clone();
        if let Some((branch, head)) = &self.amend
            && (index.as_ref().map(|i| &i.branch) != Some(branch)
                || index.as_ref().map(|i| &i.head) != Some(head))
        {
            self.amend = None;
            self.message
                .update(cx, |m, cx| m.set_text(String::new(), cx));
        }
        let branch = index.as_ref().and_then(|i| i.branch.clone());
        if self.enabled && branch != self.pr_branch {
            self.pr_branch = branch;
            self.load_pr(cx);
        }
        self.rebuild(cx);
    }
    fn load_pr(&mut self, cx: &mut Context<Self>) {
        self.pr_request += 1;
        let request = self.pr_request;
        let branch = self
            .git
            .read(cx)
            .index
            .as_ref()
            .and_then(|i| i.branch.clone());
        if self.cwd.is_empty() || self.cwd == "~" || branch.is_none() {
            self.pr = None;
            return;
        }
        let path = PathBuf::from(&self.cwd);
        let work = cx
            .background_executor()
            .spawn(async move { scm::pr_status(&path) });
        cx.spawn(async move |this, cx| {
            let pr = work.await.ok().flatten();
            let _ = this.update(cx, |this, cx| {
                if this.pr_request == request {
                    this.pr = pr;
                    cx.notify();
                }
            });
        })
        .detach();
    }
    fn staged_unstaged(index: Option<&DiffIndex>) -> (Vec<usize>, Vec<usize>) {
        let files = index.map_or(&[][..], |i| &i.files[..]);
        (
            (0..files.len()).filter(|&i| files[i].staged).collect(),
            (0..files.len()).filter(|&i| files[i].unstaged).collect(),
        )
    }
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let index = self.git.read(cx).index.clone();
        let (staged, unstaged) = Self::staged_unstaged(index.as_ref());
        let files = index.as_ref().map_or(&[][..], |i| &i.files[..]);
        let mut lines = Vec::new();
        for (kind, members, open) in [
            (Kind2::Staged, &staged, self.staged_open),
            (Kind2::Unstaged, &unstaged, self.changes_open),
        ] {
            if members.is_empty() {
                continue;
            }
            lines.push(Line::Section(kind));
            if !open {
                continue;
            }
            match self.view {
                ChangesView::List => lines.extend(members.iter().map(|&file| Line::File {
                    kind,
                    file,
                    depth: None,
                })),
                ChangesView::Tree => {
                    let tree = build_tree(files, members);
                    self.push_dir(&tree, kind, 0, &mut lines);
                }
            }
        }
        self.lines = lines;
        self.hovered = None;
        cx.notify();
    }
    fn push_dir(&self, dir: &ChangeDir, kind: Kind2, depth: usize, lines: &mut Vec<Line>) {
        for child in &dir.dirs {
            let open = !self
                .collapsed
                .contains(&format!("{}:{}", kind.key(), child.path));
            lines.push(Line::Dir {
                kind,
                path: child.path.clone(),
                name: child.name.clone(),
                depth,
                status: child.status.clone(),
                open,
            });
            if open {
                self.push_dir(child, kind, depth + 1, lines);
            }
        }
        lines.extend(dir.files.iter().map(|&file| Line::File {
            kind,
            file,
            depth: Some(depth),
        }));
    }
}

/// A provider CLI and model for one-off text, `None` for its default model.
pub(crate) type TextHarness = (Kind, PathBuf, Option<String>);

/// pickTextHarness: `chosen` (the background model picked in Settings) when
/// installed, else `preferred`, then Claude Code, then Codex on their default
/// models.
pub(crate) fn pick_text_harness(
    preferred: Kind,
    chosen: Option<(Kind, Option<String>)>,
) -> Option<TextHarness> {
    chosen
        .and_then(|(kind, model)| provider::resolve(kind).map(|binary| (kind, binary, model)))
        .or_else(|| {
            [preferred, Kind::Claude, Kind::Codex]
                .into_iter()
                .find_map(|kind| provider::resolve(kind).map(|binary| (kind, binary, None)))
        })
}

impl Render for Changes {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let available = !self.cwd.is_empty() && self.cwd != "~";
        if !available {
            return div()
                .px_3()
                .py_2()
                .text_size(px(12.))
                .text_color(content(0.5))
                .child("No project folder")
                .into_any_element();
        }
        let index = self.git.read(cx).index.clone();
        let empty = match &index {
            None => Some("Loading changes…".to_owned()),
            // Outside any repository Git reports no branch and no HEAD.
            Some(i) if i.files.is_empty() && i.branch.is_none() && i.head.is_none() => {
                Some("Not a Git repository. Pick a folder with one in the Explorer.".into())
            }
            Some(i) if i.files.is_empty() => Some(if i.ahead > 0 || i.behind > 0 {
                sync_status_label(i)
            } else {
                "No uncommitted changes".into()
            }),
            _ => None,
        };
        let count = self.lines.len();
        let entity = cx.entity();
        let menu = self.menu.as_ref().map(|open| {
            let anchor = match open.kind {
                MenuKind::Branch => self.branch_anchor.clone(),
                MenuKind::Commit => self.commit_anchor.clone(),
            };
            menu_popover(&open.menu, Anchor::Slot(anchor), cx).align(Align::End)
        });
        let picker = self.branch_picker.as_ref().map(|(picker, _)| {
            let weak = cx.entity().downgrade();
            popover("changes-git-picker", Anchor::Slot(self.name_anchor.clone()))
                .side(Side::Bottom)
                .align(Align::Start)
                .width(300.)
                .height(320.)
                .on_dismiss(move |_, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.branch_picker = None;
                        cx.notify();
                    });
                })
                .child(picker.clone())
        });
        div()
            .id("changes")
            .role(Role::TabPanel)
            .aria_label("Changes")
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(self.header(cx))
            .child(self.commit_box(cx))
            .map(|d| match empty {
                Some(text) => d.child(
                    div().py_1().child(
                        div()
                            .px_3()
                            .py_2()
                            .text_size(px(12.))
                            .text_color(content(0.45))
                            .child(text),
                    ),
                ),
                None => d.child(
                    uniform_list("change-lines", count, move |range, _, cx| {
                        entity.update(cx, |this, cx| {
                            range.map(|ix| this.line(ix, cx)).collect::<Vec<_>>()
                        })
                    })
                    .py_1()
                    .flex_1()
                    .min_h_0(),
                ),
            })
            .children(menu)
            .children(picker)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
