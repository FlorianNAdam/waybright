use serde::Deserialize;

use crate::{FocusError, Result};

#[derive(Deserialize)]
struct Workspace {
    monitor: String,
}

pub(crate) fn focused_output() -> Result<String> {
    let output = crate::command_output("hyprctl", &["activeworkspace", "-j"])?;
    let workspace: Workspace =
        serde_json::from_str(&output).map_err(|source| FocusError::InvalidResponse {
            compositor: "Hyprland",
            source,
        })?;

    if workspace.monitor.is_empty() {
        return Err(FocusError::NoFocusedOutput {
            compositor: "Hyprland",
        });
    }

    Ok(workspace.monitor)
}
