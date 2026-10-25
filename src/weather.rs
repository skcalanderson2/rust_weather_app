//! Data sources: the Open-Meteo forecast API and the local Pico W sensor.

use std::time::Duration;

use chrono::{NaiveDate, NaiveDateTime};
use serde::{Deserialize, Deserializer};

// ---------- Open-Meteo ----------

#[derive(Debug, Clone, Deserialize)]
pub struct Forecast {
    pub current: Current,
    pub daily: Daily,
    /// Absent when a custom `forecast_url` doesn't request hourly data.
    #[serde(default)]
    pub hourly: Hourly,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Current {
    pub temperature_2m: f64,
    pub apparent_temperature: f64,
    pub is_day: u8,
    pub weather_code: u8,
    pub cloud_cover: f64,
    pub wind_speed_10m: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Daily {
    pub time: Vec<NaiveDate>,
    #[serde(default)]
    pub weather_code: Vec<Option<u8>>,
    pub temperature_2m_max: Vec<Option<f64>>,
    pub temperature_2m_min: Vec<Option<f64>>,
    #[serde(default)]
    pub precipitation_probability_max: Vec<Option<f64>>,
    #[serde(default)]
    pub wind_gusts_10m_max: Vec<Option<f64>>,
    /// Local times at the configured timezone, e.g. "2026-10-20T07:15".
    #[serde(default, deserialize_with = "local_times")]
    pub sunrise: Vec<Option<NaiveDateTime>>,
    #[serde(default, deserialize_with = "local_times")]
    pub sunset: Vec<Option<NaiveDateTime>>,
}

/// Hour-by-hour forecast (3 days = 72 entries).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Hourly {
    #[serde(default, deserialize_with = "local_times")]
    pub time: Vec<Option<NaiveDateTime>>,
    #[serde(default)]
    pub temperature_2m: Vec<Option<f64>>,
    #[serde(default)]
    pub precipitation_probability: Vec<Option<f64>>,
}

/// One hour of the forecast.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hour {
    pub time: NaiveDateTime,
    pub temperature: f64,
    pub precipitation: Option<f64>,
}

impl Hourly {
    /// The `count` hours starting with the one containing `now`.
    pub fn next(&self, now: NaiveDateTime, count: usize) -> Vec<Hour> {
        let start = now - chrono::TimeDelta::hours(1);
        (0..self.time.len())
            .filter_map(|i| {
                Some(Hour {
                    time: self.time[i]?,
                    temperature: self.temperature_2m.get(i).copied().flatten()?,
                    precipitation: self.precipitation_probability.get(i).copied().flatten(),
                })
            })
            .filter(|h| h.time > start)
            .take(count)
            .collect()
    }
}

