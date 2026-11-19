//! Git-backed recordings of workspace changes during a turn, ported from
//! src-tauri/src/checkpoint/turn.rs. These snapshots record what changed.
//! A turn's baseline keeps the previous turn's pending files as they were
//! before it, so the latest recording covers the session's outstanding work.
//! Shared reviews require successful edit evidence for each displayed file,
//! without inferring which process owns individual lines. Synchronous: callers
//! run these on a background executor.
mod snapshot;
mod turns;
use snapshot::{capture_workspace, changes, overlay_baseline, resolved_turn_paths, Carried};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::vcs::git::scm::{expand_home, path_to_js, MAX_TEXT_FILE_BYTES};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointFile {
    pub path: String,
    pub relative: String,
    pub status: String,
    pub additions: i64,
    pub deletions: i64,
    /// True when an exact recorded before/after diff is available.
    pub exact: bool,
    /// False when restoring could overwrite a change made outside this session.
    pub undoable: bool,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointStatus {
    pub files: Vec<CheckpointFile>,
}

enum FileState {
    Contents(Vec<u8>),
    Missing,
    Skipped,
}

/// Per-session recordings under the app's data directory. Every operation
/// runs under one lock, as the Tauri `CheckpointStore` did.
#[derive(Clone)]
pub struct Store {
    root: PathBuf,
    gate: Arc<Mutex<()>>,
}

static STORE: OnceLock<Store> = OnceLock::new();

/// Install the app's store once, at `data_dir/checkpoints`.
pub fn init(root: PathBuf) {
    let _ = STORE.set(Store::new(root));
}

pub fn store() -> Option<&'static Store> {
    STORE.get()
}

