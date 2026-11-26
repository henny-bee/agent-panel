//! Claude Desktop's step pictures: the images a group of steps read, made or
//! got back sit as small thumbnails at the end of its line. Pointing at one
//! shows it larger with what the step did and "1 of 2"; clicking opens it full
//! screen.
use super::*;
use gpui::AnyElement;
use std::path::PathBuf;

/// Thumbnails a strip shows before "+N".
const SHOWN: usize = 4;

/// An image a step showed, and what the step did with it ("Read").
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StepImage {
    pub path: PathBuf,
    pub caption: SharedString,
}

/// The images a call returned, then the image file it read or wrote when it
/// is one on disk. `base` resolves a relative path.
pub(crate) fn step_images(call: &ToolCall, base: Option<&str>) -> Vec<StepImage> {
    let shown = call.presentation_in(base);
    let caption: SharedString = match shown.action.as_str() {
        "Write" => "Created".into(),
        "Edit" => "Edited".into(),
        "Read" => "Read".into(),
        "Run" => "Ran".into(),
        _ => call.name.clone().into(),
    };
    let mut images: Vec<StepImage> = call
        .images
        .iter()
        .map(|path| StepImage {
            path: PathBuf::from(path),
            caption: caption.clone(),
        })
        .collect();
    if images.is_empty()
        && matches!(shown.action.as_str(), "Read" | "Write" | "Edit")
        && call.state == ToolState::Completed
        && let Some(path) = shown.file_path.as_deref()
    {
        let (path, _) = crate::ui::files::file_location(path, base.unwrap_or(""));
        let path = PathBuf::from(path);
        if monocode_core::session::composer::image_mime(&path).is_some() && path.is_file() {
            images.push(StepImage { path, caption });
        }
    }
    images
}

impl TranscriptView {
    /// Thumbnails for `images`, keyed by `key` so each strip keeps its own
    /// decoded sprites and hover.
    pub(super) fn image_strip(
        &mut self,
        key: usize,
        images: &[StepImage],
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if images.is_empty() {
            return None;
        }
        let total = images.len();
        let files = attachments::Attachment::from_paths(
            &images
                .iter()
                .map(|image| image.path.clone())
                .collect::<Vec<_>>(),
        );
        let mut strip = div()
            .debug_selector(move || format!("step-images-{key}"))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1();
        let mut preview = None;
        for (ix, file) in files.iter().enumerate().take(SHOWN) {
            let anchor = self.image_anchors.entry((key, ix)).or_default().clone();
            strip = strip.child(
                self.image_tile(file, &format!("step-{key}"), ix, false, cx)
                    .w(px(30.))
                    .h(px(22.))
                    .rounded(px(4.))
                    .overflow_hidden()
                    .child(anchor.probe())
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered {
                            this.hovered_image = Some((key, ix));
                        } else if this.hovered_image == Some((key, ix)) {
                            this.hovered_image = None;
                        }
                        cx.notify();
                    })),
            );
            if self.hovered_image == Some((key, ix)) {
                preview = Some((anchor, ix));
            }
        }
        if total > SHOWN {
            strip = strip.child(
                div()
                    .text_size(px(11.))
                    .text_color(content(0.5))
                    .child(format!("+{}", total - SHOWN)),
            );
        }
        if let Some((anchor, ix)) = preview {
            let image = &images[ix];
            strip = strip.child(
                popover(("step-image-preview", key * 64 + ix), Anchor::Slot(anchor))
                    .side(Side::Top)
                    .max_width(340.)
                    .padding(6.)
                    .child(
                        div()
                            .debug_selector(|| "step-image-preview".into())
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                gpui::img(image.path.clone())
                                    .max_w(px(328.))
                                    .max_h(px(240.))
                                    .rounded(px(6.))
                                    .object_fit(gpui::ObjectFit::Contain),
                            )
                            .child(
                                div()
                                    .px_1()
                                    .flex()
                                    .justify_between()
                                    .text_size(px(12.))
                                    .text_color(content(0.6))
                                    .child(image.caption.clone())
                                    .when(total > 1, |d| d.child(format!("{} of {total}", ix + 1))),
                            ),
                    ),
            );
        }
        Some(strip.into_any_element())
    }
}

impl TranscriptView {
    /// A picture a reply links from disk, in a fixed box so the row keeps its
    /// height while it loads; clicking opens it full screen.
    pub(super) fn reply_image(
        &mut self,
        path: &str,
        caption: &str,
        _: &mut Context<Self>,
    ) -> AnyElement {
        let (path, _) =
            crate::ui::files::file_location(path, self.file_link_base.as_deref().unwrap_or(""));
        let path = PathBuf::from(path);
        if !path.is_file() {
            return div()
                .text_size(px(13.))
                .text_color(content(0.5))
                .child(format!("Image not found: {}", path.display()))
                .into_any_element();
        }
        let alt: SharedString = if caption.is_empty() {
            path.file_name().map_or("Image".into(), |name| {
                name.to_string_lossy().into_owned().into()
            })
        } else {
            caption.to_owned().into()
        };
        let source = crate::ui::image_lightbox::Source::Path(path.clone());
        let open_alt = alt.clone();
        div()
            .id(SharedString::from(format!(
                "reply-image:{}",
                path.display()
            )))
            .debug_selector(|| "reply-image".into())
            .py_2()
            .w_full()
            .flex()
            .flex_col()
            .gap_1()
            .role(Role::Button)
            .aria_label(format!("Open {alt} full screen"))
            .cursor(gpui::CursorStyle::PointingHand)
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                window.dispatch_action(
                    Box::new(crate::ui::image_lightbox::Open {
                        source: source.clone(),
                        alt: open_alt.clone(),
                    }),
                    cx,
                );
            })
            .child(
                div()
                    .w_full()
                    .max_w(px(480.))
                    .h(px(280.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(content(0.08))
                    .overflow_hidden()
                    .child(
                        gpui::img(path)
                            .size_full()
                            .object_fit(gpui::ObjectFit::Contain),
                    ),
            )
            .when(!caption.is_empty(), |d| {
                d.child(div().text_size(px(12.)).text_color(content(0.5)).child(alt))
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_step_shows_what_it_returned_or_the_image_file_it_touched() {
        let dir = std::env::temp_dir().join(format!("mcode-step-images-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let picture = dir.join("sheet.png");
        std::fs::write(&picture, b"png").unwrap();
        let mut read = ToolCall::new("r", "Read");
        read.input = serde_json::json!({"file_path": picture}).to_string();
        read.state = ToolState::Completed;
        let images = step_images(&read, None);
        assert_eq!(
            images,
            [StepImage {
                path: picture.clone(),
                caption: "Read".into()
            }]
        );
        // Text files and missing images show nothing.
        read.input = serde_json::json!({"file_path": dir.join("a.rs")}).to_string();
        assert!(step_images(&read, None).is_empty());
        // Returned images win over the file the call named.
        let mut shot = ToolCall::new("s", "mcp__browser__screenshot");
        shot.images = vec!["/cache/s-0.png".into()];
        shot.state = ToolState::Completed;
        assert_eq!(
            step_images(&shot, None)[0].path,
            PathBuf::from("/cache/s-0.png")
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
