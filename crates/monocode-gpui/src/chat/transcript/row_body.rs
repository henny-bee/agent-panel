use super::*;

impl TranscriptView {
    pub(super) fn row_body(
        &mut self,
        ix: usize,
        animate: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let row = self.rows[ix].clone();
        let prompt_width = self.split_prompt_width(ix, window);
        let message = &self.messages[row.message];
        let user = message.role == "user";
        let stacked = user && self.mono && self.stacks_on_next(message.start);
        let message_turn = message.turn;
        let error = message.role == "error";
        let reasoning = message.role == "reasoning";
        let message_start = message.start;
        let first = row.first;
        let last = row.last;
        // The final answer is under the receipt even while its expanded work
        // is leaving. Its padding must not change when those rows are removed.
        let after_receipt = message.under_line
            || (ix > 0 && matches!(self.rows[ix - 1].body, Body::WorkLine { .. }));
        let first_prose = row.key.1 == 16;
        let nested_phase = row.in_phase();
        let thought_header = matches!(row.body, Body::Reasoning);
        let attachments = message.attachments.clone();
        // A Mono's chat shows sent images above the bubble, like photos in a
        // messaging app; only other files stay inside it.
        let (photos, bubble_files) = if user && first && self.mono {
            attachments::split_photos(&attachments)
        } else {
            (Vec::new(), attachments.to_vec())
        };
        let photo_body = (!photos.is_empty())
            .then(|| self.photo_row(&photos, &format!("sent-{message_start}"), cx));
        let attachment_body = (user && first && !bubble_files.is_empty())
            .then(|| self.attachment_row(&bubble_files, &format!("sent-{message_start}"), cx));
        let step_opacity = self
            .motions
            .get(&row.key)
            .filter(|m| m.step)
            .map_or(1., |m| m.value(self.frame_now).1);
        let is_tool = matches!(
            row.body,
            Body::Tool { .. } | Body::Phase { .. } | Body::Reasoning
        );
        let body = match row.body {
            Body::GitAction(card) => self.git_card_row(&card, message_start, cx),
            Body::RemotePlan(plan) => self.remote_plan_card(&plan, cx),
            Body::Visual(document) => self.visual_row(message_start, document, cx),
            Body::Interjection(ref note) => {
                let open = self.expanded.contains(&message_start);
                let long = note.text.lines().count() > 4 || note.text.len() > 300;
                let severity = note.severity.as_deref();
                let color = match severity {
                    Some("blocker") => rgb(0xf87171).into(),
                    Some("concern") => rgb(0xfbbf24).into(),
                    _ => content(0.55),
                };
                div()
                    .w_full()
                    .min_w_0()
                    .py_4()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(div().h(px(1.)).flex_1().bg(content(0.12)))
                            .child(
                                div()
                                    .id(("interjection-label", message_start))
                                    .role(Role::Group)
                                    .aria_label(format!("Interjection: {}", note.label()))
                                    .text_size(px(12.))
                                    .text_color(content(0.55))
                                    .child(note.label().to_owned()),
                            )
                            .when_some(severity, |d, severity| {
                                let label = match severity {
                                    "blocker" => "Blocker",
                                    "concern" => "Concern",
                                    "nit" => "Nit",
                                    value => value,
                                };
                                d.child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(color)
                                        .child(label.to_owned()),
                                )
                            })
                            .child(div().h(px(1.)).flex_1().bg(content(0.12))),
                    )
                    .when(!note.text.is_empty(), |d| {
                        d.child(
                            div()
                                .w_full()
                                .text_size(px(12.5))
                                .line_height(px(20.))
                                .text_color(content(0.7))
                                .when(!open, |d| d.max_h(px(80.)).overflow_hidden())
                                .child(note.text.clone()),
                        )
                    })
                    .when(long, |d| {
                        d.child(
                            div()
                                .id(("interjection-expand", message_start))
                                .role(Role::Button)
                                .aria_expanded(open)
                                .text_size(px(11.))
                                .text_color(content(0.5))
                                .child(if open { "Show less" } else { "Show more" })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.toggle(message_start, cx);
                                })),
                        )
                    })
                    .into_any_element()
            }
            Body::MonoCard(ref value) => {
                let chosen = self.reply_after(row.message);
                crate::monos::cards::render(value, chosen.as_deref(), cx)
            }
            Body::MonoDivider => mono_divider::render(),
            Body::Pending(_) | Body::Launch(_) | Body::SavedDraft => div().into_any_element(),
            Body::Actions { turn, complete } => {
                // Tighten only a visible Mono reply followed by a user bubble;
                // stacked user messages keep their existing spacing.
                let before_user = self.mono
                    && ix > 0
                    && matches!(self.rows[ix - 1].body, Body::Markdown(_))
                    && self.messages[self.rows[ix - 1].message].role == "assistant"
                    && self.rows.get(ix + 1).is_some_and(|next| {
                        matches!(next.body, Body::Pending(_))
                            || (matches!(next.body, Body::Plain(_))
                                && self.messages[next.message].role == "user")
                    });
                self.response_actions(turn, complete, before_user, cx)
            }
            Body::WorkLine { turn, .. } if self.mono => self.mono_turn_header(turn, cx),
            Body::WorkLine { turn, expandable } => {
                let live = self.streaming
                    && message_start >= self.live_start
                    && self.source[message_start..].iter().all(|r| !r.is_prompt());
                let elapsed = if live {
                    self.started.map(|at| at.elapsed())
                } else {
                    self.elapsed.get(&turn).copied().or_else(|| {
                        self.turns
                            .get(&turn)
                            .filter(|info| info.has_duration())
                            .map(|info| std::time::Duration::from_millis(info.duration_ms))
                    })
                };
                let model = self
                    .turns
                    .get(&turn)
                    .map(|info| info.model.trim())
                    .filter(|model| !model.is_empty())
                    .or_else(|| live.then_some(self.model.as_str()));
                let title = work_title(
                    model,
                    elapsed.map(|duration| duration.as_millis() as u64),
                    live,
                );
                let open = self
                    .work_folds
                    .get(&turn)
                    .copied()
                    .unwrap_or(!self.fold_work);
                let label = if expandable {
                    if open {
                        "Hide the work".into()
                    } else {
                        "Show the work".into()
                    }
                } else {
                    title.clone()
                };
                let provider: SharedString = self
                    .turns
                    .get(&turn)
                    .map(|info| info.provider.as_str())
                    .filter(|provider| !provider.is_empty())
                    .unwrap_or(self.provider.as_str())
                    .to_owned()
                    .into();
                let work_tile = self
                    .work_tiles
                    .entry(turn)
                    .or_insert_with(|| {
                        cx.new(|_| crate::chat::transcript::phase_tile::PhaseTile::new(turn))
                    })
                    .clone();
                // Only a fold that opens shows the chevron; a header that just
                // labels unfolded work keeps the logo.
                work_tile.update(cx, |tile, cx| {
                    tile.set(
                        crate::chat::transcript::phase_tile::Mark::Provider(provider),
                        false,
                        open && expandable,
                        if open { 1. } else { 0. },
                        cx,
                    )
                });
                div()
                    .id(("work-fold", turn))
                    .role(if expandable {
                        Role::Button
                    } else {
                        Role::Label
                    })
                    .aria_label(label)
                    .aria_expanded(open)
                    .group("work-fold")
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .py_1()
                    .text_size(px(14.))
                    .line_height(px(20.))
                    .text_color(content(0.5))
                    // The provider's tile; an open fold turns it into the
                    // chevron, as a tool group's tile does.
                    .child(
                        work_tile.clone().cached(
                            StyleRefinement::default()
                                .w(px(crate::ui::tabs::tile::ACTIVITY_TILE.w))
                                .h(px(crate::ui::tabs::tile::ACTIVITY_TILE.h))
                                .flex_shrink_0(),
                        ),
                    )
                    .when(expandable, |d| {
                        let tile = work_tile.clone();
                        d.on_hover(move |hovered: &bool, _, cx| {
                            tile.update(cx, |tile, cx| tile.hover(*hovered, cx));
                        })
                    })
                    .child(div().min_w_0().flex_1().truncate().child(
                        if live && !cx.reduce_motion() {
                            crate::chat::transcript::activity::shimmer(
                                title,
                                self.started.map_or(0., |s| s.elapsed().as_secs_f32()),
                            )
                        } else {
                            div().child(title).into_any_element()
                        },
                    ))
                    .when(expandable, |d| d.hover(|s| s.text_color(content(0.8))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if expandable {
                            this.work_folds.insert(turn, !open);
                            this.list.pause_following_tail();
                            this.rebuild(turn, cx);
                        }
                    }))
                    .into_any_element()
            }
            Body::Phase {
                title,
                group,
                icon: glyph,
                active,
            } => {
                let open = self
                    .phases
                    .get(&group)
                    .copied()
                    .unwrap_or(self.phase_opens(active));
                let rotation = self
                    .motions
                    .values()
                    .find(|m| m.group == Some(group))
                    .map_or(if open { 1. } else { 0. }, |m| m.rotation(self.frame_now));
                let phase_tile = self
                    .phase_tiles
                    .entry(group)
                    .or_insert_with(|| {
                        cx.new(|_| crate::chat::transcript::phase_tile::PhaseTile::new(group))
                    })
                    .clone();
                let spinning = active && !cx.reduce_motion();
                // A running phase keeps its spinner unless pointed at.
                phase_tile.update(cx, |tile, cx| {
                    tile.set(
                        crate::chat::transcript::phase_tile::Mark::Work(glyph),
                        spinning,
                        open && !active,
                        rotation,
                        cx,
                    )
                });
                div()
                    .id(("phase", group))
                    .role(Role::Button)
                    .aria_label(format!(
                        "{} the steps for {title}",
                        if open { "Hide" } else { "Show" }
                    ))
                    .aria_expanded(open)
                    .group("activity-phase")
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .py_1()
                    .text_size(px(14.))
                    .line_height(px(20.))
                    .text_color(content(0.5))
                    // The tile animates itself; the transcript only tells it
                    // what the phase is doing.
                    .child(
                        phase_tile.clone().cached(
                            StyleRefinement::default()
                                .w(px(crate::ui::tabs::tile::ACTIVITY_TILE.w))
                                .h(px(crate::ui::tabs::tile::ACTIVITY_TILE.h))
                                .flex_shrink_0(),
                        ),
                    )
                    .child(div().min_w_0().flex_1().truncate().child(
                        if active && !cx.reduce_motion() {
                            crate::chat::transcript::activity::shimmer(
                                title,
                                self.started.map_or(0., |s| s.elapsed().as_secs_f32()),
                            )
                        } else {
                            div().child(title).into_any_element()
                        },
                    ))
                    .hover(|s| s.text_color(content(0.8)))
                    .on_hover({
                        let tile = phase_tile.clone();
                        move |hovered: &bool, _, cx| {
                            tile.update(cx, |tile, cx| tile.hover(*hovered, cx));
                        }
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_phase(group, !open, cx);
                    }))
                    .into_any_element()
            }
            Body::Tool {
                call,
                group,
                tail,
                live,
            } => {
                // A lone step leads with the spinner its phase header shows once
                // a second step joins it. It keeps spinning after it settles,
                // until whatever comes next arrives, so the hand-off to the
                // group header never drops the loader.
                let tiled = group.is_none() && !call.is_milestone() && call.panel().is_none();
                let loading =
                    tiled && (call.state == ToolState::Running || live) && !cx.reduce_motion();
                div()
                    .relative()
                    .w_full()
                    .when(group.is_some(), |d| {
                        d.pl(px(crate::chat::transcript::activity::TILE_RAIL_INDENT))
                            .child(crate::chat::transcript::activity::tile_rail(tail))
                    })
                    // A lone step leads with a tile like a group's, coloured by
                    // the kind of work; while it runs, its spinner sits there.
                    .when(tiled, |d| {
                        let tile = crate::ui::tabs::tile::ACTIVITY_TILE;
                        let lead = if loading {
                            crate::ui::tabs::tile::mark_tile(
                                &tile,
                                crate::ui::tabs::tile::Face::Neutral,
                                crate::ui::session_mark::spinner(
                                    ("tool-loader", message_start),
                                    crate::ui::motion::pulse::Rate::Half,
                                ),
                            )
                        } else {
                            let glyph = crate::chat::transcript::activity::summary(
                                std::slice::from_ref(&*call),
                                false,
                            )
                            .1;
                            let (face, mark, _) = crate::ui::tabs::tile::activity_face(glyph);
                            crate::ui::tabs::tile::mark_tile(&tile, face, mark)
                        };
                        d.flex().items_start().gap(px(8.)).child(
                            div()
                                .h(px(28.))
                                .flex_shrink_0()
                                .opacity(step_opacity)
                                .flex()
                                .items_center()
                                .child(lead),
                        )
                    })
                    .child(
                        div()
                            .when(tiled, |d| d.min_w_0().flex_1())
                            .opacity(step_opacity)
                            .child(self.tool(*call, message_start, tiled, cx)),
                    )
                    .into_any_element()
            }
            Body::Reasoning => {
                let summary = self.messages[row.message]
                    .parser
                    .tree()
                    .blocks
                    .iter()
                    .flat_map(|block| markdown::rows(&block.block))
                    .map(|row| markdown::row_text(&row))
                    .find(|text| !text.trim().is_empty())
                    .unwrap_or_else(|| "Thinking…".into())
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                let open = self.expanded.contains(&message_start);
                div()
                    .id(("thinking", message_start))
                    .debug_selector(move || format!("thinking-{message_start}"))
                    .role(Role::Button)
                    .aria_label(format!(
                        "{} thinking: {summary}",
                        if open { "Hide" } else { "Show" }
                    ))
                    .aria_expanded(open)
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .py_1()
                    .line_height(px(20.))
                    .text_size(px(14.))
                    .text_color(content(0.5))
                    .hover(|s| s.text_color(content(0.8)))
                    .when(!nested_phase, |d| d.child(icon("minus", 14.)))
                    .child(div().min_w_0().flex_1().truncate().child(if open {
                        "Thinking".into()
                    } else {
                        summary
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(message_start, cx)))
                    .into_any_element()
            }
            Body::Markdown(ref md) => {
                let prepared = self.markdown_cache.get(row.key, md);
                let md = &prepared.row;
                let body = if let Part::Code { fence, lines } = &md.part {
                    self.code(fence.clone(), lines.clone(), ix, cx)
                } else if let Part::Visual { id, caption } = &md.part {
                    self.placed_visual(id, caption, cx)
                } else {
                    markdown::selection::body_selected(
                        md,
                        format!("md-{ix}").into(),
                        if reasoning {
                            markdown::Tone::Reasoning
                        } else if row.work {
                            markdown::Tone::Work
                        } else {
                            markdown::Tone::Normal
                        },
                        self.reveals.get(&message_start).map(|r| &r.fades),
                        self.frame_now,
                        if reasoning {
                            None
                        } else {
                            self.text_selection(row.key, message_start, !user && !error, cx)
                        },
                        Some(&prepared),
                    )
                    .into_any_element()
                };
                div()
                    .when(self.mono, |d| {
                        let key = row.key;
                        d.debug_selector(move || format!("mono-prose-{}-{}", key.0, key.1))
                    })
                    .w_full()
                    .flex()
                    .pt(px(if first_prose {
                        if after_receipt || row.work { 4. } else { 12. }
                    } else if row.work {
                        md.gap.min(12.)
                    } else {
                        md.gap
                    }))
                    .pl(px(md.indent.saturating_sub(1) as f32 * 24.))
                    .when(md.quote > 0, |d| {
                        d.ml(px(4.)).pl_4().border_l_4().border_color(content(0.2))
                    })
                    .when(md.indent > 0, |d| {
                        d.child(
                            div()
                                .w(px(24.))
                                .flex_shrink_0()
                                .text_color(content(0.78))
                                .child(md.marker.clone().unwrap_or_default()),
                        )
                    })
                    .child(body)
                    .into_any_element()
            }
            Body::Plain(text)
                if user
                    && self.mono
                    && first
                    && last
                    && attachments.is_empty()
                    && is_emoji_only(&text) =>
            {
                // In a Mono's chat a few emoji stand alone: no bubble or actions.
                div()
                    .relative()
                    .text_size(px(60.))
                    .line_height(px(60.))
                    .child(text.trim().to_owned())
                    .children(self.reaction_badge(message_start))
                    .into_any_element()
            }
            Body::Plain(text) => {
                let burst = self.burst.filter(|burst| {
                    user && first && burst.prompt == message_start && burst.active(self.frame_now)
                });
                let now = self.frame_now;
                // UserLinkPreview: the prompt's first link, drawn in its chunk.
                let message_link = self.messages[row.message].user_link.clone();
                let link = message_link
                    .as_ref()
                    .filter(|(chunk, _)| user && *chunk + 16 == row.key.1)
                    .map(|(_, link)| link.clone());
                let collapsed = !self.expanded_prompts.contains(&message_start);
                let clamp = user && first && collapsed && message_link.is_none();
                // Images sent on their own need no bubble beneath them.
                let bare = photo_body.is_some() && text.is_empty() && attachment_body.is_none();
                // `overflows`: hidden chunks, or more than four wrapped lines.
                // React never clamps a message that shows a link preview.
                let overflows = user
                    && message_link.is_none()
                    && last
                    && (!first
                        || self.clamped_prompts.contains(&message_start)
                        || self.prompt_lines(&text, window) > 4);
                let bubble = div()
                    .when(photo_body.is_none(), |d| d.w_full())
                    .relative()
                    .flex()
                    .flex_col()
                    .text_color(if error {
                        rgb(0xf87171).into()
                    } else {
                        content(1.)
                    })
                    .when(user && !bare, |d| {
                        // AgentTranscript.tsx: text-sm (14px / 20px), px-3 py-2.
                        // Measurement shapes this font at the same size.
                        d.font(gpui::font(crate::ui::fonts::UI))
                            .text_size(PROMPT_TEXT_SIZE)
                            .line_height(px(20.))
                            .px(PROMPT_PAD_X * 0.5)
                            .pt(px(if first { 8. } else { 0. }))
                            .pb(px(if last { 8. } else { 0. }))
                            .child(
                                canvas(
                                    |_, _, _| {},
                                    move |bounds, _, window, cx| {
                                        // Match React's measured single-line bubble, including
                                        // when a window resize makes the same text wrap.
                                        let radius =
                                            if first && last && bounds.size.height <= px(37.) {
                                                f32::from(bounds.size.height) * 0.5
                                            } else {
                                                12.
                                            };
                                        let corners = gpui::Corners {
                                            top_left: px(if first { radius } else { 0. }),
                                            top_right: px(if first { radius } else { 0. }),
                                            bottom_left: px(if last { radius } else { 0. }),
                                            bottom_right: px(if last { radius } else { 0. }),
                                        };
                                        window.paint_quad(
                                            gpui::fill(bounds, user_bubble_fill())
                                                .corner_radii(corners),
                                        );
                                        if let Some(burst) = burst.filter(burst::Burst::behind_text)
                                        {
                                            burst.paint(bounds, corners, now, window, cx);
                                        }
                                    },
                                )
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full(),
                            )
                    })
                    .children(
                        attachment_body
                            .map(|body| div().when(!text.is_empty(), |d| d.mb_2()).child(body)),
                    )
                    .when(!text.is_empty(), |d| {
                        // A commit pill's message leads with that action's icon.
                        let action_icon = (user && first && last)
                            .then(|| {
                                crate::chat::conversation::git_actions::GitAction::message_icon(
                                    &text,
                                )
                            })
                            .flatten();
                        d.when_some(action_icon, |d, path| {
                            d.flex_row()
                                .items_center()
                                .gap(px(6.))
                                .child(icon(path, 14.).flex_shrink_0().text_color(content(0.6)))
                        })
                        .when_some(link.clone(), |d, link| {
                            d.child(self.user_link_text(&text, &link, row.key, message_start, cx))
                        })
                        .when(link.is_none(), |d| {
                            d.child(
                                div()
                                    .when(clamp, |d| d.line_clamp(4).text_ellipsis())
                                    .child({
                                        let styled = gpui::StyledText::new(text.clone())
                                            .with_highlights(markdown::mark_styles(
                                                &text,
                                                vec![],
                                                self.search_match
                                                    .as_ref()
                                                    .map_or("", |(_, q)| q.as_str()),
                                                self.search_match.as_ref().is_some_and(
                                                    |(start, _)| *start == message_start,
                                                ),
                                            ));
                                        let layout = styled.layout().clone();
                                        let child = styled.into_any_element();
                                        if let Some(selection) =
                                            self.text_selection(row.key, message_start, false, cx)
                                        {
                                            selection.wrap(text.clone(), layout, child)
                                        } else {
                                            child
                                        }
                                    }),
                            )
                        })
                    })
                    .children(burst.filter(|burst| !burst.behind_text()).map(|burst| {
                        let now = self.frame_now;
                        canvas(
                            |_, _, _| {},
                            move |bounds, _, window, cx| {
                                // The same corners the bubble's fill paints.
                                let radius = if last && bounds.size.height <= px(37.) {
                                    f32::from(bounds.size.height) * 0.5
                                } else {
                                    12.
                                };
                                let bottom = px(if last { radius } else { 0. });
                                let corners = gpui::Corners {
                                    top_left: px(radius),
                                    top_right: px(radius),
                                    bottom_left: bottom,
                                    bottom_right: bottom,
                                };
                                burst.paint(bounds, corners, now, window, cx);
                            },
                        )
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                    }))
                    .when(overflows, |d| {
                        d.child(
                            div().flex().child(
                                div()
                                    .id(("prompt-more", message_start))
                                    .role(Role::Button)
                                    .aria_expanded(!collapsed)
                                    .mt_1()
                                    .px_1()
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .text_size(px(12.))
                                    .line_height(px(16.))
                                    .text_color(content(0.6))
                                    .hover(|s| s.bg(content(0.08)).text_color(content(1.)))
                                    .child(if collapsed { "Show more" } else { "Show less" })
                                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.toggle_prompt(message_start, cx)
                                    })),
                            ),
                        )
                    })
                    .when(user && self.mono && first, |d| {
                        d.children(self.reaction_badge(message_start))
                    })
                    .when(
                        user && last
                            && !stacked
                            && (self.hovered_prompt == Some(message_start)
                                || self.hovered_prompt_actions == Some(message_start)),
                        |d| d.child(self.prompt_actions(message_start, cx)),
                    );
                match photo_body {
                    // Photos and their bubble enter as one message.
                    Some(photos) => div()
                        .min_w_0()
                        .max_w_full()
                        .flex()
                        .flex_col()
                        .items_end()
                        .gap_1()
                        .child(photos)
                        .child(bubble)
                        .into_any_element(),
                    None => bubble.into_any_element(),
                }
            }
        };
        let body = if let Some((_, tail)) = row.thought_phase {
            div()
                .relative()
                .w_full()
                .pl(px(crate::chat::transcript::activity::TILE_RAIL_INDENT))
                .when(!tail || thought_header, |d| {
                    d.child(crate::chat::transcript::activity::tile_rail_branch(
                        tail,
                        thought_header,
                    ))
                })
                .child(body)
                .into_any_element()
        } else {
            body
        };
        let key = row.key;
        let receipt = matches!(self.rows[ix].body, Body::WorkLine { .. });
        let inside_tools = row.thought_phase.is_some_and(|(_, tail)| !tail || !last)
            || matches!(
                self.rows[ix].body,
                Body::Tool {
                    group: Some(_),
                    tail: false,
                    ..
                }
            );
        // Unfolded work that opens with a tool group: the header's tile
        // connects down into that group's tile.
        let leads_into_group = !self.fold_work
            && matches!(self.rows[ix].body, Body::WorkLine { .. })
            && matches!(
                self.rows.get(ix + 1).map(|r| &r.body),
                Some(Body::Phase { .. })
            );
        let receipt_open = match self.rows[ix].body {
            // Only folding work hangs it from the header on a rail.
            Body::WorkLine { turn, .. } if self.fold_work => {
                self.work_folds.get(&turn).copied().unwrap_or(false)
            }
            _ => leads_into_group,
        };
        // The other end of that connector: from the row's top into the tile.
        let joined_to_header = !self.fold_work
            && matches!(self.rows[ix].body, Body::Phase { .. })
            && ix > 0
            && matches!(self.rows[ix - 1].body, Body::WorkLine { .. });
        let prose = matches!(self.rows[ix].body, Body::Markdown(..));
        let rail_branch = matches!(self.rows[ix].body, Body::Phase { .. })
            || (!nested_phase && (thought_header || (prose && first_prose)))
            || matches!(self.rows[ix].body, Body::Tool { group: None, .. });
        // Keep a phase header the same height whether its body is present or
        // leaving. The last tool carries the group's animated bottom padding.
        let card = matches!(self.rows[ix].body, Body::MonoCard(_));
        let (card_top, card_bottom) = if card && self.mono {
            self.mono_card_spacing(ix)
        } else if self
            .rows
            .get(ix + 1)
            .is_some_and(|r| matches!(r.body, Body::MonoCard(_) | Body::Launch(_)))
        {
            (4., 8.)
        } else {
            (4., 12.)
        };
        let bottom = if card {
            card_bottom
        } else if inside_tools || matches!(self.rows[ix].body, Body::Phase { .. }) {
            0.
        } else if receipt || (last && is_tool) {
            4.
        } else if last {
            // A prompt sits between two turns: match the room above it, so it
            // doesn't read as part of the reply below.
            8.
        } else {
            0.
        };
        let natural = div()
            .relative()
            .w_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .children(if first {
                self.mono_separator(message_start)
            } else {
                None
            })
            .children(if first {
                self.provider_separator(message_start)
            } else {
                None
            })
            .child(
                div()
                    .id(("row", ix))
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(COLUMN))
                            .min_w_0()
                            .px(px(COLUMN_PAD))
                            .pt(px(if card && self.mono {
                                card_top
                            } else if first && !is_tool && !prose {
                                if user {
                                    4.
                                } else if receipt && !self.mono {
                                    12.
                                } else {
                                    0.
                                }
                            } else {
                                0.
                            }))
                            .pb(px(if row.work {
                                0.
                            } else if self.mono && user {
                                // `pt-1 pb-5`, or `pb-0` when the next message stacks on it.
                                if last && !stacked { 20. } else { 0. }
                            } else if self.mono && receipt {
                                0.
                            } else {
                                bottom
                            }))
                            .text_size(px(14.))
                            .line_height(px(24.))
                            .when(user && self.chat_layout, |d| {
                                d.pl(px(56.)).flex().justify_end()
                            })
                            .when(user && self.mono, |d| d.pr(px(COLUMN_PAD)))
                            .child(
                                div()
                                    .relative()
                                    .min_w_0()
                                    .when(user && self.chat_layout, |d| d.max_w(PROMPT_BUBBLE_MAX))
                                    .when_some(prompt_width, |d, width| d.w(width))
                                    .when(user && !self.chat_layout, |d| d.w_full())
                                    .when(!user, |d| d.w_full())
                                    .when(row.work, |d| {
                                        d.pl(px(
                                            crate::chat::transcript::activity::TILE_RAIL_INDENT,
                                        ))
                                        .pb(px(bottom))
                                        .when(
                                            !nested_phase || !row.work_tail,
                                            |d| {
                                                d.child(
                                                crate::chat::transcript::activity::tile_rail_branch(
                                                    row.work_tail,
                                                    rail_branch,
                                                ),
                                            )
                                            },
                                        )
                                    })
                                    .when(receipt_open, |d| {
                                        d.child(
                                            div()
                                                .absolute()
                                                // From under the header's tile,
                                                // down its centre.
                                                .left(px(
                                                    crate::chat::transcript::activity::TILE_RAIL_X,
                                                ))
                                                .top(
                                                    px(4. + crate::ui::tabs::tile::ACTIVITY_TILE.h),
                                                )
                                                .bottom(px(-4.))
                                                .w(px(1.))
                                                .bg(crate::ui::theme::rail_color()),
                                        )
                                    })
                                    .when(joined_to_header, |d| {
                                        // A tool group's tile sits 5px into its
                                        // row; the line meets it there.
                                        d.child(
                                            div()
                                                .absolute()
                                                .left(px(
                                                    crate::chat::transcript::activity::TILE_RAIL_X,
                                                ))
                                                .top_0()
                                                .h(px(5.))
                                                .w(px(1.))
                                                .bg(crate::ui::theme::rail_color()),
                                        )
                                    })
                                    .child(body),
                            ),
                    ),
            );
        let outer = div()
            .id(gpui::SharedString::from(format!(
                "motion-{}-{}",
                key.0, key.1
            )))
            .w_full()
            .flex()
            .flex_col();
        let mut opacity = 1.;
        let mut translate_y = 0.;
        let motion = self
            .motions
            .get(&key)
            .filter(|_| animate)
            .filter(|m| m.group.is_none_or(|group| !self.windowed_phase(group)))
            .map(|m| (m.value(self.frame_now), m.step));
        if let Some(((_, fade), step)) = motion {
            opacity = if step { 1. } else { fade };
        }
        if let Some((prompt, born)) = self.prompt_rise
            && message_start >= prompt
            && !cx.reduce_motion()
        {
            let elapsed = born.elapsed().as_secs_f32();
            if message_start == prompt {
                let rise =
                    crate::ui::motion::reveal::bezier((elapsed / 0.56).min(1.), 0.22, 1., 0.36, 1.);
                let fade =
                    crate::ui::motion::reveal::bezier((elapsed / 0.48).min(1.), 0., 0., 0.58, 1.);
                translate_y =
                    (f32::from(self.viewport.size.height) * 0.3 - TOP_INSET).max(0.) * (1. - rise);
                opacity *= fade;
            } else {
                let t = ((elapsed - 0.56) / 0.32).clamp(0., 1.);
                let reveal = crate::ui::motion::reveal::bezier(t, 0.22, 1., 0.36, 1.);
                translate_y = 10. * (1. - reveal);
                opacity *= reveal;
            }
        }
        if matches!(self.rows[ix].body, Body::Plain(_) | Body::Markdown(_))
            && let Some(born) = self.entrances.get(&message_start)
            && !cx.reduce_motion()
        {
            // The identity line and footer persist while only the message body
            // enters, matching React's data-chat-message boundary.
            // useBottomChatMotion's entrance: a sent message rises 16px over
            // 360ms, a reply 10px over 280ms.
            let (rise, duration) = if user { (16., 0.36) } else { (10., 0.28) };
            let t = crate::ui::motion::reveal::bezier(
                (born.elapsed().as_secs_f32() / duration).min(1.),
                0.22,
                1.,
                0.36,
                1.,
            );
            translate_y += rise * (1. - t);
            opacity *= t;
        }
        // ListState positions the layout root itself. Translate its child so
        // the motion survives prepaint_as_root without affecting row height.
        let natural = natural.relative().top(px(translate_y));
        let body = if let Some(((mut height, _), _)) = motion {
            if let Some(group) = self.motions[&key].group {
                height = self.phase_layouts[&group].row_progress(key, height);
            }
            Disclosure::new(natural, self.viewport.size.width, height).into_any_element()
        } else {
            natural.into_any_element()
        };
        outer
            .when(motion.is_some(), |d| d.overflow_hidden())
            .when(self.mono || user, |d| {
                d.on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    this.mono_hover(user, message_start, message_turn, key, *hovered, cx)
                }))
            })
            .opacity(opacity)
            .child(body)
            .into_any_element()
    }
}
