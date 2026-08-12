use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use eframe::egui::{
    self, Color32, FontId, Mesh, PointerButton, Pos2, Rect, Response, RichText, Sense,
    TextureHandle, TextureOptions, Vec2, vec2,
};
use eframe::epaint::Vertex;
use egui_phosphor::regular as icons;

use crate::{
    cache::{CachedPreview, PreviewCache},
    catalog::Catalog,
    detail::{DetailImage, DetailLoader},
    loader::{DecodedImage, ImageLoader, RequestKind},
    view::ViewTransform,
};

const CACHE_BUDGET: usize = 512 * 1024 * 1024;
const TILE_SIZE: u32 = 512;
const MAX_GPU_TILES: usize = 256;
const TOOLBAR_BUTTON_HEIGHT: f32 = 34.0;

fn toolbar_icon_button(ui: &mut egui::Ui, icon: &'static str, enabled: bool) -> Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(icon).size(20.0))
            .min_size(vec2(38.0, TOOLBAR_BUTTON_HEIGHT)),
    )
}

fn open_button(ui: &mut egui::Ui) -> Response {
    ui.add_sized(
        vec2(100.0, TOOLBAR_BUTTON_HEIGHT),
        egui::Button::new(RichText::new(format!("{}  Open", icons::FOLDER_OPEN)).size(15.0)),
    )
}

fn viewport_zoom_factor(gesture_zoom: f32, scroll: Vec2) -> Option<f32> {
    if (gesture_zoom - 1.0).abs() > f32::EPSILON {
        return Some(gesture_zoom);
    }
    let wheel_delta = if scroll.y.abs() >= scroll.x.abs() {
        scroll.y
    } else {
        scroll.x
    };
    (wheel_delta.abs() > f32::EPSILON).then(|| (wheel_delta * 0.01).clamp(-1.0, 1.0).exp())
}

