use super::*;

pub(super) fn parse_pr_list(
    value: &Value,
    repo: &str,
    base_url: &str,
) -> Result<Vec<AzureDevOpsWorkItem>, String> {
    let rows = value
        .get("value")
        .and_then(Value::as_array)
        .ok_or_else(|| "Azure DevOps did not return pull requests".to_string())?;
    Ok(rows
        .iter()
        .filter_map(|row| parse_pr(row, repo, base_url))
        .collect())
}

pub(super) fn parse_pr(row: &Value, repo: &str, base_url: &str) -> Option<AzureDevOpsWorkItem> {
    let number = row.get("pullRequestId").and_then(Value::as_i64)?;
    if number <= 0 {
        return None;
    }
    let status = string_field(row, "status").unwrap_or_default();
    Some(AzureDevOpsWorkItem {
        kind: "pr".into(),
        number,
        title: string_field(row, "title").unwrap_or_default(),
        // List responses often omit `_links.web` and even `remoteUrl`, so the
        // last resort builds the standard web URL from request-side data.
        url: pr_link(row)
            .or_else(|| pr_url_from_repository(row, number))
            .or_else(|| pr_url_from_parts(repo, base_url, number))
            .unwrap_or_default(),
        state: normalize_pr_state(&status),
        updated_at: string_field(row, "closedDate")
            .or_else(|| string_field(row, "creationDate"))
            .unwrap_or_default(),
        labels: parse_pr_labels(row),
        assignees: parse_pr_assignees(row),
        draft: row.get("isDraft").and_then(Value::as_bool).unwrap_or(false),
        repo: repo.into(),
        attention_reason: String::new(),
    })
}

pub(super) fn parse_pr_details(value: &Value) -> Result<AzureDevOpsWorkItemDetails, String> {
    if !value.is_object() {
        return Err("Azure DevOps did not return that pull request".into());
    }
    let creator = value.get("createdBy");
    Ok(AzureDevOpsWorkItemDetails {
        body: string_field(value, "description").unwrap_or_default(),
        author: creator
            .and_then(|user| string_field(user, "displayName"))
            .unwrap_or_default(),
        author_avatar_url: creator
            .and_then(|user| string_field(user, "imageUrl"))
            .or_else(|| {
                creator.and_then(|user| {
                    user.get("_links")
                        .and_then(|links| links.get("avatar"))
                        .and_then(|avatar| string_field(avatar, "href"))
                })
            })
            .unwrap_or_default(),
        base_ref_name: short_ref(&string_field(value, "targetRefName").unwrap_or_default()),
        head_ref_name: short_ref(&string_field(value, "sourceRefName").unwrap_or_default()),
        review_decision: String::new(),
    })
}

pub(super) fn parse_pr_thread(
    value: &Value,
    pr_url: &str,
    truncated: bool,
) -> Result<AzureDevOpsWorkItemThread, String> {
    let rows = value
        .get("value")
        .and_then(Value::as_array)
        .ok_or_else(|| "Azure DevOps did not return comments".to_string())?;
    let mut comments: Vec<AzureDevOpsWorkItemComment> = Vec::new();
    for thread in rows {
        let thread_id = thread
            .get("id")
            .and_then(Value::as_i64)
            .map(|id| id.to_string())
            .unwrap_or_default();
        let status = string_field(thread, "status").unwrap_or_default();
        let resolved = matches!(
            status.to_ascii_lowercase().as_str(),
            "fixed" | "closed" | "resolved"
        );
        let Some(thread_comments) = thread.get("comments").and_then(Value::as_array) else {
            continue;
        };
        for comment in thread_comments {
            if comment
                .get("isDeleted")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                continue;
            }
            let body = string_field(comment, "content").unwrap_or_default();
            if body.is_empty() {
                continue;
            }
            let author = comment.get("author");
            let comment_id = comment
                .get("id")
                .and_then(Value::as_i64)
                .map(|id| id.to_string())
                .unwrap_or_default();
            comments.push(AzureDevOpsWorkItemComment {
                id: if thread_id.is_empty() {
                    comment_id.clone()
                } else {
                    format!("{thread_id}-{comment_id}")
                },
                kind: "comment".into(),
                author: author
                    .and_then(|user| string_field(user, "displayName"))
                    .unwrap_or_default(),
                author_avatar_url: author
                    .and_then(|user| string_field(user, "imageUrl"))
                    .unwrap_or_default(),
                body,
                created_at: string_field(comment, "publishedDate")
                    .or_else(|| string_field(comment, "lastUpdatedDate"))
                    .unwrap_or_default(),
                url: pr_url.into(),
                state: String::new(),
                path: String::new(),
                line: None,
                resolved,
                thread_id: thread_id.clone(),
                replies: Vec::new(),
            });
        }
    }
    comments.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(AzureDevOpsWorkItemThread {
        comments,
        truncated,
        review_decision: String::new(),
        base_ref_name: String::new(),
        head_ref_name: String::new(),
    })
}

