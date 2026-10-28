//! Raspberry Pi weather display: Rust/iced port of the Xojo `WeatherApp`.
//!
//! Layout coordinates below are taken verbatim from the Xojo `Window1`
//! controls (800x480 window) so the screen matches the original.
//! Tapping cycles through a forecast page (3-day forecast, next-24-hours
//! chart, active alert) and a sensor page (last 24 hours of outdoor
//! temperature, humidity and pressure), then back; any page returns to the
//! main screen after a minute.

mod alerts;
mod backlight;
mod chart;
mod config;
mod history;
mod pressure;
mod sky;
mod weather;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::{Local, NaiveDate, NaiveTime, TimeDelta};
use iced::alignment::{Horizontal, Vertical};
use iced::font::Weight;
use iced::gradient::Linear;
use iced::widget::{Space, canvas, container, image, mouse_area, pin, row, stack, svg, text};
use iced::{
    Background, Border, Color, ContentFit, Element, Font, Length, Radians, Subscription, Task,
    keyboard, mouse, time, window,
};

use alerts::{Alert, Level};
use backlight::Backlight;
use config::Config;
use sky::{Cover, Phase, Sky};
use weather::{DayForecast, Forecast, fmt_1dp, fmt_plain};

const WIDTH: f32 = 800.0;
const HEIGHT: f32 = 480.0;
/// Xojo `&cCCCCCC` (large labels) and `&cF7F7F7` (small labels).
const GREY: Color = Color::from_rgb8(0xCC, 0xCC, 0xCC);
const WHITE: Color = Color::from_rgb8(0xF7, 0xF7, 0xF7);
/// Fraction of the remaining distance the background moves toward its
/// target each second: cloud changes fade in over roughly half a minute.
const SKY_FADE_PER_SEC: f32 = 0.1;
/// Values older than this are drawn dimmed so an outage is visible.
const STALE_AFTER: Duration = Duration::from_secs(30 * 60);
/// The forecast and sensor pages close by themselves after this long.
const DETAILS_TIMEOUT: Duration = Duration::from_secs(60);

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
    LocalPressure,
    /// Combined endpoint (`sensor_url`): all three readings at once.
    Sensor,
    Alerts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Main,
    Forecast,
    Sensor,
}

#[derive(Debug, Clone)]
enum Message {
    Tick,
    RefreshAll,
    Fetch(Source),
    ForecastLoaded(Result<Box<Forecast>, String>),
    LocalTempLoaded(Result<f64, String>),
    LocalHumidityLoaded(Result<f64, String>),
    LocalPressureLoaded(Result<f64, String>),
    SensorLoaded(Result<weather::SensorReading, String>),
    AlertsLoaded(Result<Vec<Alert>, String>),
    /// Finger tap (or click) anywhere: next page.
    Tap,
    Exit,
}

struct App {
    config: Config,
    client: reqwest::Client,
    font: Font,
    assets: PathBuf,
    wind_icon: svg::Handle,
    sunrise_icon: svg::Handle,
    sunset_icon: svg::Handle,
    backlight: Option<Backlight>,
    /// Background currently shown; eases toward `sky_target()` every tick.
    sky: Sky,
    /// Testing aid: `WEATHER_APP_SKY=dawn-partly` (night, dawn-*, day-*,
    /// dusk-* with clear/partly/cloudy) pins the sky to one state.
    sky_override: Option<(Phase, Cover)>,

