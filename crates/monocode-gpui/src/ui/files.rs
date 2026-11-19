//! File references shared across views: open-path action, locations, dragged files.

use crate::ui::file_icons::file_type_icon;
use crate::ui::{panel::stroke, theme::*};
use gpui::{Context, Window, div, prelude::*, px};
use monocode_core::files::document::{self};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = file_editor, no_json)]
pub(crate) struct OpenPath(pub String);

/// File links may include a one-based line and column (`:12:4` or `#L12`).
pub(crate) fn file_location(raw: &str, cwd: &str) -> (String, Option<(usize, usize)>) {
    let mut path = raw.strip_prefix("file://").unwrap_or(raw).to_owned();
    let mut location = None;
    if let Some((file, line)) = path.rsplit_once("#L")
        && let Ok(line) = line.split('-').next().unwrap_or("").parse::<usize>()
    {
        location = Some((line.max(1), 1));
        path = file.into();
    } else if let Some((prefix, number)) = path.rsplit_once(':')
        && let Ok(last) = number.parse::<usize>()
    {
        if let Some((file, line)) = prefix.rsplit_once(':')
            && let Ok(line) = line.parse::<usize>()
        {
            location = Some((line.max(1), last.max(1)));
            path = file.into();
        } else {
            location = Some((last.max(1), 1));
            path = prefix.into();
        }
    }
    if raw.starts_with("file://") || raw.contains('%') {
        let bytes = path.as_bytes();
        let mut decoded = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            if bytes[at] == b'%'
                && at + 2 < bytes.len()
                && let Ok(hex) = std::str::from_utf8(&bytes[at + 1..at + 3])
                && let Ok(byte) = u8::from_str_radix(hex, 16)
            {
                decoded.push(byte);
                at += 3;
            } else {
                decoded.push(bytes[at]);
                at += 1;
            }
        }
        path = String::from_utf8_lossy(&decoded).into_owned();
    }
    let path = document::resolve_path(std::path::Path::new(&path), std::path::Path::new(cwd));
    (path.to_string_lossy().into_owned(), location)
}

/// A reference to an existing file, distinct from files copied into the tree.
#[derive(Clone)]
pub(crate) struct DraggedFile(pub PathBuf);

impl Render for DraggedFile {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let name = self
            .0
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        div()
            .flex()
            .items_center()
            .gap_1p5()
            .px_2()
            .py_1p5()
            .rounded_md()
            .border_1()
            .border_color(stroke())
            .bg(pane_fill())
            .text_size(px(12.))
            .text_color(content(1.))
            .child(file_type_icon(&name, false, false, 16.))
            .child(div().max_w(px(240.)).truncate().child(name))
    }
}
