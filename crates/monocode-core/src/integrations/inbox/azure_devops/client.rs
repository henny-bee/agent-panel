use super::*;

pub(super) struct AzureResponse {
    pub(super) value: Value,
    pub(super) truncated: bool,
}

pub(super) fn basic_auth(token: &str) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(format!(":{token}"));
    format!("Basic {encoded}")
}

pub(super) fn azure_get(config: &AzureDevOpsConfig, path: &str) -> Result<AzureResponse, String> {
    let url = format!("{}{}", config.url.trim_end_matches('/'), path);
    let agent = azure_agent();
    read_azure_response(
        agent
            .get(&url)
            .set("Authorization", &basic_auth(&config.token))
            .set("Accept", "application/json")
            .set("User-Agent", USER_AGENT)
            .call(),
    )
}

pub(super) fn azure_post_json(
    config: &AzureDevOpsConfig,
    path: &str,
    body: Value,
) -> Result<AzureResponse, String> {
    let url = format!("{}{}", config.url.trim_end_matches('/'), path);
    let payload = serde_json::to_string(&body).map_err(|error| error.to_string())?;
    let agent = azure_agent();
    read_azure_response(
        agent
            .post(&url)
            .set("Authorization", &basic_auth(&config.token))
            .set("Accept", "application/json")
            .set("Content-Type", "application/json")
            .set("User-Agent", USER_AGENT)
            .send_string(&payload),
    )
}

pub(super) fn azure_current_user_id(config: &AzureDevOpsConfig) -> Result<String, String> {
    // connectionData is still under preview and rejects stable api-versions.
    let response = azure_get(
        config,
        &format!("/_apis/connectionData?api-version={CONNECTION_DATA_API_VERSION}"),
    )?;
    let id = response
        .value
        .get("authenticatedUser")
        .and_then(|user| string_field(user, "id"))
        .or_else(|| {
            response
                .value
                .get("authorizedUser")
                .and_then(|user| string_field(user, "id"))
        })
        .unwrap_or_default();
    if id.trim().is_empty() {
        return Err("Azure DevOps did not return the current user".into());
    }
    Ok(id)
}

pub(super) fn azure_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(HTTP_TIMEOUT)
        .redirects(0)
        .build()
}

pub(super) fn read_azure_response(
    result: Result<ureq::Response, ureq::Error>,
) -> Result<AzureResponse, String> {
    let (bytes, status, truncated) = read_azure_bytes(result, None)?;
    let body = String::from_utf8(bytes)
        .map_err(|_| "Azure DevOps returned an unreadable response".to_string())?;
    if !(200..300).contains(&status) {
        return Err(azure_http_error(status, &body));
    }
    let value: Value = serde_json::from_str(&body)
        .map_err(|_| "Azure DevOps returned invalid JSON".to_string())?;
    Ok(AzureResponse { value, truncated })
}

/// Raw-bytes variant for file content, which is not necessarily UTF-8 JSON.
pub(super) fn azure_get_bytes(
    config: &AzureDevOpsConfig,
    path: &str,
) -> Result<(Vec<u8>, bool), String> {
    let url = format!("{}{}", config.url.trim_end_matches('/'), path);
    let agent = azure_agent();
    let (bytes, status, truncated) = read_azure_bytes(
        agent
            .get(&url)
            .set("Authorization", &basic_auth(&config.token))
            .set("Accept", "application/json")
            .set("User-Agent", USER_AGENT)
            .call(),
        // One byte past the cap is enough to flag the content as too large
        // without buffering an unbounded blob.
        Some(MAX_DIFF_FILE_BYTES),
    )?;
    if !(200..300).contains(&status) {
        let body = String::from_utf8_lossy(&bytes);
        return Err(azure_http_error(status, &body));
    }
    Ok((bytes, truncated))
}

pub(super) fn read_azure_bytes(
    result: Result<ureq::Response, ureq::Error>,
    max_bytes: Option<usize>,
) -> Result<(Vec<u8>, u16, bool), String> {
    let response = match result {
        Ok(response) => response,
        Err(ureq::Error::Status(401, _) | ureq::Error::Status(403, _)) => {
            return Err("Azure DevOps personal access token is invalid or lacks permission".into());
        }
        Err(ureq::Error::Status(404, _)) => {
            return Err("Azure DevOps did not find that item".into());
        }
        Err(ureq::Error::Status(status, response)) => {
            let body = response.into_string().unwrap_or_default();
            return Err(azure_http_error(status, &body));
        }
        Err(_) => return Err("Could not reach Azure DevOps".into()),
    };
    let status = response.status();
    let truncated = response
        .header("x-ms-continuationtoken")
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    let mut bytes = Vec::new();
    let read_result = if let Some(max_bytes) = max_bytes {
        response
            .into_reader()
            .take((max_bytes as u64).saturating_add(1))
            .read_to_end(&mut bytes)
    } else {
        response.into_reader().read_to_end(&mut bytes)
    };
    read_result.map_err(|_| "Azure DevOps returned an unreadable response".to_string())?;
    Ok((bytes, status, truncated))
}

pub(super) fn azure_http_error(status: u16, body: &str) -> String {
    let message = serde_json::from_str::<Value>(body).ok().and_then(|value| {
        value
            .get("message")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    });
    message.unwrap_or_else(|| format!("Azure DevOps request failed ({status})"))
}

pub(super) fn config_path(app: &Path) -> Result<PathBuf, String> {
    Ok(app.join("azure-devops-config.json"))
}

pub(super) fn read_config(app: &Path) -> Result<Option<AzureDevOpsConfig>, String> {
    let path = config_path(app)?;
    match fs::read_to_string(path) {
        Ok(raw) => {
            let mut config: AzureDevOpsConfig = serde_json::from_str(&raw)
                .map_err(|_| "Azure DevOps settings are invalid".to_string())?;
            config.url = normalize_azure_devops_url(&config.url)?;
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

pub(super) fn require_config(app: &Path) -> Result<AzureDevOpsConfig, String> {
    read_config(app)?.ok_or_else(|| "Connect Azure DevOps in Settings".to_string())
}

pub(super) fn write_config(app: &Path, config: &AzureDevOpsConfig) -> Result<(), String> {
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
