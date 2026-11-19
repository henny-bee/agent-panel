use super::*;
use std::{cell::RefCell, rc::Rc};

#[gpui::test]
fn code_tab_swaps_the_graph_for_languages_and_back(cx: &mut gpui::TestAppContext) {
    let dir = std::env::temp_dir().join(format!("mcode-code-tab-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rs"), "fn main() {}\n".repeat(10)).unwrap();
    std::fs::write(dir.join("app.ts"), "export {};\n").unwrap();
    std::fs::write(dir.join("README.md"), "# Not code\n".repeat(50)).unwrap();
    let root = dir.to_string_lossy().into_owned();
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |window, cx| {
            let git = cx.new(|cx| ProjectGit::new(root, cx));
            cx.new(|cx| {
                let mut graph = History::new(git, true, window, cx);
                graph.set_enabled(true, cx);
                graph
            })
        })
        .unwrap()
    });
    let chosen = Rc::new(RefCell::new(Vec::new()));
    let entity = window.update(cx, |_, _, cx| cx.entity()).unwrap();
    let log = chosen.clone();
    let _events = cx.update(|cx| {
        cx.subscribe(&entity, move |_, event: &HistoryEvent, _| {
            if let HistoryEvent::Code(code) = event {
                log.borrow_mut().push(*code);
            }
        })
    });
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("history-languages").is_none());

    let code = visual.debug_bounds("code-tab").unwrap();
    visual.simulate_click(code.center(), Default::default());
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(
        visual.debug_bounds("history-languages").is_some(),
        "CODE shows the languages"
    );
    window
        .update(visual, |graph, _, _| {
            let shares = graph.languages.clone().unwrap().unwrap();
            let names: Vec<_> = shares.iter().map(|s| s.name).collect();
            assert_eq!(names, ["Rust", "TypeScript"], "Markdown isn't code");
        })
        .unwrap();
    assert_eq!(*chosen.borrow(), [true]);

    let graph = visual.debug_bounds("graph-tab").unwrap();
    visual.simulate_click(graph.center(), Default::default());
    visual.refresh().unwrap();
    assert!(
        visual.debug_bounds("history-languages").is_none(),
        "GRAPH brings the graph back"
    );
    assert_eq!(*chosen.borrow(), [true, false]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn softened_lane_ink_mixes_toward_content() {
    let mixed = soft(0xFFB000);
    let pure: Hsla = rgb(0xFFB000).into();
    assert!(mixed.s < pure.s);
    assert!((over_base(0.).l - Hsla::from(rgb(BACKGROUND_BASE)).l).abs() < 0.01);
}
