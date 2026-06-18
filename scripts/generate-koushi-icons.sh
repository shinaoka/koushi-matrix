#!/usr/bin/env bash
# Generate raster icon assets from apps/desktop/src-tauri/icons/icon.svg.
# Requires: ImageMagick (convert) and a POSIX shell.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
SRC="${PROJECT_ROOT}/assets/branding/koushi-photon.svg"
OUT_DIR="${PROJECT_ROOT}/apps/desktop/src-tauri/icons"

if ! command -v convert >/dev/null 2>&1; then
  echo "Error: ImageMagick convert is required." >&2
  exit 1
fi

if [[ ! -f "${SRC}" ]]; then
  echo "Error: source SVG not found: ${SRC}" >&2
  exit 1
fi

mkdir -p "${OUT_DIR}"

echo "Generating Koushi icon assets from ${SRC}..."

# Keep a rendered copy of the source SVG in the Tauri icons directory.
cp "${SRC}" "${OUT_DIR}/icon.svg"

# macOS/Linux/Windows PNG sizes used by Tauri bundles.
convert -background none "${SRC}" -resize 32x32   "${OUT_DIR}/32x32.png"
convert -background none "${SRC}" -resize 128x128 "${OUT_DIR}/128x128.png"
convert -background none "${SRC}" -resize 256x256 "${OUT_DIR}/128x128@2x.png"
convert -background none "${SRC}" -resize 512x512 "${OUT_DIR}/icon.png"

# Favicon sizes from the simplified small-mark source.
SMALL_SRC="${PROJECT_ROOT}/assets/branding/koushi-photon-small.svg"
convert -background none "${SMALL_SRC}" -resize 16x16 "${PROJECT_ROOT}/assets/branding/favicon-16.png"
convert -background none "${SMALL_SRC}" -resize 32x32 "${PROJECT_ROOT}/assets/branding/favicon-32.png"

# Windows ICO with multiple embedded sizes (keep the 256x256 source frame).
convert -background none "${SRC}" -resize 256x256 \
  \( -clone 0 -resize 16x16 \) \
  \( -clone 0 -resize 32x32 \) \
  \( -clone 0 -resize 48x48 \) \
  \( -clone 0 -resize 64x64 \) \
  \( -clone 0 -resize 128x128 \) \
  "${OUT_DIR}/icon.ico"

# macOS ICNS: pack PNGs into a simple ICNS container. This script uses a small
# Python helper so the assets remain reproducible without macOS-only tools.
python3 "${SCRIPT_DIR}/lib/generate-icns.py" \
  "${OUT_DIR}/icon.icns" \
  "${OUT_DIR}/32x32.png" \
  "${OUT_DIR}/128x128.png" \
  "${OUT_DIR}/128x128@2x.png" \
  "${OUT_DIR}/icon.png"

echo "Done. Assets in ${OUT_DIR}:"
ls -la "${OUT_DIR}"
