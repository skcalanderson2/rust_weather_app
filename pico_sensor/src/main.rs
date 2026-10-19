//! Pico W + BME280 outdoor sensor for `weather_app`.
//!
//! Serves the same HTTP API as the original CircuitPython/DHT22 sensor on
//! port 5000, so the Pi display needs no changes:
//!
//!   GET /temperature  -> {"temperature": 54.68}   (°F)
//!   GET /humidity     -> {"humidity": 41.2}       (%RH)
//!   GET /pressure     -> {"pressure": 1013.25}    (hPa, new with the BME280)
//!   GET /             -> all three
//!
//! Until the first good reading (or if the sensor stops answering) these
//! return HTTP 503, which the display treats as "keep the last value".
//!
//! Reliability, the reason for the rewrite:
//! - Wi-Fi join retries forever, and rejoins if the link drops.
//! - No link for `NO_LINK_RESET` -> full reset.
//! - Hardware watchdog resets the chip if the executor ever stalls.
//! - Panics reset the chip (`panic-reset`).
//!
//! Wiring (BME280 breakout, I2C):  VIN -> 3V3 (pin 36), GND -> GND (pin 38),
//! SDA -> GP4 (pin 6), SCL -> GP5 (pin 7). Address 0x76 or 0x77, auto-detected.
//!
//! Build-time settings (environment variables, never committed):
//!   WIFI_SSID, WIFI_PASSWORD          required
//!   STATIC_IP=192.168.88.146/24       optional; DHCP if unset
//!   GATEWAY=192.168.88.1              optional, used with STATIC_IP
//!
//! Logs (IP address, sensor status) go to the USB serial port.

#![no_std]
#![no_main]

use core::fmt::Write as _;

use bme280_rs::{AsyncBme280, CHIP_ID, Configuration, Filter, Oversampling, Sample, SensorMode};
use cyw43::{JoinOptions, aligned_bytes};
use cyw43_pio::{DEFAULT_CLOCK_DIVIDER, PioSpi};
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::{Ipv4Address, Ipv4Cidr, Stack, StackResources, StaticConfigV4};
use embassy_rp::clocks::RoscRng;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::i2c::{self, I2c};
use embassy_rp::peripherals::{DMA_CH0, I2C0, PIO0, USB};
use embassy_rp::pio::{self, Pio};
use embassy_rp::watchdog::Watchdog;
use embassy_rp::{bind_interrupts, dma, usb};
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Delay, Duration, Instant, Timer};
use embedded_io_async::Write as _;
use heapless::String;
use static_cell::StaticCell;
use {panic_reset as _};

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => pio::InterruptHandler<PIO0>;
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>;
    I2C0_IRQ => i2c::InterruptHandler<I2C0>;
    USBCTRL_IRQ => usb::InterruptHandler<USB>;
});

const WIFI_SSID: &str = env!("WIFI_SSID");
const WIFI_PASSWORD: &str = env!("WIFI_PASSWORD");
const STATIC_IP: Option<&str> = option_env!("STATIC_IP");
const GATEWAY: Option<&str> = option_env!("GATEWAY");

const HTTP_PORT: u16 = 5000;
/// Concurrent HTTP connections (one task each).
const HTTP_TASKS: usize = 2;
/// How often the BME280 is read.
const SENSOR_PERIOD: Duration = Duration::from_secs(5);
/// Readings older than this are reported as unavailable (503).
const STALE_AFTER: Duration = Duration::from_secs(60);
/// Reset the whole board if Wi-Fi has been down this long.
const NO_LINK_RESET: Duration = Duration::from_secs(300);

#[derive(Clone, Copy)]
struct Reading {
    temperature_f: f32,
    humidity: f32,
    pressure_hpa: f32,
    at: Instant,
}

static LATEST: Mutex<CriticalSectionRawMutex, core::cell::Cell<Option<Reading>>> =
    Mutex::new(core::cell::Cell::new(None));

fn latest_fresh() -> Option<Reading> {
    LATEST
        .lock(|c| c.get())
        .filter(|r| r.at.elapsed() < STALE_AFTER)
}

// ---------- Tasks ----------

#[embassy_executor::task]
async fn logger_task(driver: usb::Driver<'static, USB>) {
    embassy_usb_logger::run!(1024, log::LevelFilter::Info, driver);
}

#[embassy_executor::task]
async fn cyw43_task(
    runner: cyw43::Runner<'static, cyw43::SpiBus<Output<'static>, PioSpi<'static, PIO0, 0>>>,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, cyw43::NetDriver<'static>>) -> ! {
    runner.run().await
}

/// Feed the hardware watchdog. If the executor stalls, this stops running
/// and the RP2040 resets after ~8 s.
#[embassy_executor::task]
async fn watchdog_task(mut wd: Watchdog) -> ! {
    wd.start(Duration::from_secs(8));
    loop {
        wd.feed(Duration::from_secs(8));
        Timer::after_secs(2).await;
    }
}

type Bme = AsyncBme280<I2c<'static, I2C0, i2c::Async>, Delay>;

