#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    process::{Command, Output},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use eframe::egui::{self, Color32, RichText, Stroke};

const ACCENT: Color32 = Color32::from_rgb(91, 140, 255);
const GOOD: Color32 = Color32::from_rgb(91, 196, 131);
const WARNING: Color32 = Color32::from_rgb(242, 177, 77);
const DANGER: Color32 = Color32::from_rgb(239, 101, 101);
const AUTO_REFRESH_INTERVAL: Duration = Duration::from_secs(8);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Volume {
    name: String,
    device: String,
    whole_disk: String,
    mount_point: String,
    filesystem: String,
    protocol: String,
    size: String,
    free_space: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Holder {
    pid: u32,
    command: String,
    executable: String,
    user: String,
    paths: Vec<String>,
}

#[derive(Clone, Debug, Default)]
struct VolumeState {
    volume: Volume,
    holders: Vec<Holder>,
}

#[derive(Clone, Debug, Default)]
struct Snapshot {
    volumes: Vec<VolumeState>,
}

enum Request {
    Scan,
    Eject(Volume),
    ResolveAndEject { volume: Volume, force: bool },
}

struct Response {
    snapshot: Result<Snapshot, String>,
    notice: Option<Result<String, String>>,
}

#[derive(Clone)]
enum Confirmation {
    QuitAndEject(Volume, Vec<Holder>),
    ForceQuitAndEject(Volume, Vec<Holder>),
}

struct CardScoutApp {
    request_tx: Sender<Request>,
    response_rx: Receiver<Response>,
    snapshot: Snapshot,
    selected_device: Option<String>,
    busy: bool,
    last_scan: Option<Instant>,
    notice: Option<Result<String, String>>,
    confirmation: Option<Confirmation>,
}

impl CardScoutApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(15, 17, 21);
        visuals.window_fill = Color32::from_rgb(23, 26, 32);
        visuals.extreme_bg_color = Color32::from_rgb(10, 12, 15);
        visuals.faint_bg_color = Color32::from_rgb(27, 30, 37);
        visuals.selection.bg_fill = ACCENT;
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(35, 39, 47);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(46, 51, 62);
        cc.egui_ctx.set_visuals(visuals);

        let (request_tx, request_rx) = mpsc::channel();
        let (response_tx, response_rx) = mpsc::channel();
        thread::Builder::new()
            .name("card-scout-worker".into())
            .spawn(move || worker(request_rx, response_tx))
            .expect("failed to start Card Scout worker");

        let app = Self {
            request_tx,
            response_rx,
            snapshot: Snapshot::default(),
            selected_device: None,
            busy: true,
            last_scan: None,
            notice: None,
            confirmation: None,
        };
        let _ = app.request_tx.send(Request::Scan);
        app
    }

    fn poll_worker(&mut self) {
        while let Ok(response) = self.response_rx.try_recv() {
            self.busy = false;
            match response.snapshot {
                Ok(snapshot) => {
                    let selected_exists = self.selected_device.as_ref().is_some_and(|device| {
                        snapshot
                            .volumes
                            .iter()
                            .any(|state| &state.volume.device == device)
                    });
                    self.snapshot = snapshot;
                    if !selected_exists {
                        self.selected_device = self
                            .snapshot
                            .volumes
                            .first()
                            .map(|state| state.volume.device.clone());
                    }
                    self.last_scan = Some(Instant::now());
                }
                Err(error) => self.notice = Some(Err(error)),
            }
            if let Some(notice) = response.notice {
                self.notice = Some(notice);
            }
        }
    }

    fn selected(&self) -> Option<&VolumeState> {
        let device = self.selected_device.as_ref()?;
        self.snapshot
            .volumes
            .iter()
            .find(|state| &state.volume.device == device)
    }

    fn send(&mut self, request: Request) {
        if !self.busy && self.request_tx.send(request).is_ok() {
            self.busy = true;
            self.notice = None;
        }
    }

    fn refresh(&mut self) {
        self.send(Request::Scan);
    }

    fn auto_refresh_if_due(&mut self) {
        let due = self
            .last_scan
            .is_some_and(|last_scan| last_scan.elapsed() >= AUTO_REFRESH_INTERVAL);
        if !self.busy
            && self.confirmation.is_none()
            && due
            && self.request_tx.send(Request::Scan).is_ok()
        {
            self.busy = true;
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .fill(Color32::from_rgb(20, 23, 28))
            .inner_margin(egui::Margin::symmetric(22, 16))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Card Scout").size(24.0).strong());
                        ui.label(
                            RichText::new(
                                "See what is holding a removable disk, then eject it safely.",
                            )
                            .color(Color32::from_gray(158)),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let button = ui.add_enabled(!self.busy, egui::Button::new("↻  Refresh"));
                        if button.clicked() {
                            self.refresh();
                        }
                        if self.busy {
                            ui.spinner();
                            ui.label(
                                RichText::new("Checking disks…").color(Color32::from_gray(150)),
                            );
                        } else if let Some(when) = self.last_scan {
                            ui.label(
                                RichText::new(format!("Updated {}s ago", when.elapsed().as_secs()))
                                    .small()
                                    .color(Color32::from_gray(125)),
                            );
                        }
                    });
                });
            });
    }

    fn volume_picker(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("REMOVABLE DISK")
                .small()
                .strong()
                .color(Color32::from_gray(140)),
        );
        ui.add_space(6.0);

        let selected_label = self
            .selected()
            .map(|state| format!("{}  ·  {}", state.volume.name, state.volume.size))
            .unwrap_or_else(|| "No removable disks found".into());

        ui.add_enabled_ui(!self.busy && !self.snapshot.volumes.is_empty(), |ui| {
            egui::ComboBox::from_id_salt("volume-picker")
                .selected_text(selected_label)
                .width(ui.available_width().min(520.0))
                .show_ui(ui, |ui| {
                    for state in &self.snapshot.volumes {
                        let label = format!(
                            "{}  —  {} ({})",
                            state.volume.name, state.volume.size, state.volume.device
                        );
                        ui.selectable_value(
                            &mut self.selected_device,
                            Some(state.volume.device.clone()),
                            label,
                        );
                    }
                });
        });
    }

    fn disk_summary(&self, ui: &mut egui::Ui, state: &VolumeState) {
        egui::Frame::new()
            .fill(Color32::from_rgb(25, 28, 34))
            .stroke(Stroke::new(1.0, Color32::from_rgb(48, 53, 63)))
            .corner_radius(10.0)
            .inner_margin(egui::Margin::same(16))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("▣").size(30.0).color(ACCENT));
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&state.volume.name).size(18.0).strong());
                        ui.label(
                            RichText::new(format!(
                                "{}  ·  {}  ·  {} free  ·  {}",
                                state.volume.device,
                                state.volume.filesystem,
                                state.volume.free_space,
                                state.volume.protocol
                            ))
                            .color(Color32::from_gray(145)),
                        );
                        ui.label(
                            RichText::new(&state.volume.mount_point)
                                .monospace()
                                .small()
                                .color(Color32::from_gray(120)),
                        );
                    });
                });
            });
    }

    fn blockers(&self, ui: &mut egui::Ui, state: &VolumeState) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("OPEN FILES")
                    .small()
                    .strong()
                    .color(Color32::from_gray(140)),
            );
            if state.holders.is_empty() {
                ui.label(RichText::new("READY TO EJECT").small().strong().color(GOOD));
            } else {
                ui.label(
                    RichText::new(format!(
                        "{} PROCESS{}",
                        state.holders.len(),
                        if state.holders.len() == 1 { "" } else { "ES" }
                    ))
                    .small()
                    .strong()
                    .color(WARNING),
                );
            }
        });
        ui.add_space(8.0);

        if state.holders.is_empty() {
            egui::Frame::new()
                .fill(Color32::from_rgb(20, 38, 29))
                .stroke(Stroke::new(1.0, Color32::from_rgb(45, 91, 62)))
                .corner_radius(10.0)
                .inner_margin(egui::Margin::same(18))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("✓  No application is holding files on this disk.")
                            .color(GOOD),
                    );
                });
            return;
        }

        for holder in &state.holders {
            egui::Frame::new()
                .fill(Color32::from_rgb(31, 29, 27))
                .stroke(Stroke::new(1.0, Color32::from_rgb(75, 61, 42)))
                .corner_radius(10.0)
                .inner_margin(egui::Margin::same(14))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("●").color(WARNING));
                        ui.label(RichText::new(&holder.command).size(17.0).strong());
                        ui.label(
                            RichText::new(format!("PID {}  ·  {}", holder.pid, holder.user))
                                .monospace()
                                .small()
                                .color(Color32::from_gray(140)),
                        );
                    });
                    if !holder.executable.is_empty() {
                        ui.label(
                            RichText::new(&holder.executable)
                                .monospace()
                                .small()
                                .color(Color32::from_gray(125)),
                        );
                    }
                    ui.add_space(5.0);
                    for path in holder.paths.iter().take(4) {
                        let relative = path.strip_prefix(&state.volume.mount_point).unwrap_or(path);
                        ui.label(
                            RichText::new(format!("  ↳ {}", relative.trim_start_matches('/')))
                                .monospace()
                                .small()
                                .color(Color32::from_gray(170)),
                        );
                    }
                    if holder.paths.len() > 4 {
                        ui.label(
                            RichText::new(format!(
                                "     + {} more open files",
                                holder.paths.len() - 4
                            ))
                            .small()
                            .color(Color32::from_gray(130)),
                        );
                    }
                });
            ui.add_space(8.0);
        }
    }

    fn action_bar(&mut self, ui: &mut egui::Ui) {
        let selected = self.selected().cloned();
        let Some(state) = selected else {
            return;
        };

        ui.horizontal(|ui| {
            let eject = ui.add_enabled(
                !self.busy,
                egui::Button::new("Eject Safely").min_size([120.0, 34.0].into()),
            );
            if eject.clicked() {
                self.send(Request::Eject(state.volume.clone()));
            }

            if !state.holders.is_empty() {
                let resolve = ui.add_enabled(
                    !self.busy,
                    egui::Button::new("Quit Blockers & Eject…")
                        .fill(Color32::from_rgb(55, 83, 155))
                        .min_size([180.0, 34.0].into()),
                );
                if resolve.clicked() {
                    self.confirmation = Some(Confirmation::QuitAndEject(
                        state.volume.clone(),
                        state.holders.clone(),
                    ));
                }

                let force = ui.add_enabled(
                    !self.busy,
                    egui::Button::new("Force Quit & Eject…")
                        .fill(Color32::from_rgb(94, 43, 43))
                        .min_size([160.0, 34.0].into()),
                );
                if force.clicked() {
                    self.confirmation = Some(Confirmation::ForceQuitAndEject(
                        state.volume.clone(),
                        state.holders.clone(),
                    ));
                }
            }
        });
        ui.add_space(5.0);
        ui.label(
            RichText::new("Safe eject never quits apps. Quit Blockers sends a normal quit signal; Force Quit may lose unsaved work.")
                .small()
                .color(Color32::from_gray(125)),
        );
    }

    fn confirmation_window(&mut self, ctx: &egui::Context) {
        let Some(confirmation) = self.confirmation.clone() else {
            return;
        };
        let (volume, holders, force) = match &confirmation {
            Confirmation::QuitAndEject(volume, holders) => (volume, holders, false),
            Confirmation::ForceQuitAndEject(volume, holders) => (volume, holders, true),
        };
        let title = if force {
            "Force quit applications?"
        } else {
            "Quit applications?"
        };
        let mut close = false;

        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.label(if force {
                    format!("Card Scout will force quit these processes, then eject {}. Unsaved work may be lost.", volume.name)
                } else {
                    format!("Card Scout will ask these processes to quit, then eject {}.", volume.name)
                });
                ui.add_space(8.0);
                for holder in holders {
                    ui.label(format!("• {} (PID {})", holder.command, holder.pid));
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    let label = if force { "Force Quit & Eject" } else { "Quit & Eject" };
                    let color = if force { DANGER } else { ACCENT };
                    if ui.add(egui::Button::new(label).fill(color)).clicked() {
                        self.send(Request::ResolveAndEject {
                            volume: volume.clone(),
                            force,
                        });
                        close = true;
                    }
                });
            });
        if close {
            self.confirmation = None;
        }
    }
}

