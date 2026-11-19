use super::*;

impl Protocol {
    pub(super) fn acp_options(&mut self, setup: &Value) -> Result<(), String> {
        let kind = self.request.kind;
        let settings = &self.request.settings;
        let planning = settings.plan || settings.access == AccessMode::ReadOnly;
        let mode = match kind {
            Kind::Antigravity => {
                if planning {
                    "default"
                } else {
                    match settings.access {
                        AccessMode::FullAccess => "yolo",
                        AccessMode::AutoAcceptEdits => "auto_edit",
                        _ => "default",
                    }
                }
            }
            Kind::Hermes => {
                if planning {
                    "default"
                } else {
                    match settings.access {
                        AccessMode::Supervised => "default",
                        AccessMode::AutoAcceptEdits => "accept_edits",
                        _ => "dont_ask",
                    }
                }
            }
            Kind::Devin => {
                if planning {
                    "plan"
                } else {
                    match settings.access {
                        AccessMode::FullAccess => "bypass",
                        AccessMode::Auto => "smart",
                        _ => "accept-edits",
                    }
                }
            }
            Kind::Fx => {
                if planning {
                    "ask"
                } else {
                    "code"
                }
            }
            Kind::OpenCode => {
                if planning {
                    "plan"
                } else {
                    "build"
                }
            }
            // Cursor's Tauri adapter enforces planning in request_permission;
            // it does not require or change the server's session mode.
            Kind::Cursor => "",
            _ => "",
        };
        let modes = setup["modes"]["availableModes"].as_array();
        if !mode.is_empty()
            && (modes.is_some_and(|modes| modes.iter().any(|m| m["id"] == mode))
                || matches!(
                    kind,
                    Kind::Antigravity | Kind::Hermes | Kind::Devin | Kind::Fx
                ))
        {
            self.acp.mode = Some(mode);
        }
        let config = setup["configOptions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let selected_model = self.request.model.as_ref().map(|model| {
            if kind == Kind::Devin {
                let (model, spent) = super::super::devin::model_value(setup, model, settings);
                self.acp.spent_effort = spent;
                model
            } else {
                model.clone()
            }
        });
        if let Some(model) = selected_model
            .as_ref()
            .filter(|model| {
                !matches!(kind, Kind::Hermes | Kind::Devin) || model.as_str() != "default"
            })
            .filter(|model| {
                !matches!(kind, Kind::Hermes | Kind::Grok)
                    || setup["models"]["currentModelId"].as_str() != Some(model.as_str())
            })
        {
            if let Some(option) = config.iter().find(|o| config_id(o) == "model").or_else(|| {
                config
                    .iter()
                    .find(|o| o["category"] == "model" && config_id(o) != "provider")
            }) {
                let mut choices = Vec::new();
                config_choices(&option["options"], &mut choices);
                if kind != Kind::Devin
                    || choices.is_empty()
                    || choices.iter().any(|choice| choice == model)
                {
                    self.acp.options.push_back((
                        "session/set_config_option",
                        json!({"sessionId":self.acp.session,"configId":config_id(option),"value":model}),
                    ));
                } else if self.acp.stale_model.as_ref() != Some(model) {
                    self.acp.stale_model = Some(model.clone());
                    self.acp.notices.push(Event::Status(format!("Devin no longer offers the model \"{model}\"; using its current model. Pick another model to change it.")));
                }
                if kind == Kind::Fx && config_id(option) != "model" {
                    self.acp.options.push_back((
                        "session/set_config_option",
                        json!({"sessionId":self.acp.session,"configId":"model","value":model}),
                    ));
                }
            } else if matches!(kind, Kind::Fx | Kind::Cursor) {
                self.acp.options.push_back((
                    "session/set_config_option",
                    json!({"sessionId":self.acp.session,"configId":"model","value":model}),
                ));
            } else if matches!(kind, Kind::Antigravity | Kind::Devin) {
                // These servers only accept config options they advertised.
            } else {
                self.acp.options.push_back((
                    "session/set_model",
                    json!({"sessionId":self.acp.session,"modelId":model}),
                ));
            }
        }
        Ok(())
    }

    pub(super) fn acp_settings(&mut self, setup: &Value) {
        let kind = self.request.kind;
        let settings = &self.request.settings;
        let mut selected = settings.model_settings.clone();
        if let Some(effort) = &settings.effort {
            selected
                .entry("effort".into())
                .or_insert_with(|| effort.clone());
        }
        selected
            .entry("fast".into())
            .or_insert_with(|| settings.fast.to_string());
        if let Some(thinking) = settings.thinking {
            selected
                .entry("thinking".into())
                .or_insert_with(|| thinking.to_string());
        }
        if kind == Kind::Hermes {
            selected.retain(|id, _| id == "effort");
        }
        if kind == Kind::Grok {
            if let Some(effort) = selected
                .get("effort")
                .or_else(|| selected.get("reasoning"))
                .filter(|effort| !effort.trim().is_empty())
            {
                self.acp.options.push_back((
                    "session/set_mode",
                    json!({"sessionId":self.acp.session,"modeId":effort}),
                ));
            }
            return;
        }
        let config = setup["configOptions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut resolved = std::collections::HashSet::new();
        for (id, mut value) in selected {
            if kind == Kind::Devin && id == "fast" {
                continue;
            }
            if kind == Kind::Devin
                && self.acp.spent_effort
                && matches!(id.as_str(), "effort" | "reasoning")
            {
                continue;
            }
            let Some(option) = setting_option(&config, &id) else {
                continue;
            };
            let config_id = config_id(option);
            if matches!(config_id, "provider" | "model" | "mode") || !resolved.insert(config_id) {
                continue;
            }
            if kind == Kind::Devin {
                let mut choices = Vec::new();
                config_choices(&option["options"], &mut choices);
                if !choices.is_empty() && !choices.contains(&value) {
                    let Some(nearest) = (if matches!(id.as_str(), "effort" | "reasoning") {
                        nearest_effort(&value, &choices)
                    } else {
                        None
                    }) else {
                        continue;
                    };
                    value = nearest;
                }
            }
            let mut params =
                json!({"sessionId":self.acp.session,"configId":config_id,"value":value});
            if kind == Kind::Antigravity && option["type"] == "boolean" {
                if !matches!(value.as_str(), "true" | "false") {
                    continue;
                }
                params["type"] = "boolean".into();
                params["value"] = (value == "true").into();
            }
            self.acp
                .options
                .push_back(("session/set_config_option", params));
        }
    }
}

