use super::*;

impl Explorer {
    pub(super) fn start_create(
        &mut self,
        is_dir: bool,
        at: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let parent = self.create_parent(at.as_deref().or(self.selected.as_deref()));
        self.next_id += 1;
        self.expand(&[self.cwd.clone(), parent.clone()], cx);
        self.begin_name(
            NameMode::Create {
                id: self.next_id,
                parent,
                is_dir,
            },
            String::new(),
            window,
            cx,
        );
    }

    pub(super) fn start_rename(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if path == self.cwd {
            return;
        }
        self.menu = None;
        let is_dir = self.is_dir_at(&path);
        let name = basename(&path).to_owned();
        self.selected = Some(path.clone());
        self.begin_name(NameMode::Rename { path, is_dir }, name.clone(), window, cx);
        // Files select their stem, folders the whole name.
        let end = if is_dir {
            name.len()
        } else {
            name.rfind('.').filter(|&d| d > 0).unwrap_or(name.len())
        };
        self.name_input
            .update(cx, |i, cx| i.select_range(0..end, cx));
    }

    pub(super) fn begin_name(
        &mut self,
        mode: NameMode,
        initial: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.naming = Some(mode);
        self.name_attempted = false;
        self.name_busy = false;
        self.name_error = None;
        self.name_input.update(cx, |i, cx| i.set_text(initial, cx));
        let focus = self.name_input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.rebuild(cx);
        if let Some(ix) = self
            .lines
            .iter()
            .position(|l| matches!(l, Line::Name { .. }))
        {
            self.list.scroll_to_reveal_item(ix);
        }
    }

