use super::*;

impl Completion {
    pub(super) fn rank(&mut self) {
        self.choices.clear();
        let Some(token) = &self.token else {
            return;
        };
        if self.mcp_open {
            let mut servers = self.mcp.clone();
            servers.sort_by_key(|server| {
                (
                    server.provider != self.provider.id() || !server.enabled,
                    server.name.to_lowercase(),
                    server.provider.clone(),
                )
            });
            self.choices = servers
                .into_iter()
                .map(|server| Choice {
                    label: server.name.clone(),
                    detail: format!(
                        "{} · {} · {}",
                        server.provider,
                        server.scope,
                        if server.provider != self.provider.id() {
                            "Different provider"
                        } else if !server.enabled {
                            "Disabled in provider configuration"
                        } else {
                            "Configured for this provider"
                        }
                    ),
                    insertion: String::new(),
                    builtin: None,
                    mode: None,
                    file: false,
                    server: Some(server),
                })
                .collect();
        } else if token.trigger == Trigger::Mention {
            self.choices = composer::rank_files(&self.files, &token.query, 60)
                .into_iter()
                .map(|file| Choice {
                    label: file.name.clone(),
                    detail: file.relative.clone(),
                    insertion: self.mention_label(file),
                    builtin: None,
                    mode: None,
                    file: true,
                    server: None,
                })
                .collect();
            let query = token
                .query
                .strip_prefix("note/")
                .unwrap_or(&token.query)
                .trim_end_matches('/');
            let mut notes: Vec<_> = self
                .notes
                .iter()
                .filter_map(|note| {
                    let title = composer::score(query, &note.title).map(|score| score + 400);
                    let slug = composer::score(query, &note.slug);
                    let tag = note
                        .tags
                        .iter()
                        .filter_map(|tag| composer::score(query.trim_start_matches('#'), tag))
                        .max()
                        .map(|score| score + 200);
                    title.or(slug).or(tag).map(|score| (score, note))
                })
                .collect();
            notes.sort_by(|(a_score, a), (b_score, b)| {
                b_score.cmp(a_score).then(b.updated_at.cmp(&a.updated_at))
            });
            if token.query.starts_with("note/") {
                self.choices.clear();
            }
            self.choices
                .extend(notes.into_iter().take(8).map(|(_, note)| Choice {
                    label: note.title.clone(),
                    detail: format!("note/{}", note.slug),
                    insertion: format!("@note/{}", note.slug),
                    builtin: None,
                    mode: None,
                    file: false,
                    server: None,
                }));
        } else {
            let mut choices = Vec::new();
            for (order, (id, label, detail)) in [
                (
                    "draft",
                    "Draft",
                    "Save this message without starting the agent",
                ),
                (
                    "orchestrator",
                    "Orchestrator",
                    "Plan and coordinate agent work",
                ),
                ("operator", "Operator", "Give this thread access to MCode"),
                ("plan", "Plan mode", "Review a plan before building"),
                (
                    "btw",
                    "Side question",
                    "Ask a read-only side question about the current turn",
                ),
                (
                    "compact",
                    "Compact context",
                    "Summarize older conversation context to free space",
                ),
                (
                    "resume",
                    "Resume",
                    "Open another Claude Code session from this folder",
                ),
                (
                    "add-to-folder",
                    "Add to folder",
                    "Place this session in an existing or new sidebar folder",
                ),
                ("mcp", "MCP server", "Find an MCP server for this message"),
                ("stage", "Stage", "Stage only this session's changes"),
                (
                    "commit",
                    "Commit",
                    "Stage and commit this session's changes",
                ),
                (
                    "visualize",
                    "Visualize",
                    "Create an inline visual of this topic or a specific request",
                ),
            ]
            .into_iter()
            .enumerate()
            {
                if matches!(id, "stage" | "commit") && self.has_user_command(id) {
                    continue;
                }
                if id == "resume" && self.provider != monocode_core::provider::Kind::Claude {
                    continue;
                }
                if id == "btw"
                    && matches!(
                        self.provider,
                        monocode_core::provider::Kind::Fx
                            | monocode_core::provider::Kind::Hermes
                            | monocode_core::provider::Kind::Antigravity
                    )
                {
                    continue;
                }
                if let Some(score) = composer::score(&token.query, &format!("{id} {label}")) {
                    // Only the bare slash menu uses the fixed command order.
                    let priority = if token.query.is_empty() {
                        100 - order as i64
                    } else {
                        100
                    };
                    choices.push((
                        score + priority,
                        Choice {
                            label: label.into(),
                            detail: detail.into(),
                            insertion: format!("/{id}"),
                            builtin: Some(id),
                            mode: (!matches!(
                                id,
                                "compact" | "btw" | "stage" | "commit" | "visualize" | "resume"
                            ))
                            .then_some(id),
                            file: false,
                            server: None,
                        },
                    ));
                }
            }
            for command in &self.native_commands {
                // MCode's own `/resume` lists these sessions for a chat.
                if self.provider == monocode_core::provider::Kind::Claude
                    && command.invocation.trim_start_matches('/') == "resume"
                {
                    continue;
                }
                if let Some(score) = composer::score(
                    &token.query,
                    &format!(
                        "{} {} {}",
                        command.invocation,
                        command.description,
                        command.aliases.join(" ")
                    ),
                ) {
                    choices.push((
                        score,
                        Choice {
                            label: command.invocation.trim_start_matches('/').into(),
                            detail: {
                                let hint = command.input_hint.clone().unwrap_or_else(|| {
                                    command
                                        .subcommands
                                        .iter()
                                        .map(|subcommand| {
                                            subcommand.usage.as_deref().unwrap_or(&subcommand.name)
                                        })
                                        .collect::<Vec<_>>()
                                        .join(" · ")
                                });
                                [command.description.clone(), hint]
                                    .into_iter()
                                    .filter(|part| !part.is_empty())
                                    .collect::<Vec<_>>()
                                    .join(" · ")
                            },
                            insertion: format!("/{}", command.invocation),
                            builtin: None,
                            mode: None,
                            file: false,
                            server: None,
                        },
                    ));
                }
            }
            for skill in &self.skills {
                if matches!(
                    skill.name.as_str(),
                    "plan"
                        | "compact"
                        | "mcp"
                        | "operator"
                        | "orchestrator"
                        | "add-to-folder"
                        | "draft"
                        | "btw"
                        | "mono"
                        | "monocode"
                        | "resume"
                ) {
                    continue;
                }
                if let Some(score) = composer::score(
                    &token.query,
                    &format!("{} {}", skill.name, skill.description),
                ) {
                    choices.push((
                        score,
                        Choice {
                            label: skill.name.trim_start_matches('/').into(),
                            detail: if skill.description.is_empty() {
                                format!("{} · {}", skill.scope, skill.source)
                            } else {
                                skill.description.clone()
                            },
                            insertion: format!("/{}", skill.name),
                            builtin: None,
                            mode: None,
                            file: false,
                            server: None,
                        },
                    ));
                }
            }
            choices.sort_by(|(a, x), (b, y)| b.cmp(a).then_with(|| x.label.cmp(&y.label)));
            let limit = if self.native_commands.is_empty() {
                60
            } else {
                usize::MAX
            };
            self.choices = choices.into_iter().take(limit).map(|(_, c)| c).collect();
        }
        self.active = self.active.min(self.choices.len().saturating_sub(1));
        self.scroll.scroll_to_item(self.active);
    }

