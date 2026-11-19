use super::*;

fn day(count: u32) -> Day {
    Day {
        date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        commits: count,
        reviews: 0,
        opened: 0,
        merged: 0,
        additions: 0,
        deletions: 0,
    }
}

fn at(date: NaiveDate, hour: u32) -> u64 {
    date.and_hms_opt(hour, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .unwrap()
        .timestamp_millis() as u64
}

fn item(kind: Kind, reference: &str, at: u64) -> Contribution {
    Contribution {
        session: 1,
        kind,
        reference: reference.into(),
        at,
        additions: 10,
        deletions: 2,
    }
}

#[test]
fn days_cover_whole_weeks_and_count_each_kind() {
    let today = NaiveDate::from_ymd_opt(2026, 10, 29).unwrap();
    let yesterday = today - Days::days(1);
    let items = [
        item(Kind::Commit, "a", at(today, 9)),
        item(Kind::Commit, "b", at(today, 10)),
        item(Kind::Review, "acme/app#1@x", at(today, 11)),
        item(Kind::Opened, "acme/app#2", at(yesterday, 12)),
        item(Kind::Landed, "acme/app#2", at(today, 13)),
        item(Kind::Merged, "acme/app#3", at(today, 14)),
        // Older than the graph.
        item(Kind::Commit, "c", at(today - Days::weeks(40), 9)),
    ];
    let days = days_from(&items, today);
    assert_eq!(days.first().unwrap().date.weekday(), chrono::Weekday::Sun);
    assert_eq!(days.len() as i64, (WEEKS - 1) * 7 + 5);
    let last = days.last().unwrap();
    assert_eq!((last.commits, last.reviews, last.merged), (2, 1, 1));
    assert_eq!((last.additions, last.deletions), (20, 4));
    // Landing counts toward merged pull requests, not contributions.
    assert_eq!(last.count(), 4);
    assert_eq!(days[days.len() - 2].opened, 1);
    assert_eq!(days.iter().map(Day::count).sum::<u32>(), 5);
    assert_eq!(merged_on(&items, today), 2);
    assert_eq!(merged_on(&items, yesterday), 0);
}

#[test]
fn levels_split_active_days_by_quartile() {
    let days: Vec<_> = [0, 1, 2, 3, 4, 5, 6, 7, 8].into_iter().map(day).collect();
    assert_eq!(levels(&days), [0, 1, 1, 1, 2, 2, 3, 3, 4]);
    assert_eq!(levels(&[day(0), day(0)]), [0, 0]);
    assert_eq!(levels(&[day(3)]), [1]);
}

#[test]
fn streaks_count_back_from_today_or_yesterday() {
    let run = |counts: &[u32]| streak(&counts.iter().map(|c| day(*c)).collect::<Vec<_>>());
    assert_eq!(run(&[1, 0, 2, 3, 1]), 3);
    assert_eq!(run(&[1, 2, 3, 0]), 3);
    assert_eq!(run(&[1, 0, 0]), 0);
}
