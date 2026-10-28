use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    error::Error,
    fs,
    io::{self, Read, Write},
    os::fd::FromRawFd,
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    process, thread,
    time::{Duration, Instant},
};

use waydark::OverlayManager;
use waydim::{BrightnessControl, BrightnessDevice, brightness_devices};

use crate::{
    brightness::{
        PercentChange, apply_percent_change, effective_brightness, parse_percent_change,
        split_brightness, validate_hardware_min,
    },
    client,
    protocol::{Request, parse_request},
};

pub struct Config {
    pub hardware_min: u8,
}

struct DaemonState {
    hardware: BTreeMap<String, BrightnessDevice>,
    hardware_brightness: BTreeMap<String, u32>,
    overlays: Option<OverlayManager>,
    last_overlay_attempt: Instant,
    hardware_min: u8,
}

struct OutputState {
    name: String,
    brightness: u8,
    hardware: Option<u32>,
    software: Option<u8>,
}

pub fn run(config: Config) -> Result<(), Box<dyn Error>> {
    validate_hardware_min(config.hardware_min)?;
    let mut state = DaemonState::new(config.hardware_min)?;
    let (listener, socket_path) = daemon_listener()?;
    listener.set_nonblocking(true)?;
    println!("waybright daemon listening on {}", socket_path.display());

    loop {
        let overlay_failed = match state.overlays.as_mut() {
            Some(overlays) => match overlays.dispatch() {
                Ok(()) => false,
                Err(error) => {
                    eprintln!("waybright: software dimming disabled: {error}");
                    state.last_overlay_attempt = Instant::now();
                    true
                }
            },
            None => false,
        };
        if overlay_failed {
            state.overlays = None;
        }
        state.reconnect_overlay_if_due();

        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    if let Err(error) = handle_client(stream, &mut state) {
                        eprintln!("waybright: client request failed: {error}");
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error.into()),
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
}

impl DaemonState {
    fn new(hardware_min: u8) -> io::Result<Self> {
        let (hardware, hardware_brightness) = match discover_hardware() {
            Ok(hardware) => hardware,
            Err(error) => {
                eprintln!("waybright: hardware brightness unavailable: {error}");
                (BTreeMap::new(), BTreeMap::new())
            }
        };
        let overlays = match OverlayManager::connect() {
            Ok(overlays) => Some(overlays),
            Err(error) => {
                eprintln!("waybright: software dimming unavailable: {error}");
                None
            }
        };

        if hardware_brightness.is_empty() && overlays.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no hardware brightness devices or Wayland outputs found",
            ));
        }

        Ok(Self {
            hardware,
            hardware_brightness,
            overlays,
            last_overlay_attempt: Instant::now(),
            hardware_min,
        })
    }

    fn outputs(&self) -> io::Result<Vec<OutputState>> {
        let mut names = self
            .hardware_brightness
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        if let Some(overlays) = &self.overlays {
            names.extend(overlays.outputs().into_iter().map(|output| output.name));
        }

        names
            .into_iter()
            .map(|name| {
                let hardware = self.hardware_brightness.get(&name).copied();
                let software = self
                    .overlays
                    .as_ref()
                    .and_then(|overlays| overlays.brightness(&name));
                let brightness = combined_brightness(&name, hardware, software, self.hardware_min)?;
                Ok(OutputState {
                    name,
                    brightness,
                    hardware,
                    software,
                })
            })
            .collect()
    }

    fn get(&self, selector: &str) -> io::Result<u8> {
        let name = resolve_selector(selector)?;
        if name == "@all" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "@all is not valid for get",
            ));
        }
        let hardware = self.hardware_brightness.get(&name).copied();
        let software = self
            .overlays
            .as_ref()
            .and_then(|overlays| overlays.brightness(&name));
        combined_brightness(&name, hardware, software, self.hardware_min)
    }

    fn set(&mut self, selector: &str, change: PercentChange) -> io::Result<()> {
        if selector == "@all" {
            let names = self
                .outputs()?
                .into_iter()
                .map(|output| output.name)
                .collect::<Vec<_>>();
            for name in names {
                self.set_one(&name, change)?;
            }
            return Ok(());
        }

        self.set_one(&resolve_selector(selector)?, change)
    }

    fn set_one(&mut self, name: &str, change: PercentChange) -> io::Result<()> {
        let hardware = self.hardware_brightness.get(name).copied();
        let software = self
            .overlays
            .as_ref()
            .and_then(|overlays| overlays.brightness(name));
        let current = combined_brightness(name, hardware, software, self.hardware_min)?;
        let target = apply_percent_change(current, change);

        match (hardware, software) {
            (Some(_), Some(_)) => {
                let (hardware, software) = split_brightness(target, self.hardware_min);
                self.set_hardware(name, hardware)?;
                self.set_software(name, software)?;
            }
            (Some(_), None) => self.set_hardware(name, target)?,
            (None, Some(_)) => self.set_software(name, target)?,
            (None, None) => unreachable!(),
        }
        Ok(())
    }

    fn set_hardware(&mut self, name: &str, brightness: u8) -> io::Result<()> {
        let device = self.hardware.get(name).ok_or_else(|| not_found(name))?;
        device.set_brightness(brightness)?;
        self.hardware_brightness
            .insert(name.to_owned(), u32::from(brightness));
        Ok(())
    }

    fn set_software(&mut self, name: &str, brightness: u8) -> io::Result<()> {
        self.overlays
            .as_mut()
            .ok_or_else(|| not_found(name))?
            .set_brightness(name, brightness)
    }

    fn refresh(&mut self) -> io::Result<()> {
        let (hardware, hardware_brightness) = discover_hardware()?;
        self.hardware = hardware;
        self.hardware_brightness = hardware_brightness;
        if self.overlays.is_none() {
            self.reconnect_overlay();
        }
        Ok(())
    }

    fn reconnect_overlay_if_due(&mut self) {
        if self.overlays.is_none() && self.last_overlay_attempt.elapsed() >= Duration::from_secs(5)
        {
            self.reconnect_overlay();
        }
    }

    fn reconnect_overlay(&mut self) {
        self.last_overlay_attempt = Instant::now();
        match OverlayManager::connect() {
            Ok(overlays) => {
                eprintln!("waybright: software dimming connected");
                self.overlays = Some(overlays);
            }
            Err(error) => eprintln!("waybright: software dimming unavailable: {error}"),
        }
    }
}

