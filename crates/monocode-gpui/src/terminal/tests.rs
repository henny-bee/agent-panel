use super::*;
use alacritty_terminal::index::Line;
use gpui::{AppContext, Modifiers};

#[test]
fn mouse_reports_match_the_requested_encoding() {
    let sgr = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
    let x10 = TermMode::MOUSE_REPORT_CLICK;
    assert_eq!(mouse_report(64, false, 4, 9, sgr), b"\x1b[<64;10;5M");
    assert_eq!(mouse_report(65, false, 0, 0, sgr), b"\x1b[<65;1;1M");
    assert_eq!(
        mouse_report(64, false, 4, 9, x10),
        [0x1b, b'[', b'M', 96, 42, 37]
    );
    // A left click, its release and a drag.
    assert_eq!(mouse_report(0, false, 4, 9, sgr), b"\x1b[<0;10;5M");
    assert_eq!(mouse_report(0, true, 4, 9, sgr), b"\x1b[<0;10;5m");
    assert_eq!(mouse_report(32, false, 4, 9, sgr), b"\x1b[<32;10;5M");
    assert_eq!(
        mouse_report(0, false, 4, 9, x10),
        [0x1b, b'[', b'M', 32, 42, 37]
    );
    assert_eq!(
        mouse_report(0, true, 4, 9, x10),
        [0x1b, b'[', b'M', 35, 42, 37]
    );
}

fn key(spec: &str) -> Keystroke {
    Keystroke::parse(spec).unwrap()
}
fn bytes(spec: &str) -> Option<Vec<u8>> {
    key_bytes(&key(spec), TermMode::empty())
}

#[gpui::test]
fn links_cover_plain_urls_and_osc8_hyperlinks(cx: &mut gpui::TestAppContext) {
    let view = cx.new(|cx| Terminal::new("/tmp".into(), cx));
    view.update(cx, |terminal, _| {
        let cell = size(px(8.), px(16.));
        terminal.resize(20, 4, cell);
        terminal.geometry.set(Some(Geometry {
            origin: point(px(0.), px(0.)),
            cell,
        }));
        // A URL wrapping onto the next row, trailing punctuation
        // excluded; then an OSC 8 link and a file:// one.
        write_note(
            &terminal.term,
            "see https://example.com/a/b. ok\r\n\x1b]8;;https://x.dev\x1b\\docs\x1b]8;;\x1b\\ \
                 \x1b]8;;file:///tmp\x1b\\tmp\x1b]8;;\x1b\\ ftp://a.b",
        );
        let at = |line: usize, column: usize| {
            point(px(column as f32 * 8. + 1.), px(line as f32 * 16. + 1.))
        };
        let url = terminal.link_at(at(1, 2)).expect("wrapped URL");
        assert_eq!(url.uri, "https://example.com/a/b");
        assert_eq!(
            url.range,
            GridPoint::new(Line(0), Column(4))..=GridPoint::new(Line(1), Column(6))
        );
        assert_eq!(terminal.link_at(at(0, 4)), Some(url));
        assert_eq!(terminal.link_at(at(0, 1)), None);
        assert_eq!(terminal.link_at(at(1, 7)), None);

        let docs = terminal.link_at(at(2, 1)).expect("OSC 8 link");
        assert_eq!(docs.uri, "https://x.dev");
        assert_eq!(
            docs.range,
            GridPoint::new(Line(2), Column(0))..=GridPoint::new(Line(2), Column(3))
        );
        assert_eq!(terminal.link_at(at(2, 6)), None, "file:// stays inert");
        assert_eq!(terminal.link_at(at(2, 10)), None, "ftp:// stays inert");
        assert_eq!(terminal.link_at(point(px(-4.), px(1.))), None);
    });
}

/// A folder with `src/main.rs` and `app/`, unique to this test.
fn path_fixture(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mcode-{name}-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("app")).unwrap();
    std::fs::write(dir.join("src/main.rs"), "").unwrap();
    dir.to_string_lossy().into_owned()
}
fn cell_at(line: usize, column: usize) -> gpui::Point<gpui::Pixels> {
    point(px(column as f32 * 8. + 1.), px(line as f32 * 16. + 1.))
}

