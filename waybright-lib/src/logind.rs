use std::io;

pub(crate) fn set_brightness(subsystem: &str, device: &str, value: u32) -> io::Result<()> {
    let connection = zbus::blocking::Connection::system().map_err(io::Error::other)?;
    let proxy = zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1/session/auto",
        "org.freedesktop.login1.Session",
    )
    .map_err(io::Error::other)?;

    proxy
        .call::<_, _, ()>("SetBrightness", &(subsystem, device, value))
        .map_err(io::Error::other)
}
