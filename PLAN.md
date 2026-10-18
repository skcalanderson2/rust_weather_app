# WeatherApp: Xojo → Rust/iced port plan

## 1. What the Xojo app does (from `WeatherApp.xojo_xml_project`)

**Window1**: 800×480, gradient backdrop (`Gradient for Weather App.png`), mouse cursor hidden, Esc quits.

| Control | Pos (x,y) / size | Font | Content |
|---|---|---|---|
| `currentTime` | 20,20 / 348×115 | Arial 80, #CCCCCC | `H:mm`, updated by `TimeTimer` every 1 s |
| `currentTemperature` | 20,112 / 573×83 | Arial 48 | `Current Temp: {temperature_2m}° F` |
| `windChill` | 20,163 / 451×83 | Arial 36 | `Windchill Temp: {apparent_temperature}° F` |
| `DayNight` (canvas) | 653,20 / 120×115 | — | clear-day, overcast-day (cloud_cover ≥ 20), or clear-night |
| `windSpeedLbl` | 496,132 / 217×83, right | Arial 24 | `{wind_speed_10m as Integer} Mph` |
| `GroupBox1` | 20,321 / 144×139 | — | box around tomorrow's forecast |
| `ForecastDay1` | 20,329 / 141×20, centre | Arial 20 bold, #F7F7F7 | `EEE d` for `daily.time[1]` |
| `ForecastMaxTemp1` / `MinTemp1` | 20,355 / 20,376 | Arial 12 bold | `Max Temp: {Format(v,"-#.##")}°F` |
| `localTempText` | 176,321 / 604×52 | Arial 50 | `Local Temp: {temperature}° F` |
| `LocalHumText` | 176,385 / 187×20 | Arial 12 bold | `Local Humidity: {humidity}%` |
| `LocalUpdateTime` | 364,440 / 187×20 | Arial 12 bold | `Local Temp Last Updated: H:mm` |

**Data sources** (fired on open and every 15 min by `WeatherTimer`):

1. **Open-Meteo** (HTTPS, JSON): `current` = temperature_2m, apparent_temperature, is_day, weather_code, cloud_cover, wind_speed_10m; `daily` (3 days) = weather_code, max/min temp, precip probability. °F, mph, tz America/Denver, home latitude/longitude. On error: `currentTemperature` shows `Error getting data: …`.
2. **Pico W sensor** (plain HTTP on LAN): `GET http://192.168.88.146:5000/temperature` → `{"temperature": 54.68}` and `/humidity` → `{"humidity": 41.2}`. No error handling in Xojo (labels keep placeholder text — which is why the screenshot shows bare "Local Humidity").

**Unused in Xojo**: `WMOCodes` dictionary / `WeatherCodes` enum, `DayNight1` canvas (image ID points to nothing), daily weather_code & precip.

## 2. Decisions (confirmed with user)

- Faithful port + small fixes; 64-bit Pi 4/5 Bookworm; build natively on the Pi; TOML config.
- Fixes: icon chosen from WMO `weather_code` + `is_day` (all mapped icons verified present by a unit test); sensor values show `--` until first success and keep last good value afterwards; failed source retries after 60 s instead of 15 min; `Format "-#.##"` quirks (`70.`) fixed; wind speed rounded rather than truncated.

## 3. Rust architecture

```
src/main.rs     iced application: State, Message, update, view, subscriptions
src/weather.rs  serde models, async fetchers (reqwest + rustls), WMO → icon map, number formatting
src/config.rs   Config (serde + TOML), defaults = Xojo constants, file lookup
assets/         background.png (800×480 gradient), icons/*.png (Bas Milius weather-icons, MIT)
```

- **iced 0.14**, Elm architecture. `Message::{Tick, Refresh, ForecastLoaded, LocalTempLoaded, LocalHumLoaded, Retry(Source), Exit}`.
- Subscriptions: `time::every(1s)` → clock, `time::every(refresh_minutes)` → refresh, keyboard Esc → exit.
- Fetches run as `Task::perform` on iced's tokio executor; three independent requests so one slow/failed source never blocks the others.
- Layout: `stack![background image, pinned widgets]`; each Xojo control is placed at its exact pixel position with `pin(...).x().y()` inside an 800×480 canvas, so the screen matches the screenshot. Labels with Xojo height are vertically centred like Xojo.
- Cursor hidden via `mouse_area(...).interaction(Interaction::Hidden)`.
- Renderer: both `wgpu` (Vulkan/GL on Pi 4/5) and `tiny-skia` (CPU) compiled in; iced falls back automatically, or force with `ICED_BACKEND=tiny-skia`.

## 4. Steps

1. ✅ Parse Xojo project, extract code/layout/constants, probe Open-Meteo.
2. ✅ Scaffold crate, copy assets, config module, data module + unit tests.
3. ✅ iced UI + update loop.
4. ✅ `cargo test`, `cargo clippy`, run on macOS in windowed mode and compare against the screenshot.
5. On the Pi: install deps, `cargo build --release`, run (see README).
