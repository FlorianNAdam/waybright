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
    hardware: BrightnessDevice,
    software: Option<u8>,
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
    let hardware_devices = brightness_devices()?;
    let software_outputs = software_outputs()?
        .into_iter()
        .map(|output| (output.name, output.brightness))
        .collect::<BTreeMap<_, _>>();

    let mut devices = BTreeMap::new();
    for (name, hardware) in hardware_devices {
        devices.insert(
            name.clone(),
            CombinedDevice {
                software: software_outputs.get(&name).copied(),
                hardware,
                name,
            },
        );
    }

    Ok(devices)
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
    let hardware = device.hardware.get_brightness()?;
    let Some(software) = device.software else {
        return Ok(hardware.clamp(0, 100) as u8);
    };

    Ok(effective_brightness(hardware, software, hardware_min))
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
            let hardware = device.hardware.get_brightness()?;
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
            let hardware = device.hardware.get_brightness()?;
            let effective = device_brightness(device, hardware_min)?;

            println!("{}", device.name);
            println!("  brightness: {effective}%");
            println!("  hardware: {hardware}%");
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
            format!("no combined brightness device named {name}"),
        )
        .into());
    };

    println!("{}%", device_brightness(device, hardware_min)?);
    Ok(())
}

fn set_one(device: &CombinedDevice, percent: u8, hardware_min: u8) -> Result<(), Box<dyn Error>> {
    let Some(_) = device.software else {
        device.hardware.set_brightness(percent)?;
        return Ok(());
    };

    let (hardware, software) = split_brightness(percent, hardware_min);
    device.hardware.set_brightness(hardware)?;
    waydark::daemon_set_brightness(&device.name, software)?;
    Ok(())
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
            format!("no combined brightness device named {name}"),
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
