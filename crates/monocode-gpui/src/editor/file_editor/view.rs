use super::*;

impl FileEditor {
    pub(super) fn update_preview(&mut self, cx: &mut Context<Self>) {
        if self.preview && self.is_svg() {
            let bytes = self.input.read(cx).content.as_bytes().to_vec();
            let renderer = cx.svg_renderer();
            let work = cx.background_executor().spawn(async move {
                Image::from_bytes(ImageFormat::Svg, bytes)
                    .to_image_data(renderer)
                    .ok()
            });
            self.svg_render = Some(cx.spawn(async move |this, cx| {
                let image = work.await;
                let _ = this.update(cx, |this, cx| {
                    this.svg_failed = image.is_none();
                    if let Some(old) = std::mem::replace(&mut this.svg_preview, image) {
                        cx.drop_image(old, None);
                    }
                    cx.notify();
                });
            }));
        }
        if self.preview && self.is_markdown() {
            let text = self.input.read(cx).content.clone();
            if self.markdown.is_none() || self.markdown_source != text {
                self.markdown_source = text.clone();
                let base = PathBuf::from(&self.path)
                    .parent()
                    .map(|p| p.to_string_lossy().into_owned());
                self.markdown = Some(cx.new(|cx| {
                    let mut view = TranscriptView::document(text.to_string(), cx);
                    view.file_link_base = base;
                    view
                }));
            }
        }
        // The page shows the file on disk, so each new version, and each
        // return from Source, reloads it.
        if self.preview && self.previews_html() {
            match &self.html {
                Some(html) => html.update(cx, |html, cx| html.reload(cx)),
                None => {
                    let path = PathBuf::from(&self.path);
                    let root = PathBuf::from(&self.cwd);
                    self.html = Some(cx.new(|_| crate::ui::html::Html::file(path, &root)));
                }
            }
        }
    }

    pub(super) fn reload_page(&mut self, cx: &mut Context<Self>) {
        if let Some(html) = &self.html {
            html.update(cx, |html, cx| html.reload(cx));
        }
    }

