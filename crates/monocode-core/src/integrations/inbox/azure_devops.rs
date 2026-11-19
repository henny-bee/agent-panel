//! Native adapter of the original Tauri Inbox provider. Network parsing and secret-file formats remain identical.
mod client;
mod diff;
mod parse;
mod urls;
use client::{
    azure_current_user_id, azure_get, azure_get_bytes, azure_post_json, delete_config, expand_home,
    read_config, require_config, write_config,
};
use diff::{diff_iteration_file, FileDiff};
use parse::{
    latest_iteration_commits, parse_iteration_change_list, parse_pr, parse_pr_details,
    parse_pr_list, parse_pr_thread, parse_wit, parse_wit_details, parse_wit_thread, string_field,
    IterationChange,
};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use urls::{
    azure_devops_repo_for, encode_segment, normalize_azure_devops_url, organization_from_url,
    pr_link, pr_repo_parts, pr_url_from_parts, pr_url_from_repository, pr_web_url, short_ref,
    split_repo, validate_item, validate_kind, validate_repo, wit_project,
};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const DEFAULT_LIMIT: u32 = 40;
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;
/// Per-file content cap for hunk generation; larger files stay listed.
const MAX_DIFF_FILE_BYTES: usize = 512 * 1024;
/// Hunks render for the first N files; the rest stay listed without hunks.
const MAX_DIFF_HUNK_FILES: usize = 40;
const USER_AGENT: &str = "MonoCode";
const API_VERSION: &str = "7.1";
const CONNECTION_DATA_API_VERSION: &str = "7.1-preview.1";
const WIT_COMMENTS_API_VERSION: &str = "7.1-preview.4";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AzureDevOpsStatus {
    pub connected: bool,
    pub url: String,
    pub organization: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct AzureDevOpsConfig {
    url: String,
    token: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AzureDevOpsLabel {
    pub name: String,
    pub color: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AzureDevOpsAssignee {
    pub login: String,
    pub avatar_url: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AzureDevOpsWorkItem {
    pub kind: String,
    pub number: i64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub updated_at: String,
    pub labels: Vec<AzureDevOpsLabel>,
    pub assignees: Vec<AzureDevOpsAssignee>,
    pub draft: bool,
    pub repo: String,
    pub attention_reason: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AzureDevOpsWorkItemDetails {
    pub body: String,
    pub author: String,
    pub author_avatar_url: String,
    pub base_ref_name: String,
    pub head_ref_name: String,
    pub review_decision: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AzureDevOpsWorkItemComment {
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
    pub replies: Vec<AzureDevOpsWorkItemComment>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AzureDevOpsWorkItemThread {
    pub comments: Vec<AzureDevOpsWorkItemComment>,
    pub truncated: bool,
    pub review_decision: String,
    pub base_ref_name: String,
    pub head_ref_name: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AzureDevOpsMrFile {
    pub path: String,
    pub additions: i64,
    pub deletions: i64,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AzureDevOpsMrDiff {
    pub additions: i64,
    pub deletions: i64,
    pub files: Vec<AzureDevOpsMrFile>,
    pub patch: String,
    pub truncated: bool,
}

pub fn azure_devops_status(app: PathBuf) -> Result<AzureDevOpsStatus, String> {
    let config = read_config(&app)?;
    Ok(AzureDevOpsStatus {
        connected: config.is_some(),
        url: config
            .as_ref()
            .map(|config| config.url.clone())
            .unwrap_or_default(),
        organization: config
            .as_ref()
            .map(|config| organization_from_url(&config.url))
            .unwrap_or_default(),
    })
}

pub fn azure_devops_set_config(
    app: PathBuf,
    url: String,
    token: String,
) -> Result<AzureDevOpsStatus, String> {
    let url = normalize_azure_devops_url(&url)?;
    let token = token.trim().to_string();
    if token.is_empty() {
        delete_config(&app)?;
        let organization = organization_from_url(&url);
        return Ok(AzureDevOpsStatus {
            connected: false,
            url,
            organization,
        });
    }
    let config = AzureDevOpsConfig { url, token };
    // Validates the PAT and fails fast on 401/203 (non-visualstudio passthrough).
    let user_id = azure_current_user_id(&config)?;
    if user_id.trim().is_empty() {
        return Err("Azure DevOps did not return the current user".into());
    }
    write_config(&app, &config)?;
    Ok(AzureDevOpsStatus {
        connected: true,
        organization: organization_from_url(&config.url),
        url: config.url,
    })
}

pub fn azure_devops_repo(app: PathBuf, cwd: String) -> Result<String, String> {
    let config = require_config(&app)?;
    azure_devops_repo_for(&expand_home(&cwd), &config.url)
}

pub fn azure_devops_list_work_items(
    app: PathBuf,
    cwd: String,
    kind: String,
    assigned_to_me: bool,
    state: String,
    limit: Option<u32>,
) -> Result<Vec<AzureDevOpsWorkItem>, String> {
    let config = require_config(&app)?;
    let repo = azure_devops_repo_for(&expand_home(&cwd), &config.url)?;
    azure_devops_list_work_items_for(
        &config,
        &repo,
        &kind,
        assigned_to_me,
        &state,
        limit.unwrap_or(DEFAULT_LIMIT),
    )
}

pub fn azure_devops_list_todos(
    app: PathBuf,
    kind: String,
    limit: Option<u32>,
) -> Result<Vec<AzureDevOpsWorkItem>, String> {
    let config = require_config(&app)?;
    azure_devops_list_todos_for(&config, &kind, limit.unwrap_or(DEFAULT_LIMIT))
}

pub fn azure_devops_work_item_details(
    app: PathBuf,
    repo: String,
    kind: String,
    number: i64,
) -> Result<AzureDevOpsWorkItemDetails, String> {
    let config = require_config(&app)?;
    // Boards work items carry only the project name in `repo`, so per-kind
    // validation happens inside the helper instead of here.
    azure_devops_work_item_details_for(&config, &repo, &kind, number)
}

pub fn azure_devops_work_item_thread(
    app: PathBuf,
    repo: String,
    kind: String,
    number: i64,
) -> Result<AzureDevOpsWorkItemThread, String> {
    let config = require_config(&app)?;
    azure_devops_work_item_thread_for(&config, &repo, &kind, number)
}

pub fn azure_devops_work_item_comment(
    app: PathBuf,
    repo: String,
    kind: String,
    number: i64,
    body: String,
) -> Result<String, String> {
    let config = require_config(&app)?;
    azure_devops_work_item_comment_for(&config, &repo, &kind, number, &body)
}

pub fn azure_devops_mr_diff(
    app: PathBuf,
    repo: String,
    number: i64,
) -> Result<AzureDevOpsMrDiff, String> {
    let config = require_config(&app)?;
    let repo = validate_repo(&repo)?;
    azure_devops_mr_diff_for(&config, &repo, number)
}

fn azure_devops_list_work_items_for(
    config: &AzureDevOpsConfig,
    repo: &str,
    kind: &str,
    assigned_to_me: bool,
    state: &str,
    limit: u32,
) -> Result<Vec<AzureDevOpsWorkItem>, String> {
    validate_kind(kind)?;
    let (project, repo_name) = split_repo(repo)?;
    let limit = limit.clamp(1, 100);
    if kind == "pr" {
        let me = if assigned_to_me {
            Some(azure_current_user_id(config)?)
        } else {
            None
        };
        return azure_list_prs_for(config, &project, &repo_name, me.as_deref(), state, limit);
    }
    azure_list_wit_for(config, Some(&project), assigned_to_me, state, limit)
}

fn azure_devops_list_todos_for(
    config: &AzureDevOpsConfig,
    kind: &str,
    limit: u32,
) -> Result<Vec<AzureDevOpsWorkItem>, String> {
    validate_kind(kind)?;
    let limit = limit.clamp(1, 100);
    if kind == "pr" {
        return azure_list_pr_reviews_for(config, limit);
    }
    let mut items = azure_list_wit_for(config, None, true, "open", limit)?;
    for item in &mut items {
        if item.attention_reason.is_empty() {
            item.attention_reason = "assigned".into();
        }
    }
    Ok(items)
}

fn azure_devops_work_item_details_for(
    config: &AzureDevOpsConfig,
    repo: &str,
    kind: &str,
    number: i64,
) -> Result<AzureDevOpsWorkItemDetails, String> {
    validate_item(kind, number)?;
    if kind == "pr" {
        let (project, repo_name) = split_repo(repo)?;
        let path = format!(
            "/{}/_apis/git/repositories/{}/pullrequests/{}?api-version={}",
            encode_segment(&project),
            encode_segment(&repo_name),
            number,
            API_VERSION
        );
        let response = azure_get(config, &path)?;
        return parse_pr_details(&response.value);
    }
    let path = format!(
        "/_apis/wit/workitems/{}?$expand=relations&api-version={}",
        number, API_VERSION
    );
    let response = azure_get(config, &path)?;
    parse_wit_details(&response.value)
}

fn azure_devops_work_item_thread_for(
    config: &AzureDevOpsConfig,
    repo: &str,
    kind: &str,
    number: i64,
) -> Result<AzureDevOpsWorkItemThread, String> {
    validate_item(kind, number)?;
    if kind == "pr" {
        let (project, repo_name) = split_repo(repo)?;
        let path = format!(
            "/{}/_apis/git/repositories/{}/pullrequests/{}/threads?api-version={}",
            encode_segment(&project),
            encode_segment(&repo_name),
            number,
            API_VERSION
        );
        let response = azure_get(config, &path)?;
        let pr_url = pr_web_url(config, &project, &repo_name, number);
        return parse_pr_thread(&response.value, &pr_url, response.truncated);
    }
    let project = wit_project(repo)?;
    let path = format!(
        "/{}/_apis/wit/workitems/{}/comments?api-version={}",
        encode_segment(&project),
        number,
        WIT_COMMENTS_API_VERSION
    );
    let response = azure_get(config, &path)?;
    parse_wit_thread(&response.value, response.truncated)
}

fn azure_devops_work_item_comment_for(
    config: &AzureDevOpsConfig,
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
    if kind == "pr" {
        let (project, repo_name) = split_repo(repo)?;
        let path = format!(
            "/{}/_apis/git/repositories/{}/pullrequests/{}/threads?api-version={}",
            encode_segment(&project),
            encode_segment(&repo_name),
            number,
            API_VERSION
        );
        let payload = json!({
            "comments": [{ "parentCommentId": 0, "content": body, "commentType": 1 }],
            "status": 1,
        });
        azure_post_json(config, &path, payload)?;
        return Ok(pr_web_url(config, &project, &repo_name, number));
    }
    let project = wit_project(repo)?;
    let path = format!(
        "/{}/_apis/wit/workitems/{}/comments?api-version={}",
        encode_segment(&project),
        number,
        WIT_COMMENTS_API_VERSION
    );
    let response = azure_post_json(config, &path, json!({ "text": body }))?;
    Ok(string_field(&response.value, "url").unwrap_or_default())
}

fn azure_devops_mr_diff_for(
    config: &AzureDevOpsConfig,
    repo: &str,
    number: i64,
) -> Result<AzureDevOpsMrDiff, String> {
    validate_item("pr", number)?;
    let (project, repo_name) = split_repo(repo)?;
    let iterations_path = format!(
        "/{}/_apis/git/repositories/{}/pullrequests/{}/iterations?api-version={}",
        encode_segment(&project),
        encode_segment(&repo_name),
        number,
        API_VERSION
    );
    let iterations = azure_get(config, &iterations_path)?;
    let Some((iteration_id, source_sha, base_sha)) = latest_iteration_commits(&iterations.value)
    else {
        return Ok(AzureDevOpsMrDiff {
            additions: 0,
            deletions: 0,
            files: Vec::new(),
            patch: String::new(),
            truncated: false,
        });
    };
    let changes_path = format!(
        "/{}/_apis/git/repositories/{}/pullrequests/{}/iterations/{}/changes?$top=100&api-version={}",
        encode_segment(&project),
        encode_segment(&repo_name),
        number,
        iteration_id,
        API_VERSION
    );
    let response = azure_get(config, &changes_path)?;
    let changes = parse_iteration_change_list(&response.value);
    let mut files: Vec<AzureDevOpsMrFile> = changes
        .iter()
        .map(|change| AzureDevOpsMrFile {
            path: change.path.clone(),
            additions: 0,
            deletions: 0,
        })
        .collect();
    let mut patch = String::new();
    let mut additions = 0_i64;
    let mut deletions = 0_i64;
    let mut truncated = response.truncated;
    let mut hunked = 0_usize;
    for (index, change) in changes.iter().enumerate() {
        if hunked >= MAX_DIFF_HUNK_FILES {
            truncated = true;
            break;
        }
        match diff_iteration_file(config, &project, &repo_name, change, &source_sha, &base_sha) {
            Ok(FileDiff::Ready {
                additions: file_adds,
                deletions: file_dels,
                block,
            }) => {
                hunked += 1;
                additions += file_adds;
                deletions += file_dels;
                if let Some(file) = files.get_mut(index) {
                    file.additions = file_adds;
                    file.deletions = file_dels;
                }
                if patch.len() + block.len() > MAX_DIFF_BYTES {
                    truncated = true;
                    continue;
                }
                patch.push_str(&block);
            }
            Ok(FileDiff::Unchanged) => {
                hunked += 1;
            }
            Ok(FileDiff::Skipped { too_large }) => {
                truncated = truncated || too_large;
            }
            Err(_) => {
                // A single unreadable file must not fail the whole diff; the
                // file stays listed without hunks.
            }
        }
    }
    Ok(AzureDevOpsMrDiff {
        additions,
        deletions,
        files,
        patch,
        truncated,
    })
}

// ---- Pull requests ----

fn azure_list_prs_for(
    config: &AzureDevOpsConfig,
    project: &str,
    repo_name: &str,
    assigned_user_id: Option<&str>,
    state: &str,
    limit: u32,
) -> Result<Vec<AzureDevOpsWorkItem>, String> {
    let repo = format!("{project}/{repo_name}");
    let reviewer = assigned_user_id.map(str::trim).filter(|id| !id.is_empty());
    if state.trim().eq_ignore_ascii_case("all") {
        let mut active = azure_fetch_prs(
            config,
            project,
            repo_name,
            &repo,
            "active",
            limit * 2,
            reviewer,
        )?;
        let mut completed = azure_fetch_prs(
            config,
            project,
            repo_name,
            &repo,
            "completed",
            limit * 2,
            reviewer,
        )?;
        // Abandoned PRs surface through the "all" view too, like closed GitHub PRs.
        let mut abandoned = azure_fetch_prs(
            config,
            project,
            repo_name,
            &repo,
            "abandoned",
            limit * 2,
            reviewer,
        )?;
        let mut items = Vec::new();
        items.append(&mut active);
        items.append(&mut completed);
        items.append(&mut abandoned);
        items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        items.truncate(limit as usize);
        return Ok(items);
    }
    azure_fetch_prs(config, project, repo_name, &repo, "active", limit, reviewer)
}

fn azure_fetch_prs(
    config: &AzureDevOpsConfig,
    project: &str,
    repo_name: &str,
    repo: &str,
    status: &str,
    limit: u32,
    reviewer_id: Option<&str>,
) -> Result<Vec<AzureDevOpsWorkItem>, String> {
    let reviewer = reviewer_id
        .map(|id| format!("&searchCriteria.reviewerId={}", encode_segment(id)))
        .unwrap_or_default();
    let path = format!(
        "/{}/_apis/git/repositories/{}/pullrequests?searchCriteria.status={}{}&$top={}&api-version={}",
        encode_segment(project),
        encode_segment(repo_name),
        status,
        reviewer,
        limit.clamp(1, 100),
        API_VERSION
    );
    let response = azure_get(config, &path)?;
    parse_pr_list(&response.value, repo, &config.url)
}

fn azure_list_pr_reviews_for(
    config: &AzureDevOpsConfig,
    limit: u32,
) -> Result<Vec<AzureDevOpsWorkItem>, String> {
    let user_id = azure_current_user_id(config)?;
    let path = format!(
        "/_apis/git/pullrequests?searchCriteria.status=active&searchCriteria.reviewerId={}&$top={}&api-version={}",
        encode_segment(&user_id),
        limit.clamp(1, 100),
        API_VERSION
    );
    let response = azure_get(config, &path)?;
    let rows = response
        .value
        .get("value")
        .and_then(Value::as_array)
        .ok_or_else(|| "Azure DevOps did not return pull requests".to_string())?;
    let mut items = Vec::new();
    for row in rows {
        let (project, repo_name) = pr_repo_parts(row);
        let repo = match (project, repo_name) {
            (Some(project), Some(repo_name)) => format!("{project}/{repo_name}"),
            _ => continue,
        };
        if let Some(mut item) = parse_pr(row, &repo, &config.url) {
            if item.attention_reason.is_empty() {
                item.attention_reason = "review_requested".into();
            }
            items.push(item);
        }
    }
    Ok(items)
}

// ---- Work items (Boards) ----

fn azure_list_wit_for(
    config: &AzureDevOpsConfig,
    project: Option<&str>,
    assigned_to_me: bool,
    state: &str,
    limit: u32,
) -> Result<Vec<AzureDevOpsWorkItem>, String> {
    let limit = limit.clamp(1, 100);
    let wiql = wit_wiql(project, assigned_to_me, state);
    let ids = azure_query_wit_ids(config, &wiql, limit)?;
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    azure_fetch_wit_batch(config, &ids)
}

fn wit_wiql(project: Option<&str>, assigned_to_me: bool, state: &str) -> String {
    let mut query = String::from("SELECT [System.Id] FROM WorkItems WHERE ");
    let mut clauses: Vec<String> = Vec::new();
    if let Some(project) = project.map(str::trim).filter(|p| !p.is_empty()) {
        clauses.push(format!(
            "[System.TeamProject] = '{}'",
            project.replace('\'', "''")
        ));
    }
    if assigned_to_me {
        clauses.push("[System.AssignedTo] = @me".into());
    }
    if !state.trim().eq_ignore_ascii_case("all") {
        clauses.push("[System.State] NOT IN ('Closed', 'Done', 'Removed')".into());
    }
    if clauses.is_empty() {
        clauses.push("[System.Id] > 0".into());
    }
    query.push_str(&clauses.join(" AND "));
    query.push_str(" ORDER BY [System.ChangedDate] DESC");
    query
}

fn azure_query_wit_ids(
    config: &AzureDevOpsConfig,
    wiql: &str,
    limit: u32,
) -> Result<Vec<i64>, String> {
    let path = format!("/_apis/wit/wiql?$top={limit}&api-version={API_VERSION}");
    let response = azure_post_json(config, &path, json!({ "query": wiql }))?;
    let relations = response
        .value
        .get("workItems")
        .and_then(Value::as_array)
        .ok_or_else(|| "Azure DevOps did not return work items".to_string())?;
    Ok(relations
        .iter()
        .filter_map(|row| row.get("id").and_then(Value::as_i64))
        .filter(|id| *id > 0)
        .take(limit as usize)
        .collect())
}

fn azure_fetch_wit_batch(
    config: &AzureDevOpsConfig,
    ids: &[i64],
) -> Result<Vec<AzureDevOpsWorkItem>, String> {
    let joined = ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let fields = [
        "System.Id",
        "System.Title",
        "System.State",
        "System.ChangedDate",
        "System.CreatedDate",
        "System.AssignedTo",
        "System.Tags",
        "System.TeamProject",
    ]
    .join(",");
    let path = format!(
        "/_apis/wit/workitems?ids={joined}&fields={fields}&$expand=relations&api-version={API_VERSION}"
    );
    let response = azure_get(config, &path)?;
    let rows = response
        .value
        .get("value")
        .and_then(Value::as_array)
        .ok_or_else(|| "Azure DevOps did not return work items".to_string())?;
    let mut items: Vec<AzureDevOpsWorkItem> = rows.iter().filter_map(parse_wit).collect();
    items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(items)
}

// ---- Parsing ----

// ---- Validation ----

// ---- HTTP ----

// ---- URLs / remotes ----

// ---- Config storage ----
