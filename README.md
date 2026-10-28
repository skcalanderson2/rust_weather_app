# weather_app

Raspberry Pi weather display. Rust + [iced](https://iced.rs) 0.14 port of the Xojo `WeatherApp`
(see [PLAN.md](PLAN.md) for the full analysis of the original project).

- Clock (`H:mm`, 1 s tick), current temp, wind chill, wind speed, condition icon: [Open-Meteo](https://open-meteo.com)
- Tomorrow's max/min: Open-Meteo daily forecast
- Refresh every 15 min; a failed source retries after 60 s. Esc quits. Cursor hidden.
- Background follows the sky: night, dawn, day, dusk (dawn/dusk = 30 min either side of
  Open-Meteo's sunrise/sunset) × clear (<25 % cloud), partly cloudy (25–75 %), cloudy (>75 %, or
  any fog/rain/snow). Colours blend smoothly; each label switches between light and dark text
  to stay readable. Preview a state with e.g. `WEATHER_APP_SKY=dusk-partly ./weather_app`
  (`night`, `dawn-*`, `day-*`, `dusk-*` with `clear`/`partly`/`cloudy`).
- Weather icons switch day/night exactly at sunrise/sunset; light icons get a soft halo on bright
  skies. Sunrise and sunset times are shown under the wind speed.
- Forecast box: tomorrow's high/low, weather icon and chance of precipitation.
- Values older than 30 minutes (forecast or sensor) are drawn faded, so an outage is visible.
- US National Weather Service alerts for your location appear as a banner (red = warning,
  orange = watch, amber = advisory). Disable with `alerts = false`.
- Outdoor BME280 sensor: temperature, humidity and pressure. Set `sensor_url` for a combined
  endpoint returning `{"temperature_c":..,"humidity_pct":..,"pressure_hpa":..}`, or the
  per-value `local_*_url`s. Pressure is shown at sea level in inHg (`pressure_unit`,
  `pressure_sea_level`, `elevation_m`, `pressure_offset_hpa` for calibration) with a 3-hour trend
  arrow (↑ rising, → steady, ↓ falling; ±1 hPa).
- Sensor history: a sample every 5 minutes, kept 48 hours in
  `~/.local/state/weather_app/history.json` (survives restarts and updates).
- Tap the screen to cycle pages: forecast (3-day forecast with high/low, precipitation and gusts,
  the alert headline, next-24-hours chart) → outdoor sensor (last 24 hours of temperature,
  humidity and pressure) → back. Any page returns to the main screen after a minute.
- Night dimming: the Pi touch display's backlight follows the sky (`day_brightness`,
  `night_brightness`, `dim_at_night` in `config.toml`); full brightness while the details page
  is open.
- Previews: `WEATHER_APP_PAGE=forecast` or `sensor` opens that page, `WEATHER_APP_ALERT="Winter Storm
  Warning"` shows a test banner.

## Build & run on the Pi (Raspberry Pi OS Bookworm, 64-bit; tested on a Pi 3 B+)

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
