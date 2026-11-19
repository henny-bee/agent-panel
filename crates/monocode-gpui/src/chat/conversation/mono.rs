mod models;
mod view;
use super::*;
use gpui::{App, EntityInputHandler};
use monocode_core::session::mono_model::{Message, Mono};
use std::path::PathBuf;
#[derive(Default)]
pub(super) struct ChatState {
    pub(super) pending: Vec<Message>,
    queue_scroll: gpui::ScrollHandle,
    queue_ids: Vec<String>,
    queue_list_focus: Option<gpui::FocusHandle>,
    queue_window: Option<gpui::AnyWindowHandle>,
    queue_focus: std::collections::HashMap<String, ([gpui::FocusHandle; 2], [Subscription; 2])>,
    pub(super) emoji: bool,
    pub(super) delivered: std::collections::HashSet<String>,
    pub(super) sending: Option<String>,
    pub(super) failed: Vec<(String, String)>,
    pub(super) external: Option<(String, bool)>,
    /// ModelPicker with `hideSettings`: the model list opens on its own.
    pub(super) models: Option<(Entity<ModelList>, Vec<Subscription>)>,
    /// The setting pills' anchors, by `ComposerMenu::Setting` index.
    pub(super) setting_anchors: Vec<AnchorSlot>,
}
pub(crate) struct Submitted {
    pub text: String,
    pub attachments: Vec<PathBuf>,
}
pub(crate) struct QueueChanged {
    pub id: String,
    pub remove: bool,
}
impl EventEmitter<QueueChanged> for Conversation {}
impl EventEmitter<Submitted> for Conversation {}
impl Conversation {
    #[cfg(test)]
    pub(crate) fn queue_test_focus(&self, id: &str, index: usize) -> gpui::FocusHandle {
        self.mono_ui.queue_focus[id].0[index].clone()
    }

    #[cfg(test)]
    pub(crate) fn queue_test_list_focus(&self) -> gpui::FocusHandle {
        self.mono_ui.queue_list_focus.as_ref().unwrap().clone()
    }

