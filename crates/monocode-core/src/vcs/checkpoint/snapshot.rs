use super::*;

pub(super) struct PrivateIndex(pub(super) PathBuf);

impl Drop for PrivateIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(self.0.with_extension("lock"));
    }
}

pub(super) fn capture_workspace(root: &Path, reference: &str) -> Result<String, String> {
    if run_git(root, &["config", "--bool", "core.sparseCheckout"])
        .ok()
        .is_some_and(|bytes| bytes.starts_with(b"true"))
    {
        return Err("Workspace checkpoints are unavailable for sparse checkouts".into());
    }
    let index =
        PrivateIndex(std::env::temp_dir().join(format!("monocode-checkpoint-{}", unique_id())));
    let run = |args: &[&str]| {
        checked_output(
            git_command(root, args)
                .env("GIT_INDEX_FILE", &index.0)
                .env("GIT_AUTHOR_NAME", "MonoCode")
                .env("GIT_AUTHOR_EMAIL", "checkpoint@monocode.local")
                .env("GIT_COMMITTER_NAME", "MonoCode")
                .env("GIT_COMMITTER_EMAIL", "checkpoint@monocode.local"),
        )
    };
    if run_git(root, &["rev-parse", "--verify", "HEAD"]).is_ok() {
        run(&["read-tree", "HEAD"])?;
    } else {
        run(&["read-tree", "--empty"])?;
    }
    let durable = [
        "-c",
        "core.fsync=objects,reference",
        "-c",
        "core.fsyncMethod=fsync",
    ];
    run(&[durable.as_slice(), &["add", "-A", "--", "."]].concat())?;
    let tree = String::from_utf8(run(&[durable.as_slice(), &["write-tree"]].concat())?)
        .map_err(|e| e.to_string())?;
    let commit = String::from_utf8(run(&[
        durable.as_slice(),
        &[
            "commit-tree",
            tree.trim(),
            "-m",
            "MonoCode workspace checkpoint",
        ],
    ]
    .concat())?)
    .map_err(|e| e.to_string())?;
    run(&[
        durable.as_slice(),
        &["update-ref", reference, commit.trim()],
    ]
    .concat())?;
    Ok(commit.trim().to_string())
}

/// Files carried into a new turn's baseline from the turn before it.
#[derive(Default)]
pub(super) struct Carried {
    pub(super) files: Vec<(String, Option<Blob>)>,
    pub(super) edited: BTreeSet<String>,
    pub(super) shared: bool,
}

/// `fresh` with each carried file put back as it was before the session's
/// pending work: its earlier blob, or absent when the session created it.
pub(super) fn overlay_baseline(
    root: &Path,
    fresh: &str,
    files: &[(String, Option<Blob>)],
    reference: &str,
) -> Result<String, String> {
    let index =
        PrivateIndex(std::env::temp_dir().join(format!("monocode-checkpoint-{}", unique_id())));
    let run = |args: &[&str]| {
        checked_output(
            git_command(root, args)
                .env("GIT_INDEX_FILE", &index.0)
                .env("GIT_AUTHOR_NAME", "MonoCode")
                .env("GIT_AUTHOR_EMAIL", "checkpoint@monocode.local")
                .env("GIT_COMMITTER_NAME", "MonoCode")
                .env("GIT_COMMITTER_EMAIL", "checkpoint@monocode.local"),
        )
    };
    run(&["read-tree", fresh])?;
    for (relative, blob) in files {
        match blob {
            Some(blob) => run(&[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("{},{},{relative}", blob.mode, blob.oid),
            ])?,
            None => run(&["update-index", "--force-remove", "--", relative])?,
        };
    }
    let durable = [
        "-c",
        "core.fsync=objects,reference",
        "-c",
        "core.fsyncMethod=fsync",
    ];
    let tree = String::from_utf8(run(&[durable.as_slice(), &["write-tree"]].concat())?)
        .map_err(|e| e.to_string())?;
    let commit = String::from_utf8(run(&[
        durable.as_slice(),
        &[
            "commit-tree",
            tree.trim(),
            "-m",
            "MonoCode workspace checkpoint",
        ],
    ]
    .concat())?)
    .map_err(|e| e.to_string())?;
    run(&[
        durable.as_slice(),
        &["update-ref", reference, commit.trim()],
    ]
    .concat())?;
    Ok(commit.trim().to_string())
}

