//! `/resume` in a Claude chat: the Claude Code sessions for this folder, as
//! the CLI's picker lists them, started here or not. The words after
//! `/resume` filter the list; ↑/↓ choose, Enter or a click opens the pick as
//! a chat in its own tab.
use super::*;
use gpui::App;
use monocode_core::provider::{self, ClaudeSession};
use monocode_core::store::Store;

/// A session `/resume` picked, to show in its tab (opened if it has none).
pub(crate) struct Resumed(pub(crate) Session);

#[derive(Default)]
pub(super) struct Picker {
    /// None until the folder has been read.
    sessions: Option<Vec<ClaudeSession>>,
    selected: usize,
    query: String,
    opening: bool,
}

/// How many rows show before the list scrolls.
const ROWS: f32 = 7.;
const ROW_HEIGHT: f32 = 30.;

fn ago(ms: u64) -> String {
    let seconds = monocode_core::sys::time::now_ms().saturating_sub(ms) / 1000;
    match seconds {
        0..60 => "now".into(),
        60..3600 => format!("{}m ago", seconds / 60),
        3600..86400 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86400),
    }
}

/// The sessions whose title or branch hold every word of `query`, except the
/// one this chat is.
fn matching<'a>(
    sessions: &'a [ClaudeSession],
    query: &str,
    current: Option<&str>,
) -> Vec<&'a ClaudeSession> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    sessions
        .iter()
        .filter(|session| Some(session.id.as_str()) != current)
        .filter(|session| {
            let text = format!(
                "{} {}",
                session.title,
                session.branch.as_deref().unwrap_or_default()
            )
            .to_lowercase();
            words.iter().all(|word| text.contains(word))
        })
        .collect()
}

impl Conversation {
    /// The `/resume` filter while the composer holds the command, in a local
    /// Claude chat.
    fn resume_query(&self, cx: &App) -> Option<String> {
        (Kind::from_id(&self.summary.provider) == Kind::Claude
            && self.settings.mono.is_none()
            && !self.is_remote())
        .then(|| {
            monocode_core::session::composer::consume_command(
                &self.input.read(cx).content,
                "resume",
            )
            .map(str::to_owned)
        })
        .flatten()
    }