pub(super) fn parse_wit(row: &Value) -> Option<AzureDevOpsWorkItem> {
    let number = row.get("id").and_then(Value::as_i64)?;
    if number <= 0 {
        return None;
    }
    let fields = row.get("fields")?;
    let project = string_field(fields, "System.TeamProject").unwrap_or_default();
    Some(AzureDevOpsWorkItem {
        kind: "issue".into(),
        number,
        title: string_field(fields, "System.Title").unwrap_or_default(),
        url: row
            .get("_links")
            .and_then(|links| links.get("html"))
            .and_then(|html| string_field(html, "href"))
            .unwrap_or_default(),
        state: normalize_wit_state(&string_field(fields, "System.State").unwrap_or_default()),
        updated_at: string_field(fields, "System.ChangedDate")
            .or_else(|| string_field(fields, "System.CreatedDate"))
            .unwrap_or_default(),
        labels: parse_wit_labels(fields),
        assignees: parse_wit_assignees(fields),
        draft: false,
        repo: project,
        attention_reason: String::new(),
    })
}

pub(super) fn parse_wit_details(value: &Value) -> Result<AzureDevOpsWorkItemDetails, String> {
    if !value.is_object() {
        return Err("Azure DevOps did not return that work item".into());
    }
    let fields = value
        .get("fields")
        .ok_or_else(|| "Azure DevOps did not return that work item".to_string())?;
    let creator = fields.get("System.CreatedBy");
    Ok(AzureDevOpsWorkItemDetails {
        body: string_field(fields, "System.Description").unwrap_or_default(),
        author: creator
            .and_then(|user| string_field(user, "displayName"))
            .unwrap_or_default(),
        author_avatar_url: creator
            .and_then(|user| string_field(user, "imageUrl"))
            .unwrap_or_default(),
        base_ref_name: String::new(),
        head_ref_name: String::new(),
        review_decision: String::new(),
    })
}

pub(super) fn parse_wit_thread(
    value: &Value,
    truncated: bool,
) -> Result<AzureDevOpsWorkItemThread, String> {
    let rows = value
        .get("comments")
        .and_then(Value::as_array)
        .ok_or_else(|| "Azure DevOps did not return comments".to_string())?;
    let mut comments: Vec<AzureDevOpsWorkItemComment> = rows
        .iter()
        .filter_map(|row| {
            let id = row
                .get("commentId")
                .or_else(|| row.get("id"))
                .and_then(Value::as_i64)?;
            let body = string_field(row, "text").unwrap_or_default();
            if body.is_empty() {
                return None;
            }
            let author = row.get("createdBy");
            Some(AzureDevOpsWorkItemComment {
                id: id.to_string(),
                kind: "comment".into(),
                author: author
                    .and_then(|user| string_field(user, "displayName"))
                    .unwrap_or_default(),
                author_avatar_url: author
                    .and_then(|user| string_field(user, "imageUrl"))
                    .unwrap_or_default(),
                body,
                created_at: string_field(row, "createdDate").unwrap_or_default(),
                url: string_field(row, "url").unwrap_or_default(),
                state: String::new(),
                path: String::new(),
                line: None,
                resolved: false,
                thread_id: String::new(),
                replies: Vec::new(),
            })
        })
        .collect();
    comments.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    let total = value
        .get("totalCount")
        .and_then(Value::as_i64)
        .unwrap_or(comments.len() as i64);
    Ok(AzureDevOpsWorkItemThread {
        truncated: truncated || total > comments.len() as i64,
        comments,
        review_decision: String::new(),
        base_ref_name: String::new(),
        head_ref_name: String::new(),
    })
}

pub(super) struct IterationChange {
    pub(super) path: String,
    pub(super) change_type: String,
}

