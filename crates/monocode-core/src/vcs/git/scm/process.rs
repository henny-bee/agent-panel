use super::*;

pub(super) fn resolve_repo_path(root: &Path, relative: &str) -> Result<String, String> {
    let relative = normalize_diff_path(relative);
    if relative.is_empty()
        || relative.starts_with('/')
        || relative
            .split('/')
            .any(|part| part.is_empty() || part == "..")
    {
        return Err("Invalid path".into());
    }
    let abs = root.join(&relative);
    if !abs.starts_with(root) {
        return Err("Invalid path".into());
    }
    Ok(relative)
}

/// Any git can start a filter (git-lfs), which a Finder launch's bare PATH
/// lacks. The cached search path finds it without waiting on the login shell.
pub(crate) fn git_cmd() -> Command {
    let mut cmd = Command::new("git");
    crate::sys::process::configure_child(&mut cmd);
    cmd.env("PATH", launch_env::cached_gui_search_path())
        .env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

pub(super) fn git_cmd_for_args_with_path(
    args: &[&str],
    gui_path: impl FnOnce() -> String,
) -> Command {
    let mut cmd = git_cmd();
    let args = args.strip_prefix(&["--literal-pathspecs"]).unwrap_or(args);
    if matches!(
        args.first().copied(),
        Some("commit" | "push" | "pull" | "fetch" | "clone")
    ) {
        // Signers, hooks, and credential helpers may only be on the login-shell PATH.
        cmd.env("PATH", gui_path());
    }
    cmd
}

pub(super) fn git_cmd_for_args(args: &[&str]) -> Command {
    git_cmd_for_args_with_path(args, launch_env::gui_search_path)
}

pub(super) fn git_checked(root: &Path, args: &[&str]) -> Result<(), String> {
    let mut cmd = git_cmd_for_args(args);
    let output = cmd
        .arg("--no-pager")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let msg = stderr.trim();
    if !msg.is_empty() {
        return Err(msg.to_string());
    }
    let msg = stdout.trim();
    if !msg.is_empty() {
        return Err(msg.to_string());
    }
    Err(format!("git {} failed", args.join(" ")))
}

pub(super) fn git_run(root: &Path, args: &[&str]) -> Option<String> {
    git_output(root, args).map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

pub(super) fn git_output(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = git_cmd_for_args(args)
        .arg("--no-pager")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()?;
    if git_status_ok(&output.status, args) {
        return Some(output.stdout);
    }
    None
}

pub(super) fn git_status_ok(status: &std::process::ExitStatus, args: &[&str]) -> bool {
    status.success()
        || (status.code() == Some(1) && matches!(args.first().copied(), Some("diff" | "grep")))
}

pub(super) fn git_branch(root: &Path) -> Option<String> {
    git_head_branch(root).or_else(|| git_stdout(root, &["rev-parse", "--short", "HEAD"]))
}

pub(super) fn git_head_branch(root: &Path) -> Option<String> {
    git_stdout(root, &["symbolic-ref", "--short", "HEAD"]).filter(|branch| branch != "HEAD")
}

pub(super) fn git_is_work_tree(root: &Path) -> bool {
    git_stdout(root, &["rev-parse", "--is-inside-work-tree"]).as_deref() == Some("true")
}

/// The checkout's Git directory and, for a linked worktree, the shared one:
/// where HEAD, the index and refs live, for change watchers.
pub fn git_dirs(root: &Path) -> Vec<std::path::PathBuf> {
    let Some(text) = git_stdout(
        root,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-dir",
            "--git-common-dir",
        ],
    ) else {
        return Vec::new();
    };
    let mut dirs: Vec<std::path::PathBuf> = text.lines().map(Into::into).collect();
    dirs.dedup();
    dirs
}

pub(super) fn git_stdout(root: &Path, args: &[&str]) -> Option<String> {
    let output = git_cmd().arg("-C").arg(root).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}
