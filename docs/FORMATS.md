# Image formats

Eagle Viewer combines Rust decoders, camera RAW parsing and the native macOS image stack.

## Direct Rust decoding

- JPEG / JPG
- PNG
- WebP
- TIFF
- GIF
- BMP
- ICO
- QOI
- TGA
- PNM, PPM and PGM
- OpenEXR
- Radiance HDR

JPEG and non-interlaced PNG have specialized large-image paths. Interlaced PNG and the remaining formats use guarded fallback decoding.

## Camera RAW

Supported RAW families include 3FR, ARI, ARW, CR2, CR3, CRW, DCR, DCS, DNG, ERF, FFF, IIQ, KDC, MEF, MOS, MRW, NEF, NRW, ORF, PEF, RAF, RAW, RW2, SR2, SRF, SRW and X3F.

The initial display uses the camera-generated embedded preview when available. Some DNG files without a suitable preview can take longer because they require native development. Full sensor-resolution RAW development and color-management controls are future work.

## Native macOS fallback

AVIF, CUR, DDS, HEIC, HEIF, ICNS, JPEG 2000, PICT, PSD/PSB, SGI, SVG and XBM are delegated to macOS Quick Look. Availability and output fidelity depend on the installed macOS release and system codecs.

## Reporting a format problem

Open a bug report with the extension, camera/application that created the file, dimensions, approximate size and macOS version. Use a redistributable sample or a synthetic reproduction; do not upload private photographs.
