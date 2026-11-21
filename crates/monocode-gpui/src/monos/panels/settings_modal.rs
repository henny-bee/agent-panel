use super::*;

pub(super) fn setting(label: &str, description: Option<&str>, control: AnyElement) -> Div {
    div()
        .min_h(px(48.))
        .flex()
        .items_center()
        .gap_4()
        .px_3()
        .py_2()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(px(13.))
                        .line_height(px(20.))
                        .text_color(content(0.9))
                        .child(label.to_string()),
                )
                .when_some(description, |d, text| {
                    d.child(
                        div()
                            .text_size(px(12.))
                            .line_height(px(20.))
                            .text_color(content(0.4))
                            .child(text.to_string()),
                    )
                }),
        )
        .child(
            div()
                .w(px(224.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_end()
                .pr_1p5()
                .text_size(px(12.))
                .text_color(content(0.85))
                .child(control),
        )
}

pub(super) fn switch_setting(
    label: &str,
    description: &str,
    disabled: bool,
    control: AnyElement,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .px_3()
        .py_2()
        .when(disabled, |d| d.opacity(0.4))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(px(13.))
                        .line_height(px(20.))
                        .text_color(content(0.9))
                        .child(label.to_string()),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .line_height(px(20.))
                        .text_color(content(0.4))
                        .truncate()
                        .child(description.to_string()),
                ),
        )
        .child(control)
}

