use super::*;

pub(super) fn sync_for(root: &Path) -> Sync {
    // Side by side, as in `diff_index_with`: only the default branch waits on
    // the remote. Without an upstream its count fails and goes unused.
    let ((upstream, upstream_counts, head_pushed), (remote, default_branch, default_counts)) =
        std::thread::scope(|s| {
            let upstream =
                s.spawn(|| git_stdout(root, &["rev-parse", "--abbrev-ref", "@{upstream}"]));
            let upstream_counts = s.spawn(|| git_ahead_behind(root, "@{upstream}"));
            let head_pushed = s.spawn(|| {
                git_stdout(
                    root,
                    &[
                        "for-each-ref",
                        "--count=1",
                        "--contains",
                        "HEAD",
                        "refs/remotes",
                    ],
                )
                .is_some()
            });
            let remote = git_remote_name(root);
            let default_branch = git_default_branch(root, remote.as_deref());
            let default_counts = match (&remote, &default_branch) {
                (Some(remote), Some(branch)) => {
                    Some(git_ahead_behind(root, &format!("{remote}/{branch}")))
                }
                _ => None,
            };
            (
                (
                    upstream.join().unwrap(),
                    upstream_counts.join().unwrap(),
                    head_pushed.join().unwrap(),
                ),
                (remote, default_branch, default_counts),
            )
        });
    let (ahead, behind) = if upstream.is_some() {
        upstream_counts
    } else {
        default_counts.unwrap_or((0, 0))
    };
    let ahead_of_default = default_counts.map_or(ahead, |(ahead, _)| ahead);
    Sync {
        remote,
        upstream,
        default_branch,
        ahead,
        behind,
        ahead_of_default,
        head_pushed,
    }
}

pub(super) fn git_remote_name(root: &Path) -> Option<String> {
    let remotes = git_stdout(root, &["remote"])?;
    let mut names = remotes
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty());
    let first = names.next()?.to_string();
    if first == "origin" || names.any(|name| name == "origin") {
        return Some("origin".into());
    }
    Some(first)
}

pub(super) fn git_default_branch(root: &Path, remote: Option<&str>) -> Option<String> {
    if let Some(remote) = remote {
        if let Some(head) = git_stdout(
            root,
            &[
                "symbolic-ref",
                "--short",
                &format!("refs/remotes/{remote}/HEAD"),
            ],
        ) {
            if let Some((_, name)) = head.split_once('/') {
                return Some(name.to_string());
            }
            return Some(head);
        }
        for name in ["main", "master"] {
            if git_ref_exists(root, &format!("refs/remotes/{remote}/{name}")) {
                return Some(name.to_string());
            }
        }
    }
    for name in ["main", "master"] {
        if git_ref_exists(root, &format!("refs/heads/{name}")) {
            return Some(name.to_string());
        }
    }
    None
}

pub(super) fn git_ref_exists(root: &Path, spec: &str) -> bool {
    git_output(root, &["show-ref", "--verify", "--quiet", spec]).is_some()
}

pub(super) fn git_ahead_behind(root: &Path, base: &str) -> (i64, i64) {
    let spec = format!("{base}...HEAD");
    let Some(text) = git_stdout(root, &["rev-list", "--left-right", "--count", &spec]) else {
        return (0, 0);
    };
    let mut parts = text.split_whitespace();
    let behind = parts
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let ahead = parts
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    (ahead, behind)
}
