//! Retain unchanged text-row heights and share probes within each frame.
use super::*;

#[derive(PartialEq)]
struct LayoutStyle {
    text: gpui::TextStyle,
    rem: Pixels,
    scale: f32,
    reduced_motion: bool,
}

#[derive(Default)]
pub(super) struct Measurements {
    diagram_generation: u64,
    style: Option<LayoutStyle>,
    viewport: gpui::Size<Pixels>,
    rows: HashMap<usize, Pixels>,
    /// Bubble widths by message start, with the content they were measured for.
    prompt_widths: HashMap<usize, (u64, Pixels)>,
    retained: HashMap<(usize, usize), RetainedHeight>,
    frame: u64,
    /// Advances on every frame that may have resized rows for reasons their
    /// own content doesn't show: anything but animation steps notified the
    /// transcript or a view inside it, or rows or panels are moving.
    layout_epoch: u64,
    #[cfg(test)]
    pub(super) fresh: usize,
    #[cfg(test)]
    fresh_prompt_widths: usize,
}
struct RetainedHeight {
    height: Pixels,
    row: UiRow,
    after_receipt: bool,
    frame: u64,
    /// The layout epoch it was measured in, for rows whose height also
    /// depends on transcript state; text rows depend on their content alone.
    epoch: Option<u64>,
}

impl Measurements {
    pub(super) fn rows_changed(&mut self) {
        // Prompt widths stay: a rebuilt message checks its fingerprint.
        self.rows.clear();
    }
    /// Heights depend on the viewport. Prompt widths are natural text widths
    /// and the bubble's `max_w` and flex shrinking handle narrow windows.
    fn viewport_changed(&mut self) {
        self.rows.clear();
        self.retained.clear();
    }
    pub(super) fn clear(&mut self) {
        self.viewport_changed();
        self.prompt_widths.clear();
    }
}

pub(super) fn prompt_fingerprint(rows: &[UiRow], attachments: &[attachments::Attachment]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut state = std::collections::hash_map::DefaultHasher::new();
    for row in rows {
        if let Body::Plain(text) = &row.body {
            text.hash(&mut state);
        }
    }
    attachments.len().hash(&mut state);
    for attachment in attachments {
        attachment.hash_layout(&mut state);
    }
    state.finish()
}

