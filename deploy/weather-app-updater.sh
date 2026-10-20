#!/usr/bin/env bash
# Watch the build tarball on the NAS and install new versions of weather_app.
#
# Runs on the Pi as a systemd user service (see weather-app-updater.service).
# Flow: ./build-pi.sh on the Mac -> Synology Drive syncs dist/ to the NAS ->
# this script sees a new tarball -> installs it -> restarts the app.
#
# Polls instead of using inotify: inotify does not see changes made on the
# NFS server (the NAS) by other machines.
#
# Restart relies on the autostart wrapper
#   sh -c 'cd ~/weather_app && until ./weather_app; do sleep 5; done'
# Killing the app makes it exit non-zero, so the wrapper starts the new binary
# inside the desktop session. This script never needs Wayland access.
#
# Settings (environment, e.g. in the service file):
#   TARBALL        path to weather_app-pi.tar.gz on the NAS mount
#   INSTALL_DIR    where the app lives (default ~/weather_app)
#   POLL_SECONDS   how often to check (default 30)
#   UPDATE_CONFIG  1 = also replace config.toml (default 0: keep the Pi's copy)
set -euo pipefail

TARBALL="${TARBALL:-/mnt/nas/sysala/sysala/rust_ai/rust_weather_app/dist/weather_app-pi.tar.gz}"
INSTALL_DIR="${INSTALL_DIR:-$HOME/weather_app}"
POLL_SECONDS="${POLL_SECONDS:-30}"
UPDATE_CONFIG="${UPDATE_CONFIG:-0}"

WORK="${XDG_CACHE_HOME:-$HOME/.cache}/weather_app-updater"
STAMP="$INSTALL_DIR/.installed.sha256"
APP=weather_app

log() { echo "$*"; }   # stdout goes to the journal: journalctl --user -u weather-app-updater

mkdir -p "$WORK"
exec 9>"$WORK/lock"
flock -n 9 || { log "another updater is already running"; exit 1; }

# "mtime size" of the tarball, or empty if it isn't there (NAS offline, etc.)
signature() { stat -c '%Y %s' "$TARBALL" 2>/dev/null || true; }

# Wait until the file stops changing, so a half-synced tarball is never used.
wait_until_stable() {
    local before after
    before=$(signature)
    for _ in 1 2 3 4 5 6; do
        sleep 5
        after=$(signature)
        [[ -n "$after" && "$after" == "$before" ]] && return 0
        before=$after
    done
    return 1
}

app_pids() { pgrep -x "$APP" || true; }

# True if the app has been running for at least $1 seconds.
app_healthy() {
    local min=$1 pid etimes
    pid=$(app_pids | head -n1)
    [[ -n "$pid" ]] || return 1
    etimes=$(ps -o etimes= -p "$pid" | tr -d ' ')
    [[ -n "$etimes" && "$etimes" -ge "$min" ]]
}

restart_app() {
    if [[ -z "$(app_pids)" ]]; then
        log "app not running; it will use the new version at next login"
        return 1
    fi
    pkill -TERM -x "$APP" || true
    return 0
}

install() {
    local new_sha=$1 stage="$WORK/stage" tmp="$WORK/incoming.tar.gz"

    # Work on a local copy so an NFS hiccup mid-read can't corrupt the install.
    cp "$TARBALL" "$tmp"
    if [[ "$(sha256sum "$tmp" | cut -d' ' -f1)" != "$new_sha" ]]; then
        log "tarball changed while copying; will retry"
        return 1
    fi

    rm -rf "$stage" && mkdir -p "$stage"
    if ! tar --warning=no-unknown-keyword -xzf "$tmp" -C "$stage"; then
        log "tarball is corrupt; skipping it"
        echo "$new_sha" > "$WORK/rejected.sha256"
        return 1
    fi
    find "$stage" -name '._*' -delete   # macOS metadata files

    local src="$stage/weather_app"
    # Sanity check: an aarch64 ELF executable (bytes 0-3 = \x7fELF, 18-19 = 0xb7 0x00).
    if [[ ! -f "$src/$APP" ]] \
        || [[ "$(head -c4 "$src/$APP" | od -An -tx1 | tr -d ' \n')" != 7f454c46 ]] \
        || [[ "$(od -An -tx1 -j18 -N2 "$src/$APP" | tr -d ' \n')" != b700 ]]; then
        log "tarball has no aarch64 $APP binary; skipping it"
        echo "$new_sha" > "$WORK/rejected.sha256"
        return 1
    fi

    mkdir -p "$INSTALL_DIR"
    # Keep the current version for rollback.
    [[ -f "$INSTALL_DIR/$APP" ]] && cp -p "$INSTALL_DIR/$APP" "$INSTALL_DIR/$APP.prev"
    rm -rf "$INSTALL_DIR/assets.prev"
    [[ -d "$INSTALL_DIR/assets" ]] && mv "$INSTALL_DIR/assets" "$INSTALL_DIR/assets.prev"

    # rename() is atomic and works while the old binary is running
    # (copying over a running executable fails with "Text file busy").
    chmod +x "$src/$APP"
    mv -f "$src/$APP" "$INSTALL_DIR/$APP"
    mv "$src/assets" "$INSTALL_DIR/assets"
    if [[ "$UPDATE_CONFIG" == 1 || ! -f "$INSTALL_DIR/config.toml" ]]; then
        [[ -f "$src/config.toml" ]] && mv -f "$src/config.toml" "$INSTALL_DIR/config.toml"
    fi
    echo "$new_sha" > "$STAMP"
    log "installed ${new_sha:0:12}"

    restart_app || return 0

    # The wrapper restarts the app ~5 s after it exits. Give the new version
    # 30 s; if it isn't up and stable by then, put the old one back.
    sleep 30
    if app_healthy 15; then
        log "new version running"
        return 0
    fi
    log "new version failed to stay up; rolling back"
    if [[ -f "$INSTALL_DIR/$APP.prev" ]]; then
        mv -f "$INSTALL_DIR/$APP.prev" "$INSTALL_DIR/$APP"
        if [[ -d "$INSTALL_DIR/assets.prev" ]]; then
            rm -rf "$INSTALL_DIR/assets"
            mv "$INSTALL_DIR/assets.prev" "$INSTALL_DIR/assets"
        fi
        rm -f "$STAMP"
        pkill -TERM -x "$APP" || true
    fi
    # Don't retry this tarball until a different one arrives.
    echo "$new_sha" > "$WORK/rejected.sha256"
    return 1
}

log "watching $TARBALL every ${POLL_SECONDS}s (install dir $INSTALL_DIR)"
last_sig=""
while true; do
    sig=$(signature)
    if [[ -n "$sig" && "$sig" != "$last_sig" ]]; then
        if wait_until_stable; then
            last_sig=$(signature)
            new_sha=$(sha256sum "$TARBALL" | cut -d' ' -f1)
            current=$(cat "$STAMP" 2>/dev/null || true)
            rejected=$(cat "$WORK/rejected.sha256" 2>/dev/null || true)
            if [[ "$new_sha" != "$current" && "$new_sha" != "$rejected" ]]; then
                log "new tarball ${new_sha:0:12} found"
                install "$new_sha" || last_sig=""   # retry next poll
            fi
        fi
    fi
    sleep "$POLL_SECONDS"
done
