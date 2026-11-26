use super::*;

/// Tips for the Graph filter: current HEAD, its upstream, and the repo
/// default branch. Omits `refs/stash` and unmerged local branches.
pub(super) fn history_tips(root: &Path) -> Vec<String> {
    // The upstream check runs beside the remote's default-branch lookup.
    std::thread::scope(|s| {
        let upstream =
            s.spawn(|| git_stdout(root, &["rev-parse", "--abbrev-ref", "@{upstream}"]).is_some());
        let default = git_remote_name(root).and_then(|remote| {
            let branch = git_default_branch(root, Some(remote.as_str()))?;
            let spec = format!("{remote}/{branch}");
            git_ref_exists(root, &format!("refs/remotes/{spec}")).then_some(spec)
        });
        let mut tips = vec!["HEAD".to_string()];
        if upstream.join().unwrap() {
            tips.push("@{upstream}".to_string());
        }
        tips.extend(default);
        tips
    })
}

pub(super) fn history_for(root: &Path, limit: Option<u32>) -> Result<History, String> {
    // Each git call is a process start, about 100ms on Windows: only the log
    // waits for the others. Outside a work tree they all fail harmlessly.
    let (work_tree, head, remotes, tips) = std::thread::scope(|s| {
        let work_tree = s.spawn(|| git_is_work_tree(root));
        let head = s.spawn(|| git_stdout(root, &["rev-parse", "HEAD"]));
        let remotes = s.spawn(|| git_remote_names(root));
        let tips = history_tips(root);
        (
            work_tree.join().unwrap(),
            head.join().unwrap(),
            remotes.join().unwrap(),
            tips,
        )
    });
    if !work_tree {
        return Ok(History::default());
    }
    let n = limit.unwrap_or(HISTORY_DEFAULT).clamp(1, HISTORY_MAX);
    let count = n.to_string();
    let mut args = vec![
        "log".to_string(),
        "--topo-order".to_string(),
        "--decorate=short".to_string(),
        "--max-count".to_string(),
        count,
        "--format=%H%x00%h%x00%P%x00%an%x00%at%x00%D%x00%s%x1e".to_string(),
    ];
    args.extend(tips);
    let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();
    let Some(text) = git_run(root, &args_ref) else {
        return Ok(History {
            head,
            commits: Vec::new(),
        });
    };
    Ok(History {
        commits: parse_history_log(&text, head.as_deref(), &remotes),
        head,
    })
}

pub(super) fn parse_history_log(
    text: &str,
    head: Option<&str>,
    remotes: &[String],
) -> Vec<HistoryCommit> {
    let mut commits = Vec::new();
    for record in text.split('\u{1e}') {
        let record = record.trim();
        if record.is_empty() {
            continue;
        }
        let mut fields = record.split('\0');
        let Some(sha) = fields.next() else { continue };
        let Some(short_sha) = fields.next() else {
            continue;
        };
        let Some(parents) = fields.next() else {
            continue;
        };
        let Some(author) = fields.next() else {
            continue;
        };
        let Some(timestamp) = fields.next() else {
            continue;
        };
        let Some(decorations) = fields.next() else {
            continue;
        };
        let subject = fields.next().unwrap_or("");
        if sha.is_empty() {
            continue;
        }
        let (is_head, refs) = parse_decorations(decorations, head, sha, remotes);
        commits.push(HistoryCommit {
            sha: sha.to_string(),
            short_sha: if short_sha.is_empty() {
                sha.chars().take(7).collect()
            } else {
                short_sha.to_string()
            },
            parents: parents.split_whitespace().map(str::to_string).collect(),
            author: author.to_string(),
            timestamp: timestamp.parse().unwrap_or(0),
            subject: subject.to_string(),
            refs,
            head: is_head,
        });
    }
    commits
}

pub(super) fn parse_decorations(
    raw: &str,
    head: Option<&str>,
    sha: &str,
    remotes: &[String],
) -> (bool, Vec<HistoryRef>) {
    let mut is_head = head == Some(sha);
    let mut refs = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some(name) = part.strip_prefix("HEAD -> ") {
            is_head = true;
            if !name.is_empty() {
                refs.push(HistoryRef {
                    name: name.to_string(),
                    kind: "local".into(),
                });
            }
        } else if part == "HEAD" {
            is_head = true;
        } else if let Some(tag) = part.strip_prefix("tag: ") {
            if !tag.is_empty() {
                refs.push(HistoryRef {
                    name: tag.to_string(),
                    kind: "tag".into(),
                });
            }
        } else if part.ends_with("/HEAD") {
            continue;
        } else if remotes
            .iter()
            .any(|remote| part == remote || part.starts_with(&format!("{remote}/")))
        {
            refs.push(HistoryRef {
                name: part.to_string(),
                kind: "remote".into(),
            });
        } else {
            refs.push(HistoryRef {
                name: part.to_string(),
                kind: "local".into(),
            });
        }
    }
    (is_head, refs)
}

pub(super) fn git_remote_names(root: &Path) -> Vec<String> {
    git_run(root, &["remote"])
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}
