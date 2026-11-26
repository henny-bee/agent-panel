//! Permission requests use the queued-message shelf and its upward reveal.
use super::*;
use std::cell::RefCell;

#[derive(Default)]
pub(super) struct State {
    current: RefCell<Option<String>>,
    motion: composer_shelf::Motion,
    /// Revise was chosen with an empty message box: the card asks for the
    /// change there.
    revise_hint: std::cell::Cell<bool>,
}

impl State {
    fn sync(&self, id: Option<&str>, animate: bool) {
        let mut current = self.current.borrow_mut();
        if current.as_deref() != id {
            *current = id.map(str::to_owned);
            if let Some(id) = id {
                self.motion.insert(id, true, animate);
            }
        }
        self.motion.retain(id.into_iter(), animate);
    }
}

impl Conversation {
    pub(super) fn approval_shelf(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let width = self.composer_anchor.get().map(|bounds| bounds.size.width);
        self.approval_ui.sync(
            self.approvals.front().map(|approval| approval.id.as_str()),
            !cx.reduce_motion() && width.is_some(),
        );
        let Some(approval) = self.approvals.front() else {
            return div().into_any_element();
        };
        let mut ink = shell_fill();
        ink.a = 1.;
        if approval.plan.is_some() {
            return self.plan_shelf(width, ink, window, cx);
        }
        // "1 of 3" when more requests wait behind this one.
        let queued = (self.approvals.len() > 1).then(|| {
            div()
                .flex_shrink_0()
                .text_size(px(11.))
                .text_color(content(0.45))
                .child(format!("1 of {}", self.approvals.len()))
        });
        let deny_id = approval.id.clone();
        let session_id = approval.id.clone();
        let allow_id = approval.id.clone();
        let actions = div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(
                approval_button("approval-deny", "Deny")
                    .child(key_hint("Esc".into()))
                    .text_color(content(0.5))
                    .hover(|s| s.bg(content(0.08)).text_color(content(0.9)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.decide_notification_request(&deny_id, Decision::Deny, cx);
                    })),
            )
            .when(approval.can_remember, |d| {
                d.child(
                    approval_button("approval-session", "Allow for this session")
                        .child(key_hint(shortcut("Shift+Enter")))
                        .bg(content(0.06))
                        .hover(|s| s.bg(content(0.1)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.decide_notification_request(&session_id, Decision::Session, cx);
                        })),
                )
            })
            .child(
                approval_button("approval-allow", "Allow once")
                    .child(key_hint(shortcut("Enter")))
                    .bg(content(1.))
                    .text_color(ink)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .hover(move |s| s.bg(content(0.85)).text_color(ink))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.decide_notification_request(&allow_id, Decision::Allow, cx);
                    })),
            );
        // The question and its answers share one row; any detail follows below.
        let card = composer_shelf::card("approval-request", self.classic_layout)
            .role(Role::Group)
            .aria_label(approval.title.clone())
            .child(
                div()
                    .debug_selector(|| "approval-header".into())
                    .min_h(px(32.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(icon("square-lock", 14.).text_color(gpui::rgb(0xfbbf24)))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .py_1()
                            .text_color(content(0.75))
                            .child(approval.title.clone()),
                    )
                    .children(queued)
                    .child(actions),
            )
            .when(!approval.detail.is_empty(), |card| {
                card.child(
                    div()
                        .id("approval-detail")
                        .role(Role::Label)
                        .aria_label(approval.detail.clone())
                        .border_t_1()
                        .border_color(content(0.05))
                        .py_1()
                        .max_h(px(180.))
                        .overflow_y_scroll()
                        .font_family(crate::ui::fonts::mono())
                        .text_size(px(12.))
                        .line_height(px(18.))
                        .text_color(content(0.75))
                        .child(approval.detail.clone()),
                )
            });
        let card_width = (width.unwrap_or_default() - px(16.)).max(px(0.));
        composer_shelf::shelf("approval-shelf")
            .debug_selector(|| "approval-shelf".into())
            .child(self.approval_ui.motion.card(card, card_width, window))
            .into_any_element()
    }
}

