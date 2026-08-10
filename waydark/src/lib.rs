use std::{
    env,
    error::Error,
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    os::fd::{AsFd, FromRawFd},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    process, thread,
    time::Duration,
};

use tempfile::tempfile;
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_buffer::WlBuffer,
        wl_callback::WlCallback,
        wl_compositor::WlCompositor,
        wl_output::{self, WlOutput},
        wl_region::WlRegion,
        wl_registry::WlRegistry,
        wl_shm::{Format, WlShm},
        wl_shm_pool::WlShmPool,
        wl_surface::WlSurface,
    },
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1},
};

#[derive(Debug)]
pub struct Output {
    pub name: String,
    pub label: String,
}

#[derive(Debug)]
pub struct OutputBrightness {
    pub name: String,
    pub brightness: u8,
    pub label: String,
}

pub fn list_outputs() -> Result<Vec<Output>, Box<dyn Error>> {
    let connection = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init::<State>(&connection)?;
    let qh = event_queue.handle();
    let shm = globals.bind::<WlShm, _, _>(&qh, 1..=1, ())?;

    let outputs = bind_outputs(&globals, &qh);
    let mut state = State {
        outputs,
        overlays: Vec::new(),
        shm,
    };
    event_queue.roundtrip(&mut state)?;

    Ok(state
        .outputs
        .iter()
        .enumerate()
        .map(|(index, output)| Output {
            name: output.name(index),
            label: output.label(index),
        })
        .collect())
}

pub fn run_daemon() -> Result<(), Box<dyn Error>> {
    let connection = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init::<State>(&connection)?;
    let qh = event_queue.handle();

    let compositor = globals.bind::<WlCompositor, _, _>(&qh, 4..=5, ())?;
    let shm = globals.bind::<WlShm, _, _>(&qh, 1..=1, ())?;
    let layer_shell = globals.bind::<ZwlrLayerShellV1, _, _>(&qh, 1..=4, ())?;

    let outputs = bind_outputs(&globals, &qh);
    let mut state = State {
        outputs,
        overlays: Vec::new(),
        shm,
    };
    event_queue.roundtrip(&mut state)?;

    if state.outputs.is_empty() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "no Wayland outputs found").into());
    }

    for index in 0..state.outputs.len() {
        let surface = compositor.create_surface(&qh, ());
        let empty_input_region = compositor.create_region(&qh, ());
        surface.set_input_region(Some(&empty_input_region));
        empty_input_region.destroy();

        let output = &state.outputs[index].wl_output;
        let layer_surface = layer_shell.get_layer_surface(
            &surface,
            Some(output),
            Layer::Overlay,
            "waydark".to_owned(),
            &qh,
            LayerSurfaceData {
                output_index: index,
            },
        );

        layer_surface.set_anchor(Anchor::Top | Anchor::Right | Anchor::Bottom | Anchor::Left);
        layer_surface.set_exclusive_zone(-1);
        layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);
        surface.commit();

        state.overlays.push(Overlay {
            surface,
            _layer_surface: layer_surface,
            buffer: None,
            file: None,
            width: 0,
            height: 0,
        });
    }

    let (listener, socket_path) = daemon_listener()?;
    listener.set_nonblocking(true)?;
    println!("waydark daemon listening on {}", socket_path.display());

    loop {
        event_queue.dispatch_pending(&mut state)?;
        if let Some(guard) = event_queue.prepare_read() {
            match guard.read() {
                Ok(_) => {}
                Err(wayland_client::backend::WaylandError::Io(error))
                    if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.into()),
            }
        }
        event_queue.dispatch_pending(&mut state)?;

        loop {
            match listener.accept() {
                Ok((stream, _)) => handle_client(stream, &mut state, &qh)?,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error.into()),
            }
        }

        event_queue.flush()?;
        thread::sleep(Duration::from_millis(20));
    }
}

pub fn daemon_list_outputs() -> io::Result<Vec<OutputBrightness>> {
    parse_output_brightness_response(&send_daemon_request("LIST")?)
}

