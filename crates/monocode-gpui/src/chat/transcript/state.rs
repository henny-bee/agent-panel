use super::*;

impl TranscriptView {
    pub(crate) fn side_question(&mut self) {
        self.side_question = true;
    }

    pub(crate) fn activity(rows: Vec<Row>, cx: &mut Context<Self>) -> Self {
        let mut view = Self::new();
        view.document = true;
        view.patch(0, rows, cx);
        view
    }

    /// AgentTranscript with `agentMascot`, `bottomAligned` and `inlineWork`.
    pub(crate) fn mono_mode(&mut self, mascot: &str, color: u32, name: &str) {
        self.mono_look = Some((mascot.into(), color, name.to_owned().into()));
        if self.mono {
            return;
        }
        self.mono = true;
        self.anchor_prompts = false;
        self.chat_layout = true;
        // A short chat rests on the composer, like `justify-end`.
        self.list = ListState::new(self.rows.len() + 1, ListAlignment::Bottom, px(320.));
        self.list.set_follow_mode(FollowMode::Tail);
        self.scroll_bound = false;
    }

    /// The rows of the store's window, numbered as the store numbers them.
    /// Older pages above it are read-only and never part of a save.
    pub(crate) fn source_rows(&self) -> &[Row] {
        &self.source[self.base..]
    }

    /// Every loaded row, older pages included, numbered as this view's turns
    /// and response actions are.
    pub(crate) fn history_rows(&self) -> &[Row] {
        &self.source
    }

    /// Whether `history_rows` holds every turn, not only the newest pages.
    pub(crate) fn history_complete(&self) -> bool {
        self.history_before.is_none()
    }

    pub(crate) fn document(text: String, cx: &mut Context<Self>) -> Self {
        let mut view = Self::new();
        view.document = true;
        // A document reads from its top. Chat's tail-follow would open it
        // scrolled to the end and keep it pinned there on every rebuild.
        view.list.set_follow_mode(FollowMode::Normal);
        view.patch(
            0,
            vec![Row {
                role: "assistant".into(),
                text,
                first: true,
                sent_at: None,
            }],
            cx,
        );
        view.list.scroll_to(ListOffset {
            item_ix: 0,
            offset_in_item: px(0.),
        });
        view
    }

