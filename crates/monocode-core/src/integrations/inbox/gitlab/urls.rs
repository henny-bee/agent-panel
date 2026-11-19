use super::*;

pub(super) fn validate_kind(kind: &str) -> Result<(), String> {
    if kind == "issue" || kind == "pr" {
        Ok(())
    } else {
        Err("Unknown GitLab task kind".into())
    }
}

pub(super) fn validate_item(kind: &str, number: i64) -> Result<(), String> {
    validate_kind(kind)?;
    if number <= 0 {
        return Err("Invalid GitLab item number".into());
    }
    Ok(())
}

pub(super) fn resource_for_kind(kind: &str) -> &'static str {
    if kind == "pr" {
        "merge_requests"
    } else {
        "issues"
    }
}

pub(super) fn item_path(repo: &str, kind: &str, number: i64) -> String {
    format!(
        "/projects/{}/{}/{number}",
        encode_path_component(repo),
        resource_for_kind(kind)
    )
}

pub(super) fn validate_repo(repo: &str) -> Result<String, String> {
    let repo = repo.trim();
    if valid_project_path(repo) {
        Ok(repo.to_string())
    } else {
        Err("Invalid GitLab project".into())
    }
}

pub(super) fn normalize_gitlab_url(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    let raw = if raw.is_empty() {
        DEFAULT_GITLAB_URL
    } else {
        raw
    };
    if raw.contains("://") && !raw.starts_with("https://") && !raw.starts_with("http://") {
        return Err("GitLab URL must use HTTP or HTTPS".into());
    }
    let value = if raw.starts_with("https://") || raw.starts_with("http://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let (_, rest) = value
        .split_once("://")
        .ok_or_else(|| "GitLab URL must use HTTP or HTTPS".to_string())?;
    let path = rest.split_once('/').map(|(_, path)| path).unwrap_or("");
    if rest.is_empty()
        || rest.starts_with('/')
        || rest.contains('@')
        || rest.contains('?')
        || rest.contains('#')
        || rest.contains('\\')
        || rest.chars().any(char::is_whitespace)
        || path.split('/').any(|segment| {
            matches!(
                segment.to_ascii_lowercase().as_str(),
                "." | ".." | "%2e" | "%2e%2e" | "%2e." | ".%2e"
            )
        })
    {
        return Err("GitLab URL is invalid".into());
    }
    let normalized = value.trim_end_matches('/');
    let normalized = normalized.strip_suffix("/api/v4").unwrap_or(normalized);
    Ok(normalized.trim_end_matches('/').to_string())
}

pub(super) fn encode_path_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(*byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

pub(super) fn gitlab_repo_for(root: &Path, gitlab_url: &str) -> Result<String, String> {
    let mut cmd = Command::new("git");
    crate::sys::process::configure_child(&mut cmd);
    crate::vcs::git::scm::launch_env::apply_gui_env(&mut cmd);
    let output = cmd
        .args(["config", "--get-regexp", r"^remote\..*\.url$"])
        .current_dir(root)
        .output()
        .map_err(|_| "Could not run git".to_string())?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Err("Could not read git remotes".into());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut matches = Vec::new();
    for line in stdout.lines() {
        let Some((name, remote)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        if let Some(repo) = project_from_remote(remote.trim(), gitlab_url) {
            matches.push((name == "remote.origin.url", repo));
        }
    }
    matches
        .iter()
        .find(|(origin, _)| *origin)
        .or_else(|| matches.first())
        .map(|(_, repo)| repo.clone())
        .ok_or_else(|| "No GitLab remote matches the configured host".to_string())
}

pub(super) fn project_from_remote(remote: &str, gitlab_url: &str) -> Option<String> {
    let configured = configured_remote(gitlab_url)?;
    let (authority, mut path) = remote_authority_path(remote)?;
    if host_without_port(&authority) != host_without_port(&configured.authority) {
        return None;
    }
    path = path.trim_matches('/').to_string();
    let prefix = configured.path.trim_matches('/');
    if !prefix.is_empty() {
        path = path.strip_prefix(&format!("{prefix}/"))?.to_string();
    }
    if let Some(stripped) = path.strip_suffix(".git") {
        path = stripped.to_string();
    }
    if !valid_project_path(&path) {
        return None;
    }
    Some(path)
}

pub(super) struct ConfiguredRemote {
    pub(super) authority: String,
    pub(super) path: String,
}

pub(super) fn configured_remote(url: &str) -> Option<ConfiguredRemote> {
    let normalized = normalize_gitlab_url(url).ok()?;
    let (_, rest) = normalized.split_once("://")?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    Some(ConfiguredRemote {
        authority: authority.to_ascii_lowercase(),
        path: path.to_string(),
    })
}

pub(super) fn remote_authority_path(remote: &str) -> Option<(String, String)> {
    let remote = remote.trim();
    if let Some((_, rest)) = remote.split_once("://") {
        let (authority, path) = rest.split_once('/')?;
        let authority = authority.rsplit('@').next()?.to_ascii_lowercase();
        return Some((authority, path.to_string()));
    }
    let (user_host, path) = remote.split_once(':')?;
    let authority = user_host.rsplit('@').next()?.to_ascii_lowercase();
    Some((authority, path.to_string()))
}

pub(super) fn host_without_port(authority: &str) -> String {
    let authority = authority.trim().to_ascii_lowercase();
    if authority.starts_with('[') {
        return authority
            .split(']')
            .next()
            .map(|host| format!("{host}]"))
            .unwrap_or(authority);
    }
    authority
        .split(':')
        .next()
        .unwrap_or(&authority)
        .to_string()
}

pub(super) fn valid_project_path(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    parts.len() >= 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && *part != "."
                && *part != ".."
                && !part.chars().any(char::is_whitespace)
                && !part.contains(['?', '#', '\\'])
        })
}
