//! A paired phone's project pages: worktrees, files, search, file reading with
//! syntax colours, changes and the commit sheet's Git actions. Ports of the
//! project handlers in mobileWorkspace.ts, mobileHighlight.ts and the Tauri
//! `mobile_list_dir` / `mobile_read_text_file` commands, replying with the same
//! JSON the phone (mobile/src/features/workspace/types.ts) reads.
//!
//! Everything here blocks: callers run it on a background executor. Deduping
//! and timing out Git actions is the caller's job, as is telling the Changes
//! panel that Git changed after an action.
#![allow(dead_code)]

use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, atomic::AtomicBool},
};

use monocode_core::{
    files::{self, ProjectFile},
    provider::Kind,
    session::composer,
    vcs::{
        diff::{self, Line, LineKind},
        git::{self, scm},
    },
};
use serde_json::{Value, json};

use crate::ui::syntax_palette::LineHighlighter;

// ---------------------------------------------------------------------------
// Roots and worktrees

/// The project's checkout, or one of its other worktrees when the phone names
/// one, checked against the project's list rather than trusted (mobileWorktree).
pub(crate) fn resolve_root(cwd: &Path, worktree: Option<&str>) -> Result<PathBuf, String> {
    let Some(wanted) = worktree else {
        return Ok(cwd.to_path_buf());
    };
    let wanted = files::path_key(wanted);
    git::worktrees(cwd)
        .ok()
        .and_then(|trees| {
            trees
                .into_iter()
                .find(|tree| !tree.main && !tree.missing && files::path_key(&tree.path) == wanted)
        })
        .map(|tree| PathBuf::from(tree.path))
        .ok_or_else(|| "That worktree is no longer available".to_string())
}

/// The other working copies a phone can start a session in, as the desktop
/// composer's workspace picker lists them.
pub(crate) fn worktrees(cwd: &Path) -> Result<Value, String> {
    let worktrees: Vec<Value> = git::worktrees(cwd)?
        .into_iter()
        .filter(|tree| !tree.main && !tree.missing)
        .map(|tree| {
            let name = tree.branch.unwrap_or_else(|| project_name(&tree.path));
            json!({ "path": tree.path, "name": name })
        })
        .collect();
    Ok(json!({ "worktrees": worktrees }))
}

fn project_name(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_string()
}

// ---------------------------------------------------------------------------
// Files

/// A path a phone named under a working copy, refused when it resolves outside
/// it, through `..`, an absolute path or a symlink alike.
pub(crate) fn inside_root(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let root = root
        .canonicalize()
        .map_err(|_| "Project folder is unavailable".to_string())?;
    let path = root
        .join(relative)
        .canonicalize()
        .map_err(|_| "No such file or folder".to_string())?;
    if !path.starts_with(&root) {
        return Err("That is outside the project".into());
    }
    Ok(path)
}

/// One folder of a working copy, by its path relative to the root, without
/// the repository's own `.git`.
pub(crate) fn files(root: &Path, relative: &str) -> Result<Value, String> {
    let dir = inside_root(root, relative)?;
    let entries: Vec<Value> = files::list_dir(&dir)?
        .into_iter()
        .filter(|entry| entry.name != ".git")
        .map(|entry| json!({ "name": entry.name, "isDir": entry.is_dir, "ignored": entry.ignored }))
        .collect();
    Ok(json!({ "entries": entries }))
}

/// Files whose paths match a phone's search, best first, from the same index
/// and ranking as the desktop's quick open. The index lists files only.
pub(crate) fn search(root: &Path, query: &str) -> Result<Value, String> {
    if query.trim().is_empty() {
        return Ok(json!({ "results": [] }));
    }
    let files = files::list_project_files(&root.to_string_lossy())?;
    let results: Vec<Value> = composer::rank_files(&files, query, 50)
        .into_iter()
        .map(|file| json!({ "path": file.relative, "name": file.name }))
        .collect();
    Ok(json!({ "results": results }))
}

