//! Active alerts from the US National Weather Service (api.weather.gov).

use std::cmp::Ordering;

use chrono::{DateTime, FixedOffset, Local};
use serde::Deserialize;

/// Alert tier, derived from the event name ("... Warning", "... Watch", ...).
#[derive(Debug, Clone, PartialEq)]
pub enum Level {
    Warning,
    Watch,
    Advisory,
    Statement,
}

impl Level {
    /// Classify an NWS event name by its last word.
    pub fn from_event(event: &str) -> Level {
        match event.trim().rsplit(' ').next().unwrap_or("") {
            "Warning" => Level::Warning,
            "Watch" => Level::Watch,
            "Advisory" => Level::Advisory,
            _ => Level::Statement,
        }
    }

    fn rank(&self) -> u8 {
        match self {
            Level::Warning => 0,
            Level::Watch => 1,
            Level::Advisory => 2,
            Level::Statement => 3,
        }
    }
}

/// One active alert, reduced to what the banner needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub event: String,
    pub headline: Option<String>,
    pub description: Option<String>,
    /// NWS severity: "Extreme", "Severe", "Moderate", "Minor" or "Unknown".
    pub severity: String,
    pub level: Level,
    /// `properties.ends`, falling back to `properties.expires`.
    pub ends: Option<DateTime<FixedOffset>>,
}

// ---------- GeoJSON response ----------

#[derive(Debug, Deserialize)]
struct Collection {
    #[serde(default)]
    features: Vec<Feature>,
}

#[derive(Debug, Deserialize)]
struct Feature {
    properties: Properties,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Properties {
    #[serde(default)]
    event: String,
    headline: Option<String>,
    description: Option<String>,
    #[serde(default)]
    severity: Option<String>,
    #[serde(default)]
    status: String,
    #[serde(default)]
    message_type: String,
    ends: Option<DateTime<FixedOffset>>,
    expires: Option<DateTime<FixedOffset>>,
}

fn parse(body: &str) -> Result<Vec<Alert>, String> {
    let collection: Collection =
        serde_json::from_str(body).map_err(|e| format!("bad JSON: {e}"))?;
    Ok(collection
        .features
        .into_iter()
        .map(|f| f.properties)
        .filter(|p| p.status == "Actual" && p.message_type != "Cancel")
        .map(|p| Alert {
            level: Level::from_event(&p.event),
            event: p.event,
            headline: p.headline,
            description: p.description,
            severity: p.severity.unwrap_or_else(|| "Unknown".into()),
            ends: p.ends.or(p.expires),
        })
        .collect())
}

// ---------- Fetching ----------

/// NWS asks for an identifying User-Agent; no contact email on purpose.
pub fn user_agent() -> &'static str {
    "weather_app (https://github.com/skcalanderson2/rust_weather_app)"
}

/// Active, actual (non-test, non-cancelled) alerts covering a point.
pub async fn fetch_alerts(
    client: reqwest::Client,
    latitude: f64,
    longitude: f64,
) -> Result<Vec<Alert>, String> {
    // NWS rejects points with more than 4 decimal places.
    let url = format!("https://api.weather.gov/alerts/active?point={latitude:.4},{longitude:.4}");
    let response = client
        .get(&url)
        .header(reqwest::header::USER_AGENT, user_agent())
        .header(reqwest::header::ACCEPT, "application/geo+json")
        .send()
        .await
        .map_err(describe)?;
    let status = response.status();
    if status == reqwest::StatusCode::BAD_REQUEST || status == reqwest::StatusCode::NOT_FOUND {
        // e.g. 400 "Parameter \"point\" is invalid: out of bounds" outside the US.
        return Err("location not covered by NWS".into());
    }
    let response = response.error_for_status().map_err(describe)?;
    let body = response.text().await.map_err(describe)?;
    parse(&body)
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

// ---------- Ranking and display ----------

fn severity_rank(severity: &str) -> u8 {
    match severity {
        "Extreme" => 0,
        "Severe" => 1,
        "Moderate" => 2,
        "Minor" => 3,
        _ => 4,
    }
}

/// Most important alert first: Level order Warning > Watch > Advisory > Statement,
/// then severity Extreme > Severe > Moderate > Minor > Unknown, then soonest ends.
pub fn sorted(mut alerts: Vec<Alert>) -> Vec<Alert> {
    alerts.sort_by(|a, b| {
        a.level
            .rank()
            .cmp(&b.level.rank())
            .then_with(|| severity_rank(&a.severity).cmp(&severity_rank(&b.severity)))
            .then_with(|| match (a.ends, b.ends) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            })
    });
    alerts
}

