use std::io;

use serde::Deserialize;

#[derive(Deserialize)]
struct Workspace {
    monitor: String,
}

pub(crate) fn focused_output() -> io::Result<String> {
    let output = crate::command_output("hyprctl", &["activeworkspace", "-j"])?;
    let workspace: Workspace = serde_json::from_str(&output).map_err(io::Error::other)?;

    if workspace.monitor.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "hyprland active workspace has no monitor",
        ));
    }

    Ok(workspace.monitor)
}
