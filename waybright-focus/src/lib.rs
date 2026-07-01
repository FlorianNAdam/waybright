use std::{env, io, process::Command};

mod hyprland;
mod kwin;
mod niri;
mod sway;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Compositor {
    Sway,
    Hyprland,
    Niri,
    Kwin,
    Unknown,
}

pub fn detect_compositor() -> Compositor {
    if env::var_os("SWAYSOCK").is_some() {
        return Compositor::Sway;
    }

    if env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        return Compositor::Hyprland;
    }

    if env::var_os("NIRI_SOCKET").is_some() {
        return Compositor::Niri;
    }

    if env_contains("XDG_CURRENT_DESKTOP", "kde")
        || env_contains("DESKTOP_SESSION", "plasma")
        || env::var_os("KDE_FULL_SESSION").is_some()
    {
        return Compositor::Kwin;
    }

    Compositor::Unknown
}

pub fn focused_output() -> io::Result<String> {
    match detect_compositor() {
        Compositor::Sway => return sway::focused_output(),
        Compositor::Hyprland => return hyprland::focused_output(),
        Compositor::Niri => return niri::focused_output(),
        Compositor::Kwin => return kwin::focused_output(),
        Compositor::Unknown => {}
    }

    sway::focused_output()
        .or_else(|_| hyprland::focused_output())
        .or_else(|_| niri::focused_output())
        .or_else(|_| kwin::focused_output())
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "focused output is only supported on sway, hyprland, niri, and kwin",
            )
        })
}

fn env_contains(name: &str, value: &str) -> bool {
    env::var(name)
        .map(|current| current.to_lowercase().contains(value))
        .unwrap_or(false)
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
