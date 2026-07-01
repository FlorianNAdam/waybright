use std::error::Error;
use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
};

use ddc::{Ddc, Edid};
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    protocol::{wl_output, wl_registry},
};

struct State {
    outputs: Vec<Output>,
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
    connector: String,
    output: String,
    method: BacklightMappingMethod,
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

    for entry in fs::read_dir(i2c_dev_path)? {
        let entry = entry?;
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
            output: connector_output_name(&connector).to_owned(),
            connector,
            method,
        });
    }

    Ok(mappings)
}

fn drm_connectors(path: &Path) -> io::Result<Vec<DrmConnector>> {
    let mut connectors = Vec::new();

    for entry in fs::read_dir(path)? {
        let entry = entry?;
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

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();

        devices.push(BacklightDevice {
            name,
            path: entry.path(),
        });
    }

    Ok(devices)
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

fn is_valid_base_edid(edid: &[u8]) -> bool {
    edid.len() == 128
        && edid.starts_with(&[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00])
        && edid.iter().fold(0_u8, |sum, byte| sum.wrapping_add(*byte)) == 0
}

fn main() -> Result<(), Box<dyn Error>> {
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

    for output in state.outputs {
        println!("{}", output.name.as_deref().unwrap_or("unknown"));
        println!(
            "\tdescription: {}",
            output.description.as_deref().unwrap_or("unknown")
        );
        println!("\tmake: {}", output.make);
        println!("\tmodel: {}", output.model);
        println!(
            "\tphysical size: {}x{}mm",
            output.physical_width, output.physical_height
        );

        if let (Some(width), Some(height)) = (output.current_width, output.current_height) {
            println!("\tcurrent mode: {width}x{height}");
        }
    }

    for mapping in map_backlights_to_connectors()? {
        println!(
            "backlight {} -> {} [{}] ({:?})",
            mapping.backlight, mapping.output, mapping.connector, mapping.method
        );
    }

    for mapping in map_ddcci_to_outputs()? {
        match (&mapping.output, &mapping.connector) {
            (Some(output), Some(connector)) => println!(
                "ddc/ci {} ({}) -> {} [{}]",
                mapping.i2c_bus,
                mapping.device.display(),
                output,
                connector
            ),
            _ => println!(
                "ddc/ci {} ({}) -> unmapped",
                mapping.i2c_bus,
                mapping.device.display()
            ),
        }
    }

    Ok(())
}
