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
    _follow: Task<()>,
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
                        .try_global::<crate::ui::preferences::Preferences>()
                        .is_some_and(|preferences| preferences.claude_terminal))
    }

    /// What this session did, read from the files Claude Code writes, so only
    /// local Claude sessions have it, in the CLI or in the chat view.
    pub(crate) fn claude_activity(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<Entity<crate::chat::claude::panels::ClaudeActivity>> {
        if Kind::from_id(&self.summary.provider) != Kind::Claude
            || self.settings.mono.is_some()
            || self.is_remote()
        {
            return None;
        }
        let me = cx.entity().downgrade();
        Some(
            self.claude_activity
                .get_or_insert_with(|| {
                    cx.new(|cx| crate::chat::claude::panels::ClaudeActivity::new(me, cx))
                })
                .clone(),
        )
    }

    /// The account, Claude session id and working folder the activity
    /// panels read, and whether the model has a 1M window; none until Claude
    /// has a session.
    pub(crate) fn claude_source(
        &self,
    ) -> Option<(
        monocode_core::session::profile::Profile,
        String,
        String,
        bool,
    )> {
        Some((
            self.settings.account.clone(),
            self.summary.provider_id.clone()?,
            self.execution_cwd().to_owned(),
            self.model
                .as_deref()
                .is_some_and(|model| model.to_ascii_lowercase().ends_with("[1m]")),
        ))
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
            .filter_map(|(key, value)| {
                Some((key.to_str()?.to_owned(), value?.to_str()?.to_owned()))
            })
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
        let pid = terminal.read(cx).pid();
        let account = self.settings.account.clone();
        let mut named = self.summary.has_messages;
        let follow = cx.spawn(async move |this, cx| {
            let mut session = session;
            // ponytail: polls every few seconds; a file watcher if many idle
            // tabs make it show.
            loop {
                cx.background_executor().timer(Duration::from_secs(3)).await;
                let (account, bound) = (account.clone(), session.clone());
                let (live, prompt) = cx
                    .background_executor()
                    .spawn(async move {
                        // A `/clear` or `/resume` typed in the CLI moves it to
                        // another session; the tab goes with it.
                        let live = provider::claude_live_session(&account, pid).unwrap_or(bound);
                        // Once named, the transcript is not read again.
                        let prompt = (!named)
                            .then(|| provider::claude_first_prompt(&account, &live))
                            .flatten();
                        (live, prompt)
                    })
                    .await;
                let followed = this.update(cx, |this, cx| {
                    this.claude_terminal_bound(&live, cx);
                    if let Some(prompt) = prompt {
                        this.claude_terminal_named(&prompt, cx);
                    }
                    this.summary.has_messages
                });
                match followed {
                    Ok(has_messages) => named = has_messages,
                    Err(_) => break,
                }
                session = live;
            }
        });
        Ok(State {
            terminal,
            _events: events,
            _follow: follow,
        })
    }

    /// Rebinds the tab to the session its running CLI moved to, so a restart
    /// resumes that conversation. One it leaves with messages keeps an entry.
    fn claude_terminal_bound(&mut self, session: &str, cx: &mut Context<Self>) {
        let running = self
            .claude_terminal
            .as_ref()
            .is_some_and(|state| !state.terminal.read(cx).exited);
        if !running || self.summary.provider_id.as_deref() == Some(session) {
            return;
        }
        let moved = (|| {
            let left = match self.summary.has_messages {
                true => Some(self.claude_terminal_left(session)?),
                false => None,
            };
            locked(&self.store)?.provider_id(self.summary.id, session)?;
            Ok::<_, String>(left)
        })();
        match moved {
            Ok(left) => {
                self.summary.provider_id = Some(session.to_owned());
                if let Some(left) = left {
                    cx.emit(SessionLeft(left));
                }
                cx.emit(SessionChanged);
                cx.notify();
            }
            Err(error) => self.status = error,
        }
    }

    /// Gives the conversation this tab is leaving an entry of its own and
    /// returns it. An idle CLI entry that already lists `session` trades places
    /// with the tab, so nothing is listed twice; otherwise a new entry is made
    /// and the tab starts over, to be named from its next first message.
    fn claude_terminal_left(&mut self, session: &str) -> Result<Session, String> {
        let mut store = locked(&self.store)?;
        let id = self.summary.id;
        // Only CLI entries trade: they hold one row, a chat entry its transcript.
        let listed = store.sessions()?.into_iter().find(|other| {
            other.id != id
                && !other.archived
                && other.has_messages
                && other.provider_id.as_deref() == Some(session)
                && store
                    .agent_settings(other.id)
                    .ok()
                    .flatten()
                    .is_some_and(|settings| settings.terminal)
        });
        let mut left = match listed {
            Some(listed) => listed,
            None => {
                let created = store.create(&self.summary.provider, &self.summary.cwd)?;
                let mut settings = self.settings.clone();
                settings.model = self.model.clone();
                store.save_agent_settings(created.id, &settings)?;
                created
            }
        };
        let (mine, theirs) = (store.load(id)?.rows, store.load(left.id)?.rows);
        store.title(left.id, &self.summary.title)?;
        store.write_rows(left.id, 0, &mine)?;
        if let Some(leaving) = &self.summary.provider_id {
            store.provider_id(left.id, leaving)?;
        }
        store.title(id, &left.title)?;
        store.write_rows(id, 0, &theirs)?;
        drop(store);
        self.cancel_title_generation();
        let (title, has_messages) = (left.title.clone(), left.has_messages);
        left.title = std::mem::replace(&mut self.summary.title, title);
        left.has_messages = std::mem::replace(&mut self.summary.has_messages, has_messages);
        left.provider_id = self.summary.provider_id.clone();
        Ok(left)
    }

    /// This entry traded places with a tab whose CLI moved onto its
    /// conversation: it now lists the one that tab left.
    pub(crate) fn claude_terminal_traded(&mut self, summary: Session, cx: &mut Context<Self>) {
        self.cancel_title_generation();
        // A CLI of its own would still be on the conversation the tab took.
        self.claude_terminal = None;
        self.summary.title = summary.title;
        self.summary.provider_id = summary.provider_id;
        self.summary.has_messages = summary.has_messages;
        cx.emit(SessionChanged);
        cx.notify();
    }

    /// The CLI has its first message: title the session and save one row, which
    /// is what lists it in the sidebar and in search after a restart.
    fn claude_terminal_named(&mut self, prompt: &str, cx: &mut Context<Self>) {
        if self.summary.has_messages {
            return;
        }
        self.generate_title(prompt, cx);
        let mut transcript = monocode_core::session::transcript::Transcript::default();
        transcript.begin("user");
        transcript.append(prompt);
        if let Err(error) = locked(&self.store)
            .and_then(|mut store| store.write_rows(self.summary.id, 0, &transcript.rows))
        {
            self.status = error;
        }
        self.summary.has_messages = true;
        self.summary.updated_at = monocode_core::sys::time::now_ms();
        cx.emit(SessionChanged);
        cx.notify();
    }
}
