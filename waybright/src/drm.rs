use std::{io, path::Path, path::PathBuf};

use crate::read_dir_optional;

#[derive(Debug)]
pub(crate) struct DrmConnector {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
}

pub(crate) fn connectors(path: &Path) -> io::Result<Vec<DrmConnector>> {
    let mut connectors = Vec::new();

    for entry in read_dir_optional(path)? {
        let name = entry.file_name().to_string_lossy().into_owned();

        if is_connector_name(&name) {
            connectors.push(DrmConnector {
                name,
                path: entry.path(),
            });
        }
    }

    Ok(connectors)
}

pub(crate) fn connector_output_name(name: &str) -> &str {
    name.strip_prefix("card")
        .and_then(|rest| rest.split_once('-'))
        .map(|(_, output)| output)
        .unwrap_or(name)
}

fn is_connector_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("card") else {
        return false;
    };

    let Some((card, connector)) = rest.split_once('-') else {
        return false;
    };

    !card.is_empty() && card.bytes().all(|byte| byte.is_ascii_digit()) && !connector.is_empty()
}