/// Try the BME280 at `address`: reset, load calibration, check the chip ID,
/// and set forced mode with 1x oversampling (Bosch's "weather monitoring"
/// profile; the sensor sleeps between reads so it doesn't warm itself).
async fn detect(mut bme: Bme, address: u8) -> Result<Bme, Bme> {
    if bme.init().await.is_err() {
        return Err(bme);
    }
    match bme.chip_id().await {
        Ok(CHIP_ID) => {}
        Ok(0x58) => {
            log::error!("0x{address:02x} is a BMP280: it has no humidity sensor");
            return Err(bme);
        }
        _ => return Err(bme),
    }
    let config = Configuration::default()
        .with_temperature_oversampling(Oversampling::Oversample1)
        .with_pressure_oversampling(Oversampling::Oversample1)
        .with_humidity_oversampling(Oversampling::Oversample1)
        .with_filter(Filter::Off)
        .with_sensor_mode(SensorMode::Forced);
    match bme.set_sampling_configuration(config).await {
        Ok(()) => {
            log::info!("BME280 found at 0x{address:02x}");
            Ok(bme)
        }
        Err(_) => Err(bme),
    }
}

#[embassy_executor::task]
async fn sensor_task(mut i2c: I2c<'static, I2C0, i2c::Async>) -> ! {
    loop {
        // Detect the sensor at either address; retry until it answers.
        let mut bme = 'found: loop {
            for address in [0x76, 0x77] {
                match detect(AsyncBme280::new_with_address(i2c, address, Delay), address).await {
                    Ok(bme) => break 'found bme,
                    Err(bme) => i2c = bme.release(),
                }
            }
            log::warn!("BME280 not found on GP4/GP5, retrying");
            Timer::after(SENSOR_PERIOD).await;
        };

        // Read until the sensor stops answering, then re-detect.
        let mut failures = 0u8;
        while failures < 3 {
            let sample = match bme.take_forced_measurement().await {
                Ok(_) => bme.read_sample().await.ok(),
                Err(_) => None,
            };
            match sample {
                Some(Sample { temperature: Some(t), humidity: Some(h), pressure: Some(p) }) => {
                    failures = 0;
                    LATEST.lock(|c| {
                        c.set(Some(Reading {
                            temperature_f: t * 9.0 / 5.0 + 32.0,
                            humidity: h,
                            pressure_hpa: p / 100.0,
                            at: Instant::now(),
                        }))
                    });
                }
                _ => {
                    failures += 1;
                    log::warn!("BME280 read failed ({failures}/3)");
                }
            }
            Timer::after(SENSOR_PERIOD).await;
        }
        i2c = bme.release();
    }
}

#[embassy_executor::task(pool_size = HTTP_TASKS)]
async fn http_task(stack: Stack<'static>, id: usize) -> ! {
    let mut rx = [0u8; 1024];
    let mut tx = [0u8; 1024];
    let mut buf = [0u8; 512];
    loop {
        let mut socket = TcpSocket::new(stack, &mut rx, &mut tx);
        socket.set_timeout(Some(Duration::from_secs(10)));
        if socket.accept(HTTP_PORT).await.is_err() {
            continue;
        }

        // Read until the end of the request headers (or the buffer fills).
        let mut len = 0;
        while len < buf.len() {
            match socket.read(&mut buf[len..]).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    len += n;
                    if buf[..len].windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
            }
        }

        let (status, body) = respond(request_path(&buf[..len]));
        let mut head: String<128> = String::new();
        let _ = write!(
            head,
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
             Access-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = socket.write_all(head.as_bytes()).await;
        let _ = socket.write_all(body.as_bytes()).await;
        let _ = socket.flush().await;
        socket.close();
        Timer::after_millis(20).await;
        socket.abort();
        log::debug!("http[{id}] served");
    }
}

// ---------- HTTP helpers ----------

/// `GET /humidity HTTP/1.1` -> `Some("/humidity")`. Trailing slash and
/// query string are ignored, matching CircuitPython's `append_slash`.
fn request_path(req: &[u8]) -> Option<&str> {
    let line = core::str::from_utf8(req).ok()?.lines().next()?;
    let mut parts = line.split(' ');
    if parts.next()? != "GET" {
        return None;
    }
    let path = parts.next()?.split('?').next()?;
    Some(if path.len() > 1 { path.trim_end_matches('/') } else { path })
}

fn respond(path: Option<&str>) -> (&'static str, String<160>) {
    let mut body: String<160> = String::new();
    let reading = latest_fresh();
    let status = match (path, reading) {
        (Some("/temperature"), Some(r)) => {
            let _ = write!(body, "{{\"temperature\": {:.2}}}", r.temperature_f);
            "200 OK"
        }
        (Some("/humidity"), Some(r)) => {
            let _ = write!(body, "{{\"humidity\": {:.1}}}", r.humidity);
            "200 OK"
        }
        (Some("/pressure"), Some(r)) => {
            let _ = write!(body, "{{\"pressure\": {:.2}}}", r.pressure_hpa);
            "200 OK"
        }
        (Some("/"), Some(r)) => {
            let _ = write!(
                body,
                "{{\"temperature\": {:.2}, \"humidity\": {:.1}, \"pressure\": {:.2}}}",
                r.temperature_f, r.humidity, r.pressure_hpa
            );
            "200 OK"
        }
        (Some("/" | "/temperature" | "/humidity" | "/pressure"), None) => {
            let _ = body.push_str("{\"error\": \"sensor not ready\"}");
            "503 Service Unavailable"
        }
        _ => {
            let _ = body.push_str("{\"error\": \"not found\"}");
            "404 Not Found"
        }
    };
    (status, body)
}

