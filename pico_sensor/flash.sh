#!/usr/bin/env bash
# Build pico_sensor and flash it to a Pico W in BOOTSEL mode.
#
# 1. Hold BOOTSEL on the Pico W while plugging in USB (an RPI-RP2 drive appears).
# 2. ./flash.sh
#
# Wi-Fi settings come from the environment, or are prompted for (the password
# is read silently, so it never lands in shell history or in git):
#   WIFI_SSID, WIFI_PASSWORD   required
#   STATIC_IP                  optional, e.g. 192.168.88.146/24 (DHCP if unset)
#   GATEWAY                    optional, e.g. 192.168.88.1
#
# Pass --uf2 to only write pico_sensor.uf2 (drag it onto RPI-RP2 yourself).
set -euo pipefail
cd "$(dirname "$0")"

if [[ -z "${WIFI_SSID:-}" ]]; then
    read -rp "Wi-Fi network name: " WIFI_SSID
fi
if [[ -z "${WIFI_PASSWORD:-}" ]]; then
    read -rsp "Wi-Fi password: " WIFI_PASSWORD
    echo
fi
if [[ -z "${STATIC_IP+x}" ]]; then
    read -rp "Static IP with prefix (blank = DHCP), e.g. 192.168.88.146/24: " STATIC_IP
fi
if [[ -n "${STATIC_IP}" && -z "${GATEWAY:-}" ]]; then
    read -rp "Gateway, e.g. 192.168.88.1: " GATEWAY
fi
export WIFI_SSID WIFI_PASSWORD
[[ -n "${STATIC_IP}" ]] && export STATIC_IP GATEWAY || unset STATIC_IP GATEWAY

ELF=target/thumbv6m-none-eabi/release/pico_sensor
cargo build --release

if [[ "${1:-}" == "--uf2" ]]; then
    elf2uf2-rs "$ELF" pico_sensor.uf2
    echo "Wrote pico_sensor.uf2 (contains your Wi-Fi password; don't share it)"
else
    elf2uf2-rs -d "$ELF"
    echo "Flashed. Watch logs with: screen /dev/tty.usbmodem* 115200"
fi