fn apply_appearance(ctx: &egui::Context, dark_mode: bool) {
    let mut visuals = if dark_mode {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    if dark_mode {
        visuals.panel_fill = Color32::from_rgb(13, 14, 17);
        visuals.extreme_bg_color = Color32::from_rgb(8, 9, 11);
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(31, 32, 36);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(58, 35, 39);
        visuals.widgets.active.bg_fill = Color32::from_rgb(166, 24, 38);
    } else {
        visuals.panel_fill = Color32::from_rgb(244, 244, 246);
        visuals.extreme_bg_color = Color32::from_rgb(222, 223, 227);
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(255, 255, 255);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(255, 226, 230);
        visuals.widgets.active.bg_fill = Color32::from_rgb(221, 48, 63);
    }
    visuals.window_fill = visuals.panel_fill;
    visuals.selection.bg_fill = Color32::from_rgb(202, 32, 48);
    visuals.selection.stroke.color = Color32::WHITE;
    visuals.widgets.hovered.bg_stroke.color = Color32::from_rgb(231, 58, 72);

    ctx.set_theme(if dark_mode {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
    ctx.set_visuals(visuals);
    ctx.all_styles_mut(|style| {
        style.interaction.tooltip_delay = 0.0;
        style.interaction.tooltip_grace_time = 0.0;
        style.interaction.show_tooltips_only_when_still = false;
    });
}

pub struct EagleApp {
    brand_icon: TextureHandle,
    loader: ImageLoader,
    detail_loader: DetailLoader,
    catalog: Catalog,
    cache: PreviewCache,
    current: Option<Arc<DecodedImage>>,
    texture: Option<TextureHandle>,
    detail: Option<Arc<DetailImage>>,
    detail_loading: bool,
    detail_error: Option<String>,
    tiles: HashMap<(u32, u32), TextureHandle>,
    tile_order: VecDeque<(u32, u32)>,
    generation: u64,
    loading: bool,
    error: Option<String>,
    view: ViewTransform,
    rotation: u8,
    flip_horizontal: bool,
    flip_vertical: bool,
    show_info: bool,
    show_filmstrip: bool,
    slideshow: bool,
    dark_mode: bool,
    last_slide: Instant,
    fullscreen: bool,
    last_viewport: Option<Rect>,
    last_frame: Instant,
    screenshot_path: Option<PathBuf>,
    screenshot_requested: bool,
    screenshot_settle_frames: u8,
}

impl EagleApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        initial_path: Option<PathBuf>,
        screenshot_path: Option<PathBuf>,
        forced_dark_mode: Option<bool>,
    ) -> Self {
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        cc.egui_ctx.set_fonts(fonts);

        let dark_mode = forced_dark_mode.unwrap_or_else(|| {
            cc.egui_ctx
                .input(|input| !matches!(input.raw.system_theme, Some(egui::Theme::Light)))
        });
        apply_appearance(&cc.egui_ctx, dark_mode);

        let icon = image::load_from_memory(include_bytes!("../assets/EagleViewerIcon.png"))
            .expect("bundled Eagle Viewer icon is invalid")
            .resize_exact(64, 64, image::imageops::FilterType::Lanczos3)
            .into_rgba8();
        let brand_icon = cc.egui_ctx.load_texture(
            "eagle-viewer-brand",
            egui::ColorImage::from_rgba_unmultiplied([64, 64], icon.as_raw()),
            TextureOptions::LINEAR,
        );

        let mut app = Self {
            brand_icon,
            loader: ImageLoader::new(),
            detail_loader: DetailLoader::new(),
            catalog: Catalog::default(),
            cache: PreviewCache::new(CACHE_BUDGET),
            current: None,
            texture: None,
            detail: None,
            detail_loading: false,
            detail_error: None,
            tiles: HashMap::new(),
            tile_order: VecDeque::new(),
            generation: 0,
            loading: false,
            error: None,
            view: ViewTransform::default(),
            rotation: 0,
            flip_horizontal: false,
            flip_vertical: false,
            show_info: false,
            show_filmstrip: true,
            slideshow: false,
            dark_mode,
            last_slide: Instant::now(),
            fullscreen: false,
            last_viewport: None,
            last_frame: Instant::now(),
            screenshot_path,
            screenshot_requested: false,
            screenshot_settle_frames: 0,
        };
        if let Some(path) = initial_path.filter(|path| path.is_file()) {
            app.open(path);
        }
        app
    }

    fn process_screenshot(&mut self, ctx: &egui::Context) {
        let captured = ctx.input(|input| {
            input.events.iter().find_map(|event| {
                if let egui::Event::Screenshot { image, .. } = event {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        let Some(image) = captured else {
            return;
        };
        let Some(path) = self.screenshot_path.take() else {
            return;
        };
        let rgba: Vec<u8> = image
            .pixels
            .iter()
            .flat_map(|pixel| pixel.to_array())
            .collect();
        if let Err(error) = image::save_buffer(
            &path,
            &rgba,
            image.size[0] as u32,
            image.size[1] as u32,
            image::ColorType::Rgba8,
        ) {
            eprintln!("could not save screenshot to {}: {error}", path.display());
        } else {
            println!("saved screenshot to {}", path.display());
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn open(&mut self, path: PathBuf) {
        self.catalog = Catalog::around(&path);
        self.load_current();
    }

    fn load_current(&mut self) {
        let Some(path) = self.catalog.current().map(ToOwned::to_owned) else {
            return;
        };
        self.generation = self.generation.wrapping_add(1);
        self.error = None;
        self.view.reset();
        self.rotation = 0;
        self.flip_horizontal = false;
        self.flip_vertical = false;
        self.clear_detail();

        if let Some(image) = self.cache.get(&path) {
            self.install_cached(image);
            self.loading = false;
        } else {
            self.loading = true;
            self.loader.request_current(path, self.generation);
        }
        self.prefetch_neighbors();
    }

    fn request_detail(&mut self, path: PathBuf) {
        if self.slideshow {
            return;
        }
        self.detail_loading = true;
        self.detail_error = None;
        self.detail_loader.request(path, self.generation);
    }

    fn clear_detail(&mut self) {
        self.detail = None;
        self.detail_loading = false;
        self.detail_error = None;
        self.tiles.clear();
        self.tile_order.clear();
    }

    fn move_by(&mut self, delta: isize) {
        let before = self.catalog.position();
        self.catalog.move_by(delta);
        if before != self.catalog.position() {
            self.load_current();
        }
    }

    fn prefetch_neighbors(&self) {
        for path in self.catalog.neighbors() {
            if !self.cache.contains(path) {
                self.loader
                    .request_prefetch(path.to_path_buf(), self.generation);
            }
        }
    }

    fn process_results(&mut self, ctx: &egui::Context) {
        while let Some(message) = self.loader.try_recv() {
            match message.result {
                Ok(image) => {
                    let image = Arc::new(image);
                    let preview = self.prepare(image, ctx);
                    let is_current = self
                        .catalog
                        .current()
                        .is_some_and(|path| path == message.path);
                    self.cache.insert(preview.clone(), self.catalog.current());
                    if is_current
                        && (message.kind == RequestKind::Prefetch
                            || message.generation == self.generation)
                    {
                        self.install_cached(preview);
                        self.loading = false;
                        self.error = None;
                    }
                }
                Err(error) => {
                    let is_current = self
                        .catalog
                        .current()
                        .is_some_and(|path| path == message.path);
                    if is_current && message.generation == self.generation {
                        self.loading = false;
                        self.error = Some(error);
                    }
                }
            }
        }
    }

    fn process_detail_results(&mut self) {
        while let Some(message) = self.detail_loader.try_recv() {
            let is_current = self
                .catalog
                .current()
                .is_some_and(|path| path == message.path);
            if !is_current || message.generation != self.generation {
                continue;
            }
            self.detail_loading = false;
            match message.result {
                Ok(detail) => {
                    self.detail = Some(detail);
                    self.detail_error = None;
                    self.tiles.clear();
                    self.tile_order.clear();
                }
                Err(error) => self.detail_error = Some(error),
            }
        }
    }

    fn prepare(&self, image: Arc<DecodedImage>, ctx: &egui::Context) -> CachedPreview {
        let color_image = egui::ColorImage::from_rgba_unmultiplied(
            [
                image.preview_size[0] as usize,
                image.preview_size[1] as usize,
            ],
            &image.rgba,
        );
        let texture = ctx.load_texture(
            image.path.to_string_lossy(),
            color_image,
            TextureOptions::LINEAR,
        );
        CachedPreview { image, texture }
    }

    fn install_cached(&mut self, preview: CachedPreview) {
        self.texture = Some(preview.texture);
        self.current = Some(preview.image);
    }

    fn open_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Open any image")
            .pick_file()
        {
            self.open(path);
        }
    }

    fn copy_current_path(&self, ctx: &egui::Context) {
        if let Some(path) = self.catalog.current() {
            ctx.copy_text(path.display().to_string());
        }
    }

    fn reveal_current(&self) {
        if let Some(path) = self.catalog.current() {
            let _ = std::process::Command::new("/usr/bin/open")
                .arg("-R")
                .arg(path)
                .spawn();
        }
    }

    fn save_current_copy(&self) {
        if let Some(source) = self.catalog.current()
            && let Some(destination) = rfd::FileDialog::new()
                .set_file_name(source.file_name().unwrap_or_default().to_string_lossy())
                .save_file()
        {
            let _ = std::fs::copy(source, destination);
        }
    }

    fn toggle_appearance(&mut self, ctx: &egui::Context) {
        self.dark_mode = !self.dark_mode;
        apply_appearance(ctx, self.dark_mode);
    }

    fn handle_input(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .find(|path| path.is_file())
        });
        if let Some(path) = dropped {
            self.open(path);
        }

        let (command, shift) = ctx.input(|input| (input.modifiers.command, input.modifiers.shift));

        if command && ctx.input(|input| input.key_pressed(egui::Key::O)) {
            self.open_dialog();
        }
        if command && shift && ctx.input(|input| input.key_pressed(egui::Key::C)) {
            self.copy_current_path(ctx);
        }
        if command && shift && ctx.input(|input| input.key_pressed(egui::Key::S)) {
            self.save_current_copy();
        }
        if command && shift && ctx.input(|input| input.key_pressed(egui::Key::R)) {
            self.reveal_current();
        }
        if !command && ctx.input(|input| input.key_pressed(egui::Key::ArrowRight)) {
            self.move_by(1);
        }
        if !command && ctx.input(|input| input.key_pressed(egui::Key::ArrowLeft)) {
            self.move_by(-1);
        }
        if ctx.input(|input| input.key_pressed(egui::Key::Num0)) {
            self.view.reset();
        }
        if ctx.input(|input| input.key_pressed(egui::Key::Num1))
            && let (Some(viewport), Some(image)) = (self.last_viewport, &self.current)
        {
            self.view
                .set_actual_size(viewport, self.oriented_size(image.original_size));
        }
        if ctx.input(|input| input.key_pressed(egui::Key::Plus)) {
            self.view.zoom_step(1.25);
        }
        if ctx.input(|input| input.key_pressed(egui::Key::Minus)) {
            self.view.zoom_step(0.8);
        }
        if !command && ctx.input(|input| input.key_pressed(egui::Key::R)) {
            self.rotation = if shift {
                (self.rotation + 3) % 4
            } else {
                (self.rotation + 1) % 4
            };
            self.view.reset();
        }
        if !command && ctx.input(|input| input.key_pressed(egui::Key::H)) {
            self.flip_horizontal = !self.flip_horizontal;
        }
        if !command && ctx.input(|input| input.key_pressed(egui::Key::V)) {
            self.flip_vertical = !self.flip_vertical;
        }
        if !command && ctx.input(|input| input.key_pressed(egui::Key::I)) {
            self.show_info = !self.show_info;
        }
        if !command && ctx.input(|input| input.key_pressed(egui::Key::B)) {
            self.show_filmstrip = !self.show_filmstrip;
        }
        if !command && ctx.input(|input| input.key_pressed(egui::Key::Space)) {
            self.slideshow = !self.slideshow;
            self.last_slide = Instant::now();
        }
        if !command && ctx.input(|input| input.key_pressed(egui::Key::T)) {
            self.toggle_appearance(ctx);
        }
        if !command && ctx.input(|input| input.key_pressed(egui::Key::F)) {
            self.toggle_fullscreen(ctx);
        }
        if self.fullscreen && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.toggle_fullscreen(ctx);
        }
        if ctx.input(|input| input.key_pressed(egui::Key::Home)) && self.catalog.set_position(0) {
            self.load_current();
        }
        if ctx.input(|input| input.key_pressed(egui::Key::End))
            && self.catalog.len() > 0
            && self.catalog.set_position(self.catalog.len() - 1)
        {
            self.load_current();
        }
    }

    fn toggle_fullscreen(&mut self, ctx: &egui::Context) {
        self.fullscreen = !self.fullscreen;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
    }

    fn oriented_size(&self, size: [u32; 2]) -> [u32; 2] {
        if self.rotation % 2 == 1 {
            [size[1], size[0]]
        } else {
            size
        }
    }

    fn top_bar(&mut self, root: &mut egui::Ui) {
        let toolbar_fill = if self.dark_mode {
            Color32::from_rgb(20, 20, 23)
        } else {
            Color32::from_rgb(246, 246, 248)
        };
        egui::Panel::top("toolbar")
            .exact_size(54.0)
            .frame(
                egui::Frame::new()
                    .fill(toolbar_fill)
                    .inner_margin(egui::Margin::symmetric(12, 8)),
            )
            .show(root, |ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
                ui.spacing_mut().button_padding = vec2(10.0, 6.0);
                ui.spacing_mut().interact_size = vec2(38.0, TOOLBAR_BUTTON_HEIGHT);
                egui::ScrollArea::horizontal()
                    .id_salt("main-toolbar")
                    .show(ui, |ui| {
                        ui.horizontal_centered(|ui| {
                            let (icon_rect, _) = ui.allocate_exact_size(
                                vec2(TOOLBAR_BUTTON_HEIGHT, 34.0),
                                Sense::hover(),
                            );
                            ui.painter().image(
                                self.brand_icon.id(),
                                icon_rect,
                                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                Color32::WHITE,
                            );
                            ui.label(
                                RichText::new("EAGLE")
                                    .strong()
                                    .color(Color32::from_rgb(235, 58, 72)),
                            );
                            ui.separator();
                            if open_button(ui)
                                .on_hover_text("Open image (Cmd+O)")
                                .clicked()
                            {
                                self.open_dialog();
                            }

                            ui.separator();
                            let can_previous = self.catalog.position() > 0;
                            if toolbar_icon_button(ui, icons::CARET_LEFT, can_previous)
                                .on_hover_text("Previous image (Left Arrow)")
                                .clicked()
                            {
                                self.move_by(-1);
                            }
                            let can_next = self.catalog.position() + 1 < self.catalog.len();
                            if toolbar_icon_button(ui, icons::CARET_RIGHT, can_next)
                                .on_hover_text("Next image (Right Arrow)")
                                .clicked()
                            {
                                self.move_by(1);
                            }

                            ui.separator();
                            if toolbar_icon_button(ui, icons::MINUS, true)
                                .on_hover_text("Zoom out (-)")
                                .clicked()
                            {
                                self.view.zoom_step(0.8);
                            }
                            if toolbar_icon_button(ui, icons::PLUS, true)
                                .on_hover_text("Zoom in (+)")
                                .clicked()
                            {
                                self.view.zoom_step(1.25);
                            }
                            if ui
                                .add_sized(
                                    vec2(64.0, TOOLBAR_BUTTON_HEIGHT),
                                    egui::Button::new(format!("{}  Fit", icons::ARROWS_IN_SIMPLE)),
                                )
                                .on_hover_text("Fit to window (0)")
                                .clicked()
                            {
                                self.view.reset();
                            }
                            if ui
                                .add_sized(
                                    vec2(52.0, TOOLBAR_BUTTON_HEIGHT),
                                    egui::Button::new("1:1"),
                                )
                                .on_hover_text("Actual pixels (1)")
                                .clicked()
                                && let (Some(viewport), Some(image)) =
                                    (self.last_viewport, &self.current)
                            {
                                self.view.set_actual_size(
                                    viewport,
                                    self.oriented_size(image.original_size),
                                );
                            }

                            ui.separator();
                            if toolbar_icon_button(ui, icons::ARROW_COUNTER_CLOCKWISE, true)
                                .on_hover_text("Rotate counterclockwise (Shift+R)")
                                .clicked()
                            {
                                self.rotation = (self.rotation + 3) % 4;
                                self.view.reset();
                            }
                            if toolbar_icon_button(ui, icons::ARROW_CLOCKWISE, true)
                                .on_hover_text("Rotate clockwise (R)")
                                .clicked()
                            {
                                self.rotation = (self.rotation + 1) % 4;
                                self.view.reset();
                            }
                            if toolbar_icon_button(ui, icons::FLIP_HORIZONTAL, true)
                                .on_hover_text("Flip horizontally (H)")
                                .clicked()
                            {
                                self.flip_horizontal = !self.flip_horizontal;
                            }
                            if toolbar_icon_button(ui, icons::FLIP_VERTICAL, true)
                                .on_hover_text("Flip vertically (V)")
                                .clicked()
                            {
                                self.flip_vertical = !self.flip_vertical;
                            }

                            ui.separator();
                            if ui
                                .add_sized(
                                    vec2(70.0, TOOLBAR_BUTTON_HEIGHT),
                                    egui::Button::new(format!("{}  Info", icons::INFO))
                                        .selected(self.show_info),
                                )
                                .on_hover_text("Image information (I)")
                                .clicked()
                            {
                                self.show_info = !self.show_info;
                            }
                            if ui
                                .add_sized(
                                    vec2(110.0, TOOLBAR_BUTTON_HEIGHT),
                                    egui::Button::new(format!("{}  Filmstrip", icons::FILM_STRIP))
                                        .selected(self.show_filmstrip),
                                )
                                .on_hover_text("Toggle filmstrip (B)")
                                .clicked()
                            {
                                self.show_filmstrip = !self.show_filmstrip;
                            }
                            if ui
                                .add_sized(
                                    vec2(82.0, TOOLBAR_BUTTON_HEIGHT),
                                    egui::Button::new(if self.slideshow {
                                        format!("{}  Pause", icons::PAUSE)
                                    } else {
                                        format!("{}  Play", icons::PLAY)
                                    })
                                    .selected(self.slideshow),
                                )
                                .on_hover_text("Play or pause slideshow (Space)")
                                .clicked()
                            {
                                self.slideshow = !self.slideshow;
                                self.last_slide = Instant::now();
                            }
                            if toolbar_icon_button(ui, icons::CORNERS_OUT, true)
                                .on_hover_text("Toggle fullscreen (F)")
                                .clicked()
                            {
                                self.toggle_fullscreen(ui.ctx());
                            }
                            if toolbar_icon_button(
                                ui,
                                if self.dark_mode {
                                    icons::SUN
                                } else {
                                    icons::MOON
                                },
                                true,
                            )
                            .on_hover_text(if self.dark_mode {
                                "Use light appearance (T)"
                            } else {
                                "Use dark appearance (T)"
                            })
                            .clicked()
                            {
                                self.toggle_appearance(ui.ctx());
                            }
                            ui.menu_button(format!("{}  More", icons::DOTS_THREE), |ui| {
                                if ui.button("Copy file path    Cmd+Shift+C").clicked() {
                                    self.copy_current_path(ui.ctx());
                                    ui.close();
                                }
                                if ui.button("Reveal in Finder    Cmd+Shift+R").clicked() {
                                    self.reveal_current();
                                    ui.close();
                                }
                                if ui.button("Save a copy...    Cmd+Shift+S").clicked() {
                                    self.save_current_copy();
                                    ui.close();
                                }
                            });

                            ui.separator();
                            if let Some(image) = &self.current {
                                let name =
                                    image.path.file_name().unwrap_or_default().to_string_lossy();
                                ui.label(RichText::new(name).strong())
                                    .on_hover_text(image.path.display().to_string());
                            }

                            if self.catalog.len() > 0 {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(format!(
                                            "{} / {}",
                                            self.catalog.position() + 1,
                                            self.catalog.len()
                                        ));
                                    },
                                );
                            }
                        });
                    });
            });
    }

    fn filmstrip(&mut self, root: &mut egui::Ui) {
        if !self.show_filmstrip || self.catalog.len() == 0 {
            return;
        }
        let panel_fill = if self.dark_mode {
            Color32::from_rgb(17, 19, 23)
        } else {
            Color32::from_rgb(236, 237, 240)
        };
        let items = self.catalog.window(18);
        let current = self.catalog.position();
        let mut selected = None;
        egui::Panel::bottom("filmstrip")
            .exact_size(108.0)
            .frame(
                egui::Frame::new()
                    .fill(panel_fill)
                    .inner_margin(egui::Margin::symmetric(10, 8)),
            )
            .show(root, |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("image-carousel")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (index, path) in items {
                                let (rect, response) =
                                    ui.allocate_exact_size(vec2(86.0, 88.0), Sense::click());
                                let selected_now = index == current;
                                ui.painter().rect_filled(
                                    rect,
                                    7.0,
                                    if selected_now {
                                        Color32::from_rgb(151, 30, 43)
                                    } else if response.hovered() {
                                        if self.dark_mode {
                                            Color32::from_rgb(39, 42, 50)
                                        } else {
                                            Color32::from_rgb(255, 220, 225)
                                        }
                                    } else if self.dark_mode {
                                        Color32::from_rgb(25, 27, 33)
                                    } else {
                                        Color32::WHITE
                                    },
                                );
                                let image_rect = Rect::from_min_max(
                                    rect.min + vec2(5.0, 5.0),
                                    Pos2::new(rect.max.x - 5.0, rect.max.y - 22.0),
                                );
                                if let Some(preview) = self.cache.peek(&path) {
                                    let source = preview.image.preview_size;
                                    let aspect = source[0] as f32 / source[1].max(1) as f32;
                                    let mut size = image_rect.size();
                                    if size.x / size.y > aspect {
                                        size.x = size.y * aspect;
                                    } else {
                                        size.y = size.x / aspect;
                                    }
                                    ui.painter().image(
                                        preview.texture.id(),
                                        Rect::from_center_size(image_rect.center(), size),
                                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                        Color32::WHITE,
                                    );
                                } else {
                                    ui.painter().text(
                                        image_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        path.extension()
                                            .and_then(|extension| extension.to_str())
                                            .unwrap_or("IMG")
                                            .to_uppercase(),
                                        FontId::monospace(12.0),
                                        Color32::from_gray(125),
                                    );
                                }
                                let name = path.file_name().unwrap_or_default().to_string_lossy();
                                let short_name = if name.chars().count() > 12 {
                                    format!("{}…", name.chars().take(11).collect::<String>())
                                } else {
                                    name.into_owned()
                                };
                                ui.painter().text(
                                    Pos2::new(rect.center().x, rect.max.y - 10.0),
                                    egui::Align2::CENTER_CENTER,
                                    short_name,
                                    FontId::proportional(10.5),
                                    if selected_now || self.dark_mode {
                                        Color32::from_gray(225)
                                    } else {
                                        Color32::from_gray(45)
                                    },
                                );
                                if response.clicked() {
                                    selected = Some(index);
                                }
                            }
                        });
                    });
            });
        if let Some(index) = selected
            && self.catalog.set_position(index)
        {
            self.load_current();
        }
    }

    fn paint_oriented(&self, painter: &egui::Painter, texture: &TextureHandle, rect: Rect) {
        let mut uvs = match self.rotation % 4 {
            0 => [
                Pos2::new(0.0, 0.0),
                Pos2::new(1.0, 0.0),
                Pos2::new(0.0, 1.0),
                Pos2::new(1.0, 1.0),
            ],
            1 => [
                Pos2::new(0.0, 1.0),
                Pos2::new(0.0, 0.0),
                Pos2::new(1.0, 1.0),
                Pos2::new(1.0, 0.0),
            ],
            2 => [
                Pos2::new(1.0, 1.0),
                Pos2::new(0.0, 1.0),
                Pos2::new(1.0, 0.0),
                Pos2::new(0.0, 0.0),
            ],
            _ => [
                Pos2::new(1.0, 0.0),
                Pos2::new(1.0, 1.0),
                Pos2::new(0.0, 0.0),
                Pos2::new(0.0, 1.0),
            ],
        };
        for uv in &mut uvs {
            if self.flip_horizontal {
                uv.x = 1.0 - uv.x;
            }
            if self.flip_vertical {
                uv.y = 1.0 - uv.y;
            }
        }
        let positions = [
            rect.left_top(),
            rect.right_top(),
            rect.left_bottom(),
            rect.right_bottom(),
        ];
        let mut mesh = Mesh::with_texture(texture.id());
        for (pos, uv) in positions.into_iter().zip(uvs) {
            mesh.vertices.push(Vertex {
                pos,
                uv,
                color: Color32::WHITE,
            });
        }
        mesh.indices.extend_from_slice(&[0, 1, 2, 2, 1, 3]);
        painter.add(mesh);
    }

    fn draw_detail_tiles(
        &mut self,
        ctx: &egui::Context,
        painter: &egui::Painter,
        viewport: Rect,
        image_rect: Rect,
        original_size: [u32; 2],
        preview_size: [u32; 2],
    ) {
        if self.rotation != 0 || self.flip_horizontal || self.flip_vertical {
            return;
        }
        let Some(detail) = self.detail.clone() else {
            return;
        };
        if detail.size != original_size || image_rect.width() <= 0.0 || image_rect.height() <= 0.0 {
            return;
        }
        let actual_scale = image_rect.width() / original_size[0] as f32;
        let preview_scale = preview_size[0] as f32 / original_size[0] as f32;
        if actual_scale < preview_scale * 0.9 {
            return;
        }
        let visible = viewport.intersect(image_rect);
        if !visible.is_positive() {
            return;
        }
        let source_left = ((visible.left() - image_rect.left()) / image_rect.width()
            * original_size[0] as f32)
            .floor()
            .max(0.0) as u32;
        let source_top = ((visible.top() - image_rect.top()) / image_rect.height()
            * original_size[1] as f32)
            .floor()
            .max(0.0) as u32;
        let source_right = ((visible.right() - image_rect.left()) / image_rect.width()
            * original_size[0] as f32)
            .ceil()
            .min(original_size[0] as f32) as u32;
        let source_bottom = ((visible.bottom() - image_rect.top()) / image_rect.height()
            * original_size[1] as f32)
            .ceil()
            .min(original_size[1] as f32) as u32;
        let mut keys = Vec::new();
        for tile_y in source_top / TILE_SIZE..=source_bottom.saturating_sub(1) / TILE_SIZE {
            for tile_x in source_left / TILE_SIZE..=source_right.saturating_sub(1) / TILE_SIZE {
                keys.push((tile_x, tile_y));
            }
        }
        let center_x = (source_left + source_right) as f32 / (2.0 * TILE_SIZE as f32);
        let center_y = (source_top + source_bottom) as f32 / (2.0 * TILE_SIZE as f32);
        keys.sort_by(|left, right| {
            let left_distance =
                (left.0 as f32 - center_x).powi(2) + (left.1 as f32 - center_y).powi(2);
            let right_distance =
                (right.0 as f32 - center_x).powi(2) + (right.1 as f32 - center_y).powi(2);
            left_distance.total_cmp(&right_distance)
        });

        let mut uploaded = 0;
        for &(tile_x, tile_y) in &keys {
            if self.tiles.contains_key(&(tile_x, tile_y)) || uploaded >= 2 {
                continue;
            }
            let x = tile_x * TILE_SIZE;
            let y = tile_y * TILE_SIZE;
            let width = TILE_SIZE.min(original_size[0] - x);
            let height = TILE_SIZE.min(original_size[1] - y);
            let rgba = detail.tile_rgba(x, y, width, height);
            let texture = ctx.load_texture(
                format!("detail-{}-{tile_x}-{tile_y}", self.generation),
                egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba),
                TextureOptions::LINEAR,
            );
            self.tiles.insert((tile_x, tile_y), texture);
            self.tile_order.push_back((tile_x, tile_y));
            uploaded += 1;
        }
        while self.tiles.len() > MAX_GPU_TILES {
            if let Some(key) = self.tile_order.pop_front() {
                self.tiles.remove(&key);
            }
        }
        for (tile_x, tile_y) in keys {
            let Some(texture) = self.tiles.get(&(tile_x, tile_y)) else {
                continue;
            };
            let x = tile_x * TILE_SIZE;
            let y = tile_y * TILE_SIZE;
            let width = TILE_SIZE.min(original_size[0] - x);
            let height = TILE_SIZE.min(original_size[1] - y);
            let rect = Rect::from_min_max(
                Pos2::new(
                    image_rect.left() + x as f32 / original_size[0] as f32 * image_rect.width(),
                    image_rect.top() + y as f32 / original_size[1] as f32 * image_rect.height(),
                ),
                Pos2::new(
                    image_rect.left()
                        + (x + width) as f32 / original_size[0] as f32 * image_rect.width(),
                    image_rect.top()
                        + (y + height) as f32 / original_size[1] as f32 * image_rect.height(),
                ),
            );
            painter.image(
                texture.id(),
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if uploaded > 0 {
            ctx.request_repaint();
        }
    }

    fn viewer(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        let canvas_fill = if self.dark_mode {
            Color32::from_rgb(10, 11, 13)
        } else {
            Color32::from_rgb(218, 219, 223)
        };
        let secondary_text = if self.dark_mode {
            Color32::from_gray(155)
        } else {
            Color32::from_gray(75)
        };
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(canvas_fill))
            .show(root, |ui| {
                let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::drag());
                let viewport = response.rect;
                self.last_viewport = Some(viewport);

                let primary_drag = response.dragged_by(PointerButton::Primary);
                let middle_drag = response.dragged_by(PointerButton::Middle);
                if primary_drag || middle_drag {
                    self.view.pan_by(response.drag_delta());
                }
                if response.contains_pointer() {
                    ui.ctx().set_cursor_icon(if primary_drag || middle_drag {
                        egui::CursorIcon::Grabbing
                    } else {
                        egui::CursorIcon::Grab
                    });
                }

                let double_clicked = response.contains_pointer()
                    && ctx
                        .input(|input| input.pointer.button_double_clicked(PointerButton::Primary));
                if double_clicked {
                    self.view.reset();
                }

                if response.contains_pointer() {
                    let pointer = ctx.input(|input| input.pointer.latest_pos());
                    let (gesture_zoom, scroll) =
                        ctx.input(|input| (input.zoom_delta(), input.smooth_scroll_delta()));
                    if let Some(factor) = viewport_zoom_factor(gesture_zoom, scroll) {
                        self.view.zoom_by(factor, pointer, viewport);
                    }
                }

                if let (Some(texture), Some(image)) = (self.texture.clone(), self.current.clone()) {
                    let oriented_size = self.oriented_size(image.original_size);
                    let rect = self.view.image_rect(viewport, oriented_size);

                    let preview_is_reduced = image.preview_size != image.original_size;
                    let zoom_needs_detail = rect.width() >= image.preview_size[0] as f32 * 0.9
                        || rect.height() >= image.preview_size[1] as f32 * 0.9;
                    if preview_is_reduced
                        && zoom_needs_detail
                        && self.rotation == 0
                        && !self.flip_horizontal
                        && !self.flip_vertical
                        && self.detail.is_none()
                        && !self.detail_loading
                        && self.detail_error.is_none()
                        && !self.slideshow
                    {
                        self.request_detail(image.path.clone());
                    }

                    painter.rect_filled(rect.expand(1.0), 0.0, Color32::BLACK);
                    self.paint_oriented(&painter, &texture, rect);
                    self.draw_detail_tiles(
                        &ctx,
                        &painter,
                        viewport,
                        rect,
                        image.original_size,
                        image.preview_size,
                    );

                    let scale = self.view.actual_scale(viewport, oriented_size) * 100.0;
                    painter.text(
                        viewport.left_bottom() + Vec2::new(14.0, -14.0),
                        egui::Align2::LEFT_BOTTOM,
                        format!("{scale:.0}%"),
                        FontId::monospace(12.0),
                        secondary_text,
                    );

                    if self.show_info {
                        let file_size = std::fs::metadata(&image.path)
                            .map(|metadata| metadata.len())
                            .unwrap_or(0);
                        let detail_status = if let Some(detail) = &self.detail {
                            format!(
                                "Full detail ready · {:.0} ms · {} tiles resident",
                                detail.elapsed.as_secs_f64() * 1000.0,
                                self.tiles.len()
                            )
                        } else if self.detail_loading {
                            "Building SSD-backed full detail…".into()
                        } else if let Some(error) = &self.detail_error {
                            format!("Preview only · {error}")
                        } else {
                            "Instant preview · zoom in for full detail".into()
                        };
                        let info_rect = Rect::from_min_size(
                            viewport.left_top() + vec2(16.0, 16.0),
                            vec2(290.0, 126.0),
                        );
                        painter.rect_filled(
                            info_rect,
                            9.0,
                            Color32::from_rgba_unmultiplied(15, 17, 22, 225),
                        );
                        painter.text(
                            info_rect.min + vec2(14.0, 13.0),
                            egui::Align2::LEFT_TOP,
                            format!(
                                "{}\n{} × {} px  ·  {}\n{}  ·  preview {} × {}\n{}",
                                image.path.file_name().unwrap_or_default().to_string_lossy(),
                                image.original_size[0],
                                image.original_size[1],
                                image.format,
                                human_bytes(file_size),
                                image.preview_size[0],
                                image.preview_size[1],
                                detail_status,
                            ),
                            FontId::monospace(12.0),
                            Color32::from_gray(225),
                        );
                    }
                } else if self.error.is_none() {
                    painter.text(
                        viewport.center(),
                        egui::Align2::CENTER_CENTER,
                        "Drop an image here\n\nor press Open…",
                        FontId::proportional(20.0),
                        secondary_text,
                    );
                }

                if self.loading {
                    painter.text(
                        viewport.right_top() + Vec2::new(-16.0, 16.0),
                        egui::Align2::RIGHT_TOP,
                        "Decoding preview…",
                        FontId::proportional(13.0),
                        Color32::from_rgb(235, 122, 131),
                    );
                }
                if self.detail_loading {
                    painter.text(
                        viewport.right_bottom() + Vec2::new(-16.0, -14.0),
                        egui::Align2::RIGHT_BOTTOM,
                        "Preparing full detail on SSD…",
                        FontId::proportional(12.0),
                        Color32::from_gray(135),
                    );
                }
                if let Some(error) = &self.error {
                    painter.text(
                        viewport.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("Could not open image\n\n{error}"),
                        FontId::proportional(16.0),
                        Color32::from_rgb(245, 130, 130),
                    );
                }
            });
    }
}

