use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn inline_selected(
    runs: &[InlineRun],
    id: SharedString,
    tone: Tone,
    fade: Option<&crate::ui::motion::reveal::WordFades>,
    now: Instant,
    word_base: usize,
    font_size: f32,
    selection: Option<TextSelection>,
    prepared: Option<&cache::PreparedInline>,
) -> AnyElement {
    if runs.iter().any(|r| r.style.code) {
        return rich_inline(runs, id, tone, fade, now, word_base, font_size, selection);
    }
    let uncached;
    let prepared = if let Some(prepared) = prepared {
        prepared
    } else {
        uncached = cache::PreparedInline::new(runs, false);
        &uncached
    };
    let mut highlights = Vec::new();
    for run in &prepared.runs {
        let style = &run.style;
        let color = if style.link.is_some() {
            accent_color()
        } else if style.bold || style.italic || style.code {
            content(tone.strong())
        } else {
            content(tone.prose())
        };
        let base_opacity = color.a;
        let mut opaque_color = color;
        opaque_color.a = 1.;
        let highlight = HighlightStyle {
            color: Some(opaque_color),
            background_color: style.search.map(search::search_background),
            fade_out: Some(1. - base_opacity),
            font_weight: style.bold.then_some(FontWeight::SEMIBOLD),
            font_style: style.italic.then_some(FontStyle::Italic),
            strikethrough: style.strikethrough.then_some(StrikethroughStyle {
                thickness: px(1.),
                color: None,
            }),
            ..Default::default()
        };
        if fade.is_some_and(|fade| {
            fade.intersects(word_base + run.word_start..word_base + run.word_start + run.words)
        }) {
            for (range, word) in &run.pieces {
                let mut highlight = highlight;
                if let Some(word) = word {
                    let alpha = fade.map_or(1., |fade| fade.opacity(word_base + *word));
                    highlight.fade_out = Some(1. - (1. - highlight.fade_out.unwrap_or(0.)) * alpha);
                }
                highlights.push((range.clone(), highlight));
            }
        } else {
            highlights.push((run.range.clone(), highlight));
        }
    }
    let text = prepared.text.clone();
    let links = prepared.links.clone();
    let urls = prepared.urls.clone();
    let styled = StyledText::new(text.clone())
        .with_highlights(highlights)
        .with_font_family_overrides(prepared.fonts.clone());
    let layout = styled.layout().clone();
    let clicked_selection = selection.clone();
    let clicked_layout = layout.clone();
    let menu_layout = layout.clone();
    let clicked_links = links.clone();
    let menu_links = links.clone();
    let clicked_urls = urls.clone();
    let text_id: SharedString = format!("{id}-text").into();
    let child = div()
        .id(id)
        .child(InteractiveText::new(text_id, styled).on_click(links, |_, _, _| {}))
        .on_click(move |event, window, cx| {
            if !activate_link(event, clicked_selection.as_ref()) {
                return;
            }
            let Ok(byte) = clicked_layout.index_for_position(event.position()) else {
                return;
            };
            if let Some(ix) = clicked_links.iter().position(|range| range.contains(&byte)) {
                // A drag starting on another link must not activate this one.
                if let ClickEvent::Mouse(mouse) = event {
                    let Ok(down) = clicked_layout.index_for_position(mouse.down.position) else {
                        return;
                    };
                    if !clicked_links[ix].contains(&down) {
                        return;
                    }
                }
                open_link(&clicked_urls[ix], window, cx);
            }
        })
        .on_mouse_down(MouseButton::Right, move |event, window, cx| {
            if let Ok(byte) = menu_layout.index_for_position(event.position)
                && let Some(ix) = menu_links.iter().position(|range| range.contains(&byte))
            {
                link_menu(&urls[ix], event.position, window, cx);
                cx.stop_propagation();
            }
        })
        .into_any_element();
    if let Some(selection) = selection {
        selection.wrap(text, layout, child)
    } else {
        child
    }
}

