use super::*;

pub(super) fn pr_link(row: &Value) -> Option<String> {
    row.get("_links")
        .and_then(|links| links.get("web"))
        .and_then(|web| string_field(web, "href"))
        .filter(|href| !href.is_empty())
}

pub(super) fn pr_url_from_repository(row: &Value, number: i64) -> Option<String> {
    let remote = row
        .get("repository")
        .and_then(|repo| string_field(repo, "remoteUrl"))
        .filter(|url| !url.is_empty())?;
    Some(format!(
        "{}/pullrequest/{number}",
        remote.trim_end_matches('/')
    ))
}

pub(super) fn pr_url_from_parts(repo: &str, base_url: &str, number: i64) -> Option<String> {
    let (project, repo_name) = split_repo(repo).ok()?;
    let base = base_url.trim_end_matches('/').trim();
    if base.is_empty() {
        return None;
    }
    Some(format!(
        "{base}/{}/_git/{}/pullrequest/{number}",
        project.trim(),
        repo_name.trim()
    ))
}

pub(super) fn pr_repo_parts(row: &Value) -> (Option<String>, Option<String>) {
    let repository = row.get("repository");
    let project = repository
        .and_then(|repo| repo.get("project"))
        .and_then(|project| string_field(project, "name"));
    let repo_name = repository.and_then(|repo| string_field(repo, "name"));
    (project, repo_name)
}

pub(super) fn pr_web_url(
    config: &AzureDevOpsConfig,
    project: &str,
    repo: &str,
    number: i64,
) -> String {
    format!(
        "{}/{}/_git/{}/pullrequest/{}",
        config.url.trim_end_matches('/'),
        project.trim(),
        repo.trim(),
        number
    )
}

pub(super) fn short_ref(value: &str) -> String {
    value
        .trim()
        .strip_prefix("refs/heads/")
        .unwrap_or(value.trim())
        .to_string()
}

pub(super) fn validate_kind(kind: &str) -> Result<(), String> {
    if kind == "issue" || kind == "pr" {
        Ok(())
    } else {
        Err("Unknown Azure DevOps task kind".into())
    }
}

pub(super) fn validate_item(kind: &str, number: i64) -> Result<(), String> {
    validate_kind(kind)?;
    if number <= 0 {
        return Err("Invalid Azure DevOps item number".into());
    }
    Ok(())
}

/// Boards work items carry only the project name in `repo` (e.g. `"platform"`
/// instead of `"platform/web"`), so comment paths take the leading segment.
pub(super) fn wit_project(repo: &str) -> Result<String, String> {
    let project = repo.split('/').next().unwrap_or("").trim().to_string();
    if project.is_empty() || project == "." || project == ".." {
        return Err("Invalid Azure DevOps project".into());
    }
    Ok(project)
}

pub(super) fn split_repo(repo: &str) -> Result<(String, String), String> {
    let repo = validate_repo(repo)?;
    let (project, name) = repo
        .split_once('/')
        .ok_or_else(|| "Invalid Azure DevOps repository".to_string())?;
    Ok((project.to_string(), name.to_string()))
}

pub(super) fn validate_repo(repo: &str) -> Result<String, String> {
    let repo = repo.trim();
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|part| {
            part.trim().is_empty()
                || *part == "."
                || *part == ".."
                || part.contains(['?', '#', '\\'])
                || part.chars().any(char::is_control)
        })
    {
        return Err("Invalid Azure DevOps repository".into());
    }
    Ok(repo.to_string())
}

