use super::*;

/// monoSpawnedSessions: sessions the run at `turn..end` started with
/// `sessions.start`, and those `known` to be launched that it followed up with
/// `sessions.send`.
pub(crate) fn run_launches(
    source: &[Row],
    turn: usize,
    end: usize,
    known: impl Fn(i64) -> bool,
) -> Vec<i64> {
    let mut found = Vec::new();
    let mut at = turn;
    while at < end {
        let next = (at + 1..end).find(|&i| source[i].first).unwrap_or(end);
        if source[at].role == "mono-launch" {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&source[at].text)
                && let Some(id) = value["sessionId"].as_i64()
                && !found.contains(&id)
            {
                found.push(id);
            }
        } else if source[at].role == "tool" {
            let text: String = source[at..next].iter().map(|r| r.text.as_str()).collect();
            if let Ok(tool) = serde_json::from_str::<ToolCall>(&text)
                && tool.state == ToolState::Completed
                && let Ok(input) = serde_json::from_str::<serde_json::Value>(&tool.input)
                && let Some(action) = input["command"]
                    .as_str()
                    .or_else(|| input["cmd"].as_str())
                    .and_then(monocode_core::mono::work::command_action)
                && matches!(action.as_str(), "sessions.start" | "sessions.send")
            {
                let receipts = serde_json::from_str::<serde_json::Value>(&tool.output)
                    .map(|v| vec![v])
                    .unwrap_or_else(|_| {
                        tool.output
                            .lines()
                            .filter_map(|line| serde_json::from_str(line.trim()).ok())
                            .collect()
                    });
                for value in receipts {
                    if value["ok"] == true
                        && let Some(id) = value["result"]["sessionId"]
                            .as_i64()
                            .or_else(|| {
                                monocode_core::mono::work::session_handle(&value["result"]["id"])
                            })
                            .or_else(|| {
                                (value["result"]["submitted"] == true)
                                    .then(|| {
                                        monocode_core::mono::work::session_handle(
                                            &value["result"]["sessionId"],
                                        )
                                    })
                                    .flatten()
                            })
                        && (action == "sessions.start" || known(id))
                        && !found.contains(&id)
                    {
                        found.push(id);
                    }
                }
            }
        }
        at = next;
    }
    found
}

/// Whether the Mono has said something the user can read in `from..to`: the
/// opening sentence of any run there. A message the user sends after it reads
/// as the next line of the chat, not as part of the prompt above.
pub(crate) fn mono_spoke(source: &[Row], from: usize, to: usize) -> bool {
    let mut working = false;
    for (at, row) in source[from.min(to)..to].iter().enumerate() {
        if row.is_prompt() {
            working = false;
        } else if row.first && row.role != "reasoning" {
            if !working && row.role == "assistant" {
                // One message spans its first row and the continuations after it.
                let at = from.min(to) + at;
                let text: String = std::iter::once(row)
                    .chain(source[at + 1..to].iter().take_while(|r| !r.first))
                    .map(|r| r.text.as_str())
                    .collect();
                let text = text.trim();
                if !text.is_empty() && !text.starts_with("<reaction>") {
                    return true;
                }
            }
            working = true;
        }
    }
    false
}

