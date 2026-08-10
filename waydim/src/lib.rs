use std::{
    collections::BTreeMap,
    env, fs, io,
    io::{Read, Write},
    os::fd::FromRawFd,
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    process, thread,
};

pub mod backlight;
pub mod ddcci;
mod drm;
mod edid;
mod logind;

use backlight::{BacklightMapping, map_backlights_to_connectors};
use ddcci::{DdcCiMapping, map_ddcci_to_outputs};

pub use waylevel::PercentChange as BrightnessChange;

#[derive(Debug)]
pub struct BrightnessValue {
    current: u32,
    max: u32,
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

#[derive(Debug, Clone)]
pub struct DaemonBrightnessDevice {
    pub name: String,
    pub brightness: u32,
    pub method: String,
    pub backlight: Option<String>,
    pub connector: Option<String>,
    pub mapping_method: Option<String>,
    pub i2c_bus: Option<String>,
    pub device: Option<PathBuf>,
}

struct DaemonState {
    devices: BTreeMap<String, BrightnessDevice>,
    brightness: BTreeMap<String, u32>,
}

impl BrightnessDevice {
    pub fn apply_brightness_change(&self, change: BrightnessChange) -> io::Result<()> {
        let current = match change {
            BrightnessChange::Absolute(_) => None,
            BrightnessChange::Delta(_)
            | BrightnessChange::Multiply(_)
            | BrightnessChange::Divide(_) => Some(self.get_brightness()?),
        };

        let percent = waylevel::apply_percent_change(current, change);

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

pub fn run_daemon() -> io::Result<()> {
    let mut state = DaemonState::new()?;
    let (listener, socket_path) = daemon_listener()?;
    println!("waydim daemon listening on {}", socket_path.display());

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => handle_client(stream, &mut state)?,
            Err(error) => return Err(error),
        }
    }

    Ok(())
}

pub fn daemon_list_devices() -> io::Result<Vec<DaemonBrightnessDevice>> {
    parse_list_response(&send_daemon_request("LIST")?)
}

pub fn daemon_get_brightness(name: &str) -> io::Result<u32> {
    send_daemon_request(&format!("GET {name}"))?
        .trim()
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub fn daemon_set_brightness(name: &str, brightness: u8) -> io::Result<()> {
    let response = send_daemon_request(&format!("SET {name} {brightness}"))?;
    if response.trim() == "OK" {
        Ok(())
    } else {
        Err(io::Error::other(response.trim().to_owned()))
    }
}

pub fn daemon_set_all_brightness(targets: &[(String, u8)]) -> io::Result<()> {
    let mut request = String::from("SET_ALL");
    for (name, brightness) in targets {
        request.push(' ');
        request.push_str(name);
        request.push(' ');
        request.push_str(&brightness.to_string());
    }

    let response = send_daemon_request(&request)?;
    if response.trim() == "OK" {
        Ok(())
    } else {
        Err(io::Error::other(response.trim().to_owned()))
    }
}

pub fn daemon_refresh() -> io::Result<()> {
    let response = send_daemon_request("REFRESH")?;
    if response.trim() == "OK" {
        Ok(())
    } else {
        Err(io::Error::other(response.trim().to_owned()))
    }
}

impl DaemonState {
    fn new() -> io::Result<Self> {
        let mut state = Self {
            devices: BTreeMap::new(),
            brightness: BTreeMap::new(),
        };
        state.refresh()?;
        Ok(state)
    }

    fn refresh(&mut self) -> io::Result<()> {
        let devices = brightness_devices()?;
        let mut brightness = BTreeMap::new();

        for (name, device) in &devices {
            if let Ok(value) = device.get_brightness() {
                brightness.insert(name.clone(), value);
            }
        }

        self.devices = devices;
        self.brightness = brightness;
        Ok(())
    }

    fn list(&self) -> Vec<DaemonBrightnessDevice> {
        self.devices
            .iter()
            .filter_map(|(name, device)| {
                let brightness = *self.brightness.get(name)?;
                Some(daemon_device(name, device, brightness))
            })
            .collect()
    }

    fn get_brightness(&self, name: &str) -> io::Result<u32> {
        self.brightness.get(name).copied().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("no brightness device named {name}"),
            )
        })
    }

    fn set_brightness(&mut self, name: &str, percent: u8) -> io::Result<()> {
        let Some(device) = self.devices.get(name) else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no brightness device named {name}"),
            ));
        };

        device.set_brightness(percent)?;
        self.brightness.insert(name.to_owned(), u32::from(percent));
        Ok(())
    }

    fn set_all_brightness(&mut self, targets: Vec<(String, u8)>) -> io::Result<()> {
        for (name, _) in &targets {
            if !self.devices.contains_key(name) {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("no brightness device named {name}"),
                ));
            }
        }

        let results = thread::scope(|scope| {
            targets
                .iter()
                .map(|(name, percent)| {
                    let device = &self.devices[name];
                    scope.spawn(move || device.set_brightness(*percent))
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .unwrap_or_else(|_| Err(io::Error::other("brightness worker panicked")))
                })
                .collect::<Vec<_>>()
        });

        for ((name, percent), result) in targets.into_iter().zip(results) {
            result?;
            self.brightness.insert(name, u32::from(percent));
        }

        Ok(())
    }
}

