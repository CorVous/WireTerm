//! Visible foreground `WireTerm` editor/sender.

#![allow(clippy::cast_precision_loss)]

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::Duration,
};

use eframe::egui;

use crate::{
    frame::{FRAME_HEIGHT, FRAME_WIDTH, PanelFrame},
    host::{HostBridge, HostEvent},
    raster::{RasterError, prepare_raster_path},
    transport::{DeviceInfo, TransferStage},
};

pub fn run() -> eframe::Result<()> {
    eframe::run_native(
        "WireTerm",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([820.0, 640.0])
                .with_min_inner_size([520.0, 560.0]),
            ..Default::default()
        },
        Box::new(|_| Ok(Box::new(WireTermApp::new()))),
    )
}

struct PreparedImage {
    path: PathBuf,
    frame: Arc<PanelFrame>,
    texture: egui::TextureHandle,
}

type PreparationResult = (PathBuf, Result<PanelFrame, RasterError>);

struct WireTermApp {
    host: HostBridge,
    devices: Vec<DeviceInfo>,
    selected_port: Option<String>,
    prepared: Option<PreparedImage>,
    preparation: Option<Receiver<PreparationResult>>,
    status: String,
    status_is_error: bool,
    transfer_progress: f32,
}

impl WireTermApp {
    fn new() -> Self {
        Self {
            host: HostBridge::new(),
            devices: Vec::new(),
            selected_port: None,
            prepared: None,
            preparation: None,
            status: "Looking for a display bridge…".to_owned(),
            status_is_error: false,
            transfer_progress: 0.0,
        }
    }

    fn choose_image(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Images", &["png", "jpg", "jpeg"])
            .pick_file()
        else {
            return;
        };

        let (sender, receiver) = mpsc::channel();
        self.preparation = Some(receiver);
        "Preparing image…".clone_into(&mut self.status);
        self.status_is_error = false;
        thread::Builder::new()
            .name("wireterm-image-preparation".to_owned())
            .spawn(move || {
                let result = prepare_raster_path(&path);
                let _ = sender.send((path, result));
            })
            .expect("image preparation worker should start");
    }

    fn poll_preparation(&mut self, ctx: &egui::Context) {
        let Some(receiver) = self.preparation.take() else {
            return;
        };
        match receiver.try_recv() {
            Ok((path, Ok(frame))) => {
                let image =
                    egui::ColorImage::from_rgb([FRAME_WIDTH, FRAME_HEIGHT], frame.preview_rgb());
                let texture =
                    ctx.load_texture("prepared-frame", image, egui::TextureOptions::NEAREST);
                self.prepared = Some(PreparedImage {
                    path,
                    frame: Arc::new(frame),
                    texture,
                });
                "Frame ready · 800 × 480 · B/W/R".clone_into(&mut self.status);
                self.status_is_error = false;
            }
            Ok((_, Err(error))) => {
                self.prepared = None;
                self.status = format!("Image error: {error}");
                self.status_is_error = true;
            }
            Err(TryRecvError::Empty) => self.preparation = Some(receiver),
            Err(TryRecvError::Disconnected) => {
                "Image preparation stopped unexpectedly".clone_into(&mut self.status);
                self.status_is_error = true;
            }
        }
    }

    fn poll_host(&mut self) {
        while let Some(event) = self.host.try_recv() {
            match event {
                HostEvent::DevicesChanged(devices) => {
                    self.update_devices(devices);
                }
                HostEvent::DeviceDiscoveryFailed(error) => {
                    self.status = format!("Device scan failed: {error}");
                    self.status_is_error = true;
                }
                HostEvent::TransferProgress(stage) => {
                    self.transfer_progress = stage.progress();
                    self.status = transfer_status(&stage);
                    self.status_is_error = false;
                    if stage == TransferStage::Complete {
                        let _ = self.host.refresh_devices();
                    }
                }
                HostEvent::TransferFailed(error) => {
                    self.status = format!("Transfer failed: {error}");
                    self.status_is_error = true;
                }
            }
        }
    }

    fn update_devices(&mut self, devices: Vec<DeviceInfo>) {
        let selected_still_present = self
            .selected_port
            .as_ref()
            .is_some_and(|selected| devices.iter().any(|device| &device.port_name == selected));
        if !selected_still_present {
            self.selected_port = devices.first().map(|device| device.port_name.clone());
        }
        self.devices = devices;

        if self.host.is_transfer_active() {
            return;
        }
        if let Some(port) = &self.selected_port {
            self.status = format!("{port} · display bridge available");
            self.status_is_error = false;
        } else {
            "No serial display bridge found".clone_into(&mut self.status);
            self.status_is_error = true;
        }
    }