impl Conversation {
    /// "Claude proposed a plan": the plan itself, then Reject, Revise… and
    /// Accept, the last keeping or choosing the access mode to carry on in.
    fn plan_shelf(
        &self,
        width: Option<gpui::Pixels>,
        ink: gpui::Hsla,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(approval) = self.approvals.front() else {
            return div().into_any_element();
        };
        let plan = approval.plan.clone().unwrap_or_default();
        let id = approval.id.clone();
        let access = self.settings.access;
        let hint = self.approval_ui.revise_hint.get();
        let decide = |id: String, decision: fn(&Self, &mut Context<Self>) -> Option<Decision>| {
            cx.listener(
                move |this: &mut Self,
                      _: &gpui::ClickEvent,
                      window: &mut Window,
                      cx: &mut Context<Self>| {
                    match decision(this, cx) {
                        Some(decision) => {
                            this.approval_ui.revise_hint.set(false);
                            this.decide_notification_request(&id, decision, cx);
                        }
                        // Revise with nothing written: ask for the change first.
                        None => {
                            this.approval_ui.revise_hint.set(true);
                            window.focus(&this.input.focus_handle(cx), cx);
                            cx.notify();
                        }
                    }
                },
            )
        };
        let actions = div()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(6.))
            .child(
                approval_button("plan-reject", "Reject")
                    .text_color(content(0.5))
                    .hover(|s| s.bg(content(0.08)).text_color(content(0.9)))
                    .on_click(decide(id.clone(), |_, _| {
                        Some(Decision::RevisePlan(String::new()))
                    })),
            )
            .child(
                approval_button("plan-revise", "Revise…")
                    .bg(content(0.06))
                    .hover(|s| s.bg(content(0.1)))
                    .on_click(decide(id.clone(), |this, cx| {
                        let text = this.input.read(cx).content.trim().to_owned();
                        (!text.is_empty()).then_some(Decision::RevisePlan(text))
                    })),
            )
            .when(
                !matches!(access, AccessMode::AutoAcceptEdits | AccessMode::FullAccess),
                |d| {
                    d.child(
                        approval_button("plan-accept-edits", "Accept and allow edits")
                            .bg(content(0.06))
                            .hover(|s| s.bg(content(0.1)))
                            .on_click(decide(id.clone(), |_, _| {
                                Some(Decision::AcceptPlan(AccessMode::AutoAcceptEdits))
                            })),
                    )
                },
            )
            .child(
                approval_button("plan-accept", "Accept")
                    .child(key_hint(shortcut("Enter")))
                    .bg(content(1.))
                    .text_color(ink)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .hover(move |s| s.bg(content(0.85)).text_color(ink))
                    .on_click(decide(id, |this, _| {
                        Some(Decision::AcceptPlan(this.settings.access))
                    })),
            );
        let card = composer_shelf::card("plan-request", self.classic_layout)
            .role(Role::Group)
            .aria_label(approval.title.clone())
            .child(
                div()
                    .debug_selector(|| "plan-header".into())
                    .min_h(px(32.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(icon("list-bullet", 14.).text_color(content(0.6)))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .py_1()
                            .text_color(content(0.85))
                            .child(approval.title.clone()),
                    ),
            )
            .child(
                div()
                    .id("plan-text")
                    .debug_selector(|| "plan-text".into())
                    .border_t_1()
                    .border_color(content(0.05))
                    .py_2()
                    .max_h(px(320.))
                    .overflow_y_scroll()
                    .text_size(px(13.))
                    .line_height(px(20.))
                    .text_color(content(0.8))
                    .child(if plan.trim().is_empty() {
                        "Claude didn't include the plan's text.".to_owned()
                    } else {
                        plan
                    }),
            )
            .when(hint, |card| {
                card.child(
                    div()
                        .pb_1()
                        .text_size(px(12.))
                        .text_color(gpui::rgb(0xfbbf24))
                        .child("Write what should change in the message box, then choose Revise… again."),
                )
            })
            .child(div().py_1().child(actions));
        let card_width = (width.unwrap_or_default() - px(16.)).max(px(0.));
        composer_shelf::shelf("approval-shelf")
            .debug_selector(|| "approval-shelf".into())
            .child(self.approval_ui.motion.card(card, card_width, window))
            .into_any_element()
    }
    /// Esc denies the request on show; Ctrl+Enter (Cmd on macOS) allows it
    /// once, or accepts a plan; Ctrl+Shift+Enter allows it for the session.
    /// Returns whether the key answered a request.
    pub(super) fn approval_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(approval) = self.approvals.front() else {
            return false;
        };
        let keys = &event.keystroke;
        let command = keys.modifiers.secondary();
        let decision = match (keys.key.as_str(), command, keys.modifiers.shift) {
            ("enter", true, false) if approval.plan.is_some() => {
                Decision::AcceptPlan(self.settings.access)
            }
            _ if approval.plan.is_some() => return false,
            ("escape", false, false) => Decision::Deny,
            ("enter", true, true) if approval.can_remember => Decision::Session,
            ("enter", true, false) => Decision::Allow,
            _ => return false,
        };
        let id = approval.id.clone();
        let decided = self.decide_notification_request(&id, decision, cx);
        if decided {
            cx.stop_propagation();
        }
        decided
    }
}