pub fn daemon_get_brightness(name: &str) -> io::Result<u8> {
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

fn bind_outputs(
    globals: &wayland_client::globals::GlobalList,
    qh: &QueueHandle<State>,
) -> Vec<OutputInfo> {
    globals
        .contents()
        .clone_list()
        .into_iter()
        .filter(|global| global.interface == "wl_output")
        .map(|global| OutputInfo {
            wl_output: globals
                .registry()
                .bind(global.name, global.version.min(4), qh, ()),
            name: None,
            description: None,
            make: None,
            model: None,
            mode: None,
            scale: 1,
            brightness: 100,
        })
        .collect()
}

fn handle_client(
    mut stream: UnixStream,
    state: &mut State,
    qh: &QueueHandle<State>,
) -> io::Result<()> {
    let mut request = String::new();
    stream.read_to_string(&mut request)?;
    let response = handle_request(request.trim(), state, qh);
    stream.write_all(response.as_bytes())?;
    Ok(())
}

fn handle_request(request: &str, state: &mut State, qh: &QueueHandle<State>) -> String {
    let mut parts = request.split_whitespace();
    match parts.next() {
        Some("LIST") => state
            .outputs
            .iter()
            .enumerate()
            .map(|(index, output)| {
                format!(
                    "{}\t{}\t{}\n",
                    output.name(index),
                    output.brightness,
                    output.label(index)
                )
            })
            .collect(),
        Some("GET") => match parts.next() {
            Some(name) => match state.output_index(name) {
                Some(index) => format!("{}\n", state.outputs[index].brightness),
                None => format!("ERR no output named {name}\n"),
            },
            None => "ERR missing output name\n".to_owned(),
        },
        Some("SET") => {
            let Some(name) = parts.next() else {
                return "ERR missing output name\n".to_owned();
            };
            let Some(brightness) = parts.next().and_then(|value| value.parse::<u8>().ok()) else {
                return "ERR missing brightness\n".to_owned();
            };
            if brightness > 100 {
                return "ERR brightness must be between 0 and 100\n".to_owned();
            }

            match state.set_brightness(name, brightness, qh) {
                Ok(()) => "OK\n".to_owned(),
                Err(error) => format!("ERR {error}\n"),
            }
        }
        Some(command) => format!("ERR unknown command {command}\n"),
        None => "ERR empty request\n".to_owned(),
    }
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

fn parse_output_brightness_response(response: &str) -> io::Result<Vec<OutputBrightness>> {
    response
        .lines()
        .map(|line| {
            let mut fields = line.splitn(3, '\t');
            let name = fields
                .next()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing output name"))?;
            let brightness = fields
                .next()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing brightness"))?
                .parse()
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            let label = fields.next().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "missing output label")
            })?;

            Ok(OutputBrightness {
                name: name.to_owned(),
                brightness,
                label: label.to_owned(),
            })
        })
        .collect()
}

fn socket_path() -> io::Result<PathBuf> {
    if let Some(socket_path) = env::var_os("WAYDARK_SOCKET") {
        return Ok(PathBuf::from(socket_path));
    }

    let runtime_dir = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"))?;
    Ok(runtime_dir.join("waydark.sock"))
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

struct State {
    outputs: Vec<OutputInfo>,
    overlays: Vec<Overlay>,
    shm: WlShm,
}

impl State {
    fn output_index(&self, name: &str) -> Option<usize> {
        self.outputs
            .iter()
            .enumerate()
            .find(|(index, output)| output.name(*index) == name)
            .map(|(index, _)| index)
    }

    fn set_brightness(
        &mut self,
        name: &str,
        brightness: u8,
        qh: &QueueHandle<State>,
    ) -> io::Result<()> {
        if name == "@all" {
            for index in 0..self.outputs.len() {
                self.set_brightness_by_index(index, brightness, qh)?;
            }
            return Ok(());
        }

        let Some(index) = self.output_index(name) else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no output named {name}"),
            ));
        };

        self.set_brightness_by_index(index, brightness, qh)
    }

    fn set_brightness_by_index(
        &mut self,
        index: usize,
        brightness: u8,
        qh: &QueueHandle<State>,
    ) -> io::Result<()> {
        self.outputs[index].brightness = brightness;
        let overlay = &mut self.overlays[index];

        if overlay.width > 0 && overlay.height > 0 {
            let (buffer, file) = draw_dim_buffer(
                &self.shm,
                &overlay.surface,
                qh,
                overlay.width,
                overlay.height,
                brightness,
            )
            .map_err(|error| io::Error::other(error.to_string()))?;
            overlay.buffer = Some(buffer);
            overlay.file = Some(file);
        }

        Ok(())
    }
}

struct OutputInfo {
    wl_output: WlOutput,
    name: Option<String>,
    description: Option<String>,
    make: Option<String>,
    model: Option<String>,
    mode: Option<(i32, i32)>,
    scale: i32,
    brightness: u8,
}

