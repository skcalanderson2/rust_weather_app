//! Raspberry Pi weather display: Rust/iced port of the Xojo `WeatherApp`.
//!
//! Layout coordinates below are taken verbatim from the Xojo `Window1`
//! controls (800x480 window) so the screen matches the original.

mod config;
mod sky;
mod weather;

use std::path::PathBuf;
use std::time::Duration;

use chrono::{Local, NaiveTime};
use iced::gradient::Linear;
use iced::alignment::{Horizontal, Vertical};
use iced::font::Weight;
use iced::widget::{container, image, mouse_area, pin, stack, svg, text};
use iced::{
    Background, Border, Color, ContentFit, Element, Font, Length, Radians, Subscription, Task,
    Shadow, Vector, keyboard, mouse, time, window,
};

use config::Config;
use sky::{Cover, Phase, Sky};
use weather::{DayForecast, Forecast, fmt_2dp, fmt_plain};

const WIDTH: f32 = 800.0;
const HEIGHT: f32 = 480.0;
/// Xojo `&cCCCCCC` (large labels) and `&cF7F7F7` (small labels).
const GREY: Color = Color::from_rgb8(0xCC, 0xCC, 0xCC);
const WHITE: Color = Color::from_rgb8(0xF7, 0xF7, 0xF7);
/// Fraction of the remaining distance the background moves toward its
/// target each second: cloud changes fade in over roughly half a minute.
const SKY_FADE_PER_SEC: f32 = 0.1;

fn main() -> iced::Result {
    let config = Config::load();
    let fullscreen = config.fullscreen;

    iced::application(move || App::new(config.clone()), App::update, App::view)
        .title("WeatherApp")
        .subscription(App::subscription)
        .window(window::Settings {
            size: iced::Size::new(WIDTH, HEIGHT),
            fullscreen,
            decorations: !fullscreen,
            resizable: false,
            ..Default::default()
        })
        .run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Forecast,
    LocalTemp,
    LocalHumidity,
}

#[derive(Debug, Clone)]
enum Message {
    Tick,
    RefreshAll,
    Fetch(Source),
    ForecastLoaded(Result<Forecast, String>),
    LocalTempLoaded(Result<f64, String>),
    LocalHumidityLoaded(Result<f64, String>),
    Exit,
}

struct App {
    config: Config,
    client: reqwest::Client,
    font: Font,
    assets: PathBuf,
    wind_icon: svg::Handle,
    /// Background currently shown; eases toward `sky_target()` every tick.
    sky: Sky,
    /// Testing aid: `WEATHER_APP_SKY=dawn-partly` (night, dawn-*, day-*,
    /// dusk-* with clear/partly/cloudy) pins the sky to one state.
    sky_override: Option<Sky>,

    clock: String,
    forecast: Option<Forecast>,
    forecast_error: Option<String>,
    icon: Option<image::Handle>,
    local_temp: Option<f64>,
    local_temp_updated: Option<NaiveTime>,
    local_humidity: Option<f64>,
}

