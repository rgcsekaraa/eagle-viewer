# Releasing

1. Update `CHANGELOG.md`, `Cargo.toml` and both version fields in `macos/Info.plist`.
2. Run the complete validation suite:

   ```sh
   cargo fmt -- --check
   cargo test --locked
   cargo clippy --locked --all-targets -- -D warnings
   cargo build --release --locked
   ./scripts/bundle.sh
   codesign --verify --deep --strict "target/release/bundle/osx/Eagle Viewer.app"
   ```

3. Test JPEG, PNG, RAW and a native fallback format on supported macOS versions.
4. Create and push an annotated `vMAJOR.MINOR.PATCH` tag.
5. Create a GitHub release containing the changelog entry and packaged application archive.

The local bundling script uses an ad-hoc signature. Public binary distribution should use a Developer ID certificate, hardened runtime and Apple notarization.