impl Monos {
    pub(super) fn open_settings_look(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mono) = self.roster.get(id).cloned() else {
            return;
        };
        let editor = cx.new(|cx| {
            let mut editor = LookEditor::new(mono, cx);
            editor.settings = true;
            editor
        });
        let focus = editor.read(cx).focus.clone();
        let changed = cx.subscribe(&editor, |this, _, event: &LookChanged, cx| {
            if let Some(mono) = this.roster.get_mut(&event.mono.id) {
                mono.mascot = event.mono.mascot.clone();
                mono.color = event.mono.color;
            }
            this.configure_chat(&event.mono.id, cx);
            this.save(cx);
        });
        let menu = cx.new(|cx| {
            Menu::new(
                "Mascot and color",
                vec![MenuItem::Custom(editor.into(), focus)],
                window,
                cx,
            )
            .width(272.)
        });
        let closed = cx.subscribe(&menu, |this, _, _: &MenuEvent, cx| {
            this.picker = None;
            cx.notify();
        });
        let anchor = Anchor::Slot(self.settings_anchor.clone());
        self.picker = Some(OpenPicker {
            menu,
            anchor,
            width: 272.,
            align: crate::ui::popover::Align::Center,
            _events: vec![changed, closed],
        });
        cx.notify();
    }

    pub(in crate::monos) fn field(
        &mut self,
        key: &str,
        label: &str,
        text: String,
        multi: bool,
        cx: &mut Context<Self>,
    ) -> Entity<Input> {
        if let Some(field) = self.fields.get(key) {
            return field.clone();
        }
        let label = label.to_string();
        let field = cx.new(|cx| {
            let mut f = if multi {
                Input::markdown_editor(&label, cx)
            } else if key == "memory-fact" || key.starts_with("memory-edit:") {
                Input::memory_fact(&label, cx)
            } else {
                Input::field(label.clone(), label.clone(), content(0.35), cx)
            };
            f.set_font_size(if key == "memory-fact" || key.starts_with("memory-edit:") {
                12.
            } else {
                13.
            });
            f.set_placeholder(label, cx);
            f.set_text(text, cx);
            f
        });
        let key_owned = key.to_string();
        self.observations.push(
            cx.subscribe(&field, move |this, input, _: &input::Changed, cx| {
                if key_owned == "mono-name" {
                    let name = input.read(cx).content.trim().chars().take(40).collect();
                    if let Some(id) = this.active.clone() {
                        if let Some(mono) = this.roster.get_mut(&id) {
                            mono.name = name;
                        }
                        this.configure_chat(&id, cx);
                        this.save(cx);
                    }
                }
                cx.notify();
            }),
        );
        self.fields.insert(key.into(), field.clone());
        field
    }

    pub(in crate::monos) fn value(&self, key: &str, cx: &App) -> String {
        self.fields
            .get(key)
            .map(|f| f.read(cx).content.to_string())
            .unwrap_or_default()
    }

    pub(in crate::monos) fn settings_modal(
        &mut self,
        mono: &Mono,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let section = self.settings?;
        let size = window.viewport_size();
        let width = 880f32.min(size.width.as_f32() - 24.);
        let height = 680f32.min(size.height.as_f32() - 32.);
        let body = match section {
            SettingsSection::General => self.general_settings(mono, window, cx),
            SettingsSection::Soul => self.soul_page(mono, cx),
            SettingsSection::Memory => self.memory_page(mono, window, cx),
        };
        self.bind_settings_blurs(window, cx);
        let weak = cx.weak_entity();
        Some(
            div()
                .absolute()
                .size_full()
                .child(
                    gpui::deferred(
                        gpui::anchored()
                            .position(gpui::point(px(0.), px(0.)))
                            .child(
                                div()
                                    .w(size.width)
                                    .h(size.height)
                                    .bg(gpui::black().opacity(0.4))
                                    .occlude()
                                    .child(crate::ui::html::cover()),
                            ),
                    )
                    .with_priority(69),
                )
                .child(
                    popover(
                        "mono-settings-modal",
                        Anchor::Point(gpui::point(
                            px((size.width.as_f32() - width) / 2.),
                            px((size.height.as_f32() - height) / 2.),
                        )),
                    )
                    .ignore(self.notice_region.clone())
                    .modal_surface(true)
                    .without_animation()
                    .priority(70)
                    .width(width)
                    .height(height)
                    .gap(0.)
                    .on_dismiss(move |_, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.save_open_file(cx);
                            this.settings = None;
                            this.picker = None;
                            cx.notify();
                        });
                    })
                    .child(
                        div()
                            .id("mono-settings-dialog")
                            .debug_selector(|| "mono-settings-dialog".into())
                            .role(Role::Dialog)
                            .track_focus(&self.settings_focus)
                            .aria_label(format!("{} settings", mono.label()))
                            .h_full()
                            .w_full()
                            .flex()
                            .relative()
                            .child(
                                div()
                                    .id("mono-settings-navigation")
                                    .debug_selector(|| "mono-settings-navigation".into())
                                    .role(Role::Navigation)
                                    .aria_label("Settings sections")
                                    .w(px(192.))
                                    .flex_shrink_0()
                                    .h_full()
                                    .border_r_1()
                                    .border_color(content(0.04))
                                    .p_2()
                                    .flex()
                                    .flex_col()
                                    .gap_px()
                                    .child(
                                        div()
                                            .px_2p5()
                                            .pt_1p5()
                                            .pb_2()
                                            .text_size(px(11.))
                                            .text_color(content(0.45))
                                            .child("Settings"),
                                    )
                                    .children(
                                        [
                                            (SettingsSection::General, "General", "settings"),
                                            (SettingsSection::Soul, "Soul", "sparkles"),
                                            (SettingsSection::Memory, "Memory", "sticky-note"),
                                        ]
                                        .into_iter()
                                        .map(
                                            |(tab, label, glyph)| {
                                                let row_id = format!("mono-settings-{label}");
                                                div()
                                                    .id(SharedString::from(row_id.clone()))
                                                    .debug_selector(move || row_id.clone())
                                                    .role(Role::Button)
                                                    .aria_label(label)
                                                    .w_full()
                                                    .h(px(32.))
                                                    .flex_shrink_0()
                                                    .flex()
                                                    .flex_row()
                                                    .items_center()
                                                    .justify_start()
                                                    .text_align(gpui::TextAlign::Left)
                                                    .gap(px(10.))
                                                    .px(px(10.))
                                                    .py(px(6.))
                                                    .rounded_lg()
                                                    .text_size(px(13.))
                                                    .line_height(px(20.))
                                                    .text_color(content(if section == tab {
                                                        1.
                                                    } else {
                                                        0.65
                                                    }))
                                                    .when(section == tab, |d| d.bg(content(0.08)))
                                                    .when(section != tab, |d| {
                                                        d.hover(|d| {
                                                            d.bg(content(0.05))
                                                                .text_color(content(1.))
                                                        })
                                                    })
                                                    .child(
                                                        div()
                                                            .id(SharedString::from(format!(
                                                                "mono-settings-icon-{label}"
                                                            )))
                                                            .debug_selector(move || {
                                                                format!(
                                                                    "mono-settings-icon-{label}"
                                                                )
                                                            })
                                                            .size(px(14.))
                                                            .flex_shrink_0()
                                                            .child(icon(glyph, 14.).text_color(
                                                                content(if section == tab {
                                                                    1.
                                                                } else {
                                                                    0.65
                                                                }),
                                                            )),
                                                    )
                                                    .child(
                                                        div()
                                                            .id(SharedString::from(format!(
                                                                "mono-settings-label-{label}"
                                                            )))
                                                            .debug_selector(move || {
                                                                format!(
                                                                    "mono-settings-label-{label}"
                                                                )
                                                            })
                                                            .child(label),
                                                    )
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            this.save_open_file(cx);
                                                            this.settings = Some(tab);
                                                            window.focus(&this.settings_focus, cx);
                                                            cx.notify();
                                                        },
                                                    ))
                                            },
                                        ),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .flex()
                                    .flex_col()
                                    .child(body),
                            )
                            .child(
                                div().absolute().right_3().top_1p5().child(
                                    icon_button(
                                        "mono-settings-close",
                                        "Close Mono settings",
                                        "close",
                                    )
                                    .size(px(28.))
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            this.save_open_file(cx);
                                            this.settings = None;
                                            cx.notify();
                                        },
                                    )),
                                ),
                            ),
                    ),
                )
                .into_any_element(),
        )
    }

    pub(super) fn general_settings(
        &mut self,
        mono: &Mono,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = self.field("mono-name", "Name", mono.name.clone(), false, cx);
        name.update(cx, |f, cx| {
            f.set_font_size(15.);
            f.set_line_height(20.);
            f.set_centered(true);
            f.set_placeholder(mono.label(), cx);
        });
        let look = mono.id.clone();
        let add = mono.id.clone();
        let reset = mono.id.clone();
        let chat = self.chat();
        let model = chat
            .as_ref()
            .map(|v| v.read(cx).mono_model_label())
            .unwrap_or_else(|| "Loading…".into());
        let provider = chat
            .as_ref()
            .map(|v| v.read(cx).summary.provider.clone())
            .unwrap_or_else(|| "codex".into());
        let mode = chat
            .as_ref()
            .map(|v| v.read(cx).settings.access)
            .unwrap_or(AccessMode::Auto);
        let model_options = chat
            .as_ref()
            .map(|v| v.read(cx).mono_model_options())
            .unwrap_or_default();
        let sub = self.subsession_choice(mono, cx);
        let access_icon = match mode {
            AccessMode::FullAccess => "shield-alert",
            AccessMode::Auto => "sparkles",
            AccessMode::AutoAcceptEdits => "pencil-edit",
            _ => "square-lock",
        };
        let plain = |id, label: SharedString| {
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(label.clone())
                .h(px(28.))
                .px_1p5()
                .mx(px(-6.))
                .rounded_md()
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(12.))
                .text_color(content(0.85))
                .hover(|s| s.bg(content(0.06)))
                .child(label)
        };
        div().id("mono-general-settings").debug_selector(||"mono-general-settings".into())
            .flex_1().min_h_0().overflow_y_scroll().px_8().pt_6().pb_8()
            .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).child("General"))
            .child(div().flex().flex_col().items_center().pt_4().pb_6()
                .child(div().id("mono-look").debug_selector(||"mono-look".into()).role(Role::Button)
                    .aria_label("Change mascot and color").relative().size(px(80.)).flex_shrink_0()
                    .flex().items_center().justify_center().rounded_2xl().border_1().border_color(content(0.04))
                    .bg(content(0.04)).hover(|s|s.bg(content(0.08)))
                    .child(self.settings_anchor.probe())
                    .child(crate::ui::avatar::avatar(&mono.mascot, mono.color, "idle", 48., false))
                    .on_click(cx.listener(move |this, _, w, cx|this.open_settings_look(&look,w,cx))))
                .child(div().id("mono-name-field").debug_selector(||"mono-name-field".into()).mt_3().h(px(32.)).w(px(256.)).max_w_full().px_2().py_1p5()
                    .rounded_md().font_weight(FontWeight::MEDIUM).hover(|s|s.bg(content(0.05))).child(name)))
            .child(heading("Model", None))
            .child(card()
                .child(setting("Model",Some("What it thinks and replies with."),
                    div().id("mono-settings-model").role(Role::Button).aria_label(format!("Choose model, {model}"))
                        .h(px(28.)).px_1p5().mx(px(-6.)).rounded_md().flex().items_center().gap_2()
                        .text_size(px(12.)).text_color(content(0.85)).hover(|s|s.bg(content(0.06)))
                        .child(self.model_anchor.probe()).child(provider_icon(&provider,14.)).child(model)
                        .on_click(cx.listener(|this,_,w,cx|this.open_model_picker(w,cx))).into_any_element()))
                .children(model_options.into_iter().filter_map(|item| {
                    let MenuItem::Action(option)=item else { return None; };
                    let label=option.label.clone();
                    let value=option.value.clone().unwrap_or_else(||if option.checked {"On"}else{"Off"}.into());
                    let anchor=AnchorSlot::default(); let probe=anchor.probe();
                    Some(setting(&label,None,
                        plain(SharedString::from(format!("mono-option-{}",option.id)),value).child(probe)
                            .on_click(cx.listener(move |this,_,w,cx| {
                                if option.submenu.is_empty() {
                                    if let Some(chat)=this.chat() && let Err(e)=chat.update(cx,|v,cx|v.mono_pick_model_option(&option.id,cx)) { this.error=Some(e); }
                                    this.changed(cx);
                                } else { this.open_model_option(option.label.clone(),option.submenu.clone(),anchor.clone(),w,cx); }
                            })).into_any_element()).border_t_1().border_color(content(0.04)))
                }))
                .child(setting("Permissions",Some("What it may do without asking."),
                    div().id("mono-settings-access").role(Role::Button).aria_label(format!("Permissions, {}",mode.label()))
                        .h(px(28.)).max_w(px(208.)).px_1p5().mx(px(-6.)).rounded_md().flex().items_center().gap_2()
                        .text_size(px(12.)).text_color(content(0.85)).hover(|s|s.bg(content(0.06)))
                        .child(self.access_anchor.probe()).child(icon(access_icon,14.).when(mode==AccessMode::FullAccess,|i|i.text_color(rgb(0xffb900))))
                        .child(mode.label()).child(icon("chevron-down",12.))
                        .on_click(cx.listener(|this,_,w,cx|this.open_access_picker(w,cx))).into_any_element())
                    .border_t_1().border_color(content(0.04))))
            .child(heading("Projects",Some("It can see every session, note and folder in these.")))
            .child(card()
                .children(mono.projects.iter().enumerate().map(|(ix,project)| {
                    let remove=project.clone();
                    let appearance=self.owner.upgrade().and_then(|o|o.read(cx).workspace.project_appearance.get(project).cloned()).unwrap_or_default();
                    let label=appearance.label(project);
                    let pretty=std::env::var("HOME").ok().and_then(|home|project.strip_prefix(&home).map(|rest|format!("~{rest}"))).unwrap_or_else(||project.clone());
                    let group:SharedString=format!("mono-project-{ix}").into();
                    div().id(("mono-project",ix)).group(group.clone()).min_h(px(48.)).flex().items_center().gap_3().px_3().py_2()
                        .when(ix>0,|d|d.border_t_1().border_color(content(0.04)))
                        .child(div().size(px(32.)).flex_shrink_0().rounded_lg().bg(content(0.05)).flex().items_center().justify_center()
                            .child(crate::ui::project_badge::project_icon(&appearance,project,16.)))
                        .child(div().flex_1().min_w_0()
                            .child(div().text_size(px(13.)).line_height(px(20.)).text_color(content(0.9)).truncate().child(label))
                            .child(div().text_size(px(11.)).line_height(px(16.)).text_color(content(0.4)).truncate().child(pretty)))
                        .child(icon_button("mono-remove-project","Remove project","close").opacity(0.).group_hover(group,|s|s.opacity(1.))
                            .on_click(cx.listener(move |this,_,_,cx| { if let Some(id)=this.active.clone() {
                                if let Some(m)=this.roster.get_mut(&id) {m.projects.retain(|p|p!=&remove);}
                                this.configure_chat(&id,cx);this.save(cx);
                            }})))
                }))
                .child(div().id("mono-add-project").role(Role::Button).aria_label("Add project").h(px(44.)).px_3().flex().items_center().gap_3()
                    .text_size(px(13.)).text_color(content(0.55)).hover(|s|s.bg(content(0.04)).text_color(content(1.)))
                    .when(!mono.projects.is_empty(),|d|d.border_t_1().border_color(content(0.04)))
                    .child(self.projects_anchor.probe())
                    .child(div().size(px(32.)).rounded_lg().border_1().border_color(content(0.15)).flex().items_center().justify_center().child(icon("plus",14.)))
                    .child("Add project").on_click(cx.listener(move |this,_,w,cx|this.pick_project(&add,w,cx)))))
            .child(heading("Sessions it starts",None))
            .child(card()
                .child(switch_setting("Show sessions in sidebar","Sessions it starts appear in your project sidebar. Hidden ones stay saved and open from its overview.",false,
                    self.switch("mono-show-started",mono.show_started_sessions_in_sidebar,false,|m,on|m.show_started_sessions_in_sidebar=on,cx)))
                .child(switch_setting("Group in a folder",&format!("Sessions it starts go in a “{}” folder in your project sidebar.",mono.label()),!mono.show_started_sessions_in_sidebar,
                    self.switch("mono-group-started",mono.show_started_sessions_in_sidebar&&mono.use_sidebar_folders,!mono.show_started_sessions_in_sidebar,|m,on|m.use_sidebar_folders=on,cx))
                    .border_t_1().border_color(content(0.04)))
                .child(setting("Model",Some("For sessions it starts without naming one."),
                    div().id("mono-subsession-model").role(Role::Button).aria_label(format!("Model for sessions it starts, {}",sub.model_label))
                        .h(px(28.)).px_1p5().mx(px(-6.)).rounded_md().flex().items_center().gap_2()
                        .text_size(px(12.)).text_color(content(0.85)).hover(|s|s.bg(content(0.06)))
                        .child(self.subsession_model_anchor.probe())
                        .when_some(sub.provider,|d,kind|d.child(provider_icon(kind.id(),14.))).child(sub.model_label.clone())
                        .on_click(cx.listener(|this,_,w,cx|this.open_subsession_model_picker(w,cx))).into_any_element())
                    .border_t_1().border_color(content(0.04)))
                .when(!sub.efforts.is_empty(),|d|d.child(setting("Effort",None,
                    plain(SharedString::from("mono-subsession-effort"),sub.effort_label.clone()).child(self.subsession_effort_anchor.probe())
                        .on_click(cx.listener(|this,_,w,cx|this.open_subsession_effort_picker(w,cx))).into_any_element())
                    .border_t_1().border_color(content(0.04)))))
            .child(heading("Heartbeat",Some("When it has been quiet for a while, it looks over its recent work, sessions you left, your notes and inbox, and suggests what to do next. It never acts without asking.")))
            .child(card()
                .child(switch_setting("Check in on its own","Quiet check-ins leave nothing behind; you only hear from it when it has an idea.",false,
                    self.switch("mono-heartbeat",mono.heartbeat_minutes>0,false,|m,on|m.heartbeat_minutes=if on {heartbeat::DEFAULT_MINUTES} else {0},cx)))
                .child(setting("After it has been quiet for",None,self.heartbeat_intervals(mono,cx))
                    .when(mono.heartbeat_minutes==0,|d|d.opacity(0.4)).border_t_1().border_color(content(0.04))))
            .child(heading("Conversation",None))
            .child(card().child(setting("Reset conversation",Some("Clear all messages and start fresh. Its soul, memory and habits stay."),
                div().id("mono-reset").role(Role::Button).aria_label("Reset").flex().items_center().px_3().py_1p5().rounded_md().text_size(px(12.)).border_1().border_color(content(0.04)).text_color(rgb(0xff6467)).hover(|s|s.bg(rgb(0xfb2c36).opacity(0.1))).child("Reset")
                    .on_click(cx.listener(move |this,_,w,cx|this.reset_conversation(&reset,w,cx))).into_any_element())))
            .into_any_element()
    }

    pub(super) fn heartbeat_intervals(&self, mono: &Mono, cx: &mut Context<Self>) -> AnyElement {
        let enabled = mono.heartbeat_minutes > 0;
        div()
            .flex()
            .gap_1()
            .children(heartbeat::INTERVALS.iter().map(|&minutes| {
                let selected = enabled && mono.heartbeat_minutes == minutes;
                let label = if minutes < 60 {
                    format!("{minutes}m")
                } else {
                    format!("{}h", minutes / 60)
                };
                div()
                    .id(("mono-heartbeat-interval", minutes as usize))
                    .role(Role::Button)
                    .aria_label(label.clone())
                    .aria_selected(selected)
                    .px_2()
                    .py_0p5()
                    .rounded_md()
                    .text_size(px(12.))
                    .border_1()
                    .border_color(content(if selected { 0.2 } else { 0.06 }))
                    .when(selected, |d| d.bg(content(0.08)).text_color(content(1.)))
                    .when(!selected, |d| d.text_color(content(0.55)))
                    .when(enabled, |d| d.hover(|s| s.bg(content(0.05))))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(id) = this.active.clone()
                            && let Some(m) = this.roster.get_mut(&id)
                            && m.heartbeat_minutes > 0
                        {
                            m.heartbeat_minutes = minutes;
                            this.save(cx);
                            cx.notify();
                        }
                    }))
            }))
            .into_any_element()
    }

    pub(super) fn switch(
        &self,
        id: &'static str,
        on: bool,
        disabled: bool,
        change: fn(&mut Mono, bool),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(id)
            .role(Role::Switch)
            .aria_label(id)
            .aria_selected(on)
            .w(px(36.))
            .h(px(20.))
            .flex_shrink_0()
            .rounded_full()
            .bg(if on { accent_color() } else { content(0.2) })
            .child(
                div()
                    .size(px(16.))
                    .mt(px(2.))
                    .ml(px(if on { 18. } else { 2. }))
                    .rounded_full()
                    .bg(gpui::white()),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                if disabled {
                    return;
                }
                if let Some(id) = this.active.clone() {
                    if let Some(m) = this.roster.get_mut(&id) {
                        change(m, !on);
                    }
                    this.configure_chat(&id, cx);
                    this.save(cx);
                }
            }))
            .into_any_element()
    }

    pub(super) fn open_background(&mut self, id: &str, window: &Window, cx: &mut Context<Self>) {
        let Some(mono) = self.roster.get(id).cloned() else {
            return;
        };
        let view = cx.new(|cx| {
            BackgroundDialog::new(
                format!("mono:{id}"),
                ProjectAppearance {
                    name: mono.label(),
                    color: Some(mono.color),
                    mascot: Some(mono.mascot),
                    background: mono.background,
                    ..Default::default()
                },
                cx,
            )
        });
        let id = id.to_string();
        let events = cx.subscribe(
            &view,
            move |this, _, event: &BackgroundEvent, cx| match event {
                BackgroundEvent::Changed(appearance) => {
                    if let Some(mono) = this.roster.get_mut(&id) {
                        mono.background = appearance.background.clone();
                    }
                    this.save(cx);
                }
                BackgroundEvent::Close => {
                    this.background = None;
                    cx.notify();
                }
            },
        );
        let width = window.viewport_size().width.as_f32();
        let at = gpui::point(px((width - 640.) / 2.), px(50.));
        self.background = Some((view, events, at));
        cx.notify();
    }

    pub(in crate::monos) fn background_modal(&self) -> Option<AnyElement> {
        let (view, _, at) = self.background.as_ref()?;
        Some(
            popover("mono-background", Anchor::Point(*at))
                .modal_surface(false)
                .without_animation()
                .priority(100)
                .width(640.)
                .child(view.clone())
                .into_any_element(),
        )
    }
}