impl Conversation {
    /// Ctrl+Enter (Cmd on macOS): allows the waiting request once, or
    /// accepts the waiting plan. Returns whether one was waiting.
    pub(super) fn approve_front(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(approval) = self.approvals.front() else {
            return false;
        };
        let decision = if approval.plan.is_some() {
            Decision::AcceptPlan(self.settings.access)
        } else {
            Decision::Allow
        };
        let id = approval.id.clone();
        self.decide_notification_request(&id, decision, cx)
    }
    /// ↑ in an empty message box brings back the messages sent earlier,
    /// newest first; ↓ goes forward again and past the newest clears it.
    /// Returns whether the key was used.
    pub(super) fn recall(&mut self, up: bool, window: &Window, cx: &mut Context<Self>) -> bool {
        if !self.input.focus_handle(cx).is_focused(window) || (!up && self.recall.is_none()) {
            return false;
        }
        let sent = sent_messages(self.transcript.read(cx).source_rows());
        let content = self.input.read(cx).content.to_string();
        let showing = self.recall.and_then(|at| sent.get(at));
        // Only an empty box, or one still showing a recalled message, moves.
        if !(content.is_empty() || showing == Some(&content)) {
            self.recall = None;
            return false;
        }
        let next = match (self.recall, up) {
            (None, true) => Some(0),
            (Some(at), true) => Some((at + 1).min(sent.len().saturating_sub(1))),
            (Some(0), false) | (None, false) => None,
            (Some(at), false) => Some(at - 1),
        }
        .filter(|&at| at < sent.len());
        if next.is_none() && self.recall.is_none() {
            return false;
        }
        self.recall = next;
        let text = next.map(|at| sent[at].clone()).unwrap_or_default();
        self.input.update(cx, |input, cx| input.set_text(text, cx));
        cx.notify();
        true
    }
}

/// The messages the user sent, newest first.
fn sent_messages(rows: &[monocode_core::session::transcript::Row]) -> Vec<String> {
    let mut sent = Vec::new();
    for (at, row) in rows.iter().enumerate() {
        if row.role == "user" && row.first {
            let end = (at + 1..rows.len())
                .find(|&next| rows[next].first)
                .unwrap_or(rows.len());
            let text: String = rows[at..end].iter().map(|row| row.text.as_str()).collect();
            if !text.trim().is_empty() {
                sent.push(text);
            }
        }
    }
    sent.reverse();
    sent.dedup();
    sent
}

/// The key that does what a button does, faint beside its label.
fn key_hint(keys: String) -> impl IntoElement {
    div()
        .ml(px(6.))
        .text_size(px(10.))
        .opacity(0.55)
        .child(keys)
}

/// A shortcut as this platform writes it: Ctrl+Enter, or Cmd+Enter on macOS.
fn shortcut(keys: &str) -> String {
    format!(
        "{}+{keys}",
        if cfg!(target_os = "macos") {
            "Cmd"
        } else {
            "Ctrl"
        }
    )
}

