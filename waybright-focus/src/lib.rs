use std::{env, io, process::Command};

mod hyprland;
mod niri;
mod sway;

pub fn focused_output() -> io::Result<String> {
    if env::var_os("SWAYSOCK").is_some() {
        return sway::focused_output();
    }

    if env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        return hyprland::focused_output();
    }

    if env::var_os("NIRI_SOCKET").is_some() {
        return niri::focused_output();
    }

    sway::focused_output()
        .or_else(|_| hyprland::focused_output())
        .or_else(|_| niri::focused_output())
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "focused output is only supported on sway, hyprland, and niri",
            )
        })
}

pub(crate) fn command_output(command: &str, args: &[&str]) -> io::Result<String> {
    let output = Command::new(command).args(args).output()?;

    if !output.status.success() {
        return Err(io::Error::other(format!(
            "{command} exited with status {}",
            output.status
        )));
    }

    String::from_utf8(output.stdout).map_err(io::Error::other)
}