#[gpui::test]
fn links_cover_paths_that_exist(cx: &mut gpui::TestAppContext) {
    let root = path_fixture("paths");
    let view = cx.new(|cx| Terminal::new(root.clone(), cx));
    view.update(cx, |terminal, _| {
        let cell = size(px(8.), px(16.));
        // Wide enough that a long temp folder does not wrap.
        terminal.resize(200, 4, cell);
        terminal.geometry.set(Some(Geometry {
            origin: point(px(0.), px(0.)),
            cell,
        }));
        write_note(
            &terminal.term,
            &format!(
                "Read(src/main.rs) and src/main.rs:42:7,\r\n\
                 open `app/` or src/gone.rs, not src\r\n{root}/app/."
            ),
        );
        let span = |line, from, to| {
            GridPoint::new(Line(line), Column(from))..=GridPoint::new(Line(line), Column(to))
        };
        // A tool line's path, without the call around it.
        let read = terminal.link_at(cell_at(0, 7)).expect("Read(path)");
        assert_eq!(read.target, Target::File);
        assert_eq!(read.uri, format!("{root}/src/main.rs"));
        assert_eq!(read.range, span(0, 5, 15));
        assert_eq!(terminal.link_at(cell_at(0, 1)), None, "the call name");
        // A line and column, trailing comma dropped.
        let at_line = terminal.link_at(cell_at(0, 25)).expect("path:line:col");
        assert_eq!(at_line.uri, format!("{root}/src/main.rs:42:7"));
        assert_eq!(at_line.range, span(0, 22, 37));
        assert_eq!(terminal.link_at(cell_at(0, 19)), None);

        let app = terminal.link_at(cell_at(1, 7)).expect("relative folder");
        assert_eq!(app.target, Target::Folder);
        assert_eq!(app.range, span(1, 6, 9));
        assert_eq!(terminal.link_at(cell_at(1, 17)), None, "missing file");
        assert_eq!(terminal.link_at(cell_at(1, 33)), None, "bare word");

        let absolute = terminal.link_at(cell_at(2, 2)).expect("absolute folder");
        assert_eq!(absolute.target, Target::Folder);
        assert_eq!(absolute.range, span(2, 0, root.len() + 4));
    });
    std::fs::remove_dir_all(&root).ok();
}

#[gpui::test]
fn ctrl_click_takes_links_from_a_program_tracking_the_mouse(cx: &mut gpui::TestAppContext) {
    let root = path_fixture("ctrl-click");
    let window = cx.add_window(|_, cx| Terminal::new(root.clone(), cx));
    window
        .update(cx, |terminal, window, cx| {
            let cell = size(px(8.), px(16.));
            terminal.resize(60, 4, cell);
            terminal.geometry.set(Some(Geometry {
                origin: point(px(0.), px(0.)),
                cell,
            }));
            // As the Claude CLI does: mouse tracking with SGR reports.
            write_note(&terminal.term, "\x1b[?1000h\x1b[?1006hsee src/main.rs now");
            let press = |position, modifiers| MouseDownEvent {
                button: MouseButton::Left,
                position,
                modifiers,
                click_count: 1,
                first_mouse: false,
            };
            let release = |position, modifiers| gpui::MouseUpEvent {
                button: MouseButton::Left,
                position,
                modifiers,
                click_count: 1,
            };
            let ctrl = Modifiers::secondary_key();
            let (link, text) = (cell_at(0, 6), cell_at(0, 1));

            // A plain click on the path still goes to the program.
            terminal.mouse_down(&press(link, Modifiers::default()), window, cx);
            assert!(terminal.reported.is_some());
            assert_eq!(terminal.pressed_link, None);
            terminal.mouse_up(&release(link, Modifiers::default()), window, cx);
            assert!(terminal.reported.is_none());

            // Ctrl+click on it is the terminal's, and the program never hears it.
            terminal.mouse_down(&press(link, ctrl), window, cx);
            assert!(terminal.reported.is_none());
            let pressed = terminal.pressed_link.clone().expect("Ctrl+click on a path");
            assert_eq!(pressed.uri, format!("{root}/src/main.rs"));
            terminal.mouse_up(&release(link, ctrl), window, cx);
            assert_eq!(terminal.pressed_link, None);
            assert!(terminal.reported.is_none());

            // Ctrl+click off any link is the program's again.
            terminal.mouse_down(&press(text, ctrl), window, cx);
            assert!(terminal.reported.is_some());
            assert_eq!(terminal.pressed_link, None);
        })
        .unwrap();
    std::fs::remove_dir_all(&root).ok();
}

