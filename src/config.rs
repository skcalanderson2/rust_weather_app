//! Runtime configuration, loaded from a TOML file.
//!
//! Defaults reproduce the constants hard-coded in the original Xojo app
//! (`App.API_URL`, `App.API_URL_Local_Temp`, `App.API_URL_Local_Hum`),
//! except the location, which is an example; set yours in `config.toml`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub latitude: f64,
    pub longitude: f64,
    /// IANA timezone passed to Open-Meteo (controls daily boundaries).
    pub timezone: String,
    /// Full forecast URL. When set, overrides latitude/longitude/timezone.
    pub forecast_url: Option<String>,
    /// Combined sensor endpoint returning temperature (°C), humidity and
    /// pressure in one JSON object. When set, the three URLs below are unused.
    pub sensor_url: String,
    /// Pico W sensor endpoints (one value each).
    pub local_temp_url: String,
    pub local_humidity_url: String,
    /// BME280 station pressure in hPa (`{"pressure": 841.4}`). Empty = none.
    pub local_pressure_url: String,
    /// "inHg" (default) or "hPa".
    pub pressure_unit: String,
    /// Show sea-level pressure (default) rather than the raw station value.
    pub pressure_sea_level: bool,
    /// Sensor elevation in metres for the sea-level reduction. Defaults to
    /// Open-Meteo's grid elevation; set it if the sensor sits much higher or
    /// lower than the surrounding area.
    pub elevation_m: Option<f64>,
    /// Calibration added to every raw sensor reading (hPa), for a BME280
    /// that reads consistently high or low against a nearby station.
    pub pressure_offset_hpa: f64,
    /// How often all sources are refreshed (Xojo `WeatherTimer`: 900 000 ms).
    pub refresh_minutes: u64,
    /// After a failed request, retry that source this soon instead of
    /// waiting for the next full refresh.
    pub retry_seconds: u64,
    /// HTTP timeout for every request.
    pub http_timeout_seconds: u64,
    /// Start fullscreen (kiosk mode on the Pi's 800x480 display).
    pub fullscreen: bool,
    /// Directory containing `icons/`.
    /// Relative paths resolve against the executable's directory first,
    /// then the current working directory.
    pub assets_dir: PathBuf,
    /// Dim the screen backlight at night (Pi touch display only).
    pub dim_at_night: bool,
    /// Backlight levels in percent. Twilight ramps between them.
    pub day_brightness: f32,
    pub night_brightness: f32,
    /// Show US National Weather Service alerts (api.weather.gov) as a banner.
    pub alerts: bool,
    /// Font family for all text. The Xojo app used Arial; "Liberation Sans"
    /// is metric-compatible and ships with Raspberry Pi OS.
    pub font: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            latitude: 39.7392,
            longitude: -104.9903,
            timezone: "America/Denver".into(),
            forecast_url: None,
            sensor_url: String::new(),
            local_temp_url: "http://192.168.88.146:5000/temperature".into(),
            local_humidity_url: "http://192.168.88.146:5000/humidity".into(),
            local_pressure_url: "http://192.168.88.146:5000/pressure".into(),
            pressure_unit: "inHg".into(),
            pressure_sea_level: true,
            elevation_m: None,
            pressure_offset_hpa: 0.0,
            refresh_minutes: 15,
            retry_seconds: 60,
            http_timeout_seconds: 10,
            fullscreen: true,
            assets_dir: PathBuf::from("assets"),
            font: "Liberation Sans".into(),
            dim_at_night: true,
            day_brightness: 100.0,
            night_brightness: 30.0,
            alerts: true,
        }
    }
}

impl Config {
    /// Load config from, in order: the path given on the command line,
    /// `./config.toml`, `$XDG_CONFIG_HOME/weather_app/config.toml`
    /// (or `~/.config/weather_app/config.toml`). Falls back to defaults.
    pub fn load() -> Self {
        let candidates = std::env::args()
            .nth(1)
            .map(PathBuf::from)
            .into_iter()
            .chain([PathBuf::from("config.toml")])
            .chain(user_config_path());

        for path in candidates {
            if !path.is_file() {
                continue;
            }
            match std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|s| toml::from_str::<Config>(&s).map_err(|e| e.to_string()))
            {
                Ok(config) => {
                    eprintln!("weather_app: using config {}", path.display());
                    return config;
                }
                Err(e) => eprintln!("weather_app: ignoring {}: {e}", path.display()),
            }
        }

        eprintln!("weather_app: no config file found, using defaults");
        Self::default()
    }

    pub fn forecast_url(&self) -> String {
        if let Some(url) = &self.forecast_url {
            return url.clone();
        }
        format!(
            "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}\
             &current=temperature_2m,apparent_temperature,is_day,weather_code,cloud_cover,wind_speed_10m\
             &daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max,sunrise,sunset,wind_gusts_10m_max&hourly=temperature_2m,precipitation_probability\
             &temperature_unit=fahrenheit&wind_speed_unit=mph&precipitation_unit=inch\
             &timezone={}&forecast_days=3",
            self.latitude,
            self.longitude,
            self.timezone.replace('/', "%2F"),
        )
    }

    /// Resolve `assets_dir` to an existing directory if possible.
    pub fn resolved_assets_dir(&self) -> PathBuf {
        if self.assets_dir.is_absolute() {
            return self.assets_dir.clone();
        }
        let next_to_exe = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join(&self.assets_dir)));
        if let Some(dir) = next_to_exe.filter(|d| d.is_dir()) {
            return dir;
        }
        // Running via `cargo run`: the manifest directory holds `assets/`.
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join(&self.assets_dir);
        if !Path::new(&self.assets_dir).is_dir() && manifest.is_dir() {
            return manifest;
        }
        self.assets_dir.clone()
    }
}

fn user_config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("weather_app").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_url_matches_xojo_format() {
        // Xojo App.API_URL shape, plus sunrise,sunset for the sky background.
        let xojo = "https://api.open-meteo.com/v1/forecast?latitude=39.7392&longitude=-104.9903&current=temperature_2m,apparent_temperature,is_day,weather_code,cloud_cover,wind_speed_10m&daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max,sunrise,sunset,wind_gusts_10m_max&hourly=temperature_2m,precipitation_probability&temperature_unit=fahrenheit&wind_speed_unit=mph&precipitation_unit=inch&timezone=America%2FDenver&forecast_days=3";
        assert_eq!(Config::default().forecast_url(), xojo);
    }

    #[test]
    fn partial_toml_keeps_defaults() {
        let c: Config = toml::from_str("refresh_minutes = 5\nfullscreen = false").unwrap();
        assert_eq!(c.refresh_minutes, 5);
        assert!(!c.fullscreen);
        assert_eq!(c.local_temp_url, Config::default().local_temp_url);
    }
}