/// monoWorkStatus: a short status for the turn's latest activity.
pub(crate) fn mono_work_label(source: &[Row], group: usize) -> String {
    let turn = source[group.min(source.len())..]
        .iter()
        .rposition(|r| r.is_prompt())
        .map_or(group, |i| group + i + 1);
    const THINKING: [&str; 4] = [
        "Thinking…",
        "Pondering…",
        "Considering…",
        "Working it through…",
    ];
    let end = (turn..source.len())
        .find(|&i| source[i].is_prompt())
        .unwrap_or(source.len());
    let mut thought = 0usize;
    let mut latest = None;
    let mut reasoning = None;
    for (ix, row) in source[turn..end].iter().enumerate() {
        if !row.first {
            continue;
        }
        match row.role.as_str() {
            "assistant" => {
                thought += 1;
                latest = None;
                reasoning = None;
            }
            "reasoning" => {
                thought += 1;
                latest = None;
                reasoning = Some(turn + ix);
            }
            "tool" => {
                latest = Some(turn + ix);
                reasoning = None;
            }
            _ => {}
        }
    }
    let message = |at: usize| -> String {
        let end = (at + 1..source.len())
            .find(|&i| source[i].first)
            .unwrap_or(source.len());
        source[at..end].iter().map(|r| r.text.as_str()).collect()
    };
    // A reasoning summary's own heading says what it is working on.
    if let Some(heading) = reasoning.and_then(|at| reasoning_heading(&message(at))) {
        return format!("{heading}…");
    }
    let Some(tool) = latest.and_then(|at| serde_json::from_str::<ToolCall>(&message(at)).ok())
    else {
        return THINKING[thought.saturating_sub(1) % THINKING.len()].into();
    };
    if tool.is_milestone() {
        return if tool.state != ToolState::Running {
            "Thinking…".into()
        } else if tool.is_handoff() {
            format!("{}…", tool.summary().0)
        } else {
            "Compacting context…".into()
        };
    }
    tool_status(&tool)
}