/// Enough of a file to read on a phone: tens of thousands of lines, well
/// inside a reply's 8MB. Its colours, being larger, come separately.
const MOBILE_FILE_BYTES: usize = 1024 * 1024;

/// One text file of a working copy, by its path relative to the root or as a
/// transcript names it (`Composer.tsx`, `app.ts:12`), with the path it was
/// found at and its syntax colours. A large file comes plain with a
/// background highlight's ID for `file_highlight`.
pub(crate) fn file(root: &Path, relative: &str) -> Result<Value, String> {
    let read = |relative: &str| inside_root(root, relative).and_then(|path| read_text(&path));
    let (path, full) = match read(relative) {
        Ok(text) => (relative.to_string(), text),
        Err(error) => {
            let resolved = resolve_openable(root, relative)
                .filter(|resolved| resolved != relative)
                .ok_or(error)?;
            let text = read(&resolved)?;
            (resolved, text)
        }
    };
    let mut cut = full.len().min(MOBILE_FILE_BYTES);
    while !full.is_char_boundary(cut) {
        cut -= 1;
    }
    let text = &full[..cut];
    let mut reply = json!({ "path": path, "text": text, "truncated": cut < full.len() });
    if let Some(highlighted) = highlight_file(&path, text) {
        reply["highlighted"] = highlighted.to_json();
    } else if text.len() > MAX_HIGHLIGHT_BYTES
        && let Some(job) = highlight_in_background(&path, text.to_string())
    {
        reply["highlightJob"] = json!(job);
    }
    Ok(reply)
}

