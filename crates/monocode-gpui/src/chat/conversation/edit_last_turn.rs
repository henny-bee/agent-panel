//! Cuts at a past user message. Edit recalls it and cuts only when the user
//! resubmits, Rewind cuts after a confirm, and Fork cuts a copy into a new chat.
//! Rewind and Edit can also restore files where the provider keeps checkpoints;
//! Fork never does.
use super::*;
use crate::chat::response_actions::Cut;
use monocode_core::{provider, session::agent::TurnInfo, store::EditSnapshot};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// What a cut said, above the composer until dismissed or the next send.
#[derive(Clone)]
pub(super) struct Notice {
    pub(super) text: SharedString,
    pub(super) error: bool,
    pub(super) undo: Option<Undo>,
}
/// What Undo rewind puts back: the rows the cut took, the binding before it
/// and their turn receipts. Files stay as they are.
#[derive(Clone)]
pub(super) struct Undo {
    edit: EditSnapshot,
    binding: Option<String>,
    turns: HashMap<usize, TurnInfo>,
}
/// Which part of the conversation a rewind takes back.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Restore {
    Conversation,
    Both,
    Code,
}

#[derive(Default)]
pub(super) struct State {
    pub(super) active: bool,
    prepared: Option<EditSnapshot>,
    /// A provider change the local history may not match holds edits and
    /// sends until Cancel edit lifts it, rather than silently appending to
    /// inconsistent context.
    blocked: bool,
    /// The user confirmed the rewind that sending the prepared edit performs,
    /// and whether it restores files too.
    confirmed: Option<bool>,
    pub(super) notice: Option<Notice>,
    /// Set by Cancel edit while a cut runs, so the cut changes nothing here.
    cancel: Option<std::sync::Arc<AtomicBool>>,
}
impl State {
    pub(super) fn pending(&self) -> bool {
        self.active || self.prepared.is_some() || self.blocked
    }
    pub(super) fn message_sent(&mut self) {
        self.notice = None;
    }
}

