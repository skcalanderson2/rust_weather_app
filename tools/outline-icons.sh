#!/usr/bin/env bash
# Regenerate the dark-outlined weather icons used on bright skies, where the
# light icons (yellow sun, white clouds) vanish:
#   assets/icons/outlined/       2 px outline (small icons: forecast box, details)
#   assets/icons/outlined-fine/  1 px outline (the big current-conditions icon)
# Radii are px at the icons' native 128 px. Needs ImageMagick 7
# (`brew install imagemagick`). Output is committed, so the app and the Pi
# don't need ImageMagick.
#
#   ./tools/outline-icons.sh [small_radius] [big_radius]
set -euo pipefail
cd "$(dirname "$0")/../assets/icons"
OUTLINE='#0B0F1A'   # matches sky::DARK_TEXT

outline() {   # outline <dir> <radius>
    mkdir -p "$1"
    for f in *.png; do
        magick "$f" \
            \( +clone -alpha extract -threshold 8% -morphology Dilate "Disk:$2" -blur 0x0.7 \
               -background "$OUTLINE" -alpha shape \) \
            +swap -compose over -composite -strip -define png:exclude-chunk=date,time "$1/$f"
    done
    echo "$1: $(ls "$1" | wc -l | tr -d ' ') icons, radius $2"
}

outline outlined "${1:-2}"
outline outlined-fine "${2:-1}"
