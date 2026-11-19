use super::*;

impl Changes {
    pub(super) fn set_status(&mut self, text: &str, cx: &mut Context<Self>) {
        self.status = Some(text.into());
        self.status_clear = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(4)).await;
            let _ = this.update(cx, |this, cx| {
                this.status = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn mutated(&mut self, cx: &mut Context<Self>) {
        self.git.update(cx, |git, cx| git.changed(cx));
    }

    /// `window.alert`.
    pub(super) fn fail(error: String, window: &mut Window, cx: &mut App) {
        drop(window.prompt(PromptLevel::Critical, &error, None, &["OK"], cx));
    }

    /// confirmNative: Tauri's `ask` with the OK label it was given.
    pub(super) fn confirm(
        message: String,
        ok: &'static str,
        window: &mut Window,
        cx: &mut App,
    ) -> gpui::Task<bool> {
        let answer = window.prompt(
            PromptLevel::Warning,
            &message,
            None,
            &[
                gpui::PromptButton::ok(ok),
                gpui::PromptButton::cancel("Cancel"),
            ],
            cx,
        );
        cx.background_executor()
            .spawn(async move { matches!(answer.await, Ok(0)) })
    }

    /// Runs `work` against the checkout with `busy` held, then rereads.
    pub(super) fn run_git(
        &mut self,
        busy: String,
        work: impl FnOnce(PathBuf) -> Result<(), String> + Send + 'static,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(busy);
        cx.notify();
        let path = PathBuf::from(&self.cwd);
        let task = cx.background_executor().spawn(async move { work(path) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = None;
                this.mutated(cx);
                match result {
                    Ok(()) => then(this, window, cx),
                    Err(error) => Self::fail(error, window, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn run_file(
        &mut self,
        file: ChangedFile,
        action: FileAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        let relative = file.relative.clone();
        let start = move |this: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
            this.run_git(
                relative.clone(),
                move |path| match action {
                    FileAction::Stage => scm::stage_file(&path, &relative),
                    FileAction::Unstage => scm::unstage_file(&path, &relative),
                    FileAction::Discard => scm::discard_file(&path, &relative),
                },
                |_, _, _| {},
                window,
                cx,
            );
        };
        if action != FileAction::Discard {
            start(self, window, cx);
            return;
        }
        let name = basename(&file.relative).to_owned();
        let untracked = file.status == "untracked";
        let ask = Self::confirm(
            if untracked {
                format!("Delete untracked file {name}?")
            } else {
                format!("Discard changes in {name}? This cannot be undone.")
            },
            if untracked { "Delete" } else { "Discard" },
            window,
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if ask.await {
                let _ = this.update_in(cx, start);
            }
        })
        .detach();
    }

    pub(super) fn run_all(
        &mut self,
        action: FileAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        let key = match action {
            FileAction::Stage => "stage",
            FileAction::Unstage => "unstage",
            FileAction::Discard => "discard",
        };
        let start = move |this: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
            this.run_git(
                key.into(),
                move |path| match action {
                    FileAction::Stage => scm::stage_all(&path),
                    FileAction::Unstage => scm::unstage_all(&path),
                    FileAction::Discard => scm::discard_all(&path),
                },
                |_, _, _| {},
                window,
                cx,
            );
        };
        if action != FileAction::Discard {
            start(self, window, cx);
            return;
        }
        let index = self.index(cx);
        let (_, unstaged) = Self::staged_unstaged(index.as_ref());
        let Some(index) = index.filter(|_| !unstaged.is_empty()) else {
            return;
        };
        let only = &index.files[unstaged[0]];
        let n = unstaged.len();
        let untracked_only = n == 1 && only.status == "untracked";
        let message = if untracked_only {
            format!("Delete untracked file {}?", basename(&only.relative))
        } else if n == 1 {
            format!(
                "Discard changes in {}? This cannot be undone.",
                basename(&only.relative)
            )
        } else {
            format!("Discard all unstaged changes in {n} files? This cannot be undone.")
        };
        let ask = Self::confirm(
            message,
            if untracked_only { "Delete" } else { "Discard" },
            window,
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if ask.await {
                let _ = this.update_in(cx, start);
            }
        })
        .detach();
    }

    pub(super) fn run_folder(
        &mut self,
        relative: String,
        stage: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = format!("{}:{relative}", if stage { "stage" } else { "unstage" });
        self.run_git(
            key,
            move |path| {
                if stage {
                    scm::stage_file(&path, &relative)
                } else {
                    scm::unstage_file(&path, &relative)
                }
            },
            |_, _, _| {},
            window,
            cx,
        );
    }

    pub(super) fn pull(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let index = self.index(cx);
        if !index.is_some_and(|i| i.remote.is_some() && i.upstream.is_some()) {
            return;
        }
        self.status = None;
        self.run_git(
            "pull".into(),
            |path| scm::pull(&path),
            |this, _, cx| this.set_status("Pull complete", cx),
            window,
            cx,
        );
    }

    pub(super) fn cancel_generate(&mut self) {
        if let Some(cancel) = self.generate_cancel.take() {
            cancel.store(true, Ordering::SeqCst);
            if self.busy.as_deref() == Some("generate") {
                self.busy = None;
            }
        }
    }

    /// pickTextHarness: the background model picked in Settings, else the
    /// conversation's provider when installed, else Claude Code, then Codex.
    pub(super) fn text_harness(&self, cx: &App) -> Option<TextHarness> {
        let chosen = cx
            .try_global::<crate::ui::preferences::Preferences>()
            .and_then(|p| p.background_text_model());
        pick_text_harness(self.text_provider, chosen)
    }

    pub(super) fn generate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let has_files = self.index(cx).is_some_and(|i| !i.files.is_empty());
        if !has_files || self.busy.is_some() || self.generate_cancel.is_some() {
            return;
        }
        let Some(harness) = self.text_harness(cx) else {
            Self::fail(
                "No provider CLI is installed to write a commit message.".into(),
                window,
                cx,
            );
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.generate_cancel = Some(cancel.clone());
        self.busy = Some("generate".into());
        cx.notify();
        let cwd = PathBuf::from(&self.cwd);
        let flag = cancel.clone();
        let work = cx
            .background_executor()
            .spawn(async move { generate_commit_message(harness, cwd, flag) });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if cancel.load(Ordering::SeqCst) {
                    return;
                }
                this.generate_cancel = None;
                this.busy = None;
                match result {
                    Ok(message) => this.message.update(cx, |m, cx| m.set_text(message, cx)),
                    Err(error) => Self::fail(error, window, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn toggle_amend(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.amend.take().is_some() {
            cx.notify();
            return;
        }
        let path = PathBuf::from(&self.cwd);
        let work = cx
            .background_executor()
            .spawn(async move { scm::head_message(&path) });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(head) => {
                    if this.message.read(cx).content.trim().is_empty() {
                        this.message.update(cx, |m, cx| m.set_text(head, cx));
                    }
                    let index = this.index(cx);
                    this.amend = Some((
                        index.as_ref().and_then(|i| i.branch.clone()),
                        index.and_then(|i| i.head),
                    ));
                    cx.notify();
                }
                Err(error) => Self::fail(error, window, cx),
            });
        })
        .detach();
    }

    pub(super) fn can_commit(&self, cx: &App) -> bool {
        let index = self.git.read(cx).index.as_ref();
        let staged = index.is_some_and(|i| i.files.iter().any(|f| f.staged));
        (staged || self.amend.is_some())
            && !self.message.read(cx).content.trim().is_empty()
            && self.busy.is_none()
    }

    pub(super) fn on_default(index: Option<&DiffIndex>) -> bool {
        index.is_some_and(|i| i.branch.is_some() && i.branch == i.default_branch)
    }

    pub(super) fn commit(
        &mut self,
        push: bool,
        create_pr: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_commit(cx) {
            return;
        }
        let index = self.index(cx);
        let mut questions = Vec::new();
        if (push || create_pr)
            && Self::on_default(index.as_ref())
            && let Some(branch) = index.as_ref().and_then(|i| i.branch.clone())
        {
            questions.push((
                if create_pr {
                    format!("Create a pull request from default branch \"{branch}\"?")
                } else {
                    format!("Push to default branch \"{branch}\"?")
                },
                "OK",
            ));
        }
        if self.amend.is_some() && index.as_ref().is_some_and(|i| i.head_pushed) {
            questions.push((
                "Amend a commit that is already pushed? MCode cannot push the result. You will need a force push from the terminal.".into(),
                "Amend",
            ));
        }
        self.menu = None;
        cx.spawn_in(window, async move |this, cx| {
            for (question, ok) in questions {
                let Ok(ask) =
                    this.update_in(cx, |_, window, cx| Self::confirm(question, ok, window, cx))
                else {
                    return;
                };
                if !ask.await {
                    return;
                }
            }
            let _ = this.update_in(cx, |this, window, cx| {
                this.commit_now(push, create_pr, window, cx)
            });
        })
        .detach();
    }

    pub(super) fn commit_now(
        &mut self,
        push: bool,
        create_pr: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = self.message.read(cx).content.to_string();
        let amend = self.amend.is_some();
        let pr = create_pr.then(|| self.text_harness(cx));
        self.run_git(
            if create_pr { "pr" } else { "commit" }.into(),
            move |path| {
                scm::commit(&path, &message, amend)?;
                if push || create_pr {
                    scm::push(&path)?;
                }
                Ok(())
            },
            move |this, window, cx| {
                this.amend = None;
                this.message
                    .update(cx, |m, cx| m.set_text(String::new(), cx));
                if let Some(harness) = pr {
                    this.open_created_pr(harness, window, cx);
                }
            },
            window,
            cx,
        );
    }

    /// generatePrContent, `gh pr create`, then the browser.
    pub(super) fn open_created_pr(
        &mut self,
        harness: Option<TextHarness>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.busy = Some("pr".into());
        cx.notify();
        let cwd = PathBuf::from(&self.cwd);
        let work = cx
            .background_executor()
            .spawn(async move { create_pr_blocking(harness, &cwd) });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = None;
                this.mutated(cx);
                this.load_pr(cx);
                match result {
                    Ok(url) => cx.open_url(url.trim()),
                    Err(error) => Self::fail(error, window, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_git(
            "sync".into(),
            |path| scm::sync(&path),
            |this, _, cx| this.load_pr(cx),
            window,
            cx,
        );
    }

    pub(super) fn create_pr(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let index = self.index(cx);
        let ahead = index.as_ref().is_some_and(|i| i.ahead > 0);
        let question = Self::on_default(index.as_ref()).then(|| {
            format!(
                "Create a pull request from default branch \"{}\"?",
                index
                    .as_ref()
                    .and_then(|i| i.branch.clone())
                    .unwrap_or_default()
            )
        });
        let harness = self.text_harness(cx);
        cx.spawn_in(window, async move |this, cx| {
            if let Some(question) = question {
                let Ok(ask) = this.update_in(cx, |_, window, cx| {
                    Self::confirm(question, "OK", window, cx)
                }) else {
                    return;
                };
                if !ask.await {
                    return;
                }
            }
            let _ = this.update_in(cx, |this, window, cx| {
                if ahead {
                    this.run_git(
                        "pr".into(),
                        |path| scm::push(&path),
                        move |this, window, cx| this.open_created_pr(harness, window, cx),
                        window,
                        cx,
                    );
                } else {
                    this.open_created_pr(harness, window, cx);
                }
            });
        })
        .detach();
    }

    pub(super) fn open_menu(
        &mut self,
        kind: MenuKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.menu.as_ref().is_some_and(|m| m.kind == kind) {
            self.menu = None;
            cx.notify();
            return;
        }
        self.menu = None;
        let index = self.index(cx);
        let items: Vec<MenuItem> = match kind {
            MenuKind::Branch => {
                let can_pull = index
                    .as_ref()
                    .is_some_and(|i| i.remote.is_some() && i.upstream.is_some());
                let pulling = self.busy.as_deref() == Some("pull");
                vec![
                    MenuAction::new("pull", if pulling { "Pulling…" } else { "Pull" })
                        .icon(if pulling { "loader" } else { "refresh-cw" })
                        .disabled(self.busy.is_some() || !can_pull)
                        .into(),
                ]
            }
            MenuKind::Commit => {
                let (can_push, can_push_pr) = self.push_options(cx);
                vec![
                    MenuAction::new("push", "Commit & Push")
                        .disabled(!can_push)
                        .into(),
                    MenuAction::new("push-pr", "Commit, Push & Create PR")
                        .disabled(!can_push_pr)
                        .into(),
                    MenuItem::Separator,
                    MenuAction::new("amend", "Amend Last Commit")
                        .checked(self.amend.is_some())
                        .into(),
                ]
            }
        };
        let (label, width) = match kind {
            MenuKind::Branch => ("Branch actions", 144.),
            MenuKind::Commit => ("Commit options", 192.),
        };
        let menu = cx.new(|cx| {
            Menu::new(label, items, window, cx)
                .width(width)
                .style(MenuStyle::Git)
        });
        let events = cx.subscribe_in(
            &menu,
            window,
            move |this, _, event: &MenuEvent, window, cx| {
                this.menu = None;
                if let MenuEvent::Picked(id, _) = event {
                    match id.as_ref() {
                        "pull" => this.pull(window, cx),
                        "push" => this.commit(true, false, window, cx),
                        "push-pr" => this.commit(true, true, window, cx),
                        "amend" => this.toggle_amend(window, cx),
                        _ => {}
                    }
                }
                cx.notify();
            },
        );
        self.menu = Some(OpenMenu {
            kind,
            menu,
            _events: events,
        });
        cx.notify();
    }

    pub(super) fn push_options(&self, cx: &App) -> (bool, bool) {
        let index = self.git.read(cx).index.as_ref();
        let has_remote = index.is_some_and(|i| i.remote.is_some());
        let diverged = index.is_some_and(|i| i.ahead > 0 && i.behind > 0);
        let head_pushed = index.is_some_and(|i| i.head_pushed);
        let can_push = self.can_commit(cx)
            && has_remote
            && !diverged
            && (self.amend.is_none() || !head_pushed);
        let open_pr = self.pr.as_ref().is_some_and(|p| p.state == "open");
        (can_push, can_push && !open_pr && !Self::on_default(index))
    }
}

/// generateCommitMessage: a message for the staged changes, written by a
/// read-only turn of `kind`. Blocking.
pub(crate) fn generate_commit_message(
    harness: TextHarness,
    cwd: PathBuf,
    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    let kind = harness.0;
    let context = scm::staged_context(&cwd)?;
    let prompt =
        scm::commit_message_prompt(context.branch.as_deref(), &context.summary, &context.patch);
    let output = text_prompt(harness, cwd, prompt, cancel)?;
    scm::parse_commit_message(&output).ok_or_else(|| {
        let snippet: String = output
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(240)
            .collect();
        if snippet.is_empty() {
            format!(
                "Could not generate a commit message. {} returned no text.",
                kind.label()
            )
        } else {
            format!("Could not generate a commit message. Model replied: {snippet}")
        }
    })
}

/// generatePrContent, falling back to the commits, then `gh pr create`; the
/// new pull request's URL. Blocking.
pub(crate) fn create_pr_blocking(
    harness: Option<TextHarness>,
    cwd: &std::path::Path,
) -> Result<String, String> {
    let range = scm::range_context(cwd)?;
    let parsed = harness.and_then(|harness| {
        let output = text_prompt(
            harness,
            cwd.to_path_buf(),
            scm::pr_content_prompt(&range),
            Arc::new(AtomicBool::new(false)),
        )
        .ok()?;
        scm::parse_pr_content(&output)
    });
    let first = range.commit_summary.lines().next().unwrap_or("").trim();
    let title = parsed
        .as_ref()
        .map(|(t, _)| t.clone())
        .filter(|t| !t.is_empty())
        .or_else(|| (!first.is_empty()).then(|| first.to_owned()))
        .unwrap_or_else(|| format!("Update {}", range.head));
    let body = parsed
        .map(|(_, b)| b)
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| range.commit_summary.trim().to_owned());
    scm::pr_create(cwd, &title, &body, &range.base, &range.head)
}

/// A read-only one-shot turn, as the React text harness runs: the final
/// assistant message's text.
pub(super) fn text_prompt(
    (kind, binary, model): TextHarness,
    cwd: PathBuf,
    prompt: String,
    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    let request = Request {
        kind,
        binary,
        cwd,
        prompt,
        provider_id: None,
        model,
        settings: Settings {
            access: AccessMode::ReadOnly,
            ..Default::default()
        },
    };
    let mut text = String::new();
    provider::run(request, cancel.clone(), |events| {
        if cancel.load(Ordering::SeqCst) {
            return Err("Cancelled".into());
        }
        for event in events {
            match event {
                Event::BeginAssistant => text.clear(),
                Event::Text(chunk) => text.push_str(&chunk),
                _ => {}
            }
        }
        Ok(())
    })?;
    Ok(text)
}
