//! Barometric pressure: sea-level reduction, units and 3-hour trend.

/// The BME280 measures station pressure (at the sensor's altitude). Weather
/// reports use sea-level pressure, which removes the altitude so readings
/// are comparable everywhere. Hypsometric reduction with the current outdoor
/// temperature (the formula Bosch documents for the BME280).
pub fn sea_level_hpa(station_hpa: f64, elevation_m: f64, temp_c: f64) -> f64 {
    let h = 0.0065 * elevation_m;
    station_hpa * (1.0 - h / (temp_c + h + 273.15)).powf(-5.257)
}

pub fn hpa_to_inhg(hpa: f64) -> f64 {
    hpa * 0.029_529_983
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trend {
    Rising,
    Steady,
    Falling,
}

impl Trend {
    /// Change over the last 3 hours, in hPa. Meteorological convention:
    /// within ±1 hPa is steady.
    pub fn from_change(delta_hpa: f64) -> Self {
        if delta_hpa >= 1.0 {
            Trend::Rising
        } else if delta_hpa <= -1.0 {
            Trend::Falling
        } else {
            Trend::Steady
        }
    }

    pub fn arrow(self) -> &'static str {
        match self {
            Trend::Rising => "↑",
            Trend::Steady => "→",
            Trend::Falling => "↓",
        }
    }
}

/// Display units, from `pressure_unit` in the config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    InHg,
    HPa,
}

impl Unit {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "hpa" | "mbar" | "mb" => Unit::HPa,
            _ => Unit::InHg,
        }
    }

    /// "30.02 inHg" / "1016.6 hPa".
    pub fn format(self, hpa: f64) -> String {
        match self {
            Unit::InHg => format!("{:.2} inHg", hpa_to_inhg(hpa)),
            Unit::HPa => format!("{hpa:.1} hPa"),
        }
    }

    /// Value in this unit (for charts).
    pub fn value(self, hpa: f64) -> f64 {
        match self {
            Unit::InHg => hpa_to_inhg(hpa),
            Unit::HPa => hpa,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Unit::InHg => "inHg",
            Unit::HPa => "hPa",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sea_level_reduction() {
        // At sea level nothing changes.
        assert!((sea_level_hpa(1013.25, 0.0, 15.0) - 1013.25).abs() < 1e-9);
        // Standard atmosphere: ~845.6 hPa at 1500 m reduces to ~1013 hPa.
        let p0 = sea_level_hpa(845.6, 1500.0, 15.0 - 0.0065 * 1500.0);
        assert!((p0 - 1013.25).abs() < 2.0, "{p0}");
    }

    #[test]
    fn units_and_trend() {
        assert_eq!(Unit::InHg.format(1016.6), "30.02 inHg");
        assert_eq!(Unit::HPa.format(1016.64), "1016.6 hPa");
        assert_eq!(Unit::parse("hPa"), Unit::HPa);
        assert_eq!(Unit::parse("inHg"), Unit::InHg);
        assert_eq!(Trend::from_change(1.2), Trend::Rising);
        assert_eq!(Trend::from_change(0.4), Trend::Steady);
        assert_eq!(Trend::from_change(-1.0), Trend::Falling);
    }
}
