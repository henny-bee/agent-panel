//! The B (agents) and B2 (in this chat) views of a Claude session, shown in
//! place of the Explorer's file tree.
use crate::claude_activity::{
    self as activity, Agent, AgentStatus, FILE_KINDS, FileKind, Snapshot, TodoStatus,
};
use crate::conversation::Conversation;
use crate::right_panel::{AMBER_400, RED_400, stroke, tabular};
use crate::theme::*;
use gpui::{
    AnyElement, Context, Entity, EventEmitter, FontWeight, FutureExt, Hsla, MouseButton, Role,
    ScrollHandle, SharedString, Subscription, Task, WeakEntity, Window, div, prelude::*, px,
    relative, rgb,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ClaudeTab {
    Agents,
    Chat,
}

/// Claude's own orange, for the numbers worth reading first.
const CLAUDE: u32 = 0xe3876a;
const GREEN: u32 = 0x34d399;
/// One color per agent, in the order they started.
const AGENT_COLORS: [u32; 8] = [
    0x60a5fa, 0xf472b6, 0xfb923c, 0xf87171, 0x4ade80, 0xa78bfa, 0x2dd4bf, 0xfacc15,
];

mod agents;
mod chat;
mod poll;
mod rows;

pub(crate) use poll::ClaudeActivity;
use rows::note;

pub(crate) enum ClaudePanelEvent {
    Hide,
    Open(String),
}

/// B or B2 for one Explorer: which view, and what is expanded in it.
pub(crate) struct ClaudePanel {
    pub(crate) activity: Entity<ClaudeActivity>,
    pub(crate) tab: ClaudeTab,
    /// Expanded agent cards and file lists, and collapsed agent groups.
    open: HashSet<String>,
    scroll: ScrollHandle,
    _observe: Subscription,
}
impl EventEmitter<ClaudePanelEvent> for ClaudePanel {}

impl ClaudePanel {
    pub(crate) fn new(
        activity: Entity<ClaudeActivity>,
        tab: ClaudeTab,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            _observe: cx.observe(&activity, |_, _, cx| cx.notify()),
            activity,
            tab,
            open: HashSet::new(),
            scroll: ScrollHandle::new(),
        }
    }

    fn toggle(&mut self, key: String, cx: &mut Context<Self>) {
        if !self.open.remove(&key) {
            self.open.insert(key);
        }
        cx.notify();
    }
}

impl Render for ClaudePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let activity = self.activity.read(cx);
        let snapshot = activity.snapshot.clone();
        let cwd = activity.cwd.clone();
        let body = if !snapshot.loaded {
            vec![note("Reading this chat…")]
        } else {
            match self.tab {
                ClaudeTab::Agents => self.agents(&snapshot, cx),
                ClaudeTab::Chat => self.chat(&snapshot, &cwd, cx),
            }
        };
        div()
            .id("claude-panel")
            .role(Role::Group)
            .aria_label(match self.tab {
                ClaudeTab::Agents => "Agents in this chat",
                ClaudeTab::Chat => "In this chat",
            })
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .pb_3()
            // The file tree's context menu has nothing to act on here.
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .children(body)
    }
}
