use super::*;

impl Shell {
    pub(super) fn bind_linked_activity(&mut self, cx: &mut Context<Self>) {
        self.observations.push(cx.subscribe(
            &self.notification_state.github,
            |this, _, event, cx| {
                match event {
                    GithubActivityEvent::Acknowledged(seen) => {
                        let seen = seen.clone();
                        let store = this.store.clone();
                        let previous = this.notification_state.linked_seen_writes.take();
                        this.notification_state.linked_seen_writes =
                            Some(cx.spawn(async move |_, cx| {
                                if let Some(previous) = previous {
                                    previous.await;
                                }
                                cx.background_executor()
                                    .spawn(async move {
                                        let _ = locked(&store)
                                            .and_then(|s| github_activity::save_seen(&s, &seen));
                                    })
                                    .await;
                            }));
                    }
                    GithubActivityEvent::Changed => {
                        this.observe_linked_activity(cx);
                        this.sync_linked_activity(cx);
                    }
                }
                cx.notify();
            },
        ));
        let store = self.store.clone();
        let metadata = self.workspace.desktop_preferences_metadata.clone();
        let work = cx.background_executor().spawn(async move {
            locked(&store)
                .and_then(|s| github_activity::load_seen_with_metadata(&s, metadata.as_ref()))
        });
        cx.spawn(async move |this, cx| {
            let seen = work.await.unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.notification_state
                    .github
                    .update(cx, |feed, _| feed.restore_seen(seen));
                this.notification_state.linked_seen_ready = true;
                this.poll_linked_activity(cx);
            });
        })
        .detach();
    }

    pub(super) fn poll_linked_activity(&mut self, cx: &mut Context<Self>) {
        if self.notification_state.linked_seen_ready
            && self
                .notification_state
                .linked_last_poll
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(60))
            && self.notification_state.github.read(cx).ready_to_poll()
        {
            self.notification_state.linked_last_poll = Some(std::time::Instant::now());
            let mut sessions = self.session_summaries.clone();
            for view in self.notification_chats(cx) {
                let summary = view.read(cx).summary.clone();
                sessions.insert(summary.id, summary);
            }
            self.notification_state.github.update(cx, |feed, cx| {
                feed.poll(sessions.into_values().collect(), cx)
            });
        }
        let pending = self.notification_state.linked_cleanup.clone();
        for (id, action) in pending {
            if self.archiving.contains(&id) || self.deleting.contains(&id) {
                continue;
            }
            let success = match action {
                Cleanup::Archive => self.session_summaries.get(&id).is_some_and(|s| s.archived),
                Cleanup::Delete => !self.session_summaries.contains_key(&id),
            };
            self.notification_state.linked_cleanup.remove(&id);
            if let Some(notice) = self.notification_state.linked.get(&id).cloned() {
                notice.update(cx, |notice, cx| notice.finish_cleanup(success, cx));
            }
            if success {
                self.notification_state
                    .github
                    .update(cx, |feed, cx| feed.acknowledge(id, cx));
            }
        }
    }

    pub(super) fn linked_activity_allowed(&self, card: &github_activity::Card) -> bool {
        let project = format!(
            "repository:github.com/{}",
            card.item.repo.trim().to_lowercase()
        );
        let category = if card.item.kind == "pr" {
            "pullRequests"
        } else {
            "issues"
        };
        let occurred = u64::try_from(card.updated_at).ok();
        occurred.is_some()
            && self.workspace.preferences.allows_notification_project(
                &project,
                category,
                occurred,
                monocode_core::sys::time::now_ms(),
            )
    }

    pub(super) fn observe_linked_activity(&mut self, cx: &mut Context<Self>) {
        let cards: Vec<_> = self
            .notification_state
            .github
            .read(cx)
            .cards
            .values()
            .cloned()
            .collect();
        for card in cards {
            let key = format!(
                "{}:{}:{}:{}",
                card.session,
                card.item.repo.to_lowercase(),
                card.item.kind,
                card.item.number
            );
            let previous = self.notification_state.linked_revisions.get(&key).copied();
            self.notification_state
                .linked_revisions
                .insert(key, previous.unwrap_or(0).max(card.updated_at));
            // The first successful feed is a baseline, never a new-activity
            // announcement. Acknowledgment and local-turn cutoffs are applied
            // by the feed before a card enters this collection.
            let newly_changed = self.notification_state.linked_primed
                && previous.is_none_or(|at| card.updated_at > at);
            let occurred = u64::try_from(card.updated_at).unwrap_or(0);
            if newly_changed
                && !card.details_error
                && self.linked_activity_allowed(&card)
                && self.workspace.preferences.sounds
                && occurred >= self.notification_state.sounds_enabled_at
                && occurred >= self.notification_state.linked_started_at
            {
                // React uses a deduplicated chime, without an OS/banner here.
                crate::ui::sound::play(crate::ui::sound::Cue::LinkedActivity);
            }
        }
        self.notification_state.linked_primed = true;
    }

    pub(super) fn sync_linked_activity(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<_> = self.notification_state.linked.keys().copied().collect();
        for id in ids {
            let card = self
                .notification_state
                .github
                .read(cx)
                .cards
                .get(&id)
                .filter(|card| self.linked_activity_allowed(card))
                .cloned();
            let can_cleanup = self.session_summaries.get(&id).is_some_and(|s| !s.archived);
            if let Some(notice) = self.notification_state.linked.get(&id) {
                notice.update(cx, |notice, cx| {
                    notice.set_snapshot(card, can_cleanup, can_cleanup, cx)
                });
            }
        }
    }

    /// The pane host positions the returned card at top/right 12px, clipped to
    /// the exact session surface, so split chats and Mono sessions stay scoped.
    pub(crate) fn linked_notice(
        &mut self,
        id: i64,
        cx: &mut Context<Self>,
    ) -> Option<Entity<LinkedActivity>> {
        let card = self
            .notification_state
            .github
            .read(cx)
            .cards
            .get(&id)
            .filter(|card| self.linked_activity_allowed(card))
            .cloned()?;
        let can_cleanup = self.session_summaries.get(&id).is_some_and(|s| !s.archived);
        if let Some(notice) = self.notification_state.linked.get(&id) {
            notice.update(cx, |notice, cx| {
                notice.set_snapshot(Some(card), can_cleanup, can_cleanup, cx)
            });
            return Some(notice.clone());
        }
        let notice = cx.new(|cx| {
            let mut notice = LinkedActivity::new();
            notice.set_snapshot(Some(card), can_cleanup, can_cleanup, cx);
            notice
        });
        self.observations
            .push(cx.subscribe(&notice, |this, _, event, cx| {
                this.handle_linked_activity(event.clone(), cx)
            }));
        self.notification_state.linked.insert(id, notice.clone());
        Some(notice)
    }

    pub(super) fn handle_linked_activity(
        &mut self,
        event: LinkedActivityEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            LinkedActivityEvent::Acknowledge(id) | LinkedActivityEvent::Dismiss(id) => {
                self.notification_state
                    .github
                    .update(cx, |feed, cx| feed.acknowledge(id, cx));
            }
            LinkedActivityEvent::OpenUrl(url) => cx.open_url(&url),
            LinkedActivityEvent::OpenDiscussion(id) => {
                // A native threaded discussion pane is not implemented yet.
                if let Some(item) = self
                    .session_summaries
                    .get(&id)
                    .and_then(|s| s.work_item.as_ref())
                {
                    cx.open_url(&item.url);
                }
            }
            LinkedActivityEvent::AddToChat(id, prompt) => {
                cx.spawn(async move |this, cx| {
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.close_settings(window, cx);
                        let opened_mono = this.monos.clone().is_some_and(|monos| {
                            monos.update(cx, |m, cx| m.open_notification_session(id, window, cx))
                        });
                        if !opened_mono {
                            this.open(id, window, cx);
                        }
                        if let Some(view) = this.notification_chat(id, cx) {
                            let input = view.read(cx).input.clone();
                            if !input.read(cx).is_composing() {
                                input.update(cx, |input, cx| {
                                    let draft = input.content.to_string();
                                    let separator = if draft.is_empty() || draft.ends_with("\n\n") {
                                        ""
                                    } else if draft.ends_with('\n') {
                                        "\n"
                                    } else {
                                        "\n\n"
                                    };
                                    input.set_text(
                                        format!("{draft}{separator}{}\n\n", prompt.trim()),
                                        cx,
                                    );
                                });
                                window.focus(&input.focus_handle(cx), cx);
                            }
                        }
                        window.activate_window();
                        cx.notify();
                    });
                })
                .detach();
            }
            LinkedActivityEvent::Archive(id) => {
                self.toggle_archive(id, cx);
                if self.archiving.contains(&id) {
                    self.notification_state
                        .linked_cleanup
                        .insert(id, Cleanup::Archive);
                } else if let Some(notice) = self.notification_state.linked.get(&id) {
                    notice.update(cx, |notice, cx| notice.finish_cleanup(false, cx));
                }
            }
            LinkedActivityEvent::Delete(id) => {
                cx.spawn(async move |this, cx| {
                    let prompt = this.update_in(cx, |_, window, cx| {
                        window.prompt(
                            gpui::PromptLevel::Warning,
                            "Delete this conversation permanently?",
                            Some("Its messages and saved settings will be removed."),
                            &["Delete", "Cancel"],
                            cx,
                        )
                    });
                    let confirmed = match prompt {
                        Ok(prompt) => matches!(prompt.await, Ok(0)),
                        Err(_) => false,
                    };
                    let _ = this.update_in(cx, |this, window, cx| {
                        if confirmed {
                            this.delete_session(id, window, cx);
                        }
                        if this.deleting.contains(&id) {
                            this.notification_state
                                .linked_cleanup
                                .insert(id, Cleanup::Delete);
                        } else if let Some(notice) = this.notification_state.linked.get(&id) {
                            notice.update(cx, |notice, cx| notice.finish_cleanup(false, cx));
                        }
                    });
                })
                .detach();
            }
        }
        cx.notify();
    }
}
