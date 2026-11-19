use super::*;

pub(super) fn parse_work_items(
    value: &Value,
    kind: &str,
    repo: &str,
) -> Result<Vec<GitlabWorkItem>, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| "GitLab did not return work items".to_string())?;
    Ok(rows
        .iter()
        .filter_map(|row| parse_work_item(row, kind, repo))
        .collect())
}

pub(super) fn parse_todos(value: &Value, kind: &str) -> Result<Vec<GitlabWorkItem>, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| "GitLab did not return to-do items".to_string())?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            let repo = row
                .get("project")
                .and_then(|project| string_field(project, "path_with_namespace"))?;
            if !valid_project_path(&repo) {
                return None;
            }
            let target = row.get("target")?;
            let mut item = parse_work_item(target, kind, &repo)?;
            if item.url.is_empty() {
                item.url = string_field(row, "target_url").unwrap_or_default();
            }
            if item.updated_at.is_empty() {
                item.updated_at = string_field(row, "created_at").unwrap_or_default();
            }
            item.attention_reason = string_field(row, "action_name").unwrap_or_default();
            Some(item)
        })
        .collect())
}

pub(super) fn parse_work_item(row: &Value, kind: &str, repo: &str) -> Option<GitlabWorkItem> {
    let number = row.get("iid").and_then(Value::as_i64)?;
    if number <= 0 {
        return None;
    }
    let title = string_field(row, "title").unwrap_or_default();
    let title_lower = title.to_ascii_lowercase();
    let draft = kind == "pr"
        && (row.get("draft").and_then(Value::as_bool).unwrap_or(false)
            || row
                .get("work_in_progress")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            || title_lower.starts_with("draft:")
            || title_lower.starts_with("wip:"));
    Some(GitlabWorkItem {
        kind: kind.into(),
        number,
        title,
        url: string_field(row, "web_url").unwrap_or_default(),
        state: normalize_state(&string_field(row, "state").unwrap_or_default()),
        updated_at: string_field(row, "updated_at").unwrap_or_default(),
        labels: parse_labels(row),
        assignees: parse_assignees(row),
        draft,
        repo: repo.into(),
        attention_reason: String::new(),
    })
}

pub(super) fn parse_work_item_details(
    value: &Value,
    kind: &str,
) -> Result<GitlabWorkItemDetails, String> {
    if !value.is_object() {
        return Err("GitLab did not return that item".into());
    }
    let author = value.get("author");
    Ok(GitlabWorkItemDetails {
        body: string_field(value, "description").unwrap_or_default(),
        author: author
            .and_then(|author| string_field(author, "username"))
            .or_else(|| author.and_then(|author| string_field(author, "name")))
            .unwrap_or_default(),
        author_avatar_url: author
            .and_then(|author| string_field(author, "avatar_url"))
            .unwrap_or_default(),
        base_ref_name: if kind == "pr" {
            string_field(value, "target_branch").unwrap_or_default()
        } else {
            String::new()
        },
        head_ref_name: if kind == "pr" {
            string_field(value, "source_branch").unwrap_or_default()
        } else {
            String::new()
        },
        review_decision: String::new(),
    })
}

pub(super) fn parse_work_item_thread(
    value: &Value,
    base_url: &str,
    repo: &str,
    kind: &str,
    number: i64,
    has_next_page: bool,
) -> Result<GitlabWorkItemThread, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| "GitLab did not return comments".to_string())?;
    let mut comments: Vec<GitlabWorkItemComment> = rows
        .iter()
        .filter(|row| !row.get("system").and_then(Value::as_bool).unwrap_or(false))
        .filter_map(|row| {
            let id = row.get("id").and_then(Value::as_i64)?;
            let author = row.get("author");
            Some(GitlabWorkItemComment {
                id: id.to_string(),
                kind: "comment".into(),
                author: author
                    .and_then(|author| string_field(author, "username"))
                    .or_else(|| author.and_then(|author| string_field(author, "name")))
                    .unwrap_or_default(),
                author_avatar_url: author
                    .and_then(|author| string_field(author, "avatar_url"))
                    .unwrap_or_default(),
                body: string_field(row, "body").unwrap_or_default(),
                created_at: string_field(row, "created_at").unwrap_or_default(),
                url: note_url(base_url, repo, kind, number, id),
                state: String::new(),
                path: String::new(),
                line: None,
                resolved: row
                    .get("resolved")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                thread_id: String::new(),
                replies: Vec::new(),
            })
        })
        .collect();
    comments.reverse();
    Ok(GitlabWorkItemThread {
        comments,
        truncated: has_next_page,
        review_decision: String::new(),
        base_ref_name: String::new(),
        head_ref_name: String::new(),
    })
}