impl OutputInfo {
    fn name(&self, index: usize) -> String {
        self.name
            .clone()
            .or_else(|| self.model.clone())
            .unwrap_or_else(|| format!("output-{index}"))
    }

    fn label(&self, index: usize) -> String {
        let name = self.name(index);
        let description = self
            .description
            .as_deref()
            .or(self.model.as_deref())
            .unwrap_or("unknown output");

        match self.mode {
            Some((width, height)) => format!(
                "{name}: {description} ({width}x{height}, scale {})",
                self.scale
            ),
            None => format!("{name}: {description} (scale {})", self.scale),
        }
    }
}

struct Overlay {
    surface: WlSurface,
    _layer_surface: ZwlrLayerSurfaceV1,
    buffer: Option<WlBuffer>,
    file: Option<File>,
    width: u32,
    height: u32,
}

struct LayerSurfaceData {
    output_index: usize,
}

fn draw_dim_buffer(
    shm: &WlShm,
    surface: &WlSurface,
    qh: &QueueHandle<State>,
    width: u32,
    height: u32,
    brightness: u8,
) -> Result<(WlBuffer, File), Box<dyn Error>> {
    let stride = width.checked_mul(4).ok_or("buffer stride overflow")?;
    let size = stride.checked_mul(height).ok_or("buffer size overflow")?;

    let mut file = tempfile()?;
    file.set_len(u64::from(size))?;
    file.seek(SeekFrom::Start(0))?;

    let alpha = 255 - ((u32::from(brightness) * 255 + 50) / 100);
    let pixel = (alpha << 24).to_ne_bytes();
    let mut row = Vec::with_capacity(stride as usize);
    for _ in 0..width {
        row.extend_from_slice(&pixel);
    }
    for _ in 0..height {
        file.write_all(&row)?;
    }
    file.flush()?;

    let pool = shm.create_pool(file.as_fd(), size as i32, qh, ());
    let buffer = pool.create_buffer(
        0,
        width as i32,
        height as i32,
        stride as i32,
        Format::Argb8888,
        qh,
        (),
    );
    pool.destroy();

    surface.attach(Some(&buffer), 0, 0);
    surface.damage_buffer(0, 0, width as i32, height as i32);
    surface.commit();

    Ok((buffer, file))
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlCompositor, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlCompositor,
        _: wayland_client::protocol::wl_compositor::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlShm, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlShm,
        _: wayland_client::protocol::wl_shm::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlShmPool, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlShmPool,
        _: wayland_client::protocol::wl_shm_pool::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlBuffer, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlBuffer,
        _: wayland_client::protocol::wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSurface, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlSurface,
        _: wayland_client::protocol::wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlRegion, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlRegion,
        _: wayland_client::protocol::wl_region::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlCallback, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlCallback,
        _: wayland_client::protocol::wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        output: &WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(info) = state
            .outputs
            .iter_mut()
            .find(|info| &info.wl_output == output)
        else {
            return;
        };

        match event {
            wl_output::Event::Geometry { make, model, .. } => {
                info.make = Some(make);
                info.model = Some(model);
            }
            wl_output::Event::Mode { width, height, .. } => {
                info.mode = Some((width, height));
            }
            wl_output::Event::Scale { factor } => {
                info.scale = factor;
            }
            wl_output::Event::Name { name } => {
                info.name = Some(name);
            }
            wl_output::Event::Description { description } => {
                info.description = Some(description);
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrLayerShellV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrLayerShellV1,
        _: wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, LayerSurfaceData> for State {
    fn event(
        state: &mut Self,
        layer_surface: &ZwlrLayerSurfaceV1,
        event: wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Event,
        data: &LayerSurfaceData,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                layer_surface.ack_configure(serial);

                if width == 0 || height == 0 {
                    return;
                }

                let shm = state.shm.clone();
                let brightness = state.outputs[data.output_index].brightness;
                let Some(overlay) = state.overlays.get_mut(data.output_index) else {
                    return;
                };

                overlay.width = width;
                overlay.height = height;

                match draw_dim_buffer(&shm, &overlay.surface, qh, width, height, brightness) {
                    Ok((buffer, file)) => {
                        overlay.buffer = Some(buffer);
                        overlay.file = Some(file);
                    }
                    Err(error) => eprintln!("failed to draw overlay: {error}"),
                }
            }
            wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Event::Closed => {
                std::process::exit(0);
            }
            _ => {}
        }
    }
}
