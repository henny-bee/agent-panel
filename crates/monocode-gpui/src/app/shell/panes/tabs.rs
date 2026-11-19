use super::super::*;

impl Shell {
    /// A pane's mark: its provider, spinning while it works and checked when
    /// it finished unseen, or the file's icon.
    pub(in crate::app::shell) fn pane_mark(&self, mark: &PaneMark, active: bool) -> AnyElement {
        let ink = content(if active { 1. } else { 0.5 });
        let mark = match mark {
            PaneMark::Session(provider, lit) => match lit.and_then(|id| self.tab_marks.get(&id)) {
                Some(mark) => mark
                    .clone()
                    .cached(StyleRefinement::default().size(px(14.)))
                    .into_any_element(),
                None => provider_icon(provider, 14.)
                    .opacity(if active { 1. } else { 0.55 })
                    .into_any_element(),
            },
            PaneMark::File(Some(name)) => {
                crate::ui::file_icons::file_type_icon(name, false, false, 14.)
            }
            PaneMark::File(None) => icon("git-compare", 14.).text_color(ink).into_any_element(),
            PaneMark::Terminal => icon("terminal", 14.).text_color(ink).into_any_element(),
            PaneMark::Mono(mascot, color) => div()
                .opacity(if active { 1. } else { 0.55 })
                .child(crate::ui::avatar::avatar(mascot, *color, "idle", 14., true))
                .into_any_element(),
        };
        div()
            .size(px(14.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .child(mark)
            .into_any_element()
    }

    /// The focused pane's mark and title on one line; while focus moves,
    /// the line leaving rolls out as the new one rolls in behind it.
    pub(in crate::app::shell) fn pane_headline(
        &self,
        copy: &TabCopy,
        (leaving, dir, progress): (Option<usize>, f32, f32),
        active: bool,
        hovered: bool,
    ) -> AnyElement {
        const LINE: f32 = 20.;
        let line = |pane: &TabPane, top: f32, opacity: f32| {
            div()
                .absolute()
                .left_0()
                .top(px(top))
                .w_full()
                .h(px(LINE))
                .opacity(opacity)
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(crate::ui::tabs::label::tab_label(pane.title.clone())),
                )
        };
        div()
            .flex_1()
            .min_w_0()
            .h(px(LINE))
            .relative()
            .overflow_hidden()
            .text_size(px(TITLE_TAB_TEXT_SIZE))
            .text_color(content(if active || hovered { 1. } else { 0.5 }))
            .group_hover("title-tab", |s| s.text_color(content(1.)))
            .when_some(copy.panes.get(copy.focus), |d, pane| {
                d.child(line(pane, dir * (1. - progress) * LINE, progress.max(0.2)))
            })
            .when_some(leaving.and_then(|ix| copy.panes.get(ix)), |d, pane| {
                d.child(line(pane, -dir * progress * LINE, 1. - progress))
            })
            .into_any_element()
    }

    /// Where tab `tab`'s headline is in its roll to the focused pane: the
    /// pane leaving, its direction, and the eased progress.
    /// Also the tiles' shuffle, linear: it runs longer than the title's
    /// roll so the card being put back can be seen lifting over the stack.
    pub(in crate::app::shell) fn pane_tick(
        &mut self,
        tab: i64,
        copy: &TabCopy,
        animate: bool,
    ) -> ((Option<usize>, f32, f32), f32) {
        let Some(shown) = copy.panes.get(copy.focus).map(|p| p.key) else {
            self.pane_ticks.remove(&tab);
            return ((None, 0., 1.), 1.);
        };
        let now = std::time::Instant::now();
        let position = |key: (i64, bool)| copy.panes.iter().position(|p| p.key == key);
        let tick = self.pane_ticks.entry(tab).or_insert(PaneTick {
            shown,
            from: None,
            dir: 0.,
            start: now,
        });
        if tick.shown != shown {
            let from = position(tick.shown).filter(|_| animate);
            tick.dir = if from.is_some_and(|ix| ix > copy.focus) {
                -1.
            } else {
                1.
            };
            tick.from = from.map(|_| tick.shown);
            tick.shown = shown;
            tick.start = now;
        }
        if !animate {
            tick.from = None;
        }
        let elapsed = now.saturating_duration_since(tick.start).as_secs_f32();
        let t = (elapsed / PANE_TICK_DURATION).min(1.);
        let shuffle = elapsed / TILE_SHUFFLE_DURATION;
        let from = tick.from.and_then(position).filter(|_| shuffle < 1.);
        if from.is_none() {
            tick.from = None;
            return ((None, 0., 1.), 1.);
        }
        (
            (
                from,
                tick.dir,
                crate::ui::motion::reveal::bezier(t, 0.22, 1., 0.36, 1.),
            ),
            shuffle,
        )
    }

    /// The focused pane's mark on a frosted tile, with the tab's other panes
    /// tucked behind it.
    /// `leaving` is the pane that had focus while `shuffle` (linear) runs
    /// the deck shuffle that deals the focused pane's tile to the front.
    pub(in crate::app::shell) fn pane_tile_stack(
        &self,
        copy: &TabCopy,
        leaving: Option<usize>,
        shuffle: f32,
    ) -> AnyElement {
        // The focused pane in front, the rest behind in layout order.
        // A tab with no panes yet (still loading, or closing) has no focus
        // to put in front, so an index past the end is skipped, never used.
        let depths = |focus: usize| {
            let count = copy.panes.len();
            let mut depth = vec![0; count];
            for (d, ix) in std::iter::once(focus)
                .filter(|ix| *ix < count)
                .chain((0..count).filter(|ix| *ix != focus))
                .enumerate()
            {
                depth[ix] = d;
            }
            depth
        };
        let to = depths(copy.focus);
        let from = leaving.map(depths).unwrap_or_else(|| to.clone());
        let tiles = copy
            .panes
            .iter()
            .enumerate()
            .map(|(ix, pane)| {
                // Only the front tile borrows a lit session's live mark: the
                // view can draw in one place at a time.
                let (face, mark) = self.pane_tile(&pane.mark, to[ix] == 0);
                crate::ui::tabs::tile::StackTile {
                    face,
                    mark,
                    from: from[ix],
                    to: to[ix],
                }
            })
            .collect();
        crate::ui::tabs::tile::tile_stack(
            &TAB_TILE,
            tiles,
            if leaving.is_some() { shuffle } else { 1. },
        )
        .debug_selector(|| "title-tab-tile-stack".into())
        .into_any_element()
    }

    /// A pane's tile: a session on its provider's colour with a white mark,
    /// or, while it works or waits to be seen, its live mark on a neutral
    /// tile; files and Monos neutral.
    pub(in crate::app::shell) fn pane_tile(
        &self,
        mark: &PaneMark,
        live: bool,
    ) -> (Face, AnyElement) {
        match mark {
            PaneMark::Session(provider, lit) => {
                match lit.filter(|_| live).and_then(|id| self.tab_marks.get(&id)) {
                    Some(view) => (
                        Face::Neutral,
                        view.clone()
                            .cached(StyleRefinement::default().size(px(14.)))
                            .into_any_element(),
                    ),
                    None => (Face::provider(provider), provider_glyph(provider, 14.)),
                }
            }
            PaneMark::File(Some(name)) => (
                Face::Neutral,
                crate::ui::file_icons::file_type_glyph(name, 14., crate::ui::theme::tile_ink()),
            ),
            PaneMark::File(None) => (
                Face::Neutral,
                icon("git-compare", 14.)
                    .text_color(crate::ui::theme::tile_ink())
                    .into_any_element(),
            ),
            PaneMark::Terminal => (
                Face::Neutral,
                icon("terminal", 14.)
                    .text_color(crate::ui::theme::tile_ink())
                    .into_any_element(),
            ),
            // A Mono keeps its mascot's own colours.
            mark => (Face::Neutral, self.pane_mark(mark, true)),
        }
    }

    pub(in crate::app::shell) fn tab_can_close(&self, id: i64, cx: &App) -> bool {
        let Some(tab) = self.workspace.tab_of(id) else {
            return false;
        };
        let layout = self.workspace.layout(tab);
        // Beside the pins, the strip's last tab counts as the only one.
        let tabs = if self.tab_pinned(&TitleTab::Session(tab)) {
            self.visible_tabs(cx)
        } else {
            self.strip_tabs(cx)
        };
        tabs.len() > 1
            || layout.is_split()
            || layout.leaf_ids().iter().any(|id| {
                self.session_summaries.contains_key(id) && {
                    let s = self.session_metadata(*id, cx);
                    s.summary.has_messages
                        || s.busy
                        || s.has_saved_draft()
                        || self
                            .sessions
                            .get(id)
                            .is_some_and(|v| !v.read(cx).input.read(cx).content.is_empty())
                } || self
                    .workspace
                    .reviews
                    .get(id)
                    .is_some_and(|r| !r.tabs.is_empty())
            })
    }

    /// The tab strip's width: between the navigation and TitleBar.tsx's right
    /// panel toggle, `pl-1.5 pr-3` clear of the edge.
    pub(in crate::app::shell) fn tabs_width(&self, window: &Window, cx: &App) -> f32 {
        let trailing = if self.workspace.boxy_layout() {
            self.right_drawn + self.strip_trail(cx)
        } else {
            self.toggle_room(cx)
        };
        // The classic strip starts beside the sidebar, after whatever the
        // navigation leaves uncovered.
        let lead = if self.workspace.boxy_layout() && !self.classic_titlebar_spans_window(cx) {
            self.left_sidebar_width(cx) + self.strip_lead(cx)
        } else {
            self.navigation_width()
        };
        (window.viewport_size().width.as_f32()
            - lead
            - trailing
            - (client_controls_width(window) - self.window_controls))
            .max(0.)
    }

    pub(in crate::app::shell) fn tabs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let strip_width = self.tabs_width(window, cx);
        let tabs = self.strip_tabs(cx);
        let count = tabs.len();
        let insert = self.pane_drag.as_ref().and_then(|d| d.insert);
        let reduced = cx.reduce_motion();
        let pane_motion = !reduced && !self.tab_drag.as_ref().is_some_and(|d| d.active);
        let animate = self.workspace.preferences.tab_animations && pane_motion;
        let (slots, opening) = self.title_tab_motion.sync(
            tabs.iter().map(|tab| (tab.clone(), ())).collect(),
            animate,
            std::time::Instant::now(),
        );
        // Pinned tabs leave the strip for the deck at its left edge; as it
        // fans out it takes more of the strip and the tabs give way.
        let closing: HashSet<TitleTab> = slots
            .iter()
            .filter_map(|s| match s {
                crate::ui::tabs::motion::Slot::Closing { key, .. } => Some(key.clone()),
                crate::ui::tabs::motion::Slot::Live { .. } => None,
            })
            .collect();
        let (dock, dock_width) = self
            .pinned_dock(&closing, window, cx)
            .map_or((None, 0.), |(dock, w)| (Some(dock), w));
        let viewport = (strip_width - dock_width).max(0.);
        // Every tab fits: they share the strip, however many there are. A tab
        // easing in or out holds its share of a slot, so the others give way
        // to it gradually, as flex-shrink does in TitleBar.tsx.
        let share: f32 = slots
            .iter()
            .map(|s| match s {
                crate::ui::tabs::motion::Slot::Live { open, .. }
                | crate::ui::tabs::motion::Slot::Closing { open, .. } => *open,
            })
            .sum();
        let avail = viewport - STRIP_PAD - STRIP_NEW_TAB - share * 2.;
        let width = if share <= 0. {
            TITLE_TAB_MAX_WIDTH
        } else {
            (avail / share).clamp(0., TITLE_TAB_MAX_WIDTH)
        };
        // The selected tab keeps a wider share; the others split the rest.
        let selected = self.active_title_tab(cx);
        let now = std::time::Instant::now();
        let selection_changed = self.title_tab_selection.1 != selected;
        if selection_changed {
            let previous = self.title_tab_selection.1.take();
            self.title_tab_selection = (previous.filter(|_| animate), selected.clone(), now);
        }
        let switch = crate::ui::tabs::motion::progress(self.title_tab_selection.2, now);
        if switch >= 1. {
            self.title_tab_selection.0 = None;
        }
        let leaving = self.title_tab_selection.0.clone();
        let switching = leaving.is_some();
        // How much of the selected tab's wider share each tab holds: the new
        // one eases in while the previous one eases out.
        let weight = |tab: &TitleTab| {
            if Some(tab) == selected.as_ref() {
                if switching { switch } else { 1. }
            } else if leaving.as_ref() == Some(tab) {
                1. - switch
            } else {
                0.
            }
        };
        let held: f32 = slots
            .iter()
            .map(|s| match s {
                crate::ui::tabs::motion::Slot::Live { key, open } => weight(key) * open,
                crate::ui::tabs::motion::Slot::Closing { .. } => 0.,
            })
            .sum();
        // The selected tab shrinks with the rest down to a floor, then holds
        // there while the others keep giving way.
        let (width, floor) = if width < SELECTED_TAB_MIN_WIDTH && held > 0. && share - held > 0. {
            let floor = SELECTED_TAB_MIN_WIDTH.min(avail.max(0.));
            (((avail - floor * held) / (share - held)).max(0.), floor)
        } else {
            (width, width)
        };
        // Fractional widths round differently from tab to tab and the error
        // adds up along the strip, so a full strip's new-tab button would
        // wander as tabs come and go. Every slot snaps to device pixels and
        // the button's margin takes what is left, keeping it still.
        let scale = window.scale_factor();
        let snap = |x: f32| (x * scale).floor() / scale;
        let crowded = width < TITLE_TAB_MAX_WIDTH;
        let base_width = |tab: &TitleTab| snap(width + (floor - width) * weight(tab));
        let slot_size = |tab: &TitleTab, open: f32| {
            (
                snap(base_width(tab) * open),
                snap(crate::ui::tabs::motion::GAP * (1. - open)),
            )
        };
        let used: f32 = slots
            .iter()
            .map(|s| match s {
                crate::ui::tabs::motion::Slot::Live { key, open }
                | crate::ui::tabs::motion::Slot::Closing { key, open, .. } => {
                    if *open >= 1. {
                        base_width(key) + 2.
                    } else {
                        let (w, give) = slot_size(key, *open);
                        w + 2. - give
                    }
                }
            })
            .sum();
        // The settled tabs take back what snapping left over, a device pixel
        // each from the left, so the new-tab button keeps its usual 2px gap.
        let mut extra: HashMap<TitleTab, f32> = HashMap::new();
        let mut new_tab_lead = 2.;
        if crowded {
            let leftover = (viewport - STRIP_PAD - STRIP_NEW_TAB - used).max(0.);
            let settled: Vec<&TitleTab> = slots
                .iter()
                .filter_map(|s| match s {
                    crate::ui::tabs::motion::Slot::Live { key, open } if *open >= 1. => Some(key),
                    _ => None,
                })
                .collect();
            let units = (leftover * scale).floor() as usize;
            if !settled.is_empty() {
                for i in 0..units {
                    *extra.entry(settled[i % settled.len()].clone()).or_default() += 1. / scale;
                }
                new_tab_lead += leftover - units as f32 / scale;
            } else {
                new_tab_lead += leftover;
            }
        }
        let tab_width = |tab: &TitleTab| base_width(tab) + extra.get(tab).copied().unwrap_or(0.);
        let dragged = self
            .tab_drag
            .as_ref()
            .filter(|d| d.active && !d.armed)
            .map(|d| d.id.clone());
        let mut moving = false;
        let shifts: Vec<Pixels> = tabs
            .clone()
            .into_iter()
            .enumerate()
            .map(|(ix, id)| {
                let target = self.tab_shift(ix).as_f32();
                let (value, active) =
                    self.eased_shift(id.clone(), target, dragged.as_ref() == Some(&id), reduced);
                moving |= active;
                px(value)
            })
            .collect();
        // Keep inactive headlines on their last pane. Only an in-tab focus
        // change rolls the title; selecting a tab settles it in the same frame.
        self.pane_ticks
            .retain(|id, _| self.workspace.tabs.contains(id));
        let mut copies = HashMap::new();
        for id in tabs.iter().filter_map(|tab| tab.session()) {
            if self
                .session_summaries
                .contains_key(&self.workspace.session_for_pane(id))
            {
                let copy = self.tab_copy(id, cx);
                // Moving focus inside a tab rolls its title and shuffles its
                // tiles whatever the open/close Tab animations setting says;
                // only Reduce Motion or a drag holds them still.
                let tick = self.pane_tick(
                    id,
                    &copy,
                    pane_motion && !selection_changed && selected == Some(TitleTab::Session(id)),
                );
                moving |= tick.0.0.is_some();
                copies.insert(id, (copy, tick));
            }
        }
        let mut session_elements: HashMap<i64, AnyElement> = tabs
            .iter()
            .enumerate()
            .filter_map(|(ix, tab)| {
                tab.session().and_then(|id| {
                    self.session_summaries
                        .contains_key(&self.workspace.session_for_pane(id))
                        .then_some((ix, id))
                })
            })
            .filter_map(|(ix, id)| {
                let (copy, ((leaving, dir, progress), shuffle)) = copies.remove(&id)?;
                let active = self.workspace.active_tab() == Some(id)
                    && !self.app_view_open()
                    && !self.mono_active(cx)
                    && !self.search_page();
                // The press itself redraws the strip to arm tab dragging.
                // Keep its ink stable until that press is released, even if
                // hover hitboxes are being replaced during that redraw.
                let hovered = self.tab_hover == Some(id)
                    || self
                        .tab_drag
                        .as_ref()
                        .is_some_and(|drag| drag.id == TitleTab::Session(id) && !drag.active);
                // A collapsed tab drops its close button, so a click anywhere
                // on it selects it; selected, it is wide enough to show it again.
                let close = self.tab_can_close(id, cx)
                    && (active || tab_width(&TitleTab::Session(id)) >= TAB_CLOSE_MIN_WIDTH);
                let dragging = self
                    .tab_drag
                    .as_ref()
                    .is_some_and(|d| d.id == TitleTab::Session(id) && d.active);
                // Any chat in the tab with a saved draft dashes its plate, as
                // the sidebar card does.
                let draft = self
                    .workspace
                    .layout(id)
                    .leaf_ids()
                    .into_iter()
                    .filter(|pane| self.session_summaries.contains_key(pane))
                    .any(|pane| self.session_metadata(pane, cx).has_saved_draft());
                let tab = div()
                    .id(("tab", id as usize))
                    .debug_selector(move || format!("title-tab-{id}"))
                    .group("title-tab")
                    .role(Role::Tab)
                    .aria_label(copy.headline.clone())
                    .aria_selected(active)
                    .cursor_pointer()
                    .w(px(tab_width(&TitleTab::Session(id))))
                    .flex_shrink_0()
                    .h(px(TITLE_BAR_HEIGHT))
                    .relative()
                    .left(shifts[ix])
                    .flex()
                    .items_center()
                    .child(
                        title_tab_plate(active, hovered)
                            .id(("tab-plate", id as usize))
                            .w_full()
                            .min_w_0()
                            // A narrow tab keeps its mark; the close button
                            // overlays it on hover instead of taking room.
                            .when(
                                close && tab_width(&TitleTab::Session(id)) >= TAB_CLOSE_MIN_WIDTH,
                                |d| d.pr_7(),
                            )
                            // Too narrow for the plate's padding: the mark sits centred.
                            .when(tab_width(&TitleTab::Session(id)) < NARROW_TAB, |d| {
                                d.px(px(
                                    ((tab_width(&TitleTab::Session(id)) - TAB_TILE.w) / 2.).max(0.)
                                ))
                            })
                            .gap(px(TAB_TILE_GAP))
                            .group_hover("title-tab", |s| s.bg(title_tab_fill(active, true)))
                            // Overlaid so the dashes take no room from the content.
                            .when(draft, |d| {
                                d.relative().child(
                                    div()
                                        .debug_selector(move || format!("title-tab-draft-{id}"))
                                        .absolute()
                                        .top_0()
                                        .left_0()
                                        .size_full()
                                        .rounded_md()
                                        .border_1()
                                        .border_dashed()
                                        .border_color(content(if active { 0.3 } else { 0.25 })),
                                )
                            })
                            .child(self.pane_tile_stack(&copy, leaving, shuffle))
                            .child(self.pane_headline(
                                &copy,
                                (leaving, dir, progress),
                                active,
                                hovered,
                            )),
                    )
                    .when(close, |d| {
                        d.child(
                            div()
                                .id(("close-tab", id as usize))
                                .debug_selector(move || format!("close-title-tab-{id}"))
                                .group("tab-close")
                                .role(Role::Button)
                                .aria_label(format!("Close tab {}", copy.headline))
                                .absolute()
                                .right_1()
                                .top(px(10.))
                                .size(px(20.))
                                .rounded_sm()
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(|s| s.bg(content(0.1)))
                                .when(copy.panes.len() > 1, |d| {
                                    d.child(
                                        div()
                                            .debug_selector(move || {
                                                format!("title-tab-pane-count-{id}")
                                            })
                                            .text_size(px(11.))
                                            .line_height(px(20.))
                                            .text_color(content(0.4))
                                            .group_hover("title-tab", |s| s.opacity(0.))
                                            .child(copy.panes.len().to_string()),
                                    )
                                })
                                .child(
                                    div()
                                        .absolute()
                                        .top_1()
                                        .left_1()
                                        .size(px(12.))
                                        .opacity(0.)
                                        .group_hover("title-tab", |s| s.opacity(1.))
                                        .child(icon("close", 12.).group_hover("tab-close", |s| {
                                            s.text_color(content(1.))
                                        })),
                                )
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.close(id, window, cx);
                                })),
                        )
                    })
                    .map(|d| {
                        let bar = || {
                            div()
                                .absolute()
                                .top(px(9.))
                                .h(px(22.))
                                .w(px(2.))
                                .rounded_full()
                                .bg(accent_color())
                        };
                        d.when(insert == Some(ix), |d| d.child(bar().left(px(-2.))))
                            .when(insert == Some(count) && ix + 1 == count, |d| {
                                d.child(bar().right(px(-2.)))
                            })
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event, _, cx| {
                            this.start_tab_drag(TitleTab::Session(id), event, cx)
                        }),
                    )
                    .on_hover(cx.listener(move |this, hovered, _, cx| {
                        if *hovered {
                            this.tab_hover = Some(id);
                        } else if this.tab_hover == Some(id) {
                            this.tab_hover = None;
                        }
                        this.tabs_frame.update(cx, |_, cx| cx.notify());
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            this.open_menu(
                                MenuTarget::Tab(id),
                                Anchor::Point(event.position),
                                window,
                                cx,
                            );
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Middle,
                        cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.close(id, window, cx);
                        }),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if std::mem::take(&mut this.skip_tab_click) {
                            return;
                        }
                        this.open_title_tab(&TitleTab::Session(id), window, cx);
                    }));
                let element = if dragging {
                    gpui::deferred(tab).with_priority(1).into_any()
                } else {
                    tab.into_any()
                };
                Some((id, element))
            })
            .collect();
        let elements: Vec<AnyElement> = tabs
            .iter()
            .enumerate()
            .filter_map(|(ix, tab)| match tab {
                TitleTab::Session(id) => session_elements.remove(id),
                TitleTab::App(view) => {
                    let element = self
                        .app_title_tab(*view, tab_width(tab), cx)
                        .left(shifts[ix]);
                    Some(if dragged.as_ref() == Some(tab) {
                        gpui::deferred(element).with_priority(1).into_any_element()
                    } else {
                        element.into_any_element()
                    })
                }
                TitleTab::Search => {
                    let element = self.search_title_tab(tab_width(tab), cx).left(shifts[ix]);
                    Some(if dragged.as_ref() == Some(tab) {
                        gpui::deferred(element).with_priority(1).into_any_element()
                    } else {
                        element.into_any_element()
                    })
                }
                TitleTab::Mono { id, .. } => {
                    let manager = self.monos.clone()?;
                    let w = tab_width(tab);
                    let element = manager.update(cx, |m, cx| m.title_tab(id, w, cx))?;
                    let drag_id = tab.clone();
                    let menu_id = id.clone();
                    let dragging = dragged.as_ref() == Some(tab);
                    let element = div()
                        .id(SharedString::from(format!("mono-tab-slot-{id}")))
                        .h(px(TITLE_BAR_HEIGHT))
                        .w(px(w))
                        .flex_shrink_0()
                        .relative()
                        .left(shifts[ix])
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event, _, cx| {
                                this.start_tab_drag(drag_id.clone(), event, cx);
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                this.open_menu(
                                    MenuTarget::MonoTab(menu_id.clone()),
                                    Anchor::Point(event.position),
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .child(element);
                    Some(if dragging {
                        gpui::deferred(element).with_priority(1).into_any()
                    } else {
                        element.into_any()
                    })
                }
            })
            .collect();
        if moving || opening || switching {
            window.request_animation_frame();
        }
        let mut live: HashMap<TitleTab, AnyElement> = tabs.iter().cloned().zip(elements).collect();
        // TabWidthMotion: the slot's width eases while the tab keeps its own.
        let slot = |tab: &TitleTab, open: f32| {
            let (w, give) = slot_size(tab, open);
            div()
                .flex_shrink_0()
                .h(px(TITLE_BAR_HEIGHT))
                .w(px(w))
                .mr(px(-give))
                .overflow_hidden()
        };
        let elements: Vec<AnyElement> = slots
            .into_iter()
            .filter_map(|s| match s {
                crate::ui::tabs::motion::Slot::Live { key, open } => {
                    let element = live.remove(&key)?;
                    Some(if open >= 1. {
                        element
                    } else {
                        slot(&key, open)
                            // In a full strip the tab opens out of the new-tab
                            // button: its right edge shows first, so it grows
                            // leftward instead of sliding under the button.
                            .when(crowded, |d| d.flex().justify_end())
                            .child(element)
                            .into_any_element()
                    })
                }
                crate::ui::tabs::motion::Slot::Closing { key, open, .. } => {
                    let w = tab_width(&key);
                    // A closed conversation stays loaded, so its copy still reads.
                    let headline = key
                        .session()
                        .filter(|id| {
                            self.session_summaries
                                .contains_key(&self.workspace.session_for_pane(*id))
                        })
                        .map(|id| {
                            let copy = self.tab_copy(id, cx);
                            [
                                self.pane_tile_stack(&copy, None, 1.),
                                self.pane_headline(&copy, (None, 0., 1.), false, false),
                            ]
                        })
                        .into_iter()
                        .flatten();
                    Some(
                        slot(&key, open)
                            .child(
                                div()
                                    .w(px(w))
                                    .h(px(TITLE_BAR_HEIGHT))
                                    .flex()
                                    .items_center()
                                    .child(
                                        title_tab_plate(false, false)
                                            .w_full()
                                            .min_w_0()
                                            .gap(px(TAB_TILE_GAP))
                                            .children(headline),
                                    ),
                            )
                            .into_any_element(),
                    )
                }
            })
            .collect();
        div()
            .id("title-tab-strip")
            .debug_selector(|| "title-tab-strip".into())
            .w(px(strip_width))
            .min_w_0()
            .h(px(TITLE_BAR_HEIGHT))
            .relative()
            .flex()
            .items_center()
            .overflow_hidden()
            .children(dock)
            .child(
                div()
                    .id("session-tabs")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_hidden()
                    .track_scroll(&self.tab_scroll)
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .pl(px(STRIP_LEAD))
                    .pr(px(STRIP_TRAIL - 2.))
                    .children(elements)
                    .child(
                        div()
                            .id("new-tab")
                            .debug_selector(|| "new-tab".into())
                            .group("new-tab")
                            .role(Role::Button)
                            .aria_label("New session tab")
                            .cursor_pointer()
                            .size(px(TAB_PLATE_HEIGHT))
                            .flex_shrink_0()
                            .ml(px(new_tab_lead))
                            .rounded_md()
                            .bg(content(0.04))
                            .opacity(0.65)
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|s| s.opacity(1.).bg(content(0.07)))
                            .relative()
                            .when(self.menu_open(&MenuTarget::NewTab), |d| {
                                d.opacity(1.).bg(content(0.07))
                            })
                            .child(self.new_tab_anchor.probe())
                            .child(
                                icon("plus", 14.)
                                    .group_hover("new-tab", |s| s.text_color(content(1.))),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                // Title bar terminals: pick a session or a terminal.
                                if !this.terminal_in_title() {
                                    this.new_chat(cx);
                                } else if this.menu_open(&MenuTarget::NewTab) {
                                    this.close_menu(cx);
                                } else {
                                    let anchor = Anchor::Slot(this.new_tab_anchor.clone());
                                    this.open_menu(MenuTarget::NewTab, anchor, window, cx);
                                }
                            })),
                    )
                    .child(
                        div()
                            .id("new-tab-model")
                            .debug_selector(|| "new-tab-model".into())
                            .group("new-tab-model")
                            .role(Role::Button)
                            .aria_label("New session with a chosen model")
                            .cursor_pointer()
                            .w(px(NEW_TAB_MODEL_WIDTH))
                            .h(px(TAB_PLATE_HEIGHT))
                            .flex_shrink_0()
                            .rounded_md()
                            .bg(content(0.04))
                            .opacity(0.65)
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|s| s.opacity(1.).bg(content(0.07)))
                            .when(self.menu_open(&MenuTarget::NewTabModel), |d| {
                                d.opacity(1.).bg(content(0.07))
                            })
                            .child(
                                icon("chevron-down", 12.)
                                    .group_hover("new-tab-model", |s| s.text_color(content(1.))),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.menu_open(&MenuTarget::NewTabModel) {
                                    this.close_menu(cx);
                                } else {
                                    let anchor = Anchor::Slot(this.new_tab_anchor.clone());
                                    this.open_menu(MenuTarget::NewTabModel, anchor, window, cx);
                                }
                            })),
                    )
                    .child(titlebar_drag_area().flex_1().min_w_0()),
            )
            .into_any()
    }
}
