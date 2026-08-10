use std::io;

use serde::Deserialize;

#[derive(Deserialize)]
struct Workspace {
    focused: bool,
    output: String,
}

pub(crate) fn focused_output() -> io::Result<String> {
    let output = crate::command_output("swaymsg", &["-t", "get_workspaces"])?;
    let workspaces: Vec<Workspace> = serde_json::from_str(&output).map_err(io::Error::other)?;

    workspaces
        .into_iter()
        .find(|workspace| workspace.focused)
        .map(|workspace| workspace.output)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "sway has no focused workspace"))
}
