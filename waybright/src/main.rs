use std::{collections::BTreeMap, error::Error, io};

use clap::{Parser, Subcommand};
use serde_json::{Map, json};
use waydim::{BrightnessControl, BrightnessDevice, brightness_devices};
use waylevel::{PercentChange, apply_percent_change, parse_percent_change};

#[derive(Parser)]
struct Cli {
    #[arg(long, default_value_t = 50)]
    hardware_min: u8,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    List {
        #[arg(long)]
        json: bool,
    },
    Get {
        name: String,
    },
    Set {
        name: String,
        #[arg(allow_hyphen_values = true)]
        percent: String,
    },
}

struct CombinedDevice {
    name: String,
    hardware: Option<HardwareDevice>,
    hardware_brightness: Option<u32>,
    software: Option<u8>,
}

enum HardwareDevice {
    Direct(BrightnessDevice),
    Daemon,
}

fn parse_brightness_change(value: &str) -> io::Result<PercentChange> {
    parse_percent_change(value)
}

fn validate_hardware_min(hardware_min: u8) -> io::Result<()> {
    if hardware_min == 0 || hardware_min >= 100 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "hardware minimum must be between 1 and 99",
        ));
    }

    Ok(())
}

fn resolve_name(name: &str) -> io::Result<String> {
    if name == "@focused" {
        return wayfocus::focused_output();
    }

    Ok(name.to_owned())
}

fn combined_devices() -> Result<BTreeMap<String, CombinedDevice>, Box<dyn Error>> {
    let hardware_devices = hardware_devices()?;
    let software_outputs = software_outputs()?
        .into_iter()
        .map(|output| (output.name, output.brightness))
        .collect::<BTreeMap<_, _>>();

    let mut devices = BTreeMap::new();
    for (name, (hardware, hardware_brightness)) in hardware_devices {
        devices.insert(
            name.clone(),
            CombinedDevice {
                software: software_outputs.get(&name).copied(),
                hardware: Some(hardware),
                hardware_brightness,
                name,
            },
        );
    }

    for (name, software) in software_outputs {
        devices.entry(name.clone()).or_insert(CombinedDevice {
            name,
            hardware: None,
            hardware_brightness: None,
            software: Some(software),
        });
    }

    Ok(devices)
}

fn hardware_devices() -> io::Result<BTreeMap<String, (HardwareDevice, Option<u32>)>> {
    match waydim::daemon_list_devices() {
        Ok(devices) => Ok(devices
            .into_iter()
            .map(|device| {
                (
                    device.name,
                    (HardwareDevice::Daemon, Some(device.brightness)),
                )
            })
            .collect()),
        Err(error) if daemon_unavailable(&error) => Ok(brightness_devices()?
            .into_iter()
            .map(|(name, device)| (name, (HardwareDevice::Direct(device), None)))
            .collect()),
        Err(error) => Err(error),
    }
}

fn software_outputs() -> io::Result<Vec<waydark::OutputBrightness>> {
    match waydark::daemon_list_outputs() {
        Ok(outputs) => Ok(outputs),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            Ok(Vec::new())
        }
        Err(error) => Err(error),
    }
}

fn device_brightness(device: &CombinedDevice, hardware_min: u8) -> io::Result<u8> {
    match (&device.hardware, device.software) {
        (Some(_), Some(software)) => Ok(effective_brightness(
            hardware_brightness(device)?,
            software,
            hardware_min,
        )),
        (Some(_), None) => Ok(hardware_brightness(device)?.clamp(0, 100) as u8),
        (None, Some(software)) => Ok(software),
        (None, None) => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no brightness device named {}", device.name),
        )),
    }
}

fn hardware_brightness(device: &CombinedDevice) -> io::Result<u32> {
    if let Some(brightness) = device.hardware_brightness {
        return Ok(brightness);
    }

    match &device.hardware {
        Some(HardwareDevice::Direct(hardware)) => hardware.get_brightness(),
        Some(HardwareDevice::Daemon) => waydim::daemon_get_brightness(&device.name),
        None => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no hardware brightness device named {}", device.name),
        )),
    }
}

fn effective_brightness(hardware: u32, software: u8, hardware_min: u8) -> u8 {
    let hardware_min = u32::from(hardware_min);

    if software < 100 {
        return ((u32::from(software) * hardware_min + 50) / 100).clamp(0, 100) as u8;
    }

    (hardware_min + (hardware.clamp(0, 100) * (100 - hardware_min) + 50) / 100).clamp(0, 100) as u8
}