#[gpui::test]
fn dragging_at_edges_extends_selection_through_history(cx: &mut gpui::TestAppContext) {
    let view = cx.new(|cx| Terminal::new("/tmp".into(), cx));
    view.update(cx, |terminal, cx| {
        let cell = size(px(8.), px(16.));
        terminal.resize(40, 4, cell);
        terminal.geometry.set(Some(Geometry {
            origin: point(px(0.), px(0.)),
            cell,
        }));
        let output = (0..30)
            .map(|n| format!("line-{n:02}"))
            .collect::<Vec<_>>()
            .join("\r\n");
        write_note(&terminal.term, &output);
        terminal.term.lock().selection = Some(Selection::new(
            SelectionType::Simple,
            GridPoint::new(Line(3), Column(6)),
            Side::Right,
        ));
        terminal.selecting = true;
        // Holding a click alone must not scroll.
        assert!(terminal.scroll_selection(cx));
        assert_eq!(terminal.term.lock().grid().display_offset(), 0);
        terminal.selection_pointer = Some(point(px(0.), px(0.)));
        assert!(terminal.scroll_selection(cx));
        assert_eq!(terminal.term.lock().grid().display_offset(), 1);
        let text = terminal.term.lock().selection_to_string().unwrap();
        assert!(text.contains("line-25"), "{text}");
        assert!(text.contains("line-29"), "{text}");
        // A second tick at the same pointer position must keep extending.
        assert!(terminal.scroll_selection(cx));
        assert!(
            terminal
                .term
                .lock()
                .selection_to_string()
                .unwrap()
                .contains("line-24")
        );
        terminal.selection_pointer = Some(point(px(0.), px(32.)));
        terminal.scroll_selection(cx);
        assert_eq!(terminal.term.lock().grid().display_offset(), 2);
        // Dragging past the bottom returns toward the live screen.
        terminal.selection_pointer = Some(point(px(0.), px(80.)));
        terminal.scroll_selection(cx);
        assert_eq!(terminal.term.lock().grid().display_offset(), 0);
        terminal.stop_selection();
        assert!(!terminal.scroll_selection(cx));
    });
}

#[test]
fn keys_encode_like_xterm_with_the_macos_editing_shortcuts() {
    assert_eq!(bytes("enter"), Some(b"\r".to_vec()));
    assert_eq!(bytes("backspace"), Some(b"\x7f".to_vec()));
    assert_eq!(bytes("up"), Some(b"\x1b[A".to_vec()));
    assert_eq!(
        key_bytes(&key("up"), TermMode::APP_CURSOR),
        Some(b"\x1bOA".to_vec())
    );
    assert_eq!(bytes("shift-right"), Some(b"\x1b[1;2C".to_vec()));
    assert_eq!(bytes("ctrl-c"), Some(vec![3]));
    assert_eq!(bytes("ctrl-["), Some(vec![27]));
    assert_eq!(bytes("pagedown"), Some(b"\x1b[6~".to_vec()));
    assert_eq!(bytes("f5"), Some(b"\x1b[15~".to_vec()));
    assert_eq!(bytes("shift-tab"), Some(b"\x1b[Z".to_vec()));
    // terminalKeys.ts
    assert_eq!(bytes("alt-left"), Some(b"\x1bb".to_vec()));
    assert_eq!(bytes("alt-right"), Some(b"\x1bf".to_vec()));
    assert_eq!(bytes("cmd-left"), Some(b"\x01".to_vec()));
    assert_eq!(bytes("cmd-right"), Some(b"\x05".to_vec()));
    assert_eq!(bytes("cmd-backspace"), Some(b"\x15".to_vec()));
    // Option is Meta.
    assert_eq!(bytes("alt-b"), Some(b"\x1bb".to_vec()));
    // App shortcuts stay with the app.
    assert_eq!(bytes("cmd-j"), None);
    assert_eq!(bytes("cmd-shift-d"), None);
    let typed = Keystroke {
        modifiers: Modifiers::default(),
        key: "a".into(),
        key_char: Some("a".into()),
    };
    assert_eq!(key_bytes(&typed, TermMode::empty()), Some(b"a".to_vec()));
}

