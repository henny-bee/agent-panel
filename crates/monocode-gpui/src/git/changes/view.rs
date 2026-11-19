use super::*;

impl Changes {
    pub(super) fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let index = self.git.read(cx).index.clone();
        let busy = self.busy.is_some();
        let menu_open = self
            .menu
            .as_ref()
            .is_some_and(|m| m.kind == MenuKind::Branch);
        let branch = index.as_ref().and_then(|i| i.branch.clone());
        let index = index.unwrap_or_default();
        let picker_open = self.branch_picker.is_some();
        pane_header("changes-header", "Changes")
            .when_some(branch.clone(), |d, branch| {
                d.child(
                    div()
                        .id("branch-switch")
                        .debug_selector(|| "branch-switch".into())
                        .role(Role::Button)
                        .aria_label("Switch branch")
                        .aria_expanded(picker_open)
                        .relative()
                        .child(self.name_anchor.probe())
                        .min_w_0()
                        // The padding shows the hover; the margin keeps the text in place.
                        .mx(px(-4.))
                        .px(px(4.))
                        .py(px(2.))
                        .rounded_md()
                        .cursor_pointer()
                        .when(picker_open, |d| d.bg(content(0.1)))
                        .when(!picker_open, |d| d.hover(|s| s.bg(content(0.05))))
                        .on_click(
                            cx.listener(|this, _, window, cx| {
                                this.toggle_branch_picker(window, cx)
                            }),
                        )
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(icon("git-branch-hi", 12.).text_color(content(0.5)))
                        .child(div().min_w_0().truncate().child(branch))
                        .when(index.ahead > 0, |d| {
                            d.child(
                                div()
                                    .flex_shrink_0()
                                    .font_features(tabular())
                                    .text_size(px(11.))
                                    .text_color(content(0.4))
                                    .child(format!("↑{}", index.ahead)),
                            )
                        })
                        .when(index.behind > 0, |d| {
                            d.child(
                                div()
                                    .flex_shrink_0()
                                    .font_features(tabular())
                                    .text_size(px(11.))
                                    .text_color(content(0.4))
                                    .child(format!("↓{}", index.behind)),
                            )
                        }),
                )
            })
            .when(branch.is_none(), |d| {
                d.child(div().flex_shrink_0().child("Changes"))
            })
            .when_some(self.status.clone(), |d, status| {
                d.child(
                    div()
                        .id("changes-status")
                        .role(Role::Status)
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(content(0.5))
                        .child(status),
                )
            })
            .child(div().ml_auto())
            .when(branch.is_some(), |d| {
                d.child(
                    div()
                        .id("branch-actions")
                        .role(Role::Button)
                        .aria_label("Branch actions")
                        .aria_expanded(menu_open)
                        .group("branch-actions")
                        .relative()
                        .size(px(24.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_md()
                        .child(self.branch_anchor.probe())
                        .when(busy, |d| d.opacity(0.4))
                        .when(menu_open, |d| d.bg(content(0.1)))
                        .when(!busy && !menu_open, |d| d.hover(|s| s.bg(content(0.05))))
                        .child(if self.busy.as_deref() == Some("pull") {
                            spinner("pull-spin", "loader", 14.)
                        } else {
                            icon("more-horizontal", 16.)
                                .text_color(content(if menu_open { 1. } else { 0.5 }))
                                .group_hover("branch-actions", |s| s.text_color(content(1.)))
                                .into_any_element()
                        })
                        .when(!busy, |d| {
                            d.on_click(cx.listener(|this, _, window, cx| {
                                this.open_menu(MenuKind::Branch, window, cx)
                            }))
                        }),
                )
            })
    }

    pub(super) fn commit_box(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let index = self.git.read(cx).index.clone();
        let has_files = index.as_ref().is_some_and(|i| !i.files.is_empty());
        let generating = self.busy.as_deref() == Some("generate");
        let can_generate = has_files && self.busy.is_none();
        let can_commit = self.can_commit(cx);
        let amend = self.amend.is_some();
        let can_open_menu =
            index.as_ref().is_some_and(|i| i.branch.is_some()) && self.busy.is_none();
        let menu_open = self
            .menu
            .as_ref()
            .is_some_and(|m| m.kind == MenuKind::Commit);
        let placeholder = if amend {
            format!("Amend message ({MOD}↩ to amend)")
        } else {
            format!("Message ({MOD}↩ to commit)")
        };
        self.message
            .update(cx, |m, cx| m.set_placeholder(placeholder, cx));
        // `bg-content` buttons with `text-background-base`, 40% when unavailable.
        let solid: Hsla = content(1.);
        let face = if can_commit { solid } else { content(0.4) };
        let ink = crate::ui::theme::background_base();
        let generate_label = if generating {
            "Cancel commit message generation"
        } else {
            "Generate commit message"
        };
        div()
            .flex_shrink_0()
            .border_b_1()
            .border_color(stroke())
            .p_2()
            .child(
                div()
                    .relative()
                    .child(
                        div()
                            .w_full()
                            .rounded_md()
                            .bg(content(0.1))
                            .py_1()
                            .pl_2()
                            .pr_8()
                            .text_size(px(13.))
                            .line_height(px(20.))
                            .text_color(content(1.))
                            .when(self.busy.is_some(), |d| d.opacity(0.4))
                            .child(self.message.clone()),
                    )
                    .child(
                        div()
                            .id("generate-commit-message")
                            .role(Role::Button)
                            .aria_label(generate_label)
                            .group("generate")
                            .absolute()
                            .top_1()
                            .right_1()
                            .size(px(20.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .bg(content(0.1))
                            .when(!generating && !can_generate, |d| d.opacity(0.4))
                            .when(generating || can_generate, |d| {
                                d.hover(|s| s.bg(content(0.2)))
                            })
                            .child(if generating {
                                div()
                                    .relative()
                                    .size(px(14.))
                                    .child(
                                        div()
                                            .absolute()
                                            .group_hover("generate", |s| s.opacity(0.))
                                            .child(spinner("generate-spin", "loader", 14.)),
                                    )
                                    .child(
                                        icon("x", 14.)
                                            .absolute()
                                            .text_color(content(1.))
                                            .opacity(0.)
                                            .group_hover("generate", |s| s.opacity(1.)),
                                    )
                                    .into_any_element()
                            } else {
                                icon("wand", 12.).text_color(content(1.)).into_any_element()
                            })
                            .when(generating || can_generate, |d| {
                                d.on_click(cx.listener(move |this, _, window, cx| {
                                    if generating {
                                        this.cancel_generate();
                                        cx.notify();
                                    } else {
                                        this.generate(window, cx);
                                    }
                                }))
                            }),
                    ),
            )
            .child(
                div()
                    .relative()
                    .mt(px(6.))
                    .flex()
                    .child(self.commit_anchor.probe())
                    .child(
                        div()
                            .id("commit")
                            .role(Role::Button)
                            .aria_label(if amend { "Amend Commit" } else { "Commit" })
                            .h(px(28.))
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap(px(6.))
                            .rounded_l_md()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .bg(face)
                            .text_color(ink)
                            .child(icon("check-2", 14.).text_color(ink))
                            .child(if amend { "Amend Commit" } else { "Commit" })
                            .when(can_commit, |d| {
                                d.on_click(cx.listener(|this, _, window, cx| {
                                    this.commit(false, false, window, cx)
                                }))
                            }),
                    )
                    .child(
                        div()
                            .id("commit-options")
                            .role(Role::Button)
                            .aria_label("Commit options")
                            .aria_expanded(menu_open)
                            .size(px(28.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_r_md()
                            .border_l_1()
                            .border_color(ink.opacity(0.1))
                            .bg(if menu_open { solid } else { face })
                            .when(can_open_menu && !menu_open, |d| {
                                d.hover(move |s| {
                                    s.bg(if can_commit { content(0.8) } else { solid })
                                })
                            })
                            .child(icon("chevron-down-2", 14.).text_color(ink))
                            .when(can_open_menu, |d| {
                                d.on_click(cx.listener(|this, _, window, cx| {
                                    this.open_menu(MenuKind::Commit, window, cx)
                                }))
                            }),
                    ),
            )
            .when_some(index, |d, index| d.child(self.sync_actions(&index, cx)))
    }

    pub(super) fn sync_actions(&self, index: &DiffIndex, cx: &mut Context<Self>) -> AnyElement {
        if index.remote.is_none() {
            return div().into_any_element();
        }
        let busy = self.busy.is_some();
        let (ahead, behind) = (index.ahead, index.behind);
        let has_open_pr = self.pr.as_ref().is_some_and(|p| p.state == "open");
        let on_default = Self::on_default(Some(index));
        let diverged = ahead > 0 && behind > 0;
        let files = index.files.len();
        let can_publish = index.upstream.is_none();
        let can_sync = index.upstream.is_some() && (ahead > 0 || behind > 0);
        let can_create_pr = index.branch.is_some()
            && index.default_branch.is_some()
            && !has_open_pr
            && !on_default
            && !diverged
            && files == 0
            && index.ahead_of_default > 0
            && behind == 0;
        let can_view_pr = has_open_pr && self.pr.as_ref().is_some_and(|p| !p.url.is_empty());
        let show_create = !has_open_pr && !on_default;
        if !can_publish && !can_sync && !show_create && !has_open_pr {
            return div().into_any_element();
        }
        let dest = index.upstream.clone().unwrap_or_else(|| {
            format!(
                "{}/{}",
                index.remote.as_deref().unwrap_or("origin"),
                index.branch.as_deref().unwrap_or("HEAD")
            )
        });
        let syncing = self.busy.as_deref() == Some("sync");
        let plural = |n: i64| if n == 1 { "" } else { "s" };
        let sync_title = if syncing {
            "Synchronizing Changes...".to_owned()
        } else if can_publish {
            index.branch.as_ref().map_or("Publish Branch".into(), |b| {
                format!("Publish Branch \"{b}\"")
            })
        } else if behind > 0 && ahead > 0 {
            format!("Pull {behind} and push {ahead} commits between {dest}")
        } else if behind > 0 {
            format!("Pull {behind} commit{} from {dest}", plural(behind))
        } else {
            format!("Push {ahead} commit{} to {dest}", plural(ahead))
        };
        let button = |id: &'static str, title: String, enabled: bool| {
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(SharedString::from(title))
                .h(px(28.))
                .w_full()
                .min_w_0()
                .flex()
                .items_center()
                .justify_center()
                .gap(px(6.))
                .px_2()
                .rounded_md()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .bg(content(0.1))
                .text_color(content(1.))
                .when(!enabled, |d| d.opacity(0.4))
                .when(enabled, |d| d.hover(|s| s.bg(content(0.15))))
        };
        let glyph = |path: &'static str| icon(path, 14.).text_color(content(1.)).into_any_element();
        div()
            .mt(px(6.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .when(can_publish, |d| {
                d.child(
                    button("publish-branch", sync_title.clone(), !busy)
                        .child(if syncing {
                            spinner("sync-spin", "loader", 14.)
                        } else {
                            glyph("cloud-upload")
                        })
                        .child(div().min_w_0().truncate().child("Publish Branch"))
                        .when(!busy, |d| {
                            d.on_click(cx.listener(|this, _, window, cx| this.sync(window, cx)))
                        }),
                )
            })
            .when(!can_publish && can_sync, |d| {
                d.child(
                    button("sync-changes", sync_title.clone(), !busy)
                        .child(if syncing {
                            spinner("sync-spin", "refresh-cw", 14.)
                        } else {
                            glyph("refresh-cw")
                        })
                        .child(div().min_w_0().truncate().child("Sync Changes"))
                        .when(behind > 0, |d| {
                            d.child(
                                div()
                                    .flex_shrink_0()
                                    .font_features(tabular())
                                    .text_color(content(0.55))
                                    .child(format!("↓{behind}")),
                            )
                        })
                        .when(ahead > 0, |d| {
                            d.child(
                                div()
                                    .flex_shrink_0()
                                    .font_features(tabular())
                                    .text_color(content(0.55))
                                    .child(format!("↑{ahead}")),
                            )
                        })
                        .when(!busy, |d| {
                            d.on_click(cx.listener(|this, _, window, cx| this.sync(window, cx)))
                        }),
                )
            })
            .when(show_create, |d| {
                let title = index
                    .default_branch
                    .as_ref()
                    .map_or("Create pull request".to_owned(), |b| {
                        format!("Create a pull request into {b}")
                    });
                let enabled = can_create_pr && !busy;
                d.child(
                    button("create-pr", title, enabled)
                        .child(if self.busy.as_deref() == Some("pr") {
                            spinner("pr-spin", "loader", 14.)
                        } else {
                            glyph("git-pull-request")
                        })
                        .child("Create PR")
                        .when(enabled, |d| {
                            d.on_click(
                                cx.listener(|this, _, window, cx| this.create_pr(window, cx)),
                            )
                        }),
                )
            })
            .when(has_open_pr, |d| {
                let pr = self.pr.clone().unwrap_or(Pr {
                    number: 0,
                    title: String::new(),
                    url: String::new(),
                    state: String::new(),
                });
                let title = if pr.title.is_empty() {
                    "View pull request".to_owned()
                } else {
                    format!("View PR #{}: {}", pr.number, pr.title)
                };
                let enabled = can_view_pr && !busy;
                let url = pr.url.clone();
                d.child(
                    button("view-pr", title, enabled)
                        .child(glyph("external-link"))
                        .child(div().min_w_0().truncate().child(if pr.number > 0 {
                            format!("View PR #{}", pr.number)
                        } else {
                            "View PR".into()
                        }))
                        .when(enabled, |d| d.on_click(move |_, _, cx| cx.open_url(&url))),
                )
            })
            .into_any_element()
    }

    pub(super) fn hover_line(&mut self, ix: usize, hovering: bool, cx: &mut Context<Self>) {
        let next = if hovering {
            Some(ix)
        } else if self.hovered == Some(ix) {
            None
        } else {
            self.hovered
        };
        if next != self.hovered {
            self.hovered = next;
            cx.notify();
        }
    }

    pub(super) fn line(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let index = self.git.read(cx).index.clone();
        let files = index.as_ref().map_or(&[][..], |i| &i.files[..]);
        let busy = self.busy.is_some();
        match self.lines[ix].clone() {
            Line::Section(kind) => {
                let (title, open) = match kind {
                    Kind2::Staged => ("Staged Changes", self.staged_open),
                    Kind2::Unstaged => ("Changes", self.changes_open),
                };
                let count = files
                    .iter()
                    .filter(|f| match kind {
                        Kind2::Staged => f.staged,
                        Kind2::Unstaged => f.unstaged,
                    })
                    .count();
                let tree = self.view == ChangesView::Tree;
                let actions: Vec<(&'static str, &'static str, &'static str)> = match kind {
                    Kind2::Staged => vec![
                        ("open-all-staged", "Open All Changes", "file-diff"),
                        ("unstage-all", "Unstage All Changes", "minus-sign"),
                    ],
                    Kind2::Unstaged => vec![
                        ("open-all-unstaged", "Open All Changes", "file-diff"),
                        ("discard-all", "Discard All Changes", "undo"),
                        ("stage-all", "Stage All Changes", "plus"),
                    ],
                };
                div()
                    .h(px(ROW))
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px(px(6.))
                    .child(
                        div()
                            .id(("section", ix))
                            .role(Role::Button)
                            .aria_label(title)
                            .aria_expanded(open)
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                icon(
                                    if open {
                                        "chevron-down"
                                    } else {
                                        "chevron-right"
                                    },
                                    14.,
                                )
                                .text_color(content(0.5)),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(10.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(content(0.55))
                                    .child(title.to_uppercase()),
                            )
                            .child(
                                div()
                                    .ml_1()
                                    .h_4()
                                    .min_w_4()
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .bg(accent_color().opacity(0.8))
                                    .px_1()
                                    .text_size(px(8.))
                                    .text_color(white(1.))
                                    .child(count.to_string()),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                match kind {
                                    Kind2::Staged => this.staged_open = !this.staged_open,
                                    Kind2::Unstaged => this.changes_open = !this.changes_open,
                                }
                                this.rebuild(cx);
                            })),
                    )
                    .child(
                        icon_action(
                            ("view", ix),
                            if tree { "View as List" } else { "View as Tree" },
                            if tree { "list-bullet" } else { "folder-tree" },
                            true,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.view = match this.view {
                                ChangesView::List => ChangesView::Tree,
                                ChangesView::Tree => ChangesView::List,
                            };
                            cx.emit(ChangesEvent::View(this.view));
                            this.rebuild(cx);
                        })),
                    )
                    .children(actions.into_iter().map(|(id, label, path)| {
                        icon_action(id, label, path, true)
                            .on_click(cx.listener(move |this, _, window, cx| match id {
                                "unstage-all" => this.run_all(FileAction::Unstage, window, cx),
                                "discard-all" => this.run_all(FileAction::Discard, window, cx),
                                "stage-all" => this.run_all(FileAction::Stage, window, cx),
                                _ => cx.emit(ChangesEvent::OpenDiff {
                                    cwd: this.cwd.clone(),
                                    path: None,
                                    staged: kind == Kind2::Staged,
                                }),
                            }))
                            .into_any_element()
                    }))
                    .into_any_element()
            }
            Line::Dir {
                kind,
                path,
                name,
                depth,
                status,
                open,
            } => {
                let key = format!("{}:{path}", kind.key());
                let stage = kind == Kind2::Unstaged;
                let folder = path.clone();
                div()
                    .id(("change-dir", ix))
                    .on_hover(cx.listener(move |this, hovering: &bool, _, cx| {
                        this.hover_line(ix, *hovering, cx)
                    }))
                    .h(px(ROW))
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_1()
                    .pl(px(8. + depth as f32 * 12.))
                    .pr_2()
                    .text_color(content(1.))
                    .hover(|s| s.bg(content(0.05)))
                    .child(
                        div()
                            .id(("change-dir-toggle", ix))
                            .role(Role::TreeItem)
                            .aria_label(SharedString::from(path.clone()))
                            .aria_expanded(open)
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .size_4()
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        icon(
                                            if open {
                                                "chevron-down"
                                            } else {
                                                "chevron-right"
                                            },
                                            14.,
                                        )
                                        .text_color(content(0.5)),
                                    ),
                            )
                            .child(file_type_icon(&name, true, open, 16.))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .truncate()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(name.clone()),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.collapsed.remove(&key) {
                                    this.collapsed.insert(key.clone());
                                }
                                this.rebuild(cx);
                            })),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .map(|d| {
                                if self.hovered == Some(ix) {
                                    d.flex()
                                } else {
                                    d.hidden()
                                }
                            })
                            .items_center()
                            .child(
                                icon_action(
                                    ("dir-action", ix),
                                    format!(
                                        "{} Changes in {path}",
                                        if stage { "Stage" } else { "Unstage" }
                                    ),
                                    if stage { "plus" } else { "minus-sign" },
                                    !busy,
                                )
                                .when(!busy, |d| {
                                    d.on_click(cx.listener(move |this, _, window, cx| {
                                        this.run_folder(folder.clone(), stage, window, cx)
                                    }))
                                }),
                            ),
                    )
                    .child(
                        div()
                            .w(px(14.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .size(px(6.))
                                    .rounded_full()
                                    .bg(status.as_deref().map_or(content(0.4), status_color)),
                            ),
                    )
                    .into_any_element()
            }
            Line::File { kind, file, depth } => {
                let Some(file) = files.get(file).cloned() else {
                    return div().h(px(ROW)).into_any_element();
                };
                let active = self
                    .selected
                    .as_ref()
                    .is_some_and(|(rel, k)| rel == &file.relative && *k == kind);
                let name = basename(&file.relative).to_owned();
                let dir = if depth.is_some() {
                    ""
                } else {
                    dirname(&file.relative)
                };
                let relative = file.relative.clone();
                let open_path = file.path.clone();
                let can_open = file.status != "deleted";
                let row_file = file.clone();
                let stage_file = file.clone();
                div()
                    .id(("change-row", ix))
                    .on_hover(cx.listener(move |this, hovering: &bool, _, cx| {
                        this.hover_line(ix, *hovering, cx)
                    }))
                    .h(px(ROW))
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_1()
                    .pr_2()
                    .map(|d| match depth {
                        Some(depth) => d.pl(px(8. + depth as f32 * 12.)),
                        None => d.pl_2(),
                    })
                    .text_color(content(1.))
                    .when(active, |d| d.bg(selection()))
                    .when(!active, |d| d.hover(|s| s.bg(content(0.05))))
                    .child(
                        div()
                            .id(("change-open", ix))
                            .role(Role::Button)
                            .aria_label(SharedString::from(file.relative.clone()))
                            .h_full()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .when(depth.is_some(), |d| d.child(div().size_4().flex_shrink_0()))
                            .child(file_type_icon(&name, false, false, 16.))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .flex()
                                    .items_baseline()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(name),
                                    )
                                    .when(!dir.is_empty(), |d| {
                                        d.child(
                                            div()
                                                .ml(px(6.))
                                                .min_w_0()
                                                .truncate()
                                                .text_size(px(11.))
                                                .text_color(content(0.4))
                                                .child(dir.to_owned()),
                                        )
                                    }),
                            )
                            .when(can_open, |d| {
                                d.on_click(cx.listener(move |this, _, _, cx| {
                                    this.selected = Some((relative.clone(), kind));
                                    cx.emit(ChangesEvent::OpenDiff {
                                        cwd: this.cwd.clone(),
                                        path: Some(open_path.clone()),
                                        staged: kind == Kind2::Staged,
                                    });
                                    cx.notify();
                                }))
                            }),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .items_center()
                            .map(|d| {
                                if active || self.hovered == Some(ix) {
                                    d.flex()
                                } else {
                                    d.hidden()
                                }
                            })
                            .when(kind == Kind2::Unstaged, |d| {
                                d.child(
                                    icon_action(("discard", ix), "Discard Changes", "undo", !busy)
                                        .when(!busy, |d| {
                                            d.on_click(cx.listener(move |this, _, window, cx| {
                                                this.run_file(
                                                    row_file.clone(),
                                                    FileAction::Discard,
                                                    window,
                                                    cx,
                                                )
                                            }))
                                        }),
                                )
                            })
                            .child(
                                icon_action(
                                    ("stage", ix),
                                    if kind == Kind2::Staged {
                                        "Unstage Changes"
                                    } else {
                                        "Stage Changes"
                                    },
                                    if kind == Kind2::Staged {
                                        "minus-sign"
                                    } else {
                                        "plus"
                                    },
                                    !busy,
                                )
                                .when(!busy, |d| {
                                    d.on_click(cx.listener(move |this, _, window, cx| {
                                        let action = if kind == Kind2::Staged {
                                            FileAction::Unstage
                                        } else {
                                            FileAction::Stage
                                        };
                                        this.run_file(stage_file.clone(), action, window, cx)
                                    }))
                                }),
                            ),
                    )
                    .child(
                        div()
                            .w(px(14.))
                            .flex_shrink_0()
                            .flex()
                            .justify_end()
                            .font_family(crate::ui::fonts::mono())
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(status_color(&file.status))
                            .child(status_letter(&file.status)),
                    )
                    .into_any_element()
            }
        }
    }
}
