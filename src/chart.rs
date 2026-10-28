//! Charts: next-24-hours forecast (forecast page) and sensor history
//! (sensor page).
//!
//! Forecast chart:
//!
//! Two stacked panels sharing one time axis (never two y-scales on one
//! plot): temperature as a 2 px line with the current, highest and lowest
//! values labelled directly on it, and chance of precipitation as thin bars
//! on a fixed 0-100 % scale. Everything is drawn in the sky-aware text colour
//! so it reads on every background.

use iced::alignment::{Horizontal, Vertical};
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, Text};
use iced::{Color, Font, Point, Rectangle, Renderer, Size, Theme, mouse};

use chrono::{NaiveDateTime, Timelike};

use crate::weather::{Hour, fmt_1dp};

pub struct HourlyChart {
    pub hours: Vec<Hour>,
    pub ink: Color,
    pub font: Font,
}

// Vertical layout inside the canvas (px).
const TEMP_TOP: f32 = 40.0; // label above the highest point clears the title
const TEMP_BOTTOM: f32 = 92.0; // ...and below the lowest one (to ~114)
const RAIN_CAPTION_Y: f32 = 116.0;
const RAIN_TOP: f32 = 134.0;
const RAIN_BOTTOM: f32 = 160.0;
const AXIS_Y: f32 = 164.0;
/// Canvas height needed for the layout above.
pub const HEIGHT: f32 = 180.0;
const SIDE: f32 = 14.0;

impl<Message> canvas::Program<Message> for HourlyChart {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let n = self.hours.len();
        if n < 2 {
            self.text(
                &mut frame,
                "Hourly forecast not available",
                Point::new(0.0, 0.0),
                14.0,
                Horizontal::Left,
                Vertical::Top,
                1.0,
            );
            return vec![frame.into_geometry()];
        }

        let width = bounds.width;
        let step = (width - 2.0 * SIDE) / (n as f32 - 1.0);
        let x = |i: usize| SIDE + step * i as f32;

        self.text(
            &mut frame,
            "Next 24 hours · °F",
            Point::new(0.0, 0.0),
            14.0,
            Horizontal::Left,
            Vertical::Top,
            0.8,
        );

        // ---- Temperature line ----
        let temps: Vec<f64> = self.hours.iter().map(|h| h.temperature).collect();
        let lo = temps.iter().cloned().fold(f64::INFINITY, f64::min);
        let hi = temps.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let span = (hi - lo).max(4.0); // flat days don't exaggerate noise
        let mid = (hi + lo) / 2.0;
        let y_temp = |t: f64| {
            let f = ((t - (mid - span / 2.0)) / span) as f32;
            TEMP_BOTTOM - f * (TEMP_BOTTOM - TEMP_TOP)
        };

        let line = Path::new(|p| {
            p.move_to(Point::new(x(0), y_temp(temps[0])));
            for (i, &t) in temps.iter().enumerate().skip(1) {
                p.line_to(Point::new(x(i), y_temp(t)));
            }
        });
        frame.stroke(
            &line,
            Stroke::default().with_color(self.ink).with_width(2.0),
        );

        // Direct labels: now, highest, lowest (skip duplicates).
        let i_hi = argmax(&temps, |a, b| a > b);
        let i_lo = argmax(&temps, |a, b| a < b);
        let mut labelled = vec![];
        // "Now" goes on the side the line is not heading toward.
        let now_above = temps[1] <= temps[0];
        for (i, above) in [(i_hi, true), (i_lo, false), (0, now_above)] {
            if labelled.contains(&i) {
                continue;
            }
            labelled.push(i);
            let p = Point::new(x(i), y_temp(temps[i]));
            frame.fill(&Path::circle(p, 4.0), self.ink);
            let label = if i == 0 {
                format!("Now {}°", fmt_1dp(temps[i]))
            } else {
                format!("{}°", fmt_1dp(temps[i]))
            };
            let align = if i == 0 {
                Horizontal::Left
            } else if i == n - 1 {
                Horizontal::Right
            } else {
                Horizontal::Center
            };
            let ly = if above { p.y - 8.0 } else { p.y + 8.0 };
            let va = if above {
                Vertical::Bottom
            } else {
                Vertical::Top
            };
            let lx = if i == 0 { p.x - 4.0 } else { p.x };
            self.text(&mut frame, &label, Point::new(lx, ly), 14.0, align, va, 1.0);
        }

