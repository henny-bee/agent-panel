use super::*;

pub(super) struct GitlabResponse {
    pub(super) value: Value,
    pub(super) has_next_page: bool,
}

pub(super) fn gitlab_get(config: &GitlabConfig, path: &str) -> Result<GitlabResponse, String> {
    read_gitlab_response(gitlab_get_request(config, path).call())
}

pub(super) fn gitlab_get_request(config: &GitlabConfig, path: &str) -> ureq::Request {
    let url = format!("{}/api/v4{}", config.url.trim_end_matches('/'), path);
    let agent = gitlab_agent();
    agent
        .get(&url)
        .set("PRIVATE-TOKEN", &config.token)
        .set("Accept", "application/json")
        .set("User-Agent", USER_AGENT)
}

pub(super) fn gitlab_post_form(
    config: &GitlabConfig,
    path: &str,
    fields: &[(&str, &str)],
) -> Result<GitlabResponse, String> {
    let url = format!("{}/api/v4{}", config.url.trim_end_matches('/'), path);
    let agent = gitlab_agent();
    read_gitlab_response(
        agent
            .post(&url)
            .set("PRIVATE-TOKEN", &config.token)
            .set("Accept", "application/json")
            .set("User-Agent", USER_AGENT)
            .send_form(fields),
    )
}

pub(super) fn gitlab_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(HTTP_TIMEOUT)
        .redirects(0)
        .build()
}

pub(super) fn read_gitlab_response(
    result: Result<ureq::Response, ureq::Error>,
) -> Result<GitlabResponse, String> {
    let response = match result {
        Ok(response) => response,
        Err(ureq::Error::Status(401, _)) | Err(ureq::Error::Status(403, _)) => {
            return Err("GitLab access token is invalid or lacks permission".into());
        }
        Err(ureq::Error::Status(status, response)) => {
            let body = response.into_string().unwrap_or_default();
            return Err(gitlab_http_error(status, &body));
        }
        Err(_) => return Err("Could not reach GitLab".into()),
    };
    let status = response.status();
    let has_next_page = response
        .header("X-Next-Page")
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    let body = response
        .into_string()
        .map_err(|_| "GitLab returned an unreadable response".to_string())?;
    if !(200..300).contains(&status) {
        return Err(gitlab_http_error(status, &body));
    }
    let value =
        serde_json::from_str(&body).map_err(|_| "GitLab returned invalid JSON".to_string())?;
    Ok(GitlabResponse {
        value,
        has_next_page,
    })
}

pub(super) fn gitlab_http_error(status: u16, body: &str) -> String {
    let message = serde_json::from_str::<Value>(body).ok().and_then(|value| {
        value
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| value.get("error").and_then(Value::as_str))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    });
    message.unwrap_or_else(|| format!("GitLab request failed ({status})"))
}

pub(super) fn config_path(app: &Path) -> Result<PathBuf, String> {
    Ok(app.join("gitlab-config.json"))
}

pub(super) fn read_config(app: &Path) -> Result<Option<GitlabConfig>, String> {
    let path = config_path(app)?;
    match fs::read_to_string(path) {
        Ok(raw) => {
            let mut config: GitlabConfig = serde_json::from_str(&raw)
                .map_err(|_| "GitLab settings are invalid".to_string())?;
            config.url = normalize_gitlab_url(&config.url)?;
            config.token = config.token.trim().to_string();
            if config.token.is_empty() {
                Ok(None)
            } else {
                Ok(Some(config))
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

pub(super) fn require_config(app: &Path) -> Result<GitlabConfig, String> {
    read_config(app)?.ok_or_else(|| "Connect GitLab in Settings".to_string())
}

pub(super) fn write_config(app: &Path, config: &GitlabConfig) -> Result<(), String> {
    let path = config_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let value = serde_json::to_string(config).map_err(|error| error.to_string())?;
    write_secret_file(&path, &value)
}

pub(super) fn delete_config(app: &Path) -> Result<(), String> {
    let path = config_path(app)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

pub(super) fn write_secret_file(path: &Path, value: &str) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|error| error.to_string())?;
        file.write_all(value.as_bytes())
            .map_err(|error| error.to_string())?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        fs::write(path, value).map_err(|error| error.to_string())
    }
}

pub(super) fn expand_home(input: &str) -> PathBuf {
    if input == "~" {
        return crate::vcs::git::scm::launch_env::dirs_home()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(input));
    }
    if let Some(rest) = input.strip_prefix("~/") {
        return crate::vcs::git::scm::launch_env::dirs_home()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("~"))
            .join(rest);
    }
    PathBuf::from(input)
}