    /// App launches are saved before installing a view. Reuse the normal durable
    /// draft/queue path instead of submitting an unrecorded seed prompt.
    pub(crate) fn install_app_launch(
        &mut self,
        settings: Settings,
        send: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.history_ready() {
            self.defer_history_action(
                move |this, cx| this.install_app_launch(settings, send, cx),
                cx,
            );
            return;
        }
        self.install_transfer(settings, None, cx);
        if send {
            self.draft_action(true, cx);
        }
        cx.emit(SessionChanged);
    }
    pub(crate) fn save_app_draft(&mut self, prompt: String, cx: &mut Context<Self>) {
        self.input
            .update(cx, |input, cx| input.set_text(prompt, cx));
        self.save_message_draft(cx);
    }
    pub(crate) fn stop_app_session(&mut self, cx: &mut Context<Self>) {
        if self.needs_history_restore() {
            self.stop_if_busy(cx);
            self.defer_history_action(|this, cx| this.stop_app_session(cx), cx);
            return;
        }
        self.stop_if_busy(cx);
        self.follow_ups.state.paused = true;
        self.follow_ups.ready = false;
        self.follow_ups.requested = None;
        self.save_follow_ups(cx);
    }
    pub(crate) fn mono_quote(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.input.read(cx).is_composing() {
            return;
        }
        self.input.update(cx, |i, cx| {
            i.set_text(
                monocode_core::session::composer::append_selection_quote(&i.content, text),
                cx,
            );
        });
        window.focus(&self.input.focus_handle(cx), cx);
    }
    pub(crate) fn mono_external_pending(&self) -> bool {
        self.mono_ui.external.is_some()
    }
    pub(crate) fn mono_external_send(
        &mut self,
        prompt: String,
        draft: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.busy
            || self.stopping
            || self.mono_ui.external.is_some()
            || !self.input.read(cx).content.trim().is_empty()
        {
            return Err("Session is busy or has an unsent draft".into());
        }
        self.mono_ui.external = Some((prompt, draft));
        if self.history_ready() {
            self.mono_flush_external(cx);
        } else {
            self.load(cx);
        }
        cx.emit(SessionChanged);
        Ok(())
    }
    pub(super) fn mono_flush_external(&mut self, cx: &mut Context<Self>) {
        if !self.history_ready() || self.stopping {
            return;
        }
        if let Some((prompt, draft)) = self.mono_ui.external.take() {
            if draft {
                self.input.update(cx, |i, cx| i.set_text(prompt, cx));
            } else {
                self.install_transfer(self.settings.clone(), Some(prompt), cx);
            }
        }
    }
    pub(crate) fn mono_attention(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        div()
            .child(self.approval_shelf(window, cx))
            .when_some(self.questions.front(), |d, (card, _)| {
                d.child(div().mb_2().child(card.clone()))
            })
            .into_any_element()
    }
    pub(crate) fn configure_mono(
        &mut self,
        mono: Mono,
        root: PathBuf,
        environment: std::collections::HashMap<String, String>,
        cx: &mut Context<Self>,
    ) {
        self.input
            .update(cx, |input, cx| input.mono_composer(&mono.label(), cx));
        let name = mono.label();
        self.transcript
            .update(cx, |t, _| t.mono_mode(&mono.mascot, mono.color, &name));
        self.settings.environment = environment;
        self.mono_ui.pending = mono.outbox.clone();
        self.mono_ui
            .delivered
            .retain(|id| mono.outbox.iter().any(|m| &m.id == id));
        self.sync_mono_pending(cx);
        self.settings.mono = Some(monocode_core::session::agent::MonoContext {
            mono,
            root,
            executable: std::env::current_exe().unwrap_or_default(),
        });
        cx.notify();
    }
    /// The user's queued messages show in the transcript as bubbles; the one
    /// just delivered is already a transcript row.
    pub(super) fn sync_mono_pending(&mut self, cx: &mut Context<Self>) {
        let pending = self
            .mono_ui
            .pending
            .iter()
            .filter(|m| !m.internal && !self.mono_ui.delivered.contains(&m.id))
            .map(|m| crate::chat::transcript::PendingMessage {
                id: m.id.clone(),
                text: m.text.clone().into(),
                error: m.error.clone(),
                sent_at: m.sent_at,
                attachments: m.attachments.iter().map(PathBuf::from).collect(),
            })
            .collect();
        self.transcript
            .update(cx, |t, cx| t.set_pending(pending, cx));
    }
    pub(crate) fn mono_dispatch(&mut self, message: &Message, cx: &mut Context<Self>) -> bool {
        if self.mono_ui.delivered.contains(&message.id) || (self.busy && message.internal) {
            return false;
        }
        // Its own turn is already running and hasn't reported it delivered.
        // Taken for a new send, it would steer into itself, or after a
        // provider switch, which stays armed until the turn takes it up, stop
        // the turn it just started before the new provider says a word.
        if self.busy && self.settings.mono_message_id.as_deref() == Some(message.id.as_str()) {
            return false;
        }
        if !self.history_ready() {
            self.load(cx);
            return false;
        }
        if self.settings.pending_provider_switch.is_some() && self.busy {
            self.stop(cx);
        }
        if self.stopping
            || !self.loaded
            || self.settings.usage_limit.is_some()
            || self.account_switching
        {
            return false;
        }
        if self.busy {
            if self.mono_ui.sending.is_some()
                || !self.follow_ups.ready
                || !Kind::from_id(&self.summary.provider).can_steer()
                || self
                    .questions
                    .iter()
                    .any(|(card, _)| card.read(cx).blocks_steering())
            {
                return false;
            }
            let prepared = self.completion.read(cx).prepare_prompt(&message.text);
            let steer = monocode_core::session::follow_up::Message {
                id: message.id.clone(),
                text: message.text.clone(),
                sent_at: message.sent_at,
                prepared_prompt: (prepared != message.text).then_some(prepared),
                attachments: message.attachments.iter().map(PathBuf::from).collect(),
                steer: true,
                ..Default::default()
            };
            if self.controls.as_ref().is_some_and(|controls| {
                controls
                    .send(Control {
                        id: message.id.clone(),
                        decision: Decision::Steer(steer),
                    })
                    .is_ok()
            }) {
                self.mono_ui.sending = Some(message.id.clone());
                return true;
            }
            return false;
        }
        self.mono_ui.sending = None;
        self.settings.mono_message_id = Some(message.id.clone());
        self.settings.mono_sent_at = message.sent_at;
        self.settings.internal_message = message.internal;
        self.settings.attachments = message.attachments.iter().map(PathBuf::from).collect();
        self.launch_prompt(message.text.clone(), cx);
        true
    }
    pub(crate) fn mono_rows<'a>(
        &self,
        cx: &'a App,
    ) -> &'a [monocode_core::session::transcript::Row] {
        self.transcript.read(cx).source_rows()
    }
    pub(crate) fn mono_patch_card(
        &mut self,
        index: usize,
        id: &str,
        value: serde_json::Value,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if !self.history_ready() || self.stopping {
            self.load(cx);
            return Err("Conversation history is loading; retry the update when ready".into());
        }
        let current = self
            .transcript
            .read(cx)
            .source_rows()
            .get(index)
            .filter(|r| r.role == "mono-card")
            .and_then(|r| serde_json::from_str::<serde_json::Value>(&r.text).ok());
        if current.as_ref().and_then(|v| v["id"].as_str()) != Some(id) {
            return Ok(());
        }
        let row = monocode_core::session::transcript::Row {
            role: "mono-card".into(),
            text: value.to_string(),
            first: true,
            sent_at: self.transcript.read(cx).source_rows()[index].sent_at,
        };
        locked(&self.store)?.write_rows(self.summary.id, index, std::slice::from_ref(&row))?;
        self.transcript
            .update(cx, |t, cx| t.patch(index, vec![row], cx));
        cx.notify();
        Ok(())
    }
    pub(crate) fn mono_status(&self) -> &str {
        if self.needs_approval() || self.settings.usage_limit.is_some() {
            "needs-you"
        } else if self.responding() {
            "working"
        } else {
            "idle"
        }
    }
    pub(crate) fn mono_model(&self) -> Option<String> {
        self.model.clone()
    }
    pub(crate) fn branch_name(&self) -> Option<&str> {
        self.branch.as_deref()
    }
    pub(crate) fn mono_model_label(&self) -> SharedString {
        self.model_label()
    }
    pub(crate) fn mono_stop(&mut self, cx: &mut Context<Self>) {
        self.stop_if_busy(cx);
    }
    pub(crate) fn mono_post(
        &mut self,
        role: &str,
        text: String,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if !self.history_ready() {
            self.load(cx);
            return Err("Conversation history is loading; retry the report when ready".into());
        }
        if self.busy || self.stopping {
            return Err("Wait for the current turn before posting a report".into());
        }
        let mut posted = monocode_core::session::transcript::Transcript::default();
        if role == "assistant" {
            // A posted report is its own completed run. Its boundary keeps a
            // later report from hiding the previous notification's final reply.
            posted.begin("mono-system");
            posted.append("[Scheduled Mono report]");
        }
        posted.begin(role);
        if role == "mono-card" {
            // Card JSON is one metadata row, including large summaries.
            posted.rows[0].text = text;
        } else {
            // Match the store's row boundaries so the next post starts after
            // the complete report, even when it spans multiple display rows.
            posted.append(&text);
        }
        let at = self.transcript.read(cx).source_rows().len();
        locked(&self.store)?.write_rows(self.summary.id, at, &posted.rows)?;
        self.transcript
            .update(cx, |t, cx| t.patch(at, posted.rows, cx));
        self.summary.has_messages = true;
        cx.emit(SessionChanged);
        cx.notify();
        Ok(())
    }
    pub(crate) fn mono_access_options(&self) -> Vec<MenuItem> {
        let provider = Kind::from_id(&self.summary.provider);
        let mut items = vec![
            MenuAction::new("access:read-only", "Read-only")
                .icon("square-lock")
                .description("Inspect without changing files.")
                .checked(self.settings.access == AccessMode::ReadOnly)
                .into(),
        ];
        items.extend(ACCESS_MODES.iter().map(|(id, label, hint, glyph)| {
                    let hint = match (provider,*id) {
                        (Kind::Cursor,"supervised") => "Confirm commands using Cursor's permission policy; Cursor may accept edits automatically.",
                        (Kind::Pi | Kind::Omp,_) => "Tool confirmations follow this provider's configured extensions.",
                        _ => *hint,
                    };
                    let action = MenuAction::new(format!("access:{id}"), *label)
                        .icon(glyph)
                        .description(hint)
                        .checked(self.settings.access.id() == *id);
                    MenuItem::from(if *id == "full-access" {
                        action.icon_color(tint(AMBER_400, 0.9))
                    } else {
                        action
                    })
                }));
        if self.busy {
            items.push(MenuItem::Note("Changes apply to the next turn.".into()));
        }
        items
    }
    pub(crate) fn mono_access(
        &mut self,
        mode: monocode_core::session::agent::AccessMode,
        cx: &mut Context<Self>,
    ) {
        self.settings.access = mode;
        super::remember_access(mode, cx);
        self.save_settings(cx);
        cx.notify();
    }
    pub(crate) fn mono_delivered(&self) -> &std::collections::HashSet<String> {
        &self.mono_ui.delivered
    }
    pub(super) fn mono_message_delivered(&mut self, id: String, cx: &mut Context<Self>) {
        if self.mono_ui.sending.as_ref() == Some(&id) {
            self.mono_ui.sending = None;
        }
        self.mono_ui.delivered.insert(id);
        self.sync_mono_pending(cx);
    }
    pub(super) fn mono_message_failed(&mut self, id: String, error: String) {
        if self.mono_ui.sending.as_ref() == Some(&id) {
            self.mono_ui.sending = None;
        }
        self.mono_ui.failed.push((id, error));
    }
    pub(crate) fn mono_take_failures(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.mono_ui.failed)
    }
    pub(crate) fn mono_config(
        &mut self,
        provider: Kind,
        model: Option<String>,
        values: std::collections::HashMap<String, String>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.busy || self.stopping {
            return Err("Wait for the current reply before changing models".into());
        }
        if self.summary.provider != provider.id() {
            self.summary.provider = provider.id().into();
            self.summary.provider_id = None;
            self.catalog.clear();
        }
        self.model = model;
        self.settings.effort = super::preferred_effort(provider.id(), cx);
        self.settings.fast = false;
        self.settings.thinking = None;
        self.settings.service_tier = None;
        if let Some(context) = &mut self.context {
            context.window = None;
        }
        self.settings.model_settings = values;
        self.resolved_model = None;
        self.refresh_catalog(cx);
        self.save_settings(cx);
        cx.emit(SessionChanged);
        cx.notify();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