impl Store {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            gate: Arc::new(Mutex::new(())),
        }
    }

    pub fn exclusive<T>(
        &self,
        operation: impl FnOnce(&Self) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = self
            .gate
            .lock()
            .map_err(|_| "Checkpoint store lock poisoned".to_string())?;
        operation(self)
    }

    fn session_dir(&self, session_id: &str) -> PathBuf {
        self.root.join(session_id)
    }

    /// Successful structured edits by this session's running turn.
    pub fn capture(&self, session_id: &str, cwd: &str, paths: &[String]) -> Result<(), String> {
        validate_id(session_id, "session")?;
        if paths.is_empty() {
            return Ok(());
        }
        self.capture_turn_paths(session_id, cwd, paths).map(|_| ())
    }

    pub fn status(&self, session_id: &str, cwd: &str) -> Result<CheckpointStatus, String> {
        validate_id(session_id, "session")?;
        Ok(self.turn_status(session_id, cwd)?.unwrap_or_default())
    }

    /// A presentation-only read, without persisting newly resolved paths.
    pub fn status_readonly(&self, session_id: &str, cwd: &str) -> Result<CheckpointStatus, String> {
        validate_id(session_id, "session")?;
        Ok(self
            .turn_status_inner(session_id, cwd, false)?
            .unwrap_or_default())
    }

    /// Native UI handles and desktop UUIDs can name the same conversation.
    /// Read its newest recording in place, without migrating either directory.
    pub fn status_readonly_aliases(
        &self,
        session_ids: &[String],
        cwd: &str,
    ) -> Result<CheckpointStatus, String> {
        for id in session_ids {
            validate_id(id, "session")?;
        }
        let latest = session_ids
            .iter()
            .enumerate()
            .filter_map(|(index, id)| {
                let modified = std::fs::metadata(self.session_dir(id).join("turn-review.json"))
                    .ok()?
                    .modified()
                    .ok()?;
                Some((modified, std::cmp::Reverse(index), id))
            })
            .max_by_key(|(modified, index, _)| (*modified, *index));
        latest.map_or_else(
            || Ok(CheckpointStatus::default()),
            |(_, _, id)| self.status_readonly(id, cwd),
        )
    }

    pub fn file_diff(
        &self,
        session_id: &str,
        cwd: &str,
        relative: &str,
    ) -> Result<crate::vcs::git::scm::FileDiff, String> {
        validate_id(session_id, "session")?;
        self.turn_file_diff(session_id, cwd, relative)?
            .ok_or_else(|| "Session changes are no longer available".into())
    }

    pub fn undo(
        &self,
        session_id: &str,
        cwd: &str,
        relative: Option<&str>,
    ) -> Result<CheckpointStatus, String> {
        validate_id(session_id, "session")?;
        Ok(self
            .undo_turn(session_id, cwd, relative)?
            .unwrap_or_default())
    }

    pub fn keep(
        &self,
        session_id: &str,
        cwd: &str,
        relative: Option<&str>,
    ) -> Result<CheckpointStatus, String> {
        validate_id(session_id, "session")?;
        Ok(self
            .keep_turn(session_id, cwd, relative)?
            .unwrap_or_default())
    }

    pub fn forget(&self, session_id: &str) -> Result<(), String> {
        validate_id(session_id, "session")?;
        self.forget_turns(session_id)?;
        let dir = self.session_dir(session_id);
        if dir.exists() {
            std::fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn turn_status(&self, session_id: &str, cwd: &str) -> Result<Option<CheckpointStatus>, String> {
        self.turn_status_inner(session_id, cwd, true)
    }

    fn turn_status_inner(
        &self,
        session_id: &str,
        cwd: &str,
        persist_resolved: bool,
    ) -> Result<Option<CheckpointStatus>, String> {
        let Some(mut review) = self.matching_turn_review(session_id, cwd)? else {
            return Ok(None);
        };
        let turn = completed_turn(&review)?;
        let root = PathBuf::from(&review.cwd);
        let head = run_git(&root, &["rev-parse", "--verify", "HEAD"])
            .ok()
            .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_string());
        let recorded = changes(&root, turn)?;
        let mut resolved = resolved_turn_paths(&root, turn, &recorded, head.as_deref())?;
        // Discarded outside the app, as from the changes panel: the file is
        // back to its contents before the session, so nothing is pending.
        resolved.extend(
            recorded
                .iter()
                .filter(|change| {
                    !turn.dismissed.contains(&change.relative)
                        && !resolved.contains(&change.relative)
                        && reverted(&root, change)
                })
                .map(|change| change.relative.clone())
                .collect::<Vec<_>>(),
        );
        if !resolved.is_empty() {
            // Dismiss accepted work durably so later edits cannot resurrect
            // this turn's card. Its saved snapshots remain available to review.
            review.latest.as_mut().unwrap().dismissed.extend(resolved);
            if persist_resolved {
                write_review(&self.session_dir(session_id), &review)?;
            }
        }
        let turn = completed_turn(&review)?;
        let can_restore =
            !turn.shared && head == turn.head && self.active_turns(&root, session_id)?.is_empty();
        let files = recorded
            .into_iter()
            .filter(|change| !turn.dismissed.contains(&change.relative))
            .filter(|change| !turn.shared || turn.edited.contains(&change.relative))
            .map(|change| {
                let undoable = can_restore && restorable(&root, &change);
                CheckpointFile {
                    path: path_to_js(&root.join(&change.relative)),
                    relative: change.relative,
                    status: change.status,
                    additions: change.additions,
                    deletions: change.deletions,
                    exact: true,
                    undoable,
                }
            })
            .collect();
        Ok(Some(CheckpointStatus { files }))
    }

    fn turn_file_diff(
        &self,
        session_id: &str,
        cwd: &str,
        relative: &str,
    ) -> Result<Option<crate::vcs::git::scm::FileDiff>, String> {
        let Some(review) = self.matching_turn_review(session_id, cwd)? else {
            return Ok(None);
        };
        let turn = completed_turn(&review)?;
        let root = Path::new(&review.cwd);
        let relative = resolve_turn_path(relative)?;
        let change = changes(root, turn)?
            .into_iter()
            .find(|change| change.relative == relative)
            .ok_or("This file did not change during the recorded turn")?;
        let original = read_blob(root, change.before.as_ref())?;
        let current = read_blob(root, change.after.as_ref())?;
        let too_large =
            matches!(original, FileState::Skipped) || matches!(current, FileState::Skipped);
        let binary = state_is_binary(&original) || state_is_binary(&current);
        let (original, current) = if too_large || binary {
            (String::new(), String::new())
        } else {
            (state_text(original), state_text(current))
        };
        Ok(Some(crate::vcs::git::scm::FileDiff {
            path: path_to_js(&root.join(&relative)),
            relative,
            status: change.status,
            original,
            current,
            binary,
            too_large,
        }))
    }

    fn keep_turn(
        &self,
        session_id: &str,
        cwd: &str,
        relative: Option<&str>,
    ) -> Result<Option<CheckpointStatus>, String> {
        let Some(mut review) = self.matching_turn_review(session_id, cwd)? else {
            return Ok(None);
        };
        let recorded = completed_turn(&review)?;
        let root = Path::new(&review.cwd);
        let dismissed = match relative {
            Some(relative) => vec![resolve_turn_path(relative)?],
            None => changes(root, recorded)?
                .into_iter()
                .map(|change| change.relative)
                .collect(),
        };
        review.latest.as_mut().unwrap().dismissed.extend(dismissed);
        write_review(&self.session_dir(session_id), &review)?;
        self.turn_status(session_id, cwd)
    }
}

