use std::time::Duration;

use crate::{FocusError, Result};

pub(crate) fn focused_output() -> Result<String> {
    let connection = dbus::blocking::Connection::new_session().map_err(FocusError::Kwin)?;
    let proxy = connection.with_proxy("org.kde.KWin", "/KWin", Duration::from_secs(2));
    let (output,): (String,) = proxy
        .method_call("org.kde.KWin", "activeOutputName", ())
        .map_err(FocusError::Kwin)?;
    let output = output.trim();

    if output.is_empty() {
        return Err(FocusError::NoFocusedOutput { compositor: "KWin" });
    }

    Ok(output.to_owned())
}