        // ---- Precipitation bars (fixed 0-100 % scale) ----
        let baseline = Path::line(Point::new(0.0, RAIN_BOTTOM), Point::new(width, RAIN_BOTTOM));
        frame.stroke(
            &baseline,
            Stroke::default()
                .with_color(Color { a: 0.3, ..self.ink })
                .with_width(1.0),
        );
        let bar_w = (step - 2.0).max(2.0); // 2 px gap between bars
        let mut wettest: Option<(usize, f64)> = None;
        for (i, h) in self.hours.iter().enumerate() {
            let Some(p) = h.precipitation.filter(|p| *p > 0.0) else {
                continue;
            };
            let height = (p.min(100.0) / 100.0) as f32 * (RAIN_BOTTOM - RAIN_TOP);
            let bar = Path::rounded_rectangle(
                Point::new(x(i) - bar_w / 2.0, RAIN_BOTTOM - height),
                Size::new(bar_w, height),
                2.0_f32.min(height / 2.0).into(),
            );
            frame.fill(
                &bar,
                Color {
                    a: 0.55,
                    ..self.ink
                },
            );
            if wettest.is_none_or(|(_, w)| p > w) {
                wettest = Some((i, p));
            }
        }
        if let Some((i, p)) = wettest {
            let top = RAIN_BOTTOM - (p.min(100.0) / 100.0) as f32 * (RAIN_BOTTOM - RAIN_TOP);
            let align = if i == 0 {
                Horizontal::Left
            } else if i == n - 1 {
                Horizontal::Right
            } else {
                Horizontal::Center
            };
            self.text(
                &mut frame,
                &format!("{p:.0}%"),
                Point::new(x(i), top - 3.0),
                12.0,
                align,
                Vertical::Bottom,
                1.0,
            );
        }
        // Right-aligned so it never meets the "Now" label on the left.
        let caption = if wettest.is_some() {
            "Chance of precipitation"
        } else {
            "Chance of precipitation: none expected"
        };
        self.text(
            &mut frame,
            caption,
            Point::new(width, RAIN_CAPTION_Y),
            12.0,
            Horizontal::Right,
            Vertical::Top,
            0.7,
        );

        // ---- Time axis: a tick label every 6 hours ----
        for (i, h) in self.hours.iter().enumerate() {
            if i == 0 || h.time.format("%H").to_string().parse::<u32>().unwrap_or(1) % 6 != 0 {
                continue;
            }
            let label = h.time.format("%-I %p").to_string();
            self.text(
                &mut frame,
                &label,
                Point::new(x(i), AXIS_Y),
                12.0,
                Horizontal::Center,
                Vertical::Top,
                0.8,
            );
        }

        vec![frame.into_geometry()]
    }
}

impl HourlyChart {
    #[allow(clippy::too_many_arguments)]
    fn text(
        &self,
        frame: &mut Frame,
        s: &str,
        at: Point,
        size: f32,
        ax: Horizontal,
        ay: Vertical,
        alpha: f32,
    ) {
        frame.fill_text(Text {
            content: s.to_owned(),
            position: at,
            color: Color {
                a: alpha,
                ..self.ink
            },
            size: size.into(),
            font: self.font,
            align_x: ax.into(),
            align_y: ay,
            ..Text::default()
        });
    }
}

/// One measurement over a time window (sensor history page): a 2 px line,
/// broken where readings are missing, with the latest, highest and lowest
/// values labelled on it. One series per chart; stack several charts for
/// several measurements rather than sharing a y-axis.
pub struct SeriesChart {
    pub points: Vec<(NaiveDateTime, f64)>,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    pub title: String,
    pub decimals: usize,
    /// Suffix for value labels, e.g. "°" or "%".
    pub suffix: String,
    /// Smallest y-range shown, so a flat line isn't stretched into noise.
    pub min_span: f64,
    /// Draw the time-axis labels (bottom chart of a stack only).
    pub show_axis: bool,
    pub ink: Color,
    pub font: Font,
}

impl<Message> canvas::Program<Message> for SeriesChart {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let text = |frame: &mut Frame, s: &str, at: Point, size: f32, ax: Horizontal, ay: Vertical, alpha: f32| {
            draw_text(frame, self.font, self.ink, s, at, size, ax, ay, alpha)
        };
        text(&mut frame, &self.title, Point::new(0.0, 0.0), 14.0, Horizontal::Left, Vertical::Top, 0.8);

