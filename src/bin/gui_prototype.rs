//! THROWAWAY PROTOTYPE: minimal `WireTerm` host GUI.

// Image dimensions, palette indices, and progress values are bounded well below
// the target types' limits in this fixed-size 800 × 480 prototype.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use eframe::egui;
use image::imageops::FilterType;

const WIDTH: usize = 800;
const HEIGHT: usize = 480;
const PLANE_BYTES: usize = WIDTH * HEIGHT / 8;
const FRAME_BYTES: usize = PLANE_BYTES * 2;

fn main() -> eframe::Result<()> {
    eframe::run_native(
        "WireTerm GUI Prototype",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([820.0, 640.0])
                .with_min_inner_size([520.0, 560.0]),
            ..Default::default()
        },
        Box::new(|_| Ok(Box::new(Prototype::new()))),
    )
}

struct PreparedFrame {
    payload: Vec<u8>,
    texture: egui::TextureHandle,
}

enum TransferEvent {
    Progress(f32, String),
    Complete(String),
    Failed(String),
}

struct Prototype {
    selected_path: Option<PathBuf>,
    prepared: Option<PreparedFrame>,
    port_name: Option<String>,
    status: String,
    transfer_progress: f32,
    receiver: Option<Receiver<TransferEvent>>,
}

impl Prototype {
    fn new() -> Self {
        let port_name = find_wireterm_port();
        let status = port_name.as_ref().map_or_else(
            || "ESP32 not found".to_owned(),
            |port| format!("{port} · ESP32 connected"),
        );
        Self {
            selected_path: None,
            prepared: None,
            port_name,
            status,
            transfer_progress: 0.0,
            receiver: None,
        }
    }

    fn choose_image(&mut self, ctx: &egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Images", &["png", "jpg", "jpeg"])
            .pick_file()
        else {
            return;
        };

        "Preparing image…".clone_into(&mut self.status);
        match prepare_frame(&path) {
            Ok((payload, preview)) => {
                let texture =
                    ctx.load_texture("prepared-frame", preview, egui::TextureOptions::NEAREST);
                self.selected_path = Some(path);
                self.prepared = Some(PreparedFrame { payload, texture });
                "Frame ready · 800 × 480 · B/W/R".clone_into(&mut self.status);
            }
            Err(error) => {
                self.prepared = None;
                self.status = format!("Image error: {error}");
            }
        }
    }

    fn start_transfer(&mut self) {
        let (Some(port_name), Some(prepared)) = (&self.port_name, &self.prepared) else {
            return;
        };
        let port_name = port_name.clone();
        let payload = prepared.payload.clone();
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.transfer_progress = 0.0;
        "Connecting…".clone_into(&mut self.status);
        thread::spawn(move || {
            if let Err(error) = send_frame(&port_name, &payload, &sender) {
                let _ = sender.send(TransferEvent::Failed(error));
            }
        });
    }

    fn poll_transfer(&mut self) {
        let Some(receiver) = self.receiver.take() else {
            return;
        };
        let mut finished = false;
        while let Ok(event) = receiver.try_recv() {
            match event {
                TransferEvent::Progress(progress, status) => {
                    self.transfer_progress = progress;
                    self.status = status;
                }
                TransferEvent::Complete(status) => {
                    self.transfer_progress = 1.0;
                    self.status = status;
                    finished = true;
                }
                TransferEvent::Failed(status) => {
                    self.status = format!("Transfer failed: {status}");
                    finished = true;
                }
            }
        }
        if !finished {
            self.receiver = Some(receiver);
        }
    }
}

impl eframe::App for Prototype {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll_transfer();
        if self.receiver.is_some() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        ctx.set_visuals(egui::Visuals::dark());
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_max_width(760.0);
                    ui.add_space(18.0);

