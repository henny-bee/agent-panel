use super::*;

impl TranscriptView {
    /// Stop visible work immediately, while the provider drains and saves.
    pub(crate) fn stop_streaming(&mut self, cx: &mut Context<Self>) {
        let start = self
            .source
            .iter()
            .rposition(Row::is_prompt)
            .map_or(0, |at| at + 1);
        for row in self
            .source
            .iter_mut()
            .skip(start)
            .filter(|row| row.role == "tool")
        {
            if let Ok(mut tool) = serde_json::from_str::<ToolCall>(&row.text)
                && tool.state == ToolState::Running
            {
                tool.state = ToolState::Cancelled;
                if tool.output.is_empty() {
                    tool.output = "Cancelled".into();
                }
                let previous_bytes = row.text.len();
                row.text = serde_json::to_string(&tool).unwrap_or_else(|_| row.text.clone());
                self.source_bytes = self
                    .source_bytes
                    .saturating_sub(previous_bytes)
                    .saturating_add(row.text.len());
            }
        }
        let now = Instant::now();
        for reveal in self.reveals.values_mut() {
            reveal.settle(now);
        }
        self.streaming(false, cx);
        self.rebuild(start, cx);
        cx.notify();
    }

    pub(crate) fn streaming(&mut self, streaming: bool, cx: &mut Context<Self>) {
        if self.streaming == streaming {
            return;
        }
        // What the user already reads while the Mono works, such as its
        // opening sentence, must not enter again when the run settles.
        let shown: HashSet<usize> = if self.mono && !streaming {
            let run = self
                .source
                .iter()
                .rposition(|r| r.is_prompt())
                .map_or(0, |i| i + 1);
            (run..self.source.len())
                .filter(|&at| self.source[at].first && !self.mono_prose_hidden(at))
                .collect()
        } else {
            HashSet::new()
        };
        self.streaming = streaming;
        if streaming {
            self.active_provider_turn_id = None;
            self.live_start = self.source.len();
            // The provider starts before its prompt arrives. Keep an ordinary
            // chat's existing spacer and scroll position until patch accepts
            // that prompt, just as React pins on lastUserId rather than busy.
            // Otherwise the old spacer collapses for a frame before the new
            // prompt gets its own anchor and entrance.
            if self.mono {
                self.show_jump = false;
                self.list.set_follow_mode(FollowMode::Tail);
            }
            self.started = Some(std::time::Instant::now());
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(1))
                        .await;
                    let keep = this
                        .update(cx, |this, cx| {
                            // The working time redraws the whole transcript;
                            // skip it while the tab or the app is out of view.
                            if this.streaming
                                && this.visible
                                && crate::ui::motion::pulse::app_active(cx)
                            {
                                cx.notify_paint();
                            }
                            this.streaming
                        })
                        .unwrap_or(false);
                    if !keep {
                        break;
                    }
                }
            })
            .detach();
        } else {
            self.follow.until = Some(Instant::now() + std::time::Duration::from_millis(1200));
            if let Some(started) = self.started.take()
                && let Some(turn) = self.active_turn()
            {
                self.elapsed.insert(turn, started.elapsed());
                let completed_at = monocode_core::sys::time::now_ms();
                let started_at =
                    Some(completed_at.saturating_sub(started.elapsed().as_millis() as u64));
                let info = self.turns.entry(turn).or_default();
                info.provider = self.provider.clone();
                info.model = self.model.clone();
                info.started_at = started_at;
                info.duration_ms = started.elapsed().as_millis() as u64;
                info.duration_known = true;
                info.completed_at = started_at.and_then(|at| at.checked_add(info.duration_ms));
            }
        }
        if !streaming && self.mono && self.visible && !cx.reduce_motion() {
            // The reply a Mono kept in its trail while it worked appears now.
            let now = Instant::now();
            let run = self
                .source
                .iter()
                .rposition(|r| r.is_prompt())
                .map_or(0, |i| i + 1);
            for at in run..self.source.len() {
                let row = &self.source[at];
                if row.first
                    && row.role == "assistant"
                    && !shown.contains(&at)
                    && !row.text.trim_start().starts_with("<reaction>")
                {
                    self.entrances.insert(at, now);
                }
            }
        }
        // Starting a normal turn must not re-open the previous turn's work or
        // rebuild it as live while the new prompt is still in transit.
        if (!streaming || self.mono) && !self.messages.is_empty() {
            let start = self
                .source
                .iter()
                .rposition(|r| r.is_prompt())
                .map_or(0, |i| i + 1);
            self.rebuild(start, cx);
        } else {
            cx.notify();
        }
    }

    /// Latest assistant message, including its virtualized continuation rows.
    pub(crate) fn notification_reply_body(&self) -> String {
        let Some(end) = self
            .source
            .iter()
            .rposition(|row| row.role == "assistant" && !row.text.trim().is_empty())
        else {
            return String::new();
        };
        let mut start = end;
        while start > 0 && !self.source[start].first && self.source[start - 1].role == "assistant" {
            start -= 1;
        }
        let mut text = String::new();
        for row in &self.source[start..=end] {
            text.push_str(&row.text);
            // A notification needs only the first paragraph and 240 code units.
            // Retain enough extra text to distinguish a 240-unit body from one
            // requiring the ellipsis, without cloning a giant reply.
            if text.encode_utf16().count() > 480 {
                break;
            }
        }
        crate::notifications::delivery::clip_body(&text)
    }

    /// Tauri's appendUser: paint the prompt and working receipt in the send
    /// handler. The worker replaces these same rows after saving them. Build
    /// the attachment manifest without touching the filesystem on the UI thread.
    pub(crate) fn submit_prompt(
        &mut self,
        prompt: &str,
        settings: &monocode_core::session::agent::Settings,
        cx: &mut Context<Self>,
    ) {
        let mut submitted = monocode_core::session::transcript::Transcript::default();
        submitted.begin(if settings.internal_message {
            "mono-system"
        } else {
            "user"
        });
        submitted.rows[0].sent_at = settings
            .mono_sent_at
            .or_else(|| {
                settings
                    .turn_model
                    .as_ref()
                    .and_then(|turn| turn.started_at)
            })
            .or(submitted.rows[0].sent_at);
        submitted.append(prompt);
        if !settings.attachments.is_empty() {
            submitted.rows.push(Row {
                role: monocode_core::session::transcript::attachments::ROLE.into(),
                text: monocode_core::session::transcript::attachments::from_paths(
                    &settings.attachments,
                )
                .to_string(),
                first: true,
                sent_at: None,
            });
        }
        submitted.begin("assistant");
        if !settings.internal_message && !self.mono && self.visible && !cx.reduce_motion() {
            self.burst = burst::Kind::from_prompt(prompt, settings)
                .map(|kind| burst::Burst::new(self.source.len(), kind));
        }
        if self.mono {
            self.submitted_mono_message = settings.mono_message_id.clone();
        }
        self.patch_view(self.source.len(), submitted.rows, cx);
    }

    /// Patch the store's window from `start`, a row of `source_rows`.
    pub(crate) fn patch(&mut self, start: usize, rows: Vec<Row>, cx: &mut Context<Self>) {
        self.patch_view(self.base + start, rows, cx);
    }

    pub(super) fn patch_view(&mut self, start: usize, rows: Vec<Row>, cx: &mut Context<Self>) {
        if rows.is_empty() {
            return;
        }
        assert!(
            start <= self.source.len(),
            "transcript patches must be contiguous"
        );
        let known = self.source.len();
        for (ix, row) in rows.into_iter().enumerate() {
            let ix = start + ix;
            let bytes = row.text.capacity().saturating_add(row.role.capacity());
            if ix < self.source.len() {
                let old = &self.source[ix];
                self.source_bytes = self
                    .source_bytes
                    .saturating_sub(old.text.capacity().saturating_add(old.role.capacity()));
                self.source[ix] = row;
            } else {
                self.source.push(row);
            }
            self.source_bytes = self.source_bytes.saturating_add(bytes);
        }
        self.visuals.retain(|&start, _| {
            self.source
                .get(start)
                .is_some_and(|row| row.role == "visual")
        });
        if start == 0 {
            self.placed.clear();
        }
        self.published.retain(|_, (at, _)| *at < start);
        let placed = self.placed.len();
        for row in &self.source[start..] {
            if row.role == "assistant" {
                visuals::placements(&row.text, &mut self.placed);
            }
        }
        let placed = self.placed.len() != placed;
        if known == 0 && self.mono && !self.streaming {
            self.history_start = history::page_start(&self.source, self.source.len());
        }
        if self.mono {
            let affected = self.source[..start.min(self.source.len())]
                .iter()
                .rposition(|row| row.first && row.is_prompt())
                .unwrap_or(0);
            self.mono_groups
                .get_mut()
                .retain(|&turn, _| turn < affected);
        }
        if self.streaming
            && let Some(id) = self.active_provider_turn_id.clone()
            && let Some(turn) = self.active_turn()
        {
            self.turns.entry(turn).or_default().provider_turn_id = Some(id);
        }
        if self.streaming
            && let Some(turn) = self.active_turn()
        {
            let provider = self.provider.clone();
            let model = self.model.clone();
            self.turns
                .entry(turn)
                .or_insert_with(|| monocode_core::session::agent::TurnInfo {
                    provider,
                    model,
                    ..Default::default()
                });
        }
        if self.streaming {
            for prompt in known..self.source.len() {
                if self.source[prompt].first && self.source[prompt].is_prompt() {
                    let turn = (prompt + 1..self.source.len())
                        .find(|&at| self.source[at].first)
                        .unwrap_or(self.source.len());
                    let provider = self.provider.clone();
                    let model = self.model.clone();
                    let provider_turn_id = self.active_provider_turn_id.clone();
                    let joined_turn = self
                        .active_turn()
                        .filter(|&root| root < turn && self.source[prompt].role == "user");
                    self.turns.entry(turn).or_insert_with(|| {
                        monocode_core::session::agent::TurnInfo {
                            provider,
                            model,
                            provider_turn_id,
                            joined_turn,
                            ..Default::default()
                        }
                    });
                }
            }
        }
        // React pins on a change to lastUserId, including an accepted steer
        // inside the current provider turn. Continuation rows and replayed
        // snapshots must not move the anchor or resume following the tail.
        if self.streaming
            && !self.mono
            && let Some(prompt) = (known..self.source.len())
                .rfind(|&ix| self.source[ix].first && self.source[ix].is_prompt())
        {
            self.anchor_turn = self.anchor_prompts.then_some(prompt);
            // React's ordinary transcript introduces a sent prompt itself.
            // Start a fresh follow baseline instead of adding a second glide.
            self.follow = Follow::default();
            self.show_jump = false;
            self.list.set_follow_mode(FollowMode::Tail);
        }
        if let Some(prompt) = self.anchor_turn
            && self.source.get(prompt).is_some_and(|r| r.is_prompt())
            && self.introduced_prompt != Some(prompt)
            && self.streaming
            && start <= prompt
        {
            self.introduced_prompt = Some(prompt);
            if self.visible && !cx.reduce_motion() {
                self.prompt_rise = Some((prompt, Instant::now()));
                self.prompt_rise_pending = true;
            }
        }
        // Messages that arrive while the chat is open slide and fade in;
        // history that loads or replays does not.
        if self.mono && self.streaming && self.visible && !cx.reduce_motion() {
            let now = Instant::now();
            for ix in start.max(known)..self.source.len() {
                let row = &self.source[ix];
                if !row.first || !matches!(row.role.as_str(), "user" | "assistant") {
                    continue;
                }
                let queued = if row.role == "user" {
                    // A long prompt spans storage rows. Match its full text so
                    // wrapping and newline chunking don't create another entrance.
                    let end = (ix + 1..self.source.len())
                        .find(|&at| self.source[at].first)
                        .unwrap_or(self.source.len());
                    let text: String = self.source[ix..end]
                        .iter()
                        .map(|r| r.text.as_str())
                        .collect();
                    self.pending
                        .iter()
                        .find(|message| text.starts_with(message.text.as_ref()))
                        .map(|message| self.pending_entrances.get(&message.id).copied())
                        .or_else(|| {
                            self.taken
                                .iter()
                                .rev()
                                .find(|message| {
                                    now.duration_since(message.removed_at).as_secs_f32() < 2.
                                        && text.starts_with(message.text.as_ref())
                                })
                                .map(|message| message.entrance)
                        })
                } else {
                    None
                };
                // Delivery keeps the same animation clock as the optimistic
                // bubble. A settled bubble stays settled; an in-flight slide
                // continues even when clearing a multiline composer resizes us.
                let entrance = queued.map_or(Some(now), |born| {
                    born.filter(|born| now.duration_since(*born).as_secs_f32() < 0.36)
                });
                if let Some(born) = entrance {
                    self.entrances.insert(ix, born);
                }
            }
        }
        self.outline.patched(&self.source, start);
        // A newly placed visual leaves its own row earlier in the turn, so the
        // whole turn rebuilds rather than just the changed suffix.
        let start = if placed {
            self.source[..start]
                .iter()
                .rposition(|r| r.is_prompt())
                .map_or(0, |i| i + 1)
        } else {
            start
        };
        self.rebuild(start, cx);
    }

    pub(super) fn rebuild(&mut self, start: usize, cx: &mut Context<Self>) {
        self.rebuild_rows(start, cx);
        cx.notify();
    }

    pub(super) fn prune_reveals(&mut self, now: Instant) {
        let latest = self
            .streaming
            .then(|| self.source.iter().rposition(|row| row.first))
            .flatten();
        self.reveals
            .retain(|at, reveal| Some(*at) == latest || reveal.active(now));
    }

    pub(super) fn prepare_reveal(
        &mut self,
        at: usize,
        end: usize,
        text: &str,
        assistant: bool,
        now: Instant,
        reduced: bool,
    ) {
        if !self.reveals.contains_key(&at)
            && !(self.visible
                && self.smooth_reveal
                && !reduced
                && self.streaming
                && end == self.source.len()
                && at >= self.live_start
                && assistant)
        {
            return;
        }
        let paced = self.reveals.entry(at).or_insert_with(|| {
            PacedText::new(
                text,
                self.visible
                    && self.smooth_reveal
                    && !reduced
                    && self.streaming
                    && at >= self.live_start
                    && assistant,
                now,
            )
        });
        paced.update(text, now);
        if !self.visible || !self.smooth_reveal || reduced {
            paced.settle(now);
        }
        paced.step(now, self.streaming && end == self.source.len());
    }

    pub(super) fn answer_visible(&self, answer: usize) -> bool {
        if self.source[answer].text.trim().is_empty() {
            return false;
        }
        if answer < self.live_start || !self.visible || !self.smooth_reveal {
            return true;
        }
        self.reveals
            .get(&answer)
            .is_none_or(|r| !r.text().trim().is_empty())
    }

    pub(super) fn index_messages(&mut self) {
        let mut first = HashMap::new();
        for (ix, row) in self.rows.iter().enumerate() {
            first.entry(row.message).or_insert(ix);
        }
        let mut next = self.rows.len();
        for (ix, message) in self.messages.iter_mut().enumerate().rev() {
            if let Some(at) = first.get(&ix) {
                next = *at;
            }
            message.display_start = next;
        }
    }

    pub(super) fn reconcile(
        &mut self,
        mut old: Vec<UiRow>,
        target: Vec<UiRow>,
        now: Instant,
        pinned_answers: &HashMap<usize, usize>,
        replaced_work: Option<std::ops::Range<usize>>,
    ) -> Vec<UiRow> {
        // Exit rows outlive a source patch, but message ordinals do not.
        let owners: HashMap<_, _> = self
            .messages
            .iter()
            .enumerate()
            .map(|(index, message)| (message.start, index))
            .collect();
        old.retain_mut(|row| {
            if let Some(&message) = owners.get(&row.key.0) {
                row.message = message;
                true
            } else {
                self.motions.remove(&row.key);
                false
            }
        });
        let phase_toggle = self.phase_toggle.take();
        if old.is_empty() && !self.streaming && self.motions.is_empty() {
            return target;
        }
        let order = |row: &UiRow| {
            if matches!(row.body, Body::Tool { .. })
                && let Some(&answer) = pinned_answers.get(&row.key.0)
            {
                (answer, 0, row.key.0, row.key.1)
            } else {
                (row.key.0, 1, row.key.0, row.key.1)
            }
        };
        // Source keys stay stable when runs are lifted below the work. The
        // answer boundary can advance between renders, so place outgoing
        // commentary in the fold above the pinned stack before merging it.
        old.sort_by_key(&order);
        let wanted: HashSet<_> = target.iter().map(|r| r.key).collect();
        let previous: HashMap<_, _> = old.iter().enumerate().map(|(ix, r)| (r.key, ix)).collect();
        let mut merged = Vec::new();
        let mut cursor = 0;
        let source = &self.source;
        let windowed = &self.windowed_phases;
        let keep_old = |row: &UiRow, motions: &mut HashMap<_, Motion>| -> bool {
            // "Show less" hides the rest of a message at once, as in React.
            if matches!(row.body, Body::Plain(_))
                && wanted.contains(&(row.key.0, 16))
                && source.get(row.key.0).is_some_and(|r| r.role == "user")
            {
                return false;
            }
            if matches!(
                row.body,
                Body::Actions { .. }
                    | Body::MonoDivider
                    | Body::Pending(_)
                    | Body::Launch(_)
                    | Body::SavedDraft
            ) {
                return false;
            }
            if wanted.contains(&row.key) {
                return false;
            }
            // React replaces the live prose/work block when its answer moves
            // on. Keeping both blocks in layout for an exit animation can
            // temporarily overflow an otherwise short anchored turn.
            // Explicit disclosure toggles still animate in place.
            if replaced_work
                .as_ref()
                .is_some_and(|range| range.contains(&row.key.0))
            {
                motions.remove(&row.key);
                return false;
            }
            let motion = motions.entry(row.key).or_insert_with(|| {
                if let Some(group) = row
                    .phase_group()
                    .filter(|g| Some(*g) == phase_toggle || windowed.contains(g))
                {
                    Motion::phase(group, false, now)
                } else {
                    Motion::new(false, false, now)
                }
            });
            if motion.opening {
                *motion = motion.redirect(false, now);
            }
            !motion.finished(now)
        };
        for row in target {
            // Outgoing work stays above the new final answer during its exit.
            // New rows have no previous index to anchor the old suffix to.
            while cursor < old.len() && order(&old[cursor]) < order(&row) {
                if keep_old(&old[cursor], &mut self.motions) {
                    merged.push(old[cursor].clone());
                }
                cursor += 1;
            }
            if let Some(&at) = previous.get(&row.key) {
                if at >= cursor {
                    for old in &old[cursor..at] {
                        if keep_old(old, &mut self.motions) {
                            merged.push(old.clone());
                        }
                    }
                    cursor = at + 1;
                }
                if self.motions.get(&row.key).is_some_and(|m| !m.opening) {
                    let motion = self.motions[&row.key].redirect(true, now);
                    self.motions.insert(row.key, motion);
                }
            } else {
                let unseen = if matches!(row.body, Body::Tool { .. }) {
                    self.seen_steps.insert(row.key.0)
                } else {
                    false
                };
                let phase = row.phase_group().filter(|g| Some(*g) == phase_toggle);
                let step = phase.is_none()
                    && self.streaming
                    && row.key.0 >= self.live_start
                    && unseen
                    && !row.work;
                if step
                    || matches!(row.body, Body::Tool { .. } | Body::Reasoning)
                    || self.expanded.contains(&row.key.0)
                    || row.work
                {
                    let mut motion = phase.map_or_else(
                        || Motion::new(true, step, now),
                        |group| Motion::phase(group, true, now),
                    );
                    if step {
                        let group = match row.body {
                            Body::Tool { group, .. } => group.unwrap_or(row.key.0),
                            _ => row.key.0,
                        };
                        let start = self.step_queue.get(&group).copied().unwrap_or(now).max(now);
                        let wait = start.duration_since(now).as_secs_f32();
                        let pace = (0.48 * ((2. - wait) / (2. - 0.96)).min(1.)).max(0.16);
                        motion.start = start;
                        motion.duration = std::time::Duration::from_secs_f32(pace);
                        self.step_queue.insert(group, start + motion.duration);
                    }
                    self.motions.insert(row.key, motion);
                }
            }
            merged.push(row);
        }
        for row in &old[cursor..] {
            if keep_old(row, &mut self.motions) {
                merged.push(row.clone());
            }
        }
        merged
    }

    pub(super) fn schedule_frame(
        &mut self,
        resized: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.visible || cx.reduce_motion() || self.frame_scheduled {
            return;
        }
        let now = Instant::now();
        let smooth = resized
            || !self.motions.is_empty()
            || self.agent_panels.animating()
            || self
                .prompt_rise
                .is_some_and(|(_, born)| born.elapsed().as_secs_f32() < 0.88)
            || self.burst.is_some_and(|burst| burst.active(now))
            || self.chat_animating();
        if smooth {
            self.frame_scheduled = true;
            let due = self.last_motion_frame.map(|at| at + MOTION_FRAME);
            motion_frame(cx.entity().downgrade(), due, window);
        } else if self.stream_animating(now) {
            // Stream presentation keeps the shared 30Hz budget. Pure fade
            // frames reuse the parsed blocks, prepared text and measured rows.
            self.frame_scheduled = true;
            let view = cx.entity().downgrade();
            crate::ui::motion::pulse::request_frame(
                crate::ui::motion::pulse::Rate::Smooth,
                window,
                cx,
                move |cx| {
                    let _ = view.update(cx, |this, cx| this.animate_frame(cx));
                },
            );
        }
    }

    pub(super) fn animate_frame(&mut self, cx: &mut Context<Self>) {
        self.last_motion_frame = Some(Instant::now());
        self.frame_scheduled = false;
        // A frame queued before switching tabs must not invalidate the hidden
        // transcript's retained layout or advance its presentation animations.
        if !self.visible {
            return;
        }
        let mut changed_phases = HashSet::new();
        // Revealing words keeps every size but the revealed rows', which
        // measure again by their content; anything else here resizes rows.
        let mut resized = self.agent_panels.animating() || !self.motions.is_empty();
        for start in self.agent_panels.animate() {
            for (ix, row) in self.rows.iter().enumerate() {
                if row.key.0 == start {
                    self.list.remeasure_items(ix..ix + 1);
                    changed_phases.extend(row.phase_group());
                }
            }
        }
        let now = Instant::now();
        let mut changed = None;
        for (&at, reveal) in &mut self.reveals {
            if reveal.step(now, self.streaming) {
                changed = Some(changed.map_or(at, |old: usize| old.min(at)));
            }
        }
        if let Some(start) = changed {
            self.rebuild_rows(start, cx);
        }
        let mut removed = Vec::new();
        let offset = self.list.logical_scroll_top();
        let anchor = (!self.list.is_following_tail())
            .then(|| self.rows.get(offset.item_ix).map(|r| r.key))
            .flatten();
        for (ix, row) in self.rows.iter().enumerate() {
            if let Some(motion) = self.motions.get(&row.key) {
                changed_phases.extend(row.phase_group());
                if motion.finished(now) && !motion.opening {
                    removed.push(ix);
                } else {
                    self.list.remeasure_items(ix..ix + 1);
                }
            }
        }
        for (ix, row) in self.rows.iter().enumerate() {
            if matches!(row.body, Body::Phase { group, .. } if changed_phases.contains(&group) && self.windowed_phase(group))
            {
                self.list.remeasure_items(ix..ix + 1);
            }
        }
        for ix in removed.into_iter().rev() {
            self.rows.remove(ix);
            self.list.splice(ix..ix + 1, 0);
        }
        if let Some(key) = anchor
            && let Some(item_ix) = self.rows.iter().position(|r| r.key == key)
            && item_ix != offset.item_ix
        {
            self.list.scroll_to(ListOffset {
                item_ix,
                offset_in_item: offset.offset_in_item,
            });
        }
        self.motions.retain(|_, motion| !motion.finished(now));
        self.burst = self.burst.filter(|burst| burst.active(now));
        self.entrances
            .retain(|_, born| now.duration_since(*born).as_secs_f32() < 0.4);
        self.pending_entrances
            .retain(|_, born| now.duration_since(*born).as_secs_f32() < 0.4);
        self.prune_reveals(now);
        for (_, born) in self.reactions.values_mut() {
            if born.is_some_and(|b| now.duration_since(b).as_secs_f32() >= 0.4) {
                *born = None;
            }
        }
        self.list
            .remeasure_items(self.footer_ix()..self.footer_ix() + 1);
        self.index_messages();
        resized |= self.agent_panels.animating() || !self.motions.is_empty();
        if resized {
            cx.notify();
        } else {
            cx.notify_paint();
        }
    }

    pub(super) fn highlight(&mut self, fence: &Arc<Fence>, cx: &mut Context<Self>) {
        if self.highlights.contains_key(&fence.key) || self.highlighting.contains(&fence.key) {
            return;
        }
        // Replace an obsolete queued version of a growing fence before it starts.
        self.highlight_queue.retain(|queued| {
            let obsolete =
                fence.language == queued.language && fence.code.starts_with(queued.code.as_ref());
            if obsolete {
                self.highlighting.remove(&queued.key);
            }
            !obsolete
        });
        if self.highlight_queue.len() >= 8 {
            return;
        }
        self.highlighting.insert(fence.key);
        self.highlight_queue.push_back(fence.clone());
        self.pump_highlights(cx);
    }

    pub(super) fn pump_highlights(&mut self, cx: &mut Context<Self>) {
        if self.highlight_active {
            return;
        }
        let Some(fence) = self.highlight_queue.pop_front() else {
            return;
        };
        self.highlight_active = true;
        let epoch = self.highlight_epoch;
        let key = fence.key;
        let bytes = fence.code.len();
        let code = fence.code.to_string();
        let language = fence.language.clone();
        let task = cx.background_executor().spawn(async move {
            crate::ui::syntax_palette::highlight(HighlightRequest {
                source: &code,
                path: None,
                fence_tag: Some(&language),
            })
            .ok()
            .map(Arc::new)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.highlight_epoch != epoch {
                    return;
                }
                this.highlighting.remove(&key);
                this.highlight_active = false;
                let bytes = if result.is_some() { bytes } else { 0 };
                while this.highlight_order.len() >= 100 || this.highlight_bytes + bytes > 500_000 {
                    let Some((key, bytes)) = this.highlight_order.pop_front() else {
                        break;
                    };
                    this.highlights.remove(&key);
                    this.highlight_bytes -= bytes;
                }
                // Oversized fences stay readable without retaining their tokens.
                this.highlight_order.push_back((key, bytes.min(500_000)));
                this.highlight_bytes += bytes.min(500_000);
                this.highlights
                    .insert(key, if bytes <= 500_000 { result } else { None });
                this.pump_highlights(cx);
                cx.notify();
            });
        })
        .detach();
    }
}
