use std::{
    collections::VecDeque,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use crossbeam_channel::{Receiver, Sender, unbounded};
use image::{DynamicImage, ImageReader, imageops::FilterType};

use crate::catalog::{is_raw, uses_native_fallback};

const PREVIEW_EDGE: u32 = 4096;
const FALLBACK_PIXEL_LIMIT: u64 = 180_000_000;
type DecodedPreview = ([u32; 2], [u32; 2], Vec<u8>, String);

#[derive(Debug)]
pub struct DecodedImage {
    pub path: PathBuf,
    pub original_size: [u32; 2],
    pub preview_size: [u32; 2],
    pub rgba: Vec<u8>,
    pub format: String,
    pub elapsed: Duration,
}

impl DecodedImage {
    pub fn memory_bytes(&self) -> usize {
        self.rgba.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    Current,
    Prefetch,
}

#[derive(Debug)]
pub struct LoadResult {
    pub generation: u64,
    pub kind: RequestKind,
    pub path: PathBuf,
    pub result: Result<DecodedImage, String>,
}

#[derive(Clone)]
struct Job {
    generation: u64,
    kind: RequestKind,
    path: PathBuf,
}

#[derive(Default)]
struct QueueState {
    current: Option<Job>,
    prefetch: VecDeque<Job>,
    stopped: bool,
}

struct SharedQueue {
    state: Mutex<QueueState>,
    ready: Condvar,
}

pub struct ImageLoader {
    queue: Arc<SharedQueue>,
    results: Receiver<LoadResult>,
    wanted_generation: Arc<AtomicU64>,
    stopped: Arc<AtomicBool>,
    workers: Vec<thread::JoinHandle<()>>,
}

impl ImageLoader {
    pub fn new() -> Self {
        let queue = Arc::new(SharedQueue {
            state: Mutex::new(QueueState::default()),
            ready: Condvar::new(),
        });
        let wanted_generation = Arc::new(AtomicU64::new(0));
        let stopped = Arc::new(AtomicBool::new(false));
        let (result_tx, results) = unbounded();

        let workers = (0..2)
            .map(|index| {
                let queue = queue.clone();
                let wanted_generation = wanted_generation.clone();
                let stopped = stopped.clone();
                let result_tx = result_tx.clone();
                thread::Builder::new()
                    .name(format!("image-decoder-{index}"))
                    .spawn(move || worker_loop(queue, wanted_generation, stopped, result_tx))
                    .expect("failed to start decoder worker")
            })
            .collect();

        Self {
            queue,
            results,
            wanted_generation,
            stopped,
            workers,
        }
    }

    pub fn request_current(&self, path: PathBuf, generation: u64) {
        self.wanted_generation.store(generation, Ordering::Release);
        let mut state = self.queue.state.lock().unwrap();
        state.current = Some(Job {
            generation,
            kind: RequestKind::Current,
            path,
        });
        self.queue.ready.notify_one();
    }

    pub fn request_prefetch(&self, path: PathBuf, generation: u64) {
        let mut state = self.queue.state.lock().unwrap();
        if state.prefetch.iter().any(|job| job.path == path) {
            return;
        }
        state.prefetch.push_back(Job {
            generation,
            kind: RequestKind::Prefetch,
            path,
        });
        state.prefetch.truncate(4);
        self.queue.ready.notify_one();
    }

    pub fn try_recv(&self) -> Option<LoadResult> {
        self.results.try_recv().ok()
    }
}

impl Drop for ImageLoader {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Ok(mut state) = self.queue.state.lock() {
            state.stopped = true;
        }
        self.queue.ready.notify_all();
        for worker in self.workers.drain(..) {
            // Decoders are not all cooperatively cancellable. Detach so closing the
            // window never waits for a multi-gigapixel sequential stream to finish.
            drop(worker);
        }
    }
}

fn worker_loop(
    queue: Arc<SharedQueue>,
    wanted_generation: Arc<AtomicU64>,
    stopped: Arc<AtomicBool>,
    result_tx: Sender<LoadResult>,
) {
    loop {
        let job = {
            let mut state = queue.state.lock().unwrap();
            while state.current.is_none() && state.prefetch.is_empty() && !state.stopped {
                state = queue.ready.wait(state).unwrap();
            }
            if state.stopped {
                return;
            }
            state.current.take().or_else(|| state.prefetch.pop_front())
        };

        let Some(job) = job else { continue };
        if stopped.load(Ordering::Acquire) {
            return;
        }
        if job.kind == RequestKind::Current
            && job.generation != wanted_generation.load(Ordering::Acquire)
        {
            continue;
        }

        let result = decode_preview(&job.path).map_err(|error| format!("{error:#}"));
        if job.kind == RequestKind::Current
            && job.generation != wanted_generation.load(Ordering::Acquire)
        {
            continue;
        }
        let _ = result_tx.send(LoadResult {
            generation: job.generation,
            kind: job.kind,
            path: job.path,
            result,
        });
    }
}

pub(crate) fn decode_preview(path: &Path) -> Result<DecodedImage> {
    let started = Instant::now();
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();

    let (original_size, preview_size, rgba, format) = if is_raw(path) {
        decode_camera_raw(path)?
    } else if uses_native_fallback(path) {
        decode_native_preview(path)?
    } else if extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg") {
        decode_jpeg(path)?
    } else if extension.eq_ignore_ascii_case("png") {
        decode_png_streaming(path)?
    } else {
        decode_fallback(path).or_else(|_| decode_native_preview(path))?
    };

    Ok(DecodedImage {
        path: path.to_path_buf(),
        original_size,
        preview_size,
        rgba,
        format,
        elapsed: started.elapsed(),
    })
}

