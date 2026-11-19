use super::*;
use std::process::Command;

struct Temp(PathBuf);
impl Temp {
    fn new(label: &str) -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "monocode-mobile-{label}-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // Windows writes a canonical path as \\?\C:\..., which Git and
        // people never type; keep the plain form.
        let dir = dir.canonicalize().unwrap().to_string_lossy().into_owned();
        Self(PathBuf::from(dir.trim_start_matches(r"\\?\")))
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run_git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=Test", "-c", "user.email=t@example.com"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn reconstructs(runs: &Runs, line: &str) -> bool {
    runs.iter()
        .map(|(text, _)| text.as_str())
        .collect::<String>()
        == line
}

#[test]
fn inside_root_refuses_paths_that_leave_the_project() {
    let dir = Temp::new("inside-root");
    let project = dir.0.join("project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("src/main.rs"), "fn main() {}").unwrap();
    std::fs::write(dir.0.join("secret"), "").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(dir.0.join("secret"), project.join("link")).unwrap();

    assert!(inside_root(&project, "src/main.rs").is_ok());
    assert!(inside_root(&project, "").is_ok());
    assert!(inside_root(&project, "../secret").is_err());
    assert!(inside_root(&project, "src/../../secret").is_err());
    assert!(inside_root(&project, &dir.0.join("secret").to_string_lossy()).is_err());
    #[cfg(unix)]
    assert!(inside_root(&project, "link").is_err());
    assert!(file(&project, "../secret").is_err());
}

#[test]
fn existing_worktrees_are_listed_and_resolve_to_their_own_root() {
    let dir = Temp::new("worktree-parity");
    let root = &dir.0;
    run_git(root, &["init", "-q"]);
    run_git(root, &["commit", "--allow-empty", "-qm", "init"]);
    let linked = root.join("linked");
    run_git(
        root,
        &[
            "worktree",
            "add",
            "-b",
            "feature/mobile",
            linked.to_str().unwrap(),
        ],
    );
    let result = worktrees(root).unwrap();
    let listed = result["worktrees"].as_array().unwrap();
    assert_eq!(listed.len(), 1);
    // Git writes C:/ on Windows; compare as paths, not text.
    assert_eq!(Path::new(listed[0]["path"].as_str().unwrap()), linked);
    assert_eq!(listed[0]["name"], "feature/mobile");
    assert_eq!(resolve_root(root, linked.to_str()).unwrap(), linked);
    let mut branches = HashMap::new();
    super::super::catalog::checkout_branch(root.to_str().unwrap(), &mut branches);
    assert_eq!(
        super::super::catalog::checkout_branch(linked.to_str().unwrap(), &mut branches),
        "feature/mobile"
    );
    std::fs::remove_dir_all(&linked).unwrap();
    assert_eq!(worktrees(root).unwrap()["worktrees"], json!([]));
    assert!(resolve_root(root, linked.to_str()).is_err());
}

#[test]
fn a_repository_lists_its_files_changes_and_shortened_paths() {
    let dir = Temp::new("repo");
    let root = &dir.0;
    run_git(root, &["init", "-q"]);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/app.rs"),
        "fn main() {\n    let a = 1;\n    println!(\"{a}\");\n}\n",
    )
    .unwrap();
    std::fs::write(root.join("notes.txt"), "hello\n").unwrap();
    run_git(root, &["add", "-A"]);
    run_git(root, &["commit", "-q", "-m", "init"]);

    let listing = files(root, "").unwrap();
    let entries = listing["entries"].as_array().unwrap();
    assert!(entries.iter().all(|entry| entry["name"] != ".git"));
    assert_eq!(entries[0]["name"], "src");
    assert_eq!(entries[0]["isDir"], true);
    assert!(entries.iter().any(|entry| entry["name"] == "notes.txt"
        && entry["isDir"] == false
        && entry["ignored"] == false));

    std::fs::write(
        root.join("src/app.rs"),
        "fn main() {\n    let a = 2;\n    println!(\"{a}\");\n}\n",
    )
    .unwrap();
    let reply = changes(root, Some("/wt")).unwrap();
    assert_eq!(reply["worktreePath"], "/wt");
    let changed = &reply["files"][0];
    assert_eq!(changed["relative"], "src/app.rs");
    assert_eq!(changed["status"], "modified");
    assert_eq!(changed["unstaged"], true);
    let lines = changed["lines"].as_array().unwrap();
    let kinds: Vec<&str> = lines
        .iter()
        .map(|line| line["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"add") && kinds.contains(&"del") && kinds.contains(&"context"));
    let del = lines.iter().find(|line| line["kind"] == "del").unwrap();
    assert_eq!(del["text"], "    let a = 1;");
    assert_eq!(del["oldNumber"], 2);
    assert_eq!(del["newNumber"], Value::Null);
    assert!(changed["palette"].as_array().is_some_and(|p| !p.is_empty()));
    for line in lines.iter().filter(|line| line.get("runs").is_some()) {
        let spelled: String = line["runs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|run| run[0].as_str().unwrap())
            .collect();
        assert_eq!(spelled, line["text"].as_str().unwrap());
    }

    let read = file(root, "app.rs:2").unwrap();
    assert_eq!(read["path"], "src/app.rs");
    assert_eq!(read["truncated"], false);
    assert!(read["highlighted"]["lines"].is_array());
    let plain = file(root, "notes.txt").unwrap();
    assert!(plain.get("highlighted").is_none() && plain.get("highlightJob").is_none());

    let found = search(root, "app").unwrap();
    assert_eq!(found["results"][0]["path"], "src/app.rs");
    assert_eq!(found["results"][0]["name"], "app.rs");
    assert_eq!(search(root, "  ").unwrap()["results"], json!([]));

    let status = git(root).unwrap();
    assert_eq!(status["files"], 1);
    assert_eq!(status["remote"], false);
    assert_eq!(status["canGenerate"], true);

    git_action(root, &json!({ "action": "stageAll" }), None).unwrap();
    assert_eq!(changes(root, None).unwrap()["files"][0]["staged"], true);
    assert!(git_action(root, &json!({ "action": "nope" }), None).is_err());
    assert!(resolve_root(root, Some("/not/a/worktree")).is_err());
    assert_eq!(worktrees(root).unwrap()["worktrees"], json!([]));
}

#[test]
fn a_file_highlighted_in_pieces_colours_as_it_does_whole() {
    // The comment opens in the first piece and closes in the second.
    let text = "/* opens here\nstill a comment\n*/ const x = 1;\nlet y = \"s\";";
    let lines: Vec<&str> = text.split('\n').collect();
    let mut palette = Palette::default();
    let whole = highlight_lines(
        &mut LineHighlighter::new(Some("js"), None),
        &lines,
        &mut palette,
    )
    .unwrap();
    let mut highlighter = LineHighlighter::new(Some("js"), None);
    let mut pieces = highlight_lines(&mut highlighter, &lines[..1], &mut palette).unwrap();
    pieces.extend(highlight_lines(&mut highlighter, &lines[1..], &mut palette).unwrap());
    assert_eq!(pieces, whole);
    let comment = whole[0][0].1;
    assert_eq!(whole[1], vec![("still a comment".to_string(), comment)]);
}

#[test]
fn highlight_runs_spell_out_each_line() {
    let long = format!("let x = \"{}\";", "a".repeat(2_000));
    let text = format!(
        "/* a comment\n spanning */\r\nconst s = \"héllo\"; // ünïcode\n\n{long}\nfunction f() {{ return 1; }}"
    );
    let highlighted = highlight_file("x.ts", &text).unwrap();
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(highlighted.lines.len(), lines.len());
    for (runs, line) in highlighted.lines.iter().zip(&lines) {
        assert!(reconstructs(runs, line), "{runs:?} vs {line:?}");
        assert!(
            runs.iter()
                .all(|(_, color)| *color < highlighted.palette.len())
        );
    }
    assert_eq!(highlighted.lines[4].len(), 1, "long lines stay plain");
    assert!(
        highlighted
            .palette
            .iter()
            .all(|c| c.len() == 7 && c.starts_with('#'))
    );
    assert!(highlight_file("notes.txt", "hello").is_none());
    assert!(highlight_file("LICENSE", "hello").is_none());
}

#[test]
fn large_files_highlight_in_the_background() {
    let text = "fn main() { let s = \"x\"; }\n".repeat(8_000);
    assert!(text.len() > MAX_HIGHLIGHT_BYTES);
    let job = highlight_in_background("big.rs", text.clone()).unwrap();
    let started = std::time::Instant::now();
    let mut progress = file_highlight(&job, 0);
    while progress["done"] != true && started.elapsed().as_secs() < 60 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        progress = file_highlight(&job, 0);
    }
    assert_eq!(progress["done"], true);
    let lines = progress["lines"].as_array().unwrap();
    assert_eq!(lines.len(), text.split('\n').count());
    let first: String = lines[0]
        .as_array()
        .unwrap()
        .iter()
        .map(|run| run[0].as_str().unwrap())
        .collect();
    assert_eq!(first, "fn main() { let s = \"x\"; }");
    assert_eq!(file_highlight(&job, lines.len())["lines"], json!([]));
    assert_eq!(file_highlight("missing", 0)["done"], true);
}

#[test]
fn locations_are_stripped_from_references() {
    assert_eq!(strip_location("src/app.ts:12"), "src/app.ts");
    assert_eq!(strip_location("src/app.ts:12:3"), "src/app.ts");
    assert_eq!(strip_location("src/app.ts#L4-L9"), "src/app.ts");
    assert_eq!(strip_location("src/app.ts"), "src/app.ts");
}
