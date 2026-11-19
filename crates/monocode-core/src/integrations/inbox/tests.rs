use super::*;

fn item(number: i64, provider: &str) -> Item {
    Item {
        provider: provider.into(),
        kind: "issue".into(),
        repo: "acme/app".into(),
        number,
        id: number.to_string(),
        updated_at: "2026-10-21T10:00:00Z".into(),
        ..Item::default()
    }
}
fn snapshot(items: Vec<Item>) -> Snapshot {
    Snapshot {
        items,
        ..Snapshot::default()
    }
}

#[test]
fn first_baseline_and_existing_revisions_never_appear() {
    let mut tracker = NotificationTracker::default();
    assert!(tracker
        .observe(&snapshot(vec![item(1, "github")]), "scope".into())
        .is_empty());
    let mut updated = item(1, "github");
    updated.updated_at = "2026-10-21T11:00:00Z".into();
    let appeared = tracker.observe(&snapshot(vec![updated, item(2, "github")]), "scope".into());
    assert_eq!(
        appeared.iter().map(|i| i.number).collect::<Vec<_>>(),
        vec![2]
    );
    assert!(tracker
        .observe(&snapshot(vec![item(2, "github")]), "scope".into())
        .is_empty());
}

#[test]
fn failed_provider_has_its_own_successful_baseline() {
    let mut tracker = NotificationTracker::default();
    let mut failed = snapshot(vec![item(1, "linear")]);
    failed.errors.insert("linear".into(), "offline".into());
    assert!(tracker.observe(&failed, "scope".into()).is_empty());
    assert!(tracker
        .observe(&snapshot(vec![item(2, "linear")]), "scope".into())
        .is_empty());
    let appeared = tracker.observe(
        &snapshot(vec![item(2, "linear"), item(3, "linear")]),
        "scope".into(),
    );
    assert_eq!(appeared.len(), 1);
    assert_eq!(appeared[0].number, 3);
}

#[test]
fn changing_query_baselines_new_scope_without_forgetting_known_items() {
    let mut tracker = NotificationTracker::default();
    assert!(tracker
        .observe(&snapshot(vec![item(1, "github")]), "one".into())
        .is_empty());
    assert!(tracker
        .observe(&snapshot(vec![item(2, "github")]), "two".into())
        .is_empty());
    assert!(tracker
        .observe(
            &snapshot(vec![item(1, "github"), item(2, "github")]),
            "two".into()
        )
        .is_empty());
    assert_eq!(
        tracker
            .observe(&snapshot(vec![item(3, "github")]), "two".into())
            .len(),
        1
    );
}

#[test]
fn seen_keys_match_desktop_identity_rules() {
    let mut github = item(12, "github");
    github.url = "https://GitHub.com/Acme/App/pull/12/".into();
    assert_eq!(github.key(), "github:acme/app:issue:12");
    let mut linear = item(7, "linear");
    linear.id = "stable-id".into();
    linear.identifier = "MON-7".into();
    assert_eq!(linear.key(), "linear:mon-7");
}

#[test]
fn two_checkouts_of_one_repository_fetch_it_through_the_active_one() {
    let first = PathBuf::from("/work/app");
    let second = PathBuf::from("/work/app-review");
    let other = PathBuf::from("/work/site");
    let projects = [first.clone(), second.clone(), other.clone()];
    let repos = |project: &Path| {
        Some(vec![if project == other {
            "acme/site".to_string()
        } else {
            "Acme/App".to_string()
        }])
    };
    for (cwd, owner) in [(&first, &first), (&second, &second), (&other, &first)] {
        let checkouts = repo_checkouts(&projects, cwd, repos);
        assert_eq!(checkouts.len(), 2, "each repository is fetched once");
        assert!(
            checkouts
                .iter()
                .any(|(project, repo)| *project == owner && repo == "Acme/App"),
            "from {cwd:?}: {checkouts:?}"
        );
    }
}