fn daemon_device(name: &str, device: &BrightnessDevice, brightness: u32) -> DaemonBrightnessDevice {
    match device {
        BrightnessDevice::Backlight(mapping) => DaemonBrightnessDevice {
            name: name.to_owned(),
            brightness,
            method: "backlight".to_owned(),
            backlight: Some(mapping.backlight.clone()),
            connector: Some(mapping.connector.clone()),
            mapping_method: Some(format!("{:?}", mapping.method).to_lowercase()),
            i2c_bus: None,
            device: None,
        },
        BrightnessDevice::DdcCi(mapping) => DaemonBrightnessDevice {
            name: name.to_owned(),
            brightness,
            method: "ddc/ci".to_owned(),
            backlight: None,
            connector: mapping.connector.clone(),
            mapping_method: None,
            i2c_bus: Some(mapping.i2c_bus.clone()),
            device: Some(mapping.device.clone()),
        },
    }
}

fn handle_client(mut stream: UnixStream, state: &mut DaemonState) -> io::Result<()> {
    let mut request = String::new();
    stream.read_to_string(&mut request)?;
    let response = handle_request(request.trim(), state);
    stream.write_all(response.as_bytes())?;
    Ok(())
}

fn handle_request(request: &str, state: &mut DaemonState) -> String {
    let mut parts = request.split_whitespace();
    match parts.next() {
        Some("LIST") => state
            .list()
            .into_iter()
            .map(|device| format_daemon_device(&device))
            .collect(),
        Some("GET") => match parts.next() {
            Some(name) => match state.get_brightness(name) {
                Ok(brightness) => format!("{brightness}\n"),
                Err(error) => format!("ERR {error}\n"),
            },
            None => "ERR missing device name\n".to_owned(),
        },
        Some("SET") => {
            let Some(name) = parts.next() else {
                return "ERR missing device name\n".to_owned();
            };
            let Some(brightness) = parts.next().and_then(|value| value.parse::<u8>().ok()) else {
                return "ERR missing brightness\n".to_owned();
            };
            if brightness > 100 {
                return "ERR brightness must be between 0 and 100\n".to_owned();
            }

            match state.set_brightness(name, brightness) {
                Ok(()) => "OK\n".to_owned(),
                Err(error) => format!("ERR {error}\n"),
            }
        }
        Some("SET_ALL") => {
            let mut targets = Vec::new();
            loop {
                let Some(name) = parts.next() else {
                    break;
                };
                let Some(brightness) = parts.next().and_then(|value| value.parse::<u8>().ok())
                else {
                    return "ERR missing brightness\n".to_owned();
                };
                if brightness > 100 {
                    return "ERR brightness must be between 0 and 100\n".to_owned();
                }
                targets.push((name.to_owned(), brightness));
            }

            match state.set_all_brightness(targets) {
                Ok(()) => "OK\n".to_owned(),
                Err(error) => format!("ERR {error}\n"),
            }
        }
        Some("REFRESH") => match state.refresh() {
            Ok(()) => "OK\n".to_owned(),
            Err(error) => format!("ERR {error}\n"),
        },
        Some(command) => format!("ERR unknown command {command}\n"),
        None => "ERR empty request\n".to_owned(),
    }
}