impl Conversation {
    /// Drops a recalled edit. With `lift_guard`, which only the Cancel edit
    /// button passes, it also lifts a recovery guard: that is the way back into
    /// a guarded chat, taking the provider's context as it is.
    pub(super) fn cancel_edit_last_turn(
        &mut self,
        lift_guard: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.editing_last_turn.active {
            let Some(cancel) = self.editing_last_turn.cancel.as_ref() else {
                return false;
            };
            cancel.store(true, Ordering::SeqCst);
            self.status = "Cancelling…".into();
            cx.notify();
            return true;
        }
        if self.editing_last_turn.blocked && !lift_guard {
            return false;
        }
        if self.editing_last_turn.blocked {
            if let Err(error) =
                locked(&self.store).and_then(|store| store.clear_edit_recovery(self.summary.id))
            {
                self.status = format!("Could not clear the edit guard: {error}");
                cx.notify();
                return true;
            }
            self.editing_last_turn.blocked = false;
        } else if self.editing_last_turn.prepared.is_none() {
            return false;
        }
        self.editing_last_turn.prepared = None;
        self.status = "Ready".into();
        self.run_history_actions(cx);
        cx.notify();
        true
    }
    pub(super) fn defer_edit_transfer(
        &mut self,
        settings: Settings,
        prompt: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.history_actions.push_back(Box::new(move |this, cx| {
            if let Some(prompt) = prompt {
                if this.preserve_new_composer(cx) {
                    this.install_transfer(settings, None, cx);
                    this.enqueue_restored_prompt(prompt, cx);
                } else {
                    this.install_transfer(settings, Some(prompt), cx);
                }
            } else {
                this.install_transfer(settings, None, cx);
            }
        }));
        self.status = "Finish or cancel the current message edit to continue".into();
        cx.notify();
    }
    /// A durable recovery guard holds edits and sends until Cancel edit lifts it.
    fn recovery_blocked(&mut self, cx: &mut Context<Self>) -> bool {
        match locked(&self.store).and_then(|store| store.edit_recovery_reason(self.summary.id)) {
            Ok(None) => return false,
            Ok(Some(reason)) => {
                self.editing_last_turn.blocked = true;
                self.status = reason;
            }
            Err(error) => {
                self.status = format!(
                    "Could not verify conversation history: {error}. Your draft is preserved."
                );
            }
        }
        cx.notify();
        true
    }
    /// A fork leaves the composer alone, so only Edit and Rewind need it empty.
    fn can_prepare_edit(&self, cut: Cut, cx: &gpui::App) -> bool {
        self.history_ready()
            && !self.busy
            && !self.stopping
            && !self.editing_last_turn.pending()
            && self.settings.mono.is_none()
            && !self.settings.worktree_removed
            && provider::supports_rewind(Kind::from_id(&self.summary.provider))
            && (cut != Cut::Fork || provider::supports_fork(Kind::from_id(&self.summary.provider)))
            && !self.account_switching
            && !self.preparing_worktree
            && !self.attaching
            && !self.planning_orchestration
            && self.orchestration_progress.is_none()
            && !self.needs_approval()
            && self.questions.is_empty()
            && self.controls.is_none()
            && self.follow_ups.state.messages.is_empty()
            && self.follow_ups.state.draft.is_none()
            && self.follow_ups.edit.is_none()
            && self.follow_ups.sending.is_none()
            && self.follow_ups.requested.is_none()
            && self.follow_ups.state.revision == self.follow_ups.persisted
            && (cut == Cut::Fork
                || (self.input.read(cx).content.is_empty()
                    && !self.input.read(cx).is_composing()
                    && self.attachments.is_empty()))
    }
    /// Shows what a cut did or why it didn't, above the composer, where it
    /// stays visible while the agent works.
    pub(super) fn cut_notice(
        &mut self,
        text: impl Into<SharedString>,
        error: bool,
        cx: &mut Context<Self>,
    ) {
        let text = text.into();
        self.status = text.to_string();
        self.editing_last_turn.notice = Some(Notice {
            text,
            error,
            undo: None,
        });
        cx.notify();
    }
    pub(super) fn dismiss_cut_notice(&mut self, cx: &mut Context<Self>) {
        self.editing_last_turn.notice = None;
        cx.notify();
    }
    /// Whether a cut can't start now. Says why, as the click does nothing else.
    fn refuse_cut(&mut self, cut: Cut, cx: &mut Context<Self>) -> bool {
        if self.can_prepare_edit(cut, cx) {
            return false;
        }
        let action = match cut {
            Cut::Fork => "Fork",
            Cut::Rewind => "Rewind",
            Cut::Edit => "Edit",
        };
        let reason = if cut != Cut::Fork
            && (!self.input.read(cx).content.is_empty() || !self.attachments.is_empty())
        {
            format!("{action} needs an empty message box. Send or clear your draft first.")
        } else if !self.follow_ups.state.messages.is_empty()
            || self.follow_ups.state.draft.is_some()
            || self.follow_ups.state.revision != self.follow_ups.persisted
        {
            format!("{action} is unavailable while messages are queued. Send or clear them first.")
        } else if self.needs_approval() || !self.questions.is_empty() {
            format!("{action} is unavailable while a request waits for your answer.")
        } else if self.editing_last_turn.pending() {
            "Finish or cancel the message you're editing first.".to_owned()
        } else if self.busy || self.stopping {
            format!(
                "{action} is unavailable while {} is working.",
                Kind::from_id(&self.summary.provider).label()
            )
        } else if !provider::supports_rewind(Kind::from_id(&self.summary.provider))
            || (cut == Cut::Fork && !provider::supports_fork(Kind::from_id(&self.summary.provider)))
        {
            format!("This session doesn't support {}.", action.to_lowercase())
        } else {
            format!("{action} is unavailable until this chat finishes what it's doing.")
        };
        self.cut_notice(reason, true, cx);
        true
    }
    /// Asks before a cut. When the cut can restore files (`files`), the user
    /// picks "Code and conversation" or "Conversation only" instead of
    /// `action`, and "Code only" too with `code_only`. `then` gets the choice,
    /// unless cancelled.
    fn confirm_cut(
        &mut self,
        title: &'static str,
        detail: String,
        action: &'static str,
        files: bool,
        code_only: bool,
        then: impl FnOnce(&mut Self, Restore, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let (detail, answers, choices) = if files {
            let mut answers = vec!["Code and conversation", "Conversation only"];
            let mut choices = vec![Restore::Both, Restore::Conversation];
            if code_only {
                answers.push("Code only");
                choices.push(Restore::Code);
            }
            answers.push("Cancel");
            (
                format!(
                    "{detail}\n\nChanges made through shell commands aren't reverted. Commits stay in place."
                ),
                answers,
                choices,
            )
        } else {
            (detail, vec![action, "Cancel"], vec![Restore::Conversation])
        };
        cx.spawn(async move |this, cx| {
            let Ok(answer) = this.update_in(cx, |_, window, cx| {
                window.prompt(
                    gpui::PromptLevel::Warning,
                    title,
                    Some(detail.as_str()),
                    &answers,
                    cx,
                )
            }) else {
                return;
            };
            if let Ok(choice) = answer.await
                && let Some(&restore) = choices.get(choice)
            {
                let _ = this.update(cx, |this, cx| then(this, restore, cx));
            }
        })
        .detach();
    }
    /// Starts a cut at the user message whose first row is `turn`.
    pub(super) fn begin_cut(
        &mut self,
        turn: usize,
        cut: Cut,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_remote() {
            self.cut_notice(
                "Rewind isn't available for sessions on a remote host.",
                true,
                cx,
            );
            return;
        }
        // The action is tied to a painted source row. A history restoration may
        // replace that row, so never replay this intent against an empty cache.
        if self.recovery_blocked(cx) || self.refuse_cut(cut, cx) {
            return;
        }
        let rows = self.transcript.read(cx).source_rows();
        if !rows
            .get(turn)
            .is_some_and(|row| row.first && row.role == "user")
        {
            return;
        }
        if cut == Cut::Rewind {
            let end = (turn + 1..rows.len())
                .find(|&row| rows[row].first)
                .unwrap_or(rows.len());
            let text: String = rows[turn..end]
                .iter()
                .map(|row| row.text.as_str())
                .collect();
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            let preview = match text.char_indices().nth(140) {
                Some((at, _)) => format!("“{}…”", &text[..at]),
                None => format!("“{text}”"),
            };
            let kind = Kind::from_id(&self.summary.provider);
            let files = provider::restores_files(kind, rows, turn);
            let preview = match changed_files(rows, turn) {
                0 => preview,
                1 => format!("{preview}\n\n1 file changed since this message."),
                n => format!("{preview}\n\n{n} files changed since this message."),
            };
            self.confirm_cut(
                "Rewind to this message?",
                preview,
                "Rewind",
                files,
                kind == Kind::Claude,
                move |this, restore, cx| this.prepare_cut(turn, cut, restore, cx),
                cx,
            );
            return;
        }
        if cut == Cut::Edit {
            window.focus(&self.input.focus_handle(cx), cx);
        }
        self.prepare_cut(turn, cut, Restore::Conversation, cx);
    }
    /// `restore` other than Conversation comes only from Rewind.
    fn prepare_cut(&mut self, turn: usize, cut: Cut, restore: Restore, cx: &mut Context<Self>) {
        if self.refuse_cut(cut, cx) {
            return;
        }
        let rows = self.transcript.read(cx).source_rows().to_vec();
        if !rows
            .get(turn)
            .is_some_and(|row| row.first && row.role == "user")
        {
            return;
        }
        self.editing_last_turn.active = true;
        self.status = "Preparing…".into();
        self.response_menu = None;
        let store = self.store.clone();
        let id = self.summary.id;
        let binding = self.summary.provider_id.clone();
        let work = cx
            .background_executor()
            .spawn(async move { locked(&store)?.prepare_edit(id, &rows, turn) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.editing_last_turn.active = false;
                match result {
                    Ok(edit)
                        if this.summary.provider_id == binding
                            && edit.matches(this.transcript.read(cx).source_rows())
                            && !this.busy
                            && this.can_prepare_edit(cut, cx) =>
                    {
                        if cut == Cut::Fork {
                            this.run_fork(edit, cx);
                        } else if restore == Restore::Code {
                            this.run_restore(edit, cx);
                        } else {
                            this.input
                                .update(cx, |input, cx| input.set_text(edit.prompt.clone(), cx));
                            this.attachments = edit.attachments.clone();
                            this.editing_last_turn.prepared = Some(edit.clone());
                            if cut == Cut::Rewind {
                                this.run_cut(edit, false, restore == Restore::Both, cx);
                            } else {
                                this.status =
                                    "Editing message — sending it restarts from here".into();
                            }
                        }
                    }
                    Ok(_) => this.cut_notice(
                        "The message box or the conversation changed, so nothing was rewound.",
                        true,
                        cx,
                    ),
                    Err(error) => this.cut_notice(
                        format!("Couldn't go back to this message: {error}"),
                        true,
                        cx,
                    ),
                }
                if !this.editing_last_turn.pending() {
                    this.run_history_actions(cx);
                }
                cx.emit(SessionChanged);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Called before ordinary send consumes composer text or attachments.
    /// Returns true while this edit owns the submission (including rejection).
    pub(super) fn submit_edited(&mut self, cx: &mut Context<Self>) -> bool {
        let confirmed = std::mem::take(&mut self.editing_last_turn.confirmed);
        if self.editing_last_turn.active
            || self.editing_last_turn.blocked
            || self.recovery_blocked(cx)
        {
            return true;
        }
        let Some(edit) = self.editing_last_turn.prepared.clone() else {
            return false;
        };
        if self.input.read(cx).content.is_empty() && self.attachments.is_empty() {
            // Clearing the recalled prompt cancels the edit without touching
            // either provider or history. The next new prompt sends normally.
            self.editing_last_turn.prepared = None;
            self.status = "Ready".into();
            self.run_history_actions(cx);
            cx.notify();
            return true;
        }
        let text = self.input.read(cx).content.trim();
        if text.eq_ignore_ascii_case("/compact")
            || text.eq_ignore_ascii_case("/mcp")
            || ["btw", "draft", "add-to-folder"].iter().any(|command| {
                monocode_core::session::composer::consume_command(text, command).is_some()
            })
            || self.draft_active(cx)
        {
            self.status = "Cancel message editing before running a local command".into();
            cx.notify();
            return true;
        }
        if !self.input.read(cx).has_message_text() && self.attachments.is_empty() {
            return true;
        }
        if self.busy
            || self.stopping
            || self.attaching
            || self.account_switching
            || self.preparing_worktree
            || self.settings.worktree_removed
            || self.input.read(cx).is_composing()
            || !self.follow_ups.state.messages.is_empty()
            || self.follow_ups.state.draft.is_some()
            || self.follow_ups.edit.is_some()
            || self.follow_ups.sending.is_some()
            || self.follow_ups.state.revision != self.follow_ups.persisted
            || !edit.matches(self.transcript.read(cx).source_rows())
            || self.summary.provider_id.as_deref() != Some(edit.provider_id.as_str())
            || self.summary.provider != edit.provider()
            || self.settings.account.id != edit.account_id
            || self.execution_cwd() != edit.cwd
        {
            self.cut_notice(
                "The conversation changed while you were editing. Cancel the edit, then try again.",
                true,
                cx,
            );
            return true;
        }
        let Some(files) = confirmed else {
            let files = provider::restores_files(
                Kind::from_id(&self.summary.provider),
                &edit.rows,
                edit.start,
            );
            self.confirm_cut(
                "Rewind and send this edit?",
                "Your edit replaces the original message. Everything after it is removed.".into(),
                "Rewind and send",
                files,
                false,
                |this, restore, cx| {
                    this.editing_last_turn.confirmed = Some(restore == Restore::Both);
                    this.send(cx);
                },
                cx,
            );
            return true;
        };
        self.run_cut(edit, true, files, cx);
        true
    }
    /// The request a cut sends to the provider: the message's text, which
    /// Claude and Pi check against their own copy, and no prompt to run.
    fn cut_request(
        &mut self,
        edit: &EditSnapshot,
        cx: &mut Context<Self>,
    ) -> Option<provider::Request> {
        let kind = Kind::from_id(&self.summary.provider);
        let Some(binary) = provider::resolve(kind) else {
            self.status = format!(
                "{} executable was not found; history was preserved",
                kind.label()
            );
            cx.notify();
            return None;
        };
        self.attach_account(cx);
        let mut settings = self.settings.clone();
        settings.environment.extend(self.app_environment.clone());
        Some(provider::Request {
            kind,
            binary,
            cwd: PathBuf::from(self.execution_cwd()),
            prompt: edit.prompt.clone(),
            provider_id: Some(edit.provider_id.clone()),
            model: self.model.clone(),
            settings,
        })
    }
    /// Cuts this conversation before the prepared message, in the provider and
    /// then locally. `send` sends the composer afterwards (Edit); otherwise the
    /// recalled message waits in the composer (Rewind). `files` restores files
    /// first, and a failed restore cuts nothing.
    fn run_cut(&mut self, edit: EditSnapshot, send: bool, files: bool, cx: &mut Context<Self>) {
        let Some(request) = self.cut_request(&edit, cx) else {
            return;
        };
        let prompt = self.input.read(cx).content.to_string();
        let attachments = self.attachments.clone();
        let checked_attachments = attachments.clone();
        self.editing_last_turn.active = true;
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        self.editing_last_turn.cancel = Some(cancel.clone());
        // Keep the visible draft in place until confirmation, including on a
        // rejected RPC. IME composition was checked before making it read-only.
        self.input.update(cx, |input, _| input.set_read_only(true));
        self.status = "Rewinding…".into();
        let store = self.store.clone();
        let id = self.summary.id;
        let cancelled = move || cancel.load(Ordering::SeqCst);
        let work = cx.background_executor().spawn(async move {
            if send {
                monocode_core::session::composer::validate_attachments(&checked_attachments)?;
            }
            locked(&store)?.validate_edit(id, &edit)?;
            if cancelled() {
                return Err(CANCELLED.to_owned());
            }
            // Nothing the user said comes before the cut, so the next send
            // starts a fresh provider session. Restoring files still needs it.
            let binding = if edit.first() && !files {
                None
            } else {
                match provider::rewind_to(
                    request,
                    edit.provider_turn_id.as_deref(),
                    edit.later(),
                    edit.sent_at(),
                    files,
                ) {
                    Ok(binding) => binding,
                    Err(error) if error.provider_may_have_changed => {
                        locked(&store)?.mark_edit_recovery_required(id)?;
                        return Err(format!("Provider recovery required: {error}"));
                    }
                    Err(error) => return Err(error.to_string()),
                }
            };
            // Provider confirmation is the only path to the destructive write.
            let in_place = binding.as_deref() == Some(edit.provider_id.as_str());
            // A cut copy changed nothing yet, so a late cancel still holds;
            // restored files or an in-place revert are done and get recorded.
            if cancelled() && !in_place && !files {
                return Err(CANCELLED.to_owned());
            }
            match locked(&store)?.finish_edit(id, &edit, binding.as_deref()) {
                Ok(()) => Ok((edit, binding)),
                Err(error) if in_place => Err(format!(
                    "Provider recovery required: Provider rewound, but local history changed: {error}"
                )),
                Err(error) if files => Err(format!(
                    "Restored the code, but couldn't rewind the conversation. Try rewinding again. ({error})"
                )),
                Err(error) => Err(error),
            }
        });
        cx.spawn(async move |this, cx| {
            let result: Result<(EditSnapshot, Option<String>), String> = work.await;
            let _ = this.update(cx, |this, cx| {
                this.editing_last_turn.active = false;
                this.editing_last_turn.cancel = None;
                this.input.update(cx, |input, _| input.set_read_only(false));
                match result {
                    Err(error) if error == CANCELLED => {
                        this.editing_last_turn.prepared = None;
                        this.cut_notice("Cancelled. Nothing was rewound.", false, cx);
                    }
                    Ok((edit, binding)) => {
                        this.editing_last_turn.prepared = None;
                        let undo = (!send
                            && binding.as_deref() != Some(edit.provider_id.as_str()))
                        .then(|| Undo {
                            edit: edit.clone(),
                            binding: binding.clone(),
                            turns: this.settings.turns.clone(),
                        });
                        this.summary.provider_id = binding;
                        this.settings.turns.retain(|&row, _| row < edit.start);
                        this.transcript.update(cx, |view, cx| {
                            view.replace_history(edit.rows[..edit.start].to_vec(), cx)
                        });
                        this.summary.has_messages = edit.start > 0;
                        this.context = None;
                        if !send {
                            this.cut_notice("Rewound. Your message is back in the box.", false, cx);
                            if let Some(notice) = this.editing_last_turn.notice.as_mut() {
                                notice.undo = undo;
                            }
                        } else if this.input.read(cx).content.as_str() == prompt
                            && this.attachments == attachments
                        {
                            // Normal submit still owns mode commands, attachments,
                            // account checks, durable acceptance and failure recovery.
                            this.send(cx);
                        } else {
                            // Programmatic composer updates can bypass read-only.
                            // Preserve both the new composer and the accepted edit.
                            this.preserve_interrupted_edit(prompt, attachments, edit.start, cx);
                        }
                        this.run_history_actions(cx);
                    }
                    Err(error) => {
                        if error.starts_with("Provider recovery required:") {
                            this.editing_last_turn.blocked = true;
                            this.editing_last_turn.prepared = None;
                            this.cut_notice(format!("{error}. Choose Cancel edit to continue here anyway; your draft is preserved."), true, cx);
                        } else {
                            // Restore messages already say what happened.
                            let lead = if error.starts_with("Couldn't") || error.starts_with("Restored") {
                                ""
                            } else {
                                "Couldn't rewind: "
                            };
                            this.cut_notice(format!("{lead}{}. The message is in the box; send it to retry, or cancel the edit.", error.trim_end_matches('.')), true, cx);
                        }
                    }
                }
                cx.emit(SessionChanged);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    /// Copies the conversation before the prepared message into a new chat
    /// with the provider cut at the same point. This chat stays as it is.
    fn run_fork(&mut self, edit: EditSnapshot, cx: &mut Context<Self>) {
        let Some(request) = self.cut_request(&edit, cx) else {
            return;
        };
        self.editing_last_turn.active = true;
        self.status = "Forking…".into();
        let store = self.store.clone();
        let id = self.summary.id;
        let work = cx.background_executor().spawn(async move {
            locked(&store)?.validate_edit(id, &edit)?;
            let binding = if edit.first() {
                None
            } else {
                provider::rewind_to(
                    request,
                    edit.provider_turn_id.as_deref(),
                    edit.later(),
                    edit.sent_at(),
                    false,
                )
                .map_err(|error| error.to_string())?
            };
            let fork = locked(&store)?.fork_edit(id, &edit, binding.as_deref())?;
            Ok::<_, String>((fork, edit))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.editing_last_turn.active = false;
                match result {
                    Ok((fork, edit)) => {
                        this.cut_notice(format!("Forked into “{}”.", fork.title), false, cx);
                        cx.emit(Forked(fork, edit.prompt, edit.attachments));
                    }
                    Err(error) => this.cut_notice(format!("Couldn't fork: {error}"), true, cx),
                }
                this.run_history_actions(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    /// "Code only": puts files back as they were before the prepared message
    /// and leaves the conversation as it is.
    fn run_restore(&mut self, edit: EditSnapshot, cx: &mut Context<Self>) {
        let Some(request) = self.cut_request(&edit, cx) else {
            return;
        };
        self.editing_last_turn.active = true;
        self.status = "Restoring the code…".into();
        let later = edit.later();
        let work = cx
            .background_executor()
            .spawn(async move { provider::restore_files_only(request, later) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.editing_last_turn.active = false;
                match result {
                    Ok(()) => this.cut_notice(
                        "Restored the code to this message. The conversation stays as it is.",
                        false,
                        cx,
                    ),
                    Err(error) => this.cut_notice(error.to_string(), true, cx),
                }
                this.run_history_actions(cx);
            });
        })
        .detach();
        cx.notify();
    }
    /// The last cut's notice above the composer, with Undo rewind when the
    /// rewind can be taken back.
    pub(super) fn cut_notice_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let notice = self.editing_last_turn.notice.clone()?;
        let color: gpui::Hsla = if notice.error {
            gpui::rgb(0xf87171).into()
        } else {
            content(0.8)
        };
        Some(
            div()
                .id("cut-notice")
                .debug_selector(|| "cut-notice".into())
                .mb(px(6.))
                .px(px(12.))
                .py(px(8.))
                .rounded(px(10.))
                .bg(content(0.05))
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(12.))
                .line_height(px(16.))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .text_color(color)
                        .child(notice.text),
                )
                .when(notice.undo.is_some(), |d| {
                    d.child(
                        tool_pill("undo-rewind", "Undo rewind")
                            .child("Undo rewind")
                            .on_click(cx.listener(|this, _, _, cx| this.undo_rewind(cx))),
                    )
                })
                .child(
                    tool_pill("dismiss-cut-notice", "Dismiss")
                        .child(icon("close", 12.))
                        .on_click(cx.listener(|this, _, _, cx| this.dismiss_cut_notice(cx))),
                )
                .into_any_element(),
        )
    }
    /// Brings back what the last rewind took out of the conversation.
    pub(super) fn undo_rewind(&mut self, cx: &mut Context<Self>) {
        let Some(undo) = self
            .editing_last_turn
            .notice
            .as_mut()
            .and_then(|notice| notice.undo.take())
        else {
            return;
        };
        if self.busy || self.editing_last_turn.pending() {
            self.cut_notice(
                "Couldn't undo this rewind while the chat is busy.",
                true,
                cx,
            );
            return;
        }
        let store = self.store.clone();
        let id = self.summary.id;
        let work = cx.background_executor().spawn(async move {
            locked(&store)?.undo_edit(id, &undo.edit, undo.binding.as_deref(), &undo.turns)?;
            Ok::<_, String>(undo)
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(undo) => {
                        // The recalled message goes back where it was.
                        if this.input.read(cx).content == undo.edit.prompt {
                            this.input
                                .update(cx, |input, cx| input.set_text(String::new(), cx));
                            this.attachments.clear();
                        }
                        this.summary.provider_id = Some(undo.edit.provider_id.clone());
                        this.settings.turns = undo.turns;
                        this.transcript.update(cx, |view, cx| {
                            view.replace_history(undo.edit.rows.clone(), cx)
                        });
                        this.summary.has_messages = true;
                        this.cut_notice("Rewind undone. Your files stay as they are.", false, cx);
                    }
                    Err(error) => this.cut_notice(error, true, cx),
                }
                cx.emit(SessionChanged);
            });
        })
        .detach();
    }
    /// Puts a message in the composer without sending it, once history is in.
    pub(crate) fn prefill(
        &mut self,
        prompt: String,
        attachments: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if self.needs_history_restore() {
            self.defer_history_action(move |this, cx| this.prefill(prompt, attachments, cx), cx);
            return;
        }
        self.input
            .update(cx, |input, cx| input.set_text(prompt, cx));
        self.attachments = attachments;
        cx.notify();
    }
    fn preserve_interrupted_edit(
        &mut self,
        text: String,
        attachments: Vec<PathBuf>,
        at: usize,
        cx: &mut Context<Self>,
    ) {
        let message = monocode_core::session::follow_up::Message {
            id: format!(
                "edited-{}-{}",
                self.summary.id,
                monocode_core::sys::time::now_ms()
            ),
            text,
            attachments,
            at,
            ..Default::default()
        };
        if self.follow_ups.state.draft.is_none() {
            self.follow_ups.state.draft = Some(message);
        } else {
            self.follow_ups.state.paused = true;
            self.follow_ups.state.messages.push(message);
        }
        self.save_follow_ups(cx);
        self.status =
            "Rewound; the edited prompt was saved as a draft because the composer changed".into();
    }
}

const CANCELLED: &str = "cancelled";

/// Files the agent edited from row `at` on, counted once each.
fn changed_files(rows: &[monocode_core::session::transcript::Row], at: usize) -> usize {
    use monocode_core::session::tool::ToolCall;
    let mut paths = std::collections::HashSet::new();
    for row in rows
        .get(at..)
        .into_iter()
        .flatten()
        .filter(|row| row.role == "tool")
    {
        let Ok(tool) = serde_json::from_str::<ToolCall>(&row.text) else {
            continue;
        };
        if !matches!(tool.summary().0.as_str(), "Edit" | "Write") {
            continue;
        }
        let input: serde_json::Value = serde_json::from_str(&tool.input).unwrap_or_default();
        if let Some(path) = input["file_path"].as_str().or(input["path"].as_str()) {
            paths.insert(path.to_owned());
        }
    }
    paths.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use monocode_core::{session::transcript::Transcript, store::Store};
    use std::sync::Mutex;
    #[gpui::test]
    fn cancelled_recall_preserves_history_and_native_provider_binding(cx: &mut TestAppContext) {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        let mut session = store.create("pi", "/project").unwrap();
        store.provider_id(session.id, "thread").unwrap();
        session.provider_id = Some("thread".into());
        let mut transcript = Transcript::default();
        transcript.begin("user");
        transcript.append("Original");
        transcript.begin("assistant");
        transcript.append("Answer");
        store.write_rows(session.id, 0, &transcript.rows).unwrap();
        let edit = store.prepare_edit(session.id, &transcript.rows, 0).unwrap();
        let chat = cx.new(|cx| Conversation::new(session, Arc::new(Mutex::new(store)), cx));
        chat.update(cx, |chat, cx| {
            chat.loaded = true;
            chat.transcript
                .update(cx, |view, cx| view.patch(0, transcript.rows.clone(), cx));
            chat.editing_last_turn.prepared = Some(edit);
            assert!(chat.submit_edited(cx));
            assert!(!chat.editing_last_turn.pending());
            assert_eq!(chat.transcript.read(cx).source_rows(), transcript.rows);
            assert_eq!(chat.summary.provider_id.as_deref(), Some("thread"));
            assert_eq!(
                locked(&chat.store)
                    .unwrap()
                    .load(chat.summary.id)
                    .unwrap()
                    .rows,
                transcript.rows
            );
        });
    }
    #[gpui::test]
    fn a_refused_cut_says_why_above_the_composer_and_cancel_stops_a_running_one(
        cx: &mut TestAppContext,
    ) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let session = store.create("claude", "/project").unwrap();
        let chat = cx.new(|cx| Conversation::new(session, Arc::new(Mutex::new(store)), cx));
        chat.update(cx, |chat, cx| {
            chat.loaded = true;
            chat.busy = true;
            assert!(chat.refuse_cut(Cut::Rewind, cx));
            let notice = chat.editing_last_turn.notice.clone().unwrap();
            assert!(notice.error);
            assert_eq!(
                notice.text.as_ref(),
                "Rewind is unavailable while Claude Code is working."
            );
            chat.busy = false;
            chat.input
                .update(cx, |input, cx| input.set_text("Draft".into(), cx));
            assert!(chat.refuse_cut(Cut::Edit, cx));
            assert!(
                chat.editing_last_turn
                    .notice
                    .as_ref()
                    .unwrap()
                    .text
                    .starts_with("Edit needs an empty message box")
            );
            // A fork leaves the composer alone.
            assert!(!chat.refuse_cut(Cut::Fork, cx));
            // Sending ends the notice.
            chat.editing_last_turn.message_sent();
            assert!(chat.editing_last_turn.notice.is_none());
            // Cancel edit reaches a cut that is still running.
            let flag = Arc::new(AtomicBool::new(false));
            chat.editing_last_turn.active = true;
            chat.editing_last_turn.cancel = Some(flag.clone());
            assert!(chat.cancel_edit_last_turn(false, cx));
            assert!(flag.load(Ordering::SeqCst));
        });
    }
    #[test]
    fn changed_files_counts_each_edited_path_once_from_the_message_on() {
        use monocode_core::session::{tool::ToolCall, transcript::Row};
        let tool = |name: &str, path: &str| {
            let mut call = ToolCall::new(path, name);
            call.input = serde_json::json!({"file_path":path}).to_string();
            Row {
                role: "tool".into(),
                text: serde_json::to_string(&call).unwrap(),
                first: true,
                sent_at: None,
            }
        };
        let rows = [
            tool("Edit", "a.rs"),
            tool("Write", "b.rs"),
            tool("Edit", "b.rs"),
            tool("Read", "c.rs"),
        ];
        assert_eq!(changed_files(&rows, 0), 2);
        assert_eq!(changed_files(&rows, 1), 1);
        assert_eq!(changed_files(&rows, 3), 0);
    }
    #[gpui::test]
    fn newer_composer_and_saved_draft_survive_interrupted_edit_recovery(cx: &mut TestAppContext) {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let session = store.create("codex", "/project").unwrap();
        let chat = cx.new(|cx| Conversation::new(session, Arc::new(Mutex::new(store)), cx));
        chat.update(cx, |chat, cx| {
            chat.loaded = true;
            chat.input
                .update(cx, |input, cx| input.set_text("New draft".into(), cx));
            chat.attachments.push("/tmp/new.png".into());
            chat.follow_ups.state.draft = Some(monocode_core::session::follow_up::Message {
                id: "saved".into(),
                text: "Existing draft".into(),
                ..Default::default()
            });
            chat.preserve_interrupted_edit("Edited prompt".into(), Vec::new(), 0, cx);
            assert_eq!(chat.input.read(cx).content, "New draft");
            assert_eq!(chat.attachments, vec![PathBuf::from("/tmp/new.png")]);
            assert_eq!(
                chat.follow_ups.state.draft.as_ref().unwrap().text,
                "Existing draft"
            );
            assert_eq!(chat.follow_ups.state.messages[0].text, "Edited prompt");
            assert!(chat.follow_ups.state.paused);
        });
    }
    #[gpui::test]
    fn local_commands_and_cancel_never_remove_recalled_history(cx: &mut TestAppContext) {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        let mut session = store.create("pi", "/project").unwrap();
        store.provider_id(session.id, "thread").unwrap();
        session.provider_id = Some("thread".into());
        let mut transcript = Transcript::default();
        transcript.begin("user");
        transcript.append("Original");
        transcript.begin("assistant");
        transcript.append("Answer");
        store.write_rows(session.id, 0, &transcript.rows).unwrap();
        let edit = store.prepare_edit(session.id, &transcript.rows, 0).unwrap();
        let chat = cx.new(|cx| Conversation::new(session, Arc::new(Mutex::new(store)), cx));
        chat.update(cx, |chat, cx| {
            chat.loaded = true;
            chat.transcript
                .update(cx, |view, cx| view.patch(0, transcript.rows.clone(), cx));
            chat.editing_last_turn.prepared = Some(edit);
            chat.input
                .update(cx, |input, cx| input.set_text("/compact".into(), cx));
            assert!(chat.submit_edited(cx));
            assert!(chat.editing_last_turn.pending());
            assert_eq!(chat.input.read(cx).content, "/compact");
            assert!(chat.cancel_edit_last_turn(true, cx));
            assert!(!chat.editing_last_turn.pending());
            assert_eq!(chat.input.read(cx).content, "/compact");
            assert_eq!(
                locked(&chat.store)
                    .unwrap()
                    .load(chat.summary.id)
                    .unwrap()
                    .rows,
                transcript.rows
            );
        });
    }
    #[gpui::test]
    fn persisted_recovery_guard_preserves_a_new_composer_until_cancel_lifts_it(
        cx: &mut TestAppContext,
    ) {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        let mut session = store.create("pi", "/project").unwrap();
        store.provider_id(session.id, "original").unwrap();
        session.provider_id = Some("original".into());
        let mut transcript = Transcript::default();
        transcript.begin("user");
        transcript.append("Original");
        store.write_rows(session.id, 0, &transcript.rows).unwrap();
        let edit = store.prepare_edit(session.id, &transcript.rows, 0).unwrap();
        store.provider_id(session.id, "changed").unwrap();
        // An in-place rewind whose local write fails leaves the guard.
        assert!(
            store
                .finish_edit(session.id, &edit, Some("original"))
                .is_err()
        );
        let chat = cx.new(|cx| Conversation::new(session, Arc::new(Mutex::new(store)), cx));
        chat.update(cx, |chat, cx| {
            chat.loaded = true;
            chat.input.update(cx, |input, cx| {
                input.set_text("Unsent edited prompt".into(), cx)
            });
            assert!(chat.submit_edited(cx));
            assert!(chat.editing_last_turn.blocked);
            // Escape leaves the guard in place.
            chat.stop_if_busy(cx);
            assert!(chat.editing_last_turn.blocked);
            assert_eq!(chat.input.read(cx).content, "Unsent edited prompt");
            // Cancel edit is the way out: it lifts the guard and keeps the draft.
            assert!(chat.cancel_edit_last_turn(true, cx));
            assert!(!chat.editing_last_turn.pending());
            assert_eq!(chat.input.read(cx).content, "Unsent edited prompt");
            let store = locked(&chat.store).unwrap();
            assert!(
                store
                    .edit_recovery_reason(chat.summary.id)
                    .unwrap()
                    .is_none()
            );
        });
    }
}
