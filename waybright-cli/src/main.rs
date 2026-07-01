use std::{error::Error, io};

use clap::{Parser, Subcommand};
use waybright_lib::{BrightnessChange, BrightnessControl, BrightnessDevice, brightness_devices};
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    protocol::{wl_output, wl_registry},
};

struct State {
    outputs: Vec<Output>,
}

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    List,
    Get {
        name: String,
    },
    Set {
        name: String,
        #[arg(allow_hyphen_values = true)]
        percent: String,
    },
}

struct Output {
    _wl_output: wl_output::WlOutput,
    global_name: u32,
    name: Option<String>,
    description: Option<String>,
    make: String,
    model: String,
    physical_width: i32,
    physical_height: i32,
    current_width: Option<i32>,
    current_height: Option<i32>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            if interface == "wl_output" {
                let wl_output =
                    registry.bind::<wl_output::WlOutput, _, _>(name, version.min(4), qh, name);

                state.outputs.push(Output {
                    _wl_output: wl_output,
                    global_name: name,
                    name: None,
                    description: None,
                    make: String::new(),
                    model: String::new(),
                    physical_width: 0,
                    physical_height: 0,
                    current_width: None,
                    current_height: None,
                });
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, u32> for State {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        global_name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(output) = state
            .outputs
            .iter_mut()
            .find(|output| output.global_name == *global_name)
        else {
            return;
        };

        match event {
            wl_output::Event::Geometry {
                physical_width,
                physical_height,
                make,
                model,
                ..
            } => {
                output.physical_width = physical_width;
                output.physical_height = physical_height;
                output.make = make;
                output.model = model;
            }
            wl_output::Event::Name { name } => output.name = Some(name),
            wl_output::Event::Description { description } => output.description = Some(description),
            wl_output::Event::Mode {
                flags,
                width,
                height,
                ..
            } => {
                if matches!(flags.into_result(), Ok(wl_output::Mode::Current)) {
                    output.current_width = Some(width);
                    output.current_height = Some(height);
                }
            }
            _ => {}
        }
    }
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
    let conn = Connection::connect_to_env()?;
    let display = conn.display();
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();
    let _registry = display.get_registry(&qh, ());

    let mut state = State {
        outputs: Vec::new(),
    };
    event_queue.roundtrip(&mut state)?;
    event_queue.roundtrip(&mut state)?;

    let mut devices = brightness_devices()?;

    for output in state.outputs {
        let name = output.name.as_deref().unwrap_or("unknown");
        println!("{name}");
        println!(
            "  description: {}",
            output.description.as_deref().unwrap_or("unknown")
        );
        println!("  make: {}", output.make);
        println!("  model: {}", output.model);
        println!(
            "  physical size: {}x{}mm",
            output.physical_width, output.physical_height
        );

        if let (Some(width), Some(height)) = (output.current_width, output.current_height) {
            println!("  current mode: {width}x{height}");
        }

        if let Some(device) = devices.remove(name) {
            print_brightness_device(name, &device);
        }
    }

    if !devices.is_empty() {
        println!("unmapped brightness devices");

        for (name, device) in devices {
            println!("{name}");
            print_brightness_device(&name, &device);
        }
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

fn set_device_brightness(name: &str, percent: &str) -> Result<(), Box<dyn Error>> {
    let change = parse_brightness_change(percent)?;
    let devices = brightness_devices()?;
    let Some(device) = devices.get(name) else {
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
    let devices = brightness_devices()?;
    let Some(device) = devices.get(name) else {
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
    match Cli::parse().command.unwrap_or(Command::List) {
        Command::List => list_devices()?,
        Command::Get { name } => get_device_brightness(&name)?,
        Command::Set { name, percent } => set_device_brightness(&name, &percent)?,
    }

    Ok(())
}