                    ui.horizontal(|ui| {
                        ui.heading("WireTerm");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let color = if self.port_name.is_some() {
                                egui::Color32::LIGHT_GREEN
                            } else {
                                egui::Color32::LIGHT_RED
                            };
                            ui.label(egui::RichText::new(&self.status).color(color));
                        });
                    });
                    ui.add_space(18.0);

                    ui.label("E-paper preview · 800 × 480");
                    ui.add_space(6.0);
                    let available_width = ui.available_width();
                    let preview_width = available_width.min(720.0);
                    let preview_height = preview_width * HEIGHT as f32 / WIDTH as f32;
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
                        if ui.button("Choose image").clicked() {
                            self.choose_image(&ctx);
                        }
                        let filename = self
                            .selected_path
                            .as_deref()
                            .and_then(Path::file_name)
                            .and_then(|name| name.to_str())
                            .unwrap_or("No image selected");
                        ui.label(filename);
                        let can_send = self.prepared.is_some()
                            && self.port_name.is_some()
                            && self.receiver.is_none();
                        if ui
                            .add_enabled(can_send, egui::Button::new("Send frame"))
                            .clicked()
                        {
                            self.start_transfer();
                        }
                    });
                    if self.receiver.is_some() {
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

fn find_wireterm_port() -> Option<String> {
    let ports = serialport::available_ports().ok()?;
    ports
        .iter()
        .find(|port| match &port.port_type {
            serialport::SerialPortType::UsbPort(info) => info.vid == 0x10C4 && info.pid == 0xEA60,
            _ => false,
        })
        .or_else(|| {
            ports
                .iter()
                .find(|port| port.port_name.eq_ignore_ascii_case("COM9"))
        })
        .map(|port| port.port_name.clone())
}

#[allow(clippy::too_many_lines)]
fn prepare_frame(path: &Path) -> Result<(Vec<u8>, egui::ColorImage), String> {
    let source = image::open(path)
        .map_err(|error| error.to_string())?
        .to_rgb8();
    let scale = (WIDTH as f32 / source.width() as f32).min(HEIGHT as f32 / source.height() as f32);
    let resized_width = ((source.width() as f32 * scale).round() as u32).max(1);
    let resized_height = ((source.height() as f32 * scale).round() as u32).max(1);
    let resized =
        image::imageops::resize(&source, resized_width, resized_height, FilterType::Lanczos3);
    let offset_x = (WIDTH - resized_width as usize) / 2;
    let offset_y = (HEIGHT - resized_height as usize) / 2;

    let mut working = vec![[255.0_f32; 3]; WIDTH * HEIGHT];
    if offset_y > 0 {
        let sample_rows = resized.height().min(8);
        let top = average_region(&resized, 0, 0, resized.width(), sample_rows);
        let bottom = average_region(
            &resized,
            0,
            resized.height() - sample_rows,
            resized.width(),
            resized.height(),
        );
        fill_region(&mut working, 0, 0, WIDTH, offset_y, top);
        fill_region(
            &mut working,
            0,
            offset_y + resized_height as usize,
            WIDTH,
            HEIGHT,
            bottom,
        );
    } else if offset_x > 0 {
        let sample_columns = resized.width().min(8);
        let left = average_region(&resized, 0, 0, sample_columns, resized.height());
        let right = average_region(
            &resized,
            resized.width() - sample_columns,
            0,
            resized.width(),
            resized.height(),
        );
        fill_region(&mut working, 0, 0, offset_x, HEIGHT, left);
        fill_region(
            &mut working,
            offset_x + resized_width as usize,
            0,
            WIDTH,
            HEIGHT,
            right,
        );
    }
    for (x, y, pixel) in resized.enumerate_pixels() {
        working[(y as usize + offset_y) * WIDTH + x as usize + offset_x] = [
            f32::from(pixel[0]),
            f32::from(pixel[1]),
            f32::from(pixel[2]),
        ];
    }

    let palette = [
        [0.0_f32, 0.0, 0.0],
        [255.0_f32, 255.0, 255.0],
        [205.0_f32, 35.0, 35.0],
    ];
    let mut classes = vec![1_u8; WIDTH * HEIGHT];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let index = y * WIDTH + x;
            let old = working[index];
            let (class, chosen) = palette
                .iter()
                .enumerate()
                .min_by(|(_, left), (_, right)| {
                    color_distance(old, **left).total_cmp(&color_distance(old, **right))
                })
                .map(|(class, color)| (class as u8, *color))
                .expect("palette is non-empty");
            classes[index] = class;
            let error = [old[0] - chosen[0], old[1] - chosen[1], old[2] - chosen[2]];
            diffuse_error(&mut working, x, y, 1, 0, error, 7.0 / 16.0);
            diffuse_error(&mut working, x, y, -1, 1, error, 3.0 / 16.0);
            diffuse_error(&mut working, x, y, 0, 1, error, 5.0 / 16.0);
            diffuse_error(&mut working, x, y, 1, 1, error, 1.0 / 16.0);
        }
    }

    let mut black = vec![0xFF_u8; PLANE_BYTES];
    let mut red = vec![0x00_u8; PLANE_BYTES];
    let mut preview = Vec::with_capacity(WIDTH * HEIGHT);
    for (index, class) in classes.into_iter().enumerate() {
        let mask = 0x80_u8 >> (index % 8);
        match class {
            0 => {
                black[index / 8] &= !mask;
                preview.push(egui::Color32::BLACK);
            }
            2 => {
                red[index / 8] |= mask;
                preview.push(egui::Color32::from_rgb(205, 35, 35));
            }
            _ => preview.push(egui::Color32::WHITE),
        }
    }
    let mut payload = black;
    payload.extend_from_slice(&red);
    debug_assert_eq!(payload.len(), FRAME_BYTES);
    Ok((payload, egui::ColorImage::new([WIDTH, HEIGHT], preview)))
}