fn review_comparison_path(path: &Path) -> PathBuf {
    // path_to_js replaces Windows separators, including in canonical verbatim
    // paths. Restore native separators before comparing path components.
    if cfg!(windows) {
        PathBuf::from(path.to_string_lossy().replace('/', "\\"))
    } else {
        path.to_path_buf()
    }
}

fn completed_turn(review: &TurnReview) -> Result<&RecordedTurn, String> {
    let turn = review
        .active
        .as_ref()
        .or(review.latest.as_ref())
        .ok_or("No recorded turn is available")?;
    if let Some(error) = &turn.error {
        return Err(format!("Could not record workspace changes: {error}"));
    }
    if review.active.is_some() {
        return Err("The turn ended without a final workspace checkpoint".into());
    }
    if turn.before.is_none() || turn.after.is_none() {
        return Err("The turn's workspace checkpoints are incomplete".into());
    }
    Ok(turn)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecordedTurn {
    id: String,
    before: Option<String>,
    after: Option<String>,
    head: Option<String>,
    shared: bool,
    /// The process recording the active turn. Older recordings have none and
    /// count as abandoned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner: Option<TurnOwner>,
    error: Option<String>,
    #[serde(default)]
    dismissed: BTreeSet<String>,
    /// Files reported by this session's successful structured edit tools.
    /// Older shared snapshots have no attribution evidence and stay out of the card.
    #[serde(default)]
    edited: BTreeSet<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct TurnOwner {
    pid: u32,
    /// Tells this launch apart from an earlier process that had the same pid.
    launch: String,
}

impl TurnOwner {
    fn current() -> Self {
        Self {
            pid: std::process::id(),
            launch: launch_id().to_owned(),
        }
    }

    fn running(&self) -> bool {
        if self.pid == std::process::id() {
            return self.launch == launch_id();
        }
        process_running(self.pid)
    }
}

fn launch_id() -> &'static str {
    static LAUNCH: OnceLock<String> = OnceLock::new();
    LAUNCH.get_or_init(new_turn_id)
}

#[cfg(unix)]
fn process_running(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // Signal 0 only checks that the process exists.
    let found = unsafe { libc::kill(pid, 0) } == 0;
    found || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Without a cheap check, another process's turn stays shared.
#[cfg(not(unix))]
fn process_running(_pid: u32) -> bool {
    true
}

#[derive(Serialize, Deserialize)]
struct TurnReview {
    cwd: String,
    repo: String,
    active: Option<RecordedTurn>,
    latest: Option<RecordedTurn>,
}

#[derive(Serialize, Deserialize)]
struct SavedTurn {
    cwd: String,
    repo: String,
    turn: RecordedTurn,
}

#[derive(Clone, PartialEq, Eq)]
struct Blob {
    mode: String,
    oid: String,
}

struct Change {
    relative: String,
    before: Option<Blob>,
    after: Option<Blob>,
    status: String,
    additions: i64,
    deletions: i64,
}

fn turn_ref(session: &str, turn: &str, side: &str) -> String {
    format!("refs/monocode/checkpoints/{session}/{turn}/{side}")
}

// Git's NUL-delimited output already contains literal paths. Never trim them
// or interpret rename notation: those characters can be part of a filename.
fn resolve_turn_path(relative: &str) -> Result<String, String> {
    if relative.is_empty()
        || relative.contains('\0')
        || Path::new(relative).is_absolute()
        || Path::new(relative)
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
        || relative
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".." | ".git"))
    {
        return Err("Invalid checkpoint path".into());
    }
    Ok(relative.into())
}

fn resolve_edit_path(workspace: &Path, root: &Path, path: &str) -> Result<String, String> {
    let path = expand_home(Path::new(path));
    let relative = if path.is_absolute() {
        let relative = path
            .strip_prefix(workspace)
            .or_else(|_| path.strip_prefix(root))
            .map_err(|_| "Path is outside the project")?;
        path_to_js(relative)
    } else {
        path_to_js(&path)
    };
    resolve_turn_path(&relative)
}

fn git_command(root: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
        .args(["--no-pager", "-C"])
        .arg(root)
        .args(["-c", "core.fsmonitor=false"])
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .env(
            "PATH",
            crate::vcs::git::scm::launch_env::cached_gui_search_path(),
        );
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
    ] {
        command.env_remove(name);
    }
    command
}