/// Short banner text, e.g. "Winter Storm Warning until Tue 6:00 PM (+2 more)".
pub fn banner_text(alert: &Alert, more: usize) -> String {
    let mut text = alert.event.clone();
    if let Some(ends) = alert.ends {
        let local = ends.with_timezone(&Local);
        text.push_str(&format!(" until {}", local.format("%a %-I:%M %p")));
    }
    if more > 0 {
        text.push_str(&format!(" (+{more} more)"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real `alerts/active?area=AK` response (descriptions cut,
    /// zone lists and geometry dropped); the last two features are synthetic
    /// test/cancel messages that must be skipped.
    const SAMPLE: &str = r#"{
        "@context": {"@version": "1.1"},
        "type": "FeatureCollection",
        "features": [
            {
                "id": "https://api.weather.gov/alerts/urn:oid:2.49.0.1.840.0.a0d91367b1dc8e3ef500730c87574d927b68d1d0.002.1",
                "type": "Feature",
                "geometry": null,
                "properties": {
                    "id": "urn:oid:2.49.0.1.840.0.a0d91367b1dc8e3ef500730c87574d927b68d1d0.002.1",
                    "areaDesc": "Northern Copper Valley",
                    "sent": "2026-10-25T05:13:00-08:00",
                    "effective": "2026-10-25T05:13:00-08:00",
                    "onset": "2026-10-25T05:13:00-08:00",
                    "expires": "2026-10-25T16:00:00-08:00",
                    "ends": "2026-10-25T22:00:00-08:00",
                    "status": "Actual",
                    "messageType": "Update",
                    "category": "Met",
                    "severity": "Moderate",
                    "certainty": "Likely",
                    "urgency": "Expected",
                    "event": "Winter Weather Advisory",
                    "senderName": "NWS Anchorage AK",
                    "headline": "Winter Weather Advisory issued October 25 at 5:13AM AKDT until October 25 at 10:00PM AKDT by NWS Anchorage AK",
                    "description": "* WHAT...Snow expected.",
                    "instruction": null,
                    "response": "Execute",
                    "parameters": {"NWSheadline": ["WINTER WEATHER ADVISORY REMAINS IN EFFECT"]}
                }
            },
            {
                "id": "x2", "type": "Feature", "geometry": null,
                "properties": {
                    "expires": "2026-10-25T16:00:00-08:00",
                    "ends": null,
                    "status": "Actual",
                    "messageType": "Alert",
                    "severity": "Minor",
                    "event": "High Surf Advisory",
                    "headline": null,
                    "description": "* WHAT...Large breaking waves."
                }
            },
            {
                "id": "x3", "type": "Feature", "geometry": null,
                "properties": {
                    "status": "Test", "messageType": "Alert",
                    "severity": "Extreme", "event": "Tornado Warning"
                }
            },
            {
                "id": "x4", "type": "Feature", "geometry": null,
                "properties": {
                    "status": "Actual", "messageType": "Cancel",
                    "severity": "Severe", "event": "Winter Storm Warning"
                }
            }
        ],
        "title": "Current watches, warnings, and advisories for Alaska",
        "updated": "2026-10-25T16:15:35+00:00"
    }"#;

    /// Real response for a US point with nothing active.
    const EMPTY: &str = r#"{"@context":{"@version":"1.1"},"type":"FeatureCollection","features":[],
        "title":"Current watches, warnings, and advisories for 39.7392 N, 104.9903 W",
        "updated":"2026-10-25T16:15:35+00:00"}"#;

    fn alert(event: &str, severity: &str, ends: Option<&str>) -> Alert {
        Alert {
            event: event.into(),
            headline: None,
            description: None,
            severity: severity.into(),
            level: Level::from_event(event),
            ends: ends.map(|s| DateTime::parse_from_rfc3339(s).unwrap()),
        }
    }

    #[test]
    fn parses_sample_and_skips_test_and_cancel() {
        let alerts = parse(SAMPLE).unwrap();
        assert_eq!(alerts.len(), 2);
        let first = &alerts[0];
        assert_eq!(first.event, "Winter Weather Advisory");
        assert_eq!(first.level, Level::Advisory);
        assert_eq!(first.severity, "Moderate");
        assert!(
            first
                .headline
                .as_deref()
                .unwrap()
                .starts_with("Winter Weather Advisory issued")
        );
        assert_eq!(
            first.ends,
            Some(DateTime::parse_from_rfc3339("2026-10-25T22:00:00-08:00").unwrap())
        );
        // `ends` is null, so `expires` is used.
        assert_eq!(
            alerts[1].ends,
            Some(DateTime::parse_from_rfc3339("2026-10-25T16:00:00-08:00").unwrap())
        );
        assert_eq!(alerts[1].headline, None);
    }

    #[test]
    fn parses_empty_collection() {
        assert_eq!(parse(EMPTY).unwrap(), vec![]);
    }

    #[test]
    fn level_from_event_suffix() {
        assert_eq!(Level::from_event("Winter Storm Warning"), Level::Warning);
        assert_eq!(Level::from_event("Tornado Watch"), Level::Watch);
        assert_eq!(Level::from_event("Wind Advisory"), Level::Advisory);
        assert_eq!(
            Level::from_event("Special Weather Statement"),
            Level::Statement
        );
        assert_eq!(Level::from_event("Red Flag Warning "), Level::Warning);
        assert_eq!(Level::from_event(""), Level::Statement);
    }

    #[test]
    fn sorted_by_level_then_severity_then_ends() {
        let alerts = vec![
            alert("Special Weather Statement", "Extreme", None),
            alert("Wind Advisory", "Minor", Some("2026-10-25T12:00:00Z")),
            alert("Wind Advisory", "Moderate", Some("2026-10-26T12:00:00Z")),
            alert("Flood Watch", "Severe", None),
            alert(
                "Winter Storm Warning",
                "Severe",
                Some("2026-10-27T00:00:00Z"),
            ),
            alert("High Wind Warning", "Severe", Some("2026-10-26T00:00:00Z")),
            alert("Blizzard Warning", "Unknown", None),
        ];
        let events: Vec<_> = sorted(alerts)
            .into_iter()
            .map(|a| format!("{} {}", a.event, a.severity))
            .collect();
        assert_eq!(
            events,
            [
                "High Wind Warning Severe",
                "Winter Storm Warning Severe",
                "Blizzard Warning Unknown",
                "Flood Watch Severe",
                "Wind Advisory Moderate",
                "Wind Advisory Minor",
                "Special Weather Statement Extreme",
            ]
        );
    }

    #[test]
    fn banner_without_ends() {
        let a = alert("Flood Watch", "Severe", None);
        assert_eq!(banner_text(&a, 0), "Flood Watch");
        assert_eq!(banner_text(&a, 2), "Flood Watch (+2 more)");
    }

    #[test]
    fn banner_with_ends_in_local_time() {
        let ends = Local::now().fixed_offset();
        let a = Alert {
            ends: Some(ends),
            ..alert("Winter Storm Warning", "Severe", None)
        };
        let expected = format!("Winter Storm Warning until {}", ends.format("%a %-I:%M %p"));
        assert_eq!(banner_text(&a, 0), expected);
        assert_eq!(banner_text(&a, 1), format!("{expected} (+1 more)"));
    }
}