fn average_region(
    image: &image::RgbImage,
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
) -> [f32; 3] {
    let mut totals = [0_u64; 3];
    let mut count = 0_u64;
    for y in top..bottom {
        for x in left..right {
            let pixel = image.get_pixel(x, y);
            for channel in 0..3 {
                totals[channel] += u64::from(pixel[channel]);
            }
            count += 1;
        }
    }
    [
        totals[0] as f32 / count as f32,
        totals[1] as f32 / count as f32,
        totals[2] as f32 / count as f32,
    ]
}

fn fill_region(
    pixels: &mut [[f32; 3]],
    left: usize,
    top: usize,
    right: usize,
    bottom: usize,
    color: [f32; 3],
) {
    for y in top..bottom {
        for x in left..right {
            pixels[y * WIDTH + x] = color;
        }
    }
}

fn color_distance(left: [f32; 3], right: [f32; 3]) -> f32 {
    let red = left[0] - right[0];
    let green = left[1] - right[1];
    let blue = left[2] - right[2];
    (blue * blue).mul_add(0.11, (green * green).mul_add(0.59, red * red * 0.30))
}

fn diffuse_error(
    pixels: &mut [[f32; 3]],
    x: usize,
    y: usize,
    dx: isize,
    dy: isize,
    error: [f32; 3],
    factor: f32,
) {
    let next_x = x as isize + dx;
    let next_y = y as isize + dy;
    if next_x < 0 || next_y < 0 || next_x >= WIDTH as isize || next_y >= HEIGHT as isize {
        return;
    }
    let pixel = &mut pixels[next_y as usize * WIDTH + next_x as usize];
    for channel in 0..3 {
        pixel[channel] = error[channel]
            .mul_add(factor, pixel[channel])
            .clamp(0.0, 255.0);
    }
}

