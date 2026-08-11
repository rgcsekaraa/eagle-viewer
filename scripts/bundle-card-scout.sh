#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
app_dir="$project_dir/target/release/bundle/osx/Card Scout.app"
contents_dir="$app_dir/Contents"

cd "$project_dir"
cargo build --release --bin card-scout
mkdir -p "$contents_dir/MacOS" "$contents_dir/Resources"
cp "$project_dir/target/release/card-scout" "$contents_dir/MacOS/card-scout"
cp "$project_dir/macos/CardScout-Info.plist" "$contents_dir/Info.plist"
cp "$project_dir/macos/PkgInfo" "$contents_dir/PkgInfo"
chmod 755 "$contents_dir/MacOS/card-scout"

echo "$app_dir"
