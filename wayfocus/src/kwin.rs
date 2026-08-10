use std::io;
use std::time::Duration;

pub(crate) fn focused_output() -> io::Result<String> {
    let connection = dbus::blocking::Connection::new_session().map_err(io::Error::other)?;
    let proxy = connection.with_proxy("org.kde.KWin", "/KWin", Duration::from_secs(2));
    let (output,): (String,) = proxy
        .method_call("org.kde.KWin", "activeOutputName", ())
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