impl eframe::App for CardScoutApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        self.poll_worker();
        self.auto_refresh_if_due();
        if self.busy {
            ctx.request_repaint_after(Duration::from_millis(100));
        } else {
            ctx.request_repaint_after(Duration::from_secs(1));
        }

        egui::Panel::top("top").show(root, |ui| self.top_bar(ui));
        egui::CentralPanel::default().show(root, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.set_max_width(920.0);
                ui.add_space(18.0);
                self.volume_picker(ui);
                ui.add_space(16.0);

                if self.snapshot.volumes.is_empty() && !self.busy {
                    egui::Frame::new()
                        .fill(Color32::from_rgb(25, 28, 34))
                        .corner_radius(12.0)
                        .inner_margin(egui::Margin::same(28))
                        .show(ui, |ui| {
                            ui.heading("No removable disk is mounted");
                            ui.label(
                                RichText::new("Insert a camera card or connect an external drive, then press Refresh.")
                                    .color(Color32::from_gray(155)),
                            );
                        });
                }

                if let Some(state) = self.selected().cloned() {
                    self.disk_summary(ui, &state);
                    ui.add_space(20.0);
                    self.blockers(ui, &state);
                    ui.add_space(12.0);
                    self.action_bar(ui);
                }

                if let Some(notice) = &self.notice {
                    ui.add_space(16.0);
                    let (text, color) = match notice {
                        Ok(message) => (format!("✓  {message}"), GOOD),
                        Err(message) => (format!("Could not complete the action: {message}"), DANGER),
                    };
                    egui::Frame::new()
                        .fill(color.gamma_multiply(0.13))
                        .stroke(Stroke::new(1.0, color.gamma_multiply(0.55)))
                        .corner_radius(8.0)
                        .inner_margin(egui::Margin::same(12))
                        .show(ui, |ui| {
                            ui.label(RichText::new(text).color(color));
                        });
                }
                ui.add_space(24.0);
            });
        });
        self.confirmation_window(&ctx);
    }
}

