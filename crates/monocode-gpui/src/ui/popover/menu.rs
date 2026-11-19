use super::*;

impl MenuAction {
    pub(crate) fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            ..Self::default()
        }
    }
    pub(crate) fn description(mut self, text: impl Into<SharedString>) -> Self {
        self.description = Some(text.into());
        self
    }
    pub(crate) fn shortcut(mut self, text: impl Into<SharedString>) -> Self {
        self.shortcut = Some(text.into());
        self
    }
    pub(crate) fn icon(mut self, path: &'static str) -> Self {
        self.icon = Some(path);
        self
    }
    pub(crate) fn icon_artwork(mut self, path: &'static str) -> Self {
        self.icon = None;
        self.icon_artwork = Some(path);
        self
    }
    pub(crate) fn image(mut self, provider: &'static str) -> Self {
        self.image = Some(provider);
        self
    }
    pub(crate) fn icon_color(mut self, color: Hsla) -> Self {
        self.icon_color = Some(color);
        self
    }
    pub(crate) fn value(mut self, text: impl Into<SharedString>) -> Self {
        self.value = Some(text.into());
        self
    }
    pub(crate) fn value_image(mut self, provider: &'static str) -> Self {
        self.value_image = Some(provider);
        self
    }
    pub(crate) fn badge(mut self, text: impl Into<SharedString>) -> Self {
        self.badge = Some(text.into());
        self
    }
    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    pub(crate) fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
    pub(crate) fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }
    pub(crate) fn toggle(mut self, on: bool) -> Self {
        self.checked = on;
        self.switch = true;
        self.keep_open = true;
        self
    }
    pub(crate) fn keep_open(mut self) -> Self {
        self.keep_open = true;
        self
    }
    pub(crate) fn submenu(mut self, items: Vec<MenuAction>) -> Self {
        self.submenu = items;
        self
    }
    pub(crate) fn flyout(mut self, flyout: Flyout) -> Self {
        self.flyout = Some(flyout);
        self
    }
    pub(super) fn opens(&self) -> bool {
        !self.disabled && (!self.submenu.is_empty() || self.flyout.is_some())
    }
}