    pub(super) fn pick(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(choice) = self.choices.get(ix).cloned() else {
            return;
        };
        let Some(token) = self.token.take() else {
            return;
        };
        // Validate against the live caret; an asynchronously loaded catalog must
        // never replace a stale range after a move or IME composition.
        let live = self.input.read(cx);
        let current_token = live.completion_token().or_else(|| {
            self.mcp_open
                .then(|| composer::token_at(&live.content, live.content.trim_end().len()))
                .flatten()
        });
        if current_token.as_ref() != Some(&token) {
            self.mcp_open = false;
            return;
        }
        if choice.mode == Some("mcp") {
            self.token = Some(token);
            self.mcp_open = true;
            if !self.loading {
                self.load(cx);
            }
            self.rank();
            cx.notify();
            return;
        }
        let insertion = if let Some(server) = &choice.server {
            let base = format!("@mcp/{}", server.name);
            let mut token = base.clone();
            if self.selected_mcp.iter().any(|(known, _)| known == &token) {
                token = format!("@mcp/{}/{}", server.provider, server.name);
            }
            if self.selected_mcp.iter().any(|(known, _)| known == &token) {
                token = format!("@mcp/{}/{}/{}", server.provider, server.scope, server.name);
            }
            let disambiguated = token.clone();
            let mut suffix = 2;
            while self.selected_mcp.iter().any(|(known, _)| known == &token) {
                token = format!("{disambiguated}-{suffix}");
                suffix += 1;
            }
            self.selected_mcp.push((token.clone(), server.clone()));
            self.known.insert(token.clone());
            format!("{token} ")
        } else {
            match choice.mode {
                Some(mode) => format!("/{mode} "),
                None => format!("{} ", choice.insertion),
            }
        };
        self.mcp_open = false;
        let mut range = token.range;
        if self.input.read(cx).content[range.end..].starts_with(' ') {
            range.end += 1;
        }
        self.input.update(cx, |input, cx| {
            input.replace_completion(range, &insertion, window, cx)
        });
        self.token = None;
        if let Some(mode) = choice.mode {
            cx.emit(ModePicked(mode));
        }
        window.focus(&self.input.focus_handle(cx), cx);
        cx.notify();
    }
}
