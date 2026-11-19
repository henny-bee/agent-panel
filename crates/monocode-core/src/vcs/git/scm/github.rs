use super::*;

pub(super) fn pr_status_for(root: &Path) -> Option<Pr> {
    let branch = git_branch(root)?;
    let repo = github_repo_for(root).ok()?;
    let head = github_pr_head_filter(&repo, &branch)?;
    let json = gh_run(
        root,
        &[
            "pr",
            "list",
            "--head",
            &head,
            "--json",
            "number,title,url,state",
            "--limit",
            "20",
            "--state",
            "all",
        ],
    )
    .ok()?;
    parse_gh_pr_list(&json)
}

pub(super) fn github_pr_head_filter(repo: &str, branch: &str) -> Option<String> {
    let (owner, _) = split_github_repo(repo).ok()?;
    Some(format!("{owner}:{branch}"))
}

pub(super) fn github_repo_for(root: &Path) -> Result<String, String> {
    let json = gh_run(root, &["repo", "view", "--json", "nameWithOwner"])?;
    #[derive(Deserialize)]
    struct View {
        #[serde(rename = "nameWithOwner")]
        name_with_owner: String,
    }
    let view: View = serde_json::from_str(&json).map_err(|error| error.to_string())?;
    let slug = view.name_with_owner.trim();
    if slug.is_empty() || !slug.contains('/') {
        return Err("GitHub did not return a repository".into());
    }
    Ok(slug.to_string())
}

pub(super) fn split_github_repo(slug: &str) -> Result<(String, String), String> {
    let slug = slug.trim();
    let Some((owner, name)) = slug.split_once('/') else {
        return Err("GitHub did not return a repository".into());
    };
    let owner = owner.trim();
    let name = name.trim();
    if owner.is_empty()
        || name.is_empty()
        || name.contains('/')
        || owner.chars().any(char::is_whitespace)
        || name.chars().any(char::is_whitespace)
    {
        return Err("GitHub did not return a repository".into());
    }
    Ok((owner.to_string(), name.to_string()))
}

pub(super) fn parse_gh_pr_list(json: &str) -> Option<Pr> {
    #[derive(Deserialize)]
    struct Row {
        number: i64,
        title: String,
        url: String,
        state: String,
    }
    let rows: Vec<Row> = serde_json::from_str(json).ok()?;
    let mut best: Option<Pr> = None;
    for row in rows {
        let pr = Pr {
            number: row.number,
            title: row.title,
            url: row.url,
            state: row.state.to_lowercase(),
        };
        if pr.state == "open" {
            return Some(pr);
        }
        if best.is_none() {
            best = Some(pr);
        }
    }
    best
}

pub(super) fn gh_run(root: &Path, args: &[&str]) -> Result<String, String> {
    let program = launch_env::resolve_gui_binary("gh")
        .ok_or_else(|| "GitHub CLI (`gh`) is not installed.".to_string())?;
    let mut cmd = Command::new(&program);
    cmd.current_dir(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_PAGER", "cat")
        .env("GIT_PAGER", "cat");
    launch_env::apply_gui_env(&mut cmd);
    crate::sys::process::configure_child(&mut cmd);
    let output = cmd.output().map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            "GitHub CLI (`gh`) is not installed.".to_string()
        } else {
            error.to_string()
        }
    })?;
    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if text.is_empty() {
            return Err("gh returned no output".into());
        }
        return Ok(text);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!("gh {} failed", args.join(" "))
    };
    Err(detail)
}
