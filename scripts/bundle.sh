#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
app_dir="$project_dir/target/release/bundle/osx/Eagle Viewer.app"
contents_dir="$app_dir/Contents"

cd "$project_dir"
cargo build --release
mkdir -p "$contents_dir/MacOS" "$contents_dir/Resources"
cp "$project_dir/target/release/eagle-viewer" "$contents_dir/MacOS/eagle-viewer"
cp "$project_dir/macos/Info.plist" "$contents_dir/Info.plist"
cp "$project_dir/macos/PkgInfo" "$contents_dir/PkgInfo"
cp "$project_dir/assets/EagleViewer.icns" "$contents_dir/Resources/EagleViewer.icns"
chmod 755 "$contents_dir/MacOS/eagle-viewer"
codesign --force --deep --sign - "$app_dir"

echo "$app_dir"