/// read_text_file_sync: a reasonably sized UTF-8 file, refusing binaries.
fn read_text(path: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err("Not a file".into());
    }
    if meta.len() > scm::MAX_TEXT_FILE_BYTES {
        return Err(format!(
            "File is too large to edit (maximum {} MB).",
            scm::MAX_TEXT_FILE_BYTES / 1024 / 1024
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.contains(&0) {
        return Err("Binary files cannot be edited.".into());
    }
    String::from_utf8(bytes).map_err(|_| "File is not valid UTF-8.".into())
}

/// resolveOpenablePath, minimally: a reference as a transcript writes it,
/// without its `:line[:col]` or `#L` suffix, matched exactly, then by a unique
/// suffix, then by file name against the project index. Relative to `root`.
fn resolve_openable(root: &Path, href: &str) -> Option<String> {
    let hint = openable_hint(root, href);
    if hint.is_empty() {
        return None;
    }
    let files = files::list_project_files(&root.to_string_lossy()).unwrap_or_default();
    Some(pick_openable(&files, &hint).unwrap_or(hint))
}

fn openable_hint(root: &Path, href: &str) -> String {
    let mut value = href.trim().replace('\\', "/");
    if let Some(rest) = value.strip_prefix("file://") {
        value = rest.to_string();
    }
    value = strip_location(&value).to_string();
    let base = root.to_string_lossy().replace('\\', "/");
    let base = base.trim_end_matches('/');
    if !base.is_empty()
        && let Some(rest) = value.strip_prefix(&format!("{base}/"))
    {
        return rest.to_string();
    }
    value
        .trim_start_matches("./")
        .trim_start_matches('/')
        .to_string()
}

/// `path` without a trailing `:12`, `:12:3`, `#L12` or `#L12-L20`.
fn strip_location(path: &str) -> &str {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if let Some((head, tail)) = path.rsplit_once("#L") {
        let tail = tail
            .split_once("-L")
            .map_or(tail, |(a, b)| if digits(b) { a } else { "x" });
        if digits(tail) {
            return head;
        }
    }
    let mut out = path;
    for _ in 0..2 {
        match out.rsplit_once(':') {
            Some((head, tail)) if digits(tail) && !head.is_empty() => out = head,
            _ => break,
        }
    }
    out
}

fn pick_openable(files: &[ProjectFile], hint: &str) -> Option<String> {
    if let Some(file) = files.iter().find(|file| file.relative == hint) {
        return Some(file.relative.clone());
    }
    let suffix = format!("/{hint}");
    let ends = |file: &&ProjectFile| file.relative.ends_with(&suffix);
    let mut suffixed: Vec<_> = files
        .iter()
        .filter(|file| ends(file) || hint.ends_with(&file.relative))
        .collect();
    if suffixed.len() == 1 {
        return Some(suffixed[0].relative.clone());
    }
    let name = hint.rsplit('/').find(|part| !part.is_empty())?;
    let mut named: Vec<_> = files.iter().filter(|file| file.name == name).collect();
    if named.len() == 1 {
        return Some(named[0].relative.clone());
    }
    suffixed.retain(ends);
    let candidates = if suffixed.is_empty() {
        &mut named
    } else {
        &mut suffixed
    };
    candidates.sort_by_key(|file| file.relative.len());
    candidates.first().map(|file| file.relative.clone())
}

// ---------------------------------------------------------------------------
// Changes and Git

/// The uncommitted changes in a working copy, each with its unified diff and
/// syntax colours while a budget lasts. Echoes the worktree, so the phone can
/// tell these are the changes it asked for.
pub(crate) fn changes(root: &Path, worktree_echo: Option<&str>) -> Result<Value, String> {
    let index = scm::diff_files(root)?;
    let mut budget: usize = 1_000_000;
    let mut files = Vec::new();
    for file in &index.files {
        let mut lines: Vec<Value> = Vec::new();
        let mut palette = None;
        if budget > 0 && files.len() < 30 {
            let diff = scm::file_diff(root, &file.relative, !file.unstaged).ok();
            if let Some(diff) = diff.filter(|diff| {
                !diff.binary
                    && !diff.too_large
                    && diff.original.len() + diff.current.len() <= budget
            }) {
                budget -= diff.original.len() + diff.current.len();
                let unified =
                    diff::build_unified_file(&diff.original, &diff.current, diff::CONTEXT_DEFAULT);
                // Its syntax colours too; the diff is still worth sending without them.
                let highlighted = highlight_diff(
                    &file.relative,
                    &diff.original,
                    &diff.current,
                    &unified.lines,
                );
                let runs = match highlighted {
                    Some((colors, runs)) => {
                        palette = Some(colors);
                        runs
                    }
                    None => vec![None; unified.lines.len()],
                };
                lines = unified
                    .lines
                    .iter()
                    .zip(runs)
                    .map(|(line, runs)| diff_line_json(line, runs))
                    .collect();
            }
        }
        let status = match file.status.as_str() {
            "added" | "deleted" | "untracked" => file.status.as_str(),
            _ => "modified",
        };
        let mut entry = json!({
            "relative": file.relative,
            "status": status,
            "additions": file.additions,
            "deletions": file.deletions,
            "staged": file.staged,
            "unstaged": file.unstaged,
            "lines": lines,
        });
        if let Some(palette) = palette {
            entry["palette"] = json!(palette);
        }
        files.push(entry);
    }
    let mut reply = json!({ "branch": index.branch.unwrap_or_default(), "files": files });
    if let Some(worktree) = worktree_echo {
        reply["worktreePath"] = json!(worktree);
    }
    Ok(reply)
}

fn diff_line_json(line: &Line, runs: Option<Runs>) -> Value {
    let kind = match line.kind {
        LineKind::Add => "add",
        LineKind::Del => "del",
        LineKind::Context => "context",
        LineKind::Hunk => "hunk",
    };
    let mut value = json!({
        "kind": kind,
        "text": line.text,
        "oldNumber": line.old_number,
        "newNumber": line.new_number,
    });
    if let Some(runs) = runs {
        value["runs"] = runs_json(&runs);
    }
    value
}

/// What the phone's commit sheet needs to offer what the desktop's Changes
/// panel would: the remote, how the branch stands, and its open pull request.
pub(crate) fn git(root: &Path) -> Result<Value, String> {
    let (index, pr) = std::thread::scope(|scope| {
        let pr = scope.spawn(|| scm::pr_status(root).ok().flatten());
        let index = scm::diff_index(root);
        (index, pr.join().ok().flatten())
    });
    let index = index?;
    let pr = pr.filter(|pr| pr.state == "open").map_or(
        Value::Null,
        |pr| json!({ "number": pr.number, "url": pr.url }),
    );
    Ok(json!({
        "branch": index.branch.unwrap_or_default(),
        "files": index.files.len(),
        "remote": index.remote.is_some(),
        "upstream": index.upstream.is_some(),
        "defaultBranch": index.default_branch.unwrap_or_default(),
        "ahead": index.ahead,
        "behind": index.behind,
        "aheadOfDefault": index.ahead_of_default,
        // The native app has no remote projects, so it can always generate.
        "canGenerate": true,
        "pr": pr,
    }))
}

/// A Changes page action from the phone, as the desktop's Changes panel runs
/// it: staging, then committing what is staged, pushing and pull requests.
/// Replies `{}`, `{ message }` or `{ url }`.
/// `chosen` is the background model picked in Settings.
pub(crate) fn git_action(
    root: &Path,
    params: &Value,
    chosen: Option<(Kind, Option<String>)>,
) -> Result<Value, String> {
    // The harness picked in the phone's composer writes the commit message and
    // pull request, as the desktop's panel uses its session's.
    let preferred = params["harness"]
        .as_str()
        .and_then(Kind::parse)
        .unwrap_or(Kind::Claude);
    let harness = || crate::git::changes::pick_text_harness(preferred, chosen.clone());
    let paths = params["paths"]
        .as_array()
        .map(|paths| paths.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    // The React app recorded pushes and pull requests as inbox self-activity;
    // the native inbox has no such record yet, so that is skipped here.
    match params["action"].as_str().unwrap_or_default() {
        "stage" => {
            for path in paths {
                scm::stage_file(root, path)?;
            }
            Ok(json!({}))
        }
        "unstage" => {
            for path in paths {
                scm::unstage_file(root, path)?;
            }
            Ok(json!({}))
        }
        "stageAll" => scm::stage_all(root).map(|()| json!({})),
        "unstageAll" => scm::unstage_all(root).map(|()| json!({})),
        "generate" => {
            let harness =
                harness().ok_or("No provider CLI is installed to write a commit message.")?;
            let message = crate::git::changes::actions::generate_commit_message(
                harness,
                root.to_path_buf(),
                Arc::new(AtomicBool::new(false)),
            )?;
            Ok(json!({ "message": message }))
        }
        "commit" => {
            let message = params["message"].as_str().unwrap_or_default();
            scm::commit(root, message, false)?;
            let create_pr = params["createPr"] == true;
            if params["push"] != true && !create_pr {
                return Ok(json!({}));
            }
            scm::push(root)?;
            if create_pr {
                Ok(json!({ "url": create_pr_url(root, harness())? }))
            } else {
                Ok(json!({}))
            }
        }
        "sync" => scm::sync(root).map(|()| json!({})),
        "pr" => {
            if scm::diff_index(root)?.ahead > 0 {
                scm::push(root)?;
            }
            Ok(json!({ "url": create_pr_url(root, harness())? }))
        }
        _ => Err("Unsupported git action".into()),
    }
}

/// createMobilePr: the Changes panel's generated title and body (or the
/// commits, when no provider can write them), then `gh pr create`.
fn create_pr_url(
    root: &Path,
    harness: Option<crate::git::changes::TextHarness>,
) -> Result<String, String> {
    crate::git::changes::actions::create_pr_blocking(harness, root)
        .map(|url| url.trim().to_string())
}

// ---------------------------------------------------------------------------
// Highlighting (mobileHighlight.ts)

/// Past this, highlighting at once would hold up the reply for too long;
/// larger files are highlighted in the background instead.
const MAX_HIGHLIGHT_BYTES: usize = 120_000;
/// Longer lines (minified bundles, say) stay plain, as in the desktop editor.
const MAX_LINE_LENGTH: usize = 1_000;
/// A background highlight works through the file this many bytes at a time,
/// publishing each chunk as it finishes.
const CHUNK_BYTES: usize = 100_000;
/// Lines highlighted up front to tell whether a large file has colours at all.
const PROBE_LINES: usize = 400;
/// Background highlights kept for phones to collect, oldest dropped first.
const MAX_JOBS: usize = 4;
const CACHE_ENTRIES: usize = 60;
const CACHE_BYTES: usize = 2_000_000;
/// The theme's plain foreground, which is all an unknown language gets.
const PLAIN: u32 = 0xD4D4D4;

/// A line as runs of one colour: (text, index into the palette).
type Runs = Vec<(String, usize)>;

/// A file's lines as runs of one colour, the colours sent once in `palette`.
#[derive(Clone, Debug, Default)]
struct Highlighted {
    palette: Vec<String>,
    lines: Vec<Runs>,
}
impl Highlighted {
    fn to_json(&self) -> Value {
        json!({
            "palette": self.palette,
            "lines": self.lines.iter().map(runs_json).collect::<Vec<_>>(),
        })
    }
}

fn runs_json(runs: &Runs) -> Value {
    Value::Array(
        runs.iter()
            .map(|(text, color)| json!([text, color]))
            .collect(),
    )
}

#[derive(Default)]
struct Palette {
    colors: Vec<String>,
    index: HashMap<u32, usize>,
}
impl Palette {
    fn index(&mut self, color: u32) -> usize {
        *self.index.entry(color).or_insert_with(|| {
            self.colors.push(format!("#{color:06X}"));
            self.colors.len() - 1
        })
    }
}

/// The extension `syntax::highlight` should treat a file as, or None to leave
/// it plain (languageForFile).
fn language_for(path: &str) -> Option<String> {
    let name = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_lowercase();
    let by_name = match name.as_str() {
        "makefile" | "gnumakefile" => Some("makefile"),
        "gemfile" | "rakefile" => Some("rb"),
        "dockerfile" => Some("Dockerfile"),
        _ => None,
    };
    if let Some(language) = by_name {
        return Some(language.into());
    }
    let (_, extension) = name.rsplit_once('.')?;
    match extension {
        // `syntax::highlight` reads these as JavaScript; they are prose.
        "" | "txt" | "text" | "plaintext" | "log" => None,
        "zsh" => Some("sh".into()),
        "lock" => Some("json".into()),
        other => Some(other.into()),
    }
}

/// Lines (split on `\n`, as the phone and the diff split them) as runs, or
/// None when highlighting fails. The highlighter carries the grammar's state,
/// so a document fed in pieces colours as it would whole. Long lines are
/// skipped, so they cost nothing, and come back as one plain run.
fn highlight_lines(
    highlighter: &mut LineHighlighter,
    lines: &[&str],
    palette: &mut Palette,
) -> Option<Vec<Runs>> {
    let long = |line: &str| line.len() > MAX_LINE_LENGTH;
    let mut document = Vec::with_capacity(lines.len());
    for line in lines {
        document.push(highlighter.line(if long(line) { "" } else { line }).ok()?);
    }
    let mut out = Vec::with_capacity(lines.len());
    for (number, line) in lines.iter().enumerate() {
        let mut runs: Runs = Vec::new();
        let mut push = |text: &str, color: u32, palette: &mut Palette| {
            if text.is_empty() {
                return;
            }
            let color = palette.index(color);
            match runs.last_mut() {
                Some(last) if last.1 == color => last.0.push_str(text),
                _ => runs.push((text.to_string(), color)),
            }
        };
        let spans = document.get(number).filter(|_| !long(line));
        let mut offset = 0;
        for span in spans.into_iter().flatten() {
            let (start, end) = (
                span.range.start.min(line.len()),
                span.range.end.min(line.len()),
            );
            if start < offset || !line.is_char_boundary(start) || !line.is_char_boundary(end) {
                continue;
            }
            push(&line[offset..start], PLAIN, palette);
            push(&line[start..end], span.color, palette);
            offset = end;
        }
        push(&line[offset..], PLAIN, palette);
        out.push(runs);
    }
    Some(out)
}

fn colourful(palette: &Palette) -> bool {
    palette.index.keys().any(|color| *color != PLAIN)
}

type HighlightCache = VecDeque<(String, Option<Arc<Highlighted>>)>;
static CACHE: LazyLock<Mutex<HighlightCache>> = LazyLock::new(Default::default);

/// A file highlighted whole for the phone's viewer, or None when it is too
/// large, in a language the desktop doesn't colour, or highlighting fails.
/// Recent results are kept, since the phone asks for the same files again.
fn highlight_file(path: &str, text: &str) -> Option<Arc<Highlighted>> {
    let language = language_for(path)?;
    if text.len() > MAX_HIGHLIGHT_BYTES {
        return None;
    }
    let key = format!("{language}\0{text}");
    {
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(at) = cache.iter().position(|(cached, _)| *cached == key) {
            let entry = cache.remove(at)?;
            let value = entry.1.clone();
            cache.push_back(entry);
            return value;
        }
    }
    let mut palette = Palette::default();
    let lines: Vec<&str> = text.split('\n').collect();
    let mut highlighter = LineHighlighter::new(Some(&language), None);
    let value = highlight_lines(&mut highlighter, &lines, &mut palette)
        .filter(|_| colourful(&palette))
        .map(|lines| {
            Arc::new(Highlighted {
                palette: palette.colors,
                lines,
            })
        });
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.push_back((key, value.clone()));
    while cache.len() > CACHE_ENTRIES
        || cache.iter().map(|(key, _)| key.len()).sum::<usize>() > CACHE_BYTES
    {
        if cache.pop_front().is_none() {
            break;
        }
    }
    value
}

/// A diff's lines' syntax colours, for the phone's diff sheet: both versions
/// highlighted whole, each line taking its colours from its side (removed
/// lines from the old version, the rest from the new), in one palette. Only
/// runs that spell the line out exactly are kept. None when neither side
/// could be highlighted.
fn highlight_diff(
    path: &str,
    original: &str,
    current: &str,
    lines: &[Line],
) -> Option<(Vec<String>, Vec<Option<Runs>>)> {
    let before = highlight_file(path, original);
    let after = highlight_file(path, current);
    if before.is_none() && after.is_none() {
        return None;
    }
    let mut palette = after
        .as_ref()
        .map(|after| after.palette.clone())
        .unwrap_or_default();
    let remap: Vec<usize> = before
        .iter()
        .flat_map(|before| before.palette.iter())
        .map(|color| {
            palette
                .iter()
                .position(|known| known == color)
                .unwrap_or_else(|| {
                    palette.push(color.clone());
                    palette.len() - 1
                })
        })
        .collect();
    let line_at = |side: &Option<Arc<Highlighted>>, number: Option<u32>| {
        let index = (number? as usize).checked_sub(1)?;
        side.as_ref()?.lines.get(index).cloned()
    };
    let runs = lines
        .iter()
        .map(|line| {
            let from_before =
                line.kind == LineKind::Del || (line.kind == LineKind::Context && after.is_none());
            let runs = match line.kind {
                LineKind::Hunk => None,
                _ if from_before => line_at(&before, line.old_number).map(|runs| {
                    runs.into_iter()
                        .map(|(text, color)| (text, remap.get(color).copied().unwrap_or(0)))
                        .collect()
                }),
                _ => line_at(&after, line.new_number),
            }?;
            let spelled: String = runs.iter().map(|(text, _)| text.as_str()).collect();
            (spelled == line.text).then_some(runs)
        })
        .collect();
    Some((palette, runs))
}

#[derive(Default)]
struct Job {
    palette: Vec<String>,
    lines: Vec<Runs>,
    done: bool,
}

#[derive(Default)]
struct Jobs {
    next: u64,
    order: VecDeque<String>,
    jobs: HashMap<String, Job>,
}

static JOBS: LazyLock<Mutex<Jobs>> = LazyLock::new(Default::default);

fn jobs() -> std::sync::MutexGuard<'static, Jobs> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Starts highlighting a file too large to highlight at once on its own
/// thread, a chunk at a time, for the phone to collect with `file_highlight`.
/// None for a file without colours (judged by its first lines).
fn highlight_in_background(path: &str, text: String) -> Option<String> {
    let language = language_for(path)?;
    let mut probe = Palette::default();
    let head: Vec<&str> = text.split('\n').take(PROBE_LINES).collect();
    highlight_lines(
        &mut LineHighlighter::new(Some(&language), None),
        &head,
        &mut probe,
    )?;
    if !colourful(&probe) {
        return None;
    }
    let id = {
        let mut jobs = jobs();
        jobs.next += 1;
        let id = jobs.next.to_string();
        jobs.jobs.insert(id.clone(), Job::default());
        jobs.order.push_back(id.clone());
        while jobs.order.len() > MAX_JOBS {
            if let Some(oldest) = jobs.order.pop_front() {
                jobs.jobs.remove(&oldest);
            }
        }
        id
    };
    let job = id.clone();
    let spawned = std::thread::Builder::new()
        .name("mobile-highlight".into())
        .spawn(move || {
            let mut palette = Palette::default();
            // One highlighter for the whole file: a comment or string that
            // spans a chunk boundary keeps its colour.
            let mut highlighter = LineHighlighter::new(Some(&language), None);
            let lines: Vec<&str> = text.split('\n').collect();
            let mut start = 0;
            while start < lines.len() {
                // A phone that stopped asking: the job was dropped, so stop.
                if !jobs().jobs.contains_key(&job) {
                    return;
                }
                let mut end = start;
                let mut bytes = 0;
                while end < lines.len() && (end == start || bytes + lines[end].len() < CHUNK_BYTES)
                {
                    bytes += lines[end].len() + 1;
                    end += 1;
                }
                // What was highlighted stays; the rest of the file stays plain.
                let Some(chunk) =
                    highlight_lines(&mut highlighter, &lines[start..end], &mut palette)
                else {
                    break;
                };
                let mut jobs = jobs();
                let Some(entry) = jobs.jobs.get_mut(&job) else {
                    return;
                };
                entry.palette.clone_from(&palette.colors);
                entry.lines.extend(chunk);
                start = end;
            }
            if let Some(entry) = jobs().jobs.get_mut(&job) {
                entry.done = true;
            }
        });
    if spawned.is_err() {
        jobs().jobs.remove(&id);
        return None;
    }
    Some(id)
}

/// A background highlight's palette and the lines finished from `from` on.
pub(crate) fn file_highlight(job: &str, from: usize) -> Value {
    let jobs = jobs();
    let Some(job) = jobs.jobs.get(job) else {
        return json!({ "palette": [], "lines": [], "done": true });
    };
    let lines: Vec<Value> = job.lines.iter().skip(from).map(runs_json).collect();
    json!({ "palette": job.palette, "lines": lines, "done": job.done })
}

#[cfg(test)]
mod tests;
