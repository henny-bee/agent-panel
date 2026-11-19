use super::*;

#[test]
fn bursts_lay_out_inside_the_bubble() {
    assert_eq!(plan_step_count(40.), 2);
    assert_eq!(plan_step_count(200.), 3);
    assert_eq!(plan_step_count(900.), 6);
    let mut rng = Rng(7);
    for (w, h) in [(60., 36.), (320., 36.), (700., 120.)] {
        let (hub, nodes) = make_constellation(w, h, || rng.next());
        assert_eq!(hub, (w / 2., h - INSET));
        let agents = nodes.iter().filter(|n| !n.child).count();
        assert!((3..=6).contains(&agents));
        assert_eq!(nodes.len() - agents, agents / 2);
        for node in &nodes {
            assert!(node.x >= INSET && node.x <= w - INSET, "{node:?}");
            assert!(node.y >= INSET && node.y <= h - INSET, "{node:?}");
        }
    }
}

#[test]
fn keyframes_ease_per_segment_and_hold_their_ends() {
    let stops = [(0., 0.), (0.25, 1.), (1., 0.)];
    assert_eq!(frames(0., &stops, EASE_OUT), 0.);
    assert_eq!(frames(0.25, &stops, EASE_OUT), 1.);
    assert_eq!(frames(1., &stops, EASE_OUT), 0.);
    assert!(frames(0.125, &stops, EASE_OUT) > 0.5);
    assert_eq!(progress(50., 150., 2400.), 0.);
    assert_eq!(progress(9000., 150., 2400.), 1.);
}
