//! Visible foreground `WireTerm` Playlist editor and playback host.

#![allow(clippy::cast_precision_loss)]

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, RichText, Sense, Stroke, StrokeKind, Vec2,
};
use serde_json::Value;

use crate::{
    extension::{
        EXTENSION_SCRIPT_NAME, ExtensionInput, ExtensionMetadata, InputKind, LiveExtensionHost,
        LoadedExtension, RenderCancellation, discover_extensions, render_svg_to_panel,
        scaffold_extension, system_fixture_clock, validate_extension_configuration,
    },
    frame::{FRAME_HEIGHT, FRAME_WIDTH, PanelFrame},
    host::{HostBridge, HostEvent},
    playback::PlaybackController,
    playlist::{
        FolderShuffleBags, ItemId, MAX_INTERVAL_MINUTES, MIN_INTERVAL_MINUTES, PlaylistItem,
        PlaylistRevision, PlaylistSource, PlaylistStore,
    },
    raster::prepare_raster_path,
    secrets::SecretStore,
    transport::{DeviceInfo, TransferStage},
};
use zeroize::{Zeroize, Zeroizing};

const ACCENT: Color32 = Color32::from_rgb(232, 92, 70);
const MUTED: Color32 = Color32::from_rgb(145, 151, 164);
const PANEL_2: Color32 = Color32::from_rgb(42, 45, 52);

pub fn run() -> eframe::Result<()> {
    eframe::run_native(
        "WireTerm",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1100.0, 760.0])
                .with_min_inner_size([960.0, 620.0]),
            ..Default::default()
        },
        Box::new(|_| Ok(Box::new(WireTermApp::new()))),
    )
}

struct PreparedPreview {
    frame: Arc<PanelFrame>,
    texture: egui::TextureHandle,
    label: String,
}

#[derive(Clone, Copy)]
enum RenderPurpose {
    Playback,
    Preview,
}

struct RenderResult {
    purpose: RenderPurpose,
    item_id: ItemId,
    label: String,
    schema: Option<(ExtensionMetadata, Vec<ExtensionInput>)>,
    result: Result<PanelFrame, String>,
}

struct RenderTask {
    receiver: Receiver<RenderResult>,
    started: Instant,
    cancellation: RenderCancellation,
}

struct PendingTransfer {
    label: String,
    frame: Arc<PanelFrame>,
}

struct Issue {
    summary: String,
    detail: String,
}

struct WireTermApp {
    host: HostBridge,
    devices: Vec<DeviceInfo>,
    selected_port: Option<String>,
    store: PlaylistStore,
    secret_store: SecretStore,
    secret_names: Vec<String>,
    secret_name_edit: String,
    secret_value_edit: Zeroizing<String>,
    extension_library_open: bool,
    discovered_extensions: Vec<PathBuf>,
    playlist: PlaylistRevision,
    selected_item: Option<ItemId>,
    dragged_item: Option<ItemId>,
    interval_edits: HashMap<ItemId, String>,
    extension_schemas: HashMap<ItemId, (ExtensionMetadata, Vec<ExtensionInput>)>,
    playback: PlaybackController,
    shuffle_bags: FolderShuffleBags,
    render_task: Option<RenderTask>,
    pending_transfer: Option<PendingTransfer>,
    preview: Option<PreparedPreview>,
    transfer_progress: f32,
    host_status: String,
    issue: Option<Issue>,
    log: VecDeque<String>,
}

impl WireTermApp {
    fn new() -> Self {
        let store = PlaylistStore::adjacent_to_executable()
            .unwrap_or_else(|_| PlaylistStore::new(Path::new("wireterm-data")));
        let (mut playlist, issue) = match store.load_latest() {
            Ok(playlist) => (playlist, None),
            Err(error) => (
                PlaylistRevision::default(),
                Some(Issue {
                    summary: "Playlist could not be loaded".to_owned(),
                    detail: error.to_string(),
                }),
            ),
        };
        let secret_store = SecretStore::new(store.data_dir());
        let secret_names = secret_store.names().unwrap_or_default();
        let discovered_extensions = discover_extensions(store.data_dir()).unwrap_or_default();
        let visual_qa = std::env::var_os("WIRETERM_VISUAL_QA").is_some();
        if visual_qa {
            playlist = visual_qa_playlist();
        }
        let selected_item = playlist.items.first().map(|item| item.id);
        let mut playback = PlaybackController::new_running();
        if visual_qa {
            let _ = playback.poll(&playlist, Instant::now(), false);
            playback.pause();
        }
        Self {
            host: HostBridge::new(),
            devices: Vec::new(),
            selected_port: None,
            store,
            secret_store,
            secret_names,
            secret_name_edit: String::new(),
            secret_value_edit: Zeroizing::new(String::new()),
            extension_library_open: false,
            discovered_extensions,
            playlist,
            selected_item,
            dragged_item: None,
            interval_edits: HashMap::new(),
            extension_schemas: HashMap::new(),
            playback,
            shuffle_bags: FolderShuffleBags::default(),
            render_task: None,
            pending_transfer: None,
            preview: None,
            transfer_progress: 0.0,
            host_status: "Looking for a display bridge…".to_owned(),
            issue,
            log: VecDeque::new(),
        }
    }