fn approval_button(id: &'static str, label: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(Role::Button)
        .aria_label(label)
        .cursor_pointer()
        .flex_shrink_0()
        .h(px(26.))
        .px(px(10.))
        .rounded_md()
        .flex()
        .items_center()
        .text_size(px(12.))
        .child(label)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext, WindowHandle};
    use monocode_core::store::Store;
    use std::{sync::Mutex, time::Duration};

    fn conversation(cx: &mut TestAppContext) -> WindowHandle<Conversation> {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let summary = store.create("codex", "/tmp").unwrap();
        cx.update(|cx| {
            input::bind(cx);
            cx.set_global(crate::ui::preferences::Preferences {
                composer_mascot: false,
                follow_up: "queue".into(),
                ..Default::default()
            });
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| {
                    let mut view = Conversation::new(summary, Arc::new(Mutex::new(store)), cx);
                    view.loaded = true;
                    view.summary.has_messages = true;
                    view.busy = true;
                    view.follow_ups.state.paused = true;
                    view
                })
            })
            .unwrap()
        })
    }

    fn request(id: &str, can_remember: bool) -> Approval {
        Approval {
            id: id.into(),
            title: "Run a command?".into(),
            detail: "git status --short".into(),
            can_remember,
            plan: None,
        }
    }

    #[test]
    fn recall_offers_sent_messages_newest_first_in_one_piece() {
        use monocode_core::session::transcript::Row;
        let row = |role: &str, text: &str, first: bool| Row {
            role: role.into(),
            text: text.into(),
            first,
            sent_at: None,
        };
        let rows = [
            row("user", "Fix the ", true),
            row("user", "build", false),
            row("assistant", "Done", true),
            row("user", "Again", true),
            row("user", "Again", true),
            row("user", "Ship it", true),
        ];
        assert_eq!(sent_messages(&rows), ["Ship it", "Again", "Fix the build"]);
    }

    #[gpui::test]
    fn up_and_down_walk_through_sent_messages_in_an_empty_composer(cx: &mut TestAppContext) {
        use monocode_core::session::transcript::Row;
        let window = conversation(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        window
            .update(visual, |view, window, cx| {
                let row = |role: &str, text: &str| Row {
                    role: role.into(),
                    text: text.into(),
                    first: true,
                    sent_at: None,
                };
                let rows = vec![
                    row("user", "First"),
                    row("assistant", "Ok"),
                    row("user", "Second"),
                ];
                view.transcript.update(cx, |t, cx| t.patch(0, rows, cx));
                window.focus(&view.input.focus_handle(cx), cx);
            })
            .unwrap();
        visual.refresh().unwrap();
        let content = |visual: &mut VisualTestContext| {
            window
                .read_with(visual, |view, cx| view.input.read(cx).content.to_string())
                .unwrap()
        };
        for (key, expected) in [
            ("up", "Second"),
            ("up", "First"),
            ("up", "First"),
            ("down", "Second"),
            ("down", ""),
        ] {
            visual.simulate_keystrokes(key);
            assert_eq!(content(visual), expected, "after {key}");
        }
        // A draft of the user's own is never replaced.
        window
            .update(visual, |view, _, cx| {
                view.input.update(cx, |i, cx| i.set_text("Mine".into(), cx))
            })
            .unwrap();
        visual.simulate_keystrokes("up");
        assert_eq!(content(visual), "Mine");
    }

    #[gpui::test]
    fn a_plan_waits_for_accept_or_revise_and_keys_answer_requests(cx: &mut TestAppContext) {
        let window = conversation(cx);
        cx.update(|cx| cx.set_reduce_motion(true));
        let (send, receive) = std::sync::mpsc::channel();
        window
            .update(cx, |view, _, cx| {
                view.controls = Some(send);
                view.approvals.push_back(Approval {
                    id: "plan".into(),
                    title: "Claude proposed a plan".into(),
                    detail: String::new(),
                    can_remember: false,
                    plan: Some("1. Read\n2. Fix".into()),
                });
                cx.notify();
            })
            .unwrap();
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.refresh().unwrap();
        assert!(visual.debug_bounds("plan-text").is_some());
        assert!(visual.debug_bounds("approval-allow").is_none());
        // Revise with an empty message box asks for the change first.
        let revise = visual.debug_bounds("plan-revise").unwrap();
        visual.simulate_click(revise.center(), Default::default());
        visual.refresh().unwrap();
        assert!(receive.try_recv().is_err());
        window
            .update(visual, |view, _, cx| {
                view.input
                    .update(cx, |input, cx| input.set_text("Add tests".into(), cx))
            })
            .unwrap();
        let revise = visual.debug_bounds("plan-revise").unwrap();
        visual.simulate_click(revise.center(), Default::default());
        visual.refresh().unwrap();
        let control = receive.try_recv().unwrap();
        assert!(matches!(control.decision, Decision::RevisePlan(ref text) if text == "Add tests"));
        window
            .read_with(visual, |view, cx| {
                assert!(view.approvals.is_empty());
                assert!(view.input.read(cx).content.is_empty());
            })
            .unwrap();
        // Esc denies an ordinary request; Ctrl+Enter allows one.
        window
            .update(visual, |view, _, cx| {
                view.approvals
                    .extend([request("esc", true), request("enter", true)]);
                cx.notify();
            })
            .unwrap();
        visual.refresh().unwrap();
        visual.simulate_keystrokes("escape");
        let control = receive.try_recv().unwrap();
        assert_eq!(control.id, "esc");
        assert!(matches!(control.decision, Decision::Deny));
        visual.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-enter"
        } else {
            "ctrl-enter"
        });
        let control = receive.try_recv().unwrap();
        assert_eq!(control.id, "enter");
        assert!(matches!(control.decision, Decision::Allow));
    }

    #[gpui::test]
    fn approval_shelf_slides_up_without_moving_the_composer_or_replaying_on_redraw(
        cx: &mut TestAppContext,
    ) {
        let window = conversation(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.refresh().unwrap();
        let input = visual.debug_bounds("composer-box").unwrap();
        for queued in [false, true] {
            window
                .update(visual, |view, _, cx| {
                    view.approvals.clear();
                    if queued {
                        view.enqueue_follow_up("A queued follow-up".into(), None, cx);
                        view.follow_ups.motion.elapsed(Duration::from_secs(1));
                    }
                    cx.notify();
                })
                .unwrap();
            visual.refresh().unwrap();
            window
                .update(visual, |view, _, cx| {
                    view.approvals
                        .push_back(request(if queued { "second" } else { "first" }, true));
                    cx.notify();
                })
                .unwrap();
            visual.refresh().unwrap();
            let start = visual.debug_bounds("approval-shelf").unwrap();
            assert!((start.top() - input.top()).abs() < px(1.));
            assert_eq!(visual.debug_bounds("composer-box").unwrap(), input);

            window
                .update(visual, |view, _, cx| {
                    view.approval_ui.motion.elapsed(Duration::from_millis(40));
                    cx.notify();
                })
                .unwrap();
            visual.refresh().unwrap();
            let middle = visual.debug_bounds("approval-shelf").unwrap();
            assert!(middle.top() < start.top() - px(5.));
            assert_eq!(visual.debug_bounds("composer-box").unwrap(), input);

            window
                .update(visual, |view, _, cx| {
                    view.approval_ui.motion.elapsed(Duration::from_secs(1));
                    cx.notify();
                })
                .unwrap();
            visual.refresh().unwrap();
            let settled = visual.debug_bounds("approval-shelf").unwrap();
            assert!(settled.top() < middle.top() - px(5.));
            assert_eq!(settled.bottom(), input.top());
            assert_eq!(visual.debug_bounds("composer-box").unwrap(), input);
            window.update(visual, |_, _, cx| cx.notify()).unwrap();
            visual.refresh().unwrap();
            assert_eq!(visual.debug_bounds("approval-shelf").unwrap(), settled);
        }
        visual.update(|_, cx| cx.set_reduce_motion(true));
        window
            .update(visual, |view, _, cx| {
                view.approvals.clear();
                view.approvals.push_back(request("reduced-motion", true));
                cx.notify();
            })
            .unwrap();
        visual.refresh().unwrap();
        assert!(visual.debug_bounds("approval-shelf").unwrap().size.height > px(60.));
        assert_eq!(visual.debug_bounds("composer-box").unwrap(), input);
    }

    #[gpui::test]
    fn approval_shelf_buttons_resolve_each_request_with_the_selected_decision(
        cx: &mut TestAppContext,
    ) {
        let window = conversation(cx);
        cx.update(|cx| cx.set_reduce_motion(true));
        let (send, receive) = std::sync::mpsc::channel();
        window
            .update(cx, |view, _, cx| {
                view.controls = Some(send);
                view.approvals.extend([
                    request("once", true),
                    request("session", true),
                    request("denied", false),
                ]);
                cx.notify();
            })
            .unwrap();
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.refresh().unwrap();
        let header = visual.debug_bounds("approval-header").unwrap();
        for button in ["approval-deny", "approval-session", "approval-allow"] {
            let bounds = visual.debug_bounds(button).unwrap();
            assert!(
                bounds.top() >= header.top() && bounds.bottom() <= header.bottom(),
                "{button} shares the title row"
            );
        }
        for (button, id) in [
            ("approval-allow", "once"),
            ("approval-session", "session"),
            ("approval-deny", "denied"),
        ] {
            if id == "denied" {
                assert!(visual.debug_bounds("approval-session").is_none());
            }
            let bounds = visual.debug_bounds(button).unwrap();
            visual.simulate_click(bounds.center(), Default::default());
            visual.refresh().unwrap();
            let control = receive.try_recv().unwrap();
            assert_eq!(control.id, id);
            assert!(matches!(
                (id, control.decision),
                ("once", Decision::Allow)
                    | ("session", Decision::Session)
                    | ("denied", Decision::Deny)
            ));
            assert!(receive.try_recv().is_err());
        }
        assert!(visual.debug_bounds("approval-shelf").is_none());
        window
            .read_with(visual, |view, _| assert!(view.approvals.is_empty()))
            .unwrap();
    }
}
