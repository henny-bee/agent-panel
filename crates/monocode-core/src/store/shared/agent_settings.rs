use super::*;

impl Store {
    pub(in crate::store) fn shared_agent(&self, id: i64) -> Result<Option<Settings>, String> {
        let fields = ["harness", "model", "model_settings", "runtime_mode"]
            .into_iter()
            .map(str::to_owned)
            .chain(
                ["worktree_cwd", "worktree_removed", "provider_account_id"]
                    .into_iter()
                    .map(|name| self.shared_field(name, "NULL")),
            )
            .chain([
                "cwd".to_owned(),
                "provider_session_id".to_owned(),
                self.shared_field(
                    "has_user_message",
                    "CASE WHEN blocks_json!='[]' THEN 1 ELSE 0 END",
                ),
            ])
            .collect::<Vec<_>>()
            .join(",");
        let raw = sql(self
            .conn
            .query_row(
                &format!("SELECT {fields} FROM sessions WHERE id=?1"),
                [self.persisted_session_id(id)?],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<bool>>(5)?,
                        r.get::<_, Option<String>>(6)?,
                        r.get::<_, String>(7)?,
                        r.get::<_, Option<String>>(8)?,
                        r.get::<_, bool>(9)?,
                    ))
                },
            )
            .optional())?;
        let Some((
            provider,
            model,
            controls,
            access,
            cwd,
            removed,
            account,
            project,
            provider_id,
            has_messages,
        )) = raw
        else {
            return Ok(None);
        };
        let mut controls: HashMap<String, String> = parse(&controls)?;
        let mut settings: Settings = self
            .shared_setting(&format!("agent:{id}"))?
            .map(|raw| parse(&raw))
            .transpose()?
            .unwrap_or_default();
        settings.model = super::super::monocode_import::native_model(&model, &provider);
        // A prior native launch may have created an untouched default blank.
        // Seed only that blank, never a saved or resumed conversation's model.
        let default_controls = controls.iter().all(|(key, value)| {
            (key == "fast" && value == "false") || (key == "serviceTier" && value == "default")
        });
        if settings.model.is_none()
            && default_controls
            && settings.revision == 0
            && !has_messages
            && provider_id.is_none()
        {
            if let Some(defaults) = self
                .desktop_preferences()
                .and_then(|preferences| preferences.session_defaults(&provider, &project))
            {
                settings.model = defaults.model;
                controls = defaults.model_settings;
            }
        }
        settings.access = crate::session::agent::AccessMode::from_id(&access);
        settings.effort = [
            "effort",
            "reasoning",
            "reasoningEffort",
            "reasoning_effort",
            "variant",
        ]
        .into_iter()
        .find_map(|key| controls.get(key).cloned());
        settings.fast = controls.get("fast").is_some_and(|v| v == "true");
        settings.service_tier = controls.get("serviceTier").cloned();
        settings.thinking = controls.get("thinking").and_then(|s| s.parse().ok());
        settings.model_settings = controls;
        settings.worktree_cwd = cwd;
        settings.worktree_removed = removed.unwrap_or(false);
        settings.account.id = account.unwrap_or_else(|| crate::session::profile::DEFAULT.into());
        // Desktop blocks are authoritative. Native-only legacy metadata remains
        // available, but cannot override a saved desktop turn's identity/time.
        let blocks = self.shared_blocks(id)?;
        settings.operator_enabled |= blocks.iter().any(|block| {
            block["role"] == "user"
                && block["draft"] != true
                && block["internal"] != true
                && (block["monocode"] == true
                    || ["mono", "monocode"].into_iter().any(|command| {
                        crate::session::composer::consume_command(
                            block["text"].as_str().unwrap_or_default(),
                            command,
                        )
                        .is_some()
                    }))
        });
        for (turn, source) in project_turns(&blocks)? {
            let info = settings.turns.entry(turn).or_default();
            if !source.provider.is_empty() {
                info.provider = source.provider;
            }
            if !source.model.is_empty() {
                info.model = source.model;
            }
            if source.model_id.is_some() {
                info.model_id = source.model_id;
            }
            if source.provider_turn_id.is_some() {
                info.provider_turn_id = source.provider_turn_id;
            }
            if source.joined_turn.is_some() {
                info.joined_turn = source.joined_turn;
            }
            if source.duration_known {
                info.duration_ms = source.duration_ms;
                info.duration_known = true;
            }
            if source.started_at.is_some() {
                info.started_at = source.started_at;
                info.completed_at = source.completed_at;
            }
            if source.metrics.is_some() {
                info.metrics = source.metrics;
            }
        }
        Ok(Some(settings))
    }

    pub(in crate::store) fn shared_save_agent(
        &self,
        id: i64,
        settings: &Settings,
    ) -> Result<(), String> {
        self.shared_save_turns(id, &settings.turns)?;
        self.shared_save_agent_preferences(id, settings)
    }

    /// Preference-only updates for validated operations which already preserve
    /// their source transcript inside a surrounding transaction (rewind and
    /// worktree cleanup). These must not re-save or re-index turn receipts.
    pub(super) fn shared_save_agent_preferences(
        &self,
        id: i64,
        settings: &Settings,
    ) -> Result<(), String> {
        let current = self.shared_agent(id)?.unwrap_or_default();
        let mut controls = current.model_settings;
        controls.extend(settings.model_settings.clone());
        let effort_key = [
            "effort",
            "reasoning",
            "reasoningEffort",
            "reasoning_effort",
            "variant",
        ]
        .into_iter()
        .find(|key| controls.contains_key(*key))
        .unwrap_or("effort");
        if let Some(effort) = &settings.effort {
            controls.insert(effort_key.into(), effort.clone());
        } else {
            controls.remove(effort_key);
        }
        controls.insert("fast".into(), settings.fast.to_string());
        if let Some(tier) = &settings.service_tier {
            controls.insert("serviceTier".into(), tier.clone());
        }
        if let Some(thinking) = settings.thinking {
            controls.insert("thinking".into(), thinking.to_string());
        }
        let mut fields = vec![
            ("model_settings", json!(encode(&controls)?)),
            ("runtime_mode", json!(settings.access.id())),
            ("worktree_cwd", json!(settings.worktree_cwd)),
            ("worktree_removed", json!(settings.worktree_removed)),
            ("provider_account_id", json!(settings.account.id)),
        ];
        let (provider, saved_model): (String, String) = sql(self.conn.query_row(
            "SELECT harness,model FROM sessions WHERE id=?1",
            [self.persisted_session_id(id)?],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ))?;
        if settings.model != super::super::monocode_import::native_model(&saved_model, &provider) {
            let model = settings.model.as_deref().unwrap_or("default");
            let model = if provider == "claude" {
                model.strip_prefix("claude-").unwrap_or(model)
            } else {
                model
            };
            fields.push(("model", json!(format!("{provider}:{model}"))));
        }
        self.shared_patch(id, &fields)?;
        self.shared_save_setting(&format!("agent:{id}"), &encode(settings)?)
    }

    pub(in crate::store) fn shared_context(&self, id: i64) -> Result<Option<Usage>, String> {
        let fields = format!(
            "{},{}",
            self.shared_field("context_used", "NULL"),
            self.shared_field("context_window", "NULL")
        );
        let (used, window) = sql(self.conn.query_row(
            &format!("SELECT {fields} FROM sessions WHERE id=?1"),
            [self.persisted_session_id(id)?],
            |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<i64>>(1)?)),
        ))?;
        Ok(used.filter(|u| *u >= 0).map(|used| Usage {
            used: used as u64,
            window: window.filter(|w| *w > 0).map(|w| w as u64),
        }))
    }
}
