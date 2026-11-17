//! A Claude session shown as the Claude Code CLI itself, in place of the
//! transcript and composer. The CLI owns the conversation; this view starts it,
//! names the session from its first message and offers a restart when it exits.
use super::*;
use crate::terminal::{Program, Terminal, TerminalEvent};
use gpui::{App, FocusHandle, Task};
use monocode_core::provider;
use std::time::Duration;

pub(super) struct State {
    terminal: Entity<Terminal>,
    _events: Subscription,
    _named: Option<Task<()>>,
}

impl Conversation {
    /// New Claude sessions run in the CLI; one that started there stays there.
    /// Sessions with a chat history here, Monos and remote hosts keep the chat.
    fn claude_terminal_wanted(&self, cx: &App) -> bool {
        Kind::from_id(&self.summary.provider) == Kind::Claude
            && self.settings.mono.is_none()
            && !self.is_remote()
            && (self.settings.terminal
                || !self.summary.has_messages
                    && cx
                        .try_global::<crate::settings::Preferences>()
                        .is_some_and(|preferences| preferences.claude_terminal))
    }

    /// What the keyboard should reach when this session becomes active.
    pub(crate) fn session_focus(&self, cx: &App) -> FocusHandle {
        match &self.claude_terminal {
            Some(state) => state.terminal.focus_handle(cx),
            None => self.input.focus_handle(cx),
        }
    }

    /// The CLI filling the session area, started on first use.
    pub(super) fn claude_terminal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.claude_terminal_wanted(cx) {
            return None;
        }
        // Saved settings name the account and whether the session began here.
        if !self.loaded {
            return Some(div().size_full().into_any());
        }
        if self.claude_terminal.is_none() {
            match self.start_claude_terminal(cx) {
                Ok(state) => self.claude_terminal = Some(state),
                Err(error) => {
                    return Some(
                        div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(gpui::rgb(MUTED))
                            .child(error)
                            .into_any(),
                    );
                }
            }
        }
        let terminal = self.claude_terminal.as_ref()?.terminal.clone();
        // The composer is not shown, so focus meant for it belongs to the CLI.
        if self.input.focus_handle(cx).is_focused(window) {
            window.focus(&terminal.focus_handle(cx), cx);
        }
        let exited = terminal.read(cx).exited;
        Some(
            div()
                .relative()
                .size_full()
                // No fill of its own: the pane's card, glass and chat
                // background show behind the CLI as behind any session.
                .child(terminal.cached(StyleRefinement::default().size_full()))
                .when(exited, |d| {
                    d.child(
                        div()
                            .id("restart-claude-terminal")
                            .absolute()
                            .bottom_3()
                            .right_3()
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(gpui::rgb(MUTED))
                            .text_color(gpui::rgb(TEXT))
                            .cursor_pointer()
                            .child("Restart Claude Code")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.claude_terminal = None;
                                // The next render hands this focus to the new terminal.
                                window.focus(&this.input.focus_handle(cx), cx);
                                cx.notify();
                            })),
                    )
                })
                .into_any(),
        )
    }

    fn start_claude_terminal(&mut self, cx: &mut Context<Self>) -> Result<State, String> {
        let binary = provider::resolve(Kind::Claude)
            .ok_or("The Claude Code executable was not found. Set it in Settings, Providers.")?;
        self.attach_account(cx);
        let mut account = std::process::Command::new(&binary);
        self.settings.account.apply(Kind::Claude, &mut account)?;
        let env = account
            .get_envs()
            .filter_map(|(key, value)| Some((key.to_str()?.to_owned(), value?.to_str()?.to_owned())))
            .collect();
        let session = match self.summary.provider_id.clone() {
            Some(session) => session,
            None => {
                let session = provider::claude_session_id();
                locked(&self.store)?.provider_id(self.summary.id, &session)?;
                self.summary.provider_id = Some(session.clone());
                session
            }
        };
        if !self.settings.terminal {
            self.settings.terminal = true;
            self.save_settings(cx);
        }
        // Claude saves a session with its first message; until then the id is
        // still free to start with.
        // ponytail: the binding follows the id the CLI was started with. A
        // `/clear` or `/resume` typed inside it moves the CLI to another session
        // that this tab does not follow; track the CLI's session id if that bites.
        let flag = if provider::claude_transcript(&self.settings.account, &session).is_some() {
            "--resume"
        } else {
            "--session-id"
        };
        let mut args = vec![flag.into(), session.clone()];
        // The model picked for the session; without one the CLI uses its own.
        if let Some(model) = &self.model {
            args.extend(["--model".into(), model.clone()]);
        }
        let program = Program {
            path: binary.to_string_lossy().into_owned(),
            args,
            env,
        };
        let cwd = self.execution_cwd().to_owned();
        let terminal = cx.new(|cx| Terminal::program(cwd, program, cx));
        let events = cx.subscribe(&terminal, |_, _, _: &TerminalEvent, cx| cx.notify());
        let named = (!self.summary.has_messages).then(|| {
            let account = self.settings.account.clone();
            cx.spawn(async move |this, cx| {
                // ponytail: polls the transcript every few seconds until the
                // first message; a file watcher if many idle tabs make it show.
                loop {
                    cx.background_executor().timer(Duration::from_secs(3)).await;
                    let (account, session) = (account.clone(), session.clone());
                    let prompt = cx
                        .background_executor()
                        .spawn(async move { provider::claude_first_prompt(&account, &session) })
                        .await;
                    let named = this.update(cx, |this, cx| {
                        prompt.is_some_and(|prompt| {
                            this.claude_terminal_named(&prompt, cx);
                            true
                        })
                    });
                    if !matches!(named, Ok(false)) {
                        break;
                    }
                }
            })
        });
        Ok(State {
            terminal,
            _events: events,
            _named: named,
        })
    }

    /// The CLI has its first message: title the session and save one row, which
    /// is what lists it in the sidebar and in search after a restart.
    fn claude_terminal_named(&mut self, prompt: &str, cx: &mut Context<Self>) {
        if self.summary.has_messages {
            return;
        }
        self.generate_title(prompt, cx);
        let mut transcript = monocode_core::transcript::Transcript::default();
        transcript.begin("user");
        transcript.append(prompt);
        if let Err(error) =
            locked(&self.store).and_then(|mut store| store.write_rows(self.summary.id, 0, &transcript.rows))
        {
            self.status = error;
        }
        self.summary.has_messages = true;
        self.summary.updated_at = monocode_core::time::now_ms();
        cx.emit(SessionChanged);
        cx.notify();
    }
}