// ---------- Network config ----------

/// Parse `a.b.c.d` (and `/prefix` when `with_prefix`).
fn parse_ipv4(s: &str) -> Option<(Ipv4Address, u8)> {
    let (addr, prefix) = match s.split_once('/') {
        Some((a, p)) => (a, p.parse().ok()?),
        None => (s, 24),
    };
    let mut octets = [0u8; 4];
    let mut parts = addr.split('.');
    for o in &mut octets {
        *o = parts.next()?.parse().ok()?;
    }
    if parts.next().is_some() || prefix > 32 {
        return None;
    }
    Some((Ipv4Address::from(octets), prefix))
}

fn net_config() -> embassy_net::Config {
    let Some((address, prefix)) = STATIC_IP.and_then(parse_ipv4) else {
        return embassy_net::Config::dhcpv4(Default::default());
    };
    let gateway = GATEWAY.and_then(parse_ipv4).map(|(g, _)| g);
    let mut dns_servers = heapless::Vec::new();
    if let Some(g) = gateway {
        let _ = dns_servers.push(g);
    }
    embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(address, prefix),
        gateway,
        dns_servers,
    })
}

// ---------- Main: bring-up and Wi-Fi supervision ----------

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    spawner.spawn(watchdog_task(Watchdog::new(p.WATCHDOG)).unwrap());
    spawner.spawn(logger_task(usb::Driver::new(p.USB, Irqs)).unwrap());

    // BME280 on I2C0: SDA = GP4, SCL = GP5. Breakouts have pull-ups.
    let mut i2c_config = i2c::Config::default();
    i2c_config.frequency = 100_000;
    let i2c = I2c::new_async(p.I2C0, p.PIN_5, p.PIN_4, Irqs, i2c_config);
    spawner.spawn(sensor_task(i2c).unwrap());

    // CYW43 Wi-Fi chip (fixed Pico W wiring).
    let fw = aligned_bytes!("../firmware/43439A0.bin");
    let clm = aligned_bytes!("../firmware/43439A0_clm.bin");
    let nvram = aligned_bytes!("../firmware/nvram_rp2040.bin");

    let pwr = Output::new(p.PIN_23, Level::Low);
    let cs = Output::new(p.PIN_25, Level::High);
    let mut pio = Pio::new(p.PIO0, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        DEFAULT_CLOCK_DIVIDER,
        pio.irq0,
        cs,
        p.PIN_24,
        p.PIN_29,
        dma::Channel::new(p.DMA_CH0, Irqs),
    );

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let (net_device, mut control, runner) =
        cyw43::new(STATE.init(cyw43::State::new()), pwr, spi, fw, nvram).await;
    spawner.spawn(cyw43_task(runner).unwrap());

    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;

    static RESOURCES: StaticCell<StackResources<{ HTTP_TASKS + 2 }>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(
        net_device,
        net_config(),
        RESOURCES.init(StackResources::new()),
        RoscRng.next_u64(),
    );
    spawner.spawn(net_task(runner).unwrap());
    for id in 0..HTTP_TASKS {
        spawner.spawn(http_task(stack, id).unwrap());
    }

    // Join, then watch the link forever. Rejoin on drop; reset if the
    // network stays unreachable (e.g. router still booting after an outage
    // longer than expected, or a wedged radio).
    let mut down_since = Instant::now();
    loop {
        control.gpio_set(0, false).await; // LED off while offline
        log::info!("joining {WIFI_SSID}...");
        match control
            .join(WIFI_SSID, JoinOptions::new(WIFI_PASSWORD.as_bytes()))
            .await
        {
            Ok(()) => {}
            Err(e) => {
                log::warn!("join failed: {}", match e {
                    cyw43::JoinError::NetworkNotFound => "network not found",
                    cyw43::JoinError::AuthenticationFailure => "wrong password",
                    cyw43::JoinError::JoinFailure(_) => "join failure",
                });
                if down_since.elapsed() > NO_LINK_RESET {
                    log::error!("no Wi-Fi for 5 min, resetting");
                    Timer::after_millis(200).await;
                    cortex_m::peripheral::SCB::sys_reset();
                }
                Timer::after_secs(5).await;
                continue;
            }
        }

        stack.wait_config_up().await;
        if let Some(cfg) = stack.config_v4() {
            log::info!("online at {} (port {HTTP_PORT})", cfg.address);
        }
        control.gpio_set(0, true).await; // LED on while online

        while stack.is_link_up() {
            Timer::after_secs(5).await;
        }
        log::warn!("Wi-Fi link lost, rejoining");
        control.leave().await;
        down_since = Instant::now();
    }
}
