use super::*;

impl Store {
    pub fn begin_turn(&self, session_id: &str, cwd: &str, id: &str) -> Result<(), String> {
        validate_id(session_id, "session")?;
        validate_id(id, "turn")?;
        let root = std::fs::canonicalize(project_root(cwd)?).map_err(|e| e.to_string())?;
        let discovered = run_git(&root, &["rev-parse", "--show-toplevel"]);
        if let Err(error) = &discovered {
            if error.contains("not a git repository")
                && read_review(&self.session_dir(session_id))?.is_none()
            {
                // Folders without Git have no recorded review.
                return Ok(());
            }
        }
        let dir = self.session_dir(session_id);
        let mut review = read_review(&dir)?
            .filter(|saved| saved.cwd == path_to_js(&root))
            .unwrap_or(TurnReview {
                cwd: path_to_js(&root),
                repo: String::new(),
                active: None,
                latest: None,
            });
        let pending = review
            .active
            .clone()
            .filter(|turn| turn.before.is_some() && turn.error.is_none());
        if let Some(pending) = &pending {
            // A reload can interrupt checkpoint completion after the provider
            // already wrote files. Continue from that original boundary.
            self.finish_turn(session_id, cwd, &pending.id)?;
            review = read_review(&dir)?.ok_or("Missing continued turn review")?;
        }
        // Work the previous turn left pending keeps its original contents in
        // this turn's baseline, so the review covers the session until it is
        // kept, undone or committed. Every other file starts fresh.
        let carried = if pending.is_none() {
            self.pending_baseline(session_id, cwd, &review)
        } else {
            Carried::default()
        };
        let mut turn = RecordedTurn {
            id: id.into(),
            before: pending.as_ref().and_then(|turn| turn.before.clone()),
            after: None,
            head: if let Some(pending) = &pending {
                pending.head.clone()
            } else {
                run_git(&root, &["rev-parse", "--verify", "HEAD"])
                    .ok()
                    .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_string())
            },
            shared: pending.as_ref().is_some_and(|turn| turn.shared) || carried.shared,
            owner: Some(TurnOwner::current()),
            error: None,
            dismissed: BTreeSet::new(),
            edited: pending
                .as_ref()
                .map(|turn| turn.edited.clone())
                .unwrap_or_else(|| carried.edited.clone()),
        };
        // Publish the pending state first. A failed baseline must not leave the
        // previous turn's card underneath the new response.
        review.active = Some(turn.clone());
        write_review(&dir, &review)?;
        let capture = (|| {
            review.repo = String::from_utf8(discovered?)
                .map_err(|e| e.to_string())?
                .trim_end_matches(['\r', '\n'])
                .to_string();
            // Shared checkouts can include another session's writes. Keep the
            // recorded diff exact, but never offer to restore those turns.
            for (other_dir, mut other) in self.active_turns(&root, session_id)? {
                turn.shared = true;
                if let Some(active) = other.active.as_mut() {
                    active.shared = true;
                }
                write_review(&other_dir, &other)?;
            }
            let reference = turn_ref(session_id, id, "before");
            if let Some(before) = &turn.before {
                run_git(&root, &["update-ref", &reference, before])?;
                Ok(before.clone())
            } else {
                let fresh = capture_workspace(&root, &reference)?;
                if carried.files.is_empty() {
                    Ok(fresh)
                } else {
                    overlay_baseline(&root, &fresh, &carried.files, &reference)
                }
            }
        })();
        match capture {
            Ok(oid) => turn.before = Some(oid),
            Err(error) => turn.error = Some(error),
        }
        review.active = Some(turn.clone());
        write_review(&dir, &review)?;
        match turn.error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub fn finish_turn(&self, session_id: &str, cwd: &str, id: &str) -> Result<(), String> {
        validate_id(session_id, "session")?;
        validate_id(id, "turn")?;
        let dir = self.session_dir(session_id);
        let Some(mut review) = self.matching_turn_review(session_id, cwd)? else {
            return Ok(());
        };
        let Some(mut turn) = review.active.take().filter(|turn| turn.id == id) else {
            return Ok(());
        };
        if turn.before.is_some() {
            match capture_workspace(Path::new(&review.cwd), &turn_ref(session_id, id, "after")) {
                Ok(oid) => turn.after = Some(oid),
                Err(error) => turn.error = Some(error),
            }
        }
        let error = turn.error.clone();
        write_json(
            &dir.join("turns").join(format!("{id}.json")),
            &SavedTurn {
                cwd: review.cwd.clone(),
                repo: review.repo.clone(),
                turn: turn.clone(),
            },
        )?;
        review.latest = Some(turn);
        write_review(&dir, &review)?;
        match error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// The previous turn's still-pending files with their contents before it,
    /// as its review shows them: not kept, undone or committed.
    pub(super) fn pending_baseline(
        &self,
        session_id: &str,
        cwd: &str,
        review: &TurnReview,
    ) -> Carried {
        let Some(previous) = review.latest.as_ref().filter(|_| review.active.is_none()) else {
            return Carried::default();
        };
        let Ok(Some(status)) = self.turn_status_inner(session_id, cwd, false) else {
            return Carried::default();
        };
        let pending: HashSet<_> = status.files.iter().map(|f| f.relative.as_str()).collect();
        let Ok(recorded) = changes(Path::new(&review.cwd), previous) else {
            return Carried::default();
        };
        let files: Vec<_> = recorded
            .into_iter()
            .filter(|change| pending.contains(change.relative.as_str()))
            .map(|change| (change.relative, change.before))
            .collect();
        Carried {
            shared: previous.shared && !files.is_empty(),
            edited: previous
                .edited
                .iter()
                .filter(|path| pending.contains(path.as_str()))
                .cloned()
                .collect(),
            files,
        }
    }

    pub(super) fn capture_turn_paths(
        &self,
        session_id: &str,
        cwd: &str,
        paths: &[String],
    ) -> Result<bool, String> {
        let Some(mut review) = self.matching_turn_review(session_id, cwd)? else {
            return Ok(false);
        };
        let Some(active) = review.active.as_mut() else {
            // Late tool events must not change the last completed turn.
            return Ok(true);
        };
        let workspace = project_root(cwd)?;
        let mut dirty = false;
        for path in paths {
            if let Ok(relative) = resolve_edit_path(&workspace, Path::new(&review.cwd), path) {
                dirty |= active.edited.insert(relative);
            }
        }
        if dirty {
            write_review(&self.session_dir(session_id), &review)?;
        }
        Ok(true)
    }

    /// Repair the interrupted-turn state left by versions that replaced its
    /// baseline on Continue. Only transcript-confirmed edits can be recovered.
    pub fn recover_orphaned_turn(
        &self,
        session_id: &str,
        cwd: &str,
        paths: &[String],
    ) -> Result<(), String> {
        if paths.is_empty() {
            return Ok(());
        }
        let Some(mut review) = self.matching_turn_review(session_id, cwd)? else {
            return Ok(());
        };
        if review.active.is_some() {
            return Ok(());
        }
        let turn = completed_turn(&review)?;
        let root = Path::new(&review.cwd);
        if !turn.edited.is_empty() || !changes(root, turn)?.is_empty() {
            return Ok(());
        }
        let workspace = project_root(cwd)?;
        let edited: BTreeSet<_> = paths
            .iter()
            .filter_map(|path| resolve_edit_path(&workspace, root, path).ok())
            .collect();
        if edited.is_empty() {
            return Ok(());
        }
        let prefix = format!("refs/monocode/checkpoints/{session_id}/");
        let refs = run_git(
            root,
            &[
                "for-each-ref",
                "--sort=-committerdate",
                "--format=%(refname) %(objectname)",
                &prefix,
            ],
        )?;
        let refs: Vec<_> = String::from_utf8_lossy(&refs)
            .lines()
            .filter_map(|line| {
                line.split_once(' ')
                    .map(|(name, oid)| (name.to_string(), oid.to_string()))
            })
            .collect();
        let names: HashSet<_> = refs.iter().map(|(name, _)| name.as_str()).collect();
        let dir = self.session_dir(session_id);
        for (reference, before) in &refs {
            let Some(id) = reference
                .strip_prefix(&prefix)
                .and_then(|name| name.strip_suffix("/before"))
            else {
                continue;
            };
            if id == turn.id
                || names.contains(turn_ref(session_id, id, "after").as_str())
                || dir.join("turns").join(format!("{id}.json")).exists()
            {
                continue;
            }
            validate_id(id, "turn")?;
            let mut recovered = turn.clone();
            recovered.before = Some(before.clone());
            recovered.edited = edited.clone();
            // The orphan lost its original HEAD and overlap metadata. Review
            // remains available, but recovery must never make Undo available.
            recovered.head = None;
            recovered.shared = true;
            if !changes(root, &recovered)?
                .iter()
                .any(|change| edited.contains(&change.relative))
            {
                continue;
            }
            let mut archived = recovered.clone();
            archived.id = id.into();
            for saved in [archived, recovered.clone()] {
                write_json(
                    &dir.join("turns").join(format!("{}.json", saved.id)),
                    &SavedTurn {
                        cwd: review.cwd.clone(),
                        repo: review.repo.clone(),
                        turn: saved,
                    },
                )?;
            }
            review.latest = Some(recovered);
            return write_review(&dir, &review);
        }
        Ok(())
    }

    pub(super) fn matching_turn_review(
        &self,
        session_id: &str,
        cwd: &str,
    ) -> Result<Option<TurnReview>, String> {
        let Some(review) = read_review(&self.session_dir(session_id))? else {
            return Ok(None);
        };
        let root = std::fs::canonicalize(project_root(cwd)?).map_err(|e| e.to_string())?;
        Ok((review.cwd == path_to_js(&root)).then_some(review))
    }

    pub(super) fn active_turns(
        &self,
        root: &Path,
        except: &str,
    ) -> Result<Vec<(PathBuf, TurnReview)>, String> {
        let root = review_comparison_path(root);
        let mut active = Vec::new();
        if !self.root.exists() {
            return Ok(active);
        }
        for entry in std::fs::read_dir(&self.root).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_name() == except {
                continue;
            }
            if let Some(review) = read_review(&entry.path())? {
                let other = review_comparison_path(Path::new(&review.cwd));
                // A turn whose process exited never finishes; it must not
                // mark this checkout's later turns as shared.
                let running = review
                    .active
                    .as_ref()
                    .is_some_and(|turn| turn.owner.as_ref().is_some_and(TurnOwner::running));
                if running && (root.starts_with(&other) || other.starts_with(&root)) {
                    active.push((entry.path(), review));
                }
            }
        }
        Ok(active)
    }

