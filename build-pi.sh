#!/usr/bin/env bash
# Cross-compile weather_app for Raspberry Pi OS Bookworm (64-bit) from macOS
# and package binary + assets + config into dist/weather_app-pi.tar.gz.
#
# One-time setup:
#   brew install zig
#   rustup target add aarch64-unknown-linux-gnu
#   cargo install --locked cargo-zigbuild
#
# Optional: PI_HOST=sysala@192.168.88.x ./build-pi.sh  -> also copies to the Pi.
set -euo pipefail
cd "$(dirname "$0")"

# Bookworm ships glibc 2.36; link against that so the binary runs there.
TARGET=aarch64-unknown-linux-gnu
cargo zigbuild --release --target "${TARGET}.2.36"

rm -rf dist/weather_app
mkdir -p dist/weather_app
cp "target/${TARGET}/release/weather_app" dist/weather_app/
cp -R assets dist/weather_app/
# Local config.toml (git-ignored, holds your location) wins over the example.
if [[ -f config.toml ]]; then
    cp config.toml dist/weather_app/
else
    cp config.example.toml dist/weather_app/config.toml
fi
# COPYFILE_DISABLE: no macOS ._* metadata files in the archive.
# Write then rename, so the Pi's updater never sees a half-written tarball.
COPYFILE_DISABLE=1 tar --no-xattrs --no-mac-metadata -C dist -czf dist/.weather_app-pi.tar.gz.tmp weather_app
mv -f dist/.weather_app-pi.tar.gz.tmp dist/weather_app-pi.tar.gz
echo "Built dist/weather_app-pi.tar.gz"

if [[ -n "${PI_HOST:-}" ]]; then
    scp dist/weather_app-pi.tar.gz "${PI_HOST}:~/"
    ssh "${PI_HOST}" 'tar -xzf ~/weather_app-pi.tar.gz -C ~ && echo "Installed to ~/weather_app"'
fi
