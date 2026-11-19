use super::*;

/// Outcome of rendering one changed file. `Skipped` keeps the file listed
/// without hunks; only `too_large` marks the whole diff truncated.
pub(super) enum FileDiff {
    Ready {
        additions: i64,
        deletions: i64,
        block: String,
    },
    Unchanged,
    Skipped {
        too_large: bool,
    },
}

pub(super) fn diff_iteration_file(
    config: &AzureDevOpsConfig,
    project: &str,
    repo_name: &str,
    change: &IterationChange,
    source_sha: &str,
    base_sha: &str,
) -> Result<FileDiff, String> {
    let change_type = change.change_type.to_ascii_lowercase();
    let need_base = change_type != "add";
    let need_target = change_type != "delete";
    if need_base && base_sha.trim().is_empty() {
        return Ok(FileDiff::Skipped { too_large: false });
    }
    if need_target && source_sha.trim().is_empty() {
        return Ok(FileDiff::Skipped { too_large: false });
    }
    let base = if need_base {
        azure_file_content(config, project, repo_name, &change.path, base_sha)?
    } else {
        None
    };
    let target = if need_target {
        azure_file_content(config, project, repo_name, &change.path, source_sha)?
    } else {
        None
    };
    if let (Some(FileContent::TooLarge), _) | (_, Some(FileContent::TooLarge)) = (&base, &target) {
        return Ok(FileDiff::Skipped { too_large: true });
    }
    if let (Some(FileContent::Binary), _) | (_, Some(FileContent::Binary)) = (&base, &target) {
        return Ok(FileDiff::Ready {
            additions: 0,
            deletions: 0,
            block: format!(
                "diff --git a/{0} b/{0}\nBinary files a/{0} and b/{0} differ\n",
                change.path
            ),
        });
    }
    let base_bytes = base.and_then(|content| content.into_text());
    let target_bytes = target.and_then(|content| content.into_text());
    if base_bytes.is_none() && target_bytes.is_none() {
        return Ok(FileDiff::Skipped { too_large: false });
    }
    if base_bytes == target_bytes {
        return Ok(FileDiff::Unchanged);
    }
    // Sides are already capped at MAX_DIFF_FILE_BYTES by azure_file_content.
    let is_add = base_bytes.is_none();
    let is_delete = target_bytes.is_none();
    let Some((additions, deletions, hunks)) =
        git_unified_hunks(base_bytes.as_deref(), target_bytes.as_deref())
    else {
        return Ok(FileDiff::Skipped { too_large: false });
    };
    let mut block = format!("diff --git a/{0} b/{0}\n", change.path);
    if is_add {
        block.push_str("new file mode 100644\n");
    }
    if is_delete {
        block.push_str("deleted file mode 100644\n");
    }
    if is_add {
        block.push_str(&format!("--- /dev/null\n+++ b/{}\n", change.path));
    } else if is_delete {
        block.push_str(&format!("--- a/{}\n+++ /dev/null\n", change.path));
    } else {
        block.push_str(&format!("--- a/{0}\n+++ b/{0}\n", change.path));
    }
    block.push_str(&hunks);
    Ok(FileDiff::Ready {
        additions,
        deletions,
        block,
    })
}

pub(super) enum FileContent {
    Text(Vec<u8>),
    Binary,
    TooLarge,
}

impl FileContent {
    pub(super) fn into_text(self) -> Option<Vec<u8>> {
        match self {
            FileContent::Text(bytes) => Some(bytes),
            FileContent::Binary | FileContent::TooLarge => None,
        }
    }
}

pub(super) fn is_binary_bytes(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|byte| *byte == 0)
}

/// Fetch one file version via the Items API. `Ok(None)` means the path does
/// not exist at that commit (the other side of an add/delete).
pub(super) fn azure_file_content(
    config: &AzureDevOpsConfig,
    project: &str,
    repo_name: &str,
    path: &str,
    sha: &str,
) -> Result<Option<FileContent>, String> {
    let encoded = path
        .split('/')
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/");
    let url_path = format!(
        "/{}/_apis/git/repositories/{}/items?path=/{}&versionDescriptor.version={}&versionDescriptor.versionType=commit&includeContent=true&api-version={}",
        encode_segment(project),
        encode_segment(repo_name),
        encoded,
        encode_segment(sha.trim()),
        API_VERSION
    );
    let (bytes, _) = match azure_get_bytes(config, &url_path) {
        Ok(content) => content,
        Err(error) if error == "Azure DevOps did not find that item" => return Ok(None),
        Err(error) => return Err(error),
    };
    if bytes.len() > MAX_DIFF_FILE_BYTES {
        // Oversized bodies stay listed without hunks; the caller marks the
        // diff truncated.
        return Ok(Some(FileContent::TooLarge));
    }
    Ok(Some(sniff_content(&bytes)))
}