    pub(super) fn open_in_browser(&mut self, cx: &mut Context<Self>) {
        let path = PathBuf::from(&self.path);
        let work = cx
            .background_executor()
            .spawn(async move { crate::ui::html::open_in_browser(&path) });
        cx.spawn(async move |this, cx| {
            if let Err(error) = work.await {
                let _ = this.update(cx, |this, cx| {
                    this.error = Some(format!("Couldn’t open the page in a browser: {error}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn toggle_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.selection.dismiss();
        self.preview = !self.preview;
        // A page always opens rendered, so it leaves the Markdown and SVG
        // choice alone.
        if !self.is_html() && cx.has_global::<crate::ui::preferences::Preferences>() {
            cx.update_global::<crate::ui::preferences::Preferences, _>(|prefs, _| {
                prefs.file_view_mode = if self.preview { "preview" } else { "source" }.into();
            });
        }
        self.update_preview(cx);
        window.focus(&self.focus_handle(cx), cx);
        cx.notify();
    }
}

impl gpui::Render for FileEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ready = self.document.is_some();
        let controls = ready.then(|| {
            div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap_1p5()
                .when(!self.preview, |d| d.child(self.word_wrap_control(cx)))
                .when(self.is_local_html(), |d| d.child(self.page_controls(cx)))
                .when(
                    self.is_markdown() || self.is_svg() || self.previews_html(),
                    |d| d.child(self.view_mode_control(cx)),
                )
                .into_any_element()
        });
        let selection_menu = self.code_selection_menu(window, cx);
        self.input.update(cx, |i, _| {
            i.set_editor_diff(self.show_diff, !self.git_staged)
        });
        if ((!ready || self.preview) && self.input.focus_handle(cx).is_focused(window))
            || (!self.search_open
                && (self.search.focus_handle(cx).is_focused(window)
                    || self.replacement.focus_handle(cx).is_focused(window)))
        {
            window.focus(&self.focus, cx);
        }
        if ready && !self.preview && self.focus.is_focused(window) {
            window.focus(&self.input.focus_handle(cx), cx);
        }
        let preview_image: Option<ImageSource> = match &self.image {
            Some(image) => Some(image.clone().into()),
            None if self.preview && self.svg_failed => {
                Some(ImageSource::Custom(Arc::new(|_, _| {
                    Some(Err(ImageCacheError::Asset("Invalid SVG".into())))
                })))
            }
            None if self.preview => self.svg_preview.clone().map(ImageSource::Render),
            None => None,
        };
        let body: AnyElement = if let Some(media) = &self.media {
            media.clone().into_any_element()
        } else if let Some(image) = preview_image {
            div()
                .size_full()
                .p_4()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    img(image)
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .with_fallback(|| {
                            div()
                                .text_size(px(12.))
                                .text_color(content(0.5))
                                .child("Couldn’t decode this image")
                                .into_any_element()
                        }),
                )
                .into_any_element()
        } else if ready {
            if self.preview {
                self.html
                    .clone()
                    .map(|h| h.into_any_element())
                    .or_else(|| self.markdown.clone().map(|m| m.into_any_element()))
                    .unwrap_or_else(|| div().into_any_element())
            } else {
                self.input.clone().into_any_element()
            }
        } else {
            div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .p_6()
                .text_size(px(12.))
                .text_color(content(0.5))
                .child(
                    self.unsupported
                        .clone()
                        .or(self.error.clone())
                        .unwrap_or_else(|| "Opening file…".into()),
                )
                .when(!self.loading, |d| {
                    d.child(
                        div()
                            .id("open-file-externally")
                            .role(Role::Button)
                            .aria_label("Open externally")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .hover(|s| s.bg(content(0.1)))
                            .child("Open externally")
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.open_url(&format!(
                                    "file://{}",
                                    this.path
                                        .replace('%', "%25")
                                        .replace(' ', "%20")
                                        .replace('#', "%23")
                                        .replace('?', "%3F")
                                ))
                            })),
                    )
                })
                .when(!self.loading, |d| {
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .id("retry-file")
                                    .role(Role::Button)
                                    .aria_label("Retry")
                                    .px_3()
                                    .py_1()
                                    .rounded_md()
                                    .bg(content(0.1))
                                    .child("Retry")
                                    .on_click(cx.listener(|this, _, _, cx| this.load(true, cx))),
                            )
                            .child(
                                div()
                                    .id("reveal-file")
                                    .role(Role::Button)
                                    .aria_label("Reveal")
                                    .px_3()
                                    .py_1()
                                    .rounded_md()
                                    .bg(content(0.1))
                                    .child("Reveal")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let path = PathBuf::from(&this.path);
                                        cx.background_executor()
                                            .spawn(async move {
                                                let _ = monocode_core::files::reveal_path(&path);
                                            })
                                            .detach();
                                    })),
                            )
                            .child(
                                div()
                                    .id("copy-file-path")
                                    .role(Role::Button)
                                    .aria_label("Copy path")
                                    .px_3()
                                    .py_1()
                                    .rounded_md()
                                    .bg(content(0.1))
                                    .child("Copy path")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                            this.path.clone(),
                                        ))
                                    })),
                            ),
                    )
                })
                .into_any_element()
        };
        div()
            .id("file-editor")
            .relative()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .key_context("FileEditor")
            .track_focus(&self.focus)
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, _, window, cx| this.finish_code_selection(window, cx)),
            )
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.selection.dismiss();
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, w, cx| {
                if this.search_open
                    && event.keystroke.key == "enter"
                    && event.keystroke.modifiers.shift
                    && this.search.focus_handle(cx).is_focused(w)
                {
                    this.step_match(true, cx);
                    cx.stop_propagation();
                    return;
                }
                if let Some(media) = &this.media {
                    media.update(cx, |m, cx| m.zoom_key(event, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &Save, _, cx| this.save(cx)))
            .on_action(cx.listener(|this, _: &ToggleWordWrap, _, cx| this.toggle_word_wrap(cx)))
            .on_action(cx.listener(|this, _: &ReloadFile, w, cx| this.reload(w, cx)))
            .on_action(cx.listener(|this, _: &Find, w, cx| this.open_search(w, cx)))
            .on_action(cx.listener(|this, _: &Replace, w, cx| {
                this.open_search(w, cx);
                this.replace_open = true;
            }))
            .on_action(cx.listener(|this, _: &Format, _, cx| this.format_document(cx)))
            .on_action(cx.listener(|this, _: &MatchCase, _, cx| {
                if this.search_open {
                    this.match_case = !this.match_case;
                    this.find_matches(cx);
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &WholeWord, _, cx| {
                if this.search_open {
                    this.whole_word = !this.whole_word;
                    this.find_matches(cx);
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &Regex, _, cx| {
                if this.search_open {
                    this.regex = !this.regex;
                    this.find_matches(cx);
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &PreviousChange, _, cx| this.step_change(true, cx)))
            .on_action(cx.listener(|this, _: &NextChange, _, cx| this.step_change(false, cx)))
            .on_action(cx.listener(|this, _: &RevertChange, _, cx| this.revert_change(cx)))
            .on_action(cx.listener(|this, _: &FindNext, _, cx| this.step_match(false, cx)))
            .on_action(cx.listener(|this, _: &FindPrevious, _, cx| this.step_match(true, cx)))
            .on_action(cx.listener(|this, _: &DismissSearch, w, cx| this.dismiss_search(w, cx)))
            .child(self.path_header(controls, cx))
            .when(self.show_diff && ready && !self.preview, |d| {
                d.child(self.diff_toolbar(cx))
            })
            .when(self.search_open, |d| d.child(self.search_panel(cx)))
            .when_some(
                self.error.clone().filter(|_| ready).or_else(|| {
                    self.disk_changed
                        .then(|| "File changed on disk. Reload to use the new version.".into())
                }),
                |d, error| {
                    d.child(
                        div()
                            .px_3()
                            .py_2()
                            .text_size(px(12.))
                            .text_color(rgb(0xfbbf24))
                            .child(error),
                    )
                },
            )
            .child(div().flex_1().min_h_0().min_w_0().child(body))
            .when(ready && !self.preview, |d| d.child(self.status_footer(cx)))
            .child(self.selection_surface(cx))
            .children(selection_menu)
    }
}
