mod apps;
mod completion;
mod details;
mod habit_pages;
mod pages;
mod panel;
use super::*;
use gpui::{TestAppContext, WindowHandle};
use monocode_core::store::Store;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, mpsc};
struct Setup {
    window: WindowHandle<Shell>,
    manager: Entity<Monos>,
    id: String,
    project: String,
    session: i64,
    root: PathBuf,
    store: Database,
}

fn setup(cx: &mut TestAppContext) -> Setup {
    setup_with(cx, |path| Store::open(path).unwrap())
}
/// `setup` over the desktop's shared database, at `<project>/native.sqlite`.
fn setup_shared(cx: &mut TestAppContext) -> Setup {
    setup_with(cx, |path| Store::open_monocode(path).unwrap())
}
fn setup_with(cx: &mut TestAppContext, open: impl FnOnce(&std::path::Path) -> Store) -> Setup {
    let dir = std::env::temp_dir().join(format!("native-monos-{}", mono::new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    let project = dir.to_string_lossy().to_string();
    let store = Arc::new(Mutex::new(open(&dir.join("native.sqlite"))));
    let db = store.clone();
    let cwd = project.clone();
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| Shell::new(db, Vec::new(), cwd, None, cx))
        })
        .unwrap()
    });
    cx.run_until_parked();
    let manager = window
        .update(cx, |shell, _, cx| {
            shell.workspace.right_panel_open = true;
            cx.notify();
            shell.mono_manager()
        })
        .unwrap();
    let handle: gpui::AnyWindowHandle = window.into();
    handle
        .update(cx, |_, w, cx| {
            manager.update(cx, |m, cx| m.create(vec![project.clone()], w, cx))
        })
        .unwrap();
    cx.run_until_parked();
    let (id, session, root) = cx.update(|cx| {
        let m = manager.read(cx);
        let mono = m.current().unwrap();
        (mono.id.clone(), mono.session_id.unwrap(), m.root.clone())
    });
    Setup {
        window,
        manager,
        id,
        project,
        session,
        root,
        store,
    }
}

fn call(
    s: &Setup,
    request: &str,
    action: &str,
    input: Value,
    cx: &mut TestAppContext,
) -> mpsc::Receiver<Result<Value, String>> {
    let (reply, rx) = mpsc::channel();
    let call = mono::bridge::Call {
        session: s.session,
        request_id: request.into(),
        action: action.into(),
        input,
        reply,
    };
    let handle: gpui::AnyWindowHandle = s.window.into();
    handle
        .update(cx, |_, w, cx| {
            s.manager.update(cx, |m, cx| m.app_call(call, w, cx))
        })
        .unwrap();
    cx.run_until_parked();
    rx
}
