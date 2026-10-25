//! Sky background: a vertical gradient chosen from the time of day
//! (night / dawn / day / dusk, from Open-Meteo sunrise and sunset) and the
//! current cloud cover, plus text colours that stay readable on it.

use chrono::{NaiveDateTime, TimeDelta};
use iced::Color;

/// Dawn runs from this long before sunrise to this long after; dusk likewise
/// around sunset.
pub const TWILIGHT_HALF: TimeDelta = TimeDelta::minutes(30);

/// Vertical two-stop gradient, `top` at y = 0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sky {
    pub top: Color,
    pub bottom: Color,
}

impl Sky {
    const fn rgb(top: u32, bottom: u32) -> Self {
        Self { top: hex(top), bottom: hex(bottom) }
    }

    /// Colour of the gradient at `t` (0 = top, 1 = bottom).
    pub fn at(&self, t: f32) -> Color {
        mix(self.top, self.bottom, t.clamp(0.0, 1.0))
    }

    pub fn lerp(&self, other: &Sky, t: f32) -> Sky {
        let t = t.clamp(0.0, 1.0);
        Sky { top: mix(self.top, other.top, t), bottom: mix(self.bottom, other.bottom, t) }
    }

    /// Move a fraction `rate` of the way toward `target` (one animation step).
    pub fn approach(&mut self, target: &Sky, rate: f32) {
        *self = self.lerp(target, rate);
    }
}

