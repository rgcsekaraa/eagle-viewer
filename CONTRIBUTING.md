# Contributing to Eagle Viewer

Thank you for helping make Eagle Viewer faster, more reliable and easier to use.

## Before opening a change

1. Search existing issues and pull requests.
2. Open an issue before large architectural changes or new dependencies.
3. Keep changes focused. Performance work should include reproducible measurements.
4. Do not include test images unless their license permits redistribution.

## Development setup

Eagle Viewer currently targets macOS 12 or newer.

```sh
git clone https://github.com/rgcsekaraa/eagle-viewer.git
cd eagle-viewer
cargo build
cargo run -- "/path/to/image"
```

Before submitting a pull request, run:

```sh
cargo fmt -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
```

## Performance contributions

Please report the image format, dimensions, file size, storage type, Mac model, macOS version and exact benchmark command. Measure preview latency and full-detail latency separately. Never commit private photographs or proprietary RAW samples; link to a redistributable fixture or describe how to generate one.

## Pull requests

- Explain the user-visible result and the implementation tradeoff.
- Add or update tests when behavior changes.
- Update documentation and keyboard tables when controls change.
- Keep decoding off the UI thread.
- Preserve bounded caches and stale-job cancellation.
- Avoid panics for malformed external files.

By contributing, you agree that your contribution is licensed under the MIT License.