impl Menu {
    pub(crate) fn submenu_region(&self) -> AnchorSlot {
        self.sub_region.clone()
    }
    pub(crate) fn surface_region(&self) -> AnchorSlot {
        self.surface.clone()
    }
    pub(crate) fn new(
        label: impl Into<SharedString>,
        items: Vec<MenuItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut menu = Self::embedded(label, items, cx);
        menu.restore = window.focused(cx);
        window.focus(&menu.content_focus(), cx);
        menu
    }
    /// A flyout is constructed before it opens; it must not steal focus then.
    pub(crate) fn embedded(
        label: impl Into<SharedString>,
        items: Vec<MenuItem>,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        let active = items
            .iter()
            .position(|item| matches!(item, MenuItem::Action(_)));
        Self {
            label: label.into(),
            rows: items.iter().map(|_| AnchorSlot::default()).collect(),
            surface: AnchorSlot::default(),
            items,
            style: MenuStyle::Explorer,
            width: MENU_WIDTH,
            row_metrics: None,
            search_navigation: false,
            submenu_width: None,
            active,
            submenu: None,
            sub_active: None,
            sub_region: AnchorSlot::default(),
            focus,
            restore: None,
        }
    }
    pub(crate) fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }
    /// Override row and icon sizes for menus whose source uses taller rows.
    pub(crate) fn row_metrics(mut self, height: f32, icon_size: f32) -> Self {
        self.row_metrics = Some((height, icon_size));
        self
    }
    /// A custom search field yields arrow and submit actions to this menu.
    pub(crate) fn search_navigation(mut self) -> Self {
        self.search_navigation = true;
        self
    }
    pub(crate) fn submenu_width(mut self, width: f32) -> Self {
        self.submenu_width = Some(width);
        self
    }
    pub(crate) fn content_focus(&self) -> FocusHandle {
        self.items
            .iter()
            .find_map(|item| match item {
                MenuItem::Custom(_, focus) => Some(focus.clone()),
                _ => None,
            })
            .unwrap_or_else(|| self.focus.clone())
    }
    pub(crate) fn select_value(mut self, value: &str) -> Self {
        self.active = self.items.iter().position(|item| {
            matches!(item, MenuItem::Action(action) if action.id.as_ref() == value && !action.disabled)
        });
        self
    }
    pub(crate) fn style(mut self, style: MenuStyle) -> Self {
        self.style = style;
        self
    }
    /// Replace a loading placeholder in place, retaining the flyout and focus.
    pub(crate) fn set_items(&mut self, items: Vec<MenuItem>, cx: &mut Context<Self>) {
        self.active = items
            .iter()
            .position(|item| matches!(item, MenuItem::Action(action) if !action.disabled));
        self.rows = items.iter().map(|_| AnchorSlot::default()).collect();
        self.items = items;
        self.submenu = None;
        self.sub_active = None;
        self.sub_region.set(None);
        cx.notify();
    }
    /// Rows of the open submenu or flyout's anchor, for anchoring follow-ups.
    pub(crate) fn row_bounds(&self, ix: usize) -> Option<Bounds<Pixels>> {
        self.rows.get(ix).and_then(AnchorSlot::get)
    }
    pub(super) fn action(&self, ix: usize) -> Option<&MenuAction> {
        match self.items.get(ix) {
            Some(MenuItem::Action(action)) => Some(action),
            _ => None,
        }
    }
    pub(super) fn submenu_items(&self) -> &[MenuAction] {
        self.submenu
            .and_then(|ix| self.action(ix))
            .map_or(&[], |action| &action.submenu)
    }
    pub(super) fn flyout_open(&self) -> bool {
        self.submenu
            .and_then(|ix| self.action(ix))
            .is_some_and(|a| a.flyout.is_some())
    }
    pub(crate) fn finish(&mut self, event: MenuEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handle) = self.restore.take() {
            window.focus(&handle, cx);
        }
        cx.emit(event);
    }
    pub(crate) fn replace_items(&mut self, items: Vec<MenuItem>, cx: &mut Context<Self>) {
        self.rows.resize_with(items.len(), AnchorSlot::default);
        self.items = items;
        self.active = self
            .active
            .filter(|&ix| self.action(ix).is_some_and(|a| !a.disabled));
        self.submenu = None;
        self.sub_active = None;
        cx.notify();
    }
    /// Refresh a live toggle without resetting keyboard focus or its flyout.
    pub(crate) fn set_toggle(&mut self, id: &str, on: bool, cx: &mut Context<Self>) {
        if let Some(action) = self.items.iter_mut().find_map(|item| match item {
            MenuItem::Action(action) if action.id.as_ref() == id && action.switch => Some(action),
            _ => None,
        }) {
            action.checked = on;
            cx.notify();
        }
    }
    pub(crate) fn toggle_state(&self, id: &str) -> Option<bool> {
        self.items.iter().find_map(|item| match item {
            MenuItem::Action(action) if action.id.as_ref() == id && action.switch => {
                Some(action.checked)
            }
            _ => None,
        })
    }
    pub(super) fn open_submenu(
        &mut self,
        ix: usize,
        active: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let changed = self.submenu != Some(ix);
        self.submenu = Some(ix);
        self.sub_active = active;
        if let Some(focus) = self
            .action(ix)
            .and_then(|a| a.flyout.as_ref())
            .and_then(|f| f.focus.clone())
        {
            window.focus(&focus, cx);
        }
        if changed && let Some(action) = self.action(ix) {
            cx.emit(MenuOpened(action.id.clone()));
        }
        cx.notify();
    }
    pub(super) fn close_submenu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.flyout_open() {
            window.focus(&self.focus, cx);
        }
        self.submenu = None;
        self.sub_active = None;
        self.sub_region.set(None);
        cx.notify();
    }
    pub(super) fn pick(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(action) = self.action(ix) else {
            return;
        };
        if action.disabled {
            return;
        }
        if action.opens() {
            let first = action.flyout.is_none().then(|| {
                action
                    .submenu
                    .iter()
                    .position(|item| item.checked)
                    .unwrap_or(0)
            });
            self.open_submenu(ix, first, window, cx);
        } else {
            let id = action.id.clone();
            let keep_open = action.keep_open;
            let row = self.row_bounds(ix);
            if keep_open {
                if self.submenu.is_some() {
                    self.close_submenu(window, cx);
                }
                cx.emit(MenuEvent::Picked(id, row));
            } else {
                self.finish(MenuEvent::Picked(id, row), window, cx);
            }
        }
    }
    pub(super) fn pick_sub(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(action) = self.submenu_items().get(ix) else {
            return;
        };
        if !action.disabled {
            let id = action.id.clone();
            self.finish(MenuEvent::Picked(id, None), window, cx);
        }
    }
    pub(super) fn step(&mut self, direction: isize, cx: &mut Context<Self>) {
        let count = self.submenu_items().len();
        if count > 0 {
            let next = match self.sub_active {
                None if direction > 0 => 0,
                None => count - 1,
                Some(at) => (at as isize + direction).rem_euclid(count as isize) as usize,
            };
            self.sub_active = Some(next);
        } else {
            let ids: Vec<_> = (0..self.items.len())
                .filter(|ix| self.action(*ix).is_some())
                .collect();
            if ids.is_empty() {
                return;
            }
            let from = self
                .active
                .and_then(|a| ids.iter().position(|ix| *ix == a))
                .unwrap_or(0);
            self.active =
                Some(ids[(from as isize + direction).rem_euclid(ids.len() as isize) as usize]);
        }
        cx.notify();
    }
    pub(super) fn row(
        &self,
        ix: usize,
        action: &MenuAction,
        style: MenuStyle,
        highlighted: bool,
        in_submenu: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let red: Hsla = rgb(RED_300).into();
        let rich = matches!(style, MenuStyle::Composer | MenuStyle::Access);
        // The plus menu dims a disabled row whole; the others grey its ink.
        let dim = action.disabled && (rich || style == MenuStyle::Git);
        let ink = if action.disabled && !dim {
            content(0.3)
        } else if action.danger {
            red.opacity(if highlighted { 1. } else { 0.9 })
        } else {
            content(1.)
        };
        let open = self.submenu == Some(ix) && !in_submenu;
        let filled = (style.highlights() && highlighted)
            || (style == MenuStyle::Access && action.checked)
            || (style == MenuStyle::Switcher && open);
        let fill = match style {
            MenuStyle::Switcher => content(0.08),
            _ if action.danger => rgb(RED_500).into(),
            _ => content(0.1),
        };
        let fill = if action.danger && style != MenuStyle::Switcher {
            fill.opacity(0.2)
        } else {
            fill
        };
        let hover = match style {
            _ if action.danger => {
                let mut c: Hsla = rgb(RED_500).into();
                c.a = 0.15;
                c
            }
            MenuStyle::Switcher => content(0.08),
            MenuStyle::Composer | MenuStyle::Git => content(0.1),
            _ => content(0.05),
        };
        let icon_size = self
            .row_metrics
            .map(|(_, size)| size)
            .unwrap_or_else(|| match style {
                MenuStyle::Group | MenuStyle::Access | MenuStyle::Git | MenuStyle::Filter => 14.,
                _ => 16.,
            });
        let icon_ink = action.icon_color.unwrap_or(match style {
            MenuStyle::Group | MenuStyle::Switcher => content(0.55),
            MenuStyle::Access => content(0.7),
            _ => content(1.),
        });
        let has_submenu = !action.submenu.is_empty() || action.flyout.is_some();
        let selector = action.id.clone();
        div()
            .id((
                if in_submenu {
                    "submenu-item"
                } else {
                    "menu-item"
                },
                ix,
            ))
            .debug_selector(move || format!("menu-action-{selector}"))
            .role(if action.switch {
                Role::MenuItemCheckBox
            } else {
                Role::MenuItem
            })
            .aria_label(action.label.clone())
            .when_some(action.description.clone(), |d, s| d.aria_description(s))
            .when_some(action.shortcut.clone(), |d, s| d.aria_keyshortcuts(s))
            .when(action.checked || action.switch, |d| {
                d.aria_toggled(if action.checked {
                    Toggled::True
                } else {
                    Toggled::False
                })
            })
            .when(has_submenu, |d| d.aria_expanded(open))
            .aria_selected(highlighted)
            .cursor_pointer()
            .relative()
            .flex_shrink_0()
            .w_full()
            .flex()
            .px_2()
            .text_size(px(if style == MenuStyle::Git { 12. } else { 13. }))
            .text_color(ink)
            .when(dim, |d| d.opacity(0.4))
            .map(|d| match style {
                MenuStyle::Explorer => d
                    .items_center()
                    .gap_3()
                    .rounded(px(8.))
                    // Not 13px: `truncate` clips to the line box, and Windows'
                    // Segoe UI lost the bottom of g/y/p at the font's own size.
                    .line_height(px(16.))
                    .map(|d| {
                        if action.description.is_some() {
                            // The 16px line (above) eats 1.5px a side.
                            d.py(px(4.5))
                        } else {
                            d.h(px(if in_submenu && self.style == MenuStyle::Settings {
                                32.
                            } else {
                                28.
                            }))
                        }
                    }),
                MenuStyle::Group => d
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(8.))
                    .min_h(px(32.))
                    .when(action.description.is_some(), |d| d.py_2()),
                MenuStyle::Switcher => d.items_center().gap(px(10.)).rounded(px(6.)).h(px(32.)),
                MenuStyle::Git | MenuStyle::Filter => {
                    d.items_center().gap_2().rounded(px(8.)).h(px(28.))
                }
                MenuStyle::Settings => d.items_center().gap_2().rounded(px(8.)).h(px(36.)),
                MenuStyle::Recent => d.items_center().gap_2().rounded(px(8.)).h(px(40.)),
                MenuStyle::Composer | MenuStyle::Access => {
                    d.items_start().gap(px(10.)).rounded(px(8.)).py_2()
                }
            })
            .when_some(self.row_metrics, |d, (height, _)| {
                d.h(px(height)).gap(px(8.))
            })
            .when(filled && !action.disabled, |d| d.bg(fill))
            .when(!filled && !action.disabled, |d| d.hover(|s| s.bg(hover)))
            .when(!in_submenu, |d| d.child(self.rows[ix].probe()))
            .when_some(action.icon, |d, path| {
                d.child(
                    icon(path, icon_size)
                        .text_color(icon_ink)
                        .when(rich, |i| i.mt(px(2.))),
                )
            })
            .when_some(action.icon_artwork, |d, path| {
                d.child(
                    gpui::img(path)
                        .size(px(icon_size))
                        .flex_shrink_0()
                        .object_fit(gpui::ObjectFit::Contain),
                )
            })
            .when_some(action.image, |d, provider| {
                d.child(provider_icon(provider, icon_size))
            })
            .child(self.row_text(action, style))
            .when_some(action.value.clone(), |d, value| {
                d.child(match style {
                    MenuStyle::Settings => div()
                        .flex_shrink_0()
                        .max_w(px(if action.value_image.is_some() {
                            144.
                        } else {
                            112.
                        }))
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_color(content(0.55))
                        .when_some(action.value_image, |d, provider| {
                            d.child(provider_icon(provider, 14.))
                        })
                        .child(div().min_w_0().truncate().child(value)),
                    _ => div()
                        .flex_shrink_0()
                        .max_w(px(96.))
                        .truncate()
                        .text_size(px(11.))
                        .text_color(content(0.4))
                        .child(value),
                })
            })
            .map(|d| {
                if has_submenu {
                    let chevron = match style {
                        MenuStyle::Switcher => content(0.4),
                        MenuStyle::Settings => content(0.45),
                        _ => content(0.5),
                    };
                    d.child(icon("chevron-right", 14.).text_color(chevron))
                } else if action.switch {
                    d.child(
                        div()
                            .relative()
                            .w(px(36.))
                            .h(px(20.))
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(content(if action.checked { 0.35 } else { 0.15 }))
                            .child(
                                div()
                                    .absolute()
                                    .top(px(2.))
                                    .left(px(if action.checked { 18. } else { 2. }))
                                    .size(px(16.))
                                    .rounded_full()
                                    .bg(content(1.)),
                            ),
                    )
                } else if action.checked && style == MenuStyle::Recent {
                    d.child(icon("check", 14.).text_color(content(0.55)))
                } else if action.checked && style != MenuStyle::Access {
                    d.child(
                        icon(
                            if in_submenu || style == MenuStyle::Explorer {
                                "check"
                            } else {
                                "check-2"
                            },
                            14.,
                        )
                        .text_color(action.icon_color.unwrap_or(content(1.)))
                        .when(rich, |i| i.mt(px(2.))),
                    )
                } else if let Some(shortcut) = action.shortcut.clone() {
                    d.child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(11.))
                            .text_color(content(0.4))
                            .child(shortcut),
                    )
                } else {
                    d
                }
            })
            .on_hover(cx.listener(move |this, hovered: &bool, window, cx| {
                if !*hovered {
                    return;
                }
                if in_submenu {
                    this.sub_active = Some(ix);
                } else {
                    this.active = Some(ix);
                    let opens = this.action(ix).is_some_and(MenuAction::opens);
                    if opens {
                        if this.submenu != Some(ix) {
                            this.open_submenu(ix, None, window, cx);
                        }
                    } else if this.submenu.is_some() {
                        this.close_submenu(window, cx);
                    }
                }
                cx.notify();
            }))
            .on_click(cx.listener(move |this, _, window, cx| {
                if in_submenu {
                    this.pick_sub(ix, window, cx);
                } else {
                    this.pick(ix, window, cx);
                }
            }))
            .into_any()
    }
    pub(super) fn row_text(&self, action: &MenuAction, style: MenuStyle) -> impl IntoElement {
        let title = div()
            .min_w_0()
            .truncate()
            .when(style == MenuStyle::Access, |d| {
                d.font_weight(gpui::FontWeight::MEDIUM).line_height(px(20.))
            })
            .when(style == MenuStyle::Composer, |d| d.line_height(px(19.5)))
            .when(style == MenuStyle::Recent, |d| d.line_height(px(16.)))
            .child(action.label.clone());
        let hint = action.description.clone().map(|text| {
            let (alpha, gap) = match style {
                MenuStyle::Composer | MenuStyle::Recent => (0.45, 0.),
                MenuStyle::Access => (0.5, 2.),
                MenuStyle::Group => (0.6, 4.),
                _ => (0.5, 4.),
            };
            div()
                .mt(px(gap))
                .text_size(px(11.))
                .line_height(px(
                    if matches!(
                        style,
                        MenuStyle::Composer | MenuStyle::Access | MenuStyle::Recent
                    ) {
                        16.
                    } else {
                        15.
                    },
                ))
                .text_color(content(alpha))
                .when(
                    matches!(style, MenuStyle::Composer | MenuStyle::Recent),
                    |d| d.truncate(),
                )
                .child(text)
        });
        div()
            .flex_1()
            .min_w_0()
            .child(match &action.badge {
                // The Orchestrator row's "v1" pill beside its title.
                Some(badge) => div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(title)
                    .child(
                        div()
                            .flex_shrink_0()
                            .mb(px(1.))
                            .px(px(6.))
                            .py(px(2.))
                            .rounded_full()
                            .bg(gpui::hsla(0.82, 1., 0.83, 0.1))
                            .text_size(px(9.))
                            .line_height(px(9.))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(gpui::hsla(0.81, 1., 0.9, 0.55))
                            .child(badge.clone()),
                    )
                    .into_any(),
                None => title.into_any(),
            })
            .children(hint)
    }
}

