//! Source control for the Changes and History panes, ported from
//! src-tauri/src/fs.rs. Synchronous: callers run these on a background executor.
mod diff_index;
mod github;
pub(crate) mod launch_env;
mod log;
pub mod process;
mod review;
mod sync;
pub(crate) mod text;
use diff_index::{
    add_name_status, add_numstat_map, diff_index_with, diff_stats_for, normalize_diff_path,
    parse_shortstat, FileAcc,
};
use github::{gh_run, github_repo_for, pr_status_for};
#[cfg(test)]
use github::{github_pr_head_filter, parse_gh_pr_list, split_github_repo};
use log::history_for;
#[cfg(test)]
use log::{parse_decorations, parse_history_log};
#[cfg(test)]
use process::git_cmd_for_args_with_path;
use process::{
    git_branch, git_checked, git_cmd, git_is_work_tree, git_output, git_run, git_stdout,
    resolve_repo_path,
};
use sync::{git_default_branch, git_ref_exists, git_remote_name, sync_for};

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::files::remote as remote_workspace;
use serde::{Deserialize, Serialize};
use serde_json::json;

pub(crate) use launch_env::{expand_home, path_to_js};
pub use review::{
    commit_file_diff, commit_files, diff_files, file_diff, file_diff_versions, review_index,
    stage_contents, staged_only, FileDiff, FileDiffVersion, MAX_TEXT_FILE_BYTES,
};

