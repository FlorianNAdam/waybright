use std::{
    error::Error,
    fs::File,
    io::{self, Seek, SeekFrom, Write},
    os::fd::AsFd,
};

use tempfile::tempfile;
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle,
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

#[derive(Clone, Debug)]
pub struct OutputBrightness {
    pub name: String,
    pub brightness: u8,
    pub description: String,
    pub mode: Option<(i32, i32)>,
    pub scale: i32,
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
        closed: false,
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

pub struct OverlayManager {
    event_queue: EventQueue<State>,
    qh: QueueHandle<State>,
    state: State,
}

impl OverlayManager {
    pub fn connect() -> Result<Self, Box<dyn Error>> {
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
            closed: false,
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

        Ok(Self {
            event_queue,
            qh,
            state,
        })
    }

    pub fn dispatch(&mut self) -> Result<(), Box<dyn Error>> {
        self.event_queue.dispatch_pending(&mut self.state)?;
        if let Some(guard) = self.event_queue.prepare_read() {
            match guard.read() {
                Ok(_) => {}
                Err(wayland_client::backend::WaylandError::Io(error))
                    if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.into()),
            }
        }
        self.event_queue.dispatch_pending(&mut self.state)?;
        self.event_queue.flush()?;
        if self.state.closed {
            return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "overlay closed").into());
        }
        Ok(())
    }

    pub fn outputs(&self) -> Vec<OutputBrightness> {
        self.state
            .outputs
            .iter()
            .enumerate()
            .map(|(index, output)| OutputBrightness {
                name: output.name(index),
                brightness: output.brightness,
                description: output.description(index),
                mode: output.mode,
                scale: output.scale,
            })
            .collect()
    }

    pub fn brightness(&self, name: &str) -> Option<u8> {
        self.state
            .output_index(name)
            .map(|index| self.state.outputs[index].brightness)
    }

    pub fn set_brightness(&mut self, name: &str, brightness: u8) -> io::Result<()> {
        if brightness > 100 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "brightness must be between 0 and 100",
            ));
        }
        if name != "@all" && self.brightness(name) == Some(brightness) {
            return Ok(());
        }
        self.state.set_brightness(name, brightness, &self.qh)
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

struct State {
    outputs: Vec<OutputInfo>,
    overlays: Vec<Overlay>,
    shm: WlShm,
    closed: bool,
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
        self.outputs[index].brightness = brightness;

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
        let description = self.description(index);

        match self.mode {
            Some((width, height)) => format!(
                "{name}: {description} ({width}x{height}, scale {})",
                self.scale
            ),
            None => format!("{name}: {description} (scale {})", self.scale),
        }
    }

    fn description(&self, index: usize) -> String {
        let name = self.name(index);
        let description = self
            .description
            .as_deref()
            .or(self.model.as_deref())
            .unwrap_or("unknown output");
        let description = description
            .strip_prefix(&format!("{name}: "))
            .unwrap_or(description);
        description
            .strip_suffix(&format!(" ({name})"))
            .unwrap_or(description)
            .to_owned()
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
                state.closed = true;
            }
            _ => {}
        }
    }
}