fn worker(request_rx: Receiver<Request>, response_tx: Sender<Response>) {
    while let Ok(request) = request_rx.recv() {
        let response = match request {
            Request::Scan => Response {
                snapshot: scan_system(),
                notice: None,
            },
            Request::Eject(volume) => {
                let notice = eject(&volume).map(|_| format!("{} was ejected safely.", volume.name));
                Response {
                    snapshot: scan_system(),
                    notice: Some(notice),
                }
            }
            Request::ResolveAndEject { volume, force } => {
                let notice = resolve_and_eject(&volume, force);
                Response {
                    snapshot: scan_system(),
                    notice: Some(notice),
                }
            }
        };
        if response_tx.send(response).is_err() {
            break;
        }
    }
}

fn scan_system() -> Result<Snapshot, String> {
    #[cfg(not(target_os = "macos"))]
    return Err("Card Scout requires macOS.".into());

    #[cfg(target_os = "macos")]
    {
        let volumes = mounted_external_volumes()?;
        let holders = holders_for_volumes(&volumes)?;
        let states = volumes
            .iter()
            .map(|volume| VolumeState {
                holders: holders_for_disk(volume, &volumes, &holders),
                volume: volume.clone(),
            })
            .collect();
        Ok(Snapshot { volumes: states })
    }
}

