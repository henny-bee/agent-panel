//! Project icon and display name.
use crate::ui::theme::*;
use gpui::{AnyElement, ObjectFit, canvas, prelude::*, px, rgb};
use monocode_core::project::appearance::ProjectAppearance;
use std::path::PathBuf;

pub(crate) fn project_icon(appearance: &ProjectAppearance, project: &str, side: f32) -> AnyElement {
    if project.starts_with("remote://") && appearance.logo.is_none() {
        return icon("internet", side)
            .text_color(content(0.7))
            .into_any_element();
    }
    match &appearance.logo {
        Some(path) => crate::ui::image_memory::thumbnail(PathBuf::from(path))
            .size(px(side))
            .object_fit(ObjectFit::Contain)
            .flex_shrink_0()
            .into_any_element(),
        None => mascot(
            appearance.mascot(project),
            side,
            rgb(appearance.color(project)).into(),
        ),
    }
}

pub(crate) fn project_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .filter(|n| !n.is_empty())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.into())
}

pub(crate) fn mascot(index: usize, side: f32, color: gpui::Hsla) -> AnyElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let unit = bounds.size.width / 8.;
            for (y, row) in SPRITES[index].iter().enumerate() {
                for (x, pixel) in row.bytes().enumerate() {
                    if pixel == b'#' {
                        window.paint_quad(gpui::fill(
                            gpui::Bounds::new(
                                bounds.origin + gpui::point(unit * x as f32, unit * y as f32),
                                gpui::size(unit, unit),
                            ),
                            color,
                        ));
                    }
                }
            }
        },
    )
    .size(px(side))
    .flex_shrink_0()
    .into_any_element()
}

// The original 8x8 project sprites from projectMascots.ts.
pub(crate) const SPRITES: [[&str; 8]; 10] = [
    [
        "..#..#..", ".######.", "##.##.##", "########", ".######.", ".#.##.#.", "#.#..#.#",
        "........",
    ],
    [
        "..####..", ".######.", "##.##.##", "########", "########", "########", "########",
        "#.##.##.",
    ],
    [
        "...#....", ".######.", ".#.##.#.", ".######.", ".#....#.", ".######.", "..#..#..",
        "........",
    ],
    [
        ".#....#.", ".##..##.", "########", "#.####.#", "########", "###..###", ".######.",
        "..#..#..",
    ],
    [
        ".######.", "########", "##.##.##", "########", ".##..##.", ".######.", ".#.##.#.",
        "........",
    ],
    [
        "#......#", ".#....#.", ".######.", "##.##.##", "########", "#.####.#", "#......#",
        "........",
    ],
    [
        "..####..", ".######.", "########", "##.##.##", "########", "...##...", "...##...",
        "..####..",
    ],
    [
        "...##...", "..####..", "..#..#..", "..####..", ".######.", ".######.", "##....##",
        "..####..",
    ],
    [
        "...#####", "...##.##", "...#####", ".#######", "########", "#####...", ".##.##..",
        "..#..#..",
    ],
    [
        "........", "##....##", "#.####.#", "########", "########", ".######.", "##....##",
        "........",
    ],
];
