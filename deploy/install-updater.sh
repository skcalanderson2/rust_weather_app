#!/usr/bin/env bash
# Install the auto-updater on the Pi (run on the Pi, from this folder).
#   ./install-updater.sh
# Or from the Mac:
#   scp deploy/* sysala@<pi>:/tmp/ && ssh sysala@<pi> 'bash /tmp/install-updater.sh'
set -euo pipefail
cd "$(dirname "$0")"

install -Dm755 weather-app-updater.sh "$HOME/.local/bin/weather-app-updater.sh"
install -Dm644 weather-app-updater.service "$HOME/.config/systemd/user/weather-app-updater.service"

# Record the running version so the first poll doesn't reinstall it.
if [[ -f "$HOME/weather_app/weather_app" && ! -f "$HOME/weather_app/.installed.sha256" ]]; then
    tarball=$(sed -n 's/^Environment=TARBALL=//p' weather-app-updater.service)
    if [[ -f "$tarball" ]]; then
        tmp=$(mktemp -d)
        tar --warning=no-unknown-keyword -xzf "$tarball" -C "$tmp" weather_app/weather_app
        if cmp -s "$tmp/weather_app/weather_app" "$HOME/weather_app/weather_app"; then
            sha256sum "$tarball" | cut -d' ' -f1 > "$HOME/weather_app/.installed.sha256"
        fi
        rm -rf "$tmp"
    fi
fi

systemctl --user daemon-reload
systemctl --user enable --now weather-app-updater.service
# Start the user manager at boot even before the desktop login.
sudo loginctl enable-linger "$USER" 2>/dev/null || true
systemctl --user --no-pager status weather-app-updater.service | head -5
echo "Logs: journalctl --user -u weather-app-updater -f"
