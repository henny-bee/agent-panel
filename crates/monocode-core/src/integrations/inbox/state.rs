use super::*;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OptionRow {
    pub id: String,
    pub name: String,
    pub key: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StatusFilter {
    pub open: bool,
    pub draft: bool,
    pub closed: bool,
    pub merged: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Filters {
    pub assigned_to_me: bool,
    pub hidden_projects: Vec<String>,
    pub hidden_linear_projects: Vec<String>,
    pub hidden_kinds: Vec<String>,
    pub time: String,
    pub status: StatusFilter,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            assigned_to_me: false,
            hidden_projects: vec![],
            hidden_linear_projects: vec![],
            hidden_kinds: vec![],
            time: "all".into(),
            status: StatusFilter::default(),
        }
    }
}

impl Filters {
    pub fn fetch_state(&self) -> &str {
        if self.status.closed || self.status.merged || !(self.status.open || self.status.draft) {
            "all"
        } else {
            "open"
        }
    }
    pub fn matches(&self, item: &Item, source: &str, search: &str) -> bool {
        if item.provider != source {
            return false;
        }
        let tracker = is_tracker(source);
        if !tracker
            && !((source == "gitlab" || source == "azuredevops") && self.assigned_to_me)
            && self.hidden_projects.contains(&item.project_path)
        {
            return false;
        }
        if !tracker && self.hidden_kinds.contains(&item.kind) {
            return false;
        }
        let linear_project = if item.project_id.is_empty() {
            "~none"
        } else {
            &item.project_id
        };
        if source == "linear"
            && self
                .hidden_linear_projects
                .iter()
                .any(|id| id == linear_project)
        {
            return false;
        }
        let s = &self.status;
        if (s.open || s.closed || (!tracker && (s.draft || s.merged)))
            && !match item.status() {
                "Open" => s.open,
                "Draft" => s.draft && !tracker,
                "Closed" => s.closed,
                "Merged" => s.merged && !tracker,
                _ => true,
            }
        {
            return false;
        }
        let now = Local::now();
        let start = match self.time.as_str() {
            "today" => Local
                .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
                .single()
                .map(|d| d.timestamp_millis())
                .unwrap_or(0),
            "7d" => now.timestamp_millis() - 7 * 86400000,
            "30d" => now.timestamp_millis() - 30 * 86400000,
            _ => 0,
        };
        if start > 0 && item.updated_ms() < start {
            return false;
        }
        let text = format!(
            "{} {} {} {} {} {}",
            item.title,
            item.reference(),
            item.number,
            item.repo,
            item.team_name,
            item.labels
                .iter()
                .map(|l| l.name.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        )
        .to_lowercase();
        search
            .split_whitespace()
            .all(|word| text.contains(&word.to_lowercase()))
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Seen {
    pub seeded: bool,
    pub items: BTreeMap<String, i64>,
}

impl Seen {
    pub fn unseen(&self, item: &Item) -> bool {
        self.seeded
            && self
                .items
                .get(&item.key())
                .is_none_or(|at| item.updated_ms() > *at)
    }
    pub fn mark(&mut self, item: &Item) {
        let old = self.items.entry(item.key()).or_default();
        *old = (*old).max(item.updated_ms());
    }
    pub fn seed(&mut self, items: &[Item]) {
        if !self.seeded && !items.is_empty() {
            for item in items {
                self.mark(item);
            }
            self.seeded = true;
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewState {
    pub source: String,
    pub filters: Filters,
    pub seen: Seen,
    pub hidden_linear_teams: Vec<String>,
    pub hidden_jira_projects: Vec<String>,
    pub list_width: f32,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            source: "github".into(),
            filters: Filters::default(),
            seen: Seen::default(),
            hidden_linear_teams: vec![],
            hidden_jira_projects: vec![],
            list_width: 280.,
        }
    }
}

impl ViewState {
    pub fn fetch_scope(&self, projects: &[PathBuf]) -> String {
        let mut paths = projects
            .iter()
            .map(|p| crate::files::path_key(&p.to_string_lossy()))
            .collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        let mut teams = self.hidden_linear_teams.clone();
        teams.sort();
        let mut jira = self.hidden_jira_projects.clone();
        jira.sort();
        format!(
            "{}:{}:{}:{}:{}",
            self.filters.assigned_to_me,
            self.filters.fetch_state(),
            paths.join("|"),
            teams.join(","),
            jira.join(",")
        )
    }
    pub fn load(store: &Store) -> Self {
        if let Some(state) = store
            .setting("native-inbox-state")
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
        {
            return state;
        }
        let desktop = store
            .data_dir()
            .map(DesktopPreferences::load)
            .unwrap_or_default();
        let mut state = Self::default();
        if let Some(source) = desktop
            .get("monocode.inboxSource")
            .filter(|s| PROVIDERS.contains(s))
        {
            state.source = source.into();
        }
        if let Some(filters) = desktop
            .json("monocode.inboxFilters")
            .and_then(|v| serde_json::from_value(v).ok())
        {
            state.filters = filters;
        }
        if let Some(seen) = desktop.json("monocode.inboxSeen") {
            state.seen = serde_json::from_value(seen.clone()).unwrap_or_else(|_| Seen {
                seeded: true,
                items: serde_json::from_value(seen).unwrap_or_default(),
            });
        }
        state.hidden_linear_teams = desktop
            .json("monocode.linearHiddenTeams")
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        state.hidden_jira_projects = desktop
            .json("monocode.jiraHiddenProjects")
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        state
    }
    pub fn save(&self, store: &Store) -> Result<(), String> {
        store.save_setting(
            "native-inbox-state",
            &serde_json::to_string(self).map_err(|e| e.to_string())?,
        )
    }
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub items: Vec<Item>,
    pub errors: BTreeMap<String, String>,
    pub connections: BTreeMap<String, Status>,
    pub linear_teams: Vec<OptionRow>,
    pub jira_projects: Vec<OptionRow>,
}

/// The Tauri InboxNotificationTracker: provider baselines and revisions are
/// separate from the user's read state. Startup and query changes never
/// trigger automations for an existing backlog.
#[derive(Default)]
pub struct NotificationTracker {
    pub(super) revisions: HashMap<String, i64>,
    pub(super) primed: HashSet<String>,
    pub(super) scope: Option<String>,
}

impl NotificationTracker {
    pub fn observe(&mut self, snapshot: &Snapshot, scope: String) -> Vec<Item> {
        if self.scope.as_ref() != Some(&scope) {
            self.primed.clear();
        }
        self.scope = Some(scope);
        let mut appeared = vec![];
        for item in &snapshot.items {
            if snapshot.errors.contains_key(&item.provider) {
                continue;
            }
            let at = item.updated_ms();
            if at == 0 {
                continue;
            }
            let project = match item.provider.as_str() {
                "linear" if !item.project_id.is_empty() => {
                    format!("linear:project:{}", item.project_id)
                }
                "linear" => format!("linear:team:{}:unassigned", item.team_id),
                "jira" => format!(
                    "jira:{}:project:{}",
                    item.url
                        .split('/')
                        .nth(2)
                        .unwrap_or("unknown")
                        .to_lowercase(),
                    if item.team_id.is_empty() {
                        &item.repo
                    } else {
                        &item.team_id
                    }
                ),
                _ if !item.project_path.is_empty() => crate::files::path_key(&item.project_path),
                _ => format!("{}:{}", item.provider, item.repo.to_lowercase()),
            };
            let id = if item.id.is_empty() {
                item.number.to_string()
            } else {
                item.id.clone()
            };
            let key = format!("{project}:{}:{id}", item.kind);
            let previous = self.revisions.get(&key).copied();
            if self.primed.contains(&item.provider) && previous.is_none() {
                appeared.push(item.clone());
            }
            self.revisions.insert(key, previous.unwrap_or(0).max(at));
        }
        for provider in PROVIDERS {
            if !snapshot.errors.contains_key(provider) {
                self.primed.insert(provider.into());
            }
        }
        appeared
    }
}