fn discover_hardware() -> io::Result<(BTreeMap<String, BrightnessDevice>, BTreeMap<String, u32>)> {
    let hardware = brightness_devices()?;
    let hardware_brightness = hardware
        .iter()
        .filter_map(|(name, device)| {
            device
                .get_brightness()
                .map(|brightness| (name.clone(), brightness))
                .ok()
        })
        .collect();
    Ok((hardware, hardware_brightness))
}

fn combined_brightness(
    name: &str,
    hardware: Option<u32>,
    software: Option<u8>,
    hardware_min: u8,
) -> io::Result<u8> {
    match (hardware, software) {
        (Some(hardware), Some(software)) => {
            Ok(effective_brightness(hardware, software, hardware_min))
        }
        (Some(hardware), None) => Ok(hardware.clamp(0, 100) as u8),
        (None, Some(software)) => Ok(software),
        (None, None) => Err(not_found(name)),
    }
}

fn resolve_selector(selector: &str) -> io::Result<String> {
    if selector == "@focused" {
        wayfocus::focused_output()
    } else {
        Ok(selector.to_owned())
    }
}

fn handle_client(mut stream: UnixStream, state: &mut DaemonState) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    let mut request = String::new();
    stream.read_to_string(&mut request)?;
    let response = match handle_request(request.trim(), state) {
        Ok(response) => response,
        Err(error) => format!("ERR {}\n", error.to_string().replace('\n', " ")),
    };
    stream.write_all(response.as_bytes())
}

fn handle_request(request: &str, state: &mut DaemonState) -> io::Result<String> {
    match parse_request(request)? {
        Request::List => Ok(state
            .outputs()?
            .into_iter()
            .map(|output| {
                format!(
                    "{}\t{}\t{}\t{}\n",
                    output.name,
                    output.brightness,
                    optional(output.hardware),
                    optional(output.software),
                )
            })
            .collect()),
        Request::Get { selector } => Ok(format!("{}\n", state.get(&selector)?)),
        Request::Set { selector, change } => {
            state.set(&selector, parse_percent_change(&change)?)?;
            Ok("OK\n".to_owned())
        }
        Request::Refresh => {
            state.refresh()?;
            Ok("OK\n".to_owned())
        }
    }
}

fn optional(value: Option<impl ToString>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

fn not_found(name: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("no brightness device named {name}"),
    )
}

fn daemon_listener() -> io::Result<(UnixListener, PathBuf)> {
    let socket_path = client::socket_path()?;
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
