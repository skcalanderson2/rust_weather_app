//! Observed conditions, to correct the forecast model where it can't see
//! local weather: valley inversions (fog, low overcast) are often smaller
//! than a model grid cell, so Open-Meteo reports clear sky under a cloud
//! deck.
//!
//! Two sources:
//! - the latest report from a nearby National Weather Service airport
//!   station (ASOS/AWOS, ids starting with "K"): present weather (fog, mist,
//!   rain, snow), visibility and cloud layers;
//! - the outdoor sensor itself: air within ~2 °F of its dew point in light
//!   wind is fog.

use chrono::{DateTime, FixedOffset, TimeDelta, Utc};
use serde::Deserialize;

use crate::alerts::user_agent;

/// Station reports older than this are ignored.
pub const MAX_AGE: TimeDelta = TimeDelta::minutes(90);

/// What an observation says, reduced to what the display needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    pub station: String,
    pub time: DateTime<FixedOffset>,
    /// WMO weather code equivalent (0 clear .. 45 fog, 61 rain, ...).
    pub code: u8,
    /// Cloud cover in percent from the cloud layers.
    pub cloud_cover: f64,
    /// Short description for the source label, e.g. "Overcast".
    pub summary: &'static str,
}

impl Observation {
    pub fn is_recent(&self, now: DateTime<Utc>) -> bool {
        now.signed_duration_since(self.time) <= MAX_AGE
    }
}

// ---------- NWS JSON ----------

#[derive(Debug, Deserialize)]
struct StationList {
    features: Vec<StationFeature>,
}