    pub(super) fn cancel_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.naming = None;
        window.focus(&self.focus, cx);
        self.rebuild(cx);
    }

    pub(super) fn finish_name(
        &mut self,
        commit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mode) = self.naming.clone() else {
            return;
        };
        if self.name_busy {
            return;
        }
        if !commit {
            self.cancel_name(window, cx);
            return;
        }
        if self
            .issue(cx)
            .is_some_and(|i| i.severity == Severity::Error)
        {
            self.name_attempted = true;
            self.rebuild(cx);
            return;
        }
        let raw = self.name_input.read(cx).content.to_string();
        let name = files::well_formed_file_name(&raw);
        let cwd = self.cwd.clone();
        match mode {
            NameMode::Rename { path, is_dir } => {
                if name.is_empty() || (name == basename(&path) && !raw.contains(['/', '\\'])) {
                    self.cancel_name(window, cx);
                    return;
                }
                self.name_busy = true;
                let from = path.clone();
                let work = cx.background_executor().spawn({
                    let name = name.clone();
                    async move { files::rename_path(Path::new(&from), &name) }
                });
                cx.spawn_in(window, async move |this, cx| {
                    let result = work.await;
                    let _ = this.update_in(cx, |this, window, cx| match result {
                        Ok(next) => {
                            let parent = parent_of(&path);
                            if is_dir {
                                this.forget(&path);
                            }
                            let touched = files::dirs_touched_by_create(&parent, &name);
                            for dir in touched.iter().chain([&parent]) {
                                this.read_dir(dir.clone(), cx);
                            }
                            this.naming = None;
                            this.expand(&touched, cx);
                            this.remap(&path, &next);
                            this.after_op(window, cx);
                        }
                        Err(error) => this.name_failed(error, cx),
                    });
                })
                .detach();
            }
            NameMode::Create { id, parent, is_dir } => {
                let as_folder = is_dir || raw.ends_with(['/', '\\']);
                self.name_busy = true;
                let at = parent.clone();
                let work = cx.background_executor().spawn({
                    let name = name.clone();
                    async move { files::create_path(Path::new(&at), &name, as_folder) }
                });
                cx.spawn_in(window, async move |this, cx| {
                    let result = work.await;
                    let _ = this.update_in(cx, |this, window, cx| match result {
                        Ok(created) => {
                            let touched = files::dirs_touched_by_create(&parent, &name);
                            for dir in &touched {
                                this.read_dir(dir.clone(), cx);
                            }
                            if matches!(&this.naming, Some(NameMode::Create { id: i, .. }) if *i == id) {
                                this.naming = None;
                            }
                            this.expand(&touched, cx);
                            this.selected = Some(created);
                            let _ = cwd;
                            this.after_op(window, cx);
                        }
                        Err(error) => this.name_failed(error, cx),
                    });
                })
                .detach();
            }
        }
    }

    pub(super) fn name_failed(&mut self, error: String, cx: &mut Context<Self>) {
        self.name_busy = false;
        self.name_error = Some(error);
        self.rebuild(cx);
    }

    pub(super) fn after_op(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.name_busy = false;
        window.focus(&self.focus, cx);
        self.git.update(cx, |git, cx| git.changed(cx));
        self.rebuild(cx);
    }

    pub(super) fn remap(&mut self, from: &str, to: &str) {
        self.expanded = self
            .expanded
            .iter()
            .map(|p| files::rebase_path(p, from, to))
            .collect();
        if let Some(selected) = &self.selected {
            self.selected = Some(files::rebase_path(selected, from, to));
        }
        if let Some(clip) = &mut self.clip
            && (clip.path == from || clip.path.starts_with(&format!("{from}/")))
        {
            clip.path = files::rebase_path(&clip.path, from, to);
        }
    }

    pub(super) fn set_operation_error(&mut self, error: Option<String>, cx: &mut Context<Self>) {
        self.op_error = error.clone();
        cx.emit(FileActionError(error));
    }

    pub(super) fn fail(&mut self, error: String, cx: &mut Context<Self>) {
        self.set_operation_error(Some(error), cx);
        self.rebuild(cx);
    }

    /// `run`: operation errors route to the shared toast until the next action.
    pub(super) fn run(
        &mut self,
        work: impl FnOnce() -> Result<Option<String>, String> + Send + 'static,
        touched: Vec<String>,
        forget: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_operation_error(None, cx);
        let task = cx.background_executor().spawn(async move { work() });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(created) => {
                    if let Some(path) = &forget {
                        this.forget(path);
                    }
                    for dir in &touched {
                        this.read_dir(dir.clone(), cx);
                    }
                    this.expand(&touched, cx);
                    if let Some(created) = created {
                        this.selected = Some(created);
                    }
                    this.after_op(window, cx);
                }
                Err(error) => this.fail(error, cx),
            });
        })
        .detach();
    }

    pub(super) fn delete(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        if path == self.cwd {
            return;
        }
        let is_dir = self.is_dir_at(&path);
        let label = basename(&path).to_owned();
        let answer = window.prompt(
            PromptLevel::Warning,
            &if is_dir {
                format!("Delete folder “{label}” and everything inside it?")
            } else {
                format!("Delete “{label}”?")
            },
            None,
            &["OK", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if !matches!(answer.await, Ok(0)) {
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| {
                let parent = parent_of(&path);
                let target = path.clone();
                this.selected = match &this.selected {
                    Some(s) if s == &path || s.starts_with(&format!("{path}/")) => {
                        Some(parent.clone())
                    }
                    other => other.clone(),
                };
                if this
                    .clip
                    .as_ref()
                    .is_some_and(|c| c.path == path || c.path.starts_with(&format!("{path}/")))
                {
                    this.clip = None;
                }
                this.run(
                    move || files::delete_path(Path::new(&target)).map(|_| None),
                    vec![parent],
                    is_dir.then_some(path),
                    window,
                    cx,
                );
            });
        })
        .detach();
    }

    pub(super) fn paste(&mut self, at: String, window: &mut Window, cx: &mut Context<Self>) {
        let dest = self.create_parent(Some(&at));
        let Some(clip) = self.clip.clone() else {
            // clipboardFilePaths: files copied in Finder.
            let paths: Vec<PathBuf> = cx
                .read_from_clipboard()
                .map(|item| {
                    item.entries()
                        .iter()
                        .filter_map(|e| match e {
                            ClipboardEntry::ExternalPaths(p) => Some(p.paths().to_vec()),
                            _ => None,
                        })
                        .flatten()
                        .collect()
                })
                .unwrap_or_default();
            self.copy_external(paths, dest, window, cx);
            return;
        };
        if clip.is_dir && (dest == clip.path || dest.starts_with(&format!("{}/", clip.path))) {
            self.fail("Cannot paste a folder into itself.".into(), cx);
            return;
        }
        let from = clip.path.clone();
        if clip.cut {
            let to_parent = dest.clone();
            let task = cx
                .background_executor()
                .spawn(async move { files::move_path(Path::new(&from), Path::new(&to_parent)) });
            let from = clip.path.clone();
            cx.spawn_in(window, async move |this, cx| {
                let result = task.await;
                let _ = this.update_in(cx, |this, window, cx| match result {
                    Ok(created) => {
                        if clip.is_dir {
                            this.forget(&from);
                        }
                        for dir in files::dirs_touched_by_move(&from, &created) {
                            this.read_dir(dir, cx);
                        }
                        this.remap(&from, &created);
                        this.clip = None;
                        this.expand(std::slice::from_ref(&dest), cx);
                        this.selected = Some(created);
                        this.after_op(window, cx);
                    }
                    Err(error) => this.fail(error, cx),
                });
            })
            .detach();
        } else {
            let to = dest.clone();
            self.run(
                move || files::copy_path(Path::new(&from), Path::new(&to)).map(Some),
                vec![dest],
                None,
                window,
                cx,
            );
        }
    }

    pub(super) fn copy_external(
        &mut self,
        paths: Vec<PathBuf>,
        dest: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() {
            return;
        }
        let to = dest.clone();
        self.run(
            move || {
                let mut created = None;
                for from in &paths {
                    created = Some(files::copy_path(from, Path::new(&to))?);
                }
                Ok(created)
            },
            vec![dest],
            None,
            window,
            cx,
        );
    }

    pub(super) fn duplicate(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        if path == self.cwd {
            return;
        }
        let parent = parent_of(&path);
        let to = parent.clone();
        self.run(
            move || files::copy_path(Path::new(&path), Path::new(&to)).map(Some),
            vec![parent],
            None,
            window,
            cx,
        );
    }

    pub(super) fn reveal(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.run(
            move || files::reveal_path(Path::new(&path)).map(|_| None),
            Vec::new(),
            None,
            window,
            cx,
        );
    }

    pub(super) fn open_menu(
        &mut self,
        target: Target,
        at: Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.naming = None;
        self.selected = Some(target.path.clone());
        let paste_parent = if target.is_dir {
            target.path.clone()
        } else {
            parent_of(&target.path)
        };
        let paste_blocked = self.clip.as_ref().is_some_and(|c| {
            c.is_dir
                && (paste_parent == c.path || paste_parent.starts_with(&format!("{}/", c.path)))
        });
        let root = target.root;
        let items: Vec<MenuItem> = vec![
            MenuAction::new("new-file", "New File").into(),
            MenuAction::new("new-folder", "New Folder").into(),
            MenuItem::Separator,
            MenuAction::new("cut", "Cut")
                .shortcut(format!("{MOD}X"))
                .disabled(root)
                .into(),
            MenuAction::new("copy", "Copy")
                .shortcut(format!("{MOD}C"))
                .disabled(root)
                .into(),
            MenuAction::new("paste", "Paste")
                .shortcut(format!("{MOD}V"))
                .disabled(paste_blocked)
                .into(),
            MenuAction::new("duplicate", "Duplicate")
                .disabled(root)
                .into(),
            MenuItem::Separator,
            MenuAction::new("copy-path", "Copy Path")
                .shortcut(format!("{MOD}{SHIFT}C"))
                .into(),
            MenuAction::new("copy-relative-path", "Copy Relative Path").into(),
            MenuItem::Separator,
            MenuAction::new("rename", "Rename")
                .shortcut("F2")
                .disabled(root)
                .into(),
            MenuAction::new("delete", "Delete")
                .shortcut("⌫")
                .disabled(root)
                .danger()
                .into(),
            MenuItem::Separator,
            MenuAction::new("reveal", REVEAL_LABEL).into(),
        ];
        self.menu = None;
        let menu = cx.new(|cx| Menu::new("File actions", items, window, cx));
        let events = cx.subscribe_in(&menu, window, |this, _, event: &MenuEvent, window, cx| {
            let Some(open) = this.menu.take() else {
                return;
            };
            if let MenuEvent::Picked(id, _) = event {
                this.run_action(id, open.target, window, cx);
            }
            cx.notify();
        });
        self.menu = Some(OpenMenu {
            target,
            anchor: at,
            menu,
            _events: events,
        });
        self.rebuild(cx);
    }

    pub(super) fn run_action(
        &mut self,
        id: &str,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match id {
            "new-file" => self.start_create(false, Some(target.path), window, cx),
            "new-folder" => self.start_create(true, Some(target.path), window, cx),
            "cut" | "copy" if !target.root => {
                self.clip = Some(Clip {
                    cut: id == "cut",
                    path: target.path,
                    is_dir: target.is_dir,
                });
                self.rebuild(cx);
            }
            "paste" => self.paste(target.path, window, cx),
            "duplicate" => self.duplicate(target.path, window, cx),
            "copy-path" => cx.write_to_clipboard(ClipboardItem::new_string(target.path)),
            "copy-relative-path" => cx.write_to_clipboard(ClipboardItem::new_string(
                files::display_path(&target.path, Some(&self.cwd)),
            )),
            "rename" => self.start_rename(target.path, window, cx),
            "delete" => self.delete(target.path, window, cx),
            "reveal" => self.reveal(target.path, window, cx),
            _ => {}
        }
    }
}
