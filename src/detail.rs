use std::{
    fs::{File, OpenOptions},
    io::BufReader,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use crossbeam_channel::{Receiver, unbounded};
use image::ImageReader;
use memmap2::{Advice, Mmap, MmapMut, UncheckedAdvice};
use zune_jpeg::{
    JpegDecoder,
    zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions},
};

use crate::catalog::is_raw;

const MAX_OWNED_PIXELS: u64 = 220_000_000;

pub enum PixelStore {
    Mapped(MmapMut),
    Owned(Vec<u8>),
}

impl PixelStore {
    pub fn as_slice(&self) -> &[u8] {
        match self {
            Self::Mapped(map) => map,
            Self::Owned(bytes) => bytes,
        }
    }
}

pub struct DetailImage {
    pub size: [u32; 2],
    pub channels: usize,
    pub pixels: PixelStore,
    pub elapsed: Duration,
}

impl DetailImage {
    pub fn tile_rgba(&self, x: u32, y: u32, width: u32, height: u32) -> Vec<u8> {
        let pixels = self.pixels.as_slice();
        let mut tile = vec![0_u8; width as usize * height as usize * 4];
        let stride = self.size[0] as usize * self.channels;
        for row in 0..height as usize {
            let source_start = (y as usize + row) * stride + x as usize * self.channels;
            let source = &pixels[source_start..source_start + width as usize * self.channels];
            let target = &mut tile[row * width as usize * 4..(row + 1) * width as usize * 4];
            match self.channels {
                4 => target.copy_from_slice(source),
                3 => {
                    for (rgb, rgba) in source.chunks_exact(3).zip(target.chunks_exact_mut(4)) {
                        rgba.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
                    }
                }
                _ => unreachable!(),
            }
        }
        tile
    }
}

#[derive(Clone)]
struct DetailJob {
    path: PathBuf,
    generation: u64,
}

#[derive(Default)]
struct DetailState {
    latest: Option<DetailJob>,
    stopped: bool,
}

pub struct DetailResult {
    pub generation: u64,
    pub path: PathBuf,
    pub result: Result<Arc<DetailImage>, String>,
}

pub struct DetailLoader {
    state: Arc<(Mutex<DetailState>, Condvar)>,
    results: Receiver<DetailResult>,
    worker: Option<thread::JoinHandle<()>>,
}

impl DetailLoader {
    pub fn new() -> Self {
        let state = Arc::new((Mutex::new(DetailState::default()), Condvar::new()));
        let (sender, results) = unbounded();
        let worker_state = state.clone();
        let worker = thread::Builder::new()
            .name("full-resolution-decoder".into())
            .spawn(move || {
                loop {
                    let job = {
                        let (lock, ready) = &*worker_state;
                        let mut state = lock.lock().unwrap();
                        while state.latest.is_none() && !state.stopped {
                            state = ready.wait(state).unwrap();
                        }
                        if state.stopped {
                            return;
                        }
                        state.latest.take()
                    };
                    let Some(job) = job else { continue };
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        decode_detail(&job.path)
                    }))
                    .map_err(|_| "full-resolution decoder panicked on this file".to_owned())
                    .and_then(|result| result.map(Arc::new).map_err(|error| format!("{error:#}")));
                    let _ = sender.send(DetailResult {
                        generation: job.generation,
                        path: job.path,
                        result,
                    });
                }
            })
            .expect("failed to start full-resolution decoder");
        Self {
            state,
            results,
            worker: Some(worker),
        }
    }

    pub fn request(&self, path: PathBuf, generation: u64) {
        let (lock, ready) = &*self.state;
        let mut state = lock.lock().unwrap();
        state.latest = Some(DetailJob { path, generation });
        ready.notify_one();
    }

    pub fn try_recv(&self) -> Option<DetailResult> {
        self.results.try_recv().ok()
    }
}

impl Drop for DetailLoader {
    fn drop(&mut self) {
        let (lock, ready) = &*self.state;
        if let Ok(mut state) = lock.lock() {
            state.stopped = true;
        }
        ready.notify_all();
        drop(self.worker.take());
    }
}

pub(crate) fn decode_detail(path: &Path) -> Result<DetailImage> {
    let started = Instant::now();
    if is_raw(path) {
        bail!("camera RAW uses its developed embedded preview");
    }
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    let (size, channels, pixels) =
        if extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg") {
            decode_jpeg_mapped(path)?
        } else if extension.eq_ignore_ascii_case("png") {
            decode_png_mapped(path)?
        } else {
            decode_owned(path)?
        };
    Ok(DetailImage {
        size,
        channels,
        pixels,
        elapsed: started.elapsed(),
    })
}

