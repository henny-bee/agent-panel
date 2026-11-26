use super::*;

/// ` 2 files changed, 3 insertions(+), 1 deletion(-)`
pub(super) fn parse_shortstat(stat: &str) -> CommitDetails {
    let mut details = CommitDetails::default();
    for part in stat.trim().split(',') {
        let mut words = part.split_whitespace();
        let n = words.next().and_then(|n| n.parse().ok()).unwrap_or(0);
        match words.next() {
            Some(w) if w.starts_with("file") => details.files = n,
            Some(w) if w.starts_with("insertion") => details.additions = n,
            Some(w) if w.starts_with("deletion") => details.deletions = n,
            _ => {}
        }
    }
    details
}

pub(super) fn diff_stats_for(root: &Path) -> DiffStats {
    if !git_is_work_tree(root) {
        return DiffStats::default();
    }
    let mut files: HashMap<String, FileAcc> = HashMap::new();
    if let Some(text) = git_run(
        root,
        &[
            "diff",
            "-z",
            "--no-ext-diff",
            "--numstat",
            "HEAD",
            "--",
            ".",
        ],
    ) {
        add_numstat_map(&text, &mut files);
    } else {
        if let Some(text) = git_run(
            root,
            &["diff", "-z", "--no-ext-diff", "--numstat", "--", "."],
        ) {
            add_numstat_map(&text, &mut files);
        }
        if let Some(text) = git_run(
            root,
            &[
                "diff",
                "-z",
                "--no-ext-diff",
                "--cached",
                "--numstat",
                "--",
                ".",
            ],
        ) {
            add_numstat_map(&text, &mut files);
        }
    }
    add_untracked_map(root, git_run(root, UNTRACKED), &mut files);
    let mut additions = 0i64;
    let mut deletions = 0i64;
    for acc in files.values() {
        additions += acc.additions;
        deletions += acc.deletions;
    }
    DiffStats {
        files: files.len() as i64,
        additions,
        deletions,
    }
}

#[derive(Clone, Default)]
pub(super) struct FileAcc {
    pub(super) additions: i64,
    pub(super) deletions: i64,
    pub(super) untracked: bool,
    pub(super) staged: bool,
    pub(super) unstaged: bool,
}

pub(super) fn diff_index_with(root: &Path, include_sync: bool) -> DiffIndex {
    // Each Git call is a process start, about 100ms on Windows. These reads
    // don't depend on each other, so they run side by side and merge in the
    // same order as before.
    let (numstat, names, untracked, cached, unstaged, sync, branch, head) =
        std::thread::scope(|s| {
            let git = |args: &'static [&'static str]| s.spawn(move || git_run(root, args));
            let numstat = git(&[
                "diff",
                "-z",
                "--relative",
                "--no-ext-diff",
                "--numstat",
                "HEAD",
                "--",
                ".",
            ]);
            let names = git(&[
                "diff",
                "-z",
                "--relative",
                "--no-ext-diff",
                "--name-status",
                "--no-renames",
                "HEAD",
                "--",
                ".",
            ]);
            let untracked = git(UNTRACKED);
            let cached = git(&[
                "diff",
                "-z",
                "--relative",
                "--cached",
                "--name-only",
                "--no-renames",
                "--",
                ".",
            ]);
            let unstaged = git(&[
                "diff",
                "-z",
                "--relative",
                "--name-only",
                "--no-renames",
                "--",
                ".",
            ]);
            let sync = s.spawn(move || {
                if include_sync {
                    sync_for(root)
                } else {
                    Sync::default()
                }
            });
            let branch = s.spawn(|| git_branch(root));
            let head = s.spawn(|| git_stdout(root, &["rev-parse", "HEAD"]));
            (
                numstat.join().unwrap(),
                names.join().unwrap(),
                untracked.join().unwrap(),
                cached.join().unwrap(),
                unstaged.join().unwrap(),
                sync.join().unwrap(),
                branch.join().unwrap(),
                head.join().unwrap(),
            )
        });
    let mut files: HashMap<String, FileAcc> = HashMap::new();
    let mut statuses: HashMap<String, &'static str> = HashMap::new();

    if let Some(text) = numstat {
        add_numstat_map(&text, &mut files);
        if let Some(names) = names {
            add_name_status(&names, &mut statuses);
        }
    } else {
        // Without a first commit there is no HEAD to diff against.
        if let Some(text) = git_run(
            root,
            &[
                "diff",
                "-z",
                "--relative",
                "--no-ext-diff",
                "--numstat",
                "--",
                ".",
            ],
        ) {
            add_numstat_map(&text, &mut files);
        }
        if let Some(text) = git_run(
            root,
            &[
                "diff",
                "-z",
                "--relative",
                "--no-ext-diff",
                "--cached",
                "--numstat",
                "--",
                ".",
            ],
        ) {
            add_numstat_map(&text, &mut files);
        }
        if let Some(names) = git_run(
            root,
            &[
                "diff",
                "-z",
                "--relative",
                "--no-ext-diff",
                "--name-status",
                "--no-renames",
                "--",
                ".",
            ],
        ) {
            add_name_status(&names, &mut statuses);
        }
        if let Some(names) = git_run(
            root,
            &[
                "diff",
                "-z",
                "--relative",
                "--no-ext-diff",
                "--cached",
                "--name-status",
                "--no-renames",
                "--",
                ".",
            ],
        ) {
            add_name_status(&names, &mut statuses);
        }
    }
    add_untracked_map(root, untracked, &mut files);
    mark_cached_and_unstaged(cached, unstaged, &mut files);

    let mut out = Vec::with_capacity(files.len());
    let mut additions = 0i64;
    let mut deletions = 0i64;
    for (relative, acc) in files {
        additions += acc.additions;
        deletions += acc.deletions;
        let abs = root.join(&relative);
        let status = if acc.untracked {
            "untracked"
        } else if let Some(status) = statuses.get(&relative) {
            *status
        } else if !abs.exists() {
            "deleted"
        } else {
            "modified"
        };
        out.push(ChangedFile {
            path: path_to_js(&abs),
            relative,
            status: status.to_string(),
            additions: acc.additions,
            deletions: acc.deletions,
            staged: acc.staged,
            unstaged: acc.untracked || acc.unstaged,
        });
    }
    out.sort_by(|a, b| a.relative.cmp(&b.relative));
    DiffIndex {
        branch,
        head,
        files: out,
        additions,
        deletions,
        remote: sync.remote,
        upstream: sync.upstream,
        default_branch: sync.default_branch,
        ahead: sync.ahead,
        behind: sync.behind,
        ahead_of_default: sync.ahead_of_default,
        head_pushed: sync.head_pushed,
    }
}