fn format_daemon_device(device: &DaemonBrightnessDevice) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        device.name,
        device.brightness,
        device.method,
        device.backlight.as_deref().unwrap_or(""),
        device.connector.as_deref().unwrap_or(""),
        device.mapping_method.as_deref().unwrap_or(""),
        device.i2c_bus.as_deref().unwrap_or(""),
        device
            .device
            .as_deref()
            .map(Path::display)
            .map(|path| path.to_string())
            .unwrap_or_default(),
    )
}

fn parse_list_response(response: &str) -> io::Result<Vec<DaemonBrightnessDevice>> {
    response
        .lines()
        .map(|line| {
            let mut fields = line.split('\t');
            let name = required_field(&mut fields, "device name")?;
            let brightness = required_field(&mut fields, "brightness")?
                .parse()
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            let method = required_field(&mut fields, "brightness method")?;
            let backlight = optional_field(&mut fields, "backlight")?;
            let connector = optional_field(&mut fields, "connector")?;
            let mapping_method = optional_field(&mut fields, "mapping method")?;
            let i2c_bus = optional_field(&mut fields, "i2c bus")?;
            let device = optional_field(&mut fields, "device")?.map(PathBuf::from);

            Ok(DaemonBrightnessDevice {
                name: name.to_owned(),
                brightness,
                method: method.to_owned(),
                backlight,
                connector,
                mapping_method,
                i2c_bus,
                device,
            })
        })
        .collect()
}

fn required_field<'a>(
    fields: &mut impl Iterator<Item = &'a str>,
    name: &str,
) -> io::Result<&'a str> {
    fields
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("missing {name}")))
}

fn optional_field<'a>(
    fields: &mut impl Iterator<Item = &'a str>,
    name: &str,
) -> io::Result<Option<String>> {
    Ok(match required_field(fields, name)? {
        "" => None,
        value => Some(value.to_owned()),
    })
}

fn send_daemon_request(request: &str) -> io::Result<String> {
    let mut stream = UnixStream::connect(socket_path()?)?;
    stream.write_all(request.as_bytes())?;
    stream.shutdown(std::net::Shutdown::Write)?;

    let mut response = String::new();
    stream.read_to_string(&mut response)?;

    if let Some(error) = response.trim().strip_prefix("ERR ") {
        return Err(io::Error::other(error.to_owned()));
    }

    Ok(response)
}

fn socket_path() -> io::Result<PathBuf> {
    if let Some(socket_path) = env::var_os("WAYDIM_SOCKET") {
        return Ok(PathBuf::from(socket_path));
    }

    let runtime_dir = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"))?;
    Ok(runtime_dir.join("waydim.sock"))
}

fn daemon_listener() -> io::Result<(UnixListener, PathBuf)> {
    let socket_path = socket_path()?;

    if systemd_listen_fds()? == 1 {
        // SAFETY: systemd socket activation passes the first inherited listener at fd 3.
        let listener = unsafe { UnixListener::from_raw_fd(3) };
        return Ok((listener, socket_path));
    }

    if socket_path.exists() {
        fs::remove_file(&socket_path)?;
    }

    UnixListener::bind(&socket_path).map(|listener| (listener, socket_path))
}

fn systemd_listen_fds() -> io::Result<u32> {
    let Some(listen_pid) = env::var_os("LISTEN_PID") else {
        return Ok(0);
    };
    if listen_pid.to_string_lossy().parse::<u32>().ok() != Some(process::id()) {
        return Ok(0);
    }

    let Some(listen_fds) = env::var_os("LISTEN_FDS") else {
        return Ok(0);
    };

    listen_fds
        .to_string_lossy()
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
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
