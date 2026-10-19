# pico_sensor

Outdoor sensor for [weather_app](../README.md): a Raspberry Pi **Pico W** with a **BME280**,
written in Rust with [Embassy](https://embassy.dev). Replaces the CircuitPython + DHT22 sensor
and serves the same HTTP API on port 5000, so the Pi display needs no changes.

| Request | Response |
|---|---|
| `GET /temperature` | `{"temperature": 61.88}` (°F) |
| `GET /humidity` | `{"humidity": 41.2}` (%RH) |
| `GET /pressure` | `{"pressure": 1013.25}` (hPa, new) |
| `GET /` | all three |

Until the first reading, or if the sensor stops answering for 60 s, these return
**503**, which the display treats as "keep the last value".

## Reliability

- Wi-Fi join retries every 5 s and rejoins if the link drops (the CircuitPython version
  connected once and gave up, so it stayed offline after a power cut).
- No Wi-Fi for 5 minutes: full reset.
- Hardware watchdog (8 s) resets the board if the firmware ever stalls; panics also reset.
- Sensor re-detected automatically after 3 failed reads (loose wire, hot-plug).
- Onboard LED is on while connected to Wi-Fi.
- BME280 runs in forced mode (sleeps between reads) so it doesn't warm itself.

## Wiring

| BME280 | Pico W |
|---|---|
| VIN / VCC | 3V3 (OUT), pin 36 |
| GND | GND, pin 38 |
| SDA | GP4, pin 6 |
| SCL | GP5, pin 7 |

I²C address 0x76 or 0x77 is detected automatically. A BMP280 (no humidity) sold as a BME280
is reported in the log. Outdoors, mount the sensor in a vented radiation shield, away from the
Pico W.

## Build and flash (from the Mac)

One-time setup:

```bash
rustup target add thumbv6m-none-eabi && cargo install elf2uf2-rs
```

Hold **BOOTSEL** on the Pico W while plugging in USB, then:

```bash
./flash.sh
```

It prompts for the Wi-Fi name and password (password is hidden) and an optional static IP.
To take over the old sensor's address so nothing else needs changing, answer
`192.168.88.146/24` and gateway `192.168.88.1` (unplug the old Pico first), or leave it blank
for DHCP and reserve an address in the router.

The Wi-Fi password is compiled into the firmware. It is never written to the repo, but treat
`pico_sensor.uf2` (from `./flash.sh --uf2`) as secret.

## Logs

The firmware logs over USB serial (IP address, sensor detection, Wi-Fi drops):

```bash
screen /dev/tty.usbmodem* 115200
```

Quit `screen` with Ctrl-A then K.

## Check it

```bash
curl http://192.168.88.146:5000/
```

Wi-Fi firmware blobs in `firmware/` are from the Embassy project (Infineon Permissive Binary
License, see `firmware/LICENSE-permissive-binary-license-1.0.txt`).
