use super::*;

impl Conversation {
    pub(super) fn composer(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let radius = COMPOSER_RADIUS;
        let command = if self.follow_ups.draft_selected {
            Some(input::Command::Draft)
        } else if self.settings.orchestrator {
            Some(input::Command::Orchestrator)
        } else if self.settings.plan {
            Some(input::Command::Plan)
        } else if self.settings.operator {
            Some(input::Command::Operator)
        } else {
            None
        };
        self.input
            .update(cx, |input, cx| input.selected_command(command, cx));
        let runner = self.composer_runner(cx);
        let beside = cx
            .try_global::<crate::ui::preferences::Preferences>()
            .is_some_and(|p| p.model_controls == "beside");
        let trigger_effort = (!beside).then(|| self.trigger_effort_label()).flatten();
        let trigger_fast = self.trigger_fast(beside);
        let controls = if beside {
            self.model_controls(cx)
        } else {
            Vec::new()
        };
        let input = self.input.read(cx);
        let has_value = input.has_message_text() || !self.attachments.is_empty();
        self.completion.update(cx, |completion, cx| {
            completion.set_cwd(
                self.settings
                    .worktree_cwd
                    .as_deref()
                    .unwrap_or(&self.summary.cwd),
                cx,
            )
        });
        let kind = Kind::from_id(&self.summary.provider);
        let menu: Vec<_> = self
            .menu
            .as_ref()
            .map(|open| {
                menu_popover(&open.menu, Anchor::Slot(self.anchor(open.kind).clone()), cx)
                    .side(Side::Top)
                    .when(open.kind == ComposerMenu::Account, |p| {
                        p.width(250.).max_height(260.)
                    })
            })
            .into_iter()
            .collect();
        div()
            .relative()
            .w_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .child(self.composer_anchor.probe())
            .children(menu)
            .children(self.model_list_popover(cx))
            .children(self.orchestration_card(cx))
            .children(if self.is_remote() { None } else { self.usage_notice(false, cx) })
            .child(self.changes_shelf(window, cx))
            .child(self.background_shelf(window, cx))
            .child(self.message_queue(window, cx))
            .children(if self.is_remote() { None } else { self.auth_notice(cx) })
            .when(self.settings.worktree_removed,|d|d.child(div().mb_2().p_3().rounded_lg().border_1().border_color(content(0.1)).flex().items_center().gap_3().text_size(px(12.)).child(div().flex_1().child("Worktree deleted. Select a branch or worktree to continue this conversation.")).child(tool_pill("reattach-worktree","Select worktree").child("Select worktree").on_click(cx.listener(|this,_,window,cx|this.open_git(GitMode::Worktree,window,cx))))))
            .when_some(self.settings.handoff.clone(), |d, handoff| {
                d.child(
                    div()
                        .mb(px(6.))
                        .px(px(12.))
                        .py(px(8.))
                        .rounded(px(10.))
                        .bg(content(0.05))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(px(12.))
                        .text_color(content(0.7))
                        .child(icon("response-handoff", 14.))
                        .child(div().min_w_0().flex_1().child(format!(
                            "Handoff from {} · Add your next instruction below",
                            Kind::from_id(&handoff.from).label()
                        )))
                        .child(
                            tool_pill("dismiss-handoff", "Dismiss handoff")
                                .child(icon("close", 12.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.settings.handoff = None;
                                    this.save_settings(cx);
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when_some(self.git_picker.clone(), |d, picker| {
                let anchor = if picker.read(cx).mode() == GitMode::Worktree {
                    self.workspace_anchor.clone()
                } else {
                    self.branch_anchor.clone()
                };
                let weak = cx.entity().downgrade();
                d.child(
                    popover("composer-git-picker", Anchor::Slot(anchor))
                        .side(Side::Top)
                        .width(300.)
                        .height(320.)
                        .on_dismiss(move |window, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.git_picker = None;
                                this.git_events = None;
                                window.focus(&this.input.focus_handle(cx), cx);
                                cx.notify();
                            });
                        })
                        .child(picker),
                )
            })
            .child(self.approval_shelf(window, cx))
            .child(
                div()
                    .id("composer-box")
                    .debug_selector(|| "composer-box".into())
                    .capture_action(cx.listener(|this, _: &input::Paste, _, cx| {
                        if this.paste_attachments(cx) {
                            cx.stop_propagation();
                        }
                    }))
                    .relative()
                    .rounded(px(radius))
                    .border_1()
                    .border_color(composer_border(self.input.focus_handle(cx).is_focused(window)))
                    .shadow(composer_shadow())
                    .when(self.file_drag, |d| d.border_color(accent_color().opacity(0.6)))
                    .flex()
                    .flex_col()
                    .on_mouse_down(
                        MouseButton::Left,
                        // Keep a focus-tracking ancestor from taking focus back.
                        cx.listener(|this, _, window, cx| {
                            window.focus(&this.input.focus_handle(cx), cx);
                            window.prevent_default();
                        }),
                    )
                    .child(self.completion.read(cx).composer_probe())
                    .child(self.btw_home.0.probe())
                    .child(composer_surface(radius))
                    .children(self.attachment_chips(cx))
                    .child(
                        div()
                            .px_3()
                            .pt(px(10.))
                            .pb(px(14.))
                            .child(self.completion.clone()),
                    )
                    .child(
                        div()
                            .debug_selector(|| "composer-toolbar".into())
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .pb(px(10.))
                            .child(
                                tool_pill("composer-add", "Add files or choose a mode")
                                    .aria_expanded(self.menu_is(ComposerMenu::Add))
                                    .when(self.menu_is(ComposerMenu::Add), |d| d.bg(content(0.2)))
                                    .child(self.add_anchor.probe())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_menu(ComposerMenu::Add, window, cx)
                                    }))
                                    .w(px(26.))
                                    .justify_center()
                                    .text_color(content(0.5))
                                    .child(
                                        icon("plus", 14.)
                                            .text_color(content(0.5))
                                            .group_hover("composer-add", |s| {
                                                s.text_color(content(1.))
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        tool_pill("composer-model", "Model")
                                            .min_w_0()
                                            .max_w(px(220.))
                                            .flex_shrink(1.)
                                            .aria_label(format!(
                                                "{}, {}{}",
                                                kind.label(),
                                                self.model_label(),
                                                trigger_effort
                                                    .as_ref()
                                                    .map(|effort| format!(", effort {effort}"))
                                                    .unwrap_or_default()
                                            ))
                                            .child(self.model_anchor.probe())
                                            .aria_expanded(self.menu_is(ComposerMenu::Model) || self.menu_is(ComposerMenu::Recent) || self.mono_ui.models.is_some())
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.toggle_menu(ComposerMenu::Model, window, cx)
                                            }))
                                            .on_mouse_down(
                                                MouseButton::Right,
                                                cx.listener(|this, _, window, cx| {
                                                    cx.stop_propagation();
                                                    this.open_recent_models(window, cx)
                                                }),
                                            )
                                            .child(provider_icon(&self.summary.provider, 16.))
                                            .child(pill_label(self.model_label()))
                                            .when_some(trigger_effort, |d, effort| {
                                                d.child(
                                                    div()
                                                        .debug_selector(|| "composer-model-effort".into())
                                                        .flex_shrink_0()
                                                        .text_size(px(11.))
                                                        .text_color(content(0.5))
                                                        .child(effort),
                                                )
                                            })
                                            .when(trigger_fast, |d| {
                                                d.child(fast_bolt())
                                            })
                                            .child(icon("chevron-down", 12.)),
                                    )
                                    .children(controls)
                                    .child(
                                        tool_pill("composer-access", "Access")
                                            .aria_label(format!(
                                                "Access, {}",
                                                self.settings.access.label()
                                            ))
                                            .aria_expanded(self.menu_is(ComposerMenu::Access))
                                            .child(self.access_anchor.probe())
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.toggle_menu(ComposerMenu::Access, window, cx)
                                            }))
                                            .child(
                                                icon(
                                                    match self.settings.access {
                                                        AccessMode::FullAccess => "shield-alert",
                                                        AccessMode::AutoAcceptEdits => {
                                                            "pencil-edit"
                                                        }
                                                        AccessMode::Auto => "sparkles",
                                                        _ => "square-lock",
                                                    },
                                                    14.,
                                                )
                                                .text_color(
                                                    if self.settings.access
                                                        == AccessMode::FullAccess
                                                    {
                                                        tint(AMBER_400, 0.9)
                                                    } else {
                                                        content(1.)
                                                    },
                                                ),
                                            )
                                            .child(pill_label(self.settings.access.label()))
                                            .child(icon("chevron-down", 12.)),
                                    ),
                            )
                            .child(self.action(has_value, window, cx)),
                    )
                    .children(self.attachment_drop_overlay(12.)),
            )
            .children(runner.map(|runner| {
                runner.cached(crate::chat::composer::runner::Runner::placement())
            }))
            .children(self.remote_notice(cx))
            .child(self.workspace_bar(cx))
            .into_any_element()
    }

    pub(super) fn workspace_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let draft = !self.summary.has_messages;
        let linked =
            self.git.as_ref().is_some_and(|s| s.linked) || self.settings.worktree_cwd.is_some();
        let new = draft && self.settings.new_worktree;
        let label = if new {
            "New worktree"
        } else if linked {
            "Worktree"
        } else {
            "Current checkout"
        };
        let enabled = !self.busy && !self.preparing_worktree && self.git.is_some();
        // Still reading git is not unavailable: don't dim, then brighten.
        let dimmed = !enabled && (self.loaded || self.busy || self.preparing_worktree);
        let workspace = if draft {
            bar_trigger(
                "composer-workspace",
                if new || linked {
                    "folder-tree"
                } else {
                    "folder"
                },
                label.into(),
            )
            .aria_label(format!("Workspace {label}"))
            .aria_expanded(self.menu_is(ComposerMenu::Workspace))
            .child(self.workspace_anchor.probe())
            .when(dimmed, |d| d.opacity(0.4))
            .on_click(cx.listener(move |this, _, window, cx| {
                if enabled {
                    this.refresh_git(cx);
                    this.toggle_menu(ComposerMenu::Workspace, window, cx);
                }
            }))
            .into_any_element()
        } else {
            // WorkspaceIdentity: a started conversation owns this working copy.
            div()
                .id("composer-workspace")
                .role(Role::Group)
                .aria_label(format!("Workspace {label}"))
                .flex_shrink_0()
                .h(px(24.))
                .ml(px(-6.))
                .px(px(6.))
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.))
                .text_color(content(0.55 * 0.4))
                .child(
                    icon(if linked { "folder-tree" } else { "folder" }, 14.)
                        .text_color(content(0.55 * 0.4)),
                )
                .child(label)
                .into_any_element()
        };
        let branch = if new {
            self.settings.worktree_base.clone().or(self.branch.clone())
        } else {
            self.branch.clone()
        };
        let ratio = self.context.and_then(|usage| usage.ratio());
        let show_context = ratio.is_some() || draft;
        let headline = ratio
            .map(|r| format!("{:.0}% context used", r * 100.))
            .unwrap_or_else(|| "No context used yet".into());
        let detail = self.context.map(|usage| {
            format!(
                "{} / {} tokens",
                monocode_core::session::context::format_tokens(usage.used),
                monocode_core::session::context::format_tokens(usage.window.unwrap_or(0))
            )
        });
        div()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(10.))
            .pt(px(6.))
            .px(px(15.))
            .child(workspace)
            .when_some(branch, |d, branch| {
                let detached = !new && self.git.as_ref().is_some_and(|s| s.detached);
                let label = if detached {
                    format!("detached {branch}")
                } else {
                    branch
                };
                d.child(
                    bar_trigger("composer-branch", "git-branch", label.clone().into())
                        .aria_label(format!(
                            "{} {label}",
                            if new { "Worktree base" } else { "Branch" }
                        ))
                        .aria_description(self.execution_cwd().to_owned())
                        .max_w(px(256.))
                        .child(self.branch_anchor.probe())
                        .when(dimmed, |d| d.opacity(0.4))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if enabled {
                                this.open_git(
                                    if new { GitMode::Base } else { GitMode::Branch },
                                    window,
                                    cx,
                                );
                            }
                        })),
                )
            })
            .child(
                div()
                    .ml_auto()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when(self.editing_last_turn.pending(), |d| {
                        d.child(
                            div()
                                .id("cancel-edit-last-turn")
                                .role(Role::Button)
                                .aria_label("Cancel editing")
                                .text_size(px(11.))
                                .text_color(content(0.6))
                                .child("Cancel edit")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_edit_last_turn(true, cx);
                                })),
                        )
                    })
                    .when(
                        self.status != "Ready"
                            && !self.busy
                            && !self.loading
                            && self.settings.usage_limit.is_none(),
                        |d| {
                            d.child(
                                div()
                                    .id("provider-status")
                                    .debug_selector(|| "provider-status".into())
                                    .role(Role::Status)
                                    .aria_label(self.status.clone())
                                    .max_w(px(220.))
                                    .truncate()
                                    .text_size(px(11.))
                                    .text_color(content(0.45))
                                    .child(self.status.clone()),
                            )
                        },
                    )
                    .when(show_context, |d| {
                        d.child(
                            div()
                                .id("context-meter")
                                .role(Role::Image)
                                .aria_label(format!(
                                    "{headline}{}",
                                    detail
                                        .as_ref()
                                        .map(|s| format!(", {s}"))
                                        .unwrap_or_default()
                                ))
                                .child(self.context_anchor.probe())
                                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                    this.context_hovered = *hovered;
                                    cx.notify();
                                }))
                                .child(crate::chat::composer::context_meter::ring(
                                    ratio.unwrap_or(0.),
                                ))
                                .when(self.context_hovered, |d| {
                                    d.child(
                                        popover(
                                            "context-tooltip",
                                            Anchor::Slot(self.context_anchor.clone()),
                                        )
                                        .side(Side::Top)
                                        .align(Align::End)
                                        .max_width(240.)
                                        .padding(10.)
                                        .child(
                                            div()
                                                .flex()
                                                .flex_col()
                                                .text_size(px(12.))
                                                .line_height(px(16.))
                                                .text_color(content(1.))
                                                .child(headline)
                                                .when_some(detail, |d, detail| {
                                                    d.child(
                                                        div()
                                                            .text_size(px(11.))
                                                            .text_color(content(0.5))
                                                            .child(detail),
                                                    )
                                                }),
                                        ),
                                    )
                                }),
                        )
                    }),
            )
    }

    // ComposerAction: a solid submit, dimmed while empty; Stop while busy.
    pub(super) fn action(
        &self,
        has_value: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let busy = self.responding() && !has_value;
        let draft = self.draft_active(cx);
        let modifiers = window.modifiers();
        let alternate = if cfg!(target_os = "macos") {
            modifiers.platform
        } else {
            modifiers.control
        };
        let follow_up = (has_value && !draft)
            .then(|| self.follow_up_action(alternate, cx))
            .flatten();
        let enabled = !self.attaching
            && (!self.preparing_worktree || self.busy || self.stopping)
            && (busy || (has_value && self.loaded))
            && (busy || !draft || (!self.busy && self.follow_ups.state.draft.is_none()));
        // Something to send grows the button into a pill naming the action;
        // empty or stopping, it rests as a square.
        let face = crate::chat::composer::send_morph::Face {
            label: if busy {
                "Stop"
            } else if draft {
                "Save draft"
            } else {
                follow_up.map_or("Send", |(label, _)| label)
            },
            glyph: follow_up.map_or("send", |(_, glyph)| glyph),
            stop: busy,
            expanded: !busy && has_value,
        };
        let frame = self.send_morph.borrow_mut().frame(
            face,
            crate::chat::composer::send_morph::expanded_width(face.label, window),
            !cx.reduce_motion(),
        );
        if frame.moving {
            window.request_animation_frame();
        }
        crate::chat::composer::send_morph::button("send-or-stop", face, &frame, enabled)
            // Holding ⌘ (Ctrl elsewhere) flips Queue and Steer; redraw so the
            // label rolls to it while the key is down.
            // Only ⌘'s own changes matter, and only while it changes the action.
            .on_modifiers_changed(cx.listener(
                |this, event: &gpui::ModifiersChangedEvent, _, cx| {
                    let alternate = if cfg!(target_os = "macos") {
                        event.modifiers.platform
                    } else {
                        event.modifiers.control
                    };
                    if std::mem::replace(&mut this.send_alternate, alternate) != alternate
                        && this.follow_up_action(alternate, cx).is_some()
                    {
                        cx.notify();
                    }
                },
            ))
            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                if !enabled {
                    return;
                }
                if busy {
                    this.stop(cx);
                } else {
                    let modifiers = event.modifiers();
                    let alternate = if cfg!(target_os = "macos") {
                        modifiers.platform
                    } else {
                        modifiers.control
                    };
                    this.send_follow_up(alternate, cx);
                }
            }))
    }

    // EmptySession.tsx: the composer centred under its question, or resting
    // where it will dock once a neighbouring pane squeezes it narrower than
    // the column. A narrow window alone keeps it centred.
    pub(super) fn empty(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        // A chat background replaces the canvas; the ink only lives in motion.
        let fill = cx
            .try_global::<crate::ui::preferences::Preferences>()
            .map_or("ink", |p| {
                if p.chat_background.is_some() {
                    "none"
                } else {
                    p.empty_session.as_str()
                }
            });
        let canvas_fill: Option<AnyElement> = match fill {
            "games" => Some(
                self.arcade
                    .get_or_insert_with(|| cx.new(crate::chat::arcade::Arcade::new))
                    .clone()
                    .into_any_element(),
            ),
            "ink" if !cx.reduce_motion() => Some(
                self.ink
                    .get_or_insert_with(|| cx.new(crate::chat::ink_field::InkField::new))
                    .clone()
                    .into_any_element(),
            ),
            _ => None,
        };
        let this = cx.entity().downgrade();
        let narrow = self.narrow;
        let docked = narrow && self.beside_pane;
        self.attachment_drop_target("empty-session", cx)
            .relative()
            .size_full()
            .flex()
            .justify_center()
            .children(canvas_fill)
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let now = bounds.size.width < px(COMPOSER_COLUMN);
                        if now != narrow {
                            cx.defer(move |cx| {
                                let _ = this.update(cx, |this, cx| {
                                    this.narrow = now;
                                    cx.notify();
                                });
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .w_full()
                    .max_w(px(COMPOSER_COLUMN))
                    .flex()
                    .flex_col()
                    .px(px(10.))
                    .pt(px(24.))
                    .pb(px(6.))
                    .child(div().flex_grow(1.).flex_basis(px(0.)))
                    .when(!docked, |d| {
                        d.child(
                            div()
                                .id("empty-session-title")
                                .role(Role::Heading)
                                .flex_shrink_0()
                                .mb_4()
                                .px(px(10.))
                                .text_size(px(18.))
                                .line_height(px(28.))
                                .text_color(content(1.))
                                .truncate()
                                .child(format!(
                                    "What should we work on in {}?",
                                    self.project_appearance.label(&self.summary.cwd)
                                )),
                        )
                    })
                    .child(self.composer(window, cx))
                    .when(!docked, |d| {
                        d.child(div().flex_grow(1.).flex_shrink(1.).flex_basis(px(18.)))
                    }),
            )
            .into_any()
    }
}

impl Render for Conversation {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self._input_focus.is_none() {
            let focus = self.input.focus_handle(cx);
            self._input_focus = Some([
                cx.on_focus(&focus, window, |_, _, cx| cx.notify()),
                cx.on_blur(&focus, window, |_, _, cx| cx.notify()),
            ]);
        }
        if self.response_events.is_none() {
            self.response_events = Some(cx.subscribe_in(
                &self.transcript,
                window,
                |this, _, request: &ResponseRequest, window, cx| {
                    this.response_request(request, window, cx)
                },
            ));
        }
        if self.settings.mono.is_some() {
            return self.render_mono(window, cx);
        }
        if let Some(terminal) = self.claude_terminal(window, cx) {
            return terminal;
        }
        if !self.summary.has_messages {
            return self.empty(window, cx);
        }
        self.attachment_drop_target("session-chat", cx)
            .relative()
            .capture_action(
                cx.listener(|this, _: &crate::ui::popover::DismissMenu, window, cx| {
                    if this.close_remote_plan(cx) {
                        cx.stop_propagation();
                        return;
                    }
                    // BtwSheet: Esc anywhere in this pane folds an open sheet away.
                    if let Some(btw) = this.btw.clone()
                        && btw.update(cx, |btw, cx| btw.dismiss(window, cx))
                    {
                        cx.stop_propagation();
                    }
                }),
            )
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .when_some(self.response_menu.as_ref(), |d, menu| {
                let panel =
                    menu_popover(&menu.menu, Anchor::Slot(menu.anchor.clone()), cx).side(Side::Top);
                d.child(
                    menu.regions
                        .iter()
                        .fold(panel, |panel, region| panel.ignore(region.clone())),
                )
            })
            .child({
                let git = self.git_action_pill(cx);
                let changes = self.changes_pill(cx);
                let beside = git.is_some() || changes.is_some();
                let pill = beside.then(|| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .children(git)
                        .children(changes)
                });
                self.transcript
                    .update(cx, |t, cx| t.set_jump_beside_pill(beside, cx));
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        self.transcript
                            .clone()
                            .cached(StyleRefinement::default().size_full()),
                    )
                    // Level with the transcript's jump button, which moves to
                    // the column's other end.
                    .children(pill.map(|pill| {
                        div()
                            .absolute()
                            .bottom(px(8.))
                            .left_0()
                            .w_full()
                            .flex()
                            .justify_center()
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(COMPOSER_COLUMN))
                                    .px(px(16.))
                                    .flex()
                                    .child(pill),
                            )
                    }))
            })
            .child(
                div()
                    .flex_shrink_0()
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(COMPOSER_COLUMN))
                            .opacity(self.btw_home.1)
                            .flex()
                            .flex_col()
                            .px(px(10.))
                            .pb(px(6.))
                            .child({
                                let composer = self
                                    .composer_motion
                                    .clone()
                                    .wrap(self.composer(window, cx))
                                    .into_any_element();
                                let focus = self.input.focus_handle(cx);
                                let card = self.questions.front().map(|(card, _)| card);
                                self.question_overlay
                                    .wrap(composer, card, &focus, window, cx)
                            }),
                    ),
            )
            .when_some(self.btw.as_ref(), |d, sheet| d.child(sheet.clone()))
            .children(self.remote_plan_reader(cx))
            .into_any()
    }
}