pub(super) fn normalize_azure_devops_url(raw: &str) -> Result<String, String> {
    let raw = raw.trim().trim_end_matches('/');
    if raw.is_empty() {
        return Err(
            "Enter your Azure DevOps organization URL (e.g. https://dev.azure.com/myorg)".into(),
        );
    }
    if raw.chars().any(char::is_whitespace) || raw.contains(['?', '#', '\\']) {
        return Err("Azure DevOps URL is invalid".into());
    }
    // Bare `myorg` becomes the hosted organization URL.
    let with_scheme = if raw.contains("://") {
        raw.to_string()
    } else if raw.contains('.') || raw.contains('/') {
        format!("https://{raw}")
    } else {
        format!("https://dev.azure.com/{raw}")
    };
    // The PAT travels on every request as Basic auth, so cleartext HTTP is
    // never accepted, including for on-premises hosts.
    if !with_scheme.starts_with("https://") {
        return Err("Azure DevOps URL must use HTTPS".into());
    }
    let (_, rest) = with_scheme
        .split_once("://")
        .ok_or_else(|| "Azure DevOps URL is invalid".to_string())?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    if authority.contains('@') {
        return Err("Azure DevOps URL is invalid".into());
    }
    // Accept on-premises authorities such as `tfs.contoso.com:8080` or a
    // dotless `tfs` host; the port must be numeric when present.
    let (host, port) = authority.split_once(':').unwrap_or((authority, ""));
    if host.is_empty() || (!port.is_empty() && port.parse::<u16>().is_err()) {
        return Err("Azure DevOps URL is invalid".into());
    }
    let mut segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    // Drop API/git suffixes pasted from the browser address bar.
    if let Some(index) = segments.iter().position(|segment| {
        segment.eq_ignore_ascii_case("_apis") || segment.eq_ignore_ascii_case("_git")
    }) {
        segments.truncate(index);
    }
    let authority_lower = authority.to_ascii_lowercase();
    if authority_lower == "dev.azure.com" || authority_lower == "ssh.dev.azure.com" {
        let org = segments.first().ok_or_else(|| {
            "Azure DevOps URL must include your organization (e.g. https://dev.azure.com/myorg)"
                .to_string()
        })?;
        return Ok(format!("https://dev.azure.com/{}", org.trim()));
    }
    if let Some(org) = authority_lower.strip_suffix(".visualstudio.com") {
        let org = org.trim();
        if org.is_empty() || org.contains('.') {
            return Err("Azure DevOps URL is invalid".into());
        }
        return Ok(format!("https://dev.azure.com/{org}"));
    }
    if segments.is_empty() {
        return Err("Azure DevOps URL must include your organization or collection".into());
    }
    // On-premises (e.g. https://host/tfs/collection): keep host plus the
    // collection path so remotes match the same base.
    let kept = segments
        .iter()
        .take(2)
        .cloned()
        .collect::<Vec<_>>()
        .join("/");
    let scheme = with_scheme
        .split_once("://")
        .map(|(scheme, _)| scheme)
        .unwrap_or("https");
    Ok(format!("{}://{}/{}", scheme, authority, kept))
}

pub(super) fn organization_from_url(url: &str) -> String {
    canonical_org_key(url).unwrap_or_default()
}

pub(super) fn canonical_org_key(url: &str) -> Option<String> {
    let normalized = normalize_azure_devops_url(url).ok()?;
    Some(normalized.to_ascii_lowercase())
}

pub(super) fn encode_segment(value: &str) -> String {
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

pub(super) fn azure_devops_repo_for(root: &Path, organization_url: &str) -> Result<String, String> {
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
        if let Some(repo) = project_repo_from_remote(remote.trim(), organization_url) {
            matches.push((name == "remote.origin.url", repo));
        }
    }
    matches
        .iter()
        .find(|(origin, _)| *origin)
        .or_else(|| matches.first())
        .map(|(_, repo)| repo.clone())
        .ok_or_else(|| "No Azure DevOps remote matches the configured organization".to_string())
}

pub(super) fn project_repo_from_remote(remote: &str, organization_url: &str) -> Option<String> {
    let expected = canonical_org_key(organization_url)?;
    let (org_key, project, repo) = parse_azure_remote(remote)?;
    if org_key.to_ascii_lowercase() != expected {
        return None;
    }
    if !valid_repo_parts(&project, &repo) {
        return None;
    }
    Some(format!("{project}/{repo}"))
}

