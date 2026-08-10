use std::{error::Error, io, thread};

use clap::{Parser, Subcommand};
use serde_json::{Map, Value, json};
use waydim::{
    BrightnessChange, BrightnessControl, BrightnessDevice, DaemonBrightnessDevice,
    brightness_devices,
};
use waylevel::parse_percent_change;

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Daemon,
    Refresh,
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
    match waydim::daemon_list_devices() {
        Ok(devices) => return print_daemon_devices(devices, json),
        Err(error) if daemon_unavailable(&error) => {}
        Err(error) => return Err(error.into()),
    }

    let devices = brightness_devices()?;

    if json {
        print_json_devices(devices)?;
    } else {
        for (name, device) in devices {
            println!("{name}");
            print_brightness_device(&device);
        }
    }

    Ok(())
}

fn print_daemon_devices(
    devices: Vec<DaemonBrightnessDevice>,
    json: bool,
) -> Result<(), Box<dyn Error>> {
    if json {
        let mut json_devices = Map::new();
        for device in devices {
            json_devices.insert(device.name.clone(), json_daemon_device(&device));
        }

        println!("{}", serde_json::to_string_pretty(&json_devices)?);
    } else {
        for device in devices {
            println!("{}", device.name);
            print_daemon_device(&device);
        }
    }

    Ok(())
}

fn json_daemon_device(device: &DaemonBrightnessDevice) -> Value {
    match device.method.as_str() {
        "backlight" => json!({
            "brightness": device.brightness,
            "brightness_error": null,
            "method": "backlight",
            "backlight": device.backlight,
            "connector": device.connector,
            "mapping_method": device.mapping_method,
        }),
        "ddc/ci" => json!({
            "brightness": device.brightness,
            "brightness_error": null,
            "method": "ddc/ci",
            "i2c_bus": device.i2c_bus,
            "device": device.device,
            "connector": device.connector,
        }),
        _ => json!({
            "brightness": device.brightness,
            "brightness_error": null,
            "method": device.method,
        }),
    }
}

fn print_daemon_device(device: &DaemonBrightnessDevice) {
    println!("  brightness: {}%", device.brightness);
    println!("  brightness method: {}", device.method);

    if let Some(backlight) = &device.backlight {
        println!("  backlight: {backlight}");
    }
    if let Some(i2c_bus) = &device.i2c_bus {
        println!("  i2c bus: {i2c_bus}");
    }
    if let Some(path) = &device.device {
        println!("  device: {}", path.display());
    }
    if let Some(connector) = &device.connector {
        println!("  connector: {connector}");
    }
    if let Some(mapping_method) = &device.mapping_method {
        println!("  mapping method: {mapping_method}");
    }
}

fn print_json_devices(
    devices: std::collections::BTreeMap<String, BrightnessDevice>,
) -> Result<(), Box<dyn Error>> {
    let mut json_devices = Map::new();

    for (name, device) in devices {
        json_devices.insert(name.clone(), json_brightness_device(&device));
    }

    println!("{}", serde_json::to_string_pretty(&json_devices)?);
    Ok(())
}

fn json_brightness_device(device: &BrightnessDevice) -> Value {
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
        }),
    }
}

fn print_brightness_device(device: &BrightnessDevice) {
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

    match set_device_brightness_with_daemon(name, change) {
        Ok(()) => return Ok(()),
        Err(error) if daemon_unavailable(&error) => {}
        Err(error) => return Err(error.into()),
    }

    let devices = brightness_devices()?;

    if name == "@all" {
        thread::scope(|scope| {
            devices
                .values()
                .map(|device| scope.spawn(move || device.apply_brightness_change(change)))
                .collect::<Vec<_>>()
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .unwrap_or_else(|_| Err(io::Error::other("brightness worker panicked")))
                })
                .collect::<io::Result<Vec<_>>>()
        })?;

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

fn set_device_brightness_with_daemon(name: &str, change: BrightnessChange) -> io::Result<()> {
    if name == "@all" {
        let mut targets = Vec::new();
        for device in waydim::daemon_list_devices()? {
            let percent = waylevel::apply_percent_change(Some(device.brightness), change);
            targets.push((device.name, percent));
        }

        waydim::daemon_set_all_brightness(&targets)?;

        return Ok(());
    }

    let name = resolve_device_name(name)?;
    let current = match change {
        BrightnessChange::Absolute(_) => None,
        BrightnessChange::Delta(_)
        | BrightnessChange::Multiply(_)
        | BrightnessChange::Divide(_) => Some(waydim::daemon_get_brightness(&name)?),
    };
    let percent = waylevel::apply_percent_change(current, change);
    waydim::daemon_set_brightness(&name, percent)
}

fn get_device_brightness(name: &str) -> Result<(), Box<dyn Error>> {
    let name = resolve_device_name(name)?;

    match waydim::daemon_get_brightness(&name) {
        Ok(brightness) => {
            println!("{brightness}%");
            return Ok(());
        }
        Err(error) if daemon_unavailable(&error) => {}
        Err(error) => return Err(error.into()),
    }

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

fn daemon_unavailable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

fn main() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Command::Daemon => waydim::run_daemon()?,
        Command::Refresh => waydim::daemon_refresh()?,
        Command::List { json } => list_devices(json)?,
        Command::Get { name } => get_device_brightness(&name)?,
        Command::Set { name, percent } => set_device_brightness(&name, &percent)?,
    }

    Ok(())
}