impl App {
    fn new(config: Config) -> (Self, Task<Message>) {
        let assets = config.resolved_assets_dir();
        // Empty name = system sans-serif. `Font::with_name` needs a
        // `'static` name; the config lives for the whole run, so leaking
        // this one string is fine.
        let font = match config.font.trim() {
            "" => Font::DEFAULT,
            name => Font::with_name(Box::leak(name.to_owned().into_boxed_str())),
        };
        let app = Self {
            client: weather::client(config.http_timeout_seconds),
            font,
            wind_icon: svg::Handle::from_path(assets.join("icons/wind.svg")),
            sky: sky::NIGHT,
            sky_override: std::env::var("WEATHER_APP_SKY").ok().and_then(|s| parse_sky(&s)),
            assets,
            config,
            clock: clock_text(),
            forecast: None,
            forecast_error: None,
            icon: None,
            local_temp: None,
            local_temp_updated: None,
            local_humidity: None,
        };
        (app, Task::done(Message::RefreshAll))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => {
                self.clock = clock_text();
                let target = self.sky_target();
                self.sky.approach(&target, SKY_FADE_PER_SEC);
                Task::none()
            }
            Message::RefreshAll => Task::batch(
                [Source::Forecast, Source::LocalTemp, Source::LocalHumidity]
                    .map(|s| self.fetch(s)),
            ),
            Message::Fetch(source) => self.fetch(source),
            Message::ForecastLoaded(Ok(forecast)) => {
                let c = &forecast.current;
                let icon = weather::icon_name(c.weather_code, c.is_day == 1);
                self.icon = Some(image::Handle::from_path(
                    self.assets.join("icons").join(format!("{icon}.png")),
                ));
                let first = self.forecast.is_none();
                self.forecast = Some(forecast);
                self.forecast_error = None;
                if first {
                    // No fade from the placeholder night sky at startup.
                    self.sky = self.sky_target();
                }
                Task::none()
            }
            Message::ForecastLoaded(Err(e)) => {
                eprintln!("weather_app: forecast: {e}");
                self.forecast_error = Some(e);
                self.retry(Source::Forecast)
            }
            Message::LocalTempLoaded(Ok(t)) => {
                self.local_temp = Some(t);
                self.local_temp_updated = Some(Local::now().time());
                Task::none()
            }
            Message::LocalTempLoaded(Err(e)) => {
                eprintln!("weather_app: local temperature: {e}");
                self.retry(Source::LocalTemp)
            }
            Message::LocalHumidityLoaded(Ok(h)) => {
                self.local_humidity = Some(h);
                Task::none()
            }
            Message::LocalHumidityLoaded(Err(e)) => {
                eprintln!("weather_app: local humidity: {e}");
                self.retry(Source::LocalHumidity)
            }
            Message::Exit => iced::exit(),
        }
    }

    /// Sky for right now: phase from today's sunrise/sunset, cover from
    /// current cloud cover and weather code. Night until the first forecast.
    fn sky_target(&self) -> Sky {
        if let Some(sky) = self.sky_override {
            return sky;
        }
        let Some(f) = &self.forecast else { return sky::NIGHT };
        let now = Local::now().naive_local();
        let Some((sunrise, sunset)) = f.daily.sun_times(now.date()) else {
            return sky::NIGHT;
        };
        let cover = Cover::from(f.current.cloud_cover, f.current.weather_code);
        sky::target(Phase::at(now, sunrise, sunset), cover)
    }

    /// Readable text colour for a label whose box is (y, h), given its
    /// normal light colour.
    fn ink(&self, light: Color, y: f32, h: f32) -> Color {
        sky::text_on(self.sky.at((y + h / 2.0) / HEIGHT), light)
    }

    fn fetch(&self, source: Source) -> Task<Message> {
        let client = self.client.clone();
        match source {
            Source::Forecast => Task::perform(
                weather::fetch_forecast(client, self.config.forecast_url()),
                Message::ForecastLoaded,
            ),
            Source::LocalTemp => Task::perform(
                weather::fetch_local_temp(client, self.config.local_temp_url.clone()),
                Message::LocalTempLoaded,
            ),
            Source::LocalHumidity => Task::perform(
                weather::fetch_local_humidity(client, self.config.local_humidity_url.clone()),
                Message::LocalHumidityLoaded,
            ),
        }
    }

    fn retry(&self, source: Source) -> Task<Message> {
        let delay = Duration::from_secs(self.config.retry_seconds.max(5));
        Task::perform(tokio::time::sleep(delay), move |_| Message::Fetch(source))
    }

    fn subscription(&self) -> Subscription<Message> {
        let refresh = Duration::from_secs(self.config.refresh_minutes.max(1) * 60);
        Subscription::batch([
            // Xojo `TimeTimer` (1 s) and `WeatherTimer` (15 min).
            time::every(Duration::from_secs(1)).map(|_| Message::Tick),
            time::every(refresh).map(|_| Message::RefreshAll),
            keyboard::listen().filter_map(|event| match event {
                keyboard::Event::KeyPressed {
                    key: keyboard::Key::Named(keyboard::key::Named::Escape),
                    ..
                } => Some(Message::Exit),
                _ => None,
            }),
        ])
    }

    fn view(&self) -> Element<'_, Message> {
        let current = self.forecast.as_ref().map(|f| &f.current);

        let current_temp = match (&self.forecast_error, current) {
            (Some(e), _) => format!("Error getting data: {e}"),
            (None, Some(c)) => format!("Current Temp: {}° F", fmt_plain(c.temperature_2m)),
            (None, None) => "Current Temp: --° F".into(),
        };
        let wind_chill = current.map_or("--".into(), |c| fmt_plain(c.apparent_temperature));
        let wind_speed = current.map_or("--".into(), |c| format!("{}", c.wind_speed_10m.round()));
        let tomorrow: Option<DayForecast> = self.forecast.as_ref().and_then(|f| f.daily.day(1));
        let opt = |v: Option<f64>| v.map_or("--".into(), fmt_2dp);

        let regular = self.font;
        let bold = Font { weight: Weight::Bold, ..self.font };
        let box_ink = self.ink(GREY, 321.0, 139.0);

        let mut layout = stack![
            // Clock, current conditions (top left)
            self.label(&self.clock, 80.0, regular, GREY, (20.0, 20.0, 348.0, 115.0), Horizontal::Left),
            self.label(&current_temp, 48.0, regular, GREY, (20.0, 112.0, 573.0, 83.0), Horizontal::Left),
            self.label(
                &format!("Windchill Temp: {wind_chill}° F"),
                36.0, regular, GREY, (20.0, 163.0, 451.0, 83.0), Horizontal::Left,
            ),
            self.label(
                &format!("{wind_speed} Mph"),
                24.0, regular, GREY, (496.0, 132.0, 217.0, 83.0), Horizontal::Right,
            ),
            // Tomorrow's forecast box (Xojo GroupBox1 + ForecastDay1/Max/Min)
            pin(container("").width(144).height(139).style(move |_| container::Style {
                border: Border { color: box_ink, width: 1.0, radius: 0.0.into() },
                ..Default::default()
            }))
            .x(20)
            .y(321),
            self.label(
                &tomorrow.as_ref().map_or("--".into(), |d| d.date.format("%a %-d").to_string()),
                20.0, regular, WHITE, (20.0, 329.0, 141.0, 20.0), Horizontal::Center,
            ),
            self.label(
                &format!("Max Temp: {}°F", opt(tomorrow.as_ref().and_then(|d| d.max))),
                12.0, bold, WHITE, (20.0, 355.0, 141.0, 20.0), Horizontal::Center,
            ),
            self.label(
                &format!("Min Temp: {}°F", opt(tomorrow.as_ref().and_then(|d| d.min))),
                12.0, bold, WHITE, (20.0, 376.0, 141.0, 20.0), Horizontal::Center,
            ),
            // Pico W sensor
            self.label(
                &format!("Local Temp: {}° F", self.local_temp.map_or("--".into(), fmt_plain)),
                50.0, regular, GREY, (176.0, 321.0, 604.0, 52.0), Horizontal::Left,
            ),
            self.label(
                &format!("Local Humidity: {}%", self.local_humidity.map_or("--".into(), fmt_plain)),
                12.0, bold, WHITE, (176.0, 385.0, 187.0, 20.0), Horizontal::Left,
            ),
            self.label(
                &format!(
                    "Local Temp Last Updated: {}",
                    self.local_temp_updated.map_or("--".into(), |t| t.format("%-H:%M").to_string())
                ),
                12.0, bold, WHITE, (364.0, 440.0, 187.0, 20.0), Horizontal::Left,
            ),
        ]
        .width(WIDTH)
        .height(HEIGHT);

        if let Some(icon) = &self.icon {
            if let Some(halo) = self.halo(713.0, 77.5, 100.0) {
                layout = layout.push(halo);
            }
            // Xojo `DayNight` canvas: 120x115 at (653, 20).
            layout = layout.push(
                pin(image(icon.clone()).width(120).height(115).content_fit(ContentFit::Contain))
                    .x(653)
                    .y(20),
            );
        }

        // Wind icon beside the speed. Xojo `DayNight1` canvas, 55x56 at
        // (718, 139); its image reference was broken in the Xojo project,
        // so it never showed there either. Monochrome SVG tinted with the
        // same readable colour as the "Mph" text next to it.
        let wind_ink = self.ink(GREY, 139.0, 56.0);
        layout = layout.push(
            pin(svg(self.wind_icon.clone())
                .width(55)
                .height(56)
                .content_fit(ContentFit::Contain)
                .style(move |_, _| svg::Style { color: Some(wind_ink) }))
            .x(718)
            .y(139),
        );

        let Sky { top, bottom } = self.sky;
        let screen = container(layout).center(Length::Fill).style(move |_| container::Style {
            // Radians(PI) runs the gradient from the top edge to the bottom.
            background: Some(Background::Gradient(
                Linear::new(Radians(std::f32::consts::PI))
                    .add_stop(0.0, top)
                    .add_stop(1.0, bottom)
                    .into(),
            )),
            ..Default::default()
        });

        // Kiosk display: hide the pointer everywhere (Xojo InvisibleCursor).
        mouse_area(screen).interaction(mouse::Interaction::Hidden).into()
    }

    /// Soft dark halo centred at (cx, cy) so a light icon stays visible on
    /// bright skies: a blurred translucent disc whose strength follows the
    /// sky's brightness there. `None` on dark skies.
    fn halo<'a>(&self, cx: f32, cy: f32, diameter: f32) -> Option<Element<'a, Message>> {
        let alpha = sky::halo_alpha(self.sky.at(cy / HEIGHT));
        if alpha <= 0.0 {
            return None;
        }
        // Fainter fill than shadow, so the edge feathers out.
        let core = Color { a: alpha * 0.6, ..sky::DARK_TEXT };
        let glow = Color { a: alpha, ..sky::DARK_TEXT };
        let r = diameter / 2.0;
        Some(
            pin(container("").width(diameter).height(diameter).style(move |_| container::Style {
                background: Some(Background::Color(core)),
                border: Border { radius: r.into(), ..Default::default() },
                shadow: Shadow { color: glow, offset: Vector::ZERO, blur_radius: diameter * 0.36 },
                ..Default::default()
            }))
            .x(cx - r)
            .y(cy - r)
            .into(),
        )
    }

    /// A Xojo `DesktopLabel`: fixed box at (x, y, w, h), text vertically
    /// centred, single line.
    fn label<'a>(
        &self,
        content: &str,
        size: f32,
        font: Font,
        color: Color,
        (x, y, w, h): (f32, f32, f32, f32),
        align: Horizontal,
    ) -> Element<'a, Message> {
        pin(
            container(
                text(content.to_owned())
                    .size(size)
                    .font(font)
                    .color(self.ink(color, y, h))
                    .wrapping(text::Wrapping::None),
            )
            .width(w)
            .height(h)
            .align_x(align)
            .align_y(Vertical::Center),
        )
        .x(x)
        .y(y)
        .into()
    }
}

/// `WEATHER_APP_SKY` value -> fixed sky (twilight shown at its peak).
fn parse_sky(s: &str) -> Option<Sky> {
    let (phase, cover) = s.split_once('-').unwrap_or((s, "clear"));
    let cover = match cover {
        "clear" => Cover::Clear,
        "partly" => Cover::Partly,
        "cloudy" => Cover::Cloudy,
        _ => return None,
    };
    let phase = match phase {
        "night" => Phase::Night,
        "dawn" => Phase::Dawn(0.5),
        "day" => Phase::Day,
        "dusk" => Phase::Dusk(0.5),
        _ => return None,
    };
    Some(sky::target(phase, cover))
}

/// Xojo `d.ToString("H:mm")`.
fn clock_text() -> String {
    Local::now().format("%-H:%M").to_string()
}