fn mounted_external_volumes() -> Result<Vec<Volume>, String> {
    let output = run("/sbin/mount", &[])?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut volumes = Vec::new();

    for (device, mount_point) in parse_mounts(&text) {
        let info_output = run("/usr/sbin/diskutil", &["info", &device])?;
        if !info_output.status.success() {
            continue;
        }
        let info = parse_diskutil_info(&String::from_utf8_lossy(&info_output.stdout));
        let external = info
            .get("Device Location")
            .is_some_and(|value| value == "External");
        let removable = info
            .get("Removable Media")
            .is_some_and(|value| value == "Removable");
        let disk_image = info
            .get("Protocol")
            .is_some_and(|value| value == "Disk Image");
        if (!external && !removable) || disk_image {
            continue;
        }

        let whole_disk = info
            .get("Part of Whole")
            .or_else(|| info.get("Device Identifier"))
            .map(|value| format!("/dev/{value}"))
            .unwrap_or_else(|| device.clone());
        let name = info
            .get("Volume Name")
            .filter(|value| !value.eq_ignore_ascii_case("Not applicable"))
            .cloned()
            .or_else(|| {
                Path::new(&mount_point)
                    .file_name()
                    .map(|name| name.to_string_lossy().into())
            })
            .unwrap_or_else(|| device.clone());

        volumes.push(Volume {
            name,
            device,
            whole_disk,
            mount_point,
            filesystem: info
                .get("Name (User Visible)")
                .or_else(|| info.get("File System Personality"))
                .cloned()
                .unwrap_or_else(|| "Unknown format".into()),
            protocol: info
                .get("Protocol")
                .cloned()
                .unwrap_or_else(|| "External".into()),
            size: concise_size(info.get("Disk Size")),
            free_space: concise_size(info.get("Volume Free Space")),
        });
    }
    volumes.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    Ok(volumes)
}