fn decode_camera_raw(path: &Path) -> Result<DecodedPreview> {
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("dng"))
    {
        return decode_native_preview(path);
    }

    let params = rawler::decoders::RawDecodeParams::default();
    let raw_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        rawler::analyze::extract_preview_pixels(path, &params)
    }));
    let image = match raw_result {
        Ok(Ok(image)) => image,
        Ok(Err(_)) | Err(_) => return decode_native_preview(path),
    };
    let original_size = [image.width(), image.height()];
    let preview = make_preview(image, PREVIEW_EDGE);
    let format = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("RAW")
        .to_uppercase();
    Ok((
        original_size,
        [preview.width(), preview.height()],
        preview.into_raw(),
        format,
    ))
}

fn decode_native_preview(path: &Path) -> Result<DecodedPreview> {
    #[cfg(not(target_os = "macos"))]
    bail!("{} needs the macOS native image stack", path.display());

    #[cfg(target_os = "macos")]
    {
        static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
        let temp_dir = std::env::temp_dir().join(format!(
            "eagle-viewer-preview-{}-{}",
            std::process::id(),
            NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&temp_dir).context("cannot create native preview directory")?;
        let _cleanup = TempPreviewDir(temp_dir.clone());

        let max_size = PREVIEW_EDGE.to_string();
        let output = Command::new("/usr/bin/qlmanage")
            .args(["-t", "-s", &max_size, "-o"])
            .arg(&temp_dir)
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .context("cannot start the macOS image decoder")?;
        if !output.status.success() {
            bail!(
                "macOS could not decode {}: {}",
                path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }

        let preview_path = std::fs::read_dir(&temp_dir)
            .context("cannot read native preview output")?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .find(|candidate| candidate.is_file())
            .context("macOS returned no image preview")?;
        let (_, preview_size, rgba, _) = decode_fallback(&preview_path)?;
        let format = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("Image")
            .to_uppercase();
        Ok((preview_size, preview_size, rgba, format))
    }
}

#[cfg(target_os = "macos")]
struct TempPreviewDir(PathBuf);

#[cfg(target_os = "macos")]
impl Drop for TempPreviewDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn decode_jpeg(path: &Path) -> Result<DecodedPreview> {
    let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut decoder = jpeg_decoder::Decoder::new(BufReader::new(file));
    decoder.read_info().context("cannot read JPEG header")?;
    let original = decoder.info().context("JPEG has no image header")?;
    let original_size = [u32::from(original.width), u32::from(original.height)];
    let requested = fit_within(original_size, PREVIEW_EDGE);
    decoder
        .scale(requested[0] as u16, requested[1] as u16)
        .context("cannot configure reduced JPEG decode")?;
    let pixels = decoder.decode().context("cannot decode JPEG")?;
    let info = decoder.info().context("JPEG metadata disappeared")?;
    let decoded_size = [u32::from(info.width), u32::from(info.height)];
    let rgba = jpeg_to_rgba(pixels, info.pixel_format)?;

    let image = image::RgbaImage::from_raw(decoded_size[0], decoded_size[1], rgba)
        .context("JPEG decoder returned a malformed buffer")?;
    let target = fit_within(decoded_size, PREVIEW_EDGE);
    let image = if target != decoded_size {
        image::imageops::resize(&image, target[0], target[1], FilterType::Triangle)
    } else {
        image
    };

    Ok((
        original_size,
        [image.width(), image.height()],
        image.into_raw(),
        "JPEG".into(),
    ))
}

fn jpeg_to_rgba(pixels: Vec<u8>, format: jpeg_decoder::PixelFormat) -> Result<Vec<u8>> {
    let channels = match format {
        jpeg_decoder::PixelFormat::L8 => 1,
        jpeg_decoder::PixelFormat::L16 => 2,
        jpeg_decoder::PixelFormat::RGB24 => 3,
        jpeg_decoder::PixelFormat::CMYK32 => 4,
    };
    let mut rgba = Vec::with_capacity(pixels.len() / channels * 4);
    match format {
        jpeg_decoder::PixelFormat::L8 => {
            for value in pixels {
                rgba.extend_from_slice(&[value, value, value, 255]);
            }
        }
        jpeg_decoder::PixelFormat::L16 => {
            for value in pixels.chunks_exact(2) {
                let value = value[0];
                rgba.extend_from_slice(&[value, value, value, 255]);
            }
        }
        jpeg_decoder::PixelFormat::RGB24 => {
            for value in pixels.chunks_exact(3) {
                rgba.extend_from_slice(&[value[0], value[1], value[2], 255]);
            }
        }
        jpeg_decoder::PixelFormat::CMYK32 => {
            for value in pixels.chunks_exact(4) {
                let c = u16::from(value[0]);
                let m = u16::from(value[1]);
                let y = u16::from(value[2]);
                let k = u16::from(value[3]);
                rgba.extend_from_slice(&[
                    ((c * k) / 255) as u8,
                    ((m * k) / 255) as u8,
                    ((y * k) / 255) as u8,
                    255,
                ]);
            }
        }
    }
    Ok(rgba)
}

fn decode_png_streaming(path: &Path) -> Result<DecodedPreview> {
    let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().context("cannot read PNG header")?;
    let info = reader.info();
    let original_size = [info.width, info.height];
    if info.interlaced {
        return decode_fallback(path);
    }
    let preview_size = fit_within(original_size, PREVIEW_EDGE);
    let mut rgba = vec![0; preview_size[0] as usize * preview_size[1] as usize * 4];
    let mut row = vec![
        0;
        reader
            .output_line_size(original_size[0])
            .context("PNG row too large")?
    ];
    let (color_type, _) = reader.output_color_type();
    let channels = png_channels(color_type)?;

    let mut target_y = 0_u32;
    for source_y in 0..original_size[1] {
        if reader
            .read_row(&mut row)
            .context("cannot decode PNG row")?
            .is_none()
        {
            break;
        }
        while target_y < preview_size[1]
            && source_y >= sample_position(target_y, original_size[1], preview_size[1])
        {
            let output_start = target_y as usize * preview_size[0] as usize * 4;
            convert_sampled_row(
                &row,
                &mut rgba[output_start..output_start + preview_size[0] as usize * 4],
                original_size[0],
                preview_size[0],
                channels,
            );
            target_y += 1;
        }
    }

    Ok((original_size, preview_size, rgba, "PNG".into()))
}

fn png_channels(color_type: png::ColorType) -> Result<usize> {
    match color_type {
        png::ColorType::Grayscale => Ok(1),
        png::ColorType::GrayscaleAlpha => Ok(2),
        png::ColorType::Rgb => Ok(3),
        png::ColorType::Rgba => Ok(4),
        png::ColorType::Indexed => bail!("indexed PNG was not expanded"),
    }
}

fn convert_sampled_row(
    source: &[u8],
    target: &mut [u8],
    source_width: u32,
    target_width: u32,
    channels: usize,
) {
    for x in 0..target_width {
        let source_x = sample_position(x, source_width, target_width) as usize;
        let source = &source[source_x * channels..source_x * channels + channels];
        let target = &mut target[x as usize * 4..x as usize * 4 + 4];
        match channels {
            1 => target.copy_from_slice(&[source[0], source[0], source[0], 255]),
            2 => target.copy_from_slice(&[source[0], source[0], source[0], source[1]]),
            3 => target.copy_from_slice(&[source[0], source[1], source[2], 255]),
            4 => target.copy_from_slice(source),
            _ => unreachable!(),
        }
    }
}

fn decode_fallback(path: &Path) -> Result<DecodedPreview> {
    let reader = ImageReader::open(path)
        .with_context(|| format!("cannot open {}", path.display()))?
        .with_guessed_format()
        .context("cannot determine image format")?;
    let format = reader
        .format()
        .map_or_else(|| "Image".to_owned(), |f| format!("{f:?}"));
    let original_size = reader
        .into_dimensions()
        .context("cannot read image dimensions")?;
    let pixels = u64::from(original_size.0) * u64::from(original_size.1);
    if pixels > FALLBACK_PIXEL_LIMIT {
        bail!(
            "{} × {} {} is too large for this format's whole-image decoder; JPEG and non-interlaced PNG already use bounded preview decoders",
            original_size.0,
            original_size.1,
            format
        );
    }

    let image = ImageReader::open(path)?
        .with_guessed_format()?
        .decode()
        .context("cannot decode image")?;
    let preview = make_preview(image, PREVIEW_EDGE);
    Ok((
        [original_size.0, original_size.1],
        [preview.width(), preview.height()],
        preview.into_raw(),
        format,
    ))
}

fn make_preview(image: DynamicImage, max_edge: u32) -> image::RgbaImage {
    let size = [image.width(), image.height()];
    let target = fit_within(size, max_edge);
    if target == size {
        image.into_rgba8()
    } else {
        image
            .resize(target[0], target[1], FilterType::Triangle)
            .into_rgba8()
    }
}

fn fit_within(size: [u32; 2], max_edge: u32) -> [u32; 2] {
    let longest = size[0].max(size[1]);
    if longest <= max_edge || longest == 0 {
        return size;
    }
    [
        ((u64::from(size[0]) * u64::from(max_edge)) / u64::from(longest)).max(1) as u32,
        ((u64::from(size[1]) * u64::from(max_edge)) / u64::from(longest)).max(1) as u32,
    ]
}

fn sample_position(target: u32, source_len: u32, target_len: u32) -> u32 {
    (((u64::from(target) * u64::from(source_len)) + u64::from(target_len / 2))
        / u64::from(target_len))
    .min(u64::from(source_len.saturating_sub(1))) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_preserves_aspect_ratio() {
        assert_eq!(fit_within([32_000, 16_000], 4096), [4096, 2048]);
        assert_eq!(fit_within([800, 600], 4096), [800, 600]);
    }

    #[test]
    fn row_sampling_hits_valid_pixels() {
        assert_eq!(sample_position(0, 32_000, 4096), 0);
        assert!(sample_position(4095, 32_000, 4096) < 32_000);
    }
}