pub(super) fn parse_mr_diff(value: &Value, has_next_page: bool) -> Result<GitlabMrDiff, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| "GitLab did not return merge request diffs".to_string())?;
    let mut files = Vec::new();
    let mut patch = String::new();
    let mut total_additions = 0;
    let mut total_deletions = 0;
    let mut truncated = has_next_page;

    for row in rows {
        let old_path = string_field(row, "old_path").unwrap_or_default();
        let new_path = string_field(row, "new_path").unwrap_or_else(|| old_path.clone());
        if new_path.is_empty() && old_path.is_empty() {
            continue;
        }
        let diff = string_field_preserve(row, "diff").unwrap_or_default();
        let (additions, deletions) = diff_counts(&diff);
        total_additions += additions;
        total_deletions += deletions;
        files.push(GitlabMrFile {
            path: if new_path.is_empty() {
                old_path.clone()
            } else {
                new_path.clone()
            },
            additions,
            deletions,
        });
        truncated |= row
            .get("too_large")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || row
                .get("collapsed")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        if diff.is_empty() || patch.len() >= MAX_DIFF_BYTES {
            continue;
        }
        let block = gitlab_diff_block(row, &old_path, &new_path, &diff);
        if patch.len() + block.len() > MAX_DIFF_BYTES {
            truncated = true;
            continue;
        }
        patch.push_str(&block);
    }

    Ok(GitlabMrDiff {
        additions: total_additions,
        deletions: total_deletions,
        files,
        patch,
        truncated,
    })
}

pub(super) fn gitlab_diff_block(row: &Value, old_path: &str, new_path: &str, diff: &str) -> String {
    let old = if old_path.is_empty() {
        new_path
    } else {
        old_path
    };
    let new = if new_path.is_empty() {
        old_path
    } else {
        new_path
    };
    let mut block = format!("diff --git a/{old} b/{new}\n");
    if row
        .get("new_file")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        block.push_str("new file mode 100644\n");
    }
    if row
        .get("deleted_file")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        block.push_str("deleted file mode 100644\n");
    }
    if row
        .get("renamed_file")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        block.push_str(&format!("rename from {old}\nrename to {new}\n"));
    }
    let old_header = if row
        .get("new_file")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "/dev/null".to_string()
    } else {
        format!("a/{old}")
    };
    let new_header = if row
        .get("deleted_file")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "/dev/null".to_string()
    } else {
        format!("b/{new}")
    };
    block.push_str(&format!("--- {old_header}\n+++ {new_header}\n"));
    block.push_str(diff);
    if !diff.ends_with('\n') {
        block.push('\n');
    }
    block
}

pub(super) fn diff_counts(diff: &str) -> (i64, i64) {
    let mut additions = 0;
    let mut deletions = 0;
    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            additions += 1;
        } else if line.starts_with('-') {
            deletions += 1;
        }
    }
    (additions, deletions)
}

pub(super) fn parse_labels(row: &Value) -> Vec<GitlabLabel> {
    row.get("labels")
        .and_then(Value::as_array)
        .map(|labels| {
            labels
                .iter()
                .filter_map(|label| {
                    if let Some(name) = label.as_str() {
                        return Some(GitlabLabel {
                            name: name.trim().to_string(),
                            color: String::new(),
                        });
                    }
                    let name = string_field(label, "name")?;
                    Some(GitlabLabel {
                        name,
                        color: string_field(label, "color")
                            .unwrap_or_default()
                            .trim_start_matches('#')
                            .to_string(),
                    })
                })
                .filter(|label| !label.name.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn parse_assignees(row: &Value) -> Vec<GitlabAssignee> {
    let mut people: Vec<&Value> = row
        .get("assignees")
        .and_then(Value::as_array)
        .map(|people| people.iter().collect())
        .unwrap_or_default();
    if people.is_empty() {
        if let Some(assignee) = row.get("assignee").filter(|value| value.is_object()) {
            people.push(assignee);
        }
    }
    people
        .into_iter()
        .filter_map(|person| {
            let login =
                string_field(person, "username").or_else(|| string_field(person, "name"))?;
            if login.is_empty() {
                return None;
            }
            Some(GitlabAssignee {
                login,
                avatar_url: string_field(person, "avatar_url").unwrap_or_default(),
            })
        })
        .collect()
}

pub(super) fn normalize_state(state: &str) -> String {
    match state.trim().to_ascii_lowercase().as_str() {
        "opened" | "reopened" => "open".into(),
        other => other.into(),
    }
}

pub(super) fn note_url(base_url: &str, repo: &str, kind: &str, number: i64, id: i64) -> String {
    let item = if kind == "pr" {
        "merge_requests"
    } else {
        "issues"
    };
    format!(
        "{}/{repo}/-/{item}/{number}#note_{id}",
        base_url.trim_end_matches('/')
    )
}

pub(super) fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(|value| value.trim().to_string())
}

pub(super) fn string_field_preserve(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}