impl eframe::App for EagleApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.process_screenshot(&ctx);
        self.process_results(&ctx);
        self.process_detail_results();
        self.handle_input(&ctx);
        self.top_bar(ui);
        self.filmstrip(ui);
        self.viewer(ui);

        if self.slideshow && self.last_slide.elapsed() >= Duration::from_secs(3) {
            if self.catalog.position() + 1 < self.catalog.len() {
                self.move_by(1);
            } else if self.catalog.set_position(0) {
                self.load_current();
            }
            self.last_slide = Instant::now();
        }

        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        if self.view.tick(dt) {
            ctx.request_repaint();
        }
        if self.loading || self.detail_loading {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        if self.slideshow {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if self.screenshot_path.is_some() && !self.screenshot_requested {
            if self.current.is_some() && !self.loading {
                self.screenshot_settle_frames = self.screenshot_settle_frames.saturating_add(1);
                if self.screenshot_settle_frames >= 8 {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                        egui::UserData::default(),
                    ));
                    self.screenshot_requested = true;
                }
            }
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_wheel_and_pinch_both_produce_zoom() {
        assert!(viewport_zoom_factor(1.0, vec2(0.0, 14.0)).unwrap() > 1.0);
        assert!(viewport_zoom_factor(1.0, vec2(0.0, -14.0)).unwrap() < 1.0);
        assert_eq!(viewport_zoom_factor(1.25, Vec2::ZERO), Some(1.25));
        assert_eq!(viewport_zoom_factor(1.0, Vec2::ZERO), None);
    }
}
