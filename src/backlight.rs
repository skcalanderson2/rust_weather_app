//! Screen backlight control via Linux sysfs (`/sys/class/backlight/*`).
//!
//! On the Raspberry Pi 7" touch display this is `10-0045`, writable by the
//! `video` group. Anywhere else (macOS, HDMI monitors) there is no device and
//! every call is a no-op.

use std::fs;
use std::path::PathBuf;

pub struct Backlight {
    brightness: PathBuf,
    max: u32,
    /// Last value written, so the file is only touched on change.
    current: Option<u32>,
}

impl Backlight {
    /// The first backlight device, if any.
    pub fn detect() -> Option<Self> {
        let dir = fs::read_dir("/sys/class/backlight").ok()?.flatten().next()?.path();
        let max = fs::read_to_string(dir.join("max_brightness")).ok()?.trim().parse().ok()?;
        Some(Self { brightness: dir.join("brightness"), max, current: None })
    }

    /// Set brightness as a percentage (clamped to 5..=100 so the screen never
    /// goes fully dark).
    pub fn set_percent(&mut self, percent: f32) {
        let value = raw_value(percent, self.max);
        if self.current == Some(value) {
            return;
        }
        match fs::write(&self.brightness, value.to_string()) {
            Ok(()) => self.current = Some(value),
            Err(e) => {
                eprintln!("weather_app: backlight {}: {e}", self.brightness.display());
                // Don't retry every second if we lack permission.
                self.current = Some(value);
            }
        }
    }
}

fn raw_value(percent: f32, max: u32) -> u32 {
    (percent.clamp(5.0, 100.0) / 100.0 * max as f32).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_maps_to_device_range() {
        assert_eq!(raw_value(100.0, 255), 255);
        assert_eq!(raw_value(25.0, 255), 64);
        assert_eq!(raw_value(0.0, 255), 13); // floor of 5 %
        assert_eq!(raw_value(150.0, 31), 31);
    }
}
