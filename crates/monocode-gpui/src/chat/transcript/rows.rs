use super::*;

impl TranscriptView {
    /// `rebuild` for a caller that notifies itself.
    pub(super) fn rebuild_rows(&mut self, start: usize, cx: &mut Context<Self>) {
        #[cfg(test)]
        {
            self.rebuild_passes += 1;
        }
        self.phase_layouts.clear();
        let now = Instant::now();
        let following = self.list.is_following_tail();
        let offset = self.list.logical_scroll_top();
        let anchor = self.rows.get(offset.item_ix).map(|r| r.key);
        let anchor_turn = self
            .rows
            .get(offset.item_ix)
            .and_then(|r| self.messages.get(r.message))
            .map(|m| m.turn);
        let current_turn = self.source[..(start + 1).min(self.source.len())]
            .iter()
            .rposition(|r| r.is_prompt())
            .map_or(0, |i| i + 1);
        let current_end = (current_turn..self.source.len())
            .find(|&i| self.source[i].is_prompt())
            .unwrap_or(self.source.len());
        // Advance incoming prose before choosing the displayed block. Its
        // first visible words and the previous block's replacement must be
        // committed together, including after an empty provider placeholder.
        if !self.mono
            && let Some(at) = (current_turn..current_end)
                .rev()
                .find(|&i| self.source[i].role == "assistant" && self.source[i].first)
        {
            let end = (at + 1..current_end)
                .find(|&i| self.source[i].first)
                .unwrap_or(current_end);
            let text: String = self.source[at..end]
                .iter()
                .map(|r| r.text.as_str())
                .collect();
            self.prepare_reveal(at, end, &text, true, now, cx.reduce_motion());
        }
        let candidate = (current_turn..current_end).rev().find(|&i| {
            self.source[i].role == "assistant" && self.source[i].first && self.answer_visible(i)
        });
        let target = candidate.filter(|&answer| {
            self.source[current_turn..answer]
                .iter()
                .any(|r| r.role == "tool")
        });
        let layout_changed = self.fold_targets.get(&current_turn).copied().flatten() != target;
        let known_message = self.messages.iter().any(|m| {
            m.start <= start
                && m.role == "assistant"
                && self
                    .source
                    .get(start)
                    .is_some_and(|r| r.role == "assistant")
        });
        let mut message_ix = self
            .messages
            .partition_point(|m| m.start <= start)
            .saturating_sub(1);
        while message_ix > 0
            && matches!(
                self.messages[message_ix].role.as_str(),
                "tool" | "reasoning"
            )
            && matches!(
                self.messages[message_ix - 1].role.as_str(),
                "tool" | "reasoning"
            )
        {
            message_ix -= 1;
        }
        if (layout_changed
            || !known_message
            || self.source.get(start).is_none_or(|r| r.role != "assistant"))
            && self.source[..start.min(self.source.len())]
                .iter()
                .rev()
                .take_while(|r| !r.is_prompt())
                .any(|r| r.role == "tool")
        {
            while message_ix > 0
                && !matches!(
                    self.messages[message_ix - 1].role.as_str(),
                    "user" | "mono-system"
                )
            {
                message_ix -= 1;
            }
        }
        if !self.mono
            && (self.source[current_turn..current_end].iter().any(|row| {
                row.role == "tool"
                    && row.first
                    && serde_json::from_str::<ToolCall>(&row.text)
                        .is_ok_and(|call| matches!(call.panel(), Some(Panel::Agent(_))))
            }) || self.rows.iter().any(|row| {
                self.messages
                    .get(row.message)
                    .is_some_and(|message| message.turn == current_turn)
                    && matches!(
                        &row.body,
                        Body::Tool { call, .. } if matches!(call.panel(), Some(Panel::Agent(_)))
                    )
            }))
        {
            // Pinned runs sit below the fold's work rather than at their
            // original source offset. Rebuild the whole turn so a suffix
            // update cannot cut across that reordered stack. Include the old
            // rows: a patch can remove the run that caused the reordering.
            while message_ix > 0 && self.messages[message_ix - 1].start >= current_turn {
                message_ix -= 1;
            }
        }
        if self.mono {
            // A run's prose hides behind later work, and a completion run
            // takes over the actions of the turn it continues, so rebuild from
            // the prompt that opens the whole group.
            let group = self.mono_group_start(current_turn);
            let prompt = self.prompt_start(group);
            while message_ix > 0
                && self
                    .messages
                    .get(message_ix)
                    .is_some_and(|m| m.start > prompt)
            {
                message_ix -= 1;
            }
        }
        let source_start = if message_ix == 0 {
            self.history_start
        } else {
            self.messages[message_ix].start
        };
        let display_start = if message_ix == 0 {
            0
        } else {
            // A hidden opening message can inherit a pinned run's later
            // position. Cut before every displayed row in the source suffix,
            // including work that now appears above that run.
            self.messages[message_ix..]
                .iter()
                .map(|message| message.display_start)
                .min()
                .unwrap_or(self.rows.len())
        };
        let old_count = self.rows.len();
        let old_rows = self.rows[display_start..].to_vec();
        let old_layouts: Vec<_> = old_rows
            .iter()
            .enumerate()
            .map(|(ix, row)| {
                (
                    row.clone(),
                    measurements::after_receipt(&self.rows, &self.messages, display_start + ix),
                )
            })
            .collect();
        let mut previous: HashMap<_, _> = self
            .messages
            .split_off(message_ix)
            .into_iter()
            .map(|m| (m.start, m))
            .collect();
        self.rows.truncate(display_start);
        let activity = if self.mono {
            phases::Phases::default()
        } else {
            phases::collect(&self.source, source_start, self.streaming)
        };
        let mut at = source_start;
        let mut turn = 0;
        let mut turn_end = 0;
        let mut final_answer = None;
        let mut awaiting_answer = None;
        let mut has_work = false;
        let mut ready = false;
        let mut git_card: Option<Arc<git_turns::GitCard>> = None;
        let mut git_live_shown = false;
        while at < self.source.len() {
            let mut end = at + 1;
            while end < self.source.len() && !self.source[end].first {
                end += 1;
            }
            let role = self.source[at].role.clone();
            let is_prompt = self.source[at].is_prompt();
            let text: String = self.source[at..end]
                .iter()
                .map(|r| r.text.as_str())
                .collect();
            // Older native versions saved user cancellation as an error row.
            let stopped_notice = role == "error" && text.trim() == "Stopped";
            let (display_text, attachments) = if role == "user" {
                attachments::message(&self.source, end, &text)
            } else {
                (None, Arc::default())
            };
            let old = previous.remove(&at).filter(|p| p.role == role);
            let mut user_link = None;
            let (mut parser, mut markdown_blocks) = old
                .map(|p| (p.parser, p.markdown_blocks))
                .unwrap_or_else(|| {
                    (
                        self.prepared_parsers.remove(&at).unwrap_or_default(),
                        Vec::new(),
                    )
                });
            let message = self.messages.len();
            let first = self.rows.len();
            if at >= turn_end {
                turn = if is_prompt {
                    (at..self.source.len())
                        .find(|&i| !self.source[i].is_prompt())
                        .unwrap_or(self.source.len())
                } else {
                    self.source[..at]
                        .iter()
                        .rposition(|r| r.is_prompt())
                        .map_or(0, |i| i + 1)
                };
                turn_end = (turn..self.source.len())
                    .find(|&i| self.source[i].is_prompt())
                    .unwrap_or(self.source.len());
                awaiting_answer = (turn..turn_end)
                    .rev()
                    .find(|&i| self.source[i].role == "assistant" && self.source[i].first)
                    .filter(|&i| !self.answer_visible(i));
                final_answer = (turn..turn_end).rev().find(|&i| {
                    self.source[i].role == "assistant"
                        && self.source[i].first
                        && self.answer_visible(i)
                });
                // A Mono's work stays in its activity panel, never folded inline.
                has_work = !self.mono
                    && final_answer.is_some_and(|answer| {
                        self.source[turn..answer].iter().any(|r| r.role == "tool")
                    });
                ready = final_answer.is_some_and(|answer| self.answer_visible(answer));
                self.fold_targets
                    .insert(turn, final_answer.filter(|_| has_work && ready));
                git_card = self.git_card(turn, turn_end, final_answer);
                git_live_shown = false;
            }
            let folded = self.fold_work
                && !is_prompt
                && !activity.pinned.contains(&at)
                && has_work
                && ready
                && final_answer.is_some_and(|answer| at < answer)
                && !self
                    .work_folds
                    .get(&turn)
                    .copied()
                    .unwrap_or(!self.fold_work);
            if !self.document
                && !is_prompt
                && !stopped_notice
                && !matches!(
                    role.as_str(),
                    "mono-card"
                        | "remote-plan"
                        | "visual"
                        | "mono-document"
                        | "mono-note"
                        | "mono-launch"
                        | monocode_core::session::transcript::attachments::ROLE
                )
                && !(self.mono && text.trim().starts_with("<reaction>"))
                && (at == 0
                    || self.source[..at]
                        .iter()
                        .rev()
                        .find(|row| row.role != monocode_core::session::transcript::attachments::ROLE)
                        .is_some_and(Row::is_prompt)
                    // A run on a later day, such as a habit report, signs
                    // again under its own day.
                    || (self.mono && dates::opens_later_day(&self.source, at)))
                && !(self.mono
                    && self.mono_group_start(turn) != turn
                    && !(self.history_start > 0 && self.prompt_start(turn) == self.history_start))
            {
                self.rows.push(UiRow {
                    key: (0, 0),
                    body: Body::WorkLine {
                        turn,
                        // Unfolded work is always shown; the header only labels it.
                        expandable: has_work && ready && self.fold_work,
                    },
                    message,
                    first: false,
                    last: false,
                    work: false,
                    work_tail: false,
                    thought_phase: None,
                });
            }
            let phase = activity.at(at);
            // A lone tool call gets the same header as a run of them ("Ran a
            // command"), opened to see the step. A milestone stays its own
            // line, and a lone thought is shown as it is.
            let grouped = phase.is_some_and(|group| {
                group.steps > 1 || (group.calls.len() == 1 && !group.calls[0].is_milestone())
            });
            let active = phase.is_some_and(|group| {
                self.streaming
                    && (group.end == self.source.len() || awaiting_answer == Some(group.end))
            });
            let phase_open = phase.is_none_or(|group| {
                !grouped
                    || self
                        .phases
                        .get(&group.start)
                        .copied()
                        .unwrap_or(self.phase_opens(active))
            });
            // A running pill turn shows one status line in place of its work
            // and prose; a settled one's card leads its reply.
            let git_stand_in = git_card.clone().filter(|card| {
                !is_prompt
                    && if card.live() {
                        matches!(role.as_str(), "tool" | "reasoning" | "assistant")
                    } else {
                        card.answer == Some(at)
                    }
            });
            if let Some(card) = &git_stand_in
                && !(card.live() && git_live_shown)
            {
                git_live_shown = true;
                self.rows.push(UiRow {
                    key: (0, 0),
                    body: Body::GitAction(card.clone()),
                    message,
                    first: false,
                    last: false,
                    work: false,
                    work_tail: false,
                    thought_phase: None,
                });
            }
            let git_hidden = git_stand_in.as_ref().is_some_and(|card| card.live());
            if !self.mono
                && !folded
                && !git_hidden
                && grouped
                && phase.is_some_and(|group| group.start == at)
            {
                let group = phase.unwrap();
                let (title, icon) = if group.calls.is_empty() {
                    (if active { "Thinking…" } else { "Thought" }.into(), "minus")
                } else {
                    crate::chat::transcript::activity::summary(&group.calls, active)
                };
                self.rows.push(UiRow {
                    key: (0, 0),
                    body: Body::Phase {
                        title,
                        group: group.start,
                        icon,
                        active,
                    },
                    message,
                    first: false,
                    last: false,
                    work: false,
                    work_tail: false,
                    thought_phase: None,
                });
            }
            let reaction =
                self.mono && role == "assistant" && text.trim().starts_with("<reaction>");
            if git_hidden {
                if role == "tool" {
                    self.seen_steps.insert(at);
                }
            } else if role == "interjection" {
                if let Ok(note) = serde_json::from_str(&text) {
                    self.rows.push(UiRow {
                        key: (0, 0),
                        body: Body::Interjection(note),
                        message,
                        first: false,
                        last: false,
                        work: false,
                        work_tail: false,
                        thought_phase: None,
                    });
                }
            } else if role == "remote-plan" {
                if let Ok(plan) = serde_json::from_str(&text) {
                    self.rows.push(UiRow {
                        key: (0, 0),
                        body: Body::RemotePlan(plan),
                        message,
                        first: false,
                        last: false,
                        work: false,
                        work_tail: false,
                        thought_phase: None,
                    });
                }
            } else if role == "visual" {
                let document =
                    serde_json::from_str::<monocode_core::session::visual::VisualDocument>(&text)
                        .ok()
                        .filter(|document| document.validate().is_ok());
                // An answer that places the visual draws it in its prose. While
                // the turn runs, the call that published it stands for it, so
                // it never appears above the answer and then jumps into it.
                let elsewhere = document.as_ref().is_some_and(|document| {
                    self.placed.contains(&document.id) || (self.streaming && at >= self.live_start)
                });
                if !elsewhere {
                    let body = match document {
                        Some(document) => Body::Visual(Arc::new(document)),
                        None => Body::Plain("This visual could not be loaded.".into()),
                    };
                    self.rows.push(UiRow {
                        key: (0, 0),
                        body,
                        message,
                        first: false,
                        last: false,
                        work: false,
                        work_tail: false,
                        thought_phase: None,
                    });
                }
            } else if matches!(role.as_str(), "mono-card" | "mono-note") {
                if let Ok(value) = serde_json::from_str(&text) {
                    self.rows.push(UiRow {
                        key: (0, 0),
                        body: Body::MonoCard(value),
                        message,
                        first: false,
                        last: false,
                        work: false,
                        work_tail: false,
                        thought_phase: None,
                    });
                }
            } else if stopped_notice
                || matches!(
                    role.as_str(),
                    "mono-document"
                        | "mono-launch"
                        | monocode_core::session::transcript::attachments::ROLE
                )
            {
                // Saved work references are rendered once under the settled reply.
            } else if reaction {
                // MonoReactionBadge: the emoji sits on the message it answers.
                if let Some(emoji) = text
                    .trim()
                    .strip_prefix("<reaction>")
                    .and_then(|s| s.strip_suffix("</reaction>"))
                    .filter(|e| e.chars().count() <= 16)
                    && let Some(prompt) = self.source[..at].iter().rposition(|r| r.is_prompt())
                {
                    let prompt = self.source[..=prompt]
                        .iter()
                        .rposition(|r| r.first)
                        .unwrap_or(prompt);
                    let live = self.streaming && at >= self.live_start && self.visible;
                    match self.reactions.get(&prompt) {
                        Some((current, _)) if current == emoji => {}
                        _ => {
                            self.reactions
                                .insert(prompt, (emoji.into(), live.then_some(now)));
                        }
                    }
                }
            } else if self.mono
                && (matches!(role.as_str(), "tool" | "reasoning")
                    || (role == "assistant" && self.mono_prose_hidden(at)))
            {
                // groupMonoTurnItems: the header carries the process, and the
                // prose said while working stays in the activity trail.
                if role == "tool" {
                    self.seen_steps.insert(at);
                }
            } else if folded || activity.hidden.contains(&at) || !phase_open {
                if role == "tool" {
                    self.seen_steps.insert(at);
                }
                // WorkFoldLine puts earlier commentary and work behind its receipt.
            } else if role == "tool"
                && let Ok(tool) = serde_json::from_str::<ToolCall>(&text)
            {
                self.rows.push(UiRow {
                    key: (0, 0),
                    body: Body::Tool {
                        call: Box::new(tool),
                        group: phase.filter(|_| grouped).map(|group| group.start),
                        tail: phase.is_none_or(|group| group.last == at),
                        live: active,
                    },
                    message,
                    first: false,
                    last: false,
                    work: false,
                    work_tail: false,
                    thought_phase: None,
                });
            } else if matches!(role.as_str(), "assistant" | "reasoning") {
                if role == "reasoning" {
                    self.rows.push(UiRow {
                        key: (0, 0),
                        body: Body::Reasoning,
                        message,
                        first: false,
                        last: false,
                        work: false,
                        work_tail: false,
                        thought_phase: None,
                    });
                }
                self.prepare_reveal(at, end, &text, role == "assistant", now, cx.reduce_motion());
                parser.set_text(self.reveals.get(&at).map_or(text.as_str(), PacedText::text));
                let tree = if self.reveals.get(&at).is_some_and(PacedText::behind)
                    || (self.streaming && end == self.source.len())
                {
                    parser.display_tree()
                } else {
                    parser.finish_display();
                    parser.tree().clone()
                };
                let mut word_cursor = 0;
                let mut cached = Vec::new();
                for (block_ix, block) in tree
                    .blocks
                    .iter()
                    .filter(|_| role != "reasoning" || self.expanded.contains(&at))
                    .enumerate()
                {
                    let (rows, words) = if let Some(old) = markdown_blocks
                        .get(block_ix)
                        .filter(|old| Arc::ptr_eq(&old.source, block))
                        .filter(|old| {
                            old.rows
                                .first()
                                .is_none_or(|row| row.word_start == word_cursor)
                        }) {
                        (old.rows.clone(), old.words)
                    } else {
                        let mut base = word_cursor;
                        let rows = markdown::rows(&block.block)
                            .into_iter()
                            .map(|mut row| {
                                row.word_start = base;
                                base += markdown::word_count(&row);
                                Arc::new(row)
                            })
                            .collect();
                        (rows, base - word_cursor)
                    };
                    for row in &rows {
                        self.rows.push(UiRow {
                            key: (0, 0),
                            body: Body::Markdown(row.clone()),
                            message,
                            first: false,
                            last: false,
                            work: false,
                            work_tail: false,
                            thought_phase: None,
                        });
                    }
                    word_cursor += words;
                    cached.push(RenderedBlock {
                        source: block.clone(),
                        rows,
                        words,
                    });
                }
                markdown_blocks = cached;
                if let Some(paced) = self.reveals.get_mut(&at) {
                    paced.words(word_cursor, now);
                }
            } else {
                let text_rows = if let Some(text) = display_text {
                    let mut transcript = monocode_core::session::transcript::Transcript::default();
                    transcript.begin("user");
                    transcript.append(&text);
                    transcript
                        .rows
                        .into_iter()
                        .map(|row| row.text)
                        .collect::<Vec<_>>()
                } else {
                    self.source[at..end]
                        .iter()
                        .filter(|r| r.role != "mono-system")
                        .map(|row| row.text.clone())
                        .collect()
                };
                if role == "user" {
                    user_link = text_rows.iter().enumerate().find_map(|(chunk, text)| {
                        parse_user_message_link(text).map(|link| (chunk, Arc::new(link)))
                    });
                }
                // `line-clamp-4`: a collapsed message shows its first lines only.
                // A message with a link preview is never clamped, as in React.
                let clamped = role == "user"
                    && user_link.is_none()
                    && !self.expanded_prompts.contains(&at)
                    && text_rows.len() > 1;
                if clamped {
                    self.clamped_prompts.insert(at);
                } else {
                    self.clamped_prompts.remove(&at);
                }
                let shown = if clamped { 1 } else { text_rows.len() };
                for text in text_rows.into_iter().take(shown) {
                    self.rows.push(UiRow {
                        key: (0, 0),
                        body: Body::Plain(text.into()),
                        message,
                        first: false,
                        last: false,
                        work: false,
                        work_tail: false,
                        thought_phase: None,
                    });
                }
            }
            let mut prose_ordinal = 16;
            for row in &mut self.rows[first..] {
                if role == "reasoning" && matches!(row.body, Body::Reasoning | Body::Markdown(_)) {
                    row.thought_phase = phase
                        .filter(|_| grouped)
                        .map(|group| (group.start, group.last == at));
                }
                let ordinal = match row.body {
                    Body::WorkLine { .. } => 0,
                    Body::Phase { .. } => 1,
                    Body::Tool { .. } => 2,
                    Body::Reasoning => 4,
                    _ => {
                        let key = prose_ordinal;
                        prose_ordinal += 1;
                        key
                    }
                };
                row.key = (at, ordinal);
                // Unfolded work lays out like the answer: no rail, indent or
                // dimmed text. Tool groups keep their own connectors.
                row.work = self.fold_work
                    && has_work
                    && ready
                    && role != "visual"
                    && !activity.pinned.contains(&at)
                    && final_answer.is_some_and(|answer| at < answer)
                    && !matches!(row.body, Body::WorkLine { .. });
            }
            if let Some(row) = self.rows.get_mut(first) {
                row.first = true;
            }
            if self.rows.len() > first {
                self.rows.last_mut().unwrap().last = true;
            }
            let prompt_fingerprint = if role == "user" {
                measurements::prompt_fingerprint(&self.rows[first..], &attachments)
            } else {
                0
            };
            self.messages.push(Message {
                start: at,
                display_start: first,
                role,
                parser,
                markdown_blocks,
                user_link,
                // Only a settled turn folds its work under the receipt line.
                // While it streams, the newest reply is the answer so far but
                // still follows open tool rows, so it keeps prose's 12px gap
                // instead of jumping when the next tool or the end arrives.
                under_line: has_work
                    && final_answer == Some(at)
                    && self.fold_work
                    && !(self.streaming && turn_end == self.source.len()),
                turn: if self.mono {
                    self.mono_group_start(turn)
                } else {
                    turn
                },
                attachments,
                prompt_fingerprint,
            });
            let last_prompt = self.source.iter().rposition(|r| r.is_prompt());
            if self.mono
                && is_prompt
                && self.mono_pending_joins_live_turn()
                && last_prompt.is_some_and(|p| end > p)
            {
                // The running turn keeps the user's messages together above its work.
                self.push_pending(at);
            }
            if self.mono && !is_prompt && end == turn_end {
                // Starting a provider precedes its hidden notification prompt.
                // Cards from the completed run stay in place during that gap.
                let settled = !(self.streaming
                    && turn_end == self.source.len()
                    && turn_end > self.live_start);
                if settled {
                    let mut documents = Vec::new();
                    for row in &self.source[self.prompt_start(turn)..turn_end] {
                        if row.role == "mono-document"
                            && let Ok(value) = serde_json::from_str::<serde_json::Value>(&row.text)
                        {
                            if let Some(old) =
                                documents.iter_mut().find(|old: &&mut serde_json::Value| {
                                    old["documentId"] == value["documentId"]
                                })
                            {
                                *old = value;
                            } else {
                                documents.push(value);
                            }
                        }
                    }
                    for (k, value) in documents.into_iter().enumerate() {
                        self.rows.push(UiRow {
                            key: (at, usize::MAX - 128 + k.min(60)),
                            body: Body::MonoCard(value),
                            message,
                            first: false,
                            last: false,
                            work: false,
                            work_tail: false,
                            thought_phase: None,
                        });
                    }
                    for (k, id) in self.run_launches(turn, turn_end).into_iter().enumerate() {
                        self.rows.push(UiRow {
                            key: (at, usize::MAX - 64 + k.min(60)),
                            body: Body::Launch(id),
                            message,
                            first: false,
                            last: false,
                            work: false,
                            work_tail: false,
                            thought_phase: None,
                        });
                    }
                }
            }
            if !self.document
                && !is_prompt
                && self.source[at].role != "mono-card"
                && self.source[at].role != monocode_core::session::transcript::attachments::ROLE
                && !reaction
                && end == turn_end
                && !(self.mono && self.mono_next_run_continues(turn, turn_end))
            {
                self.rows.push(UiRow {
                    key: (at, usize::MAX - 1),
                    body: Body::Actions {
                        turn: if self.mono {
                            self.mono_group_start(turn)
                        } else {
                            turn
                        },
                        complete: !self.streaming || end < self.source.len(),
                    },
                    message,
                    first: false,
                    last: false,
                    work: false,
                    work_tail: false,
                    thought_phase: None,
                });
            }
            at = end;
        }
        self.prepared_parsers.clear();
        if self.mono && !self.rows.iter().any(|r| matches!(r.body, Body::Pending(_))) {
            self.push_pending(self.source.len());
        }
        if self.mono {
            // An accepted steer stays beside the original user prompt, both
            // while streaming and after the provider's final answer is saved.
            let messages = &self.messages;
            let pending_group = self.mono_pending_joins_live_turn().then(|| {
                let latest = self
                    .source
                    .iter()
                    .rposition(Row::is_prompt)
                    .map_or(0, |at| at + 1);
                self.mono_group_start(latest)
            });
            let tail = self.source.len();
            self.rows[display_start..].sort_by_key(|row| {
                let message = messages.get(row.message);
                let pending = matches!(row.body, Body::Pending(_));
                let group = message
                    .map(|m| m.turn)
                    .or_else(|| pending.then_some(pending_group).flatten())
                    .unwrap_or(tail);
                (
                    group,
                    !(pending || message.is_some_and(|m| m.role == "user")),
                    row.key,
                )
            });
            mono_divider::insert(&mut self.rows, &self.messages, display_start);
        }
        self.push_saved_draft();
        let mut pinned_answers = HashMap::new();
        if !activity.pinned.is_empty() {
            let mut parked: HashMap<usize, Vec<UiRow>> = HashMap::new();
            let mut ordered = Vec::new();
            for row in self.rows.drain(display_start..) {
                let answer = self
                    .fold_targets
                    .get(&self.messages[row.message].turn)
                    .copied()
                    .flatten();
                if activity.pinned.contains(&row.key.0)
                    && matches!(row.body, Body::Tool { .. })
                    && let Some(answer) = answer.filter(|answer| row.key.0 < *answer)
                {
                    pinned_answers.insert(row.key.0, answer);
                    parked.entry(answer).or_default().push(row);
                } else {
                    let mut due: Vec<_> = parked
                        .keys()
                        .copied()
                        .filter(|answer| *answer <= row.key.0)
                        .collect();
                    due.sort_unstable();
                    for answer in due {
                        ordered.extend(parked.remove(&answer).unwrap());
                    }
                    ordered.push(row);
                }
            }
            // A folded answer is always present; retain runs defensively if
            // a transient source patch has not made its prose row yet.
            let mut remaining: Vec<_> = parked.into_iter().collect();
            remaining.sort_by_key(|(answer, _)| *answer);
            for (_, runs) in remaining {
                ordered.extend(runs);
            }
            self.rows.extend(ordered);
        }
        // A phase and its nested steps are one sibling in the outer work tree.
        // Its final outer branch ends at the header, above the inner tool rail.
        let mut next_work_sibling = false;
        for row in self.rows[display_start..].iter_mut().rev() {
            row.work_tail = row.work && !next_work_sibling;
            if !row.in_phase() {
                next_work_sibling = row.work;
            }
        }
        let target = self.rows.split_off(display_start);
        let merged = if self.visible && !cx.reduce_motion() {
            let replaced_work = self
                .fold_targets
                .get(&current_turn)
                .copied()
                .flatten()
                .filter(|_| {
                    !self
                        .work_folds
                        .get(&current_turn)
                        .copied()
                        .unwrap_or(!self.fold_work)
                })
                .filter(|_| layout_changed)
                .map(|answer| current_turn..answer);
            if replaced_work.is_some() {
                // A replacement establishes a new block's baseline. The old
                // block's removed height is not a scroll animation to replay.
                self.follow.measured = None;
            }
            self.reconcile(old_rows, target, now, &pinned_answers, replaced_work)
        } else {
            self.motions.clear();
            target
        };
        self.phase_toggle = None;
        self.rows.extend(merged);
        self.windowed_phases = self
            .rows
            .iter()
            .filter_map(|row| match row.body {
                Body::Phase { group, active, .. }
                    if active
                        || (self.phase_scrolls.contains_key(&group)
                            && !self
                                .phases
                                .get(&group)
                                .copied()
                                .unwrap_or(self.phase_opens(active))) =>
                {
                    Some(group)
                }
                _ => None,
            })
            .collect();
        // A closing live phase keeps its bounded window for the exit. Once
        // settled, an explicitly opened group returns to virtualized rows.
        let opens_running = self.phase_opens(true);
        for (&group, scroll) in &mut self.phase_scrolls {
            let enabled = self.visible && self.rows.iter().any(|row| {
                matches!(row.body, Body::Phase { group: id, active: true, .. } if id == group)
            }) && self.phases.get(&group).copied().unwrap_or(opens_running);
            scroll.enable(enabled);
        }
        self.index_messages();
        // Preserve surviving indices and scroll offsets while the tail reflows.
        let common = old_count.min(self.rows.len());
        self.measurements.rows_changed();
        for ix in display_start..common {
            let (old, after_receipt) = &old_layouts[ix - display_start];
            if !measurements::same_layout(old, &self.rows[ix])
                || *after_receipt != measurements::after_receipt(&self.rows, &self.messages, ix)
            {
                self.list.remeasure_items(ix..ix + 1);
            }
        }
        if old_count != self.rows.len() {
            self.list
                .splice(common..old_count, self.rows.len() - common);
        }
        self.list
            .remeasure_items(self.footer_ix()..self.footer_ix() + 1);
        if self.mono && display_start > 0 {
            // A new message can stack the one before it, changing its spacing.
            self.list
                .remeasure_items(display_start.saturating_sub(8)..display_start.min(common));
        }
        if !self.hover_rows.is_empty() {
            let keys: HashSet<_> = self.rows.iter().map(|r| r.key).collect();
            self.hover_rows.retain(|key, _| keys.contains(key));
        }
        if following {
            self.list.scroll_to_end();
        } else if let Some(key) = anchor {
            if let Some(item_ix) = self.rows.iter().position(|r| r.key == key) {
                self.list.scroll_to(ListOffset {
                    item_ix,
                    offset_in_item: offset.offset_in_item,
                });
            } else if let Some(turn) = anchor_turn {
                // The row being read folded away, such as tool calls joining
                // their turn's work line when the answer arrives. Stay in that
                // turn, on the nearest surviving row at or above it, else the
                // first below, rather than on whatever now holds the old
                // index or the index GPUI's splice moved the scroll to.
                let in_turn =
                    |r: &UiRow| self.messages.get(r.message).is_some_and(|m| m.turn == turn);
                let item_ix = self
                    .rows
                    .iter()
                    .rposition(|r| in_turn(r) && r.key <= key)
                    .or_else(|| self.rows.iter().position(|r| in_turn(r) && r.key > key));
                if let Some(item_ix) = item_ix {
                    self.list.scroll_to(ListOffset {
                        item_ix,
                        offset_in_item: px(0.),
                    });
                }
            }
        }
        self.prune_reveals(now);
    }
}