fn send_frame(
    port_name: &str,
    payload: &[u8],
    events: &Sender<TransferEvent>,
) -> Result<(), String> {
    let mut port = serialport::new(port_name, 115_200)
        .timeout(Duration::from_secs(3))
        .open()
        .map_err(|error| error.to_string())?;
    thread::sleep(Duration::from_secs(2));
    port.clear(serialport::ClearBuffer::Input)
        .map_err(|error| error.to_string())?;

    let mut hello = String::new();
    for _ in 0..3 {
        port.write_all(b"HELLO WIRETERM/1\n")
            .map_err(|error| error.to_string())?;
        port.flush().map_err(|error| error.to_string())?;
        hello = read_response(&mut *port)?;
        if hello.starts_with("OK WIRETERM/1") {
            break;
        }
        thread::sleep(Duration::from_millis(150));
    }
    if !hello.starts_with("OK WIRETERM/1") {
        return Err(format!("unexpected handshake: {hello}"));
    }

    let crc = crc32fast::hash(payload);
    let begin = format!("BEGIN 800 480 BWR {FRAME_BYTES} {crc:08X}\n");
    port.write_all(begin.as_bytes())
        .map_err(|error| error.to_string())?;
    let ready = read_response(&mut *port)?;
    if !ready.starts_with("OK BEGIN READY") {
        return Err(format!("device rejected frame: {ready}"));
    }

    for (chunk_index, chunk) in payload.chunks(1024).enumerate() {
        port.write_all(chunk).map_err(|error| error.to_string())?;
        let sent = ((chunk_index + 1) * 1024).min(payload.len());
        let progress = sent as f32 / payload.len() as f32 * 0.65;
        let _ = events.send(TransferEvent::Progress(
            progress,
            format!("Sending frame… {sent}/{FRAME_BYTES} bytes"),
        ));
    }
    port.flush().map_err(|error| error.to_string())?;

    port.set_timeout(Duration::from_secs(8))
        .map_err(|error| error.to_string())?;
    let verified = read_response(&mut *port)?;
    if !verified.starts_with("OK FRAME VERIFIED") {
        return Err(format!("verification failed: {verified}"));
    }
    let _ = events.send(TransferEvent::Progress(
        0.75,
        "Frame verified · refreshing display…".to_owned(),
    ));

    port.set_timeout(Duration::from_secs(60))
        .map_err(|error| error.to_string())?;
    let displayed = read_response(&mut *port)?;
    if !displayed.starts_with("OK FRAME DISPLAYED") {
        return Err(format!("display failed: {displayed}"));
    }
    let _ = events.send(TransferEvent::Complete(
        "Frame displayed · panel power off".to_owned(),
    ));
    Ok(())
}

fn read_response(port: &mut dyn serialport::SerialPort) -> Result<String, String> {
    let mut line = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        port.read_exact(&mut byte)
            .map_err(|error| error.to_string())?;
        if byte[0] == b'\n' {
            break;
        }
        if byte[0] != b'\r' {
            line.push(byte[0]);
        }
        if line.len() >= 256 {
            return Err("device response was too long".to_owned());
        }
    }
    Ok(String::from_utf8_lossy(&line)
        .trim_matches(|character: char| character.is_control() || character == '\u{FFFD}')
        .to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepares_device_ready_lighthouse_frame() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/prototype/lighthouse-test-frame-bwr-800x480.png");
        let (payload, preview) = prepare_frame(&path).expect("test image should convert");

        assert_eq!(payload.len(), FRAME_BYTES);
        assert_eq!(preview.size, [WIDTH, HEIGHT]);
        let (black, red) = payload.split_at(PLANE_BYTES);
        assert!(
            black
                .iter()
                .zip(red)
                .all(|(black_byte, red_byte)| (!black_byte & red_byte) == 0)
        );
        assert_ne!(crc32fast::hash(&payload), 0);
    }

    #[test]
    #[ignore = "requires the connected WireTerm ESP32 and refreshes the panel"]
    fn sends_lighthouse_to_connected_panel() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/prototype/lighthouse-test-frame-bwr-800x480.png");
        let (payload, _) = prepare_frame(&path).expect("test image should convert");
        let port = find_wireterm_port().expect("WireTerm ESP32 should be connected");
        let (sender, _receiver) = mpsc::channel();

        send_frame(&port, &payload, &sender).expect("frame should verify and display");
    }
}
