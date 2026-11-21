//! Shared UI building blocks. Depends on nothing above it.

pub(crate) mod assets;
pub(crate) mod avatar;
pub(crate) mod chrome;
pub(crate) mod file_icons;
pub(crate) mod files;
pub(crate) mod fonts;
pub(crate) mod html;
pub(crate) mod image_lightbox;
pub(crate) mod image_memory;
pub(crate) mod input;
pub(crate) mod modal;
pub(crate) mod motion;
pub(crate) mod palettes;
pub(crate) mod panel;
pub(crate) mod popover;
pub(crate) mod preferences;
pub(crate) mod project_badge;
pub(crate) mod session_mark;
pub(crate) mod sound;
pub(crate) mod syntax_palette;
pub(crate) mod tabs;
pub(crate) mod theme;
#[cfg(target_os = "macos")]
pub(crate) mod traffic_lights;
pub(crate) mod widgets;
