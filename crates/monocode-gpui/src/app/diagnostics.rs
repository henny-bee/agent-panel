//! Logs and watchdogs used only by main.rs.

pub(crate) mod app_log;
pub(crate) mod frame_log;
pub(crate) mod gpu_trim;
#[cfg(target_os = "macos")]
pub(crate) mod hang_watchdog;
#[cfg(target_os = "macos")]
pub(crate) mod memory_log;
