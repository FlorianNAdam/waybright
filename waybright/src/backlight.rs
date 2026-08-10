use std::{collections::HashMap, fs, io, path::Path, path::PathBuf};

use crate::drm::{connector_output_name, connectors as drm_connectors};
use crate::edid::{read_base_edid, unique_edids};
use crate::{BrightnessControl, BrightnessValue, percent_to_value, read_dir_optional, read_u32};

#[derive(Debug)]
pub struct BacklightMapping {
    pub backlight: String,
    pub path: PathBuf,
    pub connector: String,
    pub output: String,
    pub method: BacklightMappingMethod,
}

#[derive(Debug)]
pub enum BacklightMappingMethod {
    Ddc,
    Edid,
}

#[derive(Debug)]
struct BacklightDevice {
    name: String,
    path: PathBuf,
}

impl BrightnessControl for BacklightMapping {
    fn get_brightness(&self) -> io::Result<u32> {
        read_backlight_brightness(&self.path).map(|brightness| brightness.percent())
    }

    fn set_brightness(&self, percent: u8) -> io::Result<()> {
        set_backlight_brightness_percent(&self.backlight, &self.path, percent)
    }
}

pub(crate) fn map_backlights_to_connectors() -> io::Result<Vec<BacklightMapping>> {
    map_backlights_to_connectors_from(
        Path::new("/sys/class/drm"),
        Path::new("/sys/class/backlight"),
    )
}

fn map_backlights_to_connectors_from(
    drm_path: &Path,
    backlight_path: &Path,
) -> io::Result<Vec<BacklightMapping>> {
    let connectors = drm_connectors(drm_path)?;
    let backlights = backlight_devices(backlight_path)?;

    let mut mappings = Vec::new();
    let mut mapped_backlights = HashMap::new();

    let mut connectors_by_ddc = HashMap::new();
    for connector in &connectors {
        if let Ok(ddc) = fs::canonicalize(connector.path.join("ddc")) {
            connectors_by_ddc.insert(ddc, connector.name.clone());
        }
    }

    for backlight in &backlights {
        let Ok(ddc) = fs::canonicalize(backlight.path.join("device/ddc")) else {
            continue;
        };

        let Some(connector) = connectors_by_ddc.get(&ddc) else {
            continue;
        };

        mapped_backlights.insert(
            backlight.name.clone(),
            (connector.clone(), BacklightMappingMethod::Ddc),
        );
    }

    let mut connectors_by_edid = unique_edids(connectors.iter().filter_map(|connector| {
        read_base_edid(connector.path.join("edid"))
            .ok()
            .map(|edid| (edid, connector.name.clone()))
    }));

    for backlight in &backlights {
        if mapped_backlights.contains_key(&backlight.name) {
            continue;
        }

        let Ok(edid) = read_base_edid(backlight.path.join("device/edid")) else {
            continue;
        };

        let Some(connector) = connectors_by_edid.remove(&edid) else {
            continue;
        };

        mapped_backlights.insert(
            backlight.name.clone(),
            (connector, BacklightMappingMethod::Edid),
        );
    }

    for backlight in backlights {
        let Some((connector, method)) = mapped_backlights.remove(&backlight.name) else {
            continue;
        };

        mappings.push(BacklightMapping {
            backlight: backlight.name,
            path: backlight.path,
            output: connector_output_name(&connector).to_owned(),
            connector,
            method,
        });
    }

    Ok(mappings)
}

fn backlight_devices(path: &Path) -> io::Result<Vec<BacklightDevice>> {
    let mut devices = Vec::new();

    for entry in read_dir_optional(path)? {
        let name = entry.file_name().to_string_lossy().into_owned();

        devices.push(BacklightDevice {
            name,
            path: entry.path(),
        });
    }

    Ok(devices)
}

fn read_backlight_brightness(path: &Path) -> io::Result<BrightnessValue> {
    Ok(BrightnessValue {
        current: read_u32(path.join("brightness"))?,
        max: read_u32(path.join("max_brightness"))?,
    })
}

fn set_backlight_brightness_percent(backlight: &str, path: &Path, percent: u8) -> io::Result<()> {
    let max = read_u32(path.join("max_brightness"))?;
    let value = percent_to_value(percent, max);

    match fs::write(path.join("brightness"), format!("{value}\n")) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            crate::logind::set_brightness("backlight", backlight, value)
        }
        Err(error) => Err(error),
    }
}
