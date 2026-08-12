mod app;
mod cache;
mod catalog;
mod detail;
mod loader;
mod recent;
mod view;

use std::{path::PathBuf, sync::Arc};

use eframe::egui;

#[cfg(target_os = "macos")]
fn disable_unmanaged_window_restoration() {
    use objc2_foundation::{NSUserDefaults, ns_string};

    let defaults = NSUserDefaults::standardUserDefaults();
    defaults.setBool_forKey(true, ns_string!("ApplePersistenceIgnoreState"));
    defaults.setBool_forKey(false, ns_string!("NSQuitAlwaysKeepsWindows"));
}

#[cfg(not(target_os = "macos"))]
fn disable_unmanaged_window_restoration() {}

fn parse_window_size(value: &str) -> Option<[f32; 2]> {
    let value = value.strip_prefix("size=")?;
    let (width, height) = value.split_once('x')?;
    let width = width.parse::<f32>().ok()?;
    let height = height.parse::<f32>().ok()?;
    (width.is_finite() && height.is_finite() && width >= 480.0 && height >= 320.0)
        .then_some([width, height])
}

fn main() -> eframe::Result {
    // eframe owns window creation and persistence; AppKit restoration has no
    // restoration class for this window and can otherwise suppress it after a forced quit.
    disable_unmanaged_window_restoration();
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
    let mut screenshot_info = false;
    let mut screenshot_size = None;
    let initial_path = if first_argument.as_deref() == Some(std::ffi::OsStr::new("--screenshot")) {
        let Some(output) = arguments.next().map(PathBuf::from) else {
            eprintln!(
                "usage: eagle-viewer --screenshot /path/to/output.png /path/to/image [light|dark] [info] [size=WIDTHxHEIGHT]"
            );
            return Ok(());
        };
        let Some(image) = arguments.next().map(PathBuf::from) else {
            eprintln!(
                "usage: eagle-viewer --screenshot /path/to/output.png /path/to/image [light|dark] [info] [size=WIDTHxHEIGHT]"
            );
            return Ok(());
        };
        screenshot_path = Some(output);
        for option in arguments {
            match option.as_os_str() {
                value if value == std::ffi::OsStr::new("light") => forced_dark_mode = Some(false),
                value if value == std::ffi::OsStr::new("dark") => forced_dark_mode = Some(true),
                value if value == std::ffi::OsStr::new("info") => screenshot_info = true,
                value => {
                    if let Some(value) = value.to_str() {
                        screenshot_size = parse_window_size(value).or(screenshot_size);
                    }
                }
            }
        }
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
        persist_window: screenshot_path.is_none(),
        viewport: egui::ViewportBuilder::default()
            .with_title("Eagle Viewer")
            .with_inner_size(screenshot_size.unwrap_or([1280.0, 820.0]))
            .with_min_inner_size([480.0, 320.0])
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
                screenshot_info,
            )))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_safe_screenshot_window_sizes() {
        assert_eq!(parse_window_size("size=640x420"), Some([640.0, 420.0]));
        assert_eq!(parse_window_size("size=479x420"), None);
        assert_eq!(parse_window_size("size=319x420"), None);
        assert_eq!(parse_window_size("size=wide"), None);
    }
}
