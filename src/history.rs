//! Outdoor sensor history: one sample every 5 minutes, kept for 48 hours
//! in a small JSON file so restarts and auto-updates don't lose it.
//!
//! File: `$XDG_STATE_HOME/weather_app/history.json`
//! (default `~/.local/state/weather_app/history.json`).

use std::path::PathBuf;

use chrono::{NaiveDateTime, TimeDelta};
use serde::{Deserialize, Serialize};

pub const INTERVAL: TimeDelta = TimeDelta::minutes(5);
const KEEP: TimeDelta = TimeDelta::hours(48);

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub t: NaiveDateTime,
    pub temp_f: Option<f64>,
    pub humidity: Option<f64>,
    /// Station pressure as measured (sea-level reduction happens at display
    /// time, so a corrected elevation fixes the whole history).
    pub pressure_hpa: Option<f64>,
}

pub struct History {
    samples: Vec<Sample>,
    path: Option<PathBuf>,
}

impl History {
    /// Load from the state file (empty if missing or unreadable).
    pub fn load() -> Self {
        let path = state_path();
        let samples = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { samples, path }
    }

    #[cfg(test)]
    fn in_memory() -> Self {
        Self { samples: vec![], path: None }
    }

    /// Record a sample if the last one is at least `INTERVAL` old. Returns
    /// whether it was recorded.
    pub fn record(&mut self, sample: Sample) -> bool {
        if sample.temp_f.is_none() && sample.humidity.is_none() && sample.pressure_hpa.is_none() {
            return false;
        }
        if self.samples.last().is_some_and(|last| sample.t - last.t < INTERVAL) {
            return false;
        }
        self.samples.push(sample);
        let cutoff = sample.t - KEEP;
        self.samples.retain(|s| s.t >= cutoff);
        self.save();
        true
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let Ok(json) = serde_json::to_string(&self.samples) else { return };
        let result = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| {
                // Write then rename, so a power cut can't leave half a file.
                let tmp = path.with_extension("json.tmp");
                std::fs::write(&tmp, json)?;
                std::fs::rename(tmp, path)
            });
        if let Err(e) = result {
            eprintln!("weather_app: history {}: {e}", path.display());
        }
    }

    /// `(time, value)` points from the last `hours`, for one field.
    pub fn series(
        &self,
        now: NaiveDateTime,
        hours: i64,
        field: impl Fn(&Sample) -> Option<f64>,
    ) -> Vec<(NaiveDateTime, f64)> {
        let start = now - TimeDelta::hours(hours);
        self.samples
            .iter()
            .filter(|s| s.t >= start)
            .filter_map(|s| Some((s.t, field(s)?)))
            .collect()
    }

    /// Station pressure about 3 hours before `now` (closest sample between
    /// 2.5 and 3.5 hours back), for the trend arrow.
    pub fn pressure_3h_ago(&self, now: NaiveDateTime) -> Option<f64> {
        let target = now - TimeDelta::hours(3);
        self.samples
            .iter()
            .filter(|s| (s.t - target).abs() <= TimeDelta::minutes(30))
            .filter_map(|s| Some(((s.t - target).abs(), s.pressure_hpa?)))
            .min_by_key(|(d, _)| *d)
            .map(|(_, p)| p)
    }
}

fn state_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("weather_app").join("history.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn t(h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 27).unwrap().and_hms_opt(h, m, 0).unwrap()
    }

    fn s(t: NaiveDateTime, p: f64) -> Sample {
        Sample { t, temp_f: Some(50.0), humidity: Some(40.0), pressure_hpa: Some(p) }
    }

    #[test]
    fn records_every_five_minutes_and_prunes() {
        let mut h = History::in_memory();
        assert!(h.record(s(t(10, 0), 840.0)));
        assert!(!h.record(s(t(10, 3), 840.0)));
        assert!(h.record(s(t(10, 5), 840.5)));
        let empty = Sample { t: t(10, 20), temp_f: None, humidity: None, pressure_hpa: None };
        assert!(!h.record(empty));
        // 49 hours later the first two samples are pruned.
        assert!(h.record(s(t(10, 5) + TimeDelta::hours(49), 841.0)));
        assert_eq!(h.samples.len(), 1);
    }

    #[test]
    fn pressure_three_hours_back() {
        let mut h = History::in_memory();
        for (hh, mm, p) in [(6, 50, 838.0), (7, 10, 839.0), (9, 0, 840.0)] {
            h.record(s(t(hh, mm), p));
        }
        assert_eq!(h.pressure_3h_ago(t(10, 5)), Some(839.0)); // 7:10 is closest to 7:05
        assert_eq!(h.pressure_3h_ago(t(11, 0)), None); // nothing near 8:00
        assert_eq!(h.series(t(10, 0), 2, |s| s.pressure_hpa), vec![(t(9, 0), 840.0)]);
    }

    #[test]
    fn samples_round_trip_as_json() {
        let json = serde_json::to_string(&vec![s(t(10, 0), 840.0)]).unwrap();
        let back: Vec<Sample> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, vec![s(t(10, 0), 840.0)]);
    }
}