const fn hex(v: u32) -> Color {
    Color::from_rgb8((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgb(a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t)
}

// ---------- Palettes ----------

/// Top of every night / dawn / dusk sky: the original backdrop's navy,
/// a little darker.
const NIGHT_TOP: u32 = 0x050B36;

pub const NIGHT: Sky = Sky::rgb(NIGHT_TOP, 0x4A4E72);

pub const DAWN_DUSK_CLEAR: Sky = Sky::rgb(NIGHT_TOP, 0xFFEDB8); // -> yellow/white
pub const DAWN_DUSK_PARTLY: Sky = Sky::rgb(NIGHT_TOP, 0xE8673A); // -> orange/red
pub const DAWN_DUSK_CLOUDY: Sky = Sky::rgb(NIGHT_TOP, 0xB0BACB); // -> grey/blue

pub const DAY_CLEAR: Sky = Sky::rgb(0xF4C430, 0xFFF6CF); // yellow -> light yellow
pub const DAY_PARTLY: Sky = Sky::rgb(0xB9AE7E, 0xA3A7AC); // grey/yellow -> grey
pub const DAY_CLOUDY: Sky = Sky::rgb(0x8C939B, 0xAAB0B6); // grey

// ---------- Classification ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cover {
    Clear,
    Partly,
    Cloudy,
}

impl Cover {
    /// Fog, drizzle, rain, snow and storms (WMO code >= 45) count as
    /// cloudy; otherwise <25 % clear, 25-75 % partly, >75 % cloudy.
    pub fn from(cloud_cover: f64, weather_code: u8) -> Self {
        if weather_code >= 45 || cloud_cover > 75.0 {
            Cover::Cloudy
        } else if cloud_cover >= 25.0 {
            Cover::Partly
        } else {
            Cover::Clear
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Night,
    /// 0.0 at sunrise - 30 min, 0.5 at sunrise, 1.0 at sunrise + 30 min.
    Dawn(f32),
    Day,
    /// 0.0 at sunset - 30 min, 0.5 at sunset, 1.0 at sunset + 30 min.
    Dusk(f32),
}

impl Phase {
    pub fn at(now: NaiveDateTime, sunrise: NaiveDateTime, sunset: NaiveDateTime) -> Self {
        let progress = |center: NaiveDateTime| {
            let start = center - TWILIGHT_HALF;
            let span = (TWILIGHT_HALF * 2).num_seconds() as f32;
            ((now - start).num_seconds() as f32 / span).clamp(0.0, 1.0)
        };
        if (now - sunrise).abs() <= TWILIGHT_HALF {
            Phase::Dawn(progress(sunrise))
        } else if (now - sunset).abs() <= TWILIGHT_HALF {
            Phase::Dusk(progress(sunset))
        } else if now > sunrise && now < sunset {
            Phase::Day
        } else {
            Phase::Night
        }
    }
}

impl Phase {
    /// Sun above the horizon (sunrise..sunset). Picks the day or night
    /// variant of the weather icon, in step with the background.
    pub fn daylight(self) -> bool {
        match self {
            Phase::Day => true,
            Phase::Night => false,
            Phase::Dawn(p) => p >= 0.5,
            Phase::Dusk(p) => p < 0.5,
        }
    }

    /// Backlight percentage: `day` in daytime, `night` at night, ramping
    /// linearly across the hour of dawn or dusk.
    pub fn brightness(self, day: f32, night: f32) -> f32 {
        match self {
            Phase::Day => day,
            Phase::Night => night,
            Phase::Dawn(p) => night + (day - night) * p,
            Phase::Dusk(p) => day + (night - day) * p,
        }
    }
}

fn day(cover: Cover) -> Sky {
    match cover {
        Cover::Clear => DAY_CLEAR,
        Cover::Partly => DAY_PARTLY,
        Cover::Cloudy => DAY_CLOUDY,
    }
}

fn twilight(cover: Cover) -> Sky {
    match cover {
        Cover::Clear => DAWN_DUSK_CLEAR,
        Cover::Partly => DAWN_DUSK_PARTLY,
        Cover::Cloudy => DAWN_DUSK_CLOUDY,
    }
}

/// Target sky. Twilight blends smoothly: dawn goes night -> dawn colours
/// (peak at sunrise) -> day; dusk goes day -> dusk colours (peak at sunset)
/// -> night.
pub fn target(phase: Phase, cover: Cover) -> Sky {
    match phase {
        Phase::Night => NIGHT,
        Phase::Day => day(cover),
        Phase::Dawn(p) if p < 0.5 => NIGHT.lerp(&twilight(cover), p * 2.0),
        Phase::Dawn(p) => twilight(cover).lerp(&day(cover), (p - 0.5) * 2.0),
        Phase::Dusk(p) if p < 0.5 => day(cover).lerp(&twilight(cover), p * 2.0),
        Phase::Dusk(p) => twilight(cover).lerp(&NIGHT, (p - 0.5) * 2.0),
    }
}

// ---------- Readable text ----------

/// Dark text for light skies (near-black navy).
pub const DARK_TEXT: Color = hex(0x0B0F1A);
/// Contrast a label's normal colour must reach before we swap it.
const KEEP_IF_AT_LEAST: f32 = 4.5;

/// WCAG relative luminance.
fn luminance(c: Color) -> f32 {
    let lin = |v: f32| {
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// WCAG contrast ratio (1..21).
pub fn contrast(a: Color, b: Color) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Keep `light` (the label's normal colour, e.g. the original #CCCCCC) when
/// it is readable on `background`; otherwise use white or near-black,
/// whichever contrasts more.
pub fn text_on(background: Color, light: Color) -> Color {
    if contrast(light, background) >= KEEP_IF_AT_LEAST {
        return light;
    }
    if contrast(Color::WHITE, background) >= contrast(DARK_TEXT, background) {
        Color::WHITE
    } else {
        DARK_TEXT
    }
}

/// Whether weather icons drawn on `background` need their dark-outlined
/// variant: the light icons (a yellow sun, white clouds) vanish on bright
/// skies. Dark skies (night, orange twilight) keep the plain icons.
pub fn outline_icons(background: Color) -> bool {
    luminance(background) > 0.2
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn t(h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 20).unwrap().and_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn phases_follow_sunrise_and_sunset() {
        let (sr, ss) = (t(7, 15), t(18, 13));
        assert_eq!(Phase::at(t(3, 0), sr, ss), Phase::Night);
        assert_eq!(Phase::at(t(6, 44), sr, ss), Phase::Night);
        assert_eq!(Phase::at(t(6, 45), sr, ss), Phase::Dawn(0.0));
        assert_eq!(Phase::at(t(7, 15), sr, ss), Phase::Dawn(0.5));
        assert_eq!(Phase::at(t(7, 45), sr, ss), Phase::Dawn(1.0));
        assert_eq!(Phase::at(t(12, 0), sr, ss), Phase::Day);
        assert_eq!(Phase::at(t(18, 13), sr, ss), Phase::Dusk(0.5));
        assert_eq!(Phase::at(t(18, 44), sr, ss), Phase::Night);
        assert_eq!(Phase::at(t(23, 0), sr, ss), Phase::Night);
    }

    #[test]
    fn daylight_and_brightness_follow_phase() {
        assert!(Phase::Day.daylight() && !Phase::Night.daylight());
        assert!(!Phase::Dawn(0.4).daylight() && Phase::Dawn(0.5).daylight());
        assert!(Phase::Dusk(0.4).daylight() && !Phase::Dusk(0.6).daylight());
        assert_eq!(Phase::Night.brightness(100.0, 30.0), 30.0);
        assert_eq!(Phase::Dawn(0.5).brightness(100.0, 30.0), 65.0);
        assert_eq!(Phase::Dusk(1.0).brightness(100.0, 30.0), 30.0);
    }

    #[test]
    fn outlines_only_on_bright_skies() {
        let icon_y = 77.5 / 480.0;
        assert!(!outline_icons(NIGHT.at(icon_y)));
        assert!(!outline_icons(DAWN_DUSK_PARTLY.at(icon_y)));
        assert!(outline_icons(DAY_CLEAR.at(icon_y)));
        assert!(outline_icons(DAY_CLOUDY.at(icon_y)));
        // Forecast box, lower on the screen.
        assert!(outline_icons(DAY_PARTLY.at(414.0 / 480.0)));
    }

    #[test]
    fn cover_bands() {
        assert_eq!(Cover::from(10.0, 1), Cover::Clear);
        assert_eq!(Cover::from(25.0, 2), Cover::Partly);
        assert_eq!(Cover::from(75.0, 2), Cover::Partly);
        assert_eq!(Cover::from(76.0, 3), Cover::Cloudy);
        assert_eq!(Cover::from(5.0, 61), Cover::Cloudy); // rain
        assert_eq!(Cover::from(5.0, 45), Cover::Cloudy); // fog
    }

    fn close(a: Sky, b: Sky) -> bool {
        let c = |x: Color, y: Color| {
            (x.r - y.r).abs() < 1e-4 && (x.g - y.g).abs() < 1e-4 && (x.b - y.b).abs() < 1e-4
        };
        c(a.top, b.top) && c(a.bottom, b.bottom)
    }

    #[test]
    fn twilight_peaks_at_sunrise_and_blends_continuously() {
        for cover in [Cover::Clear, Cover::Partly, Cover::Cloudy] {
            assert!(close(target(Phase::Dawn(0.0), cover), NIGHT));
            assert!(close(target(Phase::Dawn(0.5), cover), twilight(cover)));
            assert!(close(target(Phase::Dawn(1.0), cover), day(cover)));
            assert!(close(target(Phase::Dusk(0.0), cover), day(cover)));
            assert!(close(target(Phase::Dusk(1.0), cover), NIGHT));
        }
    }

    /// Every label stays readable on every sky the app can show. The ten
    /// named states meet WCAG AA (large text 3:1, small 4.5:1). Mid-blends
    /// pass through a mid-grey where no text colour reaches 4.5:1 (the
    /// ceiling is ~4.35:1), so for those few minutes small text must stay
    /// above 4:1. Rows are (label centre y / screen height, normal colour,
    /// AA minimum) for the actual layout.
    #[test]
    fn text_is_readable_on_every_sky() {
        let grey = Color::from_rgb8(0xCC, 0xCC, 0xCC);
        let white = Color::from_rgb8(0xF7, 0xF7, 0xF7);
        let labels = [
            (77.5 / 480.0, grey, 3.0),   // clock
            (153.5 / 480.0, grey, 3.0),  // current temp
            (204.5 / 480.0, grey, 3.0),  // windchill
            (173.5 / 480.0, grey, 3.0),  // wind speed
            (339.0 / 480.0, white, 3.0), // forecast day (20 px)
            (365.0 / 480.0, white, 4.5), // max temp (12 px)
            (386.0 / 480.0, white, 4.5), // min temp
            (347.0 / 480.0, grey, 3.0),  // local temp
            (395.0 / 480.0, white, 4.5), // humidity
            (450.0 / 480.0, white, 4.5), // last updated
        ];
        let mut skies = vec![(NIGHT, true)];
        for cover in [Cover::Clear, Cover::Partly, Cover::Cloudy] {
            skies.push((day(cover), true));
            skies.push((twilight(cover), true));
            for i in 0..=40 {
                let p = i as f32 / 40.0;
                skies.push((target(Phase::Dawn(p), cover), false));
                skies.push((target(Phase::Dusk(p), cover), false));
            }
        }
        for (sky, named) in &skies {
            for &(y, light, aa) in &labels {
                let bg = sky.at(y);
                let ratio = contrast(text_on(bg, light), bg);
                let min: f32 = if *named { aa } else { f32::min(aa, 4.0) };
                assert!(ratio >= min, "contrast {ratio:.2} < {min} at y={y:.2} on {sky:?}");
            }
        }
    }
}
