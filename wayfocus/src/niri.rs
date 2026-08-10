use std::io;

use serde::Deserialize;

#[derive(Deserialize)]
struct Output {
    name: String,
}

pub(crate) fn focused_output() -> io::Result<String> {
    let output = crate::command_output("niri", &["msg", "--json", "focused-output"])?;
    let output: Output = serde_json::from_str(&output).map_err(io::Error::other)?;

    if output.name.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "niri focused output has no name",
        ));
    }

    Ok(output.name)
}
