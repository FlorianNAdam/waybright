use std::{io, path::Path, path::PathBuf};

use ddc::{Ddc, Edid};

use crate::drm::{connector_output_name, connectors as drm_connectors};
use crate::edid::{is_valid_base_edid, read_base_edid, unique_edids};
use crate::{
    BrightnessControl, BrightnessValue, is_i2c_bus_name, percent_to_value, read_dir_optional,
};

#[derive(Debug)]
pub struct DdcCiMapping {
    pub i2c_bus: String,
    pub device: PathBuf,
    pub connector: Option<String>,
    pub output: Option<String>,
}

#[derive(Debug)]
struct DdcCiDevice {
    i2c_bus: String,
    device: PathBuf,
    edid: Option<Vec<u8>>,
}

impl BrightnessControl for DdcCiMapping {
    fn get_brightness(&self) -> io::Result<u32> {
        read_ddcci_brightness(&self.device).map(|brightness| brightness.percent())
    }

    fn set_brightness(&self, percent: u8) -> io::Result<()> {
        set_ddcci_brightness_percent(&self.device, percent)
    }
}

pub(crate) fn map_ddcci_to_outputs() -> io::Result<Vec<DdcCiMapping>> {
    map_ddcci_to_outputs_from(
        Path::new("/sys/class/drm"),
        Path::new("/sys/class/i2c-dev"),
        Path::new("/dev"),
    )
}

fn map_ddcci_to_outputs_from(
    drm_path: &Path,
    i2c_dev_path: &Path,
    dev_path: &Path,
) -> io::Result<Vec<DdcCiMapping>> {
    let devices = discover_ddcci_devices_from(i2c_dev_path, dev_path)?;
    let connectors = drm_connectors(drm_path)?;
    let mut connectors_by_edid = unique_edids(connectors.iter().filter_map(|connector| {
        read_base_edid(connector.path.join("edid"))
            .ok()
            .map(|edid| (edid, connector.name.clone()))
    }));

    Ok(devices
        .into_iter()
        .map(|device| {
            let connector = device
                .edid
                .as_ref()
                .and_then(|edid| connectors_by_edid.remove(edid));
            let output = connector
                .as_deref()
                .map(connector_output_name)
                .map(str::to_owned);

            DdcCiMapping {
                i2c_bus: device.i2c_bus,
                device: device.device,
                connector,
                output,
            }
        })
        .collect())
}

fn discover_ddcci_devices_from(
    i2c_dev_path: &Path,
    dev_path: &Path,
) -> io::Result<Vec<DdcCiDevice>> {
    let mut devices = Vec::new();

    for entry in read_dir_optional(i2c_dev_path)? {
        let i2c_bus = entry.file_name().to_string_lossy().into_owned();

        if !is_i2c_bus_name(&i2c_bus) {
            continue;
        }

        let device = dev_path.join(&i2c_bus);
        let Ok(mut ddc) = ddc_i2c::from_i2c_device(&device) else {
            continue;
        };

        if ddc.get_vcp_feature(0x10).is_err() {
            continue;
        };

        devices.push(DdcCiDevice {
            i2c_bus,
            device,
            edid: read_ddc_edid(&mut ddc).ok(),
        });
    }

    Ok(devices)
}

fn read_ddc_edid(ddc: &mut ddc_i2c::I2cDeviceDdc) -> io::Result<Vec<u8>> {
    let mut edid = vec![0_u8; 128];
    let len = ddc.read_edid(0, &mut edid).map_err(io::Error::other)?;
    edid.truncate(len);

    if !is_valid_base_edid(&edid) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid edid"));
    }

    Ok(edid)
}

fn read_ddcci_brightness(path: &Path) -> io::Result<BrightnessValue> {
    let mut ddc = ddc_i2c::from_i2c_device(path)?;
    let value = ddc.get_vcp_feature(0x10).map_err(io::Error::other)?;

    Ok(BrightnessValue {
        current: u32::from(value.value()),
        max: u32::from(value.maximum()),
    })
}

fn set_ddcci_brightness_percent(path: &Path, percent: u8) -> io::Result<()> {
    let mut ddc = ddc_i2c::from_i2c_device(path)?;
    let max = u32::from(
        ddc.get_vcp_feature(0x10)
            .map_err(io::Error::other)?
            .maximum(),
    );
    let value = percent_to_value(percent, max);
    ddc.set_vcp_feature(0x10, value as u16)
        .map_err(io::Error::other)
}
