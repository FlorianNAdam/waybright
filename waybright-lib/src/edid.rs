use std::{collections::HashMap, fs, io, path::Path};

pub(crate) fn unique_edids(
    items: impl Iterator<Item = (Vec<u8>, String)>,
) -> HashMap<Vec<u8>, String> {
    let mut counts = HashMap::<Vec<u8>, Option<String>>::new();

    for (edid, name) in items {
        counts
            .entry(edid)
            .and_modify(|name| *name = None)
            .or_insert(Some(name));
    }

    counts
        .into_iter()
        .filter_map(|(edid, name)| name.map(|name| (edid, name)))
        .collect()
}

pub(crate) fn read_base_edid(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let mut edid = read_non_empty(path)?;
    edid.truncate(128);

    if !is_valid_base_edid(&edid) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid edid"));
    }

    Ok(edid)
}

pub(crate) fn is_valid_base_edid(edid: &[u8]) -> bool {
    edid.len() == 128
        && edid.starts_with(&[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00])
        && edid.iter().fold(0_u8, |sum, byte| sum.wrapping_add(*byte)) == 0
}

fn read_non_empty(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let bytes = fs::read(path)?;

    if bytes.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "empty file"));
    }

    Ok(bytes)
}
