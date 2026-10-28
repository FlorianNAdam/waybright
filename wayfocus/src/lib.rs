use std::{env, io, process::Command, string::FromUtf8Error};

use thiserror::Error;

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

#[derive(Debug, Error)]
pub enum FocusError {
    #[error("failed to run {command}")]
    Command {
        command: String,
        #[source]
        source: io::Error,
    },
    #[error("{command} exited with status {status}: {stderr}")]
    CommandFailed {
        command: String,
        status: std::process::ExitStatus,
        stderr: String,
    },
    #[error("{command} returned invalid UTF-8")]
    InvalidUtf8 {
        command: String,
        #[source]
        source: FromUtf8Error,
    },
    #[error("invalid response from {compositor}")]
    InvalidResponse {
        compositor: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("{compositor} reported no focused output")]
    NoFocusedOutput { compositor: &'static str },
    #[error("failed to query KWin")]
    Kwin(#[source] dbus::Error),
    #[error("focused output is only supported on sway, hyprland, niri, and kwin")]
    Unsupported,
}

pub type Result<T> = std::result::Result<T, FocusError>;

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

pub fn focused_output() -> Result<String> {
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
        .map_err(|_| FocusError::Unsupported)
}

fn env_contains(name: &str, value: &str) -> bool {
    env::var(name)
        .map(|current| current.to_lowercase().contains(value))
        .unwrap_or(false)
}

pub(crate) fn command_output(command: &str, args: &[&str]) -> Result<String> {
    let invocation = std::iter::once(command)
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join(" ");
    let output = Command::new(command)
        .args(args)
        .output()
        .map_err(|source| FocusError::Command {
            command: invocation.clone(),
            source,
        })?;

    if !output.status.success() {
        return Err(FocusError::CommandFailed {
            command: invocation,
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    String::from_utf8(output.stdout).map_err(|source| FocusError::InvalidUtf8 {
        command: invocation,
        source,
    })
}
