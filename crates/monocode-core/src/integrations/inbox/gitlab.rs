//! Native adapter of the original Tauri Inbox provider. Network parsing and secret-file formats remain identical.
mod client;
mod parse;
mod urls;
use client::{
    delete_config, expand_home, gitlab_get, gitlab_get_request, gitlab_post_form, read_config,
    read_gitlab_response, require_config, write_config,
};
use parse::{
    note_url, parse_mr_diff, parse_todos, parse_work_item_details, parse_work_item_thread,
    parse_work_items,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use urls::{
    encode_path_component, gitlab_repo_for, item_path, normalize_gitlab_url, resource_for_kind,
    valid_project_path, validate_item, validate_kind, validate_repo,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

const DEFAULT_GITLAB_URL: &str = "https://gitlab.com";
const DEFAULT_LIMIT: u32 = 40;
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;
const USER_AGENT: &str = "MonoCode";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitlabStatus {
    pub connected: bool,
    pub url: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct GitlabConfig {
    url: String,
    token: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitlabLabel {
    pub name: String,
    pub color: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitlabAssignee {
    pub login: String,
    pub avatar_url: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitlabWorkItem {
    pub kind: String,
    pub number: i64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub updated_at: String,
    pub labels: Vec<GitlabLabel>,
    pub assignees: Vec<GitlabAssignee>,
    pub draft: bool,
    pub repo: String,
    pub attention_reason: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitlabWorkItemDetails {
    pub body: String,
    pub author: String,
    pub author_avatar_url: String,
    pub base_ref_name: String,
    pub head_ref_name: String,
    pub review_decision: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitlabWorkItemComment {
    pub id: String,
    pub kind: String,
    pub author: String,
    pub author_avatar_url: String,
    pub body: String,
    pub created_at: String,
    pub url: String,
    pub state: String,
    pub path: String,
    pub line: Option<i64>,
    pub resolved: bool,
    pub thread_id: String,
    pub replies: Vec<GitlabWorkItemComment>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitlabWorkItemThread {
    pub comments: Vec<GitlabWorkItemComment>,
    pub truncated: bool,
    pub review_decision: String,
    pub base_ref_name: String,
    pub head_ref_name: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitlabMrFile {
    pub path: String,
    pub additions: i64,
    pub deletions: i64,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitlabMrDiff {
    pub additions: i64,
    pub deletions: i64,
    pub files: Vec<GitlabMrFile>,
    pub patch: String,
    pub truncated: bool,
}

pub fn gitlab_status(app: PathBuf) -> Result<GitlabStatus, String> {
    let config = read_config(&app)?;
    Ok(GitlabStatus {
        connected: config.is_some(),
        url: config
            .map(|config| config.url)
            .unwrap_or_else(|| DEFAULT_GITLAB_URL.into()),
    })
}

pub fn gitlab_set_config(app: PathBuf, url: String, token: String) -> Result<GitlabStatus, String> {
    let url = normalize_gitlab_url(&url)?;
    let token = token.trim().to_string();
    if token.is_empty() {
        delete_config(&app)?;
        return Ok(GitlabStatus {
            connected: false,
            url,
        });
    }
    let config = GitlabConfig { url, token };
    let response = gitlab_get(&config, "/user")?;
    if response.value.get("id").and_then(Value::as_i64).is_none() {
        return Err("GitLab did not return the current user".into());
    }
    write_config(&app, &config)?;
    Ok(GitlabStatus {
        connected: true,
        url: config.url,
    })
}

pub fn gitlab_repo(app: PathBuf, cwd: String) -> Result<String, String> {
    let config = require_config(&app)?;
    gitlab_repo_for(&expand_home(&cwd), &config.url)
}

pub fn gitlab_list_work_items(
    app: PathBuf,
    cwd: String,
    kind: String,
    assigned_to_me: bool,
    state: String,
    limit: Option<u32>,
) -> Result<Vec<GitlabWorkItem>, String> {
    let config = require_config(&app)?;
    let repo = gitlab_repo_for(&expand_home(&cwd), &config.url)?;
    gitlab_list_work_items_for(
        &config,
        &repo,
        &kind,
        assigned_to_me,
        &state,
        limit.unwrap_or(DEFAULT_LIMIT),
    )
}

pub fn gitlab_list_todos(
    app: PathBuf,
    kind: String,
    limit: Option<u32>,
) -> Result<Vec<GitlabWorkItem>, String> {
    let config = require_config(&app)?;
    gitlab_list_todos_for(&config, &kind, limit.unwrap_or(DEFAULT_LIMIT))
}

pub fn gitlab_work_item_details(
    app: PathBuf,
    repo: String,
    kind: String,
    number: i64,
) -> Result<GitlabWorkItemDetails, String> {
    let config = require_config(&app)?;
    let repo = validate_repo(&repo)?;
    gitlab_work_item_details_for(&config, &repo, &kind, number)
}

pub fn gitlab_work_item_thread(
    app: PathBuf,
    repo: String,
    kind: String,
    number: i64,
) -> Result<GitlabWorkItemThread, String> {
    let config = require_config(&app)?;
    let repo = validate_repo(&repo)?;
    gitlab_work_item_thread_for(&config, &repo, &kind, number)
}

pub fn gitlab_work_item_comment(
    app: PathBuf,
    repo: String,
    kind: String,
    number: i64,
    body: String,
) -> Result<String, String> {
    let config = require_config(&app)?;
    let repo = validate_repo(&repo)?;
    gitlab_work_item_comment_for(&config, &repo, &kind, number, &body)
}

pub fn gitlab_mr_diff(app: PathBuf, repo: String, number: i64) -> Result<GitlabMrDiff, String> {
    let config = require_config(&app)?;
    let repo = validate_repo(&repo)?;
    gitlab_mr_diff_for(&config, &repo, number)
}

fn gitlab_list_work_items_for(
    config: &GitlabConfig,
    repo: &str,
    kind: &str,
    assigned_to_me: bool,
    state: &str,
    limit: u32,
) -> Result<Vec<GitlabWorkItem>, String> {
    validate_kind(kind)?;
    let resource = resource_for_kind(kind);
    let state = if state.trim().eq_ignore_ascii_case("all") {
        "all"
    } else {
        "opened"
    };
    let limit = limit.clamp(1, 100);
    let assigned = if assigned_to_me {
        "&scope=assigned_to_me"
    } else {
        "&scope=all"
    };
    let path = format!(
        "/projects/{}/{resource}?state={state}&order_by=updated_at&sort=desc&per_page={limit}&with_labels_details=true{assigned}",
        encode_path_component(repo)
    );
    let response = gitlab_get(config, &path)?;
    parse_work_items(&response.value, kind, repo)
}

fn gitlab_list_todos_for(
    config: &GitlabConfig,
    kind: &str,
    limit: u32,
) -> Result<Vec<GitlabWorkItem>, String> {
    validate_kind(kind)?;
    let target_type = if kind == "pr" {
        "MergeRequest"
    } else {
        "Issue"
    };
    let limit = limit.clamp(1, 100);
    let path = format!("/todos?state=pending&type={target_type}&per_page={limit}");
    let response = gitlab_get(config, &path)?;
    parse_todos(&response.value, kind)
}

fn gitlab_work_item_details_for(
    config: &GitlabConfig,
    repo: &str,
    kind: &str,
    number: i64,
) -> Result<GitlabWorkItemDetails, String> {
    validate_item(kind, number)?;
    let path = item_path(repo, kind, number);
    let response = gitlab_get(config, &path)?;
    parse_work_item_details(&response.value, kind)
}

fn gitlab_work_item_thread_for(
    config: &GitlabConfig,
    repo: &str,
    kind: &str,
    number: i64,
) -> Result<GitlabWorkItemThread, String> {
    validate_item(kind, number)?;
    let path = format!(
        "{}/notes?order_by=created_at&sort=desc&per_page=100",
        item_path(repo, kind, number)
    );
    let response = gitlab_get(config, &path)?;
    parse_work_item_thread(
        &response.value,
        &config.url,
        repo,
        kind,
        number,
        response.has_next_page,
    )
}

fn gitlab_work_item_comment_for(
    config: &GitlabConfig,
    repo: &str,
    kind: &str,
    number: i64,
    body: &str,
) -> Result<String, String> {
    validate_item(kind, number)?;
    let body = body.trim();
    if body.is_empty() {
        return Err("Comment cannot be empty".into());
    }
    let path = format!("{}/notes", item_path(repo, kind, number));
    let response = gitlab_post_form(config, &path, &[("body", body)])?;
    let id = response
        .value
        .get("id")
        .and_then(Value::as_i64)
        .ok_or_else(|| "GitLab did not return a comment".to_string())?;
    Ok(note_url(&config.url, repo, kind, number, id))
}

fn gitlab_mr_diff_for(
    config: &GitlabConfig,
    repo: &str,
    number: i64,
) -> Result<GitlabMrDiff, String> {
    validate_item("pr", number)?;
    let path = format!(
        "/projects/{}/merge_requests/{number}/diffs?per_page=100",
        encode_path_component(repo)
    );
    match gitlab_get_request(config, &path).call() {
        // Older self-hosted GitLab versions expose /changes but not /diffs.
        // Match the HTTP status so permission and transport errors still surface.
        Err(ureq::Error::Status(404, _)) => {
            let path = format!("{}/changes", item_path(repo, "pr", number));
            let response = gitlab_get(config, &path)?;
            let changes = response
                .value
                .get("changes")
                .ok_or_else(|| "GitLab did not return merge request diffs".to_string())?;
            let overflow = response
                .value
                .get("overflow")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            parse_mr_diff(changes, response.has_next_page || overflow)
        }
        result => {
            let response = read_gitlab_response(result)?;
            parse_mr_diff(&response.value, response.has_next_page)
        }
    }
}
