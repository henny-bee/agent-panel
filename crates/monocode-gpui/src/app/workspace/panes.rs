use super::*;

impl Workspace {
    /// File, Mono and terminal panes share the negative IDs below every
    /// session.
    pub fn next_pane_id(&self) -> i64 {
        self.review_owners
            .keys()
            .chain(self.mono_panes.keys())
            .chain(self.terminal_panes.keys())
            .copied()
            .min()
            .unwrap_or(0)
            .min(0)
            - 1
    }

    pub fn mono_pane(&self, mono: &str) -> Option<i64> {
        self.mono_panes.iter().find_map(|(pane, tab)| {
            matches!(tab, TitleTab::Mono { id, .. } if id == mono).then_some(*pane)
        })
    }

    /// The pane that scopes a tab to a project: its focus, unless that is a
    /// Mono, which works across projects.
    pub fn scope_pane(&self, tab: i64) -> i64 {
        let focus = self.tab_focus(tab);
        if !self.mono_panes.contains_key(&focus) {
            return focus;
        }
        self.layout(tab)
            .leaf_ids()
            .into_iter()
            .find(|id| !self.mono_panes.contains_key(id))
            .unwrap_or(focus)
    }

    /// Focus a session, opening it as its own tab unless a tab holds it.
    pub fn open(&mut self, id: i64) {
        match self.tab_of(id) {
            Some(tab) => {
                if self.layouts.contains_key(&tab) {
                    self.focused.insert(tab, id);
                }
            }
            None => {
                self.tabs.push(id);
                self.title_order.push(TitleTab::Session(id));
            }
        }
        self.active = Some(id);
    }

    /// Close the whole title tab holding `id`; its sessions keep running.
    pub fn close(&mut self, id: i64) {
        let Some(tab) = self.tab_of(id) else {
            return;
        };
        let ix = self.tabs.iter().position(|t| *t == tab).unwrap_or(0);
        let layout = self.layout(tab);
        self.tabs.remove(ix);
        self.tab_workspaces.remove(&tab);
        self.title_order
            .retain(|entry| *entry != TitleTab::Session(tab));
        self.scope_focus
            .retain(|_, entry| *entry != TitleTab::Session(tab));
        self.layouts.remove(&tab);
        self.focused.remove(&tab);
        self.project_focus.retain(|_, id| !layout.contains(*id));
        if self
            .active
            .is_some_and(|active| self.tab_of(active).is_none())
        {
            self.active = self
                .tabs
                .get(ix)
                .or_else(|| self.tabs.last())
                .map(|tab| self.tab_focus(*tab));
        }
    }

    /// Store a tab's tree, rekeying it when its key session left and
    /// dropping the tree once a single pane remains.
    pub(super) fn set_layout(&mut self, tab: i64, layout: PaneNode) -> i64 {
        let Some(ix) = self.tabs.iter().position(|t| *t == tab) else {
            return tab;
        };
        let focus = self.focused.remove(&tab);
        self.layouts.remove(&tab);
        // A tab is keyed by a conversation when it holds one, never a Mono.
        let key = if layout.contains(tab) && !self.mono_panes.contains_key(&tab) {
            tab
        } else {
            let leaves = layout.leaf_ids();
            leaves
                .iter()
                .copied()
                .find(|id| !self.mono_panes.contains_key(id))
                .unwrap_or(leaves[0])
        };
        self.tabs[ix] = key;
        if key != tab
            && let Some(scope) = self.tab_workspaces.remove(&tab)
        {
            self.tab_workspaces.insert(key, scope);
        }
        for entry in self
            .title_order
            .iter_mut()
            .chain(self.scope_focus.values_mut())
            .chain(self.pinned_tabs.iter_mut())
        {
            if *entry == TitleTab::Session(tab) {
                *entry = TitleTab::Session(key);
            }
        }
        if layout.is_split() {
            if let Some(focus) = focus.filter(|id| layout.contains(*id)) {
                self.focused.insert(key, focus);
            }
            self.layouts.insert(key, layout);
        }
        key
    }

