use std::error::Error;
use std::{
    collections::{BTreeMap, HashMap},
    fs, io,
    path::{Path, PathBuf},
};

use clap::{Parser, Subcommand};
use ddc::{Ddc, Edid};
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
    Get { name: String },
    Set { name: String, percent: String },
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

#[derive(Debug)]
struct BacklightMapping {
    backlight: String,
    path: PathBuf,
    connector: String,
    output: String,
    method: BacklightMappingMethod,
}

#[derive(Debug)]
struct BrightnessValue {
    current: u32,
    max: u32,
}

#[derive(Debug)]
enum BacklightMappingMethod {
    Ddc,
    Edid,
}

#[derive(Debug)]
struct DrmConnector {
    name: String,
    path: PathBuf,
}

#[derive(Debug)]
struct BacklightDevice {
    name: String,
    path: PathBuf,
}

#[derive(Debug)]
struct DdcCiDevice {
    i2c_bus: String,
    device: PathBuf,
    edid: Option<Vec<u8>>,
}

#[derive(Debug)]
struct DdcCiMapping {
    i2c_bus: String,
    device: PathBuf,
    connector: Option<String>,
    output: Option<String>,
}

#[derive(Debug)]
enum BrightnessDevice {
    Backlight(BacklightMapping),
    DdcCi(DdcCiMapping),
}

impl BrightnessDevice {
    fn brightness(&self) -> io::Result<BrightnessValue> {
        match self {
            BrightnessDevice::Backlight(mapping) => read_backlight_brightness(&mapping.path),
            BrightnessDevice::DdcCi(mapping) => read_ddcci_brightness(&mapping.device),
        }
    }

    fn set_brightness_percent(&self, percent: u8) -> io::Result<()> {
        match self {
            BrightnessDevice::Backlight(mapping) => {
                set_backlight_brightness_percent(&mapping.path, percent)
            }
            BrightnessDevice::DdcCi(mapping) => {
                set_ddcci_brightness_percent(&mapping.device, percent)
            }
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

fn map_backlights_to_connectors() -> io::Result<Vec<BacklightMapping>> {
    map_backlights_to_connectors_from(
        Path::new("/sys/class/drm"),
        Path::new("/sys/class/backlight"),
    )
}

fn map_ddcci_to_outputs() -> io::Result<Vec<DdcCiMapping>> {
    map_ddcci_to_outputs_from(
        Path::new("/sys/class/drm"),
        Path::new("/sys/class/i2c-dev"),
        Path::new("/dev"),
    )
}

fn brightness_devices() -> io::Result<BTreeMap<String, BrightnessDevice>> {
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

fn drm_connectors(path: &Path) -> io::Result<Vec<DrmConnector>> {
    let mut connectors = Vec::new();

    for entry in read_dir_optional(path)? {
        let name = entry.file_name().to_string_lossy().into_owned();

        if is_drm_connector_name(&name) {
            connectors.push(DrmConnector {
                name,
                path: entry.path(),
            });
        }
    }

    Ok(connectors)
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

fn read_dir_optional(path: &Path) -> io::Result<Vec<fs::DirEntry>> {
    match fs::read_dir(path) {
        Ok(entries) => entries.collect(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn is_drm_connector_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("card") else {
        return false;
    };

    let Some((card, connector)) = rest.split_once('-') else {
        return false;
    };

    !card.is_empty() && card.bytes().all(|byte| byte.is_ascii_digit()) && !connector.is_empty()
}

fn is_i2c_bus_name(name: &str) -> bool {
    let Some(bus) = name.strip_prefix("i2c-") else {
        return false;
    };

    !bus.is_empty() && bus.bytes().all(|byte| byte.is_ascii_digit())
}

fn connector_output_name(name: &str) -> &str {
    name.strip_prefix("card")
        .and_then(|rest| rest.split_once('-'))
        .map(|(_, output)| output)
        .unwrap_or(name)
}

fn unique_edids(items: impl Iterator<Item = (Vec<u8>, String)>) -> HashMap<Vec<u8>, String> {
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

fn read_non_empty(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let bytes = fs::read(path)?;

    if bytes.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "empty file"));
    }

    Ok(bytes)
}

fn read_base_edid(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let mut edid = read_non_empty(path)?;
    edid.truncate(128);

    if !is_valid_base_edid(&edid) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid edid"));
    }

    Ok(edid)
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

fn read_backlight_brightness(path: &Path) -> io::Result<BrightnessValue> {
    Ok(BrightnessValue {
        current: read_u32(path.join("brightness"))?,
        max: read_u32(path.join("max_brightness"))?,
    })
}

fn set_backlight_brightness_percent(path: &Path, percent: u8) -> io::Result<()> {
    let max = read_u32(path.join("max_brightness"))?;
    let value = percent_to_value(percent, max);
    fs::write(path.join("brightness"), format!("{value}\n"))
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

fn parse_percent(value: &str) -> io::Result<u8> {
    let percent = value
        .trim_end_matches('%')
        .parse::<u8>()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;

    if percent > 100 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "brightness percent must be between 0 and 100",
        ));
    }

    Ok(percent)
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
    let brightness = device.brightness();
    let brightness = brightness
        .as_ref()
        .map(|brightness| format!("{}%", brightness.percent()))
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
    let percent = parse_percent(percent)?;
    let devices = brightness_devices()?;
    let Some(device) = devices.get(name) else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no brightness device named {name}"),
        )
        .into());
    };

    device.set_brightness_percent(percent)?;
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

    let brightness = device.brightness()?;
    println!("{}%", brightness.percent());
    Ok(())
}

fn is_valid_base_edid(edid: &[u8]) -> bool {
    edid.len() == 128
        && edid.starts_with(&[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00])
        && edid.iter().fold(0_u8, |sum, byte| sum.wrapping_add(*byte)) == 0
}

fn main() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command.unwrap_or(Command::List) {
        Command::List => list_devices()?,
        Command::Get { name } => get_device_brightness(&name)?,
        Command::Set { name, percent } => set_device_brightness(&name, &percent)?,
    }

    Ok(())
}