        let (w, h) = (bounds.width, bounds.height);
        let top = 40.0; // label above the highest point clears the title
        let bottom = h - if self.show_axis { 38.0 } else { 22.0 };
        let window = (self.end - self.start).num_seconds().max(1) as f32;
        let x = |t: NaiveDateTime| SIDE + (w - 2.0 * SIDE) * ((t - self.start).num_seconds() as f32 / window);

        if self.show_axis {
            // Labels every 6 hours on the hour.
            let mut t = self.start.date().and_hms_opt(self.start.hour(), 0, 0).unwrap_or(self.start);
            while t <= self.end {
                if t > self.start && t.hour().is_multiple_of(6) {
                    let label = t.format("%-I %p").to_string();
                    text(&mut frame, &label, Point::new(x(t), h - 16.0), 12.0, Horizontal::Center, Vertical::Top, 0.8);
                }
                t += chrono::TimeDelta::hours(1);
            }
        }

        if self.points.len() < 2 {
            text(&mut frame, "Collecting readings…", Point::new(w / 2.0, (top + bottom) / 2.0), 14.0, Horizontal::Center, Vertical::Center, 0.7);
            return vec![frame.into_geometry()];
        }

        let values: Vec<f64> = self.points.iter().map(|p| p.1).collect();
        let lo = values.iter().cloned().fold(f64::INFINITY, f64::min);
        let hi = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let span = (hi - lo).max(self.min_span);
        let mid = (hi + lo) / 2.0;
        let y = |v: f64| bottom - ((v - (mid - span / 2.0)) / span) as f32 * (bottom - top);

        // Break the line across gaps longer than three sample intervals.
        let gap = crate::history::INTERVAL * 3;
        let line = Path::new(|p| {
            let mut prev: Option<NaiveDateTime> = None;
            for &(t, v) in &self.points {
                let pt = Point::new(x(t), y(v));
                match prev {
                    Some(pt_prev) if t - pt_prev <= gap => p.line_to(pt),
                    _ => p.move_to(pt),
                }
                prev = Some(t);
            }
        });
        frame.stroke(&line, Stroke::default().with_color(self.ink).with_width(2.0));

        let last = self.points.len() - 1;
        let i_hi = argmax(&values, |a, b| a > b);
        let i_lo = argmax(&values, |a, b| a < b);
        let fmt = |v: f64| format!("{v:.*}{}", self.decimals, self.suffix);
        let mut done = vec![];
        for (i, above, prefix) in [(i_hi, true, "High "), (i_lo, false, "Low "), (last, values[last] >= mid, "Now ")] {
            if done.contains(&i) {
                continue;
            }
            done.push(i);
            let p = Point::new(x(self.points[i].0), y(values[i]));
            frame.fill(&Path::circle(p, 4.0), self.ink);
            let align = if p.x > w - 90.0 { Horizontal::Right } else if p.x < 90.0 { Horizontal::Left } else { Horizontal::Center };
            let (ly, va) = if above { (p.y - 8.0, Vertical::Bottom) } else { (p.y + 8.0, Vertical::Top) };
            text(&mut frame, &format!("{prefix}{}", fmt(values[i])), Point::new(p.x, ly), 13.0, align, va, 1.0);
        }
        vec![frame.into_geometry()]
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_text(frame: &mut Frame, font: Font, ink: Color, s: &str, at: Point, size: f32, ax: Horizontal, ay: Vertical, alpha: f32) {
    frame.fill_text(Text {
        content: s.to_owned(),
        position: at,
        color: Color { a: alpha, ..ink },
        size: size.into(),
        font,
        align_x: ax.into(),
        align_y: ay,
        ..Text::default()
    });
}

/// Index of the value that wins `better` (first one on ties).
fn argmax(v: &[f64], better: impl Fn(f64, f64) -> bool) -> usize {
    let mut best = 0;
    for (i, &x) in v.iter().enumerate() {
        if better(x, v[best]) {
            best = i;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::argmax;

    #[test]
    fn argmax_finds_first_extreme() {
        let v = [3.0, 7.0, 7.0, 1.0];
        assert_eq!(argmax(&v, |a, b| a > b), 1);
        assert_eq!(argmax(&v, |a, b| a < b), 3);
    }
}
