use serde::Deserialize;

use crate::{FocusError, Result};

#[derive(Deserialize)]
struct Workspace {
    focused: bool,
    output: String,
}

pub(crate) fn focused_output() -> Result<String> {
    let output = crate::command_output("swaymsg", &["-t", "get_workspaces"])?;
    let workspaces: Vec<Workspace> =
        serde_json::from_str(&output).map_err(|source| FocusError::InvalidResponse {
            compositor: "Sway",
            source,
        })?;

    workspaces
        .into_iter()
        .find(|workspace| workspace.focused)
        .map(|workspace| workspace.output)
        .ok_or(FocusError::NoFocusedOutput { compositor: "Sway" })
}
