//! Application shell: window, workspace, menus and startup diagnostics.

pub(crate) mod app_menu;
#[cfg(debug_assertions)]
pub(crate) mod dev_reload;
pub(crate) mod diagnostics;
#[cfg(unix)]
pub(crate) mod instance;
pub(crate) mod shell;
pub(crate) mod window_state;
pub(crate) mod workspace;