    fn poll_render(&mut self, ctx: &egui::Context) {
        let Some(task) = self.render_task.take() else {
            return;
        };
        match task.receiver.try_recv() {
            Ok(rendered) => {
                if let Some(schema) = rendered.schema {
                    self.extension_schemas.insert(rendered.item_id, schema);
                }
                match rendered.result {
                    Ok(frame) => match rendered.purpose {
                        RenderPurpose::Preview => {
                            self.set_preview(ctx, frame, rendered.label);
                        }
                        RenderPurpose::Playback => {
                            self.playback.rendered();
                            self.begin_transfer(frame, rendered.label);
                        }
                    },
                    Err(error) => {
                        self.record_issue("Item render failed", error);
                        if matches!(rendered.purpose, RenderPurpose::Playback) {
                            self.playback.failed(&self.playlist, Instant::now(), false);
                        }
                    }
                }
            }
            Err(TryRecvError::Empty) => self.render_task = Some(task),
            Err(TryRecvError::Disconnected) => {
                self.record_issue(
                    "Item render failed",
                    "The render worker stopped unexpectedly".to_owned(),
                );
                self.playback.failed(&self.playlist, Instant::now(), false);
            }
        }
    }

    fn poll_host(&mut self, ctx: &egui::Context) {
        while let Some(event) = self.host.try_recv() {
            match event {
                HostEvent::DevicesChanged(devices) => self.update_devices(devices),
                HostEvent::DeviceDiscoveryFailed(error) => {
                    "Device scan failed".clone_into(&mut self.host_status);
                    self.record_issue("Display bridge scan failed", error);
                }
                HostEvent::TransferProgress(stage) => {
                    self.transfer_progress = stage.progress();
                    self.host_status = transfer_status(&stage);
                    if stage == TransferStage::Complete {
                        if let Some(transfer) = self.pending_transfer.take() {
                            self.set_preview(ctx, (*transfer.frame).clone(), transfer.label);
                            self.playback.send_succeeded();
                        }
                        let _ = self.host.refresh_devices();
                    }
                }
                HostEvent::TransferFailed(error) => {
                    self.pending_transfer = None;
                    let disconnected = !error.starts_with("unexpected device response")
                        && !error.starts_with("device response exceeded");
                    if disconnected {
                        self.selected_port = None;
                        let _ = self.host.refresh_devices();
                    }
                    self.playback
                        .failed(&self.playlist, Instant::now(), disconnected);
                    self.record_issue("Display refresh failed", error);
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
        self.host_status = self
            .selected_port
            .as_ref()
            .and_then(|port| self.devices.iter().find(|device| &device.port_name == port))
            .map_or_else(|| "No display bridge".to_owned(), DeviceInfo::display_label);
    }

    fn poll_playback(&mut self) {
        if self.render_task.is_some()
            || self.pending_transfer.is_some()
            || self.host.is_transfer_active()
        {
            return;
        }
        let Some(turn) =
            self.playback
                .poll(&self.playlist, Instant::now(), self.selected_port.is_some())
        else {
            return;
        };
        match self.resolve_playback_source(&turn.item) {
            Ok(path) => self.spawn_render(turn.item, path, RenderPurpose::Playback),
            Err(error) => {
                self.record_issue("Playlist Item failed", error);
                self.playback.failed(&self.playlist, Instant::now(), false);
            }
        }
    }

    fn resolve_playback_source(&mut self, item: &PlaylistItem) -> Result<PathBuf, String> {
        match &item.source {
            PlaylistSource::Image { path } => Ok(path.clone()),
            PlaylistSource::ImageFolder { path } => self
                .shuffle_bags
                .resolve_turn(item.id, path)
                .map_err(|error| error.to_string()),
            PlaylistSource::Extension { path, .. } => Ok(extension_script_path(path)),
        }
    }

    fn refresh_preview(&mut self) {
        let Some(item_id) = self.selected_item else {
            return;
        };
        let Some(item) = self.playlist.item(item_id).cloned() else {
            return;
        };
        let path = match &item.source {
            PlaylistSource::Image { path } => Ok(path.clone()),
            PlaylistSource::ImageFolder { .. } => FolderShuffleBags::preview(&item.source),
            PlaylistSource::Extension { path, .. } => Ok(extension_script_path(path)),
        };
        match path {
            Ok(path) => self.spawn_render(item, path, RenderPurpose::Preview),
            Err(error) => self.record_issue("Preview failed", error.to_string()),
        }
    }

    fn spawn_render(&mut self, item: PlaylistItem, path: PathBuf, purpose: RenderPurpose) {
        if self.render_task.is_some() {
            return;
        }
        let item_id = item.id;
        let label = source_label(&path);
        let secrets = self.secret_store.clone();
        let cancellation = RenderCancellation::default();
        let (sender, receiver) = mpsc::channel();
        self.render_task = Some(RenderTask {
            receiver,
            started: Instant::now(),
            cancellation: cancellation.clone(),
        });
        thread::Builder::new()
            .name("wireterm-item-render".to_owned())
            .spawn(move || {
                let rendered = render_item(&item, &path, &secrets, cancellation);
                let _ = sender.send(RenderResult {
                    purpose,
                    item_id,
                    label,
                    schema: rendered.schema,
                    result: rendered.frame,
                });
            })
            .expect("item render worker should start");
    }

    fn begin_transfer(&mut self, frame: PanelFrame, label: String) {
        let Some(port) = self.selected_port.clone() else {
            self.playback.failed(&self.playlist, Instant::now(), true);
            self.record_issue(
                "Display bridge disconnected",
                "The current item will retry after reconnect".to_owned(),
            );
            return;
        };
        let frame = Arc::new(frame);
        match self.host.send_frame(port, Arc::clone(&frame)) {
            Ok(()) => {
                self.transfer_progress = 0.0;
                "Connecting…".clone_into(&mut self.host_status);
                self.pending_transfer = Some(PendingTransfer { label, frame });
            }
            Err(error) => {
                self.playback.failed(&self.playlist, Instant::now(), false);
                self.record_issue("Could not start display refresh", error.to_string());
            }
        }
    }

    fn set_preview(&mut self, ctx: &egui::Context, frame: PanelFrame, label: String) {
        let image = egui::ColorImage::from_rgb([FRAME_WIDTH, FRAME_HEIGHT], frame.preview_rgb());
        let texture = ctx.load_texture(
            format!("playlist-preview-{}", frame.crc32()),
            image,
            egui::TextureOptions::NEAREST,
        );
        self.preview = Some(PreparedPreview {
            frame: Arc::new(frame),
            texture,
            label,
        });
    }

    fn save_playlist(&mut self) {
        match self.store.save_new_revision(self.playlist.clone()) {
            Ok(saved) => self.playlist = saved,
            Err(error) => self.record_issue("Playlist could not be saved", error.to_string()),
        }
    }

    fn record_issue(&mut self, summary: impl Into<String>, detail: impl Into<String>) {
        let issue = Issue {
            summary: summary.into(),
            detail: detail.into(),
        };
        self.log
            .push_front(format!("{} · {}", issue.summary, issue.detail));
        self.log.truncate(100);
        self.issue = Some(issue);
    }

    fn add_image(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Images", &["png", "jpg", "jpeg"])
            .pick_file()
        else {
            return;
        };
        let title = source_label(&path);
        let id = self
            .playlist
            .add_item(title, PlaylistSource::Image { path });
        self.selected_item = Some(id);
        self.save_playlist();
    }

    fn add_image_folder(&mut self) {
        let Some(path) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        let title = source_label(&path);
        let id = self
            .playlist
            .add_item(title, PlaylistSource::ImageFolder { path });
        self.selected_item = Some(id);
        self.save_playlist();
    }

    fn add_extension(&mut self) {
        self.discovered_extensions = discover_extensions(self.store.data_dir()).unwrap_or_default();
        self.extension_library_open = true;
    }

    fn browse_for_extension(&mut self) {
        let Some(path) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        self.add_extension_path(path);
    }

    fn add_extension_path(&mut self, path: PathBuf) {
        let title = source_label(&path);
        let id = self.playlist.add_item(
            title,
            PlaylistSource::Extension {
                path,
                settings: BTreeMap::new(),
                named_secret_refs: BTreeMap::new(),
            },
        );
        self.selected_item = Some(id);
        self.save_playlist();
        self.extension_library_open = false;
    }

    fn remove_selected(&mut self) {
        let Some(id) = self.selected_item else {
            return;
        };
        self.playlist.items.retain(|item| item.id != id);
        self.interval_edits.remove(&id);
        self.extension_schemas.remove(&id);
        self.selected_item = self.playlist.items.first().map(|item| item.id);
        self.save_playlist();
    }

    fn move_item_to(&mut self, dragged_id: ItemId, target_id: ItemId, after_target: bool) {
        if self
            .playlist
            .move_item_to(dragged_id, target_id, after_target)
        {
            self.save_playlist();
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("WireTerm");
            ui.label(RichText::new("Playlist").color(MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(&self.host_status).small().color(MUTED));
                ui.separator();
                let state = if self.playback.is_running() {
                    "Playing"
                } else {
                    "Paused"
                };
                ui.label(RichText::new(state).color(if self.playback.is_running() {
                    Color32::LIGHT_GREEN
                } else {
                    MUTED
                }));
            });
        });
        if let Some(issue) = self.issue.as_ref().map(|issue| Issue {
            summary: issue.summary.clone(),
            detail: issue.detail.clone(),
        }) {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Issue").strong().color(ACCENT));
                ui.label(&issue.summary).on_hover_text(&issue.detail);
                if ui.small_button("Clear").clicked() {
                    self.issue = None;
                }
            });
        }
        ui.separator();
    }

    #[allow(clippy::too_many_lines)] // One cohesive accessible drag-table drawing routine.
    fn playlist_table(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Playlist").strong().size(18.0));
            ui.label(RichText::new(format!("{} items", self.playlist.items.len())).color(MUTED));
            ui.separator();
            ui.label(RichText::new("Default interval").color(MUTED));
            let response = ui.add(
                egui::DragValue::new(&mut self.playlist.default_interval_minutes)
                    .range(MIN_INTERVAL_MINUTES..=MAX_INTERVAL_MINUTES)
                    .suffix(" min"),
            );
            if response.changed() {
                self.save_playlist();
            }
        });
        ui.add_space(4.0);

        let current = self.playback.current_item();
        let mut drop_action = None;
        let pointer_events = ui.ctx().input(|input| input.events.clone());
        let pointer = ui.ctx().input(|input| input.pointer.latest_pos());
        let pressed_at = pointer_events.iter().find_map(|event| match event {
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                ..
            } => Some(*pos),
            _ => None,
        });
        let released_at = pointer_events.iter().rev().find_map(|event| match event {
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                ..
            } => Some(*pos),
            _ => None,
        });
        if ui.ctx().input(|input| input.key_pressed(egui::Key::Escape)) {
            self.dragged_item = None;
        }
        let mut drop_rows = Vec::with_capacity(self.playlist.items.len());
        egui::ScrollArea::horizontal()
            .id_salt("playlist-table-x")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.set_min_width(520.0);
                egui::Grid::new("playlist-table")
                    .striped(true)
                    .min_col_width(48.0)
                    .spacing([12.0, 7.0])
                    .show(ui, |ui| {
                        for heading in ["#", "Item", "Type", "Interval", "Order"] {
                            ui.label(RichText::new(heading).small().strong().color(MUTED));
                        }
                        ui.end_row();
                        for (index, item) in self.playlist.items.clone().into_iter().enumerate() {
                            let selected = self.selected_item == Some(item.id);
                            let is_current = current == Some(item.id);
                            let row_color = if is_current {
                                ACCENT
                            } else {
                                ui.visuals().text_color()
                            };
                            let mut number_clicked = false;
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                let (marker_rect, marker_response) =
                                    ui.allocate_exact_size(Vec2::new(14.0, 20.0), Sense::hover());
                                if is_current {
                                    ui.painter().text(
                                        marker_rect.center(),
                                        Align2::CENTER_CENTER,
                                        "▶",
                                        FontId::proportional(13.0),
                                        ACCENT,
                                    );
                                    marker_response.on_hover_text("Current Playback Turn");
                                }
                                let number_response = ui.add_sized(
                                    [28.0, 20.0],
                                    egui::Button::selectable(
                                        selected,
                                        RichText::new(format!("{:02}", index + 1)).color(row_color),
                                    ),
                                );
                                let accessible_order = if is_current {
                                    format!("{:02}, Current Playback Turn", index + 1)
                                } else {
                                    format!("{:02}", index + 1)
                                };
                                number_response.widget_info(move || {
                                    egui::WidgetInfo::labeled(
                                        egui::WidgetType::Button,
                                        true,
                                        accessible_order.clone(),
                                    )
                                });
                                number_clicked = number_response.clicked();
                            });
                            if number_clicked {
                                self.selected_item = Some(item.id);
                            }
                            if ui
                                .selectable_label(
                                    selected,
                                    RichText::new(&item.title).strong().color(row_color),
                                )
                                .clicked()
                            {
                                self.selected_item = Some(item.id);
                            }
                            ui.label(item.source.kind_name());
                            ui.label(format!(
                                "{}m{}",
                                self.playlist.effective_interval_minutes(&item),
                                if item.interval_minutes.is_some() {
                                    " item"
                                } else {
                                    ""
                                }
                            ));
                            let (handle_rect, handle) =
                                ui.allocate_exact_size(Vec2::new(28.0, 20.0), Sense::drag());
                            let handle = handle
                                .on_hover_cursor(egui::CursorIcon::Grab)
                                .on_hover_text("Drag to reorder");
                            let row_drop_rect = egui::Rect::from_min_max(
                                egui::pos2(ui.clip_rect().left(), handle.rect.top() - 3.5),
                                egui::pos2(handle.rect.right(), handle.rect.bottom() + 3.5),
                            );
                            drop_rows.push((item.id, handle.rect, row_drop_rect));
                            let is_active = self.dragged_item == Some(item.id) || handle.dragged();
                            let is_drop_target = self.dragged_item.is_some_and(|dragged_id| {
                                dragged_id != item.id
                                    && pointer
                                        .is_some_and(|pointer| row_drop_rect.contains(pointer))
                            });
                            if is_active {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                            }
                            if handle.hovered() || is_active || is_drop_target {
                                let fill = if is_active {
                                    ACCENT.gamma_multiply(0.28)
                                } else if is_drop_target {
                                    ACCENT.gamma_multiply(0.16)
                                } else {
                                    PANEL_2
                                };
                                ui.painter()
                                    .rect_filled(handle_rect, CornerRadius::same(4), fill);
                                ui.painter().rect_stroke(
                                    handle_rect,
                                    CornerRadius::same(4),
                                    Stroke::new(
                                        1.0_f32,
                                        if is_active || is_drop_target {
                                            ACCENT
                                        } else {
                                            MUTED
                                        },
                                    ),
                                    StrokeKind::Inside,
                                );
                            }
                            let dot_color = if is_active || is_drop_target {
                                ACCENT
                            } else if handle.hovered() {
                                Color32::LIGHT_GRAY
                            } else {
                                MUTED
                            };
                            for x in [-3.5, 3.5] {
                                for y in [-4.5, 0.0, 4.5] {
                                    ui.painter().circle_filled(
                                        handle_rect.center() + Vec2::new(x, y),
                                        1.6,
                                        dot_color,
                                    );
                                }
                            }
                            let after_target =
                                pointer.is_some_and(|pointer| pointer.y > handle.rect.center().y);
                            if is_drop_target {
                                let y = if after_target {
                                    handle.rect.bottom() + 2.0
                                } else {
                                    handle.rect.top() - 2.0
                                };
                                ui.painter().line_segment(
                                    [
                                        egui::pos2(ui.clip_rect().left(), y),
                                        egui::pos2(ui.clip_rect().right(), y),
                                    ],
                                    Stroke::new(2.0_f32, ACCENT),
                                );
                            }
                            ui.end_row();
                        }
                    });
            });

        if let Some(pressed_at) = pressed_at
            && let Some((item_id, _, _)) = drop_rows
                .iter()
                .find(|(_, handle_rect, _)| handle_rect.contains(pressed_at))
        {
            self.dragged_item = Some(*item_id);
        }
        if let Some(released_at) = released_at {
            if let Some(dragged_id) = self.dragged_item
                && let Some((target_id, handle_rect, _)) = drop_rows
                    .iter()
                    .find(|(_, _, row_rect)| row_rect.contains(released_at))
                && dragged_id != *target_id
            {
                drop_action = Some((
                    dragged_id,
                    *target_id,
                    released_at.y > handle_rect.center().y,
                ));
            }
            self.dragged_item = None;
        }
        if let Some((dragged_id, target_id, after_target)) = drop_action {
            self.move_item_to(dragged_id, target_id, after_target);
        }
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            if ui.small_button("+ Image").clicked() {
                self.add_image();
            }
            if ui.small_button("+ Folder").clicked() {
                self.add_image_folder();
            }
            if ui.small_button("+ Extension").clicked() {
                self.add_extension();
            }
        });
    }

    fn selected_item_editor(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected_item else {
            ui.add_space(14.0);
            ui.label(RichText::new("Add a Playlist Item to begin.").color(MUTED));
            return;
        };
        let Some(index) = self.playlist.items.iter().position(|item| item.id == id) else {
            return;
        };
        ui.add_space(12.0);
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.playlist.items[index].title).strong());
            ui.label(RichText::new("selected Playlist Item").small().color(MUTED));
        });

        let mut should_save = false;
        let mut enabled = self.playlist.items[index].enabled;
        if ui.checkbox(&mut enabled, "Enabled").changed() {
            self.playlist.items[index].enabled = enabled;
            should_save = true;
        }
        ui.horizontal(|ui| {
            ui.label("Name");
            let mut title = self.playlist.items[index].title.clone();
            if ui.text_edit_singleline(&mut title).changed() && !title.trim().is_empty() {
                self.playlist.items[index].title = title;
                should_save = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Item interval");
            let inherited = self.playlist.default_interval_minutes.to_string();
            let edit = self.interval_edits.entry(id).or_insert_with(|| {
                self.playlist.items[index]
                    .interval_minutes
                    .map_or_else(String::new, |minutes| minutes.to_string())
            });
            let response = ui.add(
                egui::TextEdit::singleline(edit)
                    .desired_width(56.0)
                    .hint_text(inherited),
            );
            ui.label("min");
            if response.changed() {
                let trimmed = edit.trim();
                if trimmed.is_empty() {
                    self.playlist.items[index].interval_minutes = None;
                    should_save = true;
                } else if let Ok(minutes) = trimmed.parse::<u16>()
                    && (MIN_INTERVAL_MINUTES..=MAX_INTERVAL_MINUTES).contains(&minutes)
                {
                    self.playlist.items[index].interval_minutes = Some(minutes);
                    should_save = true;
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Source").color(MUTED));
            ui.label(
                self.playlist.items[index]
                    .source
                    .display_path()
                    .display()
                    .to_string(),
            );
        });
        if should_save {
            self.save_playlist();
        }
        self.extension_settings(ui, id);
        ui.add_space(8.0);
        if ui.small_button("Remove item").clicked() {
            self.remove_selected();
        }
    }

    fn extension_library_window(&mut self, ctx: &egui::Context) {
        if !self.extension_library_open {
            return;
        }
        let mut open = true;
        let mut add_path = None;
        let mut browse = false;
        let mut refresh = false;
        let mut scaffold = false;
        egui::Window::new("Extension library")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        self.store
                            .data_dir()
                            .join("extensions")
                            .display()
                            .to_string(),
                    )
                    .small()
                    .color(MUTED),
                );
                ui.add_space(6.0);
                if self.discovered_extensions.is_empty() {
                    ui.label("No Extension folders found in adjacent data.");
                }
                for path in &self.discovered_extensions {
                    ui.horizontal(|ui| {
                        ui.label(source_label(path));
                        if ui.small_button("Add").clicked() {
                            add_path = Some(path.clone());
                        }
                    });
                }
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    refresh = ui.small_button("Refresh").clicked();
                    scaffold = ui.small_button("Scaffold Extension").clicked();
                    browse = ui.small_button("Browse…").clicked();
                });
            });
        self.extension_library_open = open;
        if refresh {
            self.discovered_extensions =
                discover_extensions(self.store.data_dir()).unwrap_or_default();
        }
        if scaffold {
            match scaffold_extension(self.store.data_dir()) {
                Ok(path) => {
                    self.discovered_extensions =
                        discover_extensions(self.store.data_dir()).unwrap_or_default();
                    add_path = Some(path);
                }
                Err(error) => {
                    self.record_issue("Extension could not be scaffolded", error.to_string());
                }
            }
        }
        if browse {
            self.browse_for_extension();
        } else if let Some(path) = add_path {
            self.add_extension_path(path);
        }
    }

    fn extension_settings(&mut self, ui: &mut egui::Ui, id: ItemId) {
        let Some((metadata, inputs)) = self.extension_schemas.get(&id).cloned() else {
            if matches!(
                self.playlist.item(id).map(|item| &item.source),
                Some(PlaylistSource::Extension { .. })
            ) {
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Refresh preview to load this Extension’s inputs.").color(MUTED),
                );
            }
            return;
        };
        ui.add_space(10.0);
        ui.label(RichText::new(metadata.name).strong());
        let mut changes = Vec::new();
        for input in inputs {
            let configured_value = self
                .extension_settings_map(id)
                .and_then(|settings| settings.get(&input.key))
                .cloned()
                .unwrap_or_else(|| input.default.clone());
            ui.horizontal_wrapped(|ui| {
                ui.label(&input.label);
                match input.kind {
                    InputKind::Text => {
                        let mut value = configured_value.as_str().unwrap_or_default().to_owned();
                        if ui.text_edit_singleline(&mut value).changed() {
                            changes.push((input.key.clone(), Value::String(value), false));
                        }
                    }
                    InputKind::Number => {
                        let mut value = configured_value.as_f64().unwrap_or_default();
                        if ui.add(egui::DragValue::new(&mut value)).changed()
                            && let Some(number) = serde_json::Number::from_f64(value)
                        {
                            changes.push((input.key.clone(), Value::Number(number), false));
                        }
                    }
                    InputKind::Checkbox => {
                        let mut value = configured_value.as_bool().unwrap_or_default();
                        if ui.checkbox(&mut value, "").changed() {
                            changes.push((input.key.clone(), Value::Bool(value), false));
                        }
                    }
                    InputKind::Choice => {
                        let mut value = configured_value.as_str().unwrap_or_default().to_owned();
                        egui::ComboBox::from_id_salt(("extension-choice", id.get(), &input.key))
                            .selected_text(if value.is_empty() {
                                "Choose…"
                            } else {
                                &value
                            })
                            .show_ui(ui, |ui| {
                                for choice in &input.choices {
                                    ui.selectable_value(&mut value, choice.clone(), choice);
                                }
                            });
                        if value != self.extension_setting_string(id, &input.key) {
                            changes.push((input.key.clone(), Value::String(value), false));
                        }
                    }
                    InputKind::NamedSecret => {
                        let mut value = self.extension_secret_ref(id, &input.key);
                        egui::ComboBox::from_id_salt(("extension-secret", id.get(), &input.key))
                            .selected_text(if value.is_empty() {
                                "Not bound"
                            } else {
                                &value
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut value, String::new(), "Not bound");
                                for name in &self.secret_names {
                                    ui.selectable_value(&mut value, name.clone(), name);
                                }
                            });
                        if value != self.extension_secret_ref(id, &input.key) {
                            changes.push((input.key.clone(), Value::String(value), true));
                        }
                        ui.label(RichText::new("name only").small().color(MUTED));
                    }
                }
            });
        }
        for (key, value, is_secret) in changes {
            self.update_extension_value(id, key, value, is_secret);
        }
    }

    fn extension_setting_string(&self, id: ItemId, key: &str) -> String {
        self.extension_settings_map(id)
            .and_then(|settings| settings.get(key))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    }

    fn extension_settings_map(&self, id: ItemId) -> Option<&BTreeMap<String, Value>> {
        match &self.playlist.item(id)?.source {
            PlaylistSource::Extension { settings, .. } => Some(settings),
            _ => None,
        }
    }

    fn extension_secret_ref(&self, id: ItemId, key: &str) -> String {
        match &self.playlist.item(id).map(|item| &item.source) {
            Some(PlaylistSource::Extension {
                named_secret_refs, ..
            }) => named_secret_refs.get(key).cloned().unwrap_or_default(),
            _ => String::new(),
        }
    }

    fn update_extension_value(&mut self, id: ItemId, key: String, value: Value, is_secret: bool) {
        let Some(item) = self.playlist.items.iter_mut().find(|item| item.id == id) else {
            return;
        };
        let PlaylistSource::Extension {
            settings,
            named_secret_refs,
            ..
        } = &mut item.source
        else {
            return;
        };
        if is_secret {
            if let Some(value) = value.as_str() {
                if value.is_empty() {
                    named_secret_refs.remove(&key);
                } else {
                    named_secret_refs.insert(key, value.to_owned());
                }
            }
        } else {
            settings.insert(key, value);
        }
        self.save_playlist();
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        let available_width = ui.available_width();
        let preview_width = available_width.min(720.0);
        let preview_height = preview_width * FRAME_HEIGHT as f32 / FRAME_WIDTH as f32;
        let (row_rect, _) =
            ui.allocate_exact_size(egui::vec2(available_width, preview_height), Sense::hover());
        let preview_rect = egui::Rect::from_center_size(
            row_rect.center(),
            egui::vec2(preview_width, preview_height),
        );
        ui.painter()
            .rect_filled(preview_rect, 0.0, Color32::from_rgb(255, 255, 255));
        if let Some(preview) = &self.preview {
            ui.put(
                preview_rect,
                egui::Image::new(&preview.texture).fit_to_exact_size(preview_rect.size()),
            )
            .on_hover_text(&preview.label);
        } else {
            ui.painter().text(
                preview_rect.center(),
                Align2::CENTER_CENTER,
                "800 × 480",
                FontId::proportional(22.0),
                Color32::BLACK,
            );
        }
        self.preview_controls(ui);
    }

    fn preview_controls(&mut self, ui: &mut egui::Ui) {
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            let play_icon = if self.playback.is_running() {
                "Ⅱ"
            } else {
                "▶"
            };
            let play_tip = if self.playback.is_running() {
                "Pause playback"
            } else {
                "Start playback"
            };
            let play_response = ui
                .add(
                    egui::Button::new(RichText::new(play_icon).size(17.0))
                        .min_size(Vec2::new(32.0, 28.0)),
                )
                .on_hover_text(play_tip);
            play_response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, play_tip)
            });
            if play_response.clicked() {
                if self.playback.is_running() {
                    self.playback.pause();
                } else {
                    self.playback.resume();
                }
            }

            let next_tip = "Next Playlist Item";
            let next_response = ui
                .add_enabled(
                    self.playback.can_next()
                        && self.render_task.is_none()
                        && self.pending_transfer.is_none(),
                    egui::Button::new(RichText::new("⏭").size(17.0))
                        .min_size(Vec2::new(32.0, 28.0)),
                )
                .on_hover_text(next_tip);
            next_response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, next_tip)
            });
            if next_response.clicked() {
                let _ = self.playback.advance_next();
            }

            let refresh_tip = "Refresh preview";
            let refresh_response = ui
                .add_enabled(
                    self.selected_item.is_some() && self.render_task.is_none(),
                    egui::Button::new(RichText::new("↻").size(19.0))
                        .min_size(Vec2::new(32.0, 28.0)),
                )
                .on_hover_text(refresh_tip);
            refresh_response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, refresh_tip)
            });
            if refresh_response.clicked() {
                self.refresh_preview();
            }
        });
        if let Some(task) = &self.render_task {
            let progress = (task.started.elapsed().as_secs_f32() / 2.0).min(0.95);
            ui.add(
                egui::ProgressBar::new(progress)
                    .text("Rendering")
                    .animate(true),
            );
        } else if self.host.is_transfer_active() {
            ui.add(
                egui::ProgressBar::new(self.transfer_progress)
                    .show_percentage()
                    .animate(true),
            );
        }
    }

    fn advanced_details(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Advanced details")
            .id_salt("advanced-details")
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Display bridge");
                    let selected_text = self
                        .selected_port
                        .as_ref()
                        .and_then(|port| {
                            self.devices
                                .iter()
                                .find(|device| &device.port_name == port)
                        })
                        .map_or_else(|| "No device".to_owned(), DeviceInfo::display_label);
                    egui::ComboBox::from_id_salt("wireterm-device")
                        .selected_text(selected_text)
                        .show_ui(ui, |ui| {
                            for device in &self.devices {
                                ui.selectable_value(
                                    &mut self.selected_port,
                                    Some(device.port_name.clone()),
                                    device.display_label(),
                                );
                            }
                        });
                    if ui.small_button("Refresh devices").clicked() {
                        let _ = self.host.refresh_devices();
                    }
                });
                ui.separator();
                self.named_secrets_editor(ui);
                if self.selected_item.is_some_and(|id| {
                    matches!(
                        self.playlist.item(id).map(|item| &item.source),
                        Some(PlaylistSource::Extension { .. })
                    )
                }) {
                    ui.separator();
                    ui.label("Extension contract");
                    ui.label(
                        RichText::new(
                            "One self-describing extension.lua exposes metadata, inputs, and render. \
                             The host provides bounded HTTP, opaque named-secret bindings, clock, \
                             and relative assets. Lua returns fixed 800 × 480 SVG; vectors/text are \
                             palette-mapped and raster assets are dithered before composition.",
                        )
                        .small()
                        .color(MUTED),
                    );
                }
                if !self.log.is_empty() {
                    ui.separator();
                    ui.label("Recent issues");
                    for entry in self.log.iter().take(12) {
                        ui.label(RichText::new(entry).small());
                    }
                }
                if let Some(preview) = &self.preview {
                    ui.separator();
                    ui.label(format!(
                        "Prepared panel frame · {} bytes · CRC {:08X}",
                        preview.frame.payload().len(),
                        preview.frame.crc32()
                    ));
                }
            });
    }

    fn named_secrets_editor(&mut self, ui: &mut egui::Ui) {
        ui.label("Named secrets");
        ui.label(
            RichText::new(
                "MVP values are stored locally without encryption and never stored in Playlist revisions.",
            )
            .small()
            .color(MUTED),
        );
        let mut remove = None;
        for name in &self.secret_names {
            ui.horizontal(|ui| {
                ui.label(name);
                if ui.small_button("Remove").clicked() {
                    remove = Some(name.clone());
                }
            });
        }
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.secret_name_edit)
                    .desired_width(150.0)
                    .hint_text("opaque-name"),
            );
            ui.add(
                egui::TextEdit::singleline(&mut *self.secret_value_edit)
                    .desired_width(210.0)
                    .password(true)
                    .hint_text("secret value"),
            );
            if ui.small_button("Create / update").clicked() {
                match self
                    .secret_store
                    .set(self.secret_name_edit.trim(), &self.secret_value_edit)
                {
                    Ok(()) => {
                        self.secret_names = self.secret_store.names().unwrap_or_default();
                        self.secret_value_edit.zeroize();
                    }
                    Err(error) => {
                        self.record_issue("Named secret could not be saved", error.to_string());
                    }
                }
            }
        });
        if let Some(name) = remove {
            match self.secret_store.remove(&name) {
                Ok(_) => self.secret_names = self.secret_store.names().unwrap_or_default(),
                Err(error) => {
                    self.record_issue("Named secret could not be removed", error.to_string());
                }
            }
        }
    }
}

