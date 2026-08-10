use std::{error::Error, io};

use clap::{Parser, Subcommand};
use serde_json::{Map, Value, json};
use waybright::{BrightnessChange, BrightnessControl, BrightnessDevice, brightness_devices};
use waylevel::parse_percent_change;

#[derive(Parser)]
struct Cli {
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

fn parse_brightness_change(value: &str) -> io::Result<BrightnessChange> {
    parse_percent_change(value)
}

fn list_devices(json: bool) -> Result<(), Box<dyn Error>> {
    let devices = brightness_devices()?;

    if json {
        print_json_devices(devices)?;
    } else {
        for (name, device) in devices {
            println!("{name}");
            print_brightness_device(&name, &device);
        }
    }

    Ok(())
}

fn print_json_devices(
    devices: std::collections::BTreeMap<String, BrightnessDevice>,
) -> Result<(), Box<dyn Error>> {
    let mut json_devices = Map::new();

    for (name, device) in devices {
        json_devices.insert(name.clone(), json_brightness_device(&name, &device));
    }

    println!("{}", serde_json::to_string_pretty(&json_devices)?);
    Ok(())
}

fn json_brightness_device(name: &str, device: &BrightnessDevice) -> Value {
    let (brightness, brightness_error) = match device.get_brightness() {
        Ok(brightness) => (Some(brightness), None),
        Err(error) => (None, Some(error.to_string())),
    };

    match device {
        BrightnessDevice::Backlight(mapping) => json!({
            "brightness": brightness,
            "brightness_error": brightness_error,
            "method": "backlight",
            "backlight": mapping.backlight,
            "connector": mapping.connector,
            "mapping_method": format!("{:?}", mapping.method).to_lowercase(),
        }),
        BrightnessDevice::DdcCi(mapping) => json!({
            "brightness": brightness,
            "brightness_error": brightness_error,
            "method": "ddc/ci",
            "i2c_bus": mapping.i2c_bus,
            "device": mapping.device,
            "connector": mapping.connector,
            "output": mapping.output.as_deref().unwrap_or(name),
        }),
    }
}

fn print_brightness_device(name: &str, device: &BrightnessDevice) {
    let brightness = device.get_brightness();
    let brightness = brightness
        .as_ref()
        .map(|brightness| format!("{brightness}%"))
        .unwrap_or_else(|error| format!("unknown ({error})"));

    println!("  brightness: {brightness}");

    match device {
        BrightnessDevice::Backlight(mapping) => {
            println!("  brightness method: backlight");
            println!("  backlight: {}", mapping.backlight);
            println!("  connector: {}", mapping.connector);
            println!("  mapping method: {:?}", mapping.method);
        }
        BrightnessDevice::DdcCi(mapping) => match &mapping.connector {
            Some(connector) => {
                println!("  brightness method: ddc/ci");
                println!("  i2c bus: {}", mapping.i2c_bus);
                println!("  device: {}", mapping.device.display());
                println!("  connector: {connector}");
            }
            None => {
                println!("  brightness method: ddc/ci");
                println!("  i2c bus: {}", mapping.i2c_bus);
                println!("  device: {}", mapping.device.display());
                println!("  output: {name}");
            }
        },
    }
}

fn resolve_device_name(name: &str) -> io::Result<String> {
    if name == "@focused" {
        return wayfocus::focused_output();
    }

    Ok(name.to_owned())
}

fn set_device_brightness(name: &str, percent: &str) -> Result<(), Box<dyn Error>> {
    let change = parse_brightness_change(percent)?;
    let devices = brightness_devices()?;

    if name == "@all" {
        for device in devices.values() {
            device.apply_brightness_change(change)?;
        }

        return Ok(());
    }

    let name = resolve_device_name(name)?;
    let Some(device) = devices.get(&name) else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no brightness device named {name}"),
        )
        .into());
    };

    device.apply_brightness_change(change)?;
    Ok(())
}

fn get_device_brightness(name: &str) -> Result<(), Box<dyn Error>> {
    let name = resolve_device_name(name)?;
    let devices = brightness_devices()?;
    let Some(device) = devices.get(&name) else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no brightness device named {name}"),
        )
        .into());
    };

    let brightness = device.get_brightness()?;
    println!("{brightness}%");
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Command::List { json } => list_devices(json)?,
        Command::Get { name } => get_device_brightness(&name)?,
        Command::Set { name, percent } => set_device_brightness(&name, &percent)?,
    }

    Ok(())
}