pub(super) fn changes(root: &Path, turn: &RecordedTurn) -> Result<Vec<Change>, String> {
    let before = turn.before.as_deref().ok_or("Missing turn baseline")?;
    let after = turn.after.as_deref().ok_or("Missing turn result")?;
    let args = [
        "diff",
        "--relative",
        "--no-ext-diff",
        "--no-textconv",
        "--no-renames",
        "--no-abbrev",
        "-z",
    ];
    let raw = run_git(
        root,
        &[args.as_slice(), &["--raw", before, after, "--", "."]].concat(),
    )?;
    let numstat = run_git(
        root,
        &[args.as_slice(), &["--numstat", before, after, "--", "."]].concat(),
    )?;
    let mut stats = BTreeMap::new();
    for row in numstat
        .split(|byte| *byte == 0)
        .filter(|row| !row.is_empty())
    {
        let mut fields = row.splitn(3, |byte| *byte == b'\t');
        let additions = fields.next().ok_or("Invalid checkpoint line counts")?;
        let deletions = fields.next().ok_or("Invalid checkpoint line counts")?;
        let path = fields.next().ok_or("Invalid checkpoint path")?;
        let count = |bytes: &[u8]| -> Result<i64, String> {
            if bytes == b"-" {
                return Ok(0);
            }
            String::from_utf8_lossy(bytes)
                .parse()
                .map_err(|_| "Invalid checkpoint line counts".into())
        };
        stats.insert(path.to_vec(), (count(additions)?, count(deletions)?));
    }
    let mut records = raw.split(|byte| *byte == 0).filter(|row| !row.is_empty());
    let mut out = Vec::new();
    while let Some(header) = records.next() {
        let path = records.next().ok_or("Invalid checkpoint path")?;
        let fields: Vec<_> = std::str::from_utf8(header)
            .map_err(|e| e.to_string())?
            .split_whitespace()
            .collect();
        if fields.len() != 5 || !fields[0].starts_with(':') {
            return Err("Invalid checkpoint file status".into());
        }
        let blob = |mode: &str, oid: &str| {
            (mode != "000000").then(|| Blob {
                mode: mode.into(),
                oid: oid.into(),
            })
        };
        let (additions, deletions) = stats.remove(path).ok_or("Missing checkpoint line counts")?;
        out.push(Change {
            relative: String::from_utf8(path.to_vec()).map_err(|e| e.to_string())?,
            before: blob(&fields[0][1..], fields[2]),
            after: blob(fields[1], fields[3]),
            status: match fields[4] {
                "A" => "added",
                "D" => "deleted",
                _ => "modified",
            }
            .into(),
            additions,
            deletions,
        });
    }
    out.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(out)
}

pub(super) fn resolved_turn_paths(
    root: &Path,
    turn: &RecordedTurn,
    recorded: &[Change],
    head: Option<&str>,
) -> Result<Vec<String>, String> {
    let Some(head) = head.filter(|head| Some(*head) != turn.head.as_deref()) else {
        return Ok(Vec::new());
    };
    if recorded
        .iter()
        .all(|change| turn.dismissed.contains(&change.relative))
    {
        return Ok(Vec::new());
    }
    let diff = [
        "diff",
        "--relative",
        "--no-ext-diff",
        "--no-textconv",
        "--no-renames",
        "--name-only",
        "-z",
    ];
    let after = turn.after.as_deref().ok_or("Missing turn result")?;
    let differs_from_result =
        git_paths(root, &[diff.as_slice(), &[after, head, "--", "."]].concat())?;
    let mut pending = git_paths(root, &[diff.as_slice(), &[head, "--", "."]].concat())?;
    pending.extend(git_paths(
        root,
        &[
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            ".",
        ],
    )?);
    // A file is resolved when its recorded result is in HEAD, even if someone
    // edited it again, or when it is clean after a commit that included more edits.
    Ok(recorded
        .iter()
        .filter(|change| {
            !turn.dismissed.contains(&change.relative)
                && (!differs_from_result.contains(&change.relative)
                    || !pending.contains(&change.relative))
        })
        .map(|change| change.relative.clone())
        .collect())
}

pub(super) fn git_paths(root: &Path, args: &[&str]) -> Result<HashSet<String>, String> {
    run_git(root, args)?
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8(path.to_vec()).map_err(|e| e.to_string()))
        .collect()
}