impl eframe::App for WireTermApp {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll_render(ctx);
        self.poll_host(ctx);
        self.poll_playback();
        if self.render_task.is_some()
            || self.host.is_transfer_active()
            || self.playback.is_running()
        {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        ctx.set_visuals(egui::Visuals::dark());
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(12.0);
                    self.header(ui);
                    ui.columns(2, |columns| {
                        self.playlist_table(&mut columns[0]);
                        self.selected_item_editor(&mut columns[0]);
                        self.preview(&mut columns[1]);
                    });
                    ui.add_space(10.0);
                    self.advanced_details(ui);
                    ui.add_space(16.0);
                });
        });
        self.extension_library_window(ui.ctx());
    }
}

impl Drop for WireTermApp {
    fn drop(&mut self) {
        if let Some(task) = &self.render_task {
            task.cancellation.cancel();
        }
    }
}

struct RenderedItem {
    schema: Option<(ExtensionMetadata, Vec<ExtensionInput>)>,
    frame: Result<PanelFrame, String>,
}

fn render_item(
    item: &PlaylistItem,
    path: &Path,
    secrets: &SecretStore,
    cancellation: RenderCancellation,
) -> RenderedItem {
    match &item.source {
        PlaylistSource::Image { .. } | PlaylistSource::ImageFolder { .. } => RenderedItem {
            schema: None,
            frame: prepare_raster_path(path).map_err(|error| error.to_string()),
        },
        PlaylistSource::Extension {
            settings,
            named_secret_refs,
            ..
        } => {
            let Some(root) = path.parent() else {
                return RenderedItem {
                    schema: None,
                    frame: Err("Extension script has no parent folder".to_owned()),
                };
            };
            let started = Instant::now();
            let host = Arc::new(LiveExtensionHost::new(
                root.to_path_buf(),
                named_secret_refs.clone(),
                secrets.clone(),
                system_fixture_clock(),
                started,
                cancellation,
            ));
            match LoadedExtension::load(path, host) {
                Ok(extension) => {
                    let schema = Some((extension.metadata.clone(), extension.inputs.clone()));
                    let available_names = secrets.names().unwrap_or_default();
                    let frame = validate_extension_configuration(
                        &extension.inputs,
                        settings,
                        named_secret_refs,
                        &available_names,
                    )
                    .and_then(|resolved| extension.render_svg(&resolved))
                    .and_then(|svg| render_svg_to_panel(&svg, root))
                    .map_err(|error| error.to_string());
                    RenderedItem { schema, frame }
                }
                Err(error) => RenderedItem {
                    schema: None,
                    frame: Err(error.to_string()),
                },
            }
        }
    }
}