    clock: String,
    /// "Sun Oct 25", shown right of the clock.
    date: String,
    forecast: Option<Forecast>,
    forecast_at: Option<Instant>,
    forecast_error: Option<String>,
    /// Current-conditions icon, re-picked every tick so its day/night
    /// variant changes exactly with the sky.
    icon_name: String,
    icon: Option<image::Handle>,
    local_temp: Option<f64>,
    local_temp_updated: Option<NaiveTime>,
    local_temp_at: Option<Instant>,
    local_humidity: Option<f64>,
    local_humidity_at: Option<Instant>,
    /// Most important first.
    alerts: Vec<Alert>,
    /// BME280 station pressure, hPa.
    local_pressure: Option<f64>,
    local_pressure_at: Option<Instant>,
    pressure_unit: pressure::Unit,
    /// Outdoor readings every 5 minutes, persisted across restarts.
    history: history::History,
    page: Page,
    /// When `page` was shown (for the timeout back to the main screen).
    page_opened: Instant,
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
        let icon = |name: &str| svg::Handle::from_path(assets.join("icons").join(name));
        let mut app = Self {
            client: weather::client(config.http_timeout_seconds),
            font,
            wind_icon: icon("wind.svg"),
            sunrise_icon: icon("sunrise.svg"),
            sunset_icon: icon("sunset.svg"),
            backlight: if config.dim_at_night { Backlight::detect() } else { None },
            sky: sky::NIGHT,
            sky_override: std::env::var("WEATHER_APP_SKY").ok().and_then(|s| parse_sky(&s)),
            pressure_unit: pressure::Unit::parse(&config.pressure_unit),
            assets,
            config,
            clock: clock_text(),
            date: date_text(Local::now().date_naive()),
            forecast: None,
            forecast_at: None,
            forecast_error: None,
            icon_name: String::new(),
            icon: None,
            local_temp: None,
            local_temp_updated: None,
            local_temp_at: None,
            local_humidity: None,
            local_humidity_at: None,
            alerts: test_alert().into_iter().collect(),
            local_pressure: None,
            local_pressure_at: None,
            history: history::History::load(),
            // Testing aid: WEATHER_APP_PAGE=forecast|sensor opens that page.
            page: match std::env::var("WEATHER_APP_PAGE").as_deref() {
                Ok("sensor") => Page::Sensor,
                Ok(_) => Page::Forecast,
                Err(_) => Page::Main,
            },
            page_opened: Instant::now(),
        };
        app.sky = app.sky_target();
        (app, Task::done(Message::RefreshAll))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => {
                self.clock = clock_text();
                self.date = date_text(Local::now().date_naive());
                let target = self.sky_target();
                self.sky.approach(&target, SKY_FADE_PER_SEC);
                self.refresh_icon();
                if self.page != Page::Main
                    && self.page_opened.elapsed() > DETAILS_TIMEOUT
                    && std::env::var_os("WEATHER_APP_PAGE").is_none()
                {
                    self.page = Page::Main;
                }
                self.record_history();
                self.update_backlight();
                Task::none()
            }
            Message::RefreshAll => {
                let mut sources = vec![Source::Forecast];
                if !self.config.sensor_url.is_empty() {
                    sources.push(Source::Sensor);
                } else {
                    sources.extend([Source::LocalTemp, Source::LocalHumidity]);
                    if !self.config.local_pressure_url.is_empty() {
                        sources.push(Source::LocalPressure);
                    }
                }
                if self.config.alerts {
                    sources.push(Source::Alerts);
                }
                Task::batch(sources.into_iter().map(|s| self.fetch(s)))
            }
            Message::Fetch(source) => self.fetch(source),
            Message::ForecastLoaded(Ok(forecast)) => {
                let first = self.forecast.is_none();
                self.forecast = Some(*forecast);
                self.forecast_at = Some(Instant::now());
                self.forecast_error = None;
                if first {
                    // No fade from the placeholder night sky at startup.
                    self.sky = self.sky_target();
                }
                self.refresh_icon();
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
                self.local_temp_at = Some(Instant::now());
                Task::none()
            }
            Message::LocalTempLoaded(Err(e)) => {
                eprintln!("weather_app: local temperature: {e}");
                self.retry(Source::LocalTemp)
            }
            Message::LocalHumidityLoaded(Ok(h)) => {
                self.local_humidity = Some(h);
                self.local_humidity_at = Some(Instant::now());
                Task::none()
            }
            Message::LocalHumidityLoaded(Err(e)) => {
                eprintln!("weather_app: local humidity: {e}");
                self.retry(Source::LocalHumidity)
            }
            Message::LocalPressureLoaded(Ok(p)) => {
                self.local_pressure = Some(p);
                self.local_pressure_at = Some(Instant::now());
                Task::none()
            }
            Message::LocalPressureLoaded(Err(e)) => {
                eprintln!("weather_app: local pressure: {e}");
                self.retry(Source::LocalPressure)
            }
            Message::SensorLoaded(Ok(r)) => {
                let now = Some(Instant::now());
                if let Some(t) = r.temp_f {
                    self.local_temp = Some(t);
                    self.local_temp_updated = Some(Local::now().time());
                    self.local_temp_at = now;
                }
                if let Some(h) = r.humidity {
                    self.local_humidity = Some(h);
                    self.local_humidity_at = now;
                }
                if let Some(p) = r.pressure_hpa {
                    self.local_pressure = Some(p);
                    self.local_pressure_at = now;
                }
                Task::none()
            }
            Message::SensorLoaded(Err(e)) => {
                eprintln!("weather_app: sensor: {e}");
                self.retry(Source::Sensor)
            }
            Message::AlertsLoaded(Ok(list)) => {
                self.alerts = alerts::sorted(list);
                self.alerts.extend(test_alert());
                Task::none()
            }
            Message::AlertsLoaded(Err(e)) => {
                eprintln!("weather_app: alerts: {e}");
                // Outside the US there will never be alerts; stop asking.
                if e.contains("not covered") {
                    self.config.alerts = false;
                    Task::none()
                } else {
                    self.retry(Source::Alerts)
                }
            }
            Message::Tap => {
                self.page = match self.page {
                    Page::Main => Page::Forecast,
                    Page::Forecast => Page::Sensor,
                    Page::Sensor => Page::Main,
                };
                self.page_opened = Instant::now();
                self.update_backlight();
                Task::none()
            }
            Message::Exit => iced::exit(),
        }
    }

    /// Time of day from today's sunrise/sunset, or `None` before the
    /// first forecast.
    fn phase(&self) -> Option<Phase> {
        if let Some((phase, _)) = self.sky_override {
            return Some(phase);
        }
        let f = self.forecast.as_ref()?;
        let now = Local::now().naive_local();
        let (sunrise, sunset) = f.daily.sun_times(now.date())?;
        Some(Phase::at(now, sunrise, sunset))
    }

    /// Sky for right now: phase from sunrise/sunset, cover from current
    /// cloud cover and weather code. Night until the first forecast.
    fn sky_target(&self) -> Sky {
        if let Some((phase, cover)) = self.sky_override {
            return sky::target(phase, cover);
        }
        let (Some(phase), Some(f)) = (self.phase(), &self.forecast) else { return sky::NIGHT };
        let cover = Cover::from(f.current.cloud_cover, f.current.weather_code);
        sky::target(phase, cover)
    }

    /// Re-pick the current-conditions icon. Day/night comes from our own
    /// sunrise/sunset phase (minute-accurate, matches the background)
    /// rather than Open-Meteo's 15-minute `is_day` flag.
    fn refresh_icon(&mut self) {
        let Some(f) = &self.forecast else { return };
        let daylight = self.phase().map_or(f.current.is_day == 1, Phase::daylight);
        let name = weather::icon_name(f.current.weather_code, daylight);
        // Key includes the variant so the outline switches with the sky.
        let key = format!("{name}{}", if sky::outline_icons(self.sky.at(77.5 / HEIGHT)) { "+outline" } else { "" });
        if key != self.icon_name {
            self.icon = Some(self.icon_handle(&name, 77.5, true));
            self.icon_name = key;
        }
    }

    /// Weather icon `name` for drawing centred at height `cy`: the
    /// dark-outlined variant where the sky behind it is bright. `fine`
    /// picks the thinner outline used for the big icon.
    fn icon_handle(&self, name: &str, cy: f32, fine: bool) -> image::Handle {
        let mut path = self.assets.join("icons");
        if sky::outline_icons(self.sky.at(cy / HEIGHT)) {
            path.push(if fine { "outlined-fine" } else { "outlined" });
        }
        image::Handle::from_path(path.join(format!("{name}.png")))
    }

    /// Dim at night, brighten through dawn; full brightness while the
    /// forecast or sensor page is open (someone is looking).
    fn update_backlight(&mut self) {
        let (day, night) = (self.config.day_brightness, self.config.night_brightness);
        let level = match (self.page, self.phase()) {
            (Page::Main, Some(phase)) => phase.brightness(day, night),
            _ => day,
        };
        if let Some(bl) = &mut self.backlight {
            bl.set_percent(level);
        }
    }

    fn fetch(&self, source: Source) -> Task<Message> {
        let client = self.client.clone();
        match source {
            Source::Forecast => Task::perform(
                weather::fetch_forecast(client, self.config.forecast_url()),
                |r| Message::ForecastLoaded(r.map(Box::new)),
            ),
            Source::LocalTemp => Task::perform(
                weather::fetch_local_temp(client, self.config.local_temp_url.clone()),
                Message::LocalTempLoaded,
            ),
            Source::LocalHumidity => Task::perform(
                weather::fetch_local_humidity(client, self.config.local_humidity_url.clone()),
                Message::LocalHumidityLoaded,
            ),
            Source::Sensor => Task::perform(
                weather::fetch_sensor(client, self.config.sensor_url.clone()),
                Message::SensorLoaded,
            ),
            Source::LocalPressure => Task::perform(
                weather::fetch_local_pressure(client, self.config.local_pressure_url.clone()),
                Message::LocalPressureLoaded,
            ),
            Source::Alerts => Task::perform(
                alerts::fetch_alerts(client, self.config.latitude, self.config.longitude),
                Message::AlertsLoaded,
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

    // ---------- Views ----------

    fn view(&self) -> Element<'_, Message> {
        let layout = match self.page {
            Page::Forecast => self.view_details(),
            Page::Sensor => self.view_sensor(),
            Page::Main => self.view_main(),
        };

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

        // Kiosk display: hide the pointer everywhere (Xojo InvisibleCursor);
        // a tap anywhere moves to the next page.
        mouse_area(screen)
            .interaction(mouse::Interaction::Hidden)
            .on_press(Message::Tap)
            .into()
    }

    fn view_main(&self) -> Element<'_, Message> {
        let current = self.forecast.as_ref().map(|f| &f.current);
        let stale_fc = is_stale(self.forecast_at);
        let stale_temp = is_stale(self.local_temp_at);
        let stale_hum = is_stale(self.local_humidity_at);
        let stale_pres = is_stale(self.local_pressure_at);

        let current_temp = match (&self.forecast_error, current) {
            (Some(e), _) => format!("Error getting data: {e}"),
            (None, Some(c)) => format!("Current Temp: {}° F", fmt_plain(c.temperature_2m)),
            (None, None) => "Current Temp: --° F".into(),
        };
        let wind_chill = current.map_or("--".into(), |c| fmt_plain(c.apparent_temperature));
        let wind_speed = current.map_or("--".into(), |c| format!("{}", c.wind_speed_10m.round()));
        let tomorrow = self.day_after(1);
        let opt = |v: Option<f64>| v.map_or("--".into(), fmt_1dp);

        let regular = self.font;
        let bold = Font { weight: Weight::Bold, ..self.font };
        let box_ink = self.ink(GREY, 321.0, 139.0);

        let mut layout = stack![
            // Clock, current conditions (top left)
            self.clock_and_date(regular),
            self.label(&current_temp, 48.0, regular, GREY, (20.0, 112.0, 573.0, 83.0), Horizontal::Left, stale_fc),
            self.label(
                &format!("Windchill Temp: {wind_chill}° F"),
                36.0, regular, GREY, (20.0, 163.0, 451.0, 83.0), Horizontal::Left, stale_fc,
            ),
            self.label(
                &format!("{wind_speed} Mph"),
                24.0, regular, GREY, (496.0, 132.0, 217.0, 83.0), Horizontal::Right, stale_fc,
            ),
            self.sun_times(),
            // Tomorrow's forecast box (Xojo GroupBox1 + ForecastDay1/Max/Min,
            // plus icon and chance of precipitation in the space below)
            pin(container("").width(144).height(139).style(move |_| container::Style {
                border: Border { color: box_ink, width: 1.0, radius: 0.0.into() },
                ..Default::default()
            }))
            .x(20)
            .y(321),
            self.label(
                &tomorrow.as_ref().map_or("--".into(), |d| d.date.format("%a %-d").to_string()),
                20.0, regular, WHITE, (20.0, 329.0, 141.0, 20.0), Horizontal::Center, stale_fc,
            ),
            self.label(
                &format!("Max Temp: {}°F", opt(tomorrow.as_ref().and_then(|d| d.max))),
                12.0, bold, WHITE, (20.0, 355.0, 141.0, 20.0), Horizontal::Center, stale_fc,
            ),
            self.label(
                &format!("Min Temp: {}°F", opt(tomorrow.as_ref().and_then(|d| d.min))),
                12.0, bold, WHITE, (20.0, 376.0, 141.0, 20.0), Horizontal::Center, stale_fc,
            ),
            self.label(
                &format!("Precip: {}%", tomorrow.as_ref().and_then(|d| d.precipitation).map_or("--".into(), |p| format!("{p:.0}"))),
                12.0, bold, WHITE, (20.0, 436.0, 141.0, 18.0), Horizontal::Center, stale_fc,
            ),
            // Pico W sensor
            self.label(
                &format!("Local Temp: {}° F", self.local_temp.map_or("--".into(), fmt_1dp)),
                50.0, regular, GREY, (176.0, 321.0, 604.0, 52.0), Horizontal::Left, stale_temp,
            ),
            // Humidity and pressure, larger than Xojo's 12 px humidity label.
            self.label(
                &format!("Humidity {}%", self.local_humidity.map_or("--".into(), fmt_1dp)),
                22.0, regular, GREY, (176.0, 378.0, 200.0, 34.0), Horizontal::Left, stale_hum,
            ),
            self.label(
                &format!("Pressure {}", self.pressure_text()),
                22.0, regular, GREY, (380.0, 378.0, 400.0, 34.0), Horizontal::Left, stale_pres,
            ),
            self.label(
                &format!(
                    "Local Temp Last Updated: {}",
                    self.local_temp_updated.map_or("--".into(), |t| t.format("%-H:%M").to_string())
                ),
                12.0, bold, WHITE, (364.0, 440.0, 187.0, 20.0), Horizontal::Left, stale_temp,
            ),
        ]
        .width(WIDTH)
        .height(HEIGHT);

        // Tomorrow's weather icon inside the box.
        if let Some(code) = tomorrow.as_ref().and_then(|d| d.code) {
            layout = layout.push(
                pin(image(self.icon_handle(&weather::icon_name(code, true), 414.0, false))
                    .width(44)
                    .height(44)
                    .opacity(if stale_fc { 0.5_f32 } else { 1.0 }))
                .x(70)
                .y(392),
            );
        }

        if let Some(icon) = &self.icon {
            // Xojo `DayNight` canvas: 120x115 at (653, 20).
            layout = layout.push(
                pin(image(icon.clone())
                    .width(120)
                    .height(115)
                    .content_fit(ContentFit::Contain)
                    .opacity(if stale_fc { 0.5_f32 } else { 1.0 }))
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
            pin(tinted(self.wind_icon.clone(), 55.0, 56.0, wind_ink)).x(718).y(139),
        );

        if let Some(banner) = self.alert_banner(20.0, 256.0, 760.0, 44.0, false) {
            layout = layout.push(banner);
        }

        layout.into()
    }

    /// Details page: three day columns, the active alert, and a
    /// next-24-hours chart.
    fn view_details(&self) -> Element<'_, Message> {
        let regular = self.font;
        let bold = Font { weight: Weight::Bold, ..self.font };
        let mut layout = stack![
            self.label("3-Day Forecast", 26.0, bold, WHITE, (20.0, 10.0, 400.0, 36.0), Horizontal::Left, false),
            self.label("Tap for sensor history", 14.0, regular, WHITE, (500.0, 10.0, 280.0, 36.0), Horizontal::Right, false),
        ]
        .width(WIDTH)
        .height(HEIGHT);

        let today = Local::now().date_naive();
        for i in 0..3 {
            let x = 20.0 + 260.0 * i as f32;
            let w = 240.0;
            let day = self.day_after(i);
            let name = match i {
                0 => "Today".to_owned(),
                1 => "Tomorrow".to_owned(),
                _ => (today + TimeDelta::days(i)).format("%A").to_string(),
            };
            layout = layout.push(self.label(&name, 22.0, bold, WHITE, (x, 52.0, w, 28.0), Horizontal::Center, false));
            if let Some(code) = day.as_ref().and_then(|d| d.code) {
                let cx = x + w / 2.0;
                layout = layout.push(
                    pin(image(self.icon_handle(&weather::icon_name(code, true), 118.0, false)).width(72).height(72))
                        .x(cx - 36.0)
                        .y(82),
                );
            }
            let opt = |v: Option<f64>| v.map_or("--".into(), fmt_1dp);
            let hi_lo = format!(
                "High {}°  Low {}°",
                opt(day.as_ref().and_then(|d| d.max)),
                opt(day.as_ref().and_then(|d| d.min))
            );
            let precip = day.as_ref().and_then(|d| d.precipitation).map_or("--".into(), |p| format!("{p:.0}"));
            let gusts = day.as_ref().and_then(|d| d.gusts).map_or("--".into(), |g| format!("{}", g.round()));
            layout = layout
                .push(self.label(&hi_lo, 18.0, regular, WHITE, (x, 158.0, w, 24.0), Horizontal::Center, false))
                .push(self.label(&format!("Precip {precip}%   Gusts {gusts} mph"), 15.0, regular, WHITE, (x, 186.0, w, 22.0), Horizontal::Center, false));
        }

        if let Some(banner) = self.alert_banner(20.0, 222.0, 760.0, 56.0, true) {
            layout = layout.push(banner);
        }

        // Next 24 hours. Chart ink follows the sky at the chart's middle.
        let hours = self.forecast.as_ref().map_or(vec![], |f| f.hourly.next(Local::now().naive_local(), 24));
        let ink = self.ink(WHITE, HEIGHT - chart::HEIGHT - 4.0, chart::HEIGHT);
        layout = layout.push(
            pin(canvas(chart::HourlyChart { hours, ink, font: regular }).width(760).height(chart::HEIGHT))
                .x(20)
                .y(HEIGHT - chart::HEIGHT - 4.0),
        );
        layout.into()
    }

    /// Sensor page: last 24 hours of outdoor temperature, humidity and
    /// pressure as three stacked charts sharing one time window.
    fn view_sensor(&self) -> Element<'_, Message> {
        let regular = self.font;
        let bold = Font { weight: Weight::Bold, ..self.font };
        let now = Local::now().naive_local();
        let start = now - TimeDelta::hours(24);
        let unit = self.pressure_unit;
        let reference = if self.config.pressure_sea_level { "sea level" } else { "station" };
        // (title, points, decimals, label suffix, minimum y-span)
        type Panel = (String, Vec<(chrono::NaiveDateTime, f64)>, usize, &'static str, f64);
        let charts: [Panel; 3] = [
            ("Temperature · °F".into(), self.history.series(now, 24, |s| s.temp_f), 1, "°", 4.0),
            ("Humidity · %".into(), self.history.series(now, 24, |s| s.humidity), 0, "%", 10.0),
            (
                format!("Pressure · {} ({reference})", unit.label()),
                self.history.series(now, 24, |s| {
                    Some(unit.value(self.reported_pressure(s.pressure_hpa?, s.temp_f)?))
                }),
                if unit == pressure::Unit::InHg { 2 } else { 1 },
                "",
                if unit == pressure::Unit::InHg { 0.1 } else { 3.0 },
            ),
        ];

        let mut layout = stack![
            self.label("Outdoor Sensor · Last 24 Hours", 26.0, bold, WHITE, (20.0, 10.0, 500.0, 36.0), Horizontal::Left, false),
            self.label("Tap to return", 14.0, regular, WHITE, (580.0, 10.0, 200.0, 36.0), Horizontal::Right, false),
        ]
        .width(WIDTH)
        .height(HEIGHT);
        let (top, gap, height) = (52.0, 4.0, 138.0);
        for (i, (title, points, decimals, suffix, min_span)) in charts.into_iter().enumerate() {
            let y = top + (height + gap) * i as f32;
            let last = i == 2;
            let h = if last { height + 14.0 } else { height };
            let chart = chart::SeriesChart {
                points,
                start,
                end: now,
                title,
                decimals,
                suffix: suffix.into(),
                min_span,
                show_axis: last,
                ink: self.ink(WHITE, y, h),
                font: regular,
            };
            layout = layout.push(pin(canvas(chart).width(760).height(h)).x(20).y(y));
        }
        layout.into()
    }

    /// Station pressure converted for display: sea level (default) using
    /// the sensor's elevation and outdoor temperature, or as measured.
    fn reported_pressure(&self, station_hpa: f64, temp_f: Option<f64>) -> Option<f64> {
        let station_hpa = station_hpa + self.config.pressure_offset_hpa;
        if !self.config.pressure_sea_level {
            return Some(station_hpa);
        }
        let elevation = self.config.elevation_m.or(self.forecast.as_ref()?.elevation)?;
        let temp_c = (temp_f? - 32.0) * 5.0 / 9.0;
        Some(pressure::sea_level_hpa(station_hpa, elevation, temp_c))
    }

    /// "30.02 inHg ↑" (arrow once 3 hours of history exist).
    fn pressure_text(&self) -> String {
        let Some(now_hpa) = self.local_pressure.and_then(|p| self.reported_pressure(p, self.local_temp)) else {
            return "--".into();
        };
        let mut s = self.pressure_unit.format(now_hpa);
        // Trend on station pressure: elevation cancels out of the difference.
        if let (Some(p), Some(then)) =
            (self.local_pressure, self.history.pressure_3h_ago(Local::now().naive_local()))
        {
            s.push(' ');
            s.push_str(pressure::Trend::from_change(p - then).arrow());
        }
        s
    }

    /// Save a history sample (every 5 minutes) from readings no older than
    /// 10 minutes.
    fn record_history(&mut self) {
        let fresh = |v: Option<f64>, at: Option<Instant>| {
            v.filter(|_| at.is_some_and(|t| t.elapsed() < Duration::from_secs(600)))
        };
        self.history.record(history::Sample {
            t: Local::now().naive_local(),
            temp_f: fresh(self.local_temp, self.local_temp_at),
            humidity: fresh(self.local_humidity, self.local_humidity_at),
            pressure_hpa: fresh(self.local_pressure, self.local_pressure_at),
        });
    }

    /// Forecast for today + `days`, looked up by date so it stays right
    /// across midnight until the next refresh.
    fn day_after(&self, days: i64) -> Option<DayForecast> {
        let date: NaiveDate = Local::now().date_naive() + TimeDelta::days(days);
        self.forecast.as_ref()?.daily.on(date)
    }

    /// Today's sunrise and sunset, right-aligned below the wind speed.
    fn sun_times(&self) -> Element<'_, Message> {
        let times = self
            .forecast
            .as_ref()
            .and_then(|f| f.daily.sun_times(Local::now().date_naive()));
        let fmt = |t: Option<chrono::NaiveDateTime>| t.map_or("--".into(), |t| t.format("%-H:%M").to_string());
        let (rise, set) = (fmt(times.map(|t| t.0)), fmt(times.map(|t| t.1)));
        let ink = self.ink(GREY, 212.0, 36.0);
        let label = |s: String| text(s).size(20).font(self.font).color(ink);
        pin(
            container(
                row![
                    tinted(self.sunrise_icon.clone(), 36.0, 36.0, ink),
                    label(rise),
                    Space::new().width(14),
                    tinted(self.sunset_icon.clone(), 36.0, 36.0, ink),
                    label(set),
                ]
                .spacing(4)
                .align_y(Vertical::Center),
            )
            .width(277)
            .height(36)
            .align_x(Horizontal::Right)
            .align_y(Vertical::Center),
        )
        .x(496)
        .y(212)
        .into()
    }

    /// Banner for the most important active alert. `full` shows the
    /// headline (details page); otherwise a one-line summary.
    fn alert_banner<'a>(&self, x: f32, y: f32, w: f32, h: f32, full: bool) -> Option<Element<'a, Message>> {
        let first = self.alerts.first()?;
        let bg = level_color(&first.level);
        let fg = sky::text_on(bg, Color::WHITE);
        let summary = alerts::banner_text(first, self.alerts.len() - 1);
        let content = if full {
            first.headline.clone().unwrap_or(summary)
        } else {
            summary
        };
        Some(
            pin(
                container(
                    text(content)
                        .size(if full { 16 } else { 20 })
                        .font(Font { weight: Weight::Bold, ..self.font })
                        .color(fg)
                        .wrapping(if full { text::Wrapping::Word } else { text::Wrapping::None }),
                )
                .width(w)
                .height(h)
                .padding([0, 14])
                .align_y(Vertical::Center)
                .clip(true)
                .style(move |_| container::Style {
                    background: Some(Background::Color(bg)),
                    border: Border { radius: 6.0.into(), ..Default::default() },
                    ..Default::default()
                }),
            )
            .x(x)
            .y(y)
            .into(),
        )
    }

    /// Readable text colour for a label whose box is (y, h), given its
    /// normal light colour.
    fn ink(&self, light: Color, y: f32, h: f32) -> Color {
        sky::text_on(self.sky.at((y + h / 2.0) / HEIGHT), light)
    }

    /// Clock (Xojo `currentTime`: 80 px in a 348x115 box at 20,20) with the
    /// date right after it at 56 px. Bottom-aligned so the two sit on
    /// roughly the same baseline; the row grows right as the time widens.
    fn clock_and_date<'a>(&self, font: Font) -> Element<'a, Message> {
        let ink = self.ink(GREY, 20.0, 115.0);
        let clock = text(self.clock.clone()).size(80).font(font).color(ink);
        // Nudge up so the smaller text's baseline meets the clock's.
        let date = container(text(self.date.clone()).size(56).font(font).color(ink))
            .padding(iced::Padding { bottom: 6.0, ..Default::default() });
        pin(
            container(row![clock, date].spacing(24).align_y(Vertical::Bottom))
                .height(115)
                .align_y(Vertical::Center),
        )
        .x(20)
        .y(20)
        .into()
    }

    /// A Xojo `DesktopLabel`: fixed box at (x, y, w, h), text vertically
    /// centred, single line. `dim` draws it faded (data is stale).
    #[allow(clippy::too_many_arguments)]
    fn label<'a>(
        &self,
        content: &str,
        size: f32,
        font: Font,
        color: Color,
        (x, y, w, h): (f32, f32, f32, f32),
        align: Horizontal,
        dim: bool,
    ) -> Element<'a, Message> {
        let ink = self.ink(color, y, h);
        let ink = if dim { Color { a: 0.45, ..ink } } else { ink };
        pin(
            container(text(content.to_owned()).size(size).font(font).color(ink).wrapping(text::Wrapping::None))
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

/// Monochrome SVG icon recoloured to `ink`.
fn tinted<'a>(handle: svg::Handle, w: f32, h: f32, ink: Color) -> Element<'a, Message> {
    svg(handle)
        .width(w)
        .height(h)
        .content_fit(ContentFit::Contain)
        .style(move |_, _| svg::Style { color: Some(ink) })
        .into()
}

fn is_stale(at: Option<Instant>) -> bool {
    at.is_some_and(|t| t.elapsed() > STALE_AFTER)
}

/// Banner colour by alert type: warnings red, watches orange, advisories
/// amber, statements slate.
fn level_color(level: &Level) -> Color {
    match level {
        Level::Warning => Color::from_rgb8(0xB3, 0x26, 0x1E),
        Level::Watch => Color::from_rgb8(0xC2, 0x41, 0x0C),
        Level::Advisory => Color::from_rgb8(0xE0, 0xA8, 0x00),
        Level::Statement => Color::from_rgb8(0x47, 0x55, 0x69),
    }
}

/// Testing aid: `WEATHER_APP_ALERT="Winter Storm Warning"` shows a fake
/// alert so the banner can be previewed.
fn test_alert() -> Option<Alert> {
    let event = std::env::var("WEATHER_APP_ALERT").ok()?;
    Some(Alert {
        level: Level::from_event(&event),
        headline: Some(format!("{event} issued for testing; this is not a real alert.")),
        description: None,
        severity: "Severe".into(),
        ends: Some((Local::now() + TimeDelta::hours(30)).fixed_offset()),
        event,
    })
}

/// `WEATHER_APP_SKY` value -> fixed (phase, cover); twilight at its peak.
fn parse_sky(s: &str) -> Option<(Phase, Cover)> {
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
    Some((phase, cover))
}

/// Xojo `d.ToString("H:mm")`.
fn clock_text() -> String {
    Local::now().format("%-H:%M").to_string()
}

/// Short weekday and date, e.g. "Sun Oct 25".
fn date_text(date: NaiveDate) -> String {
    date.format("%a %b %-d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_is_short_weekday_month_day() {
        let d = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert_eq!(date_text(d(2026, 10, 25)), "Sun Oct 25");
        assert_eq!(date_text(d(2026, 11, 3)), "Tue Nov 3");
    }

    #[test]
    fn alert_banners_are_readable() {
        for level in [Level::Warning, Level::Watch, Level::Advisory, Level::Statement] {
            let bg = level_color(&level);
            assert!(sky::contrast(sky::text_on(bg, Color::WHITE), bg) >= 4.5, "{level:?}");
        }
    }
}
