use serde::Deserialize;

use crate::{FocusError, Result};

#[derive(Deserialize)]
struct Output {
    name: String,
}

pub(crate) fn focused_output() -> Result<String> {
    let output = crate::command_output("niri", &["msg", "--json", "focused-output"])?;
    let output: Output =
        serde_json::from_str(&output).map_err(|source| FocusError::InvalidResponse {
            compositor: "Niri",
            source,
        })?;

    if output.name.is_empty() {
        return Err(FocusError::NoFocusedOutput { compositor: "Niri" });
    }

    Ok(output.name)
}