#[derive(Debug, Deserialize)]
struct StationFeature {
    properties: StationProps,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StationProps {
    station_identifier: String,
}

#[derive(Debug, Deserialize)]
struct PointResponse {
    properties: PointProps,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PointProps {
    observation_stations: String,
}

#[derive(Debug, Deserialize)]
struct ObservationResponse {
    properties: ObsProps,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ObsProps {
    timestamp: DateTime<FixedOffset>,
    #[serde(default)]
    present_weather: Vec<PresentWeather>,
    #[serde(default)]
    visibility: Option<Value>,
    #[serde(default)]
    cloud_layers: Vec<CloudLayer>,
}

#[derive(Debug, Deserialize)]
struct PresentWeather {
    #[serde(default)]
    weather: Option<String>,
    #[serde(default)]
    intensity: Option<String>,
    #[serde(default)]
    modifier: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CloudLayer {
    amount: String,
}

#[derive(Debug, Deserialize)]
struct Value {
    value: Option<f64>,
}

// ---------- Fetching ----------

async fn get<T: serde::de::DeserializeOwned>(client: &reqwest::Client, url: &str) -> Result<T, String> {
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, user_agent())
        .header(reqwest::header::ACCEPT, "application/geo+json")
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| {
            if e.is_timeout() {
                "timed out".into()
            } else if let Some(s) = e.status() {
                format!("HTTP {s}")
            } else {
                "cannot connect".into()
            }
        })?;
    let body = response.text().await.map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| format!("bad JSON: {e}"))
}

/// The nearest NWS airport station to a point (the stations list is
/// ordered by distance).
pub async fn nearest_station(client: reqwest::Client, latitude: f64, longitude: f64) -> Result<String, String> {
    let point: PointResponse =
        get(&client, &format!("https://api.weather.gov/points/{latitude:.4},{longitude:.4}")).await?;
    let stations: StationList = get(&client, &point.properties.observation_stations).await?;
    stations
        .features
        .into_iter()
        .map(|f| f.properties.station_identifier)
        .find(|id| id.len() == 4 && id.starts_with('K'))
        .ok_or_else(|| "no airport station nearby".into())
}

pub async fn latest(client: reqwest::Client, station: String) -> Result<Observation, String> {
    let url = format!("https://api.weather.gov/stations/{station}/observations/latest");
    let obs: ObservationResponse = get(&client, &url).await?;
    Ok(interpret(station, obs.properties))
}

// ---------- Interpretation ----------

/// Cloud cover in percent for a METAR amount.
fn amount_percent(amount: &str) -> Option<f64> {
    Some(match amount {
        "SKC" | "CLR" => 0.0,
        "FEW" => 20.0,
        "SCT" => 45.0,
        "BKN" => 75.0,
        "OVC" | "VV" => 100.0, // VV = vertical visibility: sky obscured (fog)
        _ => return None,
    })
}

fn interpret(station: String, p: ObsProps) -> Observation {
    let cloud_cover = p.cloud_layers.iter().filter_map(|l| amount_percent(&l.amount)).fold(0.0, f64::max);
    let obscured = p.cloud_layers.iter().any(|l| l.amount == "VV");
    let visibility = p.visibility.and_then(|v| v.value);

    // Present weather, most significant first.
    let mut code: Option<(u8, &'static str)> = None;
    let mut fog = obscured || visibility.is_some_and(|v| v < 1000.0);
    for w in &p.present_weather {
        let kind = w.weather.as_deref().unwrap_or("");
        let heavy = w.intensity.as_deref() == Some("heavy");
        let freezing = w.modifier.as_deref() == Some("freezing");
        let found = match kind {
            k if k.contains("thunder") => Some((95, "Thunderstorms")),
            k if k.contains("snow") => Some((if heavy { 75 } else { 73 }, "Snow")),
            k if k.contains("rain") && freezing => Some((66, "Freezing rain")),
            k if k.contains("rain") => Some((if heavy { 65 } else { 63 }, "Rain")),
            k if k.contains("drizzle") => Some((53, "Drizzle")),
            k if k.contains("fog") || k.contains("mist") => {
                fog = true;
                None
            }
            _ => None,
        };
        if found.is_some_and(|(c, _)| code.is_none_or(|(old, _)| c > old)) {
            code = found;
        }
    }

    let (code, summary) = code.unwrap_or(if fog {
        (45, "Fog")
    } else {
        match cloud_cover {
            c if c >= 100.0 => (3, "Overcast"),
            c if c >= 75.0 => (3, "Mostly cloudy"),
            c if c >= 45.0 => (2, "Partly cloudy"),
            c if c > 0.0 => (1, "Mostly clear"),
            _ => (0, "Clear"),
        }
    });
    Observation { station, time: p.timestamp, code, cloud_cover, summary }
}

/// Dew point (°F) from temperature (°F) and relative humidity, Magnus
/// formula.
pub fn dew_point_f(temp_f: f64, humidity: f64) -> f64 {
    let t = (temp_f - 32.0) * 5.0 / 9.0;
    let (a, b) = (17.62, 243.12);
    let g = (humidity.clamp(1.0, 100.0) / 100.0).ln() + a * t / (b + t);
    let dew_c = b * g / (a - g);
    dew_c * 9.0 / 5.0 + 32.0
}

/// Fog at the sensor: temperature within `max_spread_f` of the dew point
/// and wind light enough for it to persist. `max_spread_f <= 0` disables.
pub fn local_fog(temp_f: f64, humidity: f64, wind_mph: f64, max_spread_f: f64) -> bool {
    max_spread_f > 0.0 && wind_mph <= 7.0 && temp_f - dew_point_f(temp_f, humidity) <= max_spread_f
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(json: &str) -> Observation {
        let r: ObservationResponse = serde_json::from_str(json).unwrap();
        interpret("KGPI".into(), r.properties)
    }

    // Trimmed real KGPI reports from 2026-10-30/31.
    const OVERCAST: &str = r#"{"properties":{"timestamp":"2026-10-31T22:55:00+00:00","textDescription":"Cloudy","presentWeather":[],"visibility":{"unitCode":"wmoUnit:m","value":16093.44},"cloudLayers":[{"base":{"unitCode":"wmoUnit:m","value":426.72},"amount":"OVC"}]}}"#;
    const MIST: &str = r#"{"properties":{"timestamp":"2026-10-31T11:07:00+00:00","presentWeather":[{"intensity":null,"modifier":null,"rawString":"BR","weather":"fog_mist"}],"visibility":{"value":4828.0},"cloudLayers":[{"base":{"value":152.4},"amount":"BKN"}]}}"#;
    const FREEZING_FOG: &str = r#"{"properties":{"timestamp":"2026-10-30T16:20:00+00:00","presentWeather":[{"intensity":null,"modifier":"freezing","rawString":"FZFG","weather":"fog"}],"visibility":{"value":402.0},"cloudLayers":[{"base":{"value":30.0},"amount":"VV"}]}}"#;
    const RAIN: &str = r#"{"properties":{"timestamp":"2026-10-31T05:00:00+00:00","presentWeather":[{"intensity":"light","modifier":null,"rawString":"-RA","weather":"rain"}],"visibility":{"value":16093.44},"cloudLayers":[{"amount":"BKN"},{"amount":"OVC"}]}}"#;
    const CLEAR: &str = r#"{"properties":{"timestamp":"2026-10-29T20:00:00+00:00","presentWeather":[],"visibility":{"value":16093.44},"cloudLayers":[{"base":{"value":null},"amount":"CLR"}]}}"#;

    #[test]
    fn interprets_real_reports() {
        let o = obs(OVERCAST);
        assert_eq!((o.code, o.cloud_cover, o.summary), (3, 100.0, "Overcast"));
        assert_eq!(obs(MIST).code, 45);
        assert_eq!(obs(FREEZING_FOG).summary, "Fog");
        let r = obs(RAIN);
        assert_eq!((r.code, r.summary, r.cloud_cover), (63, "Rain", 100.0));
        assert_eq!(obs(CLEAR).code, 0);
    }

    #[test]
    fn stale_reports_are_ignored() {
        let o = obs(OVERCAST);
        let t = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        assert!(o.is_recent(t("2026-10-31T23:30:00Z")));
        assert!(!o.is_recent(t("2026-11-01T01:00:00Z")));
    }

    #[test]
    fn dew_point_and_local_fog() {
        // 41.3 °F at 87.7 % -> dew point ~37.9 °F (spread ~3.4 °F): not fog.
        let dp = dew_point_f(41.342, 87.71);
        assert!((dp - 37.9).abs() < 0.3, "{dp}");
        assert!(!local_fog(41.342, 87.71, 2.7, 2.0));
        // 39 °F at 95 % -> spread ~1.4 °F in calm air: fog.
        assert!(local_fog(39.0, 95.0, 2.0, 2.0));
        // Same air but windy: mixed out, not fog.
        assert!(!local_fog(39.0, 95.0, 12.0, 2.0));
        // Disabled.
        assert!(!local_fog(39.0, 100.0, 0.0, 0.0));
    }
}
