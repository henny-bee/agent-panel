use super::*;
use chrono::TimeZone;

#[test]
fn heartbeat_seeds_waits_for_quiet_then_raises_a_left_session_once() {
    let dir = std::env::temp_dir().join(format!("mono-heartbeat-{}", mono::new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    let project = dir.to_string_lossy().to_string();
    let store: Database = std::sync::Arc::new(std::sync::Mutex::new(
        monocode_core::store::Store::open(&dir.join("native.sqlite")).unwrap(),
    ));
    let root = dir.join("monos");
    let (chat, left) = {
        let mut db = locked(&store).unwrap();
        let chat = db.create("codex", &project).unwrap();
        let left = db.create("codex", &project).unwrap();
        db.title(left.id, "Hermes effort").unwrap();
        let mut t = monocode_core::session::transcript::Transcript::default();
        t.begin("user");
        t.append("lets do option B");
        t.begin("assistant");
        t.append("Plan ready. Want me to start?");
        db.write_rows(left.id, 0, &t.rows).unwrap();
        (chat.id, left.id)
    };
    let beat = HeartbeatCandidate {
        mono: Mono {
            session_id: Some(chat),
            projects: vec![project.clone()],
            ..Default::default()
        },
        skip: HashSet::from([chat]),
    };
    let no_inbox: fn(&Database, &Mono) -> Vec<monocode_core::integrations::inbox::Item> =
        |_, _| Vec::new();
    let t = now_ms();
    let min = 60_000;
    let prepare = |at| prepare_heartbeat(&store, &root, &beat, at, no_inbox).unwrap();
    assert_eq!(prepare(t), None, "the first sight only seeds the cursor");
    assert_eq!(prepare(t + 10 * min), None, "not quiet for long enough");
    let text = prepare(t + 2 * heartbeat::IDLE_MS).expect("a left session is worth a heartbeat");
    assert!(text.contains(&format!("\"Hermes effort\" (sessionId {left})")));
    assert!(text.contains("lets do option B"));
    assert!(text.contains("Want me to start?"));
    assert_eq!(
        prepare(t + 2 * heartbeat::IDLE_MS + 31 * min),
        None,
        "nothing new and no standing instructions costs no model call"
    );
    assert_eq!(
        heartbeat::load(&root, &beat.mono.id).unwrap().last_run_at,
        t + 2 * heartbeat::IDLE_MS + 31 * min
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn habits_list_groups_today_with_running_first_and_paused_last() {
    let at = |day, hour| {
        chrono::Local
            .with_ymd_and_hms(2026, 10, day, hour, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64
    };
    let now = at(12, 12);
    let make = |name: &str, next_run_at| {
        let mut h = Habit::new(
            name.into(),
            "Check updates".into(),
            Schedule::default(),
            now,
        )
        .unwrap();
        h.id = name.into();
        h.next_run_at = next_run_at;
        h
    };
    let mut morning = make("Morning", at(13, 9));
    morning.last_run_at = Some(at(12, 8));
    let mut paused = make("Paused", at(13, 7));
    paused.enabled = false;
    let mut starting = make("Starting", at(13, 9));
    starting.run_requested = true;
    let habits = [
        make("Weekly", at(16, 9)),
        make("Evening", at(12, 18)),
        morning,
        paused,
        starting,
        make("Running", at(13, 9)),
    ];
    assert_eq!(
        grouped_habits(&habits, now, |h| (h.id == "Running").then_some(at(12, 11))),
        [vec![5, 4, 1], vec![2], vec![0, 3]]
    );
}

#[test]
fn a_habit_that_ran_and_is_due_again_today_is_only_upcoming() {
    let now = chrono::Local
        .with_ymd_and_hms(2026, 10, 12, 12, 0, 0)
        .single()
        .unwrap()
        .timestamp_millis() as u64;
    let mut h = Habit::new(
        "Hourly".into(),
        "Check builds".into(),
        Schedule {
            schedule_kind: "hourly".into(),
            ..Default::default()
        },
        now,
    )
    .unwrap();
    h.last_run_at = Some(now - 60_000);
    assert_eq!(
        grouped_habits(&[h], now, |_| None),
        [vec![0], vec![], vec![]]
    );
    assert_eq!(duration(72_000), "1m 12s");
    assert_eq!(duration(60_000), "1m");
}