pub(super) fn add_numstat_map(text: &str, files: &mut HashMap<String, FileAcc>) {
    // `git diff -z --numstat` ends each record with NUL. A rename is
    // `add <TAB> del <TAB> <NUL> old <NUL> new <NUL>`; every other record is
    // `add <TAB> del <TAB> path <NUL>`.
    let mut fields = text.split('\0');
    while let Some(record) = fields.next() {
        if record.is_empty() {
            continue;
        }
        let mut parts = record.splitn(3, '\t');
        let Some(add) = parts.next() else { continue };
        let Some(del) = parts.next() else { continue };
        let Some(path) = parts.next() else { continue };
        let path = if path.is_empty() {
            let _previous = fields.next();
            match fields.next() {
                Some(destination) if !destination.is_empty() => destination,
                _ => continue,
            }
        } else {
            path
        };
        let relative = literal_diff_path(path);
        if relative.is_empty() {
            continue;
        }
        let entry = files.entry(relative).or_default();
        if add != "-" && del != "-" {
            entry.additions += add.parse::<i64>().unwrap_or(0);
            entry.deletions += del.parse::<i64>().unwrap_or(0);
        }
    }
}

pub(super) fn add_name_status(text: &str, statuses: &mut HashMap<String, &'static str>) {
    // `--no-renames -z` is `status <NUL> path <NUL>`, repeated.
    let mut fields = text.split('\0');
    while let Some(code) = fields.next() {
        if code.is_empty() {
            continue;
        }
        let Some(path) = fields.next() else {
            break;
        };
        let status = match code.as_bytes().first() {
            Some(b'A') => "added",
            Some(b'D') => "deleted",
            Some(b'M' | b'T') => "modified",
            _ => continue,
        };
        let relative = literal_diff_path(path);
        if !relative.is_empty() {
            statuses.insert(relative, status);
        }
    }
}

pub(super) fn literal_diff_path(path: &str) -> String {
    let path = path.trim();
    if path.is_empty() {
        String::new()
    } else {
        path_to_js(Path::new(path))
    }
}

pub(super) fn normalize_diff_path(path: &str) -> String {
    let path = path.trim();
    if path.is_empty() {
        return String::new();
    }
    let path = if let Some((_, new)) = path.split_once(" => ") {
        new.trim_end_matches('}')
    } else {
        path
    };
    path_to_js(Path::new(path))
}

/// Untracked files, without ignored ones.
pub(super) const UNTRACKED: &[&str] = &["ls-files", "-o", "--exclude-standard", "-z", "--", "."];

/// Adds the files of an `UNTRACKED` listing, with their line counts.
pub(super) fn add_untracked_map(
    root: &Path,
    listing: Option<String>,
    files: &mut HashMap<String, FileAcc>,
) {
    let Some(stdout) = listing else {
        return;
    };
    for rel in stdout.split('\0') {
        if rel.is_empty() {
            continue;
        }
        let relative = path_to_js(Path::new(rel));
        let entry = files.entry(relative.clone()).or_default();
        entry.untracked = true;
        if entry.additions == 0 {
            entry.additions = text_line_count(&root.join(rel));
        }
    }
}

pub(super) fn text_line_count(path: &Path) -> i64 {
    let Ok(meta) = std::fs::metadata(path) else {
        return 0;
    };
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_UNTRACKED_BYTES {
        return 0;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return 0;
    };
    if bytes.contains(&0) {
        return 0;
    }
    let mut lines = 1i64;
    for byte in &bytes {
        if *byte == b'\n' {
            lines += 1;
        }
    }
    if bytes.last() == Some(&b'\n') {
        lines -= 1;
    }
    lines
}

/// Marks the files of `diff --cached --name-only` as staged and those of
/// `diff --name-only` as unstaged.
pub(super) fn mark_cached_and_unstaged(
    cached: Option<String>,
    unstaged: Option<String>,
    files: &mut HashMap<String, FileAcc>,
) {
    if let Some(names) = cached {
        for line in names.split('\0') {
            let relative = literal_diff_path(line);
            if !relative.is_empty() {
                files.entry(relative).or_default().staged = true;
            }
        }
    }
    if let Some(names) = unstaged {
        for line in names.split('\0') {
            let relative = literal_diff_path(line);
            if !relative.is_empty() {
                files.entry(relative).or_default().unstaged = true;
            }
        }
    }
}