pub(crate) fn body_selected(
    row: &MarkdownRow,
    id: SharedString,
    tone: Tone,
    fade: Option<&crate::ui::motion::reveal::WordFades>,
    now: Instant,
    selection: Option<TextSelection>,
    prepared: Option<&PreparedRow>,
) -> AnyElement {
    // Share one logical row across its styled fragments. Selection boundaries
    // then span inline code and formatting without scanning transcript history.
    let selection = selection.map(|selection| {
        selection.with_text(prepared.map_or_else(
            || selection_text(row).into(),
            |prepared| prepared.selection.clone(),
        ))
    });
    let mut body = div()
        .w_full()
        .min_w_0()
        .text_size(px(14.))
        .line_height(px(24.));
    match &row.part {
        Part::Text { runs, heading } => {
            let mut runs = std::borrow::Cow::Borrowed(runs.as_slice());
            if *heading > 0 && prepared.and_then(|p| p.inline.as_ref()).is_none() {
                for run in runs.to_mut() {
                    run.style.bold = true;
                }
            }
            let heading = if tone == Tone::Work { 0 } else { *heading };
            if heading > 0 {
                body = body
                    .text_size(px(match heading {
                        1 => 22.,
                        2 => 18.,
                        _ => 16.,
                    }))
                    .font_weight(FontWeight::SEMIBOLD)
                    .line_height(px(if heading == 1 { 30. } else { 27. }));
            }
            body.child(inline_selected(
                &runs,
                id,
                tone,
                fade,
                now,
                row.word_start,
                match heading {
                    1 => 22.,
                    2 => 18.,
                    0 => 14.,
                    _ => 16.,
                },
                selection,
                prepared.and_then(|prepared| prepared.inline.as_ref()),
            ))
            .into_any_element()
        }
        Part::Rule => body
            .py_2()
            .child(div().w_full().h(px(1.)).bg(content(0.1)))
            .into_any_element(),
        // Only the transcript can draw a session's visuals; elsewhere the
        // caption stands in for it.
        Part::Visual { caption, .. } => body
            .text_color(content(0.5))
            .child(if caption.is_empty() {
                "Visual".to_owned()
            } else {
                caption.clone()
            })
            .into_any_element(),
        Part::Math(tex) => {
            let styled = StyledText::new(tex.clone());
            let layout = styled.layout().clone();
            let child = styled.into_any_element();
            let child = if let Some(selection) = selection {
                selection.wrap(tex.clone().into(), layout, child)
            } else {
                child
            };
            body.font_family(crate::ui::fonts::mono())
                .text_color(content(0.78))
                .child(child)
                .into_any_element()
        }
        Part::Table {
            cells,
            columns,
            align,
            header,
            last,
        } => body
            .id(id.clone())
            .overflow_x_scroll()
            .rounded_t(px(if *header { 10. } else { 0. }))
            .rounded_b(px(if *last { 10. } else { 0. }))
            .border_1()
            .border_color(content(0.1))
            .bg(content(0.06))
            .child(
                div()
                    .w_full()
                    .min_w(px(columns.len() as f32 * 160.))
                    .flex()
                    .flex_col()
                    .children(cells.iter().enumerate().map(|(ix, cells)| {
                        div()
                            .w_full()
                            .flex()
                            .when(ix > 0 || !*header, |d| {
                                d.border_t_1().border_color(content(0.05))
                            })
                            .children(columns.iter().enumerate().map(|(col, width)| {
                                let cell = cells.get(col).map(Vec::as_slice).unwrap_or_default();
                                div()
                                    .w(px(*width))
                                    .min_w(px(160.))
                                    .flex_grow(*width)
                                    .flex_shrink_1()
                                    .px(px(10.))
                                    .py_2()
                                    .text_size(px(12.))
                                    .line_height(px(18.))
                                    .when(*header && ix == 0, |d| {
                                        d.font_weight(FontWeight::SEMIBOLD)
                                    })
                                    .when(align.get(col) == Some(&TableAlign::Center), |d| {
                                        d.text_center()
                                    })
                                    .when(align.get(col) == Some(&TableAlign::Right), |d| {
                                        d.text_right()
                                    })
                                    .child(inline_selected(
                                        cell,
                                        format!("{id}-{ix}-{col}").into(),
                                        tone,
                                        None,
                                        now,
                                        0,
                                        12.,
                                        selection.as_ref().map(|selection| {
                                            selection.offset(table_offset(&row.part, ix, col))
                                        }),
                                        None,
                                    ))
                            }))
                    })),
            )
            .into_any_element(),
        Part::Code { .. } => body.into_any_element(),
        Part::Mermaid { fence } => body
            .child(mermaid::Diagram {
                id,
                fence: fence.clone(),
                selection,
            })
            .into_any_element(),
    }
}

pub(crate) fn code_lines_selected(
    fence: &Fence,
    range: Range<usize>,
    highlighted: Option<&HighlightedDocument>,
    query: &str,
    current: bool,
    selection: Option<TextSelection>,
) -> Div {
    let selection = selection.map(|selection| {
        selection.with_text(
            range
                .clone()
                .map(|ix| fence.code[fence.lines[ix].clone()].to_owned())
                .collect::<Vec<_>>()
                .join("\n")
                .into(),
        )
    });
    let start = range.start;
    let longest = fence
        .lines
        .iter()
        .map(|r| fence.code[r.clone()].chars().count())
        .max()
        .unwrap_or(0);
    div()
        .w(px(longest as f32 * 7.3 + 48.))
        .min_w_full()
        .flex()
        .flex_col()
        .children(range.map(|ix| {
            let line = &fence.code[fence.lines[ix].clone()];
            let styles = highlighted
                .and_then(|h| h.lines.get(ix))
                .into_iter()
                .flatten()
                .map(|span| {
                    (
                        span.range.clone(),
                        HighlightStyle {
                            color: Some(rgb(span.color).into()),
                            ..Default::default()
                        },
                    )
                })
                .collect();
            let styles = mark_styles(line, styles, query, current);
            let text: SharedString = if line.is_empty() {
                " ".into()
            } else {
                line.to_owned().into()
            };
            let styled = StyledText::new(text.clone()).with_highlights(styles);
            let layout = styled.layout().clone();
            let child = styled.into_any_element();
            let child = if let Some(selection) = selection.as_ref() {
                let offset = (start..ix).map(|line| fence.lines[line].len() + 1).sum();
                selection.offset(offset).wrap(text, layout, child)
            } else {
                child
            };
            div()
                .flex()
                .min_w_full()
                .line_height(px(20.))
                .text_size(px(12.))
                .font_family(crate::ui::fonts::mono())
                .when(fence.numbered, |d| {
                    d.child(
                        div()
                            .w(px(24.))
                            .mr_2()
                            .flex_shrink_0()
                            .text_right()
                            .text_size(px(10.))
                            .text_color(content(0.35))
                            .child((ix + fence.start_line).to_string()),
                    )
                })
                .when(!fence.numbered, |d| d.pl(px(10.)))
                .child(child)
        }))
}

pub(super) fn table_offset(part: &Part, row: usize, col: usize) -> usize {
    let Part::Table { cells, .. } = part else {
        return 0;
    };
    cells
        .iter()
        .take(row)
        .map(|row| {
            row.iter().map(|cell| runs_text(cell).len()).sum::<usize>()
                + row.len().saturating_sub(1)
                + 1
        })
        .sum::<usize>()
        + cells[row]
            .iter()
            .take(col)
            .map(|cell| runs_text(cell).len() + 1)
            .sum::<usize>()
}
