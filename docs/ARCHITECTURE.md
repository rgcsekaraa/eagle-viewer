# Architecture

Eagle Viewer separates time-to-first-pixel from full-resolution work.

```text
open / navigate
      |
      v
folder catalog -----> neighboring-image prefetch
      |
      v
latest-request preview queue (2 workers)
      |
      +---- reduced JPEG decode
      +---- streamed PNG row sampling
      +---- embedded RAW preview
      +---- image crate / macOS Quick Look fallback
      |
      v
bounded CPU preview + GPU texture cache
      |
      v
egui / wgpu / Metal presentation
      |
      +---- zoom exceeds preview resolution?
                    |
                    v
          SSD-backed full raster worker
                    |
                    v
          visible 512 px GPU tiles only
```

## Responsiveness rules

- Decoding never runs on the UI thread.
- A new foreground request replaces queued foreground work.
- Results carry a generation number; stale results are discarded.
- Prefetch jobs have lower priority than the current image.
- The preview cache is bounded by bytes, not file count.
- Full detail is lazy and does not delay fit-to-window viewing.
- GPU detail residency is capped and old tiles are evicted.

## Preview path

JPEG uses decoder-native downscaling where possible. Non-interlaced PNG rows are sampled while streaming instead of first allocating the complete source raster. RAW files prefer an embedded developed preview. Formats handled through the native fallback are converted to a temporary PNG by macOS Quick Look.

## Full-detail path

Huge JPEG and non-interlaced PNG images decode into temporary memory-mapped files. The operating system can page these mappings through the SSD. The renderer extracts and uploads only tiles intersecting the viewport. Other formats use a guarded in-memory decode, while RAW/native fallback formats retain their developed preview today.

## Failure boundaries

Decoders run behind error handling and panic containment. A failed preview or detail pass is reported in the interface without terminating the event loop. Full-detail allocation is guarded to avoid unbounded image-crate fallbacks.
