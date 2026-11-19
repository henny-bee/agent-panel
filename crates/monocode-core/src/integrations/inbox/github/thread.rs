use super::*;

pub(super) const GITHUB_ISSUE_THREAD_QUERY: &str = r#"
query InboxIssueThread($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    issue(number: $number) {
      comments(last: 40) {
        totalCount
        nodes {
          id
          author { login }
          body
          createdAt
          url
          isMinimized
        }
      }
    }
  }
}
"#;

pub(super) const GITHUB_PR_THREAD_QUERY: &str = r#"
query InboxPullRequestThread($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      reviewDecision
      baseRefName
      headRefName
      commits(last: 40) {
        totalCount
        nodes {
          commit {
            oid
            messageHeadline
            committedDate
            url
            author {
              name
              user { login }
            }
          }
        }
      }
      comments(last: 40) {
        totalCount
        nodes {
          id
          author { login }
          body
          createdAt
          url
          isMinimized
        }
      }
      reviews(last: 40) {
        totalCount
        nodes {
          id
          author { login }
          body
          state
          submittedAt
          url
        }
      }
      reviewThreads(last: 20) {
        totalCount
        nodes {
          id
          isResolved
          path
          comments(first: 8) {
            totalCount
            nodes {
              id
              author { login }
              body
              createdAt
              url
              path
              line
              originalLine
              isMinimized
            }
          }
        }
      }
    }
  }
}
"#;

pub(super) const GITHUB_REVIEW_REPLY_MUTATION: &str = r#"
mutation InboxReviewReply($threadId: ID!, $body: String!) {
  addPullRequestReviewThreadReply(input: {
    pullRequestReviewThreadId: $threadId
    body: $body
  }) {
    comment { url }
  }
}
"#;

pub(super) fn github_comment_input<'a>(
    kind: &'a str,
    number: i64,
    body: &'a str,
) -> Result<(&'a str, &'a str), String> {
    let kind = kind.trim();
    if kind != "issue" && kind != "pr" {
        return Err("Unknown GitHub task kind".into());
    }
    if number <= 0 {
        return Err("Invalid GitHub item number".into());
    }
    let body = body.trim();
    if body.is_empty() {
        return Err("Comment cannot be empty".into());
    }
    Ok((kind, body))
}

pub(super) fn with_temp_markdown(
    body: &str,
    run: impl FnOnce(&str) -> Result<String, String>,
) -> Result<String, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("monocode-comment-{stamp}.md"));
    std::fs::write(&path, body).map_err(|error| error.to_string())?;
    let path_str = path.to_string_lossy().into_owned();
    let result = run(&path_str);
    let _ = std::fs::remove_file(&path);
    result
}

pub(super) fn valid_github_node_id(id: &str) -> bool {
    let id = id.trim();
    !id.is_empty()
        && id.len() < 256
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '='))
}

pub(super) fn github_url_from_output(output: &str, missing: &str) -> Result<String, String> {
    output
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| line.starts_with("http://") || line.starts_with("https://"))
        .map(str::to_string)
        .ok_or_else(|| {
            if output.trim().is_empty() {
                missing.to_string()
            } else {
                output.trim().to_string()
            }
        })
}

pub(super) fn parse_github_review_reply_url(json: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|error| error.to_string())?;
    if let Some(message) = value
        .get("errors")
        .and_then(|errors| errors.as_array())
        .and_then(|errors| errors.first())
        .and_then(|error| error.get("message"))
        .and_then(|message| message.as_str())
        .map(str::trim)
        .filter(|message| !message.is_empty())
    {
        return Err(message.to_string());
    }
    let url = value
        .pointer("/data/addPullRequestReviewThreadReply/comment/url")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim();
    if url.starts_with("http://") || url.starts_with("https://") {
        return Ok(url.to_string());
    }
    Err("GitHub did not return a comment URL".into())
}

#[derive(Deserialize)]
pub(super) struct GithubGraphqlEnvelope {
    #[serde(default)]
    pub(super) data: Option<GithubGraphqlData>,
    #[serde(default)]
    pub(super) errors: Vec<GithubGraphqlError>,
}

#[derive(Deserialize)]
pub(super) struct GithubGraphqlError {
    #[serde(default)]
    pub(super) message: String,
}