/// The PATH a login terminal would have, for shells the app starts: a
/// Finder-launched app inherits launchd's bare PATH.
pub fn terminal_search_path() -> String {
    launch_env::gui_search_path()
}
/// Resolve a CLI using the login-shell PATH available to Finder-launched apps.
pub fn resolve_gui_binary(name: &str) -> Option<std::path::PathBuf> {
    launch_env::resolve_gui_binary(name)
}
pub use text::{commit_message_prompt, parse_commit_message, parse_pr_content, pr_content_prompt};

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiffStats {
    pub files: i64,
    pub additions: i64,
    pub deletions: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    pub path: String,
    pub relative: String,
    pub status: String,
    pub additions: i64,
    pub deletions: i64,
    pub staged: bool,
    pub unstaged: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiffIndex {
    pub branch: Option<String>,
    pub head: Option<String>,
    pub files: Vec<ChangedFile>,
    pub additions: i64,
    pub deletions: i64,
    pub remote: Option<String>,
    pub upstream: Option<String>,
    pub default_branch: Option<String>,
    pub ahead: i64,
    pub behind: i64,
    pub ahead_of_default: i64,
    pub head_pushed: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRef {
    pub name: String,
    /// `"local"`, `"remote"`, or `"tag"`.
    pub kind: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryCommit {
    pub sha: String,
    pub short_sha: String,
    pub parents: Vec<String>,
    pub author: String,
    pub timestamp: i64,
    pub subject: String,
    pub refs: Vec<HistoryRef>,
    pub head: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct History {
    pub head: Option<String>,
    pub commits: Vec<HistoryCommit>,
}

/// The Graph's right-click card: what a history row leaves out.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommitDetails {
    pub message: String,
    pub files: i64,
    pub additions: i64,
    pub deletions: i64,
    /// The commit's page on its hosted remote.
    pub url: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StagedContext {
    pub branch: Option<String>,
    pub summary: String,
    pub patch: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RangeContext {
    pub base: String,
    pub head: String,
    pub commit_summary: String,
    pub diff_summary: String,
    pub diff_patch: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Pr {
    pub number: i64,
    pub title: String,
    pub url: String,
    pub state: String,
}

const HISTORY_DEFAULT: u32 = 200;
const HISTORY_MAX: u32 = 500;
const MAX_UNTRACKED_BYTES: u64 = 1024 * 1024;

/// Uncommitted line counts: staged + unstaged vs HEAD, plus untracked
/// (gitignore-aware) files counted as additions.
pub fn diff_stats(cwd: &Path) -> Result<DiffStats, String> {
    if remote_workspace::is_remote(cwd) {
        return remote_workspace::command("git_diff_stats", json!({"cwd":cwd}));
    }
    Ok(diff_stats_for(&expand_home(cwd)))
}

/// Changed files under `cwd` (paths relative to it), with per-file line counts,
/// status, and branch/upstream synchronization metadata.
pub fn diff_index(cwd: &Path) -> Result<DiffIndex, String> {
    if remote_workspace::is_remote(cwd) {
        return remote_workspace::command("git_diff_index", json!({"cwd":cwd}));
    }
    Ok(diff_index_with(&expand_home(cwd), true))
}

/// Recent commits for the Graph view: HEAD, upstream, and the default branch.
/// Newest first, with parent SHAs for the graph.
pub fn history(cwd: &Path, limit: Option<u32>) -> Result<History, String> {
    if remote_workspace::is_remote(cwd) {
        return remote_workspace::command("git_history", json!({"cwd":cwd,"limit":limit}));
    }
    history_for(&expand_home(cwd), limit)
}

/// One commit's full message, `--shortstat` and web page.
pub fn commit_details(cwd: &Path, sha: &str) -> Result<CommitDetails, String> {
    if remote_workspace::is_remote(cwd) {
        return Err("Commit details are local only".into());
    }
    let root = expand_home(cwd);
    let text = git_run(&root, &["show", "--shortstat", "--format=%B%x00", sha])
        .ok_or_else(|| format!("Could not read commit {sha}"))?;
    let (message, stat) = text.split_once('\0').unwrap_or((&text, ""));
    let mut details = parse_shortstat(stat);
    details.message = message.trim().to_owned();
    details.url = git_remote_name(&root)
        .and_then(|remote| git_stdout(&root, &["remote", "get-url", &remote]))
        .and_then(|remote| crate::vcs::git::report::web_url(&remote))
        .map(|(web, _)| format!("{web}/commit/{sha}"));
    Ok(details)
}

/// Stage a changed file or folder (`git add`).
pub fn stage_file(cwd: &Path, relative: &str) -> Result<(), String> {
    if remote_workspace::is_remote(cwd) {
        return remote_workspace::command("git_stage_file", json!({"cwd":cwd,"relative":relative}));
    }
    let root = expand_home(cwd);
    let relative = resolve_repo_path(&root, relative)?;
    git_checked(&root, &["--literal-pathspecs", "add", "--", &relative])
}

/// Unstage a file or folder (`git restore --staged`).
pub fn unstage_file(cwd: &Path, relative: &str) -> Result<(), String> {
    if remote_workspace::is_remote(cwd) {
        return remote_workspace::command(
            "git_unstage_file",
            json!({"cwd":cwd,"relative":relative}),
        );
    }
    let root = expand_home(cwd);
    let relative = resolve_repo_path(&root, relative)?;
    git_checked(
        &root,
        &[
            "--literal-pathspecs",
            "restore",
            "--staged",
            "--",
            &relative,
        ],
    )
}

/// Discard unstaged changes so the file matches the index (or delete if untracked).
pub fn discard_file(cwd: &Path, relative: &str) -> Result<(), String> {
    if remote_workspace::is_remote(cwd) {
        return remote_workspace::command(
            "git_discard_file",
            json!({"cwd":cwd,"relative":relative}),
        );
    }
    discard_file_for(&expand_home(cwd), relative)
}

/// Stage every changed file in the repo.
pub fn stage_all(cwd: &Path) -> Result<(), String> {
    if remote_workspace::is_remote(cwd) {
        return remote_workspace::command("git_stage_all", json!({"cwd":cwd}));
    }
    git_checked(&expand_home(cwd), &["add", "-A", "--", "."])
}

/// Unstage every staged file.
pub fn unstage_all(cwd: &Path) -> Result<(), String> {
    if remote_workspace::is_remote(cwd) {
        return remote_workspace::command("git_unstage_all", json!({"cwd":cwd}));
    }
    git_checked(&expand_home(cwd), &["restore", "--staged", "--", "."])
}

/// Discard every unstaged change (restore tracked files; delete untracked).
pub fn discard_all(cwd: &Path) -> Result<(), String> {
    if remote_workspace::is_remote(cwd) {
        return remote_workspace::command("git_discard_all", json!({"cwd":cwd}));
    }
    let root = expand_home(cwd);
    let files: Vec<String> = diff_index_with(&root, true)
        .files
        .into_iter()
        .filter(|file| file.unstaged)
        .map(|file| file.relative)
        .collect();
    for relative in files {
        discard_file_for(&root, &relative)?;
    }
    Ok(())
}

/// Create a commit from the current index, or rewrite HEAD with it when `amend` is set.
pub fn commit(cwd: &Path, message: &str, amend: bool) -> Result<(), String> {
    if remote_workspace::is_remote(cwd) {
        return remote_workspace::command(
            "git_commit",
            json!({"cwd":cwd,"message":message,"amend":amend}),
        );
    }
    let extra: &[&str] = if amend { &["--amend"] } else { &[] };
    commit_args(&expand_home(cwd), message, extra)
}

/// Full message (subject and body) of the commit at HEAD.
pub fn head_message(cwd: &Path) -> Result<String, String> {
    if remote_workspace::is_remote(cwd) {
        return remote_workspace::command("git_head_message", json!({"cwd":cwd}));
    }
    git_stdout(&expand_home(cwd), &["log", "-1", "--pretty=%B"])
        .ok_or_else(|| "No commits yet".to_string())
}

/// Push the current branch to its upstream, or set upstream on first push.
pub fn push(cwd: &Path) -> Result<(), String> {
    if remote_workspace::is_remote(cwd) {
        return remote_workspace::command("git_push", json!({"cwd":cwd}));
    }
    push_for(&expand_home(cwd))
}

/// Fast-forward the current branch from its upstream.
pub fn pull(cwd: &Path) -> Result<(), String> {
    if remote_workspace::is_remote(cwd) {
        return remote_workspace::command("git_pull", json!({"cwd":cwd}));
    }
    git_checked(&expand_home(cwd), &["pull", "--ff-only"])
}

/// Pull incoming commits, then push local commits.
pub fn sync(cwd: &Path) -> Result<(), String> {
    if remote_workspace::is_remote(cwd) {
        return remote_workspace::command("git_sync", json!({"cwd":cwd}));
    }
    let root = expand_home(cwd);
    if git_stdout(&root, &["rev-parse", "--abbrev-ref", "@{upstream}"]).is_some() {
        git_checked(&root, &["pull", "--no-edit", "--ff"]).map_err(with_signing_hint)?;
        return git_checked(&root, &["push"]);
    }
    push_for(&root)
}

/// Staged diff (or unstaged vs HEAD if nothing is staged) for commit text generation.
pub fn staged_context(cwd: &Path) -> Result<StagedContext, String> {
    if remote_workspace::is_remote(cwd) {
        return remote_workspace::command("git_staged_context", json!({"cwd":cwd}));
    }
    let root = expand_home(cwd);
    let root = root.as_path();
    let mut summary = git_run(root, &["diff", "--cached", "--stat", "--", "."]).unwrap_or_default();
    let mut patch =
        git_run(root, &["diff", "--cached", "--no-ext-diff", "--", "."]).unwrap_or_default();

    if summary.trim().is_empty() && patch.trim().is_empty() {
        summary = git_run(root, &["diff", "HEAD", "--stat", "--", "."]).unwrap_or_default();
        patch = git_run(root, &["diff", "HEAD", "--no-ext-diff", "--", "."]).unwrap_or_default();
        if let Some(untracked) = git_run(root, &["ls-files", "--others", "--exclude-standard"]) {
            let names = untracked.trim();
            if !names.is_empty() {
                if !summary.trim().is_empty() {
                    summary.push('\n');
                }
                summary.push_str("Untracked files:\n");
                summary.push_str(names);
            }
        }
    }

    if summary.trim().is_empty() && patch.trim().is_empty() {
        return Err("No changes to summarize".into());
    }

    Ok(StagedContext {
        branch: git_branch(root),
        summary,
        patch,
    })
}

/// Commits and diff between the default branch and HEAD, for PR text generation.
pub fn range_context(cwd: &Path) -> Result<RangeContext, String> {
    if remote_workspace::is_remote(cwd) {
        return remote_workspace::command("git_range_context", json!({"cwd":cwd}));
    }
    let root = expand_home(cwd);
    let root = root.as_path();
    let head = git_branch(root).ok_or_else(|| "Not on a branch".to_string())?;
    let remote = git_remote_name(root);
    let default_branch = git_default_branch(root, remote.as_deref())
        .ok_or_else(|| "Could not resolve the default branch".to_string())?;
    let base_ref = match &remote {
        Some(remote)
            if git_ref_exists(root, &format!("refs/remotes/{remote}/{default_branch}")) =>
        {
            format!("{remote}/{default_branch}")
        }
        _ => default_branch.clone(),
    };
    let spec = format!("{base_ref}...HEAD");
    let commit_summary =
        git_run(root, &["log", "--format=%s", &format!("{base_ref}..HEAD")]).unwrap_or_default();
    let diff_summary = git_run(root, &["diff", "--stat", &spec]).unwrap_or_default();
    let diff_patch = git_run(root, &["diff", "--no-ext-diff", &spec]).unwrap_or_default();
    if commit_summary.trim().is_empty() && diff_patch.trim().is_empty() {
        return Err("No commits to include in a pull request".into());
    }
    Ok(RangeContext {
        base: default_branch,
        head,
        commit_summary,
        diff_summary,
        diff_patch,
    })
}

/// Latest pull request for the current branch, if `gh` can see one.
pub fn pr_status(cwd: &Path) -> Result<Option<Pr>, String> {
    if remote_workspace::is_remote(cwd) {
        return remote_workspace::command("git_pr_status", json!({"cwd":cwd}));
    }
    Ok(pr_status_for(&expand_home(cwd)))
}

/// The working copy's GitHub `owner/name`, via `gh` (Tauri's git_github_repo).
pub fn github_repo(cwd: &Path) -> Result<String, String> {
    if remote_workspace::is_remote(cwd) {
        return Err("GitHub repository lookup is local only".into());
    }
    github_repo_for(&expand_home(cwd))
}

/// Create a GitHub pull request with `gh` and return its URL.
pub fn pr_create(
    cwd: &Path,
    title: &str,
    body: &str,
    base: &str,
    head: &str,
) -> Result<String, String> {
    if remote_workspace::is_remote(cwd) {
        return remote_workspace::command(
            "git_pr_create",
            json!({"cwd":cwd,"title":title,"body":body,"base":base,"head":head}),
        );
    }
    let root = expand_home(cwd);
    let title = title.trim();
    if title.is_empty() {
        return Err("Pull request title cannot be empty".into());
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let body_path = std::env::temp_dir().join(format!("monocode-pr-{stamp}.md"));
    std::fs::write(&body_path, body.trim()).map_err(|e| e.to_string())?;
    let result = gh_run(
        &root,
        &[
            "pr",
            "create",
            "--title",
            title,
            "--body-file",
            &body_path.to_string_lossy(),
            "--base",
            base.trim(),
            "--head",
            head.trim(),
        ],
    );
    let _ = std::fs::remove_file(&body_path);
    result.and_then(|output| {
        output
            .lines()
            .rev()
            .find(|line| line.starts_with("http://") || line.starts_with("https://"))
            .map(|line| line.trim().to_string())
            .ok_or_else(|| {
                if output.trim().is_empty() {
                    "gh returned no pull request URL".into()
                } else {
                    output
                }
            })
    })
}

fn discard_file_for(root: &Path, relative: &str) -> Result<(), String> {
    let relative = resolve_repo_path(root, relative)?;
    let abs = root.join(&relative);
    if git_checked(root, &["ls-files", "--error-unmatch", "--", &relative]).is_err() {
        if abs.is_file() {
            std::fs::remove_file(&abs).map_err(|e| e.to_string())?;
        } else if abs.exists() {
            git_checked(root, &["clean", "-fd", "--", &relative])?;
        }
        return Ok(());
    }
    git_checked(root, &["restore", "--worktree", "--", &relative])
}

fn commit_args(root: &Path, message: &str, extra: &[&str]) -> Result<(), String> {
    let message = message.trim();
    if message.is_empty() {
        return Err("Commit message cannot be empty".into());
    }
    let mut args = vec!["commit"];
    args.extend_from_slice(extra);
    args.extend(["--cleanup=strip", "-m", message]);
    git_checked(root, &args).map_err(with_signing_hint)
}

/// Explain why signing fails here when the same commit works in a terminal.
fn with_signing_hint(error: String) -> String {
    if !error.contains("failed to sign") && !error.contains("ssh-keygen") {
        return error;
    }
    format!(
        "{error}\n\nGit couldn't sign this commit. MCode runs git without a terminal, \
         so your signer needs a GUI passphrase prompt (e.g. pinentry-mac) or an unlocked agent."
    )
}

fn push_for(root: &Path) -> Result<(), String> {
    if git_stdout(root, &["rev-parse", "--abbrev-ref", "@{upstream}"]).is_some() {
        return git_checked(root, &["push"]);
    }
    let remote = git_remote_name(root).ok_or_else(|| "No git remote to push to".to_string())?;
    git_checked(root, &["push", "-u", &remote, "HEAD"])
}

#[derive(Default)]
struct Sync {
    remote: Option<String>,
    upstream: Option<String>,
    default_branch: Option<String>,
    ahead: i64,
    behind: i64,
    ahead_of_default: i64,
    head_pushed: bool,
}

#[cfg(test)]
mod tests;
