#[test]
fn stat_details_lend_their_leading_change_to_the_delta() {
    assert_eq!(
        super::split_delta("+12.4% vs last month"),
        Some(("+12.4%", "vs last month"))
    );
    assert_eq!(super::split_delta("-3"), Some(("-3", "")));
    assert_eq!(super::split_delta("requests per day"), None);
    assert_eq!(super::split_delta("+ more"), None);
}