/// Open-Meteo sends ISO local times without seconds ("2026-10-20T07:15"),
/// which chrono's default `NaiveDateTime` deserializer rejects.
fn local_times<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Option<NaiveDateTime>>, D::Error> {
    let raw: Vec<Option<String>> = Deserialize::deserialize(d)?;
    Ok(raw
        .into_iter()
        .map(|s| s.and_then(|s| NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M").ok()))
        .collect())
}

/// One forecast day (the box at the bottom left, and the details page).
#[derive(Debug, Clone, PartialEq)]
pub struct DayForecast {
    pub date: NaiveDate,
    pub code: Option<u8>,
    pub max: Option<f64>,
    pub min: Option<f64>,
    pub precipitation: Option<f64>,
    pub gusts: Option<f64>,
}

impl Daily {
    pub fn day(&self, index: usize) -> Option<DayForecast> {
        Some(DayForecast {
            date: *self.time.get(index)?,
            code: self.weather_code.get(index).copied().flatten(),
            max: self.temperature_2m_max.get(index).copied().flatten(),
            min: self.temperature_2m_min.get(index).copied().flatten(),
            precipitation: self.precipitation_probability_max.get(index).copied().flatten(),
            gusts: self.wind_gusts_10m_max.get(index).copied().flatten(),
        })
    }

    /// The forecast for a calendar date. Looking up by date (not index)
    /// keeps "tomorrow" right between midnight and the next refresh, when
    /// index 1 of the last fetch is already today.
    pub fn on(&self, date: NaiveDate) -> Option<DayForecast> {
        self.day(self.time.iter().position(|d| *d == date)?)
    }

    /// Sunrise and sunset for `date` (normally today).
    pub fn sun_times(&self, date: NaiveDate) -> Option<(NaiveDateTime, NaiveDateTime)> {
        let i = self.time.iter().position(|d| *d == date)?;
        Some((self.sunrise.get(i).copied().flatten()?, self.sunset.get(i).copied().flatten()?))
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

/// One decimal, trailing ".0" dropped (50.72 -> "50.7", 51.0 -> "51").
pub fn fmt_1dp(v: f64) -> String {
    let s = format!("{v:.1}");
    let s = s.strip_suffix(".0").unwrap_or(&s);
    if s == "-0" { "0".into() } else { s.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"latitude":39.73915,"longitude":-104.984703,"generationtime_ms":0.9,"utc_offset_seconds":-21600,"timezone":"America/Denver","timezone_abbreviation":"GMT-6","elevation":892.0,"current_units":{},"current":{"time":"2026-10-16T12:00","interval":900,"temperature_2m":51.7,"apparent_temperature":48.7,"is_day":1,"weather_code":1,"cloud_cover":31,"wind_speed_10m":1.9},"daily_units":{},"daily":{"time":["2026-10-16","2026-10-17","2026-10-18"],"weather_code":[3,3,3],"temperature_2m_max":[64.7,69.1,54.8],"temperature_2m_min":[36.2,45.7,42.0],"precipitation_probability_max":[0,4,1],"sunrise":["2026-10-16T07:42","2026-10-17T07:43","2026-10-18T07:45"],"sunset":["2026-10-16T18:33","2026-10-17T18:31","2026-10-18T18:29"]},"hourly":{"time":["2026-10-16T12:00","2026-10-16T13:00","2026-10-16T14:00","2026-10-16T15:00"],"temperature_2m":[51.7,52.0,52.4,null],"precipitation_probability":[0,5,10,15]}}"#;

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
        assert_eq!(f.current.cloud_cover, 31.0);
        let (sr, ss) = f.daily.sun_times(NaiveDate::from_ymd_opt(2026, 10, 17).unwrap()).unwrap();
        assert_eq!(sr.format("%H:%M").to_string(), "07:43");
        assert_eq!(ss.format("%H:%M").to_string(), "18:31");
    }

    #[test]
    fn parses_sensor_responses() {
        let t: TemperatureReading = serde_json::from_str(r#"{"temperature": 54.68}"#).unwrap();
        let h: HumidityReading = serde_json::from_str(r#"{"humidity": 41.2}"#).unwrap();
        assert_eq!(t.temperature, 54.68);
        assert_eq!(h.humidity, 41.2);
    }

    #[test]
    fn one_decimal_formatting() {
        assert_eq!(fmt_1dp(50.72), "50.7");
        assert_eq!(fmt_1dp(51.0), "51");
        assert_eq!(fmt_1dp(68.75), "68.8");
        assert_eq!(fmt_1dp(-0.04), "0");
    }

    #[test]
    fn tomorrow_by_date_survives_midnight() {
        let f: Forecast = serde_json::from_str(SAMPLE).unwrap();
        let d = |m, day| NaiveDate::from_ymd_opt(2026, m, day).unwrap();
        // Fetched on the 16th; after midnight "tomorrow" is the 18th, not index 1.
        assert_eq!(f.daily.on(d(10, 18)).unwrap().max, Some(54.8));
        assert_eq!(f.daily.on(d(10, 17)).unwrap().code, Some(3));
        assert_eq!(f.daily.on(d(10, 17)).unwrap().precipitation, Some(4.0));
        assert!(f.daily.on(d(10, 19)).is_none());
    }

    #[test]
    fn hourly_next_starts_at_current_hour() {
        let f: Forecast = serde_json::from_str(SAMPLE).unwrap();
        let now = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap().and_hms_opt(13, 20, 0).unwrap();
        let hours = f.hourly.next(now, 2);
        assert_eq!(hours.len(), 2);
        assert_eq!(hours[0].time.format("%H:%M").to_string(), "13:00");
        assert_eq!(hours[0].temperature, 52.0);
        assert_eq!(hours[1].precipitation, Some(10.0));
    }

    #[test]
    fn formatting_matches_xojo() {
        assert_eq!(fmt_plain(51.1), "51.1");
        assert_eq!(fmt_plain(54.68), "54.68");
        assert_eq!(fmt_plain(51.0), "51");
    }

    #[test]
    fn every_icon_exists() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons");
        assert!(dir.join("wind.svg").is_file(), "missing wind.svg");
        let codes = [
            0, 1, 2, 3, 45, 48, 51, 53, 55, 56, 57, 61, 63, 65, 66, 67, 71, 73, 75, 77, 80, 81,
            82, 85, 86, 95, 96, 99, 200,
        ];
        for code in codes {
            for day in [true, false] {
                let name = icon_name(code, day);
                assert!(dir.join(format!("{name}.png")).is_file(), "missing {name}.png");
                assert!(dir.join(format!("outlined/{name}.png")).is_file(), "missing outlined/{name}.png");
                assert!(dir.join(format!("outlined-fine/{name}.png")).is_file(), "missing outlined-fine/{name}.png");
            }
        }
    }
}
