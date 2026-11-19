//! Keeps one session's activity current: read again as soon as its files
//! change.
use super::*;

/// One session's activity, read again as soon as its files change.
pub(crate) struct ClaudeActivity {
    pub(crate) snapshot: Snapshot,
    pub(super) cwd: String,
    _poll: Task<()>,
}

impl ClaudeActivity {
    pub(crate) fn new(conversation: WeakEntity<Conversation>, cx: &mut Context<Self>) -> Self {
        let poll = cx.spawn(async move |this, cx| {
            let mut tracker = activity::Tracker::default();
            // The session's folder, and the signal its changes send.
            let mut folder: Option<PathBuf> = None;
            let mut changes = None;
            loop {
                let source = this.update(cx, |_, cx| {
                    conversation
                        .upgrade()
                        .and_then(|c| c.read(cx).claude_source())
                });
                let Ok(source) = source else {
                    break;
                };
                let mut running = false;
                if let Some((account, session, cwd, wide)) = source {
                    let (back, snapshot) = cx
                        .background_executor()
                        .spawn(async move {
                            tracker.refresh(&account, &session);
                            let now = monocode_core::sys::time::now_ms() as i64;
                            let snapshot = tracker.snapshot(&account, now, wide);
                            (tracker, snapshot)
                        })
                        .await;
                    tracker = back;
                    // A running agent's or background task's clock moves
                    // even when nothing is written.
                    running = !snapshot.background.is_empty()
                        || snapshot
                            .agents
                            .iter()
                            .any(|agent| agent.status == AgentStatus::Running);
                    let updated = this.update(cx, |this, cx| {
                        if running || this.snapshot != snapshot || this.cwd != cwd {
                            this.snapshot = snapshot;
                            this.cwd = cwd;
                            cx.notify();
                        }
                    });
                    if updated.is_err() {
                        break;
                    }
                    if let Some(found) = tracker.transcript().and_then(Path::parent)
                        && folder.as_deref() != Some(found)
                    {
                        let found = found.to_path_buf();
                        folder = Some(found.clone());
                        changes = cx
                            .background_executor()
                            .spawn(async move { crate::runtime::fs_watch::watch(&[found]) })
                            .await;
                    }
                }
                // A change wakes this at once. The timeout is the backup for a
                // missed change, and the tick of a running clock.
                let wait = Duration::from_secs(if running || changes.is_none() { 1 } else { 5 });
                if let Some((_, signal)) = &changes {
                    if let Ok(Err(_)) = signal
                        .recv()
                        .with_timeout(wait, cx.background_executor())
                        .await
                    {
                        // The watcher stopped: poll instead.
                        changes = None;
                    }
                } else {
                    cx.background_executor().timer(wait).await;
                }
            }
        });
        Self {
            snapshot: Snapshot::default(),
            cwd: String::new(),
            _poll: poll,
        }
    }

    /// A snapshot nothing reads again, for tests.
    #[cfg(test)]
    pub(super) fn fixed(snapshot: Snapshot) -> Self {
        Self {
            snapshot,
            cwd: String::new(),
            _poll: Task::ready(()),
        }
    }
}