    /// liveAgents.ts's toLiveAgent: the last turn's latest tool, as "Read
    /// src/app.rs", when that turn was sent, and how long it ran once done.
    pub(crate) fn live_turn(&self, cwd: &str) -> (Option<String>, Option<u64>, Option<u64>) {
        let Some(prompt) = self.source.iter().rposition(|r| r.first && r.is_prompt()) else {
            return (None, None, None);
        };
        let turn = (prompt + 1..self.source.len())
            .find(|&at| self.source[at].first)
            .unwrap_or(self.source.len());
        let tool = (turn..self.source.len())
            .rev()
            .find(|&at| self.source[at].first && self.source[at].role == "tool")
            .and_then(|at| {
                let end = (at + 1..self.source.len())
                    .find(|&i| self.source[i].first)
                    .unwrap_or(self.source.len());
                let text: String = self.source[at..end]
                    .iter()
                    .map(|r| r.text.as_str())
                    .collect();
                serde_json::from_str::<ToolCall>(&text).ok()
            })
            .map(|tool| {
                let shown = tool.presentation_in(Some(cwd));
                [shown.action, shown.target]
                    .into_iter()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .filter(|label| !label.is_empty());
        let duration = self
            .turns
            .get(&turn)
            .filter(|info| info.has_duration())
            .map(|info| info.duration_ms);
        (tool, self.source[prompt].sent_at, duration)
    }

    /// Follow-ups delivered during a provider run share its original receipt.
    pub(crate) fn active_turn(&self) -> Option<usize> {
        let start = self.live_start.min(self.source.len());
        let prompt = self.source[start..]
            .iter()
            .position(|row| row.first && row.is_prompt())?
            + start;
        Some(
            (prompt + 1..self.source.len())
                .find(|&at| self.source[at].first)
                .unwrap_or(self.source.len()),
        )
    }

    pub(crate) fn metrics(&mut self, metrics: monocode_core::session::agent::TurnMetrics) {
        let Some(turn) = self.active_turn() else {
            return;
        };
        self.turns.entry(turn).or_default().metrics = Some(metrics);
    }

    pub(crate) fn record_provider_turn(&mut self, id: String) {
        self.active_provider_turn_id = Some(id.clone());
        let Some(turn) = self.active_turn() else {
            return;
        };
        self.turns.entry(turn).or_default().provider_turn_id = Some(id);
    }

    pub(crate) fn response_rows(&self, turn: usize) -> (Vec<Row>, Vec<Row>) {
        let end = (turn..self.source.len())
            .find(|&i| {
                self.source[i].is_prompt() && !(self.mono && self.mono_next_run_continues(turn, i))
            })
            .unwrap_or(self.source.len());
        let mut start = self.source[..turn.min(self.source.len())]
            .iter()
            .rposition(|r| r.is_prompt())
            .unwrap_or(turn);
        while start > 0 && !self.source[start].first {
            start -= 1;
        }
        (
            self.source[start..end].to_vec(),
            self.source[..end].to_vec(),
        )
    }

    pub(crate) fn note_result(
        &mut self,
        turn: usize,
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        self.note_pending.remove(&turn);
        let success = result.is_ok();
        self.note_feedback.insert(turn, result);
        if success && self.selection_turn() == Some(turn) {
            self.selection.clear();
        }
        cx.notify();
        if success {
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;
                let _ = this.update(cx, |this, cx| {
                    this.note_feedback.remove(&turn);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    pub(crate) fn new() -> Self {
        Self {
            selection: Default::default(),
            selection_focus: None,
            links: Default::default(),
            work_item_cards: Default::default(),
            source: Vec::new(),
            source_bytes: 0,
            attachment_images: HashMap::new(),
            visuals: HashMap::new(),
            placed: HashSet::new(),
            published: HashMap::new(),
            history_position: None,
            history_start: 0,
            history_loading: false,
            history_requested: false,
            base: 0,
            history_before: None,
            window_before: None,
            history_pager: None,
            history_failed: false,
            history_search: None,
            prepared_parsers: HashMap::new(),
            mono: false,
            mono_groups: Default::default(),
            mono_look: None,
            hovered_turn: None,
            hovered_prompt: None,
            hovered_prompt_actions: None,
            hover_rows: HashMap::new(),
            entrances: HashMap::new(),
            reactions: HashMap::new(),
            follow: Follow::default(),
            pending: Vec::new(),
            submitted_mono_message: None,
            saved_draft: None,
            taken: Vec::new(),
            pending_entrances: HashMap::new(),
            launches: HashMap::new(),
            started_at: Instant::now(),
            frame_now: Instant::now(),
            measurements: Default::default(),
            markdown_cache: Default::default(),
            messages: Vec::new(),
            rows: Vec::new(),
            streaming: false,
            perf_scroll: Default::default(),
            chat_layout: true,
            anchor_prompts: true,
            smooth_reveal: true,
            collapse_groups: false,
            fold_work: true,
            search_match: None,
            highlights: HashMap::new(),
            highlighting: HashSet::new(),
            highlight_queue: VecDeque::new(),
            highlight_active: false,
            highlight_epoch: 0,
            highlight_order: VecDeque::new(),
            highlight_bytes: 0,
            copied: None,
            copied_response: None,
            note_pending: HashSet::new(),
            note_feedback: HashMap::new(),
            action_anchors: HashMap::new(),
            hovered_action: None,
            phase_tiles: HashMap::new(),
            work_tiles: HashMap::new(),
            target_action: None,
            document: false,
            side_question: false,
            file_link_base: None,
            expanded: HashSet::new(),
            agent_panels: Default::default(),
            expanded_prompts: HashSet::new(),
            clamped_prompts: HashSet::new(),
            phases: HashMap::new(),
            phase_toggle: None,
            phase_layouts: HashMap::new(),
            phase_scrolls: HashMap::new(),
            windowed_phases: HashSet::new(),
            phase_max_height: px(280.),
            work_folds: HashMap::new(),
            reveals: HashMap::new(),
            live_start: 0,
            motions: HashMap::new(),
            reflow: Default::default(),
            frame_scheduled: false,
            last_motion_frame: None,
            visible: true,
            viewport: Bounds::default(),
            anchor_turn: None,
            prompt_rise: None,
            prompt_rise_pending: false,
            burst: None,
            introduced_prompt: None,
            show_jump: false,
            commit_files: HashMap::new(),
            copied_commit: None,
            jump_beside_pill: false,
            jump_anchor: Default::default(),
            scroll_bound: false,
            fold_targets: HashMap::new(),
            step_queue: HashMap::new(),
            seen_steps: HashSet::new(),
            turns: HashMap::new(),
            provider: "codex".into(),
            model: "Codex".into(),
            active_provider_turn_id: None,
            started: None,
            elapsed: HashMap::new(),
            outline: outline::Outline::default(),
            list: {
                // The trailing item holds the viewport's bottom padding and,
                // after a send, the unused part of the current turn's min-height.
                let list = ListState::new(1, ListAlignment::Top, px(320.));
                list.set_follow_mode(FollowMode::Tail);
                list
            },
            rendered: 0,
            render_passes: 0,
            #[cfg(test)]
            rebuild_passes: 0,
        }
    }

    pub(crate) fn apply_preferences(
        &mut self,
        p: &crate::ui::preferences::Preferences,
        cx: &mut Context<Self>,
    ) {
        let chat = p.transcript_layout == "chat" || self.mono;
        let anchor_prompts = p.anchor_prompts && !self.mono;
        if self.chat_layout == chat
            && self.anchor_prompts == anchor_prompts
            && self.smooth_reveal == p.paced_reveal
            && self.collapse_groups == p.collapse_tool_groups
            && self.fold_work == p.fold_finished_work
        {
            return;
        }
        self.collapse_groups = p.collapse_tool_groups;
        self.fold_work = p.fold_finished_work;
        self.chat_layout = chat;
        self.anchor_prompts = anchor_prompts;
        self.smooth_reveal = p.paced_reveal;
        self.measurements.clear();
        if !self.anchor_prompts {
            self.anchor_turn = None;
        }
        if !self.smooth_reveal {
            for reveal in self.reveals.values_mut() {
                reveal.settle(Instant::now());
            }
        }
        self.rebuild(0, cx);
        cx.notify();
    }

    pub(crate) fn visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.selection.clear();
        let now = Instant::now();
        // Settled tabs retain their parsed rows, measured heights and scroll
        // anchor. Only undisplayed text or outgoing animation rows need a
        // source rebuild when visibility changes.
        let rebuild_from = self
            .reveals
            .iter()
            .filter(|(_, reveal)| reveal.behind())
            .map(|(&at, _)| at)
            .chain(
                self.motions
                    .iter()
                    .filter(|(_, motion)| !motion.opening)
                    .map(|(key, _)| key.0),
            )
            .min();
        let panels: HashSet<_> = self.agent_panels.animate().into_iter().collect();
        if !self.motions.is_empty() || !panels.is_empty() {
            self.phase_layouts.clear();
            for (ix, row) in self.rows.iter().enumerate() {
                if self.motions.contains_key(&row.key) || panels.contains(&row.key.0) {
                    self.list.remeasure_items(ix..ix + 1);
                }
            }
        }
        self.prompt_rise = None;
        self.prompt_rise_pending = false;
        self.burst = None;
        self.entrances.clear();
        self.follow = Follow::default();
        for reveal in self.reveals.values_mut() {
            reveal.settle(now);
        }
        self.visible = false;
        for scroll in self.phase_scrolls.values_mut() {
            scroll.enable(false);
        }
        self.motions.clear();
        self.agent_panels.settle();
        if let Some(start) = rebuild_from.filter(|_| !self.source.is_empty()) {
            self.rebuild(start.min(self.source.len() - 1), cx);
        }
        self.visible = visible;
        if visible {
            cx.notify();
        }
    }

    pub(crate) fn identity(&mut self, provider: &str, model: &str) {
        self.provider = provider.into();
        self.model = model.into();
        if self.streaming
            && let Some(turn) = self.active_turn()
        {
            let info = self.turns.entry(turn).or_default();
            info.provider = provider.into();
            info.model = model.into();
        }
    }
}