#[test]
fn agent_clis_map_to_their_provider() {
    assert_eq!(agent_cli("claude"), Some("claude"));
    assert_eq!(agent_cli("codex.js"), Some("codex"));
    assert_eq!(agent_cli("cursor-agent"), Some("cursor"));
    assert_eq!(agent_cli("pi-coding-agent"), Some("pi"));
    assert_eq!(agent_cli("vim"), None);
}
#[test]
fn foreground_labels_name_the_script_an_interpreter_runs() {
    assert_eq!(
        command_label("node /x/node_modules/.bin/vite --port 3000").as_deref(),
        Some("vite")
    );
    assert_eq!(
        command_label("/usr/bin/python3 -m http.server").as_deref(),
        Some("http.server")
    );
    assert_eq!(
        command_label("cargo watch -x test").as_deref(),
        Some("cargo")
    );
    assert_eq!(command_label("-zsh").as_deref(), Some("zsh"));
    assert!(is_shell_name("zsh"));
    assert_eq!(default_title("/work/monocode"), "monocode");
    assert_eq!(default_title("/"), "Terminal");
}

#[test]
fn colours_follow_the_dark_palette_and_xterm_cube() {
    assert_eq!(indexed_rgb(1), 0xf87171);
    assert_eq!(indexed_rgb(16), 0x000000);
    assert_eq!(indexed_rgb(196), 0xff0000);
    assert_eq!(indexed_rgb(232), 0x080808);
    assert_eq!(indexed_rgb(255), 0xeeeeee);
    let colors = Colors::default();
    assert_eq!(hex(color_rgb(257, &colors)), OSC_BACKGROUND);
    assert_eq!(hex(color_rgb(258, &colors)), ACCENT);
}

#[test]
fn a_login_shell_runs_and_its_output_reaches_the_grid() {
    let dir = std::env::temp_dir().to_string_lossy().into_owned();
    let size = GridSize {
        columns: 60,
        lines: 10,
    };
    let spawned = spawn(&dir, size, 1, None);
    let sender = spawned.sender.expect("the shell started");
    assert!(spawned.shell_pid > 1);
    // Both print mono-code without the typed line containing it. Windows
    // runs PowerShell, which has no printf, or Bash when SHELL names it;
    // both expand an empty $().
    let command: &[u8] = if cfg!(windows) {
        b"echo mono$()-code\r"
    } else {
        b"printf 'mono%scode\\n' -\r"
    };
    let _ = sender.send(Msg::Input(Cow::Borrowed(command)));
    let screen = || {
        let term = spawned.term.lock();
        let grid = term.grid();
        (0..grid.screen_lines())
            .map(|line| {
                (0..grid.columns())
                    .map(|c| grid[Line(line as i32)][Column(c)].c)
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut seen = false;
    // PowerShell can take seconds to start; a fast shell stops early.
    for _ in 0..750 {
        std::thread::sleep(Duration::from_millis(20));
        if screen().contains("mono-code") {
            seen = true;
            break;
        }
    }
    assert!(seen, "{}", screen());
    assert_eq!(spawned.term.lock().columns(), 60);
    // The shell owns the foreground while it waits at a prompt.
    assert_eq!(foreground_label(spawned.master_fd, spawned.shell_pid), None);
    let _ = sender.send(Msg::Input(Cow::Borrowed(b"exit\r")));
    let mut exited = false;
    for _ in 0..750 {
        std::thread::sleep(Duration::from_millis(20));
        if let Ok(TermEvent::ChildExit(_)) = spawned.events.try_recv() {
            exited = true;
            break;
        }
    }
    assert!(exited);
}