pub(super) fn parse_iteration_change_list(value: &Value) -> Vec<IterationChange> {
    let entries = value
        .get("changeEntries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut changes = Vec::new();
    for entry in &entries {
        let item = entry.get("item");
        let path = item
            .and_then(|item| string_field(item, "path"))
            .unwrap_or_default();
        let is_folder = item
            .and_then(|item| item.get("isFolder"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if path.is_empty() || is_folder || path == "/" {
            continue;
        }
        let clean = path.trim_start_matches('/').to_string();
        if clean.is_empty() {
            continue;
        }
        changes.push(IterationChange {
            path: clean,
            change_type: string_field(entry, "changeType").unwrap_or_default(),
        });
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    changes.dedup_by(|a, b| a.path == b.path);
    changes
}

pub(super) fn parse_pr_labels(row: &Value) -> Vec<AzureDevOpsLabel> {
    row.get("labels")
        .and_then(Value::as_array)
        .map(|labels| {
            labels
                .iter()
                .filter_map(|label| {
                    let name = string_field(label, "name")?;
                    if name.is_empty() {
                        return None;
                    }
                    Some(AzureDevOpsLabel {
                        name,
                        color: String::new(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn parse_pr_assignees(row: &Value) -> Vec<AzureDevOpsAssignee> {
    let mut people: Vec<AzureDevOpsAssignee> = row
        .get("reviewers")
        .and_then(Value::as_array)
        .map(|reviewers| {
            reviewers
                .iter()
                .filter_map(|reviewer| {
                    let login = string_field(reviewer, "displayName")?;
                    if login.is_empty() {
                        return None;
                    }
                    Some(AzureDevOpsAssignee {
                        login,
                        avatar_url: string_field(reviewer, "imageUrl").unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if let Some(creator) = row.get("createdBy") {
        if let Some(login) = string_field(creator, "displayName").filter(|name| !name.is_empty()) {
            if !people.iter().any(|person| person.login == login) {
                people.push(AzureDevOpsAssignee {
                    login,
                    avatar_url: string_field(creator, "imageUrl").unwrap_or_default(),
                });
            }
        }
    }
    people
}

pub(super) fn parse_wit_labels(fields: &Value) -> Vec<AzureDevOpsLabel> {
    string_field(fields, "System.Tags")
        .map(|tags| {
            tags.split(';')
                .map(str::trim)
                .filter(|tag| !tag.is_empty())
                .map(|tag| AzureDevOpsLabel {
                    name: tag.to_string(),
                    color: String::new(),
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn parse_wit_assignees(fields: &Value) -> Vec<AzureDevOpsAssignee> {
    let assigned = fields.get("System.AssignedTo");
    assigned
        .and_then(|user| {
            let login = string_field(user, "displayName")?;
            if login.is_empty() {
                return None;
            }
            Some(vec![AzureDevOpsAssignee {
                login,
                avatar_url: string_field(user, "imageUrl").unwrap_or_default(),
            }])
        })
        .unwrap_or_default()
}

pub(super) fn normalize_pr_state(state: &str) -> String {
    match state.trim().to_ascii_lowercase().as_str() {
        "active" => "open".into(),
        "completed" => "merged".into(),
        "abandoned" => "closed".into(),
        other => other.into(),
    }
}

pub(super) fn normalize_wit_state(state: &str) -> String {
    match state.trim().to_ascii_lowercase().as_str() {
        "new" | "active" | "to do" | "doing" | "in progress" | "resolved" | "proposed"
        | "approved" | "committed" => "open".into(),
        "closed" | "done" | "removed" | "abandoned" | "cut" => "closed".into(),
        other => other.into(),
    }
}

pub(super) fn latest_iteration_commits(value: &Value) -> Option<(i64, String, String)> {
    let rows = value.get("value").and_then(Value::as_array)?;
    let row = rows
        .iter()
        .filter(|row| row.get("id").and_then(Value::as_i64).unwrap_or(0) > 0)
        .max_by_key(|row| row.get("id").and_then(Value::as_i64).unwrap_or(0))?;
    let id = row.get("id").and_then(Value::as_i64)?;
    let commit_id = |key: &str| {
        row.get(key)
            .and_then(|commit| string_field(commit, "commitId"))
            .unwrap_or_default()
    };
    let source = commit_id("sourceRefCommit");
    if source.trim().is_empty() {
        return None;
    }
    let base = commit_id("commonRefCommit");
    let base = if base.trim().is_empty() {
        commit_id("targetRefCommit")
    } else {
        base
    };
    if base.trim().is_empty() {
        return None;
    }
    Some((id, source, base))
}

pub(super) fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(|value| value.trim().to_string())
}