#[derive(Deserialize)]
pub(super) struct GithubGraphqlData {
    pub(super) repository: Option<GithubGraphqlRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GithubGraphqlRepository {
    pub(super) issue: Option<GithubGraphqlIssue>,
    pub(super) pull_request: Option<GithubGraphqlPullRequest>,
}

#[derive(Deserialize)]
pub(super) struct GithubGraphqlIssue {
    #[serde(default)]
    pub(super) comments: GithubGraphqlNodes<GithubGraphqlComment>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GithubGraphqlPullRequest {
    #[serde(default)]
    pub(super) review_decision: Option<String>,
    #[serde(default)]
    pub(super) base_ref_name: String,
    #[serde(default)]
    pub(super) head_ref_name: String,
    #[serde(default)]
    pub(super) commits: GithubGraphqlNodes<GithubGraphqlCommitNode>,
    #[serde(default)]
    pub(super) comments: GithubGraphqlNodes<GithubGraphqlComment>,
    #[serde(default)]
    pub(super) reviews: GithubGraphqlNodes<GithubGraphqlReview>,
    #[serde(default)]
    pub(super) review_threads: GithubGraphqlNodes<GithubGraphqlReviewThread>,
}

#[derive(Deserialize)]
pub(super) struct GithubGraphqlCommitNode {
    pub(super) commit: GithubGraphqlCommit,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GithubGraphqlCommit {
    #[serde(default)]
    pub(super) oid: String,
    #[serde(default)]
    pub(super) message_headline: String,
    #[serde(default)]
    pub(super) committed_date: String,
    #[serde(default)]
    pub(super) url: String,
    #[serde(default)]
    pub(super) author: Option<GithubGraphqlCommitAuthor>,
}

#[derive(Deserialize)]
pub(super) struct GithubGraphqlCommitAuthor {
    #[serde(default)]
    pub(super) name: String,
    #[serde(default)]
    pub(super) user: Option<GithubGraphqlActor>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GithubGraphqlNodes<T> {
    pub(super) total_count: i64,
    pub(super) nodes: Vec<T>,
}

impl<T> Default for GithubGraphqlNodes<T> {
    fn default() -> Self {
        Self {
            total_count: 0,
            nodes: Vec::new(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GithubGraphqlComment {
    #[serde(default)]
    pub(super) id: String,
    #[serde(default)]
    pub(super) author: Option<GithubGraphqlActor>,
    #[serde(default)]
    pub(super) body: String,
    #[serde(default)]
    pub(super) created_at: String,
    #[serde(default)]
    pub(super) url: String,
    #[serde(default)]
    pub(super) is_minimized: bool,
    #[serde(default)]
    pub(super) path: String,
    #[serde(default)]
    pub(super) line: Option<i64>,
    #[serde(default)]
    pub(super) original_line: Option<i64>,
}

#[derive(Deserialize)]
pub(super) struct GithubGraphqlReview {
    #[serde(default)]
    pub(super) id: String,
    #[serde(default)]
    pub(super) author: Option<GithubGraphqlActor>,
    #[serde(default)]
    pub(super) body: String,
    #[serde(default)]
    pub(super) state: String,
    #[serde(default, rename = "submittedAt")]
    pub(super) submitted_at: Option<String>,
    #[serde(default)]
    pub(super) url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GithubGraphqlReviewThread {
    #[serde(default)]
    pub(super) id: String,
    #[serde(default)]
    pub(super) is_resolved: bool,
    #[serde(default)]
    pub(super) path: String,
    #[serde(default)]
    pub(super) comments: GithubGraphqlNodes<GithubGraphqlComment>,
}

#[derive(Deserialize)]
pub(super) struct GithubGraphqlActor {
    #[serde(default)]
    pub(super) login: String,
}

pub(super) fn parse_github_work_item_thread(
    json: &str,
    kind: &str,
) -> Result<GitHubWorkItemThread, String> {
    let envelope: GithubGraphqlEnvelope =
        serde_json::from_str(json).map_err(|error| error.to_string())?;
    let graphql_error = envelope
        .errors
        .iter()
        .map(|error| error.message.trim())
        .find(|message| !message.is_empty())
        .map(str::to_string);
    let Some(repository) = envelope.data.and_then(|data| data.repository) else {
        return Err(graphql_error.unwrap_or_else(|| "GitHub item not found".into()));
    };

    let mut comments = Vec::new();
    let mut commits = Vec::new();
    let mut truncated = false;
    let mut review_decision = String::new();
    let mut base_ref_name = String::new();
    let mut head_ref_name = String::new();

    if kind == "pr" {
        let Some(pull) = repository.pull_request else {
            return Err(graphql_error.unwrap_or_else(|| "GitHub pull request not found".into()));
        };
        review_decision = pull.review_decision.unwrap_or_default();
        base_ref_name = pull.base_ref_name;
        head_ref_name = pull.head_ref_name;
        truncated |= github_nodes_truncated(&pull.commits);
        commits.extend(pull.commits.nodes.into_iter().filter_map(|node| {
            let commit = node.commit;
            if commit.oid.trim().is_empty() || commit.committed_date.trim().is_empty() {
                return None;
            }
            let author = commit
                .author
                .map(|author| {
                    author
                        .user
                        .map(|user| user.login)
                        .filter(|login| !login.trim().is_empty())
                        .unwrap_or(author.name)
                })
                .unwrap_or_default();
            Some(GitHubWorkItemCommit {
                oid: commit.oid,
                message_headline: commit.message_headline,
                author,
                committed_date: commit.committed_date,
                url: commit.url,
            })
        }));
        truncated |= github_nodes_truncated(&pull.comments);
        comments.extend(
            pull.comments
                .nodes
                .into_iter()
                .filter_map(github_issue_comment),
        );
        truncated |= github_nodes_truncated(&pull.reviews);
        comments.extend(
            pull.reviews
                .nodes
                .into_iter()
                .filter_map(github_review_comment),
        );
        truncated |= github_nodes_truncated(&pull.review_threads);
        comments.extend(
            pull.review_threads
                .nodes
                .into_iter()
                .filter_map(github_review_thread),
        );
    } else {
        let Some(issue) = repository.issue else {
            return Err(graphql_error.unwrap_or_else(|| "GitHub issue not found".into()));
        };
        truncated |= github_nodes_truncated(&issue.comments);
        comments.extend(
            issue
                .comments
                .nodes
                .into_iter()
                .filter_map(github_issue_comment),
        );
    }

    comments.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(GitHubWorkItemThread {
        comments,
        commits,
        truncated,
        review_decision,
        base_ref_name,
        head_ref_name,
    })
}

pub(super) fn github_nodes_truncated<T>(nodes: &GithubGraphqlNodes<T>) -> bool {
    (nodes.nodes.len() as i64) < nodes.total_count
}

pub(super) fn github_actor_login(author: Option<GithubGraphqlActor>) -> String {
    author
        .map(|author| author.login)
        .unwrap_or_default()
        .trim()
        .to_string()
}

pub(super) fn github_comment_id(kind: &str, id: &str, author: &str, created_at: &str) -> String {
    let id = id.trim();
    if !id.is_empty() {
        return id.to_string();
    }
    format!("{kind}:{author}:{created_at}")
}

pub(super) fn github_issue_comment(comment: GithubGraphqlComment) -> Option<GitHubWorkItemComment> {
    github_mapped_comment(comment, "comment", "", false)
}

pub(super) fn github_mapped_comment(
    comment: GithubGraphqlComment,
    kind: &str,
    fallback_path: &str,
    resolved: bool,
) -> Option<GitHubWorkItemComment> {
    if comment.is_minimized {
        return None;
    }
    let author = github_actor_login(comment.author);
    let created_at = comment.created_at.trim().to_string();
    let path = if comment.path.trim().is_empty() {
        fallback_path.trim().to_string()
    } else {
        comment.path.trim().to_string()
    };
    Some(GitHubWorkItemComment {
        id: github_comment_id(kind, &comment.id, &author, &created_at),
        kind: kind.to_string(),
        author_avatar_url: github_avatar_url(&author),
        author,
        body: comment.body,
        created_at,
        url: comment.url,
        state: String::new(),
        path,
        line: comment.line.or(comment.original_line),
        resolved,
        thread_id: String::new(),
        replies: Vec::new(),
    })
}

pub(super) fn github_review_comment(review: GithubGraphqlReview) -> Option<GitHubWorkItemComment> {
    let state = review.state.trim().to_uppercase();
    if state.is_empty() || state == "PENDING" {
        return None;
    }
    let body = review.body.trim();
    if state == "COMMENTED" && body.is_empty() {
        return None;
    }
    let created_at = review
        .submitted_at
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();
    if created_at.is_empty() {
        return None;
    }
    let author = github_actor_login(review.author);
    Some(GitHubWorkItemComment {
        id: github_comment_id("review", &review.id, &author, &created_at),
        kind: "review".into(),
        author_avatar_url: github_avatar_url(&author),
        author,
        body: review.body,
        created_at,
        url: review.url,
        state,
        path: String::new(),
        line: None,
        resolved: false,
        thread_id: String::new(),
        replies: Vec::new(),
    })
}

pub(super) fn github_review_thread(
    thread: GithubGraphqlReviewThread,
) -> Option<GitHubWorkItemComment> {
    let thread_id = thread.id.trim().to_string();
    let mut mapped = thread.comments.nodes.into_iter().filter_map(|comment| {
        github_mapped_comment(comment, "review_comment", &thread.path, thread.is_resolved)
    });
    let mut first = mapped.next()?;
    first.thread_id = thread_id.clone();
    first.replies = mapped
        .map(|mut reply| {
            reply.thread_id = thread_id.clone();
            reply
        })
        .collect();
    Some(first)
}

pub(super) fn github_avatar_url(login: &str) -> String {
    let login = login.trim();
    if login.is_empty() {
        return String::new();
    }
    let mut encoded = String::with_capacity(login.len());
    for byte in login.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    format!("https://avatars.githubusercontent.com/{encoded}?s=64")
}
