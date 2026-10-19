# weather_app

Raspberry Pi weather display. Rust + [iced](https://iced.rs) 0.14 port of the Xojo `WeatherApp`
(see [PLAN.md](PLAN.md) for the full analysis of the original project).

- Clock (`H:mm`, 1 s tick), current temp, wind chill, wind speed, condition icon: [Open-Meteo](https://open-meteo.com)
- Tomorrow's max/min: Open-Meteo daily forecast
- Local temp + humidity: Pico W sensor (`GET /temperature` → `{"temperature": 54.68}`, `GET /humidity` → `{"humidity": 41.2}`).
  Rust/Embassy firmware for a Pico W + BME280 is in [pico_sensor/](pico_sensor/README.md).
- Refresh every 15 min; a failed source retries after 60 s. Esc quits. Cursor hidden.

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
