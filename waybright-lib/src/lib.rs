use std::{collections::BTreeMap, fs, io, path::Path};

pub mod backlight;
pub mod ddcci;
mod drm;
mod edid;
mod logind;

use backlight::{BacklightMapping, map_backlights_to_connectors};
use ddcci::{DdcCiMapping, map_ddcci_to_outputs};

#[derive(Debug)]
pub struct BrightnessValue {
    current: u32,
    max: u32,
}

#[derive(Debug)]
pub enum BrightnessChange {
    Absolute(u8),
    Delta(i8),
    Multiply(u16),
    Divide(u16),
}

pub trait BrightnessControl {
    fn get_brightness(&self) -> io::Result<u32>;
    fn set_brightness(&self, percent: u8) -> io::Result<()>;
}

#[derive(Debug)]
pub enum BrightnessDevice {
    Backlight(BacklightMapping),
    DdcCi(DdcCiMapping),
}

impl BrightnessDevice {
    pub fn apply_brightness_change(&self, change: BrightnessChange) -> io::Result<()> {
        let percent = match change {
            BrightnessChange::Absolute(percent) => percent,
            BrightnessChange::Delta(delta) => {
                let current = self.get_brightness()? as i16;
                current.saturating_add(i16::from(delta)).clamp(0, 100) as u8
            }
            BrightnessChange::Multiply(factor) => {
                let current = self.get_brightness()?;
                ((current * u32::from(factor) + 50) / 100).clamp(0, 100) as u8
            }
            BrightnessChange::Divide(factor) => {
                let current = self.get_brightness()?;
                ((current * 100 + u32::from(factor) / 2) / u32::from(factor)).clamp(0, 100) as u8
            }
        };

        match self {
            BrightnessDevice::Backlight(mapping) => mapping.set_brightness(percent),
            BrightnessDevice::DdcCi(mapping) => mapping.set_brightness(percent),
        }
    }
}

impl BrightnessControl for BrightnessDevice {
    fn get_brightness(&self) -> io::Result<u32> {
        match self {
            BrightnessDevice::Backlight(mapping) => mapping.get_brightness(),
            BrightnessDevice::DdcCi(mapping) => mapping.get_brightness(),
        }
    }

    fn set_brightness(&self, percent: u8) -> io::Result<()> {
        match self {
            BrightnessDevice::Backlight(mapping) => mapping.set_brightness(percent),
            BrightnessDevice::DdcCi(mapping) => mapping.set_brightness(percent),
        }
    }
}

impl BrightnessValue {
    fn percent(&self) -> u32 {
        if self.max == 0 {
            return 0;
        }

        self.current * 100 / self.max
    }
}

pub fn brightness_devices() -> io::Result<BTreeMap<String, BrightnessDevice>> {
    let mut devices = BTreeMap::new();

    for mapping in map_backlights_to_connectors()? {
        devices.insert(mapping.output.clone(), BrightnessDevice::Backlight(mapping));
    }

    for mapping in map_ddcci_to_outputs()? {
        let name = mapping
            .output
            .clone()
            .unwrap_or_else(|| mapping.i2c_bus.clone());
        devices.insert(name, BrightnessDevice::DdcCi(mapping));
    }

    Ok(devices)
}

fn read_dir_optional(path: &Path) -> io::Result<Vec<fs::DirEntry>> {
    match fs::read_dir(path) {
        Ok(entries) => entries.collect(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn is_i2c_bus_name(name: &str) -> bool {
    let Some(bus) = name.strip_prefix("i2c-") else {
        return false;
    };

    !bus.is_empty() && bus.bytes().all(|byte| byte.is_ascii_digit())
}

fn read_u32(path: impl AsRef<Path>) -> io::Result<u32> {
    let value = fs::read_to_string(path)?;
    value
        .trim()
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn percent_to_value(percent: u8, max: u32) -> u32 {
    (u32::from(percent) * max).div_ceil(100)
}