/// `[data-composer-box]::before`: the frosted (or, in light themes, solid)
/// surface inside the composer's border.
pub(super) fn composer_surface(radius: f32) -> impl IntoElement {
    let blur = composer_blur();
    let fill = composer_fill();
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let surface = gpui::fill(bounds, fill).corner_radii(px(radius - 1.));
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            let surface = if blur > 0. {
                surface.border_color(gpui::backdrop_blur_marker(blur * window.scale_factor()))
            } else {
                surface
            };
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            let _ = blur;
            window.paint_quad(surface);
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// ComposerAction: the solid send button, or a stop square while busy.
pub(super) fn send_button(
    id: &'static str,
    label: &'static str,
    busy: bool,
    glyph: &'static str,
    enabled: bool,
) -> gpui::Stateful<gpui::Div> {
    let accent = primary_action(enabled);
    // Without an accent the button is the inverse of the surface: white with
    // black ink on dark, black with white ink on light.
    let light = crate::ui::theme::is_light();
    let solid: fn(f32) -> gpui::Hsla = if light { black } else { white };
    let ink: fn(f32) -> gpui::Hsla = if light { white } else { black };
    div()
        .id(id)
        .debug_selector(|| id.into())
        .role(Role::Button)
        .aria_label(label)
        .cursor_pointer()
        .flex_shrink_0()
        .size(px(26.))
        .rounded_md()
        .flex()
        .items_center()
        .justify_center()
        .map(|d| match accent {
            Some((fill, _)) => d
                .bg(fill)
                .when(enabled, |d| d.hover(|s| s.bg(fill.blend(content(0.12))))),
            None => d
                .bg(solid(if enabled { 1. } else { 0.3 }))
                .when(enabled, |d| d.hover(|s| s.bg(solid(0.9)))),
        })
        .map(|d| {
            let ink = accent.map_or(ink(if enabled { 1. } else { 0.4 }), |(_, ink)| ink);
            if busy {
                d.child(div().size(px(10.)).rounded(px(2.)).bg(ink))
            } else {
                d.child(icon(glyph, 14.).text_color(ink))
            }
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

// GitPickerTrigger: the quiet `h-6` controls in the bar under the box.
pub(super) fn bar_trigger(
    id: &'static str,
    path: &'static str,
    label: SharedString,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(Role::Button)
        .group(id)
        .cursor_pointer()
        .min_w_0()
        .h(px(24.))
        .ml(px(-6.))
        .px(px(6.))
        .rounded_md()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(12.))
        .text_color(content(0.55))
        .hover(|s| s.bg(content(0.08)).text_color(content(1.)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            icon(path, 14.)
                .text_color(content(0.55))
                .group_hover(id, |s| s.text_color(content(1.))),
        )
        .child(div().min_w_0().truncate().child(label))
}