impl Render for Menu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let style = self.style;
        let rows: Vec<AnyElement> = self
            .items
            .iter()
            .enumerate()
            .map(|(ix, item)| match item {
                MenuItem::Custom(view, _) => {
                    div().flex_shrink_0().child(view.clone()).into_any_element()
                }
                MenuItem::Header(text) => div()
                    .flex_shrink_0()
                    .px_2()
                    .pt(px(2.))
                    .pb_1()
                    .text_size(px(10.))
                    .line_height(px(15.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(content(0.4))
                    .child(text.to_uppercase())
                    .into_any(),
                MenuItem::Note(text) => div()
                    .flex_shrink_0()
                    .px_2()
                    .py(px(6.))
                    .text_size(px(11.))
                    .line_height(px(16.))
                    .text_color(content(0.5))
                    .child(text.clone())
                    .into_any(),
                MenuItem::Separator => separator(ix, style),
                MenuItem::Action(action) => {
                    self.row(ix, action, style, self.active == Some(ix), false, cx)
                }
            })
            .collect();
        let submenu = self.submenu.and_then(|ix| {
            let action = self.action(ix)?;
            let submenu = submenu_popover(
                ("submenu", ix),
                self.rows[ix].clone(),
                self.surface.clone(),
                self.sub_region.clone(),
            );
            if let Some(flyout) = &action.flyout {
                let p = submenu
                    .width(flyout.width)
                    .padding(flyout.padding)
                    .when_some(flyout.surface.clone(), |p, slot| p.track(slot));
                let p = match (flyout.height, flyout.max_height) {
                    (Some(h), _) => p.height(h),
                    (None, Some(h)) => p.max_height(h),
                    _ => p,
                };
                return Some(p.child(flyout.view.clone()));
            }
            let rows: Vec<_> = action
                .submenu
                .iter()
                .enumerate()
                .map(|(sub, item)| {
                    self.row(
                        sub,
                        item,
                        MenuStyle::Explorer,
                        self.sub_active == Some(sub),
                        true,
                        cx,
                    )
                })
                .collect();
            Some(
                submenu
                    .map(|p| match self.submenu_width {
                        Some(width) => p.width(width),
                        None => p.max_width(MENU_WIDTH).min_width(MENU_MIN_WIDTH),
                    })
                    .padding(4.)
                    .child(
                        div()
                            .id("popover-submenu")
                            .role(Role::Menu)
                            .aria_label(action.label.clone())
                            .flex()
                            .flex_col()
                            .children(rows),
                    ),
            )
        });
        div()
            .id("popover-menu")
            .role(Role::Menu)
            .aria_label(self.label.clone())
            .key_context("PopoverMenu")
            .track_focus(&self.focus)
            .capture_action(cx.listener(|this, _: &crate::ui::input::Up, window, cx| {
                if !this.search_navigation {
                    cx.propagate();
                    return;
                }
                this.step(-1, cx);
                window.focus(&this.focus, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &crate::ui::input::Down, window, cx| {
                if !this.search_navigation {
                    cx.propagate();
                    return;
                }
                this.step(1, cx);
                window.focus(&this.focus, cx);
                cx.stop_propagation();
            }))
            .capture_action(
                cx.listener(|this, _: &crate::ui::input::Submit, window, cx| {
                    if !this.search_navigation {
                        cx.propagate();
                        return;
                    }
                    window.focus(&this.focus, cx);
                    match (this.submenu, this.sub_active) {
                        (Some(_), Some(sub)) => this.pick_sub(sub, window, cx),
                        _ => {
                            if let Some(ix) = this.active {
                                this.pick(ix, window, cx);
                            }
                        }
                    }
                    cx.stop_propagation();
                }),
            )
            // Space activates a focused menu, but remains text in its fields.
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key != "space" || !this.focus.is_focused(window) {
                    return;
                }
                match (this.submenu, this.sub_active) {
                    (Some(_), Some(sub)) => this.pick_sub(sub, window, cx),
                    _ => {
                        if let Some(ix) = this.active {
                            this.pick(ix, window, cx);
                        }
                    }
                }
                window.prevent_default();
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &Confirm, window, cx| {
                match (this.submenu, this.sub_active) {
                    (Some(_), Some(sub)) => this.pick_sub(sub, window, cx),
                    _ => {
                        if let Some(ix) = this.active {
                            this.pick(ix, window, cx)
                        }
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &OpenSubmenu, window, cx| {
                if let Some(ix) = this.active
                    && this.action(ix).is_some_and(MenuAction::opens)
                {
                    let first = this
                        .action(ix)
                        .filter(|action| action.flyout.is_none())
                        .map(|action| {
                            action
                                .submenu
                                .iter()
                                .position(|item| item.checked)
                                .unwrap_or(0)
                        });
                    this.open_submenu(ix, first, window, cx);
                }
            }))
            .on_action(
                cx.listener(|this, _: &CloseSubmenu, window, cx| this.close_submenu(window, cx)),
            )
            .on_action(cx.listener(|this, _: &Cancel, window, cx| {
                // A flyout's Escape closes everything, as the pickers' does.
                if this.submenu.is_some() && !this.flyout_open() {
                    this.close_submenu(window, cx);
                } else {
                    this.finish(MenuEvent::Dismissed, window, cx);
                }
            }))
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .children(rows)
            .children(submenu)
    }
}