pub(super) fn parse_azure_remote(remote: &str) -> Option<(String, String, String)> {
    let remote = remote.trim();
    if remote.is_empty() {
        return None;
    }
    // SSH: git@ssh.dev.azure.com:v3/{org}/{project}/{repo} (no scheme).
    if !remote.contains("://") && remote.contains('@') {
        if let Some((_, path)) = remote.split_once(':') {
            let path = path.strip_prefix("v3/").unwrap_or(path);
            let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
            if parts.len() >= 3 && remote.contains("dev.azure.com") {
                let org = parts[0].to_string();
                let project = percent_decode(parts[1]);
                let repo = percent_decode(parts[2..].join("/").trim_end_matches(".git"));
                // Repository names never contain `/`; extra segments belong to
                // on-prem collection paths, which HTTPS remotes already cover.
                let repo = repo.split('/').next_back().unwrap_or("").to_string();
                return Some((
                    format!("https://dev.azure.com/{}", org.to_ascii_lowercase()),
                    project,
                    repo,
                ));
            }
        }
    }
    // HTTPS (credentials stripped first).
    let without_scheme = remote.split_once("://")?.1;
    let after_auth = without_scheme.rsplit('@').next().unwrap_or(without_scheme);
    let (authority, path) = after_auth.split_once('/')?;
    let authority_lower = authority.to_ascii_lowercase();
    let segments: Vec<String> = path
        .split('/')
        .filter(|part| !part.is_empty())
        .map(percent_decode)
        .collect();
    if authority_lower == "dev.azure.com" {
        // {org}/{project}/_git/{repo}
        if segments.len() >= 4 && segments[2].eq_ignore_ascii_case("_git") {
            let org_key = format!("https://dev.azure.com/{}", segments[0].to_ascii_lowercase());
            let repo = segments[3..]
                .join("/")
                .trim_end_matches(".git")
                .trim()
                .to_string();
            return Some((org_key, segments[1].clone(), repo));
        }
        return None;
    }
    if let Some(org) = authority_lower.strip_suffix(".visualstudio.com") {
        // {project}/_git/{repo}
        if segments.len() >= 3 && segments[1].eq_ignore_ascii_case("_git") {
            let org_key = format!("https://dev.azure.com/{}", org.to_ascii_lowercase());
            let repo = segments[2..]
                .join("/")
                .trim_end_matches(".git")
                .trim()
                .to_string();
            return Some((org_key, segments[0].clone(), repo));
        }
        return None;
    }
    // On-premises or custom host: {collection...}/{project}/_git/{repo}
    let git_index = segments
        .iter()
        .position(|segment| segment.eq_ignore_ascii_case("_git"))?;
    if git_index < 2 || git_index + 1 >= segments.len() {
        return None;
    }
    let project = segments[git_index - 1].clone();
    let repo = segments[git_index + 1..]
        .join("/")
        .trim_end_matches(".git")
        .trim()
        .to_string();
    let org_path = segments[..git_index - 1].join("/");
    Some((
        format!("https://{authority_lower}/{org_path}").to_ascii_lowercase(),
        project,
        repo,
    ))
}

pub(super) fn percent_decode(value: &str) -> String {
    let mut out: Vec<u8> = Vec::new();
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            {
                out.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out)
        .trim()
        .trim_end_matches(".git")
        .to_string()
}

pub(super) fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(super) fn valid_repo_parts(project: &str, repo: &str) -> bool {
    for part in [project, repo] {
        if part.trim().is_empty()
            || part == "."
            || part == ".."
            || part.contains(['?', '#', '\\'])
            || part.chars().any(char::is_control)
        {
            return false;
        }
    }
    !project.contains('/') && !repo.contains('/')
}