fn checked_output(command: &mut Command) -> Result<Vec<u8>, String> {
    let output = command.output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(output.stdout)
}

fn run_git(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    checked_output(&mut git_command(root, args))
}

/// The file is back to its recorded baseline: absent when the session
/// created it, or a regular file with the earlier blob and mode.
fn reverted(root: &Path, change: &Change) -> bool {
    if path_contains_symlink(root, &change.relative) {
        return false;
    }
    workspace_matches(root, &change.relative, change.before.as_ref())
}

fn workspace_matches(root: &Path, relative: &str, blob: Option<&Blob>) -> bool {
    let path = root.join(relative);
    match (blob, std::fs::symlink_metadata(&path)) {
        (None, Err(error)) => error.kind() == std::io::ErrorKind::NotFound,
        (Some(blob), Ok(meta)) if meta.is_file() => {
            if !matches!(blob.mode.as_str(), "100644" | "100755") {
                return false;
            }
            let mode = if file_mode(&path).is_some_and(|mode| mode & 0o111 != 0) {
                "100755"
            } else {
                "100644"
            };
            mode == blob.mode
                && run_git(
                    root,
                    &["hash-object", &format!("--path={relative}"), "--", relative],
                )
                .is_ok_and(|bytes| String::from_utf8_lossy(&bytes).trim() == blob.oid)
        }
        _ => false,
    }
}

fn restorable(root: &Path, change: &Change) -> bool {
    if path_contains_symlink(root, &change.relative)
        || [change.before.as_ref(), change.after.as_ref()]
            .into_iter()
            .flatten()
            .any(|blob| !matches!(blob.mode.as_str(), "100644" | "100755"))
    {
        return false;
    }
    workspace_matches(root, &change.relative, change.after.as_ref())
}

fn read_blob(root: &Path, blob: Option<&Blob>) -> Result<FileState, String> {
    let Some(blob) = blob else {
        return Ok(FileState::Missing);
    };
    if blob.mode == "160000" {
        return Ok(FileState::Contents(
            format!("Subproject commit {}\n", blob.oid).into_bytes(),
        ));
    }
    let size = run_git(root, &["cat-file", "-s", &blob.oid])?;
    let size: u64 = String::from_utf8_lossy(&size)
        .trim()
        .parse()
        .map_err(|_| "Invalid checkpoint blob size")?;
    if size > MAX_TEXT_FILE_BYTES {
        return Ok(FileState::Skipped);
    }
    Ok(FileState::Contents(run_git(
        root,
        &["cat-file", "blob", &blob.oid],
    )?))
}

fn read_review(dir: &Path) -> Result<Option<TurnReview>, String> {
    match std::fs::read(dir.join("turn-review.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| e.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn write_review(dir: &Path, review: &TurnReview) -> Result<(), String> {
    write_json(&dir.join("turn-review.json"), review)
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let parent = path.parent().ok_or("Invalid checkpoint metadata path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec(value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    std::fs::rename(tmp, path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    std::fs::File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn unique_id() -> String {
    format!("{:032x}", rand::random::<u128>())
}

/// A turn id for `begin_turn`.
pub fn new_turn_id() -> String {
    unique_id()
}

fn project_root(cwd: &str) -> Result<PathBuf, String> {
    let trimmed = cwd.trim();
    if trimmed.is_empty() || trimmed == "~" {
        return Err("cwd is required".into());
    }
    let root = expand_home(Path::new(trimmed));
    if !root.is_dir() {
        return Err(format!("{}: Not a directory", root.display()));
    }
    Ok(root)
}

fn validate_id(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(format!("Invalid {label} id"));
    }
    Ok(())
}

fn path_contains_symlink(root: &Path, relative: &str) -> bool {
    let mut current = root.to_path_buf();
    for part in relative.split('/') {
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => return true,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
            Err(_) => return true,
        }
    }
    false
}

#[cfg(unix)]
fn file_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::symlink_metadata(path)
        .ok()
        .filter(|meta| meta.is_file() && !meta.file_type().is_symlink())
        .map(|meta| meta.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn file_mode(_path: &Path) -> Option<u32> {
    None
}

fn state_is_binary(state: &FileState) -> bool {
    matches!(state, FileState::Contents(bytes) if bytes.contains(&0))
}

fn state_text(state: FileState) -> String {
    match state {
        FileState::Contents(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        FileState::Missing | FileState::Skipped => String::new(),
    }
}