pub(super) fn sniff_content(bytes: &[u8]) -> FileContent {
    let trimmed_start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .map(|start| &bytes[start..])
        .unwrap_or(&[]);
    if trimmed_start.starts_with(b"{") {
        if let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(bytes) {
            if map
                .get("isTruncated")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                return FileContent::TooLarge;
            }
            if let Some(content) = map.get("content").and_then(Value::as_str) {
                let text = content.as_bytes();
                if is_binary_bytes(text) {
                    return FileContent::Binary;
                }
                return FileContent::Text(text.to_vec());
            }
        }
    }
    if is_binary_bytes(bytes) {
        return FileContent::Binary;
    }
    FileContent::Text(bytes.to_vec())
}

/// Creates a temporary directory with an unpredictable name and, on Unix,
/// owner-only permissions, so other local users on a shared machine cannot
/// guess or read the diff scratch files.
pub(super) fn create_secure_tmp_dir() -> Option<PathBuf> {
    let dir = std::env::temp_dir().join(format!(
        "monocode-ado-diff-{}",
        crate::provider::accounts::request_id()
    ));
    if fs::create_dir(&dir).is_err() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).is_err() {
            let _ = fs::remove_dir_all(&dir);
            return None;
        }
    }
    Some(dir)
}

/// Unified hunks between two file versions via the local `git` binary (the
/// provider already requires git for remote detection). Returns `None` when
/// the sides are identical or git is unavailable/fails, so callers degrade to
/// a hunk-less file entry instead of failing the whole diff.
pub(super) fn git_unified_hunks(
    old: Option<&[u8]>,
    new: Option<&[u8]>,
) -> Option<(i64, i64, String)> {
    let dir = create_secure_tmp_dir()?;
    let cleanup = || {
        let _ = fs::remove_dir_all(&dir);
    };
    let old_path = dir.join("old");
    let new_path = dir.join("new");
    if let Some(bytes) = old {
        if fs::write(&old_path, bytes).is_err() {
            cleanup();
            return None;
        }
    }
    if let Some(bytes) = new {
        if fs::write(&new_path, bytes).is_err() {
            cleanup();
            return None;
        }
    }
    // Isolate the subprocess from user configuration: a global
    // `diff.external`, textconv driver, or attributes file could otherwise
    // replace the unified output this parser expects.
    let null_device = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let null_file: &Path = Path::new(null_device);
    let mut cmd = Command::new("git");
    crate::sys::process::configure_child(&mut cmd);
    crate::vcs::git::scm::launch_env::apply_gui_env(&mut cmd);
    let output = cmd
        .args([
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.safecrlf=false",
            "-c",
            "core.quotepath=false",
        ])
        .arg("-c")
        .arg(format!("core.attributesFile={null_device}"))
        .args([
            "diff",
            "--no-index",
            "--no-ext-diff",
            "--no-textconv",
            "--unified=3",
            "--no-color",
            "--",
        ])
        .arg(if old.is_some() {
            old_path.as_path()
        } else {
            null_file
        })
        .arg(if new.is_some() {
            new_path.as_path()
        } else {
            null_file
        })
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", null_device)
        .current_dir(&dir)
        .output();
    cleanup();
    let output = output.ok()?;
    // Exit 0 means identical; 1 means differences; anything else is a failure.
    if output
        .status
        .code()
        .is_none_or(|code| code != 0 && code != 1)
    {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let (additions, deletions, hunks, binary) = parse_git_diff_hunks(&stdout);
    if binary || !hunks.is_empty() {
        return Some((additions, deletions, hunks));
    }
    // Exit 1 without hunks only happens for mode-only changes; surface the
    // file without hunks rather than dropping it.
    if output.status.code() == Some(1) {
        return Some((0, 0, String::new()));
    }
    None
}

/// Split `git diff` output into counts plus the `@@` hunks (or the binary
/// marker). Header lines are regenerated by the caller with repo paths.
pub(super) fn parse_git_diff_hunks(output: &str) -> (i64, i64, String, bool) {
    let mut additions = 0_i64;
    let mut deletions = 0_i64;
    let mut hunks = String::new();
    let mut in_hunks = false;
    for line in output.lines() {
        if line.starts_with("Binary files ") {
            return (0, 0, String::new(), true);
        }
        if line.starts_with("@@") {
            in_hunks = true;
        }
        if !in_hunks {
            continue;
        }
        hunks.push_str(line);
        hunks.push('\n');
        if line.starts_with('+') && !line.starts_with("+++") {
            additions += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            deletions += 1;
        }
    }
    (additions, deletions, hunks, false)
}