    /// Opens, filters or closes the picker as the composer changes. The folder
    /// is read once per opening, off the UI thread.
    pub(super) fn sync_resume(&mut self, cx: &mut Context<Self>) {
        let Some(query) = self.resume_query(cx) else {
            if self.resume.take().is_some() {
                cx.notify();
            }
            return;
        };
        if let Some(picker) = &mut self.resume {
            if picker.query != query {
                picker.query = query;
                picker.selected = 0;
                cx.notify();
            }
            return;
        }
        self.resume = Some(Picker {
            query,
            ..Default::default()
        });
        let account = self.settings.account.clone();
        let cwd = self.summary.cwd.clone();
        let work = cx
            .background_executor()
            .spawn(async move { provider::claude_sessions(&account, &cwd) });
        cx.spawn(async move |this, cx| {
            let sessions = work.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(picker) = &mut this.resume {
                    picker.sessions = Some(sessions);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn resume_choices(&self) -> Vec<&ClaudeSession> {
        let Some(picker) = &self.resume else {
            return Vec::new();
        };
        let current = self
            .summary
            .provider_id
            .as_deref()
            .map(provider::claude_session);
        matching(
            picker.sessions.as_deref().unwrap_or_default(),
            &picker.query,
            current,
        )
    }

    /// ↑/↓ while the picker is open. False when it isn't.
    pub(super) fn resume_step(&mut self, up: bool, cx: &mut Context<Self>) -> bool {
        let count = self.resume_choices().len();
        let Some(picker) = &mut self.resume else {
            return false;
        };
        if count > 0 {
            picker.selected = if up {
                picker.selected.checked_sub(1).unwrap_or(count - 1)
            } else {
                (picker.selected + 1) % count
            };
            cx.notify();
        }
        true
    }

    /// Enter while the picker is open opens the chosen session. False when
    /// the picker isn't open.
    pub(super) fn resume_pick(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(picker) = &self.resume else {
            return false;
        };
        let chosen = self.resume_choices().get(picker.selected).copied().cloned();
        if let Some(session) = chosen {
            self.resume_open(session, cx);
        }
        true
    }

    /// Shows `session` in a tab: the chat MCode already has for it, else a new
    /// chat holding what the CLI wrote.
    fn resume_open(&mut self, session: ClaudeSession, cx: &mut Context<Self>) {
        let Some(picker) = &mut self.resume else {
            return;
        };
        if picker.opening {
            return;
        }
        picker.opening = true;
        cx.notify();
        let store = self.store.clone();
        let account = self.settings.account.clone();
        let cwd = self.summary.cwd.clone();
        let mut settings = self.settings.clone();
        settings.terminal = false;
        settings.chat = true;
        settings.plan = false;
        settings.turns.clear();
        settings.mono = None;
        settings.model = self.model.clone();
        let work = cx.background_executor().spawn(async move {
            let listed = |store: &Store| {
                Ok::<_, String>(store.sessions()?.into_iter().find(|listed| {
                    !listed.archived
                        && Kind::from_id(&listed.provider) == Kind::Claude
                        && listed
                            .provider_id
                            .as_deref()
                            .is_some_and(|bound| provider::claude_session(bound) == session.id)
                }))
            };
            if let Some(found) = listed(&*locked(&store)?)? {
                return Ok(found);
            }
            // Read without holding the store: a long session takes a moment.
            let rows = match provider::claude_transcript(&account, &session.id) {
                Some(path) => provider::claude_history(&path, &session.id)?,
                None => Vec::new(),
            };
            let mut store = locked(&store)?;
            let created = store.create("claude", &cwd)?;
            store.provider_id(created.id, &session.id)?;
            store.title(created.id, &session.title)?;
            store.save_agent_settings(created.id, &settings)?;
            store.replace_rows(created.id, &rows)?;
            store
                .sessions()?
                .into_iter()
                .find(|listed| listed.id == created.id)
                .ok_or_else(|| "The resumed session was not saved".to_owned())
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(picker) = &mut this.resume {
                    picker.opening = false;
                }
                match result {
                    Ok(session) => {
                        this.input
                            .update(cx, |input, cx| input.set_text(String::new(), cx));
                        this.resume = None;
                        cx.emit(Resumed(session));
                    }
                    Err(error) => this.cut_notice(error, true, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn resume_picker(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let picker = self.resume.as_ref()?;
        let choices = self.resume_choices();
        let empty = match &picker.sessions {
            None => Some("Reading Claude Code sessions…"),
            Some(all) if all.is_empty() => Some("No Claude Code sessions in this folder yet."),
            Some(_) if choices.is_empty() => Some("No session matches."),
            Some(_) => None,
        };
        let rows = choices.iter().enumerate().map(|(ix, session)| {
            let on = ix == picker.selected;
            let picked = (*session).clone();
            div()
                .id(("resume-session", ix))
                .debug_selector(move || format!("resume-session-{ix}"))
                .role(Role::ListBoxOption)
                .aria_selected(on)
                .h(px(ROW_HEIGHT))
                .flex_shrink_0()
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded(px(8.))
                .cursor_pointer()
                .when(on, |d| d.bg(content(0.08)))
                .hover(|s| s.bg(content(0.06)))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .truncate()
                        .text_color(content(0.9))
                        .child(session.title.clone()),
                )
                .children(session.branch.clone().map(|branch| {
                    div()
                        .flex_shrink_0()
                        .max_w(px(160.))
                        .truncate()
                        .font_family(crate::ui::fonts::mono())
                        .text_size(px(11.))
                        .text_color(content(0.45))
                        .child(branch)
                }))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(11.))
                        .text_color(content(0.45))
                        .child(ago(session.modified)),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.resume_open(picked.clone(), cx)))
        });
        Some(
            div()
                .id("resume-picker")
                .debug_selector(|| "resume-picker".into())
                .role(Role::ListBox)
                .aria_label("Resume a Claude Code session")
                .mb(px(6.))
                .p(px(4.))
                .rounded(px(10.))
                .bg(content(0.05))
                .flex()
                .flex_col()
                .text_size(px(12.))
                .child(
                    div()
                        .px(px(10.))
                        .py(px(6.))
                        .text_size(px(11.))
                        .text_color(content(0.45))
                        .child(if picker.opening {
                            "Opening…"
                        } else {
                            "Resume a Claude Code session · type to filter · ↑↓ Enter"
                        }),
                )
                .child(
                    div()
                        .id("resume-sessions")
                        .max_h(px(ROWS * ROW_HEIGHT))
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .children(rows),
                )
                .children(empty.map(|text| {
                    div()
                        .px(px(10.))
                        .py(px(6.))
                        .text_color(content(0.55))
                        .child(text)
                }))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, title: &str, branch: Option<&str>) -> ClaudeSession {
        ClaudeSession {
            id: id.into(),
            title: title.into(),
            modified: 0,
            branch: branch.map(Into::into),
        }
    }

    #[gpui::test]
    fn typing_resume_lists_sessions_and_a_pick_opens_once(cx: &mut gpui::TestAppContext) {
        use std::sync::{Arc, Mutex};
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let summary = store
            .create("claude", "/work/no-claude-sessions-here")
            .unwrap();
        let store = Arc::new(Mutex::new(store));
        let window = cx.update(|cx| {
            crate::ui::input::bind(cx);
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| Conversation::new(summary, store.clone(), cx))
            })
            .unwrap()
        });
        cx.run_until_parked();
        window
            .update(cx, |c, _, cx| {
                c.input
                    .update(cx, |input, cx| input.set_text("/resume parser".into(), cx));
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |c, _, _| {
                let picker = c.resume.as_ref().expect("picker opens for /resume");
                assert_eq!(picker.query, "parser");
                assert_eq!(picker.sessions.as_deref(), Some(&[][..]));
            })
            .unwrap();
        let picked = session("not-on-disk", "Fix the parser", Some("main"));
        let opened = Arc::new(Mutex::new(Vec::new()));
        window
            .update(cx, |_, _, cx| {
                let opened = opened.clone();
                cx.subscribe(&cx.entity(), move |_, _, event: &Resumed, _| {
                    opened.lock().unwrap().push(event.0.id)
                })
                .detach();
            })
            .unwrap();
        for _ in 0..2 {
            window
                .update(cx, |c, _, cx| {
                    c.input
                        .update(cx, |input, cx| input.set_text("/resume".into(), cx));
                    c.sync_resume(cx);
                    c.resume_open(picked.clone(), cx);
                })
                .unwrap();
            cx.run_until_parked();
        }
        let opened = opened.lock().unwrap().clone();
        assert_eq!(opened.len(), 2);
        assert_eq!(
            opened[0], opened[1],
            "the second pick reuses the first chat"
        );
        let store = store.lock().unwrap();
        let made = store.sessions().unwrap();
        let made = made.iter().find(|s| s.id == opened[0]).unwrap();
        assert_eq!(made.provider_id.as_deref(), Some("not-on-disk"));
        assert!(store.agent_settings(made.id).unwrap().unwrap().chat);
        window
            .update(cx, |c, _, cx| {
                assert!(c.resume.is_none());
                assert!(c.input.read(cx).content.is_empty());
            })
            .unwrap();
    }

    #[test]
    fn every_word_must_match_the_title_or_branch_and_this_chat_is_left_out() {
        let all = [
            session("a", "Fix the parser", Some("main")),
            session("b", "Landing page redesign", Some("feat/landing")),
            session("c", "Parser speedups", None),
        ];
        let ids = |query: &str, current: Option<&str>| {
            matching(&all, query, current)
                .iter()
                .map(|s| s.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids("", None), ["a", "b", "c"]);
        assert_eq!(ids("PARSER", None), ["a", "c"]);
        assert_eq!(ids("parser main", None), ["a"]);
        assert_eq!(ids("landing", Some("b")), Vec::<String>::new());
    }
}
