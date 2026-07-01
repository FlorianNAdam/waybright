use std::{error::Error, io};

use clap::{Parser, Subcommand};
use waybright_lib::{BrightnessChange, BrightnessControl, BrightnessDevice, brightness_devices};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    List,
    FocusedOutput,
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
    let Some(value) = value.strip_suffix('%') else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "brightness value must end with %",
        ));
    };

    if let Some(delta) = value.strip_prefix('+') {
        return parse_delta(delta).map(BrightnessChange::Delta);
    }

    if let Some(delta) = value.strip_prefix('-') {
        return parse_delta(delta).map(|delta| BrightnessChange::Delta(-delta));
    }

    if let Some(factor) = value.strip_prefix('*') {
        return parse_factor(factor).map(BrightnessChange::Multiply);
    }

    if let Some(factor) = value.strip_prefix('/') {
        return parse_factor(factor).map(BrightnessChange::Divide);
    }

    let percent = value
        .parse::<u8>()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;

    if percent > 100 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "brightness percent must be between 0 and 100",
        ));
    }

    Ok(BrightnessChange::Absolute(percent))
}

fn parse_delta(value: &str) -> io::Result<i8> {
    let delta = value
        .parse::<i8>()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;

    if delta > 100 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "brightness delta must be between -100 and 100",
        ));
    }

    Ok(delta)
}

fn parse_factor(value: &str) -> io::Result<u16> {
    let factor = value
        .parse::<u16>()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;

    if factor == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "brightness factor must be greater than 0%",
        ));
    }

    Ok(factor)
}

fn list_devices() -> Result<(), Box<dyn Error>> {
    for (name, device) in brightness_devices()? {
        println!("{name}");
        print_brightness_device(&name, &device);
    }

    Ok(())
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
        return waybright_focus::focused_output();
    }

    Ok(name.to_owned())
}

fn set_device_brightness(name: &str, percent: &str) -> Result<(), Box<dyn Error>> {
    let name = resolve_device_name(name)?;
    let change = parse_brightness_change(percent)?;
    let devices = brightness_devices()?;
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

fn print_focused_output() -> Result<(), Box<dyn Error>> {
    println!("{}", waybright_focus::focused_output()?);
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command.unwrap_or(Command::List) {
        Command::List => list_devices()?,
        Command::FocusedOutput => print_focused_output()?,
        Command::Get { name } => get_device_brightness(&name)?,
        Command::Set { name, percent } => set_device_brightness(&name, &percent)?,
    }

    Ok(())
}
