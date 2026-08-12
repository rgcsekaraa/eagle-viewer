# Changelog

All notable changes to Eagle Viewer will be documented in this file. The project follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.2.0] - 2026-08-12

### Added

- Persistent Open Recent history for up to 12 images, with missing-file cleanup, clear history, and `Cmd+Shift+O` access.
- Automatic, black, white, and checkerboard canvas backgrounds.
- Configurable 2, 3, 5, or 10 second slideshow timing.
- A single action and `X` shortcut to reset view rotation and flips.

### Fixed

- Made the image-information card responsive and dynamically sized so long filenames and full-detail status text wrap without overflowing.

## [0.1.1] - 2026-08-12

### Added

- Standard Phosphor icons across the image-viewer toolbar.
- Immediate primary-button and middle-button mouse panning with grab cursors.
- Mouse-wheel zoom and native trackpad pinch zoom centered on the pointer.

### Fixed

- Removed drag hesitation caused by mixed click-and-drag sensing.
- Increased wheel zoom sensitivity and removed direct-manipulation animation lag.

## [0.1.0] - 2026-08-11

### Added

- Dark and light appearances with keyboard switching.
- Immediate, font-independent toolbar tooltips and vector icons.
- Complete keyboard access to viewer and file actions.
- In-app screenshot capture for project documentation.

- GPU-presented macOS image viewer built with Rust, egui and wgpu.
- Reduced-resolution first previews for huge JPEG and PNG images.
- SSD-backed full-detail rasters and visible GPU tiles.
- Camera RAW embedded previews, including Sony ARW.
- Native macOS fallback, folder prefetch, filmstrip and slideshow.

[Unreleased]: https://github.com/rgcsekaraa/eagle-viewer/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/rgcsekaraa/eagle-viewer/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/rgcsekaraa/eagle-viewer/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/rgcsekaraa/eagle-viewer/releases/tag/v0.1.0
