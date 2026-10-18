//! Raspberry Pi weather display: Rust/iced port of the Xojo `WeatherApp`.
//!
//! Layout coordinates below are taken verbatim from the Xojo `Window1`
//! controls (800x480 window) so the screen matches the original.

mod config;
mod weather;

use std::path::PathBuf;
use std::time::Duration;

use chrono::{Local, NaiveTime};
use iced::alignment::{Horizontal, Vertical};
use iced::font::Weight;
use iced::widget::{container, image, mouse_area, pin, stack, text};
use iced::{
    Border, Color, ContentFit, Element, Font, Length, Subscription, Task, keyboard, mouse, time,
    window,
};

use config::Config;
use weather::{DayForecast, Forecast, fmt_2dp, fmt_plain};

const WIDTH: f32 = 800.0;
const HEIGHT: f32 = 480.0;
/// Xojo `&cCCCCCC` (large labels) and `&cF7F7F7` (small labels).
const GREY: Color = Color::from_rgb8(0xCC, 0xCC, 0xCC);
const WHITE: Color = Color::from_rgb8(0xF7, 0xF7, 0xF7);

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
    background: image::Handle,

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
            background: image::Handle::from_path(assets.join("background.png")),
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
                self.forecast = Some(forecast);
                self.forecast_error = None;
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
            pin(container("").width(144).height(139).style(|_| container::Style {
                border: Border { color: GREY, width: 1.0, radius: 0.0.into() },
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
            // Xojo `DayNight` canvas: 120x115 at (653, 20).
            layout = layout.push(
                pin(image(icon.clone()).width(120).height(115).content_fit(ContentFit::Contain))
                    .x(653)
                    .y(20),
            );
        }

        let screen = stack![
            image(self.background.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(ContentFit::Cover),
            container(layout).center(Length::Fill),
        ];

        // Kiosk display: hide the pointer everywhere (Xojo InvisibleCursor).
        mouse_area(screen).interaction(mouse::Interaction::Hidden).into()
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
                    .color(color)
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

/// Xojo `d.ToString("H:mm")`.
fn clock_text() -> String {
    Local::now().format("%-H:%M").to_string()
}