    /// Take a session out of whichever tab holds it. A tab left empty
    /// closes; the active session moves to a neighbouring pane.
    pub(super) fn take_out(&mut self, id: i64) {
        let Some(tab) = self.tab_of(id) else {
            return;
        };
        let layout = self.layout(tab);
        self.project_focus.retain(|_, focus| *focus != id);
        match layout.remove(id) {
            Some(rest) => {
                let next = layout.sibling(id).unwrap_or_else(|| rest.first_leaf());
                let key = self.set_layout(tab, rest);
                if self.active == Some(id) {
                    self.active = Some(next);
                }
                if self.layouts.contains_key(&key) && self.focused.get(&key) == Some(&id) {
                    self.focused.insert(key, next);
                }
            }
            None => self.close(id),
        }
    }

    /// Close one pane of a split tab. False when it is the tab's only pane.
    pub fn close_pane(&mut self, id: i64) -> bool {
        let Some(tab) = self.tab_of(id) else {
            return false;
        };
        if !self.layout(tab).is_split() {
            return false;
        }
        self.take_out(id);
        if let Some(active) = self.active
            && let Some(tab) = self.tab_of(active)
            && self.layouts.contains_key(&tab)
        {
            self.focused.insert(tab, active);
        }
        true
    }

    /// cmd-d: a new session beside the focused pane.
    pub fn split(&mut self, new: i64, dir: SplitDir) {
        let Some(focused) = self.active else {
            self.open(new);
            return;
        };
        let Some(tab) = self.tab_of(focused) else {
            self.open(new);
            return;
        };
        self.take_out(new);
        let tab = self.tab_of(focused).unwrap_or(tab);
        let key = self.set_layout(tab, self.layout(tab).split_pane(focused, dir, new));
        self.focused.insert(key, new);
        self.active = Some(new);
    }

    /// A pane, tab-strip session or sidebar session dropped on a pane edge.
    pub fn place(&mut self, id: i64, target: i64, edge: Edge) {
        if id == target || self.tab_of(target).is_none() {
            return;
        }
        let same = self.tab_of(id) == self.tab_of(target);
        if !same {
            self.take_out(id);
        }
        let Some(tab) = self.tab_of(target) else {
            return;
        };
        let key = self.set_layout(tab, self.layout(tab).place_pane(id, target, edge));
        self.focused.insert(key, id);
        self.active = Some(id);
    }

    /// A whole title tab dropped on a pane edge: its tree moves intact.
    pub fn place_tab(&mut self, source: i64, target: i64, edge: Edge) {
        let Some(target_tab) = self.tab_of(target) else {
            return;
        };
        if source == target_tab || !self.tabs.contains(&source) {
            return;
        }
        let incoming = self.layout(source);
        let focus = self.tab_focus(source);
        self.tabs.retain(|t| *t != source);
        self.tab_workspaces.remove(&source);
        self.title_order
            .retain(|entry| *entry != TitleTab::Session(source));
        self.layouts.remove(&source);
        self.focused.remove(&source);
        let key = self.set_layout(
            target_tab,
            self.layout(target_tab).place_layout(incoming, target, edge),
        );
        self.focused.insert(key, focus);
        self.active = Some(focus);
    }

    /// A Mono tab dropped on a pane edge leaves the strip for the split.
    pub fn place_mono(&mut self, mono: TitleTab, target: i64, edge: Edge) -> Option<i64> {
        if !matches!(mono, TitleTab::Mono { .. }) || self.tab_of(target).is_none() {
            return None;
        }
        let pane = self.next_pane_id();
        self.title_order.retain(|entry| *entry != mono);
        self.scope_focus.retain(|_, entry| *entry != mono);
        self.mono_panes.insert(pane, mono);
        self.place(pane, target, edge);
        Some(pane)
    }

    /// The visible Mono becomes a split's first pane, its tab keeping its
    /// slot, so a tab or pane can be dropped beside it.
    pub fn mono_tab_pane(&mut self, mono: TitleTab) -> i64 {
        let pane = self.next_pane_id();
        self.tabs.push(pane);
        for entry in self
            .title_order
            .iter_mut()
            .chain(self.scope_focus.values_mut())
            .chain(self.pinned_tabs.iter_mut())
        {
            if *entry == mono {
                *entry = TitleTab::Session(pane);
            }
        }
        if !self.title_order.contains(&TitleTab::Session(pane)) {
            self.title_order.push(TitleTab::Session(pane));
        }
        self.mono_panes.insert(pane, mono);
        self.active = Some(pane);
        pane
    }

