mod index_and_log;
mod remote;
mod staging;
use super::*;
use std::path::PathBuf;

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn git(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .args([
            "-c",
            "user.name=MonoCode",
            "-c",
            "user.email=monocode@test",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "MonoCode")
        .env("GIT_AUTHOR_EMAIL", "monocode@test")
        .env("GIT_COMMITTER_NAME", "MonoCode")
        .env("GIT_COMMITTER_EMAIL", "monocode@test")
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn init_git(dir: &Path, branch: &str) -> bool {
    git(dir, &["init"])
        && git(
            dir,
            &["symbolic-ref", "HEAD", &format!("refs/heads/{branch}")],
        )
        && git(dir, &["config", "user.name", "MonoCode"])
        && git(dir, &["config", "user.email", "monocode@test"])
        && git(dir, &["config", "commit.gpgsign", "false"])
        && git(dir, &["config", "core.autocrlf", "false"])
}

fn init_git_commit(dir: &Path, files: &[(&str, &str)]) -> bool {
    if !init_git(dir, "main") {
        return false;
    }
    for (name, contents) in files {
        if std::fs::write(dir.join(name), contents).is_err() {
            return false;
        }
    }
    git(dir, &["add", "."]) && git(dir, &["commit", "-m", "init"])
}

fn init_bare(dir: &Path) -> bool {
    git(dir, &["init", "--bare"])
}

fn index(dir: &Path) -> DiffIndex {
    diff_index(dir).unwrap()
}

fn file(dir: &Path, relative: &str) -> ChangedFile {
    index(dir)
        .files
        .into_iter()
        .find(|file| file.relative == relative)
        .unwrap()
}

fn with_origin(repo: &Path, origin: &Path) {
    assert!(init_bare(origin));
    let url = origin.to_string_lossy().into_owned();
    assert!(git(repo, &["remote", "add", "origin", &url]));
    assert!(git(repo, &["push", "-u", "origin", "main"]));
}

fn git_cmd_path(cmd: &Command) -> Vec<PathBuf> {
    let path = cmd
        .get_envs()
        .find(|(key, _)| *key == std::ffi::OsStr::new("PATH"))
        .and_then(|(_, value)| value)
        .expect("git should get an explicit PATH");
    std::env::split_paths(path).collect()
}
