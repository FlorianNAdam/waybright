use std::io;

pub(crate) fn focused_output() -> io::Result<String> {
    let connection = zbus::blocking::Connection::session().map_err(io::Error::other)?;
    let proxy = zbus::blocking::Proxy::new(&connection, "org.kde.KWin", "/KWin", "org.kde.KWin")
        .map_err(io::Error::other)?;
    let output: String = proxy
        .call("activeOutputName", &())
        .map_err(io::Error::other)?;
    let output = output.trim();

    if output.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "kwin active output has no name",
        ));
    }

    Ok(output.to_owned())
}