    /// A Mono pane left alone in its tab becomes its Mono tab again, in the
    /// same slot; one no tab holds is forgotten. Returns the Mono when it
    /// had focus, for the caller to show.
    pub fn settle_mono_panes(&mut self) -> Option<TitleTab> {
        let mut focused = None;
        let panes: Vec<_> = self.mono_panes.keys().copied().collect();
        for pane in panes {
            match self.tab_of(pane) {
                Some(tab) if self.layout(tab).is_split() => continue,
                Some(tab) => {
                    let mono = self.mono_panes[&pane].clone();
                    self.tabs.retain(|t| *t != tab);
                    self.tab_workspaces.remove(&tab);
                    self.layouts.remove(&tab);
                    self.focused.remove(&tab);
                    let present = self.title_order.contains(&mono);
                    self.title_order
                        .retain(|entry| !(present && *entry == TitleTab::Session(tab)));
                    for entry in self
                        .title_order
                        .iter_mut()
                        .chain(self.scope_focus.values_mut())
                        .chain(self.pinned_tabs.iter_mut())
                    {
                        if *entry == TitleTab::Session(tab) {
                            *entry = mono.clone();
                        }
                    }
                    if self.active == Some(pane) {
                        self.active = None;
                        focused = Some(mono);
                    }
                }
                None => {}
            }
            self.mono_panes.remove(&pane);
        }
        focused
    }

    /// Terminal panes no tab holds any more, forgotten. Their shells stop
    /// with them, so the caller drops their views.
    pub fn prune_terminal_panes(&mut self) -> Vec<i64> {
        let open: HashSet<_> = self.open_sessions().into_iter().collect();
        let gone: Vec<_> = self
            .terminal_panes
            .keys()
            .copied()
            .filter(|id| !open.contains(id))
            .collect();
        for id in &gone {
            self.terminal_panes.remove(id);
        }
        gone
    }

    /// A pane dragged onto the tab strip becomes its own tab at `to`.
    pub fn detach(&mut self, id: i64, to: usize) {
        if !self
            .tab_of(id)
            .is_some_and(|tab| self.layout(tab).is_split())
        {
            return;
        }
        let scope = self
            .tab_of(id)
            .and_then(|tab| self.tab_workspaces.get(&tab))
            .cloned();
        self.take_out(id);
        self.tabs.insert(to.min(self.tabs.len()), id);
        if let Some(scope) = scope {
            self.tab_workspaces.insert(id, scope);
        }
        let next = self
            .tabs
            .iter()
            .skip(to + 1)
            .find_map(|next| {
                self.title_order
                    .iter()
                    .position(|tab| *tab == TitleTab::Session(*next))
            })
            .unwrap_or(self.title_order.len());
        self.title_order.insert(next, TitleTab::Session(id));
        self.active = Some(id);
    }

    /// Show `to` in `from`'s pane (Session: Next/Previous in tab).
    pub fn swap_session(&mut self, from: i64, to: i64) {
        if self.tab_of(to).is_some() {
            return;
        }
        let Some(tab) = self.tab_of(from) else {
            return;
        };
        let key = self.set_layout(tab, self.layout(tab).replace_leaf(from, to));
        if self.layouts.contains_key(&key) {
            self.focused.insert(key, to);
        }
    }

    /// cmd-opt-arrows within the active tab.
    pub fn neighbor(&self, dir: Edge) -> Option<i64> {
        let active = self.active?;
        self.layout(self.tab_of(active)?).neighbor(active, dir)
    }

    pub fn place_sash(&mut self, tab: i64, path: &[usize], index: usize, boundary: f32) {
        if let Some(layout) = self.layouts.get_mut(&tab) {
            layout.place_sash(path, index, boundary);
        }
    }

    pub fn sash_snaps(&self, tab: i64, path: &[usize], index: usize) -> Vec<f32> {
        self.layouts
            .get(&tab)
            .map_or_else(Vec::new, |layout| layout.sash_snaps(path, index))
    }

    pub fn sash_range(&self, tab: i64, path: &[usize], index: usize) -> Option<(f32, f32, f32)> {
        self.layouts.get(&tab)?.sash_range(path, index)
    }
}