fn parse_mounts(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let (device, rest) = line.split_once(" on ")?;
            let (mount_point, _) = rest.rsplit_once(" (")?;
            (device.starts_with("/dev/disk") && mount_point.starts_with("/Volumes/"))
                .then(|| (device.to_owned(), mount_point.to_owned()))
        })
        .collect()
}

fn parse_diskutil_info(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.trim().split_once(':')?;
            Some((key.trim().to_owned(), value.trim().to_owned()))
        })
        .collect()
}

fn concise_size(value: Option<&String>) -> String {
    value
        .and_then(|value| {
            value
                .split_once(" (")
                .map(|(human, _)| human.trim().to_owned())
        })
        .or_else(|| value.cloned())
        .unwrap_or_else(|| "Unknown".into())
}

fn holders_for_volumes(volumes: &[Volume]) -> Result<BTreeMap<String, Vec<Holder>>, String> {
    let mut result: BTreeMap<String, BTreeMap<u32, Holder>> = volumes
        .iter()
        .map(|volume| (volume.mount_point.clone(), BTreeMap::new()))
        .collect();
    if volumes.is_empty() {
        return Ok(BTreeMap::new());
    }

    let output = run("/usr/sbin/lsof", &["-nP", "-FpcLn"])?;
    // lsof returns 1 when it found nothing, so its exit status is not useful here.
    let records = parse_lsof(&String::from_utf8_lossy(&output.stdout));
    let self_pid = std::process::id();

    for record in records {
        if record.pid == self_pid {
            continue;
        }
        for volume in volumes {
            let prefix = format!("{}/", volume.mount_point.trim_end_matches('/'));
            let matching: BTreeSet<_> = record
                .paths
                .iter()
                .filter(|path| *path == &volume.mount_point || path.starts_with(&prefix))
                .cloned()
                .collect();
            if matching.is_empty() {
                continue;
            }

            let executable = process_executable(record.pid);
            let command = display_name(&record.command, &executable);
            let holder = result
                .get_mut(&volume.mount_point)
                .expect("volume map initialized")
                .entry(record.pid)
                .or_insert_with(|| Holder {
                    pid: record.pid,
                    command,
                    executable,
                    user: record.user.clone(),
                    paths: Vec::new(),
                });
            holder.paths.extend(matching);
            holder.paths.sort();
            holder.paths.dedup();
        }
    }

    Ok(result
        .into_iter()
        .map(|(mount, holders)| (mount, holders.into_values().collect()))
        .collect())
}

