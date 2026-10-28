use std::{collections::VecDeque, ffi::c_void, ptr, time::Instant};

use eframe::egui;
use waybright::client::{self, OutputState};

fn main() -> eframe::Result {
    link_runtime_libraries();
    eframe::run_native(
        "Waybright latency tester",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([520.0, 230.0]),
            ..Default::default()
        },
        Box::new(|_| Ok(Box::new(BrightnessApp::new()))),
    )
}

#[link(name = "wayland-client")]
unsafe extern "C" {
    fn wl_display_connect(name: *const i8) -> *mut c_void;
    fn wl_display_disconnect(display: *mut c_void);
}

#[link(name = "xkbcommon")]
unsafe extern "C" {
    fn xkb_context_new(flags: u32) -> *mut c_void;
    fn xkb_context_unref(context: *mut c_void);
}

fn link_runtime_libraries() {
    // Direct references give NixOS binaries a RUNPATH for libraries winit loads dynamically.
    let display = unsafe { wl_display_connect(ptr::null()) };
    if !display.is_null() {
        unsafe { wl_display_disconnect(display) };
    }

    let xkb_context = unsafe { xkb_context_new(0) };
    if !xkb_context.is_null() {
        unsafe { xkb_context_unref(xkb_context) };
    }
}

struct BrightnessApp {
    outputs: Vec<OutputState>,
    selected: usize,
    brightness: u8,
    samples_ms: VecDeque<f64>,
    request_count: u64,
    status: Result<String, String>,
}

impl BrightnessApp {
    fn new() -> Self {
        let mut app = Self {
            outputs: Vec::new(),
            selected: 0,
            brightness: 50,
            samples_ms: VecDeque::with_capacity(120),
            request_count: 0,
            status: Ok("Ready".to_owned()),
        };
        app.refresh_outputs();
        app
    }

    fn refresh_outputs(&mut self) {
        let selected_name = self.selected_output().map(str::to_owned);
        match client::list() {
            Ok(outputs) if !outputs.is_empty() => {
                self.selected = selected_name
                    .and_then(|name| outputs.iter().position(|output| output.name == name))
                    .unwrap_or(0);
                self.brightness = outputs[self.selected].brightness;
                self.outputs = outputs;
                self.status = Ok("Outputs refreshed".to_owned());
            }
            Ok(_) => {
                self.outputs.clear();
                self.status = Err("The daemon reported no outputs".to_owned());
            }
            Err(error) => self.status = Err(error.to_string()),
        }
    }

    fn selected_output(&self) -> Option<&str> {
        self.outputs
            .get(self.selected)
            .map(|output| output.name.as_str())
    }

    fn apply_brightness(&mut self) {
        let Some(output) = self.selected_output().map(str::to_owned) else {
            return;
        };
        let started = Instant::now();
        let result = client::set(&output, &format!("{}%", self.brightness));
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

        if self.samples_ms.len() == 120 {
            self.samples_ms.pop_front();
        }
        self.samples_ms.push_back(elapsed_ms);
        self.request_count += 1;
        self.status = result
            .map(|()| format!("Applied {}% to {output}", self.brightness))
            .map_err(|error| error.to_string());
    }

    fn latency(&self) -> Option<(f64, f64, f64)> {
        let last = *self.samples_ms.back()?;
        let average = self.samples_ms.iter().sum::<f64>() / self.samples_ms.len() as f64;
        let max = self.samples_ms.iter().copied().fold(0.0, f64::max);
        Some((last, average, max))
    }
}

impl eframe::App for BrightnessApp {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        ui.heading("Waybright latency tester");
        ui.label("Every slider change waits for the daemon and display update.");
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.label("Output");
            let previous = self.selected;
            egui::ComboBox::from_id_salt("output")
                .selected_text(self.selected_output().unwrap_or("No outputs"))
                .show_ui(ui, |ui| {
                    for (index, output) in self.outputs.iter().enumerate() {
                        ui.selectable_value(&mut self.selected, index, &output.name);
                    }
                });
            if self.selected != previous {
                self.brightness = self.outputs[self.selected].brightness;
                self.samples_ms.clear();
            }
            if ui.button("Refresh").clicked() {
                self.refresh_outputs();
            }
        });

        ui.add_space(8.0);
        let slider = ui.add_enabled(
            !self.outputs.is_empty(),
            egui::Slider::new(&mut self.brightness, 0..=100)
                .text("Brightness")
                .suffix("%")
                .show_value(true),
        );
        if slider.changed() {
            self.apply_brightness();
        }

        ui.add_space(8.0);
        if let Some((last, average, max)) = self.latency() {
            ui.monospace(format!(
                "last {last:6.1} ms    avg {average:6.1} ms    max {max:6.1} ms    requests {}",
                self.request_count
            ));
        } else {
            ui.monospace("Drag the slider to collect latency samples");
        }

        match &self.status {
            Ok(status) => ui.label(egui::RichText::new(status).color(egui::Color32::LIGHT_GREEN)),
            Err(error) => ui.label(egui::RichText::new(error).color(egui::Color32::LIGHT_RED)),
        };
    }
}
