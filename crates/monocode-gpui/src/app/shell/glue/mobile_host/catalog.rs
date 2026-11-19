use super::*;

impl Shell {
    /// Projects on the rail and those Monos work in.
    pub(super) fn mobile_projects(&self, cx: &App) -> Vec<String> {
        let mut paths: Vec<String> = Vec::new();
        let monos = self
            .monos
            .as_ref()
            .map(|m| {
                m.read(cx)
                    .roster
                    .monos
                    .iter()
                    .flat_map(|mono| mono.projects.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for path in self.workspace.projects.iter().cloned().chain(monos) {
            if !path.is_empty()
                && path != "~"
                && !paths.iter().any(|p| {
                    monocode_core::files::path_key(p) == monocode_core::files::path_key(&path)
                })
            {
                paths.push(path);
            }
        }
        paths
    }

    pub(super) fn mobile_project_path(&self, id: &str, cx: &App) -> Result<String, String> {
        self.mobile_projects(cx)
            .into_iter()
            .find(|path| project_id(path) == id)
            .ok_or_else(|| "Project is no longer available".into())
    }

    pub(super) fn mobile_catalog_sessions(&self, paths: &[String], cx: &App) -> Vec<CatalogEntry> {
        self.order
            .iter()
            .filter(|id| !self.sidebar_hidden_sessions.contains(id))
            .filter_map(|&id| {
                let metadata = self.session_metadata(id, cx);
                let summary = metadata.summary;
                if summary.archived
                    || !summary.has_messages
                    || self.orchestration_worker(id)
                    || self
                        .monos
                        .as_ref()
                        .is_some_and(|m| m.read(cx).hidden_session(id))
                {
                    return None;
                }
                let key = monocode_core::files::path_key(&summary.cwd);
                let project = paths
                    .iter()
                    .find(|path| monocode_core::files::path_key(path) == key)?;
                let live = self.sessions.get(&id).map(|view| {
                    let view = view.read(cx);
                    mobile_catalog::LiveSession {
                        loaded: view.history_loaded(),
                        metadata: monocode_core::store::MobileSessionMetadata {
                            model: view.mobile_model().map(str::to_owned),
                            branch: view.branch_name().map(str::to_owned),
                            worktree_cwd: view.settings.worktree_cwd.clone(),
                            worktree_removed: view.settings.worktree_removed,
                            access: view.settings.access,
                        },
                        busy: view.busy,
                        waiting: match view.mobile_waiting(cx) {
                            Some(crate::chat::conversation::MobileWaiting::Approval) => {
                                Some("approval")
                            }
                            Some(crate::chat::conversation::MobileWaiting::Question) => {
                                Some("question")
                            }
                            None => None,
                        },
                    }
                });
                Some(CatalogEntry {
                    id,
                    project: project_id(project),
                    summary: summary.clone(),
                    live,
                })
            })
            .collect()
    }

    pub(super) fn publish_mobile_catalog(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self.mobile.host.clone() else {
            return;
        };
        if self.mobile.publishing || !host.enabled() {
            return;
        }
        let paths = self.mobile_projects(cx);
        let projects: Vec<Value> = paths
            .iter()
            .map(|path| {
                let appearance = self.appearance(path);
                let (kind, model, access) = self.mobile_defaults(path, cx);
                let mut project = json!({
                    "id": project_id(path),
                    "path": path,
                    "name": appearance.label(path),
                    "machine": "Desktop",
                    "branch": "",
                    "color": hex(appearance.color(path)),
                    "additions": 0,
                    "deletions": 0,
                    "defaultHarness": kind.id(),
                    "defaultModelName": pickers::cached_model_name(kind, model.as_deref()).to_string(),
                    "defaultRuntimeMode": access.id(),
                });
                if let Some(model) = model {
                    project["defaultModel"] = json!(mobile_catalog::model_id(kind, Some(&model)));
                }
                if let Some(mascot) = appearance.mascot {
                    project["mascot"] = json!(mascot);
                }
                project
            })
            .collect();
        let entries = self.mobile_catalog_sessions(&paths, cx);
        let monos: Vec<(monocode_core::session::mono_model::Mono, Option<Value>)> = self
            .monos
            .as_ref()
            .map(|m| {
                let manager = m.read(cx);
                manager
                    .roster
                    .monos
                    .iter()
                    .map(|mono| {
                        let live = manager.open_chat(&mono.id).map(|chat| {
                            let chat = chat.read(cx);
                            let rows = chat.transcript.read(cx).source_rows();
                            let last = turns::messages(rows).pop();
                            json!({
                                "status": chat.mono_status(),
                                "busy": chat.busy,
                                "harness": chat.summary.provider,
                                "model": mobile_catalog::model_id(Kind::from_id(&chat.summary.provider), chat.mobile_model()),
                                "lastMessage": last.map(|m| m["text"].clone()).unwrap_or(json!("")),
                                "lastActiveAt": chat.summary.updated_at,
                            })
                        });
                        (mono.clone(), live)
                    })
                    .collect()
            })
            .unwrap_or_default();
        // A Mono's conversation not open here still shows its last message
        // on the phone, read from its saved history.
        let saved_monos: Vec<(i64, Option<u64>)> = monos
            .iter()
            .filter(|(_, live)| live.is_none())
            .filter_map(|(mono, _)| mono.session_id)
            .map(|id| (id, self.session_summaries.get(&id).map(|s| s.updated_at)))
            .collect();
        let mut previews = self.mobile.mono_previews.clone();
        let store = self.store.clone();
        let mut ids = self.mobile.ids.clone();
        self.mobile.publishing = true;
        let work = cx.background_executor().spawn(async move {
            let saved = {
                let store = locked(&store)?;
                for id in entries.iter().map(|entry| entry.id).chain(monos.iter().filter_map(|(mono, _)| mono.session_id)) {
                    if let std::collections::hash_map::Entry::Vacant(entry) = ids.entry(id)
                        && let Ok(value) = store.persisted_session_id(id)
                    {
                        entry.insert(value);
                    }
                }
                for (id, updated_at) in &saved_monos {
                    let fresh = previews
                        .get(id)
                        .is_some_and(|p| updated_at.is_some_and(|at| at == p.updated_at));
                    if fresh {
                        continue;
                    }
                    let Ok(history) = store.load(*id) else {
                        continue;
                    };
                    let text = turns::messages(&history.rows)
                        .pop()
                        .map(|m| m["text"].clone())
                        .unwrap_or(json!(""));
                    previews.insert(*id, MonoPreview { updated_at: updated_at.unwrap_or(0), text });
                }
                previews.retain(|id, _| saved_monos.iter().any(|(saved, _)| saved == id));
                store.mobile_session_metadata()?
            };
            let mut branches = HashMap::new();
            let projects: Vec<_> = projects.into_iter().map(|mut project| {
                let cwd = project["path"].as_str().unwrap_or_default().to_owned();
                project["branch"] = json!(mobile_catalog::checkout_branch(&cwd, &mut branches));
                project
            }).collect();
            let mut sessions = Vec::new();
            for entry in entries {
                let Some(id) = ids.get(&entry.id) else {
                    continue;
                };
                let metadata = mobile_catalog::metadata(saved.get(&entry.id).unwrap_or(&Default::default()), entry.live.as_ref());
                let branch = if !metadata.worktree_removed && metadata.branch.as_ref().is_none_or(|branch| branch.is_empty()) {
                    mobile_catalog::checkout_branch(metadata.worktree_cwd.as_deref().unwrap_or(&entry.summary.cwd), &mut branches)
                } else { String::new() };
                sessions.push(mobile_catalog::session(&entry.summary, id, &entry.project, &metadata, entry.live.as_ref(), &branch));
            }
            sessions.sort_by_key(|s| std::cmp::Reverse(s["updatedAt"].as_u64()));
            let monos: Vec<Value> = monos
                .into_iter()
                .map(|(mono, live)| {
                    let mut value = json!({
                        "id": mono.id,
                        "name": mono.label(),
                        "mascot": mono.mascot,
                        "color": hex(mono.color),
                        "projectIds": mono.projects.iter().map(|p| project_id(p)).collect::<Vec<_>>(),
                        "unread": 0,
                        "status": "idle",
                        "busy": false,
                        "live": live.is_some(),
                        "lastMessage": "",
                        "lastActiveAt": 0,
                    });
                    if let Some(session) = mono.session_id.and_then(|id| ids.get(&id)) {
                        value["sessionId"] = json!(session);
                    }
                    if let Some(subsessions) = &mono.subsessions {
                        value["subsessions"] = json!(subsessions);
                    }
                    if let Some(preview) = mono.session_id.and_then(|id| previews.get(&id)) {
                        value["lastMessage"] = preview.text.clone();
                        value["lastActiveAt"] = json!(preview.updated_at);
                    }
                    if let Some(live) = live {
                        for (key, field) in live.as_object().into_iter().flatten() {
                            if !field.is_null() {
                                value[key] = field.clone();
                            }
                        }
                    }
                    value
                })
                .collect();
            Ok::<_, String>((
                // `monoTools`: this desktop answers the phone's Mono profile,
                // editing, reset, delete and create; the older app doesn't,
                // and its phones hide them. `push`: it notifies phones through
                // Expo, so they no longer post their own alerts. `monoFiles`:
                // it reads and saves a Mono's soul and memory for the phone.
                json!({"projects": projects, "monos": monos, "sessions": sessions, "monoTools": true, "push": true, "monoFiles": true}),
                ids,
                previews,
            ))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, _| {
                this.mobile.publishing = false;
                if let Ok((catalog, ids, previews)) = result {
                    this.mobile.ids = ids;
                    this.mobile.mono_previews = previews;
                    let _ = host.publish(catalog);
                }
            });
        })
        .detach();
    }

    pub(super) fn mobile_target(&self, id: &str, cx: &App) -> Result<Target, String> {
        if let Some(mono) = id.strip_prefix("mono:") {
            return self
                .monos
                .as_ref()
                .is_some_and(|m| m.read(cx).roster.get(mono).is_some())
                .then(|| Target::Mono(mono.to_owned()))
                .ok_or_else(|| "Mono is no longer available".into());
        }
        let session = self
            .mobile
            .ids
            .iter()
            .find(|(_, persisted)| *persisted == id)
            .map(|(id, _)| *id)
            .or_else(|| locked(&self.store).ok()?.resolve_existing_session_id(id))
            .filter(|id| self.session_summaries.contains_key(id))
            .ok_or("Session is no longer available")?;
        Ok(Target::Session(session))
    }

    /// The sessions a Mono started, as the cards in its chat show them on the
    /// phone: the phone's IDs for each and the project it runs in, its title
    /// and how it stands. None for a session's own conversation.
    pub(super) fn mobile_launches(&self, target: &Target, cx: &App) -> HashMap<i64, Value> {
        let Target::Mono(mono) = target else {
            return HashMap::new();
        };
        let Some(mono) = self
            .monos
            .as_ref()
            .and_then(|m| m.read(cx).roster.get(mono).cloned())
        else {
            return HashMap::new();
        };
        let projects = self.mobile_projects(cx);
        crate::monos::cards::launch_views(&mono, self, cx)
            .into_iter()
            .map(|(id, view)| {
                // A worktree's session belongs to the project it was made from.
                let cwd = view.cwd.as_str();
                let project = projects
                    .iter()
                    .find(|path| {
                        monocode_core::files::path_key(path) == monocode_core::files::path_key(cwd)
                    })
                    .or_else(|| {
                        mono.projects.iter().find(|path| {
                            projects.contains(path) && cwd.starts_with(path.as_str())
                        })
                    });
                let name = match project {
                    Some(path) => self.appearance(path).label(path),
                    None => std::path::Path::new(cwd)
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                };
                let phone_id = self.mobile.ids.get(&id).cloned().unwrap_or_else(|| {
                    locked(&self.store)
                        .ok()
                        .and_then(|store| store.persisted_session_id(id).ok())
                        .unwrap_or_else(|| id.to_string())
                });
                let card = json!({
                    "id": phone_id,
                    "projectId": project_id(project.map_or(cwd, String::as_str)),
                    "project": name,
                    "title": monocode_core::session::title::display_title(&view.title, &view.harness),
                    "harness": view.harness,
                    // One the desktop can't find any more reads as done.
                    "state": if view.state == "unavailable" { "done" } else { view.state },
                });
                (id, card)
            })
            .collect()
    }

    pub(super) fn mobile_session_id(&mut self, id: i64) -> String {
        if let Some(value) = self.mobile.ids.get(&id) {
            return value.clone();
        }
        let value = locked(&self.store)
            .ok()
            .and_then(|store| store.persisted_session_id(id).ok())
            .unwrap_or_else(|| id.to_string());
        self.mobile.ids.insert(id, value.clone());
        value
    }

    /// Project reads run on the background executor against a checked root.
    pub(super) fn mobile_project(&mut self, request: Request, cx: &mut Context<Self>) {
        let params = request.params.clone();
        let cwd =
            match self.mobile_project_path(&string(&params, "projectId").unwrap_or_default(), cx) {
                Ok(cwd) => cwd,
                Err(error) => return request.reply(Err(error)),
            };
        let method = request.method.clone();
        let work = cx.background_executor().spawn(async move {
            let cwd = PathBuf::from(cwd);
            let worktree = params["worktreePath"].as_str();
            let path = string(&params, "path").unwrap_or_default();
            match method.as_str() {
                "project.worktrees" => project::worktrees(&cwd),
                "project.changes" => {
                    project::changes(&project::resolve_root(&cwd, worktree)?, worktree)
                }
                "project.files" => project::files(&project::resolve_root(&cwd, worktree)?, &path),
                "project.file" => project::file(&project::resolve_root(&cwd, worktree)?, &path),
                "project.search" => project::search(
                    &project::resolve_root(&cwd, worktree)?,
                    params["query"].as_str().unwrap_or_default(),
                ),
                "project.git" => project::git(&project::resolve_root(&cwd, worktree)?),
                _ => Err("Unsupported mobile operation".into()),
            }
        });
        cx.spawn(async move |_, _| request.reply(work.await))
            .detach();
    }

    /// The desktop footer's usage windows for a provider's accounts, from the
    /// same cache; only an explicit refresh probes again.
    /// A document a Mono wrote, for the phone's reader: only one a Mono's
    /// chat lists, read from the store when the roster has no body for it.
    pub(super) fn mobile_document(&mut self, request: Request, cx: &mut Context<Self>) {
        let id = string(&request.params, "documentId").unwrap_or_default();
        let listed = self.monos.as_ref().and_then(|monos| {
            monos.read(cx).roster.monos.iter().find_map(|mono| {
                let document = mono.documents.iter().find(|d| d.id == id);
                (document.is_some() || mono.desktop_document_ids.contains(&id))
                    .then(|| document.cloned())
            })
        });
        let Some(document) = listed else {
            return request.reply(Err("This document is no longer available".into()));
        };
        // A visual runs here and goes as its rendered tree, which the phone
        // draws with the same dither; its source stays on the desktop.
        let reply = |document: monocode_core::session::mono_model::Document| {
            let mut value =
                json!({"title": document.title, "kind": document.kind, "body": document.body});
            if document.kind == "visual" {
                match visual_tree(&document.body) {
                    Ok(tree) => value["tree"] = tree,
                    Err(error) => value["treeError"] = json!(error),
                }
            }
            value
        };
        if let Some(document) = document.filter(|d| !d.body.is_empty()) {
            return request.reply(Ok(reply(document)));
        }
        let store = self.store.clone();
        let work = cx
            .background_executor()
            .spawn(async move { locked(&store)?.mono_document(&id) });
        cx.spawn(async move |_, _| {
            request.reply(work.await.and_then(|document| {
                document
                    .map(reply)
                    .ok_or_else(|| "This document is no longer available".into())
            }))
        })
        .detach();
    }

    /// A Mono's details for the phone's profile, or a change to its look.
    pub(super) fn mobile_mono(&mut self, request: Request, cx: &mut Context<Self>) {
        let id = string(&request.params, "monoId").unwrap_or_default();
        let Some(monos) = self.monos.clone() else {
            return request.reply(Err("Mono is no longer available".into()));
        };
        let result = if request.method == "mono.create" {
            let result = monos.update(cx, |monos, cx| monos.mobile_create(cx));
            if result.is_ok() {
                self.publish_mobile_catalog(cx);
            }
            result
        } else if matches!(request.method.as_str(), "mono.reset" | "mono.delete") {
            let reset = request.method == "mono.reset";
            let result = monos.update(cx, |monos, cx| {
                if reset {
                    monos.mobile_reset(&id, cx)
                } else {
                    monos.mobile_delete(&id, cx)
                }
            });
            if result.is_ok() {
                self.publish_mobile_catalog(cx);
                // A reset chat is a new one: phones watching it follow it now.
                self.push_mobile_watches(cx);
            }
            result
        } else if request.method == "mono.update" {
            // Projects come as the phone's IDs; the roster keeps paths.
            let mut params = request.params.clone();
            if let Some(add) = string(&request.params, "addProject") {
                match self.mobile_project_path(&add, cx) {
                    Ok(path) => params["addPath"] = json!(path),
                    Err(error) => return request.reply(Err(error)),
                }
            }
            if let Some(remove) = string(&request.params, "removeProject") {
                let paths: Vec<String> = monos
                    .read(cx)
                    .roster
                    .get(&id)
                    .map(|mono| {
                        mono.projects
                            .iter()
                            .filter(|p| project_id(p) == remove)
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                params["removePaths"] = json!(paths);
            }
            let result = monos.update(cx, |monos, cx| monos.mobile_update(&id, &params, cx));
            // The phone's drawer and header show the new look at once.
            if result.is_ok() {
                self.publish_mobile_catalog(cx);
            }
            result
        } else {
            let views = self.mono_session_views(cx);
            monos.update(cx, |monos, cx| monos.mobile_profile(&id, &views, cx))
        };
        request.reply(result);
    }

    /// A Mono's soul or memory for the phone to read, or its edit to save.
    pub(super) fn mobile_mono_file(&mut self, request: Request, cx: &mut Context<Self>) {
        let id = string(&request.params, "monoId").unwrap_or_default();
        let Some(monos) = self.monos.clone() else {
            return request.reply(Err("Mono is no longer available".into()));
        };
        let work = monos.update(cx, |monos, cx| monos.mobile_file(&id, &request.params, cx));
        cx.spawn(async move |_, _| request.reply(work.await))
            .detach();
    }
}

/// A catalog model as the phone's model sheet lists it, with its settings.
pub(super) fn catalog_model(kind: Kind, model: monocode_core::provider::catalog::Model) -> Value {
    let toggle = |id: &str, label: &str| {
        json!({
            "id": id, "label": label, "kind": "toggle", "value": "false",
            "options": [{"value": "true", "label": "On"}, {"value": "false", "label": "Off"}],
        })
    };
    let mut settings = Vec::new();
    if !model.efforts.is_empty() {
        settings.push(json!({
            "id": "effort",
            "label": "Reasoning",
            "kind": "select",
            "value": model.default_effort.clone().or_else(|| model.efforts.first().cloned()).unwrap_or_default(),
            "options": model.efforts.iter().map(|e| json!({"value": e, "label": title_case(e)})).collect::<Vec<_>>(),
        }));
    }
    if model.fast {
        settings.push(toggle("fast", "Fast"));
    }
    if model.thinking {
        settings.push(toggle("thinking", "Thinking"));
    }
    for control in &model.controls {
        let is_toggle = control.choices.len() == 2
            && control
                .choices
                .iter()
                .all(|(value, _)| value == "true" || value == "false");
        settings.push(json!({
            "id": control.id,
            "label": control.name,
            "kind": if is_toggle { "toggle" } else { "select" },
            "value": control.current,
            "options": control.choices.iter().map(|(value, label)| json!({"value": value, "label": label})).collect::<Vec<_>>(),
        }));
    }
    json!({
        "id": mobile_catalog::model_id(kind, Some(&model.id)),
        "harness": kind.id(),
        "name": pickers::model_display_name(kind, &model.id, &model.name).to_string(),
        "settings": settings,
    })
}