fn holders_for_disk(
    selected: &Volume,
    volumes: &[Volume],
    holders_by_mount: &BTreeMap<String, Vec<Holder>>,
) -> Vec<Holder> {
    let mut merged: BTreeMap<u32, Holder> = BTreeMap::new();
    for sibling in volumes
        .iter()
        .filter(|volume| volume.whole_disk == selected.whole_disk)
    {
        for holder in holders_by_mount
            .get(&sibling.mount_point)
            .into_iter()
            .flatten()
        {
            let merged_holder = merged.entry(holder.pid).or_insert_with(|| Holder {
                pid: holder.pid,
                command: holder.command.clone(),
                executable: holder.executable.clone(),
                user: holder.user.clone(),
                paths: Vec::new(),
            });
            merged_holder.paths.extend(holder.paths.iter().cloned());
            merged_holder.paths.sort();
            merged_holder.paths.dedup();
        }
    }
    merged.into_values().collect()
}

#[derive(Default)]
struct LsofRecord {
    pid: u32,
    command: String,
    user: String,
    paths: Vec<String>,
}

fn parse_lsof(text: &str) -> Vec<LsofRecord> {
    let mut records = Vec::new();
    let mut current: Option<LsofRecord> = None;

    for line in text.lines() {
        let Some((field, value)) = line.split_at_checked(1) else {
            continue;
        };
        match field {
            "p" => {
                if let Some(record) = current.take() {
                    records.push(record);
                }
                current = value.parse().ok().map(|pid| LsofRecord {
                    pid,
                    ..Default::default()
                });
            }
            "c" => {
                if let Some(record) = &mut current {
                    record.command = value.to_owned();
                }
            }
            "L" => {
                if let Some(record) = &mut current {
                    record.user = value.to_owned();
                }
            }
            "n" => {
                if let Some(record) = &mut current {
                    record.paths.push(value.to_owned());
                }
            }
            _ => {}
        }
    }
    if let Some(record) = current {
        records.push(record);
    }
    records
}

fn process_executable(pid: u32) -> String {
    run("/bin/ps", &["-ww", "-p", &pid.to_string(), "-o", "comm="])
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default()
}

fn display_name(lsof_command: &str, executable: &str) -> String {
    executable
        .split("/Contents/MacOS/")
        .next()
        .and_then(|bundle| bundle.rsplit('/').next())
        .and_then(|name| name.strip_suffix(".app"))
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            Path::new(executable)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| lsof_command.to_owned())
}

fn resolve_and_eject(volume: &Volume, force: bool) -> Result<String, String> {
    let mounted = mounted_external_volumes()?;
    let siblings: Vec<_> = mounted
        .into_iter()
        .filter(|mounted_volume| mounted_volume.whole_disk == volume.whole_disk)
        .collect();
    let volumes = if siblings.is_empty() {
        vec![volume.clone()]
    } else {
        siblings
    };
    let holders_by_mount = holders_for_volumes(&volumes)?;
    let holders = holders_for_disk(volume, &volumes, &holders_by_mount);
    let signal = if force { "-KILL" } else { "-TERM" };
    let self_pid = std::process::id();
    let mut failures = Vec::new();

    for holder in &holders {
        if holder.pid <= 1 || holder.pid == self_pid {
            continue;
        }
        match run("/bin/kill", &[signal, &holder.pid.to_string()]) {
            Ok(output) if output.status.success() => {}
            Ok(output) => failures.push(format!(
                "{} (PID {}): {}",
                holder.command,
                holder.pid,
                output_message(&output)
            )),
            Err(error) => {
                failures.push(format!("{} (PID {}): {error}", holder.command, holder.pid))
            }
        }
    }

    thread::sleep(if force {
        Duration::from_millis(400)
    } else {
        Duration::from_secs(2)
    });
    match eject(volume) {
        Ok(()) => Ok(format!(
            "{} was ejected after {} {} process{}.",
            volume.name,
            if force { "force quitting" } else { "quitting" },
            holders.len(),
            if holders.len() == 1 { "" } else { "es" }
        )),
        Err(eject_error) if !failures.is_empty() => Err(format!(
            "{eject_error}. Could not stop: {}",
            failures.join("; ")
        )),
        Err(error) => Err(error),
    }
}

fn eject(volume: &Volume) -> Result<(), String> {
    let output = run("/usr/sbin/diskutil", &["eject", &volume.whole_disk])?;
    if output.status.success() {
        Ok(())
    } else {
        Err(output_message(&output))
    }
}