fn decode_jpeg_mapped(path: &Path) -> Result<([u32; 2], usize, PixelStore)> {
    let input_file = File::open(path)?;
    // SAFETY: the input file remains open and is never mutated for the mapping lifetime.
    let input = unsafe { Mmap::map(&input_file) }.context("cannot map JPEG from SSD")?;
    let options = DecoderOptions::default()
        .set_max_width(u16::MAX as usize)
        .set_max_height(u16::MAX as usize)
        .jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(&input[..]), options);
    decoder
        .decode_headers()
        .context("cannot read full JPEG header")?;
    let (width, height) = decoder.dimensions().context("JPEG has no dimensions")?;
    let output_len = decoder
        .output_buffer_size()
        .context("JPEG full raster is too large")?;
    let mut output = create_temp_map(output_len)?;
    decoder
        .decode_into(&mut output[..])
        .context("cannot decode full-resolution JPEG")?;
    cool_mapping(&output)?;
    Ok(([width as u32, height as u32], 3, PixelStore::Mapped(output)))
}

fn decode_png_mapped(path: &Path) -> Result<([u32; 2], usize, PixelStore)> {
    let mut decoder = png::Decoder::new(BufReader::new(File::open(path)?));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let info = reader.info();
    if info.interlaced {
        return decode_owned(path);
    }
    let size = [info.width, info.height];
    let output_len = usize::try_from(u64::from(size[0]) * u64::from(size[1]) * 4)
        .context("PNG full raster is too large")?;
    let mut output = create_temp_map(output_len)?;
    let mut row = vec![
        0;
        reader
            .output_line_size(size[0])
            .context("PNG row too large")?
    ];
    let channels = match reader.output_color_type().0 {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => bail!("indexed PNG was not expanded"),
    };
    for y in 0..size[1] as usize {
        if reader.read_row(&mut row)?.is_none() {
            bail!("PNG ended before its final row");
        }
        let target = &mut output[y * size[0] as usize * 4..(y + 1) * size[0] as usize * 4];
        for (source, rgba) in row.chunks_exact(channels).zip(target.chunks_exact_mut(4)) {
            match channels {
                1 => rgba.copy_from_slice(&[source[0], source[0], source[0], 255]),
                2 => rgba.copy_from_slice(&[source[0], source[0], source[0], source[1]]),
                3 => rgba.copy_from_slice(&[source[0], source[1], source[2], 255]),
                4 => rgba.copy_from_slice(source),
                _ => unreachable!(),
            }
        }
    }
    cool_mapping(&output)?;
    Ok((size, 4, PixelStore::Mapped(output)))
}

fn decode_owned(path: &Path) -> Result<([u32; 2], usize, PixelStore)> {
    let reader = ImageReader::open(path)?.with_guessed_format()?;
    let (width, height) = reader.into_dimensions()?;
    if u64::from(width) * u64::from(height) > MAX_OWNED_PIXELS {
        bail!("this codec has no bounded full-resolution decoder yet");
    }
    let image = ImageReader::open(path)?
        .with_guessed_format()?
        .decode()?
        .into_rgba8();
    Ok(([width, height], 4, PixelStore::Owned(image.into_raw())))
}

fn create_temp_map(length: usize) -> Result<MmapMut> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "eagle-detail-{}-{}.rgba",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.set_len(length as u64)?;
    // SAFETY: this newly-created file is exclusively owned and sized to `length`.
    let mapping = unsafe { MmapMut::map_mut(&file) }.context("cannot map detail cache")?;
    let _ = std::fs::remove_file(path);
    Ok(mapping)
}

fn cool_mapping(mapping: &MmapMut) -> Result<()> {
    mapping.flush().context("cannot flush full detail to SSD")?;
    // SAFETY: decoding is complete and there are no outstanding borrows into the map.
    // The shared file mapping has been flushed, so future reads safely fault pages back from SSD.
    unsafe { mapping.unchecked_advise(UncheckedAdvice::DontNeed) }
        .context("cannot release completed detail pages")?;
    mapping
        .advise(Advice::Random)
        .context("cannot configure random tile access")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_rgb_tile_as_rgba() {
        let image = DetailImage {
            size: [2, 1],
            channels: 3,
            pixels: PixelStore::Owned(vec![10, 20, 30, 40, 50, 60]),
            elapsed: Duration::ZERO,
        };
        assert_eq!(
            image.tile_rgba(0, 0, 2, 1),
            vec![10, 20, 30, 255, 40, 50, 60, 255]
        );
    }
}