    fn start_transfer(&mut self) {
        let (Some(port_name), Some(prepared)) = (&self.selected_port, &self.prepared) else {
            return;
        };
        match self
            .host
            .send_frame(port_name.clone(), Arc::clone(&prepared.frame))
        {
            Ok(()) => {
                self.transfer_progress = 0.0;
                "Connecting…".clone_into(&mut self.status);
                self.status_is_error = false;
            }
            Err(error) => {
                self.status = format!("Could not start transfer: {error}");
                self.status_is_error = true;
            }
        }
    }

    fn device_picker(&mut self, ui: &mut egui::Ui) {
        let selected_text = self.selected_port.as_deref().unwrap_or("No device");
        egui::ComboBox::from_id_salt("wireterm-device")
            .selected_text(selected_text)
            .show_ui(ui, |ui| {
                for device in &self.devices {
                    let suffix = if device.is_wireterm_candidate {
                        " · likely WireTerm"
                    } else {
                        ""
                    };
                    ui.selectable_value(
                        &mut self.selected_port,
                        Some(device.port_name.clone()),
                        format!("{} · {}{suffix}", device.port_name, device.description),
                    );
                }
            });
        if ui.button("Refresh devices").clicked() {
            match self.host.refresh_devices() {
                Ok(()) => {
                    "Looking for a display bridge…".clone_into(&mut self.status);
                    self.status_is_error = false;
                }
                Err(error) => {
                    self.status = format!("Could not refresh devices: {error}");
                    self.status_is_error = true;
                }
            }
        }
    }
}

impl eframe::App for WireTermApp {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll_preparation(ctx);
        self.poll_host();
        if self.preparation.is_some() || self.host.is_transfer_active() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        ctx.set_visuals(egui::Visuals::dark());
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_max_width(760.0);
                    ui.add_space(18.0);

                    ui.horizontal(|ui| {
                        ui.heading("WireTerm");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let color = if self.status_is_error {
                                egui::Color32::LIGHT_RED
                            } else {
                                egui::Color32::LIGHT_GREEN
                            };
                            ui.label(egui::RichText::new(&self.status).color(color));
                        });
                    });
                    ui.add_space(18.0);

                    ui.label("E-paper preview · 800 × 480");
                    ui.add_space(6.0);
                    let available_width = ui.available_width();
                    let preview_width = available_width.min(720.0);
                    let preview_height = preview_width * FRAME_HEIGHT as f32 / FRAME_WIDTH as f32;
                    let (row_rect, _) = ui.allocate_exact_size(
                        egui::vec2(available_width, preview_height),
                        egui::Sense::hover(),
                    );
                    let preview_rect = egui::Rect::from_center_size(
                        row_rect.center(),
                        egui::vec2(preview_width, preview_height),
                    );
                    ui.painter().rect_filled(
                        preview_rect,
                        0.0,
                        egui::Color32::from_rgb(238, 237, 229),
                    );
                    if let Some(prepared) = &self.prepared {
                        ui.put(
                            preview_rect,
                            egui::Image::new(&prepared.texture)
                                .fit_to_exact_size(preview_rect.size()),
                        );
                    } else {
                        ui.painter().text(
                            preview_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "800 × 480\nBLACK · WHITE · RED",
                            egui::FontId::proportional(24.0),
                            egui::Color32::BLACK,
                        );
                    }

                    ui.add_space(16.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(
                                self.preparation.is_none(),
                                egui::Button::new("Choose image"),
                            )
                            .clicked()
                        {
                            self.choose_image();
                        }
                        let filename = self
                            .prepared
                            .as_ref()
                            .map(|prepared| prepared.path.as_path())
                            .and_then(Path::file_name)
                            .and_then(|name| name.to_str())
                            .unwrap_or("No image selected");
                        ui.label(filename);
                    });

                    ui.add_space(10.0);
                    ui.horizontal_wrapped(|ui| self.device_picker(ui));
                    ui.add_space(10.0);

                    let can_send = self.prepared.is_some()
                        && self.selected_port.is_some()
                        && !self.host.is_transfer_active();
                    if ui
                        .add_enabled(can_send, egui::Button::new("Send frame"))
                        .clicked()
                    {
                        self.start_transfer();
                    }
                    if self.host.is_transfer_active() {
                        ui.add_space(8.0);
                        ui.add(
                            egui::ProgressBar::new(self.transfer_progress)
                                .show_percentage()
                                .animate(true),
                        );
                    }
                    ui.add_space(18.0);
                });
        });
    }
}

fn transfer_status(stage: &TransferStage) -> String {
    match stage {
        TransferStage::Connecting => "Connecting…".to_owned(),
        TransferStage::Handshaking => "Checking WireTerm/1 receiver…".to_owned(),
        TransferStage::Sending { sent, total } => {
            format!("Sending frame… {sent}/{total} bytes")
        }
        TransferStage::Verified => "Frame CRC verified".to_owned(),
        TransferStage::Refreshing => "Refreshing display…".to_owned(),
        TransferStage::Complete => "Frame displayed · panel power off".to_owned(),
    }
}
