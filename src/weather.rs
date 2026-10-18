//! Data sources: the Open-Meteo forecast API and the local Pico W sensor.

use std::time::Duration;

use chrono::NaiveDate;
use serde::Deserialize;

// ---------- Open-Meteo ----------

#[derive(Debug, Clone, Deserialize)]
pub struct Forecast {
    pub current: Current,
    pub daily: Daily,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Current {
    pub temperature_2m: f64,
    pub apparent_temperature: f64,
    pub is_day: u8,
    pub weather_code: u8,
    pub wind_speed_10m: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Daily {
    pub time: Vec<NaiveDate>,
    pub temperature_2m_max: Vec<Option<f64>>,
    pub temperature_2m_min: Vec<Option<f64>>,
}

/// One forecast day, as shown in the box at the bottom left.
#[derive(Debug, Clone, PartialEq)]
pub struct DayForecast {
    pub date: NaiveDate,
    pub max: Option<f64>,
    pub min: Option<f64>,
}

impl Daily {
    /// Xojo `UpdateForecast` displays index 1 (tomorrow).
    pub fn day(&self, index: usize) -> Option<DayForecast> {
        Some(DayForecast {
            date: *self.time.get(index)?,
            max: self.temperature_2m_max.get(index).copied().flatten(),
            min: self.temperature_2m_min.get(index).copied().flatten(),
        })
    }
}

// ---------- Pico W sensor ----------

#[derive(Debug, Deserialize)]
struct TemperatureReading {
    temperature: f64,
}

#[derive(Debug, Deserialize)]
struct HumidityReading {
    humidity: f64,
}

// ---------- Fetching ----------

pub fn client(timeout_secs: u64) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .user_agent(concat!("weather_app/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("failed to build HTTP client")
}

async fn get_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
) -> Result<T, String> {
    let response = client
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(describe)?;
    let body = response.text().await.map_err(describe)?;
    serde_json::from_str(&body).map_err(|e| format!("bad JSON: {e}"))
}

pub async fn fetch_forecast(client: reqwest::Client, url: String) -> Result<Forecast, String> {
    get_json(&client, &url).await
}

pub async fn fetch_local_temp(client: reqwest::Client, url: String) -> Result<f64, String> {
    get_json::<TemperatureReading>(&client, &url)
        .await
        .map(|r| r.temperature)
}

pub async fn fetch_local_humidity(client: reqwest::Client, url: String) -> Result<f64, String> {
    get_json::<HumidityReading>(&client, &url)
        .await
        .map(|r| r.humidity)
}

fn describe(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "timed out".into()
    } else if e.is_connect() {
        "cannot connect".into()
    } else if let Some(status) = e.status() {
        format!("HTTP {status}")
    } else {
        e.to_string()
    }
}

// ---------- Icons ----------

/// Map a WMO weather code (Open-Meteo `weather_code`) to a Bas Milius
/// "fill" icon file name (without extension).
///
/// The Xojo app built a WMO dictionary but only ever chose between
/// clear-day / overcast-day / clear-night using `cloud_cover`; this uses
/// the code itself so rain, snow, fog and storms get the right picture.
pub fn icon_name(weather_code: u8, is_day: bool) -> String {
    let dn = if is_day { "day" } else { "night" };
    match weather_code {
        0 => format!("clear-{dn}"),
        1 | 2 => format!("partly-cloudy-{dn}"),
        3 => format!("overcast-{dn}"),
        45 | 48 => format!("fog-{dn}"),
        51 | 53 => format!("partly-cloudy-{dn}-drizzle"),
        55 => "drizzle".into(),
        56 | 57 | 66 | 67 => "sleet".into(),
        61 | 80 | 81 => format!("partly-cloudy-{dn}-rain"),
        63 | 65 => "rain".into(),
        82 => format!("extreme-{dn}-rain"),
        71 | 85 => format!("partly-cloudy-{dn}-snow"),
        73 | 75 | 77 | 86 => "snow".into(),
        95 => format!("thunderstorms-{dn}"),
        96 | 99 => format!("thunderstorms-{dn}-rain"),
        _ => "not-available".into(),
    }
}

// ---------- Formatting ----------

/// Xojo `Double.ToString`: shortest round-trip representation
/// (51.1 -> "51.1", 54.68 -> "54.68", 51.0 -> "51").
pub fn fmt_plain(v: f64) -> String {
    format!("{v}")
}

/// Xojo `Format(v, "-#.##")`: at most two decimals, no trailing zeros.
/// (Fixes Xojo's "70." / ".5" output for whole numbers and |v| < 1.)
pub fn fmt_2dp(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"latitude":39.73915,"longitude":-104.984703,"generationtime_ms":0.9,"utc_offset_seconds":-21600,"timezone":"America/Denver","timezone_abbreviation":"GMT-6","elevation":892.0,"current_units":{},"current":{"time":"2026-10-16T12:00","interval":900,"temperature_2m":51.7,"apparent_temperature":48.7,"is_day":1,"weather_code":1,"cloud_cover":31,"wind_speed_10m":1.9},"daily_units":{},"daily":{"time":["2026-10-16","2026-10-17","2026-10-18"],"weather_code":[3,3,3],"temperature_2m_max":[64.7,69.1,54.8],"temperature_2m_min":[36.2,45.7,42.0],"precipitation_probability_max":[0,4,1]}}"#;

    #[test]
    fn parses_open_meteo_response() {
        let f: Forecast = serde_json::from_str(SAMPLE).unwrap();
        assert_eq!(f.current.temperature_2m, 51.7);
        assert_eq!(f.current.is_day, 1);
        let tomorrow = f.daily.day(1).unwrap();
        assert_eq!(tomorrow.date, NaiveDate::from_ymd_opt(2026, 10, 17).unwrap());
        assert_eq!(tomorrow.max, Some(69.1));
        assert_eq!(tomorrow.min, Some(45.7));
        assert!(f.daily.day(5).is_none());
    }

    #[test]
    fn parses_sensor_responses() {
        let t: TemperatureReading = serde_json::from_str(r#"{"temperature": 54.68}"#).unwrap();
        let h: HumidityReading = serde_json::from_str(r#"{"humidity": 41.2}"#).unwrap();
        assert_eq!(t.temperature, 54.68);
        assert_eq!(h.humidity, 41.2);
    }

    #[test]
    fn formatting_matches_xojo() {
        assert_eq!(fmt_plain(51.1), "51.1");
        assert_eq!(fmt_plain(54.68), "54.68");
        assert_eq!(fmt_plain(51.0), "51");
        assert_eq!(fmt_2dp(69.1), "69.1");
        assert_eq!(fmt_2dp(45.678), "45.68");
        assert_eq!(fmt_2dp(70.0), "70");
        assert_eq!(fmt_2dp(-3.5), "-3.5");
        assert_eq!(fmt_2dp(-0.001), "0");
    }

    #[test]
    fn every_icon_exists() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons");
        let codes = [
            0, 1, 2, 3, 45, 48, 51, 53, 55, 56, 57, 61, 63, 65, 66, 67, 71, 73, 75, 77, 80, 81,
            82, 85, 86, 95, 96, 99, 200,
        ];
        for code in codes {
            for day in [true, false] {
                let name = icon_name(code, day);
                assert!(dir.join(format!("{name}.png")).is_file(), "missing {name}.png");
            }
        }
    }
}