fn split_brightness(percent: u8, hardware_min: u8) -> (u8, u8) {
    if percent >= hardware_min {
        let hardware = ((u32::from(percent - hardware_min) * 100
            + u32::from(100 - hardware_min) / 2)
            / u32::from(100 - hardware_min))
        .clamp(0, 100) as u8;

        return (hardware, 100);
    }

    let software = ((u32::from(percent) * 100 + u32::from(hardware_min) / 2)
        / u32::from(hardware_min))
    .clamp(0, 100) as u8;

    (0, software)
}

fn list_devices(hardware_min: u8, json: bool) -> Result<(), Box<dyn Error>> {
    let devices = combined_devices()?;

    if json {
        let mut json_devices = Map::new();
        for device in devices.values() {
            let hardware = device
                .hardware
                .as_ref()
                .map(|_| hardware_brightness(device))
                .transpose()?;
            let effective = device_brightness(device, hardware_min)?;

            json_devices.insert(
                device.name.clone(),
                json!({
                    "brightness": effective,
                    "hardware": hardware,
                    "software": device.software,
                }),
            );
        }

        println!("{}", serde_json::to_string_pretty(&json_devices)?);
    } else {
        for device in devices.values() {
            let hardware = device
                .hardware
                .as_ref()
                .map(|_| hardware_brightness(device))
                .transpose()?;
            let effective = device_brightness(device, hardware_min)?;

            println!("{}", device.name);
            println!("  brightness: {effective}%");
            match hardware {
                Some(hardware) => println!("  hardware: {hardware}%"),
                None => println!("  hardware: unavailable"),
            }
            match device.software {
                Some(software) => println!("  software: {software}%"),
                None => println!("  software: unavailable"),
            }
        }
    }

    Ok(())
}

fn get_brightness(name: &str, hardware_min: u8) -> Result<(), Box<dyn Error>> {
    let name = resolve_name(name)?;
    let devices = combined_devices()?;
    let Some(device) = devices.get(&name) else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no brightness device named {name}"),
        )
        .into());
    };

    println!("{}%", device_brightness(device, hardware_min)?);
    Ok(())
}

fn set_one(device: &CombinedDevice, percent: u8, hardware_min: u8) -> Result<(), Box<dyn Error>> {
    match (&device.hardware, device.software) {
        (Some(_), Some(_)) => {
            let (hardware, software) = split_brightness(percent, hardware_min);
            set_hardware_brightness(device, hardware)?;
            waydark::daemon_set_brightness(&device.name, software)?;
        }
        (Some(_), None) => set_hardware_brightness(device, percent)?,
        (None, Some(_)) => waydark::daemon_set_brightness(&device.name, percent)?,
        (None, None) => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no brightness device named {}", device.name),
            )
            .into());
        }
    }

    Ok(())
}

fn set_hardware_brightness(device: &CombinedDevice, percent: u8) -> io::Result<()> {
    match &device.hardware {
        Some(HardwareDevice::Direct(hardware)) => hardware.set_brightness(percent),
        Some(HardwareDevice::Daemon) => waydim::daemon_set_brightness(&device.name, percent),
        None => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no hardware brightness device named {}", device.name),
        )),
    }
}

fn daemon_unavailable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

fn set_brightness(name: &str, percent: &str, hardware_min: u8) -> Result<(), Box<dyn Error>> {
    let change = parse_brightness_change(percent)?;
    let devices = combined_devices()?;

    if name == "@all" {
        for device in devices.values() {
            let current = device_brightness(device, hardware_min)?;
            let percent = apply_percent_change(Some(u32::from(current)), change);
            set_one(device, percent, hardware_min)?;
        }

        return Ok(());
    }

    let name = resolve_name(name)?;
    let Some(device) = devices.get(&name) else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no brightness device named {name}"),
        )
        .into());
    };

    let current = device_brightness(device, hardware_min)?;
    let percent = apply_percent_change(Some(u32::from(current)), change);
    set_one(device, percent, hardware_min)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    validate_hardware_min(cli.hardware_min)?;

    match cli.command {
        Command::List { json } => list_devices(cli.hardware_min, json),
        Command::Get { name } => get_brightness(&name, cli.hardware_min),
        Command::Set { name, percent } => set_brightness(&name, &percent, cli.hardware_min),
    }
}