    pub(super) fn undo_turn(
        &self,
        session_id: &str,
        cwd: &str,
        relative: Option<&str>,
    ) -> Result<Option<CheckpointStatus>, String> {
        let Some(review) = self.matching_turn_review(session_id, cwd)? else {
            return Ok(None);
        };
        let status = self.turn_status(session_id, cwd)?.unwrap();
        let root = Path::new(&review.cwd);
        let relative = relative.map(resolve_turn_path).transpose()?;
        let selected: Vec<_> = status
            .files
            .iter()
            .filter(|file| {
                relative
                    .as_ref()
                    .is_none_or(|relative| *relative == file.relative)
            })
            .collect();
        if selected.iter().any(|file| !file.undoable) {
            return Err("Cannot safely undo this turn: the workspace was shared, the branch moved, or a recorded file changed afterward".into());
        }
        let turn = completed_turn(&review)?;
        let selected: HashSet<_> = selected.iter().map(|file| file.relative.as_str()).collect();
        let changes: Vec<_> = changes(root, turn)?
            .into_iter()
            .filter(|change| selected.contains(change.relative.as_str()))
            .collect();
        // Validate all paths again before restoring any, preserving later edits
        // and refusing symlink traversal. The user's Git index is never reset.
        if changes.iter().any(|change| !restorable(root, change)) {
            return Err("Cannot safely undo: a recorded file changed after the checkpoint".into());
        }
        for change in changes {
            if change.before.is_some() {
                run_git(
                    root,
                    &[
                        "restore",
                        "--source",
                        turn.before.as_deref().unwrap(),
                        "--worktree",
                        "--",
                        &change.relative,
                    ],
                )?;
            } else {
                std::fs::remove_file(root.join(&change.relative)).map_err(|e| e.to_string())?;
            }
        }
        self.keep_turn(session_id, cwd, relative.as_deref())
    }

    pub(super) fn forget_turns(&self, session_id: &str) -> Result<(), String> {
        let dir = self.session_dir(session_id);
        let mut repos = BTreeSet::new();
        if let Some(review) = read_review(&dir)? {
            repos.insert(review.repo);
        }
        let history = dir.join("turns");
        if history.exists() {
            for entry in std::fs::read_dir(history).map_err(|e| e.to_string())? {
                let saved: SavedTurn = serde_json::from_slice(
                    &std::fs::read(entry.map_err(|e| e.to_string())?.path())
                        .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                repos.insert(saved.repo);
            }
        }
        let prefix = format!("refs/monocode/checkpoints/{session_id}/");
        for repo in repos {
            if !Path::new(&repo).is_dir() {
                continue;
            }
            let refs = run_git(
                Path::new(&repo),
                &["for-each-ref", "--format=%(refname)", &prefix],
            )?;
            for reference in String::from_utf8_lossy(&refs)
                .lines()
                .filter(|reference| reference.starts_with(&prefix))
            {
                run_git(Path::new(&repo), &["update-ref", "-d", reference])?;
            }
        }
        Ok(())
    }
}
