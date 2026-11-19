use super::*;

impl QuickComposer {
    // useQuickAttachments: files, pasted images and screenshots.
    pub(super) fn add_attachments(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if !self.attachments_supported() {
            self.error = Some("This provider does not support attachments".into());
            cx.notify();
            return;
        }
        let mut all = self.attachments.clone();
        for path in paths {
            if !all.contains(&path) {
                all.push(path);
            }
        }
        match monocode_core::session::composer::validate_attachments(&all) {
            Ok(()) => {
                self.attachments = all;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    pub(super) fn paste(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(clipboard) = cx.read_from_clipboard() else {
            return false;
        };
        let mut paths = Vec::new();
        let mut images = Vec::new();
        for entry in clipboard.entries() {
            match entry {
                gpui::ClipboardEntry::ExternalPaths(p) => paths.extend(p.paths().to_vec()),
                gpui::ClipboardEntry::Image(image) => images.push(image.clone()),
                _ => {}
            }
        }
        if paths.is_empty() && images.is_empty() {
            return false;
        }
        if !self.attachments_supported() {
            return false;
        }
        for image in images {
            let dir = paste_staging_dir();
            let path = dir.join(format!(
                "pasted-{}.{}",
                monocode_core::mono::new_id(),
                image.format().extension()
            ));
            match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, image.bytes())) {
                Ok(()) => {
                    self.staged_attachments.push(path.clone());
                    paths.push(path);
                }
                Err(error) => self.error = Some(format!("Could not paste the image: {error}")),
            }
        }
        self.add_attachments(paths, cx);
        self.staged_attachments
            .retain(|path| self.attachments.contains(path));
        true
    }

    pub(super) fn choose_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_picker(window, cx);
        let prompt = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = prompt.await {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.add_attachments(paths, cx);
                    this.focus(window, cx);
                });
            }
        })
        .detach();
    }

    /// User-initiated region or window capture. The panel steps aside while
    /// the system overlay is up; cancelling leaves the draft intact.
    pub(super) fn take_screenshot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_picker_state(cx);
        self.attaching = true;
        #[cfg(all(target_os = "macos", not(test)))]
        crate::ui::traffic_lights::hide(window);
        let path = std::env::temp_dir()
            .join("monocode-quick-composer")
            .join(format!("screenshot-{}.png", monocode_core::mono::new_id()));
        let work = cx.background_executor().spawn({
            let path = path.clone();
            async move {
                std::fs::create_dir_all(path.parent().unwrap_or(&path))
                    .map_err(|e| e.to_string())?;
                // Let WindowServer remove the panel before the overlay appears.
                std::thread::sleep(std::time::Duration::from_millis(150));
                let output = std::process::Command::new("/usr/sbin/screencapture")
                    .args(["-i", "-x", "-t", "png"])
                    .arg(&path)
                    .output()
                    .map_err(|e| format!("Could not take a screenshot: {e}"))?;
                if path.is_file() {
                    return Ok(Some(path));
                }
                let error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                if !output.status.success() && !error.is_empty() {
                    return Err(format!("Could not take a screenshot: {error}"));
                }
                Ok(None)
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.attaching = false;
                match result {
                    Ok(Some(path)) => {
                        this.staged_attachments.push(path.clone());
                        this.add_attachments(vec![path], cx);
                        this.staged_attachments
                            .retain(|path| this.attachments.contains(path));
                    }
                    Ok(None) => {}
                    Err(error) => this.error = Some(error),
                }
                this.focus(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
