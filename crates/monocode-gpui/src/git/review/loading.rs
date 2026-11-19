use super::*;

impl ReviewPane {
    pub(crate) fn set_deferred(&mut self, deferred: bool, cx: &mut Context<Self>) {
        if self.deferred != deferred {
            if deferred {
                self.remember_body_focus(cx);
            }
            if self.deferred {
                self.revealed = (!cx.reduce_motion()).then(PaneReveal::default);
            }
            self.deferred = deferred;
            if !deferred {
                self.load_active(cx);
            }
            cx.notify();
        }
    }

    pub(crate) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.closed {
            return;
        }
        let was = self.visible;
        self.visible = visible;
        if !visible && was {
            for source in &mut self.sources {
                source.cancel();
            }
        }
        for (ix, source) in self.sources.iter_mut().enumerate() {
            if ix != self.state.active && source.loading {
                source.cancel();
            }
            if let Some(editor) = &source.editor {
                editor.update(cx, |e, cx| {
                    e.set_visible(
                        source.editor_mode && visible && !self.deferred && ix == self.state.active,
                        cx,
                    )
                });
            }
        }
        if visible && !was {
            self.load_active(cx);
        }
    }

    pub(crate) fn cache_closed(&mut self, cx: &mut Context<Self>) {
        self.closed = true;
        self.visible = false;
        for source in &mut self.sources {
            source.cancel();
            if let Some(editor) = &source.editor {
                editor.update(cx, |editor, cx| editor.set_visible(false, cx));
            }
            self.git
                .update(cx, |git, _| git.review_cache.remember(source));
        }
    }

    /// Focus and Git changes reread the working tree; commits never change.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.visible
            && let Some(source) = self.sources.get(self.state.active)
            && matches!(
                source.tab,
                ReviewTab::Changes { .. } | ReviewTab::Session { .. }
            )
        {
            self.load_active(cx);
        }
    }

    pub(super) fn load_active(&mut self, cx: &mut Context<Self>) {
        if !self.visible || self.deferred {
            return;
        }
        for (ix, source) in self.sources.iter_mut().enumerate() {
            if ix != self.state.active && source.loading {
                source.cancel();
            }
            if let Some(editor) = &source.editor {
                editor.update(cx, |e, cx| {
                    e.set_visible(
                        source.editor_mode && self.visible && ix == self.state.active,
                        cx,
                    )
                });
            }
        }
        let ix = self.state.active;
        let Some(source) = self.sources.get_mut(ix) else {
            return;
        };
        if source.editor_mode && source.editor.is_some() {
            return;
        }
        if source.complete && matches!(source.tab, ReviewTab::Commit { .. }) {
            return;
        }
        if source.loading {
            source.refresh_pending = true;
            return;
        }
        source.loading = true;
        let generation = source.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let tab = source.tab.clone();
        let counter = source.generation.clone();
        let known_index = match &tab {
            ReviewTab::Changes { cwd, .. } if self.git.read(cx).project() == cwd => {
                self.git.read(cx).index.clone()
            }
            _ => None,
        };
        let list = cx.background_executor().spawn({
            let tab = tab.clone();
            async move {
                match &tab {
                    ReviewTab::File { .. } => unreachable!(),
                    ReviewTab::Changes { cwd, .. } => {
                        let root = PathBuf::from(cwd);
                        let index = scm::review_index(&root, known_index.as_ref())?;
                        let versions = scm::file_diff_versions(&root, &index).ok();
                        Ok((index.files, versions))
                    }
                    ReviewTab::Commit { cwd, sha, .. } => {
                        scm::commit_files(&PathBuf::from(cwd), sha).map(|files| (files, None))
                    }
                    ReviewTab::Session { cwd, session, .. } => session_files(cwd, *session),
                }
            }
        });
        let task = cx.spawn(async move |this, cx| {
            let listed = list.await;
            let order = this.update(cx, |this, cx| {
                let source = this
                    .sources
                    .iter_mut()
                    .find(|s| Arc::ptr_eq(&s.generation, &counter))?;
                if counter.load(Ordering::SeqCst) != generation {
                    return None;
                }
                match listed {
                    Ok((files, versions)) => {
                        source.error = None;
                        let order: Vec<(String, Option<bool>, ChangedFile)> = match &tab {
                            ReviewTab::File { .. } => unreachable!(),
                            ReviewTab::Changes { path, staged, .. } => {
                                let mut entries: Vec<_> = entries(&files, *staged)
                                    .into_iter()
                                    .map(|(id, kind, file)| (id, Some(kind), file))
                                    .collect();
                                // The focused file loads first.
                                if let Some(at) = path.as_ref().and_then(|p| {
                                    entries
                                        .iter()
                                        .position(|e| &e.2.path == p || &e.2.relative == p)
                                }) {
                                    let focused = entries.remove(at);
                                    entries.insert(0, focused);
                                }
                                entries
                            }
                            ReviewTab::Commit { .. } => files
                                .iter()
                                .map(|f| (f.relative.clone(), None, f.clone()))
                                .collect(),
                            ReviewTab::Session { path, .. } => {
                                let mut entries: Vec<_> = files
                                    .iter()
                                    .map(|f| (f.relative.clone(), None, f.clone()))
                                    .collect();
                                if let Some(at) = path
                                    .as_ref()
                                    .and_then(|p| entries.iter().position(|e| &e.2.path == p))
                                {
                                    let focused = entries.remove(at);
                                    entries.insert(0, focused);
                                }
                                entries
                            }
                        };
                        let retained: std::collections::HashSet<_> =
                            order.iter().map(|e| e.0.clone()).collect();
                        source.loaded.retain(|id, _| retained.contains(id));
                        source.files = Some(files);
                        this.publish(cx);
                        Some((order, versions))
                    }
                    Err(error) => {
                        source.error = Some(error);
                        source.files = Some(Vec::new());
                        source.loading = false;
                        source.refresh_pending = false;
                        cx.notify();
                        None
                    }
                }
            });
            let Ok(Some((order, versions))) = order else {
                return;
            };
            let queue = Arc::new(std::sync::Mutex::new(order.into_iter()));
            let versions = Arc::new(versions);
            let mut workers = Vec::new();
            for _ in 0..CONCURRENCY {
                let queue = queue.clone();
                let versions = versions.clone();
                let counter = counter.clone();
                let tab = tab.clone();
                let this = this.clone();
                let mut cx = cx.clone();
                workers.push(async move {
                    loop {
                        if counter.load(Ordering::SeqCst) != generation {
                            return;
                        }
                        let Some((id, staged, file)) = queue.lock().unwrap().next() else {
                            return;
                        };
                        let previous = this
                            .update(&mut cx, |this, _| {
                                this.sources
                                    .iter()
                                    .find(|s| Arc::ptr_eq(&s.generation, &counter))
                                    .and_then(|s| s.loaded.get(&id).cloned())
                            })
                            .ok()
                            .flatten();
                        let version = versions.as_ref().as_ref().and_then(|v| v.get(&id)).cloned();
                        if previous.as_ref().is_some_and(|p| {
                            p.error.is_none()
                                && (matches!(tab, ReviewTab::Commit { .. })
                                    || (version.is_some() && p.version == version))
                        }) {
                            continue;
                        }
                        let tab = tab.clone();
                        let relative = file.relative.clone();
                        let work_counter = counter.clone();
                        let loaded = cx
                            .background_executor()
                            .spawn(async move {
                                let diff = match &tab {
                                    ReviewTab::File { .. } => unreachable!(),
                                    ReviewTab::Changes { cwd, .. } => scm::file_diff(
                                        &PathBuf::from(cwd),
                                        &relative,
                                        staged == Some(true),
                                    ),
                                    ReviewTab::Commit { cwd, sha, .. } => {
                                        scm::commit_file_diff(&PathBuf::from(cwd), sha, &relative)
                                    }
                                    ReviewTab::Session { cwd, session, .. } => {
                                        session_file_diff(cwd, *session, &relative)
                                    }
                                };
                                if work_counter.load(Ordering::SeqCst) != generation {
                                    return None;
                                }
                                match diff {
                                    Ok(diff) => {
                                        if let Some(prev) = previous.filter(|p| {
                                            p.error.is_none()
                                                && p.diff.original == diff.original
                                                && p.diff.current == diff.current
                                                && p.diff.binary == diff.binary
                                                && p.diff.too_large == diff.too_large
                                        }) {
                                            return Some(Arc::new(Loaded {
                                                version,
                                                ..prev.as_ref().clone()
                                            }));
                                        }
                                        let unified =
                                            (!diff.binary && !diff.too_large).then(|| {
                                                diff::build_unified_file(
                                                    &diff.original,
                                                    &diff.current,
                                                    diff::CONTEXT_DEFAULT,
                                                )
                                            });
                                        Some(Arc::new(Loaded {
                                            version,
                                            additions: unified.as_ref().map_or(0, |u| u.additions),
                                            deletions: unified.as_ref().map_or(0, |u| u.deletions),
                                            unified: unified.map(|u| Arc::new(u.blocks)),
                                            diff: Arc::new(diff),
                                            error: None,
                                        }))
                                    }
                                    Err(error) => Some(Arc::new(Loaded {
                                        version: None,
                                        diff: Arc::new(FileDiff::default()),
                                        unified: None,
                                        additions: 0,
                                        deletions: 0,
                                        error: Some(error),
                                    })),
                                }
                            })
                            .await;
                        let Some(loaded) = loaded else {
                            return;
                        };
                        let _ = this.update(&mut cx, |this, cx| {
                            if counter.load(Ordering::SeqCst) != generation {
                                return;
                            }
                            if let Some(source) = this
                                .sources
                                .iter_mut()
                                .find(|s| Arc::ptr_eq(&s.generation, &counter))
                            {
                                source.loaded.insert(id, loaded);
                            }
                            this.publish(cx);
                        });
                    }
                });
            }
            futures_join_all(workers).await;
            let _ = this.update(cx, |this, cx| {
                let mut refresh = false;
                if let Some(source) = this
                    .sources
                    .iter_mut()
                    .find(|s| Arc::ptr_eq(&s.generation, &counter))
                    && counter.load(Ordering::SeqCst) == generation
                {
                    source.complete = source.loaded.values().all(|l| l.error.is_none());
                    source.loading = false;
                    refresh = std::mem::take(&mut source.refresh_pending);
                }
                if refresh {
                    this.load_active(cx);
                }
            });
        });
        if let Some(source) = self.sources.get_mut(ix) {
            source._tasks = vec![task];
        }
    }

    /// WorkingTreeDiff's and CommitDiff's file models.
    pub(super) fn publish(&mut self, cx: &mut Context<Self>) {
        let ix = self.state.active;
        let Some(source) = self.sources.get(ix) else {
            return;
        };
        let Some(files) = &source.files else {
            cx.notify();
            return;
        };
        let models: Vec<DiffFile> = match &source.tab {
            ReviewTab::File { .. } => return,
            ReviewTab::Changes { staged, .. } => entries(files, *staged)
                .into_iter()
                .map(|(id, kind, file)| {
                    let loaded = source.loaded.get(&id);
                    let unchanged = loaded.is_some_and(|l| {
                        l.unified.is_some()
                            && l.additions == 0
                            && l.deletions == 0
                            && !l.diff.binary
                    });
                    let index_counts = loaded.is_some_and(|l| l.error.is_none())
                        && !(file.staged && file.unstaged);
                    let label = if file.staged && file.unstaged {
                        format!(
                            "{} ({})",
                            file.relative,
                            if kind { "Staged" } else { "Unstaged" }
                        )
                    } else {
                        file.relative.clone()
                    };
                    DiffFile {
                        id: id.into(),
                        path: file.path.clone(),
                        label,
                        binary: loaded.is_some_and(|l| l.diff.binary),
                        too_large: loaded.is_some_and(|l| l.diff.too_large),
                        empty_message: match loaded {
                            None => Some("Loading…".into()),
                            Some(l) if l.error.is_some() => Some(format!(
                                "Couldn’t load diff: {}",
                                l.error.clone().unwrap_or_default()
                            )),
                            _ if unchanged => Some(
                                if kind {
                                    "No staged changes"
                                } else {
                                    "No unstaged changes"
                                }
                                .into(),
                            ),
                            _ => None,
                        },
                        additions: loaded
                            .filter(|l| l.unified.is_some())
                            .map(|l| l.additions)
                            .unwrap_or(if index_counts {
                                file.additions as u32
                            } else {
                                0
                            }),
                        deletions: loaded
                            .filter(|l| l.unified.is_some())
                            .map(|l| l.deletions)
                            .unwrap_or(if index_counts {
                                file.deletions as u32
                            } else {
                                0
                            }),
                        blocks: if unchanged {
                            Arc::new(Vec::new())
                        } else {
                            loaded
                                .and_then(|l| l.unified.clone())
                                .unwrap_or_else(|| Arc::new(Vec::new()))
                        },
                        can_stage: !kind,
                        can_discard: !kind,
                        can_stage_hunk: !kind
                            && !loaded.is_some_and(|l| l.diff.binary || l.diff.too_large),
                    }
                })
                .collect(),
            ReviewTab::Commit { .. } | ReviewTab::Session { .. } => files
                .iter()
                .map(|file| {
                    let loaded = source.loaded.get(&file.relative);
                    let unified = loaded.and_then(|l| l.unified.clone());
                    DiffFile {
                        id: file.relative.clone().into(),
                        path: file.path.clone(),
                        label: file.relative.clone(),
                        binary: loaded.is_some_and(|l| l.diff.binary),
                        too_large: loaded.is_some_and(|l| l.diff.too_large),
                        empty_message: match loaded {
                            None => Some("Loading…".into()),
                            Some(l) if l.error.is_some() => Some(format!(
                                "Couldn’t load diff: {}",
                                l.error.clone().unwrap_or_default()
                            )),
                            Some(l)
                                if l.unified.is_some()
                                    && l.additions == 0
                                    && l.deletions == 0
                                    && !l.diff.binary =>
                            {
                                Some("No textual diff".into())
                            }
                            _ => None,
                        },
                        additions: loaded
                            .filter(|l| l.unified.is_some())
                            .map_or(file.additions as u32, |l| l.additions),
                        deletions: loaded
                            .filter(|l| l.unified.is_some())
                            .map_or(file.deletions as u32, |l| l.deletions),
                        blocks: unified.unwrap_or_else(|| Arc::new(Vec::new())),
                        can_stage: false,
                        can_discard: false,
                        can_stage_hunk: false,
                    }
                })
                .collect(),
        };
        let totals = models
            .iter()
            .fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions));
        let (focus, file_count) = match &source.tab {
            ReviewTab::File { .. } => return,
            ReviewTab::Changes { path, staged, .. } => (
                path.as_ref().and_then(|p| {
                    entries(files, *staged)
                        .into_iter()
                        .find(|(_, kind, f)| {
                            staged.is_none_or(|s| s == *kind) && (&f.path == p || &f.relative == p)
                        })
                        .map(|(id, _, _)| SharedString::from(id))
                }),
                if staged.is_some() {
                    models.len()
                } else {
                    files.len()
                },
            ),
            ReviewTab::Commit { .. } => (None, models.len()),
            ReviewTab::Session { path, .. } => (
                path.as_ref().and_then(|p| {
                    files
                        .iter()
                        .find(|f| &f.path == p)
                        .map(|f| SharedString::from(f.relative.clone()))
                }),
                models.len(),
            ),
        };
        source.view.update(cx, |view, cx| {
            view.totals = Some(totals);
            view.file_count = Some(file_count);
            view.set_files(models, cx);
            view.focus_file(focus, cx);
        });
        cx.notify();
    }

    pub(super) fn diff_event(&mut self, ix: usize, event: &DiffEvent, cx: &mut Context<Self>) {
        if self.closed {
            return;
        }
        let source = &self.sources[ix];
        let ReviewTab::Changes { cwd, .. } = &source.tab else {
            if let DiffEvent::Comment(text) = event {
                cx.emit(ReviewEvent::Comment(text.clone()));
            }
            return;
        };
        let cwd = PathBuf::from(cwd);
        let (id, work): (SharedString, Box<dyn FnOnce() -> Result<(), String> + Send>) = match event
        {
            DiffEvent::Comment(text) => {
                cx.emit(ReviewEvent::Comment(text.clone()));
                return;
            }
            DiffEvent::StageFile(id) | DiffEvent::DiscardFile(id) => {
                let Some(relative) = id.strip_prefix("unstaged:").map(str::to_owned) else {
                    return;
                };
                let stage = matches!(event, DiffEvent::StageFile(_));
                (
                    id.clone(),
                    Box::new(move || {
                        if stage {
                            scm::stage_file(&cwd, &relative)
                        } else {
                            scm::discard_file(&cwd, &relative)
                        }
                    }),
                )
            }
            DiffEvent::StageHunk(id, chunk) => {
                let Some(relative) = id.strip_prefix("unstaged:").map(str::to_owned) else {
                    return;
                };
                let Some(loaded) = source.loaded.get(id.as_ref()) else {
                    return;
                };
                // The same diff the view showed, so the same hunk is staged.
                let Some(next) =
                    diff::stage_chunk(&loaded.diff.original, &loaded.diff.current, *chunk)
                else {
                    return;
                };
                (
                    id.clone(),
                    Box::new(move || scm::stage_contents(&cwd, &relative, next.as_bytes())),
                )
            }
        };
        let view = source.view.clone();
        view.update(cx, |v, cx| {
            v.busy_id = Some(id.clone());
            cx.notify();
        });
        let task = cx.background_executor().spawn(async move { work() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                view.update(cx, |v, cx| {
                    v.busy_id = None;
                    cx.notify();
                });
                this.git.update(cx, |git, cx| git.changed(cx));
                if let Err(error) = result {
                    log_error(&error);
                }
            });
        })
        .detach();
    }
}
