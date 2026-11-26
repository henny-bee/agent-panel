//! The project's Git status, shared by the right panel and the title bar:
//! GitChangesPanel's `useDiffIndex`, useGitFileStatuses and
//! useProjectDiffStats over one off-thread reader. Reads never overlap; a
//! read requested meanwhile runs once the current one returns, and only a
//! changed snapshot notifies.
use gpui::{Context, EventEmitter, Task};
use monocode_core::vcs::git::scm::{self, ChangedFile, DiffIndex, DiffStats};
use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, Instant},
};

/// Without a file watcher, visible status polls on this interval.
const POLL: Duration = Duration::from_secs(2);
const RESUME_TTL: Duration = Duration::from_secs(30);
/// A burst of writes settles into one read this long after the last.
const SETTLE: Duration = Duration::from_millis(300);
/// Watched status still reads this often, in case an event went missing.
const REPAIR: Duration = Duration::from_secs(30);

/// A disk or Git change: Changes, History and the Explorer reread.
pub(crate) struct GitChanged;

#[derive(Default, Clone, PartialEq)]
pub(crate) struct StatusMap {
    pub files: HashMap<String, String>,
    pub dirs: HashMap<String, String>,
}
fn priority(status: &str) -> u8 {
    match status {
        "modified" => 3,
        "deleted" => 2,
        "added" | "untracked" => 1,
        _ => 0,
    }
}
/// useGitFileStatuses's `buildStatusMaps`: each folder takes its most
/// significant descendant status.
pub(crate) fn status_maps(index: &DiffIndex, root: &str) -> StatusMap {
    let mut map = StatusMap::default();
    for file in &index.files {
        map.files.insert(file.path.clone(), file.status.clone());
        let prio = priority(&file.status);
        if prio == 0 {
            continue;
        }
        let mut dir = parent(&file.path);
        while dir.len() > root.len() {
            let existing = map.dirs.get(dir).map_or(0, |s| priority(s));
            if prio <= existing {
                break;
            }
            map.dirs.insert(dir.to_owned(), file.status.clone());
            dir = parent(dir);
        }
    }
    map
}
fn parent(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

pub(crate) struct ProjectGit {
    /// The active session's working folder (its worktree, if it has one):
    /// Sidebar.tsx's `gitRoot`.
    project: String,
    pub root: String,
    pub index: Option<DiffIndex>,
    pub stats: Option<DiffStats>,
    pub statuses: StatusMap,
    /// Bumped on every observed change, for views that reread on it.
    pub epoch: u64,
    pub review_cache: crate::git::review::ReviewCache,
    versions: Option<HashMap<String, scm::FileDiffVersion>>,
    remembered: HashMap<String, (u64, DiffIndex)>,
    clock: u64,
    in_flight: bool,
    pending: bool,
    /// Bumped by every change MCode makes; a read begun before one is stale.
    mutations: u64,
    stats_loaded: Option<Instant>,
    /// Reads while status is visible: a file watcher, or a timer without one.
    poll: Option<[Task<()>; 2]>,
}
impl EventEmitter<GitChanged> for ProjectGit {}

impl ProjectGit {
    pub(crate) fn new(project: String, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            root: project.clone(),
            project: String::new(),
            index: None,
            stats: None,
            statuses: StatusMap::default(),
            epoch: 0,
            review_cache: Default::default(),
            versions: None,
            remembered: HashMap::new(),
            clock: 0,
            in_flight: false,
            pending: false,
            mutations: 0,
            stats_loaded: None,
            poll: None,
        };
        this.set_project(project, cx);
        this
    }
    pub(crate) fn project(&self) -> &str {
        &self.project
    }
    pub(crate) fn available(&self) -> bool {
        !self.project.is_empty() && self.project != "~"
    }
    /// The first frame of a switch already shows the new checkout's last
    /// snapshot, never the previous one's files and actions.
    pub(crate) fn set_project(&mut self, project: String, cx: &mut Context<Self>) {
        if project == self.project {
            return;
        }
        self.project = project.clone();
        self.root = project.clone();
        self.index = self.remembered.get(&project).map(|(_, i)| i.clone());
        self.stats = self.index.as_ref().map(stats_of);
        self.statuses = self
            .index
            .as_ref()
            .map(|i| status_maps(i, &self.root))
            .unwrap_or_default();
        self.stats_loaded = None;
        self.versions = None;
        self.epoch += 1;
        cx.emit(GitChanged);
        cx.notify();
        self.warm_choices(cx);
        if self.poll.take().is_some() {
            // Watch the new checkout instead; this reads it straight away.
            self.set_polling(true, cx);
        } else {
            self.reload(cx);
        }
    }
    /// While the project's Git status is visible, read it again whenever the
    /// checkout or its Git directory changes on disk, and every 30s besides.
    /// Without a file watcher, poll every 2s instead.
    pub(crate) fn set_polling(&mut self, polling: bool, cx: &mut Context<Self>) {
        if polling == self.poll.is_some() {
            return;
        }
        if !polling {
            self.poll = None;
            return;
        }
        self.reload(cx);
        let project = PathBuf::from(&self.project);
        let watched = cx.spawn(async move |this, cx| {
            let watch = cx
                .background_executor()
                .spawn(async move {
                    let mut roots = vec![project.clone()];
                    roots.extend(
                        scm::process::git_dirs(&project)
                            .into_iter()
                            .filter(|dir| !dir.starts_with(&project)),
                    );
                    crate::runtime::fs_watch::watch(&roots)
                })
                .await;
            let Some((_watch, changes)) = watch else {
                loop {
                    cx.background_executor().timer(POLL).await;
                    if this.update(cx, |this, cx| this.reload(cx)).is_err() {
                        return;
                    }
                }
            };
            while changes.recv().await.is_ok() {
                cx.background_executor().timer(SETTLE).await;
                while changes.try_recv().is_ok() {}
                if this.update(cx, |this, cx| this.reload(cx)).is_err() {
                    return;
                }
            }
        });
        let repair = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(REPAIR).await;
                if this.update(cx, |this, cx| this.reload(cx)).is_err() {
                    break;
                }
            }
        });
        self.poll = Some([watched, repair]);
    }
    /// useProjectDiffStats reads again on focus once its snapshot is 30s old.
    pub(crate) fn resume(&mut self, cx: &mut Context<Self>) {
        if self
            .stats_loaded
            .is_none_or(|at| at.elapsed() >= RESUME_TTL)
            || self.poll.is_some()
        {
            self.warm_choices(cx);
            self.reload(cx);
        }
    }
    /// A mutation or file operation: reread now and tell every view.
    pub(crate) fn changed(&mut self, cx: &mut Context<Self>) {
        self.mutations += 1;
        self.epoch += 1;
        cx.emit(GitChanged);
        self.warm_choices(cx);
        self.reload(cx);
    }
    /// Shows a stage or unstage of `relative` (a file or folder; `None` for
    /// every file) before Git runs it, since a reread takes most of a second.
    /// The reread after the command confirms it, or undoes it on failure.
    pub(crate) fn assume_staged(
        &mut self,
        relative: Option<&str>,
        staged: bool,
        cx: &mut Context<Self>,
    ) {
        self.mutations += 1;
        if let Some(index) = &mut self.index {
            assume_staged(&mut index.files, relative, staged);
            self.statuses = status_maps(index, &self.root);
            cx.notify();
        }
    }
    /// Project selection warms both menus before the user opens either one.
    fn warm_choices(&self, cx: &Context<Self>) {
        if !self.available() {
            return;
        }
        let project = self.project.clone();
        cx.background_executor()
            .spawn(async move {
                let _ = crate::runtime::git_cache::load(std::path::Path::new(&project));
            })
            .detach();
    }
    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        if !self.available() {
            return;
        }
        if self.in_flight {
            self.pending = true;
            return;
        }
        self.in_flight = true;
        let mutations = self.mutations;
        let project = self.project.clone();
        let path = PathBuf::from(&project);
        let work = cx.background_executor().spawn(async move {
            let index = scm::diff_index(&path)?;
            let versions = scm::file_diff_versions(&path, &index).ok();
            Ok::<_, String>((index, versions))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.in_flight = false;
                // A slow or unavailable checkout keeps its last snapshot, and
                // never takes another checkout's. A read begun before a change
                // would undo a stage shown early; the change's own reread follows.
                if this.project == project
                    && this.mutations == mutations
                    && let Ok((next, versions)) = result
                {
                    this.stats_loaded = Some(Instant::now());
                    this.remember(&project, &next);
                    if this.index.as_ref() != Some(&next) || this.versions != versions {
                        let had = this.index.is_some();
                        this.stats = Some(stats_of(&next));
                        this.statuses = status_maps(&next, &this.root);
                        this.index = Some(next);
                        this.versions = versions;
                        if had {
                            this.epoch += 1;
                            cx.emit(GitChanged);
                        }
                        cx.notify();
                    }
                }
                if std::mem::take(&mut this.pending) {
                    this.reload(cx);
                }
            });
        })
        .detach();
    }
    /// rememberIndex: recent checkouts open on their last snapshot, within 24
    /// checkouts and 20,000 files.
    fn remember(&mut self, project: &str, index: &DiffIndex) {
        self.clock += 1;
        self.remembered
            .insert(project.to_owned(), (self.clock, index.clone()));
        let mut files: usize = self.remembered.values().map(|(_, i)| i.files.len()).sum();
        while self.remembered.len() > 24 || files > 20_000 {
            let Some(oldest) = self
                .remembered
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some((_, gone)) = self.remembered.remove(&oldest) {
                files -= gone.files.len();
            }
        }
    }
    pub(crate) fn has_changes(&self) -> bool {
        self.stats.as_ref().is_some_and(|s| s.files > 0)
    }
}
/// What `git add` or `git restore --staged` leaves on `relative` and the
/// files under it: all staged or all unstaged. A new file staged is
/// `added`; unstaged again it is `untracked`.
fn assume_staged(files: &mut [ChangedFile], relative: Option<&str>, staged: bool) {
    let under = |path: &str| {
        relative.is_none_or(|folder| {
            path.strip_prefix(folder)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        })
    };
    for file in files.iter_mut().filter(|file| under(&file.relative)) {
        file.staged = staged;
        file.unstaged = !staged;
        match (file.status.as_str(), staged) {
            ("untracked", true) => file.status = "added".into(),
            ("added", false) => file.status = "untracked".into(),
            _ => {}
        }
    }
}
fn stats_of(index: &DiffIndex) -> DiffStats {
    DiffStats {
        files: index.files.len() as i64,
        additions: index.additions,
        deletions: index.deletions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn file(path: &str, status: &str) -> ChangedFile {
        ChangedFile {
            path: path.into(),
            relative: path.trim_start_matches("/r/").into(),
            status: status.into(),
            additions: 0,
            deletions: 0,
            staged: false,
            unstaged: true,
        }
    }
    #[test]
    fn folders_take_their_most_significant_status() {
        let index = DiffIndex {
            files: vec![
                file("/r/a/b/new.rs", "untracked"),
                file("/r/a/b/old.rs", "modified"),
                file("/r/a/gone.rs", "deleted"),
            ],
            ..Default::default()
        };
        let map = status_maps(&index, "/r");
        assert_eq!(map.dirs["/r/a/b"], "modified");
        assert_eq!(map.dirs["/r/a"], "modified");
        assert!(!map.dirs.contains_key("/r"));
        assert_eq!(map.files["/r/a/gone.rs"], "deleted");
    }
    #[test]
    fn an_assumed_stage_moves_a_folder_and_flips_new_files() {
        let mut files = vec![
            file("/r/a/new.rs", "untracked"),
            file("/r/a/b/old.rs", "modified"),
            file("/r/ab.rs", "modified"),
        ];
        let flags = |f: &ChangedFile| (f.staged, f.unstaged, f.status.clone());
        assume_staged(&mut files, Some("a"), true);
        assert_eq!(flags(&files[0]), (true, false, "added".into()));
        assert_eq!(flags(&files[1]), (true, false, "modified".into()));
        assert_eq!(
            flags(&files[2]),
            (false, true, "modified".into()),
            "a sibling that only shares the name prefix stays"
        );
        assume_staged(&mut files, None, false);
        assert_eq!(flags(&files[0]), (false, true, "untracked".into()));
        assert!(files.iter().all(|f| !f.staged && f.unstaged));
    }
}