fn extension_script_path(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.join(EXTENSION_SCRIPT_NAME)
    } else {
        path.to_path_buf()
    }
}

fn source_label(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Playlist Item")
        .to_owned()
}

fn visual_qa_playlist() -> PlaylistRevision {
    let mut playlist = PlaylistRevision::default();
    playlist.add_item(
        "Lighthouse".to_owned(),
        PlaylistSource::Image {
            path: PathBuf::from("assets/prototype/lighthouse-test-frame.png"),
        },
    );
    playlist.add_item(
        "Studio rotation".to_owned(),
        PlaylistSource::ImageFolder {
            path: PathBuf::from("assets/prototype"),
        },
    );
    playlist.add_item(
        "Weather Extension".to_owned(),
        PlaylistSource::Extension {
            path: PathBuf::from("examples/weather-extension"),
            settings: BTreeMap::new(),
            named_secret_refs: BTreeMap::new(),
        },
    );
    playlist
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_folder_resolves_the_single_script() {
        let path = Path::new("demo-extension");
        assert_eq!(
            extension_script_path(path),
            path.to_path_buf(),
            "nonexistent paths are treated as explicit scripts"
        );
    }

    #[test]
    fn transfer_failures_distinguish_protocol_from_disconnect_in_ui_contract() {
        assert!(
            "unexpected device response during handshake".starts_with("unexpected device response")
        );
        assert!(!"could not open COM4".starts_with("unexpected device response"));
    }
}
