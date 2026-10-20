# weather_app

Raspberry Pi weather display. Rust + [iced](https://iced.rs) 0.14 port of the Xojo `WeatherApp`
(see [PLAN.md](PLAN.md) for the full analysis of the original project).

- Clock (`H:mm`, 1 s tick), current temp, wind chill, wind speed, condition icon: [Open-Meteo](https://open-meteo.com)
- Tomorrow's max/min: Open-Meteo daily forecast
- Local temp + humidity: Pico W sensor (`GET /temperature` → `{"temperature": 54.68}`, `GET /humidity` → `{"humidity": 41.2}`).
  Rust/Embassy firmware for a Pico W + BME280 is in [pico_sensor/](pico_sensor/README.md).
- Refresh every 15 min; a failed source retries after 60 s. Esc quits. Cursor hidden.
- Background follows the sky: night, dawn, day, dusk (dawn/dusk = 30 min either side of
  Open-Meteo's sunrise/sunset) × clear (<25 % cloud), partly cloudy (25–75 %), cloudy (>75 %, or
  any fog/rain/snow). Colours blend smoothly; each label switches between light and dark text
  to stay readable. Preview a state with e.g. `WEATHER_APP_SKY=dusk-partly ./weather_app`
  (`night`, `dawn-*`, `day-*`, `dusk-*` with `clear`/`partly`/`cloudy`).

## Build & run on the Pi (Raspberry Pi OS Bookworm, 64-bit, Pi 4/5)

```bash
sudo apt install -y build-essential pkg-config cmake libxkbcommon-dev libwayland-dev mesa-vulkan-drivers fonts-liberation2
```

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

```bash
cargo build --release
```

```bash
./target/release/weather_app
```

First release build on a Pi 4 takes a while (~15–25 min); later builds are incremental.

To deploy elsewhere, copy `target/release/weather_app`, the `assets/` folder (next to the binary)
and optionally `config.toml`.

### Cross-compile from macOS (recommended: native Pi builds can run out of RAM)

One-time setup:

```bash
brew install zig && rustup target add aarch64-unknown-linux-gnu && cargo install --locked cargo-zigbuild
```

Build and package (`dist/weather_app-pi.tar.gz`, ~1 min):

```bash
./build-pi.sh
```

Build and copy straight to the Pi (unpacks to `~/weather_app`):

```bash
PI_HOST=sysala@<pi-ip> ./build-pi.sh
```

On the Pi the binary needs only the X11/Wayland/xkbcommon runtime libraries that Raspberry Pi OS
desktop already has.

### Automatic updates on the Pi

`deploy/weather-app-updater.sh` runs on the Pi as a systemd user service. It polls the tarball on
the NAS every 30 s; when `./build-pi.sh` produces a new one and Synology Drive syncs it, the
updater installs it into `~/weather_app` and restarts the app (about a minute end to end).

- Waits until the file stops changing, copies it locally, checks it is a valid aarch64 build.
- Keeps the Pi's own `config.toml` (set `UPDATE_CONFIG=1` in the service to replace it).
- Keeps the previous version as `weather_app.prev` / `assets.prev`; if the new build doesn't stay
  running for 15 s, it rolls back and ignores that tarball.
- Restarts by stopping the app; the autostart wrapper (`until ./weather_app; do sleep 5; done`)
  launches the new binary in the desktop session.

Install (on the Pi, from `deploy/`):

```bash
./install-updater.sh
```

Logs:

```bash
journalctl --user -u weather-app-updater -f
```

### Renderer

GPU (wgpu/Vulkan) is tried first, then the CPU renderer (tiny-skia). If the GPU path misbehaves:

```bash
ICED_BACKEND=tiny-skia ./target/release/weather_app
```

## Configuration

Copy [config.example.toml](config.example.toml) to `config.toml` and set your `latitude` and
`longitude` (the example location is downtown Denver; all keys are optional). `config.toml` is
git-ignored, and `build-pi.sh` packages it with the binary. Lookup order:
first CLI argument, `./config.toml`, `~/.config/weather_app/config.toml`.
Set `fullscreen = false` for an 800×480 window when testing on a desktop.

## Development

```bash
cargo test
```

Icons: [Bas Milius weather-icons](https://github.com/basmilius/weather-icons) (MIT, see `assets/icons/LICENSE`).
