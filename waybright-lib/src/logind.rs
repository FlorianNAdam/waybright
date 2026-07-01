use std::io;
use std::time::Duration;

pub(crate) fn set_brightness(subsystem: &str, device: &str, value: u32) -> io::Result<()> {
    let connection = dbus::blocking::Connection::new_system().map_err(io::Error::other)?;
    let proxy = connection.with_proxy(
        "org.freedesktop.login1",
        "/org/freedesktop/login1/session/auto",
        Duration::from_secs(2),
    );

    proxy
        .method_call(
            "org.freedesktop.login1.Session",
            "SetBrightness",
            (subsystem, device, value),
        )
        .map_err(io::Error::other)
}