impl TranscriptView {
    /// TurnDuration with `revealOnHover`: copy, save as note and activity in
    /// the turn's fixed gap, then its time, shown while the turn is hovered.
    pub(super) fn mono_actions(
        &mut self,
        turn: usize,
        output: String,
        before_user: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let copied = self.copied_response.is_some_and(|(id, _)| id == turn);
        let saved = self.note_feedback.get(&turn).is_some_and(|r| r.is_ok());
        let pending = self.note_pending.contains(&turn);
        let opacity = match self.hovered_turn {
            Some((hovered, since)) if hovered == turn => {
                (since.elapsed().as_secs_f32() / HOVER_FADE).min(1.)
            }
            _ if copied || pending => 1.,
            _ => 0.,
        };
        let completed_at = self
            .turns
            .iter()
            .filter(|(run, _)| self.mono_group_start(**run) == turn)
            .max_by_key(|(run, _)| *run)
            .and_then(|(_, info)| info.completed_at);
        let dot = || {
            div()
                .size(px(3.))
                .flex_shrink_0()
                .rounded_full()
                .bg(content(0.25))
        };
        let mut actions = div().flex().flex_shrink_0().items_center().gap_1();
        if output.is_empty() {
            actions = actions.child(icon("check", 14.).text_color(content(0.4)));
        } else {
            let text = output.clone();
            let quote = output.clone();
            actions = actions
                .child(
                    response_actions::button(
                        turn,
                        "mono-copy-response",
                        if copied { "Copied" } else { "Copy response" },
                        if copied { "check" } else { "response-copy" },
                        false,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                        let born = Instant::now();
                        this.copied_response = Some((turn, born));
                        cx.notify();
                        cx.spawn(async move |this, cx| {
                            cx.background_executor()
                                .timer(std::time::Duration::from_secs(2))
                                .await;
                            let _ = this.update(cx, |this, cx| {
                                if this.copied_response == Some((turn, born)) {
                                    this.copied_response = None;
                                    cx.notify();
                                }
                            });
                        })
                        .detach();
                    })),
                )
                .child(
                    response_actions::button(
                        turn,
                        "mono-note-response",
                        if saved {
                            "Saved to Notes"
                        } else if pending {
                            "Saving note…"
                        } else {
                            "Save as note"
                        },
                        if saved { "check" } else { "response-note" },
                        false,
                    )
                    .when(pending, |d| d.opacity(0.4))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.note_pending.insert(turn) {
                            this.note_feedback.remove(&turn);
                            cx.emit(ResponseRequest::SaveNote {
                                turn,
                                text: output.clone(),
                                selection: false,
                            });
                            cx.notify();
                        }
                    })),
                )
                .child(
                    response_actions::button(
                        turn,
                        "mono-quote-response",
                        "Quote response",
                        "comment-add",
                        false,
                    )
                    .on_click(cx.listener(move |_, _, window, cx| {
                        cx.emit(MonoResponseAction::Quote {
                            text: quote.clone(),
                            window: window.window_handle(),
                        })
                    })),
                );
        }
        actions = actions.child(
            response_actions::button(
                turn,
                "mono-response-activity",
                "Show activity",
                "list-bullet",
                false,
            )
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(MonoResponseAction::Activity(turn)))),
        );
        div()
            .id(("mono-turn-actions", turn))
            .relative()
            .top(px(-4.))
            .h(px(24.))
            .mb(px(if before_user { 0. } else { 4. }))
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(10.))
            .overflow_hidden()
            .text_size(px(14.))
            .text_color(content(0.4))
            .opacity(opacity)
            .child(actions)
            .when_some(completed_at, |d, time| {
                d.child(dot()).child(
                    div()
                        .flex_shrink_0()
                        .text_color(content(0.35))
                        .child(monocode_core::session::response::local_clock(time)),
                )
            })
            .into_any_element()
    }

    /// The Mono signs its turn with its mascot and name, beside a live ticker.
    pub(super) fn mono_turn_header(&self, turn: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        // Completion runs share the original turn's header.
        let label = self
            .mono_activity()
            .filter(|(group, _)| *group == turn)
            .map(|(_, label)| label);
        self.mono_header(turn, label, cx)
    }

    /// The latest live activity, without relabeling a completed turn before
    /// the next prompt arrives.
    pub(super) fn mono_activity(&self) -> Option<(usize, String)> {
        if !self.mono || !self.streaming || self.source.len() <= self.live_start {
            return None;
        }
        let latest = self
            .source
            .iter()
            .rposition(|r| r.is_prompt())
            .map_or(0, |ix| ix + 1);
        if latest < self.live_start && self.source[..latest].iter().any(Row::is_prompt) {
            return None;
        }
        let typing = (latest..self.source.len())
            .rev()
            .find(|&at| self.source[at].first)
            .is_some_and(|message| {
                self.source[message].role == "assistant"
                    && self.source[message..]
                        .iter()
                        .any(|row| !row.text.trim().is_empty())
                    && self.mono_prose_hidden(message)
            });
        let group = self.mono_group_start(latest);
        Some((
            group,
            if typing {
                "Typing…".into()
            } else {
                mono_work_label(&self.source, group)
            },
        ))
    }

    pub(super) fn mono_header(
        &self,
        turn: usize,
        label: Option<String>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some((mascot, color, name)) = self.mono_look.clone() else {
            return div().into_any_element();
        };
        // As a model's "Worked for" line: the Mono's mascot on a tile, then
        // its name.
        let pill = div()
            .debug_selector(|| "mono-signature-pill".into())
            .flex()
            .min_w_0()
            .flex_shrink_0()
            .max_w(px(320.))
            .items_center()
            .gap(px(8.))
            .child(
                crate::ui::tabs::tile::mark_tile(
                    &crate::ui::tabs::tile::ACTIVITY_TILE,
                    crate::ui::tabs::tile::Face::Neutral,
                    crate::ui::avatar::avatar(&mascot, color, "idle", 14., true),
                )
                .flex_shrink_0(),
            )
            .child(div().min_w_0().truncate().child(name));
        div()
            .id(("mono-turn-header", turn))
            .flex()
            .min_w_0()
            .items_center()
            .gap_1p5()
            .py_1()
            .text_size(px(14.))
            .line_height(px(20.))
            .text_color(content(0.5))
            .child(pill)
            .when_some(label, |d, label| {
                d.child(div().flex_shrink_0().text_color(content(0.25)).child("·"))
                    .child(
                        div()
                            .id(("mono-turn-ticker", turn))
                            .debug_selector(|| "mono-turn-ticker".into())
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .hover(|s| s.opacity(0.7))
                            .child(if cx.reduce_motion() {
                                div().child(label).into_any_element()
                            } else {
                                crate::chat::transcript::activity::shimmer(
                                    label,
                                    self.started.map_or(0., |s| s.elapsed().as_secs_f32()),
                                )
                            })
                            .when(turn < self.source.len(), |d| {
                                d.role(Role::Button).aria_label("Show activity").on_click(
                                    cx.listener(move |_, _, _, cx| {
                                        cx.emit(MonoResponseAction::Activity(turn))
                                    }),
                                )
                            }),
                    )
            })
            .into_any_element()
    }

    /// MonoReactionBadge, on the top left corner of the message it answers.
    pub(super) fn reaction_badge(&self, prompt: usize) -> Option<gpui::AnyElement> {
        let (emoji, born) = self.reactions.get(&prompt)?;
        // agent-chat-bubble-in: it lands with a small bounce.
        let (scale, opacity) = born.map_or((1., 1.), |born| {
            let t = (born.elapsed().as_secs_f32() / 0.32).min(1.);
            (
                0.9 + 0.1 * crate::ui::motion::reveal::bezier(t, 0.2, 1.4, 0.4, 1.),
                (t / 0.4).min(1.),
            )
        });
        let height = 22. * scale;
        Some(
            div()
                .id(("mono-reaction", prompt))
                .role(Role::Image)
                .absolute()
                .top(px(-10. - (height - 22.) / 2.))
                .left(px(-10.))
                .h(px(height))
                .px(px(7. * scale))
                .flex()
                .items_center()
                .rounded(px(8. * scale))
                .bg(content(0.17))
                .border_1()
                .border_color(content(0.1))
                .text_size(px(11. * scale))
                .line_height(px(11. * scale))
                .opacity(opacity)
                .aria_label(format!("Reacted {emoji}"))
                .child(emoji.clone())
                .into_any_element(),
        )
    }

    /// `.transcript-turn:hover` shows the reply's actions; hovering a prompt
    /// shows only its own.
    pub(super) fn mono_hover(
        &mut self,
        prompt: bool,
        start: usize,
        turn: usize,
        key: (usize, usize),
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        if prompt {
            if hovered {
                self.hovered_prompt = Some(start);
            } else if self.hovered_prompt == Some(start) {
                self.hovered_prompt = None;
            }
        } else if hovered {
            self.hover_rows.insert(key, turn);
        } else {
            self.hover_rows.remove(&key);
        }
        let current = if !prompt && hovered {
            Some(turn)
        } else {
            self.hovered_turn
                .map(|(t, _)| t)
                .filter(|t| self.hover_rows.values().any(|v| v == t))
                .or_else(|| self.hover_rows.values().next().copied())
        };
        if current != self.hovered_turn.map(|(t, _)| t) {
            self.hovered_turn = current.map(|t| (t, Instant::now()));
        }
        cx.notify();
    }

    /// Starting the provider precedes its new prompt. Until that prompt
    /// arrives, the previous answer remains a finished turn.
    pub(super) fn mono_pending_joins_live_turn(&self) -> bool {
        self.streaming
            && self
                .source
                .iter()
                .rposition(Row::is_prompt)
                .is_some_and(|prompt| {
                    prompt >= self.live_start
                        && !mono_spoke(
                            &self.source,
                            self.mono_group_start(prompt + 1),
                            self.source.len(),
                        )
                })
    }

    /// Whether `at` sits in a completion run that continues the turn above,
    /// returning the turn that opens the group: groupMonoChatTurns.
    pub(super) fn mono_group_start(&self, mut turn: usize) -> usize {
        let mut visited = Vec::new();
        let group = loop {
            if let Some(group) = self.mono_groups.borrow().get(&turn).copied() {
                break group;
            }
            visited.push(turn);
            match self
                .turns
                .get(&turn)
                .and_then(|info| info.joined_turn)
                // A steer stays beside the prompt it joins only until the
                // Mono has replied in words; after that, the chat reads in order.
                .filter(|&root| {
                    root < turn && !mono_spoke(&self.source, root, prompt_start(&self.source, turn))
                })
                .or_else(|| mono_continues(&self.source, turn))
            {
                Some(previous) => turn = previous,
                None => break turn,
            }
        };
        let mut groups = self.mono_groups.borrow_mut();
        for turn in visited {
            groups.insert(turn, group);
        }
        group
    }

    pub(super) fn mono_next_run_continues(&self, turn: usize, end: usize) -> bool {
        end < self.source.len()
            && self.mono_group_start(
                (end..self.source.len())
                    .find(|&at| !self.source[at].is_prompt())
                    .unwrap_or(self.source.len()),
            ) == self.mono_group_start(turn)
    }

    /// groupMonoTurnItems: prose stays in the activity trail while its run is
    /// live, and when later work in the run follows it.
    pub(super) fn mono_prose_hidden(&self, at: usize) -> bool {
        let end = (at + 1..self.source.len())
            .find(|&i| self.source[i].is_prompt())
            .unwrap_or(self.source.len());
        let turn = self.source[..at]
            .iter()
            .rposition(Row::is_prompt)
            .map_or(0, |prompt| prompt + 1);
        let internal = turn > 0 && self.source[turn - 1].role == "mono-system";
        // A direct reply keeps its opening sentence for the conversation.
        // Notification runs keep all commentary in activity and show only
        // their final message. Every Mono message appears whole.
        let start = self.source[..=at]
            .iter()
            .rposition(|r| r.first)
            .unwrap_or(at);
        let complete = (start + 1..self.source.len()).any(|i| self.source[i].first)
            || !(self.streaming && start >= self.live_start);
        if !internal
            && self.source[turn..start]
                .iter()
                .all(|r| !r.first || r.role == "reasoning")
        {
            return !complete;
        }
        // Starting a request precedes delivery of its prompt. History remains
        // settled during that gap, even though it is still the source's tail.
        (self.streaming && at >= self.live_start && end == self.source.len())
            || (self.source.get(end).is_some_and(|row| row.role == "user")
                && self.mono_next_run_continues(turn, end))
            || self.source[at + 1..end].iter().any(|r| {
                r.first
                    && (matches!(r.role.as_str(), "tool" | "reasoning")
                        || (internal && r.role == "assistant"))
            })
    }

    /// monoSpawnedSessions: sessions a run's `sessions.start` calls accepted.
    pub(super) fn run_launches(&self, turn: usize, end: usize) -> Vec<i64> {
        run_launches(&self.source, turn, end, |id| {
            self.launches.contains_key(&id)
        })
    }

    /// The live state of the sessions this Mono launched, for their cards.
    pub(crate) fn set_launches(
        &mut self,
        launches: HashMap<i64, LaunchView>,
        cx: &mut Context<Self>,
    ) {
        if self.launches != launches {
            let structure_changed = self.launches.len() != launches.len()
                || self.launches.keys().any(|id| !launches.contains_key(id));
            self.launches = launches;
            if structure_changed {
                self.rebuild(0, cx);
            } else {
                cx.notify();
            }
        }
    }

    pub(super) fn launch_row(
        &mut self,
        ix: usize,
        id: i64,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let (top, bottom) = self.mono_card_spacing(ix);
        let view = self.launches.get(&id).cloned();
        div()
            .w_full()
            .flex()
            .justify_center()
            .child(
                div()
                    .relative()
                    .w_full()
                    .max_w(px(COLUMN))
                    .px(px(COLUMN_PAD))
                    .pt(px(top))
                    .pb(px(bottom))
                    .child(crate::monos::cards::launch(id, view.as_ref(), cx)),
            )
            .into_any_element()
    }

    pub(super) fn mono_card_spacing(&self, ix: usize) -> (f32, f32) {
        let is_card = |row: &UiRow| matches!(row.body, Body::Launch(_) | Body::MonoCard(_));
        let top = if ix > 0 && is_card(&self.rows[ix - 1]) {
            0.
        } else {
            4.
        };
        // Prose already supplies its own 12px top gap. Do not add another
        // 12px below the card when a completion continues the conversation.
        let bottom = match self.rows.get(ix + 1) {
            Some(row) if is_card(row) => 8.,
            Some(row) if matches!(row.body, Body::Markdown(_)) => 0.,
            _ => 12.,
        };
        (top, bottom)
    }
}
