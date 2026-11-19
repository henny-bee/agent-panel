use super::*;

impl Explorer {
    pub(super) fn read_dir(&mut self, path: String, cx: &mut Context<Self>) {
        if !self.reading.insert(path.clone()) {
            return;
        }
        let dir = PathBuf::from(&path);
        let work = cx
            .background_executor()
            .spawn(async move { files::list_dir(&dir) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.reading.remove(&path);
                if this.dirs.get(&path) != Some(&result) {
                    this.dirs.insert(path, result);
                    this.rebuild(cx);
                }
            });
        })
        .detach();
    }

    /// Rereads every folder shown, keeping the current rows meanwhile.
    pub(super) fn refresh_loaded(&mut self, cx: &mut Context<Self>) {
        let mut shown: Vec<String> = self
            .expanded
            .iter()
            .filter(|p| *p == &self.cwd || self.dirs.contains_key(&parent_of(p)))
            .cloned()
            .collect();
        if !shown.contains(&self.cwd) {
            shown.push(self.cwd.clone());
        }
        for path in shown {
            self.read_dir(path, cx);
        }
    }

    pub(super) fn forget(&mut self, path: &str) {
        let prefix = format!("{path}/");
        self.dirs
            .retain(|k, _| k != path && !k.starts_with(&prefix));
    }

    pub(super) fn is_dir_at(&self, path: &str) -> bool {
        if path == self.cwd {
            return true;
        }
        match self.dirs.get(&parent_of(path)) {
            Some(Ok(entries)) => entries
                .iter()
                .find(|e| e.path == path)
                .map(|e| e.is_dir)
                .unwrap_or_else(|| self.dirs.contains_key(path)),
            _ => self.dirs.contains_key(path),
        }
    }

    pub(super) fn create_parent(&self, at: Option<&str>) -> String {
        files::create_parent_of(&self.cwd, at, |p| self.is_dir_at(p))
    }

    pub(super) fn siblings(&self, parent: &str, except: Option<&str>) -> Vec<String> {
        match self.dirs.get(parent) {
            Some(Ok(entries)) => entries
                .iter()
                .map(|e| e.name.clone())
                .filter(|n| Some(n.as_str()) != except)
                .collect(),
            _ => Vec::new(),
        }
    }

    pub(super) fn issue(&self, cx: &App) -> Option<files::NameIssue> {
        let value = self.name_input.read(cx).content.to_string();
        let siblings = match self.naming.as_ref()? {
            NameMode::Create { parent, .. } => self.siblings(parent, None),
            NameMode::Rename { path, .. } => self.siblings(&parent_of(path), Some(basename(path))),
        };
        files::validate_file_name(&value, &siblings)
    }

    pub(super) fn rebuild(&mut self, cx: &mut Context<Self>) {
        let mut lines = Vec::new();
        if self.available() && self.expanded.contains(&self.cwd) {
            self.push_children(&self.cwd.clone(), 0, &mut lines, cx);
        }
        // Splice only the rows that changed. Replacing the whole range makes
        // ListState reset its scroll top to row 0 on every toggle/refresh.
        let (start, old_end, new_end) = changed_range(&self.lines, &lines);
        self.lines = lines;
        if start < old_end || start < new_end {
            self.list.splice(start..old_end, new_end - start);
        }
        cx.notify();
    }

    pub(super) fn push_children(
        &self,
        parent: &str,
        depth: usize,
        lines: &mut Vec<Line>,
        cx: &App,
    ) {
        let listing = self.dirs.get(parent);
        let creating = match &self.naming {
            Some(NameMode::Create {
                parent: p, is_dir, ..
            }) if p == parent => Some(*is_dir),
            _ => None,
        };
        if let Some(Err(error)) = listing {
            lines.push(Line::Note {
                depth,
                text: error.clone(),
            });
        }
        if creating == Some(true) {
            self.push_name(depth, true, lines, cx);
        }
        if listing.is_none() {
            lines.push(Line::Note {
                depth,
                text: "…".into(),
            });
        }
        let entries: Vec<&DirEntry> = match listing {
            Some(Ok(entries)) => entries
                .iter()
                .filter(|e| self.show_excluded || !e.ignored)
                .collect(),
            _ => Vec::new(),
        };
        for entry in entries.iter().filter(|e| e.is_dir) {
            self.push_entry(entry, depth, lines, cx);
        }
        if creating == Some(false) {
            self.push_name(depth, false, lines, cx);
        }
        for entry in entries.iter().filter(|e| !e.is_dir) {
            self.push_entry(entry, depth, lines, cx);
        }
    }

    pub(super) fn push_entry(
        &self,
        entry: &DirEntry,
        depth: usize,
        lines: &mut Vec<Line>,
        cx: &App,
    ) {
        let open = entry.is_dir && self.expanded.contains(&entry.path);
        if matches!(&self.naming, Some(NameMode::Rename { path, .. }) if path == &entry.path) {
            self.push_name(depth, entry.is_dir, lines, cx);
        } else {
            lines.push(Line::Entry {
                entry: entry.clone(),
                depth,
                open,
            });
        }
        if open {
            self.push_children(&entry.path, depth + 1, lines, cx);
        }
    }

    /// NameRow and its NameIssueView.
    pub(super) fn push_name(&self, depth: usize, is_dir: bool, lines: &mut Vec<Line>, cx: &App) {
        lines.push(Line::Name { depth, is_dir });
        let value = self.name_input.read(cx).content.to_string();
        let issue = self.issue(cx);
        let show = self.name_error.is_some()
            || issue.as_ref().is_some_and(|i| {
                i.severity == Severity::Warning
                    || (i.kind != NameIssueKind::Empty && !value.is_empty())
                    || (i.kind == NameIssueKind::Empty && self.name_attempted)
            });
        if !show {
            return;
        }
        let text: Vec<(String, bool)> = if let Some(error) = &self.name_error {
            vec![(error.clone(), false)]
        } else if let Some(issue) = &issue {
            match issue.kind {
                NameIssueKind::Empty => {
                    vec![("A file or folder name must be provided.".into(), false)]
                }
                NameIssueKind::Slash => {
                    vec![(
                        "A file or folder name cannot start with a slash.".into(),
                        false,
                    )]
                }
                NameIssueKind::Exists => vec![
                    ("A file or folder ".into(), false),
                    (issue.name.clone(), true),
                    (
                        " already exists at this location. Please choose a different name.".into(),
                        false,
                    ),
                ],
                NameIssueKind::Invalid => vec![
                    ("The name ".into(), false),
                    (issue.name.clone(), true),
                    (
                        " is not valid as a file or folder name. Please choose a different name."
                            .into(),
                        false,
                    ),
                ],
                NameIssueKind::Whitespace => vec![(
                    "Leading or trailing whitespace detected in file or folder name.".into(),
                    false,
                )],
            }
        } else {
            return;
        };
        let error = self.name_error.is_some()
            || issue.as_ref().is_none_or(|i| i.severity == Severity::Error);
        lines.push(Line::Issue { depth, text, error });
    }

    pub(super) fn select(&mut self, path: String, cx: &mut Context<Self>) {
        self.selected = Some(path);
        cx.notify();
    }

    pub(super) fn toggle(&mut self, path: &str, cx: &mut Context<Self>) {
        if !self.expanded.remove(path) {
            self.expanded.insert(path.to_owned());
            if !self.dirs.contains_key(path) {
                self.read_dir(path.to_owned(), cx);
            }
        }
        self.rebuild(cx);
    }

    pub(super) fn expand(&mut self, dirs: &[String], cx: &mut Context<Self>) {
        for dir in dirs {
            if self.expanded.insert(dir.clone()) && !self.dirs.contains_key(dir) {
                self.read_dir(dir.clone(), cx);
            }
        }
    }

    pub(super) fn collapse_all(&mut self, cx: &mut Context<Self>) {
        self.naming = None;
        self.expanded = HashSet::from([self.cwd.clone()]);
        self.rebuild(cx);
    }

    /// The rows keyboard navigation walks: the root, then each entry shown.
    pub(super) fn nav_rows(&self) -> Vec<(String, bool)> {
        let mut rows = vec![(self.cwd.clone(), true)];
        rows.extend(self.lines.iter().filter_map(|l| match l {
            Line::Entry { entry, .. } => Some((entry.path.clone(), entry.is_dir)),
            _ => None,
        }));
        rows
    }

    pub(super) fn focus_row(&mut self, path: String, cx: &mut Context<Self>) {
        self.keyboard = true;
        if let Some(ix) = self
            .lines
            .iter()
            .position(|l| matches!(l, Line::Entry { entry, .. } if entry.path == path))
        {
            self.list.scroll_to_reveal_item(ix);
        }
        self.select(path, cx);
    }

    pub(super) fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let rows = self.nav_rows();
        let current = self.selected.clone().unwrap_or_else(|| self.cwd.clone());
        let ix = rows.iter().position(|(p, _)| *p == current);
        let next = match ix {
            None if delta > 0 => 0,
            None => rows.len() - 1,
            Some(ix) => (ix as isize + delta).clamp(0, rows.len() as isize - 1) as usize,
        };
        if let Some((path, _)) = rows.get(next).cloned() {
            self.focus_row(path, cx);
        }
    }

    pub(super) fn page(&self, window: &Window) -> isize {
        let height = self.list.viewport_bounds().size.height.as_f32();
        let _ = window;
        ((height / ROW).floor() as isize - 1).max(1)
    }

    pub(super) fn current(&self) -> (String, bool) {
        let path = self.selected.clone().unwrap_or_else(|| self.cwd.clone());
        let is_dir = self.is_dir_at(&path);
        (path, is_dir)
    }

    pub(super) fn activate(&mut self, cx: &mut Context<Self>) {
        let (path, is_dir) = self.current();
        if is_dir {
            self.toggle(&path, cx);
        } else {
            cx.emit(OpenFile(path));
        }
    }

    pub(super) fn on_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus.is_focused(window) {
            return;
        }
        let keystroke = &event.keystroke;
        let m = &keystroke.modifiers;
        if m.control || m.alt || m.platform || m.function {
            return;
        }
        if keystroke.key == "space" {
            self.activate(cx);
            cx.stop_propagation();
            return;
        }
        // Type a name prefix to jump to the next matching row; repeating one
        // letter cycles through its matches.
        let Some(ch) = keystroke
            .key_char
            .as_ref()
            .filter(|c| c.chars().count() == 1)
        else {
            return;
        };
        let now = Instant::now();
        let fresh = self
            .typeahead
            .1
            .is_none_or(|at| now.duration_since(at).as_millis() > 700);
        if fresh {
            self.typeahead.0 = ch.clone();
        } else {
            self.typeahead.0.push_str(ch);
        }
        self.typeahead.1 = Some(now);
        let lower = self.typeahead.0.to_lowercase();
        let first = lower.chars().next().unwrap_or_default();
        let cycling = lower.chars().all(|c| c == first);
        let needle = if cycling { first.to_string() } else { lower };
        let rows = self.nav_rows();
        let current = self.selected.clone().unwrap_or_else(|| self.cwd.clone());
        let ix = rows.iter().position(|(p, _)| *p == current);
        let start = if cycling {
            ix.map_or(0, |i| i + 1)
        } else {
            ix.unwrap_or(0)
        };
        let ordered = rows[start.min(rows.len())..]
            .iter()
            .chain(&rows[..start.min(rows.len())]);
        if let Some((path, _)) = ordered
            .filter(|(p, _)| *p != self.cwd)
            .find(|(p, _)| basename(p).to_lowercase().starts_with(&needle))
            .cloned()
        {
            self.focus_row(path, cx);
        }
        cx.stop_propagation();
    }
}
