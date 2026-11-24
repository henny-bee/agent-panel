//! Provider-neutral tool presentation, persisted as a native `tool` row.
use serde::{Deserialize, Serialize};
mod presentation;
pub mod tasks;
pub use presentation::{
    app_activity, app_cli_action, app_tool, visual_tool, CommandStep, ToolPresentation, VisualStep,
};

/// Saves an image a tool returned, given as base64 `data` of `media_type`,
/// and returns where it went. The name follows the call, so a replayed
/// result reuses its file.
// shortcut: images live in the user cache folder and nothing prunes them;
// add a size cap there if they ever pile up.
pub fn save_image(tool_id: &str, index: usize, media_type: &str, data: &str) -> Option<String> {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".cache"))
        })?;
    save_image_in(
        &cache.join("mcode").join("tool-images"),
        tool_id,
        index,
        media_type,
        data,
    )
}
fn save_image_in(
    dir: &std::path::Path,
    tool_id: &str,
    index: usize,
    media_type: &str,
    data: &str,
) -> Option<String> {
    let extension = match media_type {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/png" => "png",
        _ => return None,
    };
    let name: String = tool_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(80)
        .collect();
    let path = dir.join(format!("{name}-{index}.{extension}"));
    if !path.is_file() {
        let bytes = crate::session::composer::decode_image_data(data).ok()?;
        std::fs::create_dir_all(dir).ok()?;
        std::fs::write(&path, bytes).ok()?;
    }
    Some(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod image_tests {
    #[test]
    fn a_returned_image_is_saved_once_under_its_call() {
        let dir = tempfile::tempdir().unwrap();
        // "hi" in base64; the bytes need not be a real picture here.
        let path = super::save_image_in(dir.path(), "toolu_../01", 0, "image/png", "aGk=").unwrap();
        assert!(path.ends_with("toolu_01-0.png"));
        assert_eq!(std::fs::read(&path).unwrap(), b"hi");
        assert!(super::save_image_in(dir.path(), "t", 0, "text/plain", "aGk=").is_none());
        assert!(super::save_image_in(dir.path(), "t", 1, "image/png", "!!").is_none());
    }
}

/// The app's own step while a conversation carries over from another model.
pub const HANDOFF: &str = "providerHandoff";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolState {
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: String,
    pub output: String,
    pub state: ToolState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel: Option<tasks::Panel>,
    /// Descriptive provider/desktop metadata, separate from actual arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<serde_json::Value>,
    /// Images the tool returned (a screenshot, an image it read), saved as
    /// files so the transcript can show them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
}
impl ToolCall {
    pub fn new(id: &str, name: &str) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            input: String::new(),
            output: String::new(),
            state: ToolState::Running,
            panel: None,
            display: None,
            images: Vec::new(),
        }
    }
    pub fn is_compaction(&self) -> bool {
        self.name == "contextCompaction"
    }
    pub fn is_handoff(&self) -> bool {
        self.name == HANDOFF
    }
    /// Session milestones read as a status line rather than a tool step.
    pub fn is_milestone(&self) -> bool {
        self.is_compaction() || self.is_handoff()
    }
    pub fn append_output(&mut self, delta: &str) {
        self.output.push_str(delta);
        self.bound_output();
    }
    pub fn bound_output(&mut self) {
        // Keep a useful tail without retaining unbounded command output in the UI.
        const LIMIT: usize = 64 * 1024;
        if self.output.len() > LIMIT {
            let mut at = self.output.len() - LIMIT;
            while !self.output.is_char_boundary(at) {
                at += 1;
            }
            self.output = format!("… earlier output omitted …\n{}", &self.output[at..]);
        }
    }
    /// One of Monocode's own visual tools, under any provider's naming.
    pub fn visual_step(&self) -> Option<VisualStep> {
        visual_tool(&self.name)
    }
    pub fn summary(&self) -> (String, String) {
        match self.panel() {
            Some(tasks::Panel::Tasks(list)) => return ("Tasks".into(), list.progress()),
            Some(tasks::Panel::Agent(run)) => return ("Agent".into(), run.title().into()),
            None => {}
        }
        let shown = self.presentation();
        (shown.action, shown.target)
    }
}

impl ToolCall {
    /// Provider metadata affects display only. The persisted arguments stay intact.
    pub fn presentation(&self) -> ToolPresentation {
        presentation::present(self, None)
    }
    pub fn presentation_in(&self, cwd: Option<&str>) -> ToolPresentation {
        presentation::present(self, cwd)
    }
    /// Show the provider's actual input rather than the desktop adapter envelope.
    pub fn input_detail(&self) -> String {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&self.input) else {
            return self.input.clone();
        };
        let raw = if value["__desktop"].is_object() {
            value
                .get("rawInput")
                .filter(|raw| !raw.is_null())
                .unwrap_or(&value["__desktop"])
        } else {
            &value
        };
        raw.as_str().map(str::to_owned).unwrap_or_else(|| {
            serde_json::to_string_pretty(raw).unwrap_or_else(|_| self.input.clone())
        })
    }
}

/// Strip a recognized shell launcher for display without executing its script.
pub fn shell_command(command: &str) -> String {
    presentation::shell_command(command)
}