pub(super) fn config_id(option: &Value) -> &str {
    option["id"]
        .as_str()
        .or_else(|| option["configId"].as_str())
        .unwrap_or("")
}

pub(super) fn setting_option<'a>(options: &'a [Value], setting: &str) -> Option<&'a Value> {
    let setting = setting.trim().to_lowercase();
    options
        .iter()
        .find(|option| config_id(option).eq_ignore_ascii_case(&setting))
        .or_else(|| {
            options.iter().find(|option| {
                let id = config_id(option);
                match setting.as_str() {
                    "effort" | "reasoning" => {
                        matches!(
                            id,
                            "effort"
                                | "reasoning"
                                | "reasoning_effort"
                                | "thought_level"
                                | "thinking_level"
                        ) || (option["category"] == "thought_level" && id != "thinking")
                    }
                    "fast" | "fastmode" => id.to_lowercase().contains("fast"),
                    "context" | "contextwindow" => matches!(id, "context" | "context_size"),
                    _ => false,
                }
            })
        })
}

pub(super) fn config_choices(options: &Value, choices: &mut Vec<String>) {
    if let Some(options) = options.as_array() {
        for option in options {
            if let Some(value) = option["value"].as_str() {
                choices.push(value.into());
            }
            config_choices(&option["options"], choices);
        }
    }
}

pub(super) fn nearest_effort(value: &str, choices: &[String]) -> Option<String> {
    let order = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];
    let requested = order.iter().position(|effort| *effort == value)?;
    order
        .iter()
        .enumerate()
        .filter(|(_, effort)| choices.iter().any(|choice| choice == **effort))
        .min_by_key(|(index, _)| index.abs_diff(requested))
        .map(|(_, effort)| (*effort).into())
}