/// Shapes a user bubble's text as render lays it out.
pub(super) fn shape_prompt(
    text: SharedString,
    wrap_width: Option<Pixels>,
    window: &Window,
) -> impl Iterator<Item = gpui::WrappedLine> {
    // The transcript inherits the shell's UI font; the bubble sets it too.
    let run = gpui::TextRun {
        len: text.len(),
        font: gpui::font(crate::ui::fonts::UI),
        color: Default::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_text(text, PROMPT_TEXT_SIZE, &[run], wrap_width, None)
        .unwrap_or_default()
        .into_iter()
}

fn prompt_bubble_width(widths: impl Iterator<Item = Pixels>) -> Pixels {
    // Match StyledText's intrinsic measurement. Without rounding up, adding
    // then subtracting the padding can leave less room than the widest line.
    // The minimum also leaves room for the disclosure on a blank last chunk.
    // Text at least as wide as the cap renders exactly as the cap does, so
    // stop shaping there: pasted logs and code usually reach it at once.
    let cap = PROMPT_BUBBLE_MAX - PROMPT_PAD_X;
    let mut widest = PROMPT_DISCLOSURE_MIN;
    for width in widths {
        widest = widest.max(width);
        if widest >= cap {
            return PROMPT_BUBBLE_MAX;
        }
    }
    (widest.ceil() + PROMPT_PAD_X).min(PROMPT_BUBBLE_MAX)
}

impl TranscriptView {
    /// Virtualized chunks share one bubble width, even when only a narrow
    /// continuation is on screen. Single-row prompts keep their natural layout.
    pub(super) fn split_prompt_width(&mut self, ix: usize, window: &Window) -> Option<Pixels> {
        let row = &self.rows[ix];
        let message = &self.messages[row.message];
        if !self.chat_layout || message.role != "user" || (row.first && row.last) {
            return None;
        }
        let fingerprint = message.prompt_fingerprint;
        if let Some((cached, width)) = self.measurements.prompt_widths.get(&message.start)
            && *cached == fingerprint
        {
            return Some(*width);
        }
        let width = if message.attachments.is_empty() {
            prompt_bubble_width(
                self.rows[message.display_start..]
                    .iter()
                    .take_while(|other| other.message == row.message)
                    .filter_map(|other| match &other.body {
                        Body::Plain(text) => Some(text.clone()),
                        _ => None,
                    })
                    .flat_map(|text| shape_prompt(text, None, window))
                    .map(|line| line.width()),
            )
        } else {
            // Attachment chips wrap inside the same column as every text chunk.
            PROMPT_BUBBLE_MAX
        };
        if self.measurements.prompt_widths.len() >= 256 {
            self.measurements.prompt_widths.clear();
        }
        #[cfg(test)]
        {
            self.measurements.fresh_prompt_widths += 1;
        }
        self.measurements
            .prompt_widths
            .insert(message.start, (fingerprint, width));
        Some(width)
    }

    pub(super) fn begin_measurements(&mut self, window: &Window, cx: &Context<Self>) {
        let generation = markdown::mermaid::generation(cx);
        if self.measurements.diagram_generation != generation {
            self.measurements.diagram_generation = generation;
            self.measurements.retained.retain(|_, entry| {
                !matches!(&entry.row.body, Body::Markdown(md) if matches!(md.part, markdown::Part::Mermaid { .. }))
            });
            for (ix, row) in self.rows.iter().enumerate() {
                if matches!(&row.body, Body::Markdown(md) if matches!(md.part, markdown::Part::Mermaid { .. }))
                {
                    self.list.remeasure_items(ix..ix + 1);
                }
            }
        }
        let style = LayoutStyle {
            text: window.text_style(),
            rem: window.rem_size(),
            scale: window.scale_factor(),
            reduced_motion: cx.reduce_motion(),
        };
        if self.measurements.style.as_ref() != Some(&style) {
            // GPUI retains measured item heights independently of text styling.
            self.list.remeasure_items(0..self.list.item_count());
            self.phase_layouts.clear();
            self.measurements.style = Some(style);
            self.measurements.clear();
        }
        if self.measurements.viewport != self.viewport.size {
            self.measurements.viewport_changed();
            self.measurements.viewport = self.viewport.size;
        }
        self.measurements.rows.clear();
        if window.view_layout_dirty(cx.entity_id())
            || !self.motions.is_empty()
            || self.agent_panels.animating()
        {
            self.measurements.layout_epoch = self.measurements.layout_epoch.wrapping_add(1);
        }
        let motions = &self.motions;
        let previous = self.measurements.frame;
        self.measurements.frame = previous.wrapping_add(1);
        self.measurements
            .retained
            .retain(|key, entry| entry.frame == previous && !motions.contains_key(key));
    }

    pub(super) fn measure_row_height(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Pixels {
        // The viewport probe can discover a resize after render's follow pass.
        if self.measurements.viewport != self.viewport.size {
            self.measurements.viewport_changed();
            self.measurements.viewport = self.viewport.size;
        }
        if let Some(height) = self.measurements.rows.get(&ix) {
            return *height;
        }
        let key = self.rows[ix].key;
        let retention = retention(
            &self.rows[ix],
            &self.messages,
            &self.motions,
            &self.windowed_phases,
        );
        let stable = retention.is_some();
        let epoch = self.measurements.layout_epoch;
        let after_receipt = after_receipt(&self.rows, &self.messages, ix);
        if stable
            && let Some(retained) = self.measurements.retained.get_mut(&key)
            && same_row(&retained.row, &self.rows[ix])
            && retained.after_receipt == after_receipt
            && retained.epoch.is_none_or(|at| at == epoch)
        {
            retained.frame = self.measurements.frame;
            let height = retained.height;
            self.measurements.rows.insert(ix, height);
            if verify_heights() {
                self.verify_height(ix, height, window, cx);
            }
            return height;
        }
        let started = Instant::now();
        let kind = match (&self.rows[ix].body, retention) {
            (_, None) if self.motions.contains_key(&key) => "measure:moving",
            (Body::Markdown(_), _) => "measure:markdown",
            (Body::Tool { .. }, _) => "measure:tool",
            (Body::Phase { .. }, _) => "measure:phase",
            (Body::Plain(_), _) => "measure:plain",
            (Body::Actions { .. }, _) => "measure:actions",
            (Body::WorkLine { .. }, _) => "measure:workline",
            (Body::Reasoning, _) => "measure:reasoning",
            _ => "measure:other",
        };
        let mut row = self.row(ix, window, cx);
        let height = row
            .layout_as_root(
                size(
                    AvailableSpace::Definite(self.viewport.size.width),
                    AvailableSpace::MinContent,
                ),
                window,
                cx,
            )
            .height;
        gpui::frame_stats::add(kind, started.elapsed().as_micros() as u32);
        #[cfg(test)]
        {
            self.measurements.fresh += 1;
        }
        self.measurements.rows.insert(ix, height);
        if stable {
            if self.measurements.retained.len() >= 256
                && !self.measurements.retained.contains_key(&key)
            {
                // This is a viewport/follow working set, never a history cache.
                self.measurements.retained.clear();
            }
            self.measurements.retained.insert(
                key,
                RetainedHeight {
                    height,
                    row: self.rows[ix].clone(),
                    after_receipt,
                    frame: self.measurements.frame,
                    epoch: (retention == Some(Retention::Epoch)).then_some(epoch),
                },
            );
        }
        height
    }
}

/// How long a row's measured height can be reused.
#[derive(Clone, Copy, PartialEq)]
enum Retention {
    /// While its content is unchanged.
    Content,
    /// While its content is unchanged, within one layout epoch.
    Epoch,
}

/// How a row's measured height can be reused, if at all.
fn retention(
    row: &UiRow,
    messages: &[Message],
    motions: &HashMap<(usize, usize), Motion>,
    windowed: &HashSet<usize>,
) -> Option<Retention> {
    // Attachments and moving rows resize without a source patch. Drafts and
    // queued messages have no message.
    if motions.contains_key(&row.key)
        || messages
            .get(row.message)
            .is_none_or(|message| !message.attachments.is_empty())
    {
        return None;
    }
    // A live phase's window draws its rows, so their heights follow the window.
    if row
        .phase_group()
        .is_some_and(|group| windowed.contains(&group))
    {
        return None;
    }
    match &row.body {
        Body::Markdown(_) if !row.in_phase() => Some(Retention::Content),
        // Disclosures, panels and child views resize these without a source
        // patch, and every such change notifies the transcript.
        Body::Markdown(_) | Body::Tool { .. } | Body::Reasoning => Some(Retention::Epoch),
        Body::Phase { group, .. } if !windowed.contains(group) => Some(Retention::Epoch),
        _ => None,
    }
}

/// `same_layout`, extended to the rows `retention` keeps within an epoch.
fn same_row(a: &UiRow, b: &UiRow) -> bool {
    if same_layout(a, b) {
        return true;
    }
    a.key == b.key
        && a.first == b.first
        && a.last == b.last
        && a.work == b.work
        && a.work_tail == b.work_tail
        && a.thought_phase == b.thought_phase
        && match (&a.body, &b.body) {
            (
                Body::Tool {
                    call,
                    group,
                    tail,
                    live,
                },
                Body::Tool {
                    call: other_call,
                    group: other_group,
                    tail: other_tail,
                    live: other_live,
                },
            ) => {
                call == other_call
                    && group == other_group
                    && tail == other_tail
                    && live == other_live
            }
            (Body::Reasoning, Body::Reasoning) => true,
            (
                Body::Phase {
                    title,
                    group,
                    icon,
                    active,
                },
                Body::Phase {
                    title: other_title,
                    group: other_group,
                    icon: other_icon,
                    active: other_active,
                },
            ) => {
                title == other_title
                    && group == other_group
                    && icon == other_icon
                    && active == other_active
            }
            _ => false,
        }
}

/// `MONOCODE_VERIFY_ROW_HEIGHTS=1` re-measures every reused row height and
/// reports any that changed, to check that nothing resizes rows unnoticed.
fn verify_heights() -> bool {
    static VERIFY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *VERIFY.get_or_init(|| std::env::var_os("MONOCODE_VERIFY_ROW_HEIGHTS").is_some())
}

impl TranscriptView {
    fn verify_height(
        &mut self,
        ix: usize,
        retained: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut row = self.row(ix, window, cx);
        let fresh = row
            .layout_as_root(
                size(
                    AvailableSpace::Definite(self.viewport.size.width),
                    AvailableSpace::MinContent,
                ),
                window,
                cx,
            )
            .height;
        if (fresh - retained).abs() > px(0.5) {
            eprintln!(
                "monocode: row {:?} reused height {retained:?} but measures {fresh:?}",
                self.rows[ix].key
            );
        }
    }
}

pub(super) fn after_receipt(rows: &[UiRow], messages: &[Message], ix: usize) -> bool {
    messages
        .get(rows[ix].message)
        .is_some_and(|message| message.under_line)
        || (ix > 0 && matches!(rows[ix - 1].body, Body::WorkLine { .. }))
}

pub(super) fn same_layout(a: &UiRow, b: &UiRow) -> bool {
    a.key == b.key
        && a.first == b.first
        && a.last == b.last
        && a.work == b.work
        && a.work_tail == b.work_tail
        && a.thought_phase == b.thought_phase
        && match (&a.body, &b.body) {
            (Body::Markdown(a), Body::Markdown(b)) => Arc::ptr_eq(a, b) || a == b,
            _ => false,
        }
}

#[cfg(test)]
mod tests;
