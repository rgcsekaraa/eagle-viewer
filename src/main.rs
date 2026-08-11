mod app;
mod cache;
mod catalog;
mod detail;
mod loader;
mod view;

use std::{path::PathBuf, sync::Arc};

use eframe::egui;

fn main() -> eframe::Result {
    let mut arguments = std::env::args_os().skip(1);
    let first_argument = arguments.next();
    if matches!(
        first_argument.as_deref(),
        Some(value) if value == std::ffi::OsStr::new("--bench-preview")
            || value == std::ffi::OsStr::new("--bench-detail")
    ) {
        let detail_benchmark =
            first_argument.as_deref() == Some(std::ffi::OsStr::new("--bench-detail"));
        let Some(path) = arguments.next().map(PathBuf::from) else {
            eprintln!("usage: eagle-viewer --bench-preview|--bench-detail /path/to/image");
            return Ok(());
        };
        if detail_benchmark {
            match detail::decode_detail(&path) {
                Ok(image) => println!(
                    "{}: {}x{} full detail, {:.2} ms, {:.1} MiB SSD-backed raster",
                    path.display(),
                    image.size[0],
                    image.size[1],
                    image.elapsed.as_secs_f64() * 1000.0,
                    image.pixels.as_slice().len() as f64 / (1024.0 * 1024.0),
                ),
                Err(error) => eprintln!("{error:#}"),
            }
            return Ok(());
        }
        match loader::decode_preview(&path) {
            Ok(image) => println!(
                "{}: {}x{} -> {}x{} preview, {}, {:.2} ms, {:.1} MiB",
                image.path.display(),
                image.original_size[0],
                image.original_size[1],
                image.preview_size[0],
                image.preview_size[1],
                image.format,
                image.elapsed.as_secs_f64() * 1000.0,
                image.memory_bytes() as f64 / (1024.0 * 1024.0),
            ),
            Err(error) => eprintln!("{error:#}"),
        }
        return Ok(());
    }
    let mut screenshot_path = None;
    let mut forced_dark_mode = None;
    let initial_path = if first_argument.as_deref() == Some(std::ffi::OsStr::new("--screenshot")) {
        let Some(output) = arguments.next().map(PathBuf::from) else {
            eprintln!(
                "usage: eagle-viewer --screenshot /path/to/output.png /path/to/image [light|dark]"
            );
            return Ok(());
        };
        let Some(image) = arguments.next().map(PathBuf::from) else {
            eprintln!(
                "usage: eagle-viewer --screenshot /path/to/output.png /path/to/image [light|dark]"
            );
            return Ok(());
        };
        screenshot_path = Some(output);
        forced_dark_mode = match arguments.next().as_deref() {
            Some(value) if value == std::ffi::OsStr::new("light") => Some(false),
            Some(value) if value == std::ffi::OsStr::new("dark") => Some(true),
            _ => None,
        };
        Some(image)
    } else {
        first_argument.map(PathBuf::from)
    };

    let icon = image::load_from_memory(include_bytes!("../assets/EagleViewerIcon.png"))
        .expect("bundled Eagle Viewer icon is invalid")
        .resize_exact(256, 256, image::imageops::FilterType::Lanczos3)
        .into_rgba8();
    let icon_data = egui::IconData {
        rgba: icon.into_raw(),
        width: 256,
        height: 256,
    };
    let native_options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("Eagle Viewer")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([640.0, 420.0])
            .with_drag_and_drop(true)
            .with_icon(Arc::new(icon_data)),
        ..Default::default()
    };

    eframe::run_native(
        "Eagle Viewer",
        native_options,
        Box::new(move |cc| {
            Ok(Box::new(app::EagleApp::new(
                cc,
                initial_path,
                screenshot_path,
                forced_dark_mode,
            )))
        }),
    )
}