fn run(program: &str, arguments: &[&str]) -> Result<Output, String> {
    Command::new(program)
        .args(arguments)
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .output()
        .map_err(|error| format!("failed to run {program}: {error}"))
}

fn output_message(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!("command exited with {}", output.status)
    }
}

fn main() -> eframe::Result {
    #[cfg(not(target_os = "macos"))]
    eprintln!("Card Scout requires macOS.");

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("Card Scout")
            .with_inner_size([820.0, 680.0])
            .with_min_inner_size([650.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Card Scout",
        options,
        Box::new(|cc| Ok(Box::new(CardScoutApp::new(cc)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mounted_volume_paths_with_spaces() {
        let mounts = parse_mounts(
            "/dev/disk5s1 on /Volumes/camera (exfat, local)\n/dev/disk4s2 on /Volumes/My Card (hfs, local)\nmap auto_home on /System/Volumes/Data/home (autofs)\n",
        );
        assert_eq!(
            mounts,
            vec![
                ("/dev/disk5s1".into(), "/Volumes/camera".into()),
                ("/dev/disk4s2".into(), "/Volumes/My Card".into()),
            ]
        );
    }

    #[test]
    fn parses_diskutil_fields_and_human_size() {
        let info = parse_diskutil_info(
            "   Volume Name:               CAMERA\n   Protocol:                  USB\n   Disk Size:                 128.0 GB (128000000000 Bytes)\n",
        );
        assert_eq!(info["Volume Name"], "CAMERA");
        assert_eq!(info["Protocol"], "USB");
        assert_eq!(concise_size(info.get("Disk Size")), "128.0 GB");
    }

    #[test]
    fn groups_lsof_paths_by_process() {
        let records = parse_lsof(
            "p77992\ncAirDrop\nLrachel\nf12\nn/Volumes/camera/DCIM/one.ARW\nf13\nn/Volumes/camera/DCIM/two.ARW\np900\ncFinder\nLrachel\nf9\nn/Volumes/camera\n",
        );
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].pid, 77992);
        assert_eq!(records[0].command, "AirDrop");
        assert_eq!(records[0].paths.len(), 2);
        assert_eq!(records[1].user, "rachel");
    }

    #[test]
    fn shows_app_bundle_name() {
        assert_eq!(
            display_name(
                "AirDrop",
                "/System/Applications/Utilities/AirDrop.app/Contents/MacOS/AirDrop"
            ),
            "AirDrop"
        );
        assert_eq!(
            display_name("mdworker", "/usr/libexec/mdworker_shared"),
            "mdworker_shared"
        );
    }

    #[test]
    fn merges_blockers_from_every_volume_on_the_same_disk() {
        let first = Volume {
            device: "/dev/disk5s1".into(),
            whole_disk: "/dev/disk5".into(),
            mount_point: "/Volumes/PHOTOS".into(),
            ..Default::default()
        };
        let second = Volume {
            device: "/dev/disk5s2".into(),
            whole_disk: "/dev/disk5".into(),
            mount_point: "/Volumes/VIDEO".into(),
            ..Default::default()
        };
        let unrelated = Volume {
            device: "/dev/disk6s1".into(),
            whole_disk: "/dev/disk6".into(),
            mount_point: "/Volumes/BACKUP".into(),
            ..Default::default()
        };
        let mut holders = BTreeMap::new();
        holders.insert(
            first.mount_point.clone(),
            vec![Holder {
                pid: 10,
                paths: vec!["/Volumes/PHOTOS/a.jpg".into()],
                ..Default::default()
            }],
        );
        holders.insert(
            second.mount_point.clone(),
            vec![Holder {
                pid: 10,
                paths: vec!["/Volumes/VIDEO/a.mov".into()],
                ..Default::default()
            }],
        );
        holders.insert(
            unrelated.mount_point.clone(),
            vec![Holder {
                pid: 20,
                paths: vec!["/Volumes/BACKUP/data".into()],
                ..Default::default()
            }],
        );

        let merged = holders_for_disk(&first, &[first.clone(), second, unrelated], &holders);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].pid, 10);
        assert_eq!(merged[0].paths.len(), 2);
    }
}
