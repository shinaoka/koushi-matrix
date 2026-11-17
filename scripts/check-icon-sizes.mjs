#!/usr/bin/env node
// Guard for the macOS bundle icon (apps/desktop/src-tauri/icons/icon.icns, #1942).
//
// An ICNS container is a header plus typed entries, and macOS picks the decoder
// from the entry's type code. Packing a PNG payload under the legacy
// `icp4`/`icp5`/`icp6` codes produced colour noise wherever macOS drew a small
// icon (System Settings > Notifications), even though the payload was a correct
// render and `sips` downscaled the whole file fine. Type codes alone would not
// catch a payload that is noise, so every entry is also decoded and compared
// with the brand look: full-bleed navy, a near-white photon at the centre.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { inflateSync } from "node:zlib";

const projectRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const ICNS_PATH = join(projectRoot, "apps/desktop/src-tauri/icons/icon.icns");

// Type code -> PNG pixel width. Mirrors scripts/lib/generate-icns.py.
export const SLOT_PIXELS = {
  ic11: 32, // 16x16 @2x
  ic12: 64, // 32x32 @2x
  ic07: 128, // 128x128 @1x
  ic08: 256, // 256x256 @1x
  ic13: 256, // 128x128 @2x
  ic14: 512, // 256x256 @2x
  ic09: 512, // 512x512 @1x
  ic10: 1024, // 512x512 @2x
};

const PNG_SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
const BRAND_NAVY = [0x13, 0x23, 0x3f];
const CORNER_TOLERANCE = 14;
// The measured navy share is 0.645 at 32px and 0.84 at 1024px, so 0.5 leaves
// room for a future source mark while still failing on noise.
const MIN_NAVY_SHARE = 0.5;

/** Decode an 8-bit, non-interlaced PNG into `{width, height, rows}` of RGB triples. */
export function decodePng(buffer) {
  if (!buffer.subarray(0, 8).equals(PNG_SIGNATURE)) {
    throw new Error("not a PNG file");
  }
  let offset = 8;
  let width = 0;
  let height = 0;
  let channels = 0;
  let colorType = 0;
  let palette = [];
  const idat = [];
  while (offset < buffer.length) {
    const length = buffer.readUInt32BE(offset);
    const type = buffer.subarray(offset + 4, offset + 8).toString("ascii");
    const data = buffer.subarray(offset + 8, offset + 8 + length);
    if (type === "IHDR") {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      const depth = data[8];
      colorType = data[9];
      const interlace = data[12];
      if (depth !== 8) {
        throw new Error(
          `payload is ${depth}-bit; these assets are 8-bit (pass -depth 8 to ImageMagick)`
        );
      }
      if (interlace !== 0) throw new Error("interlaced PNG is not supported");
      // RGB, palette, RGBA. ImageMagick emits the first two for these renders.
      channels = { 2: 3, 3: 1, 6: 4 }[colorType];
      if (!channels) throw new Error(`unsupported color type ${colorType}`);
    } else if (type === "PLTE") {
      // ImageMagick writes palette PNGs when a render uses few colours.
      for (let i = 0; i < data.length; i += 3) {
        palette.push([data[i], data[i + 1], data[i + 2]]);
      }
    } else if (type === "IDAT") {
      idat.push(data);
    } else if (type === "IEND") {
      break;
    }
    offset += 12 + length;
  }
  if (!width || !height) throw new Error("missing IHDR");
  if (colorType === 3 && palette.length === 0) throw new Error("palette PNG without PLTE");

  const raw = inflateSync(Buffer.concat(idat));
  const stride = width * channels;
  const rows = [];
  let previous = Buffer.alloc(stride);
  for (let y = 0; y < height; y += 1) {
    const filter = raw[y * (stride + 1)];
    const line = Buffer.from(raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1)));
    unfilterRow(filter, line, previous, channels);
    rows.push(toRgbRow(line, width, colorType, palette));
    previous = line;
  }
  return { width, height, rows };
}

function toRgbRow(line, width, colorType, palette) {
  const step = { 2: 3, 3: 1, 6: 4 }[colorType];
  const rgb = Buffer.alloc(width * 3);
  for (let x = 0; x < width; x += 1) {
    const index = x * step;
    const pixel =
      colorType === 3
        ? (palette[line[index]] ?? [0, 0, 0])
        : [line[index], line[index + 1], line[index + 2]];
    rgb.set(pixel, x * 3);
  }
  return rgb;
}

function unfilterRow(filter, line, previous, channels) {
  for (let i = 0; i < line.length; i += 1) {
    const left = i >= channels ? line[i - channels] : 0;
    const up = previous[i];
    const upLeft = i >= channels ? previous[i - channels] : 0;
    switch (filter) {
      case 0:
        break;
      case 1:
        line[i] = (line[i] + left) & 0xff;
        break;
      case 2:
        line[i] = (line[i] + up) & 0xff;
        break;
      case 3:
        line[i] = (line[i] + ((left + up) >> 1)) & 0xff;
        break;
      case 4:
        line[i] = (line[i] + paeth(left, up, upLeft)) & 0xff;
        break;
      default:
        throw new Error(`unknown PNG filter ${filter}`);
    }
  }
}

function paeth(left, up, upLeft) {
  const estimate = left + up - upLeft;
  const toLeft = Math.abs(estimate - left);
  const toUp = Math.abs(estimate - up);
  const toUpLeft = Math.abs(estimate - upLeft);
  if (toLeft <= toUp && toLeft <= toUpLeft) return left;
  return toUp <= toUpLeft ? up : upLeft;
}

function pixelAt(png, x, y) {
  const row = png.rows[y];
  const index = x * 3;
  return [row[index], row[index + 1], row[index + 2]];
}

function isNear(pixel, expected, tolerance) {
  return pixel.every((value, index) => Math.abs(value - expected[index]) <= tolerance);
}

/** Read the typed entries of an ICNS container. */
export function readEntries(buffer) {
  if (buffer.subarray(0, 4).toString("ascii") !== "icns") {
    throw new Error("missing icns magic");
  }
  const declared = buffer.readUInt32BE(4);
  const entries = [];
  let offset = 8;
  while (offset < buffer.length) {
    if (offset + 8 > buffer.length) {
      throw new Error(`truncated entry header at byte ${offset}`);
    }
    const code = buffer.subarray(offset, offset + 4).toString("ascii");
    const length = buffer.readUInt32BE(offset + 4);
    // A zero or oversized length used to spin here forever on a mangled file.
    if (length < 8 || offset + length > buffer.length) {
      throw new Error(`${code} at byte ${offset} declares ${length} bytes`);
    }
    entries.push({ code, length, payload: buffer.subarray(offset + 8, offset + length) });
    offset += length;
  }
  return { declared, entries };
}

/** Return every problem found in an ICNS container; an empty list means healthy. */
export function checkIcns(buffer) {
  const problems = [];
  let declared;
  let entries;
  try {
    ({ declared, entries } = readEntries(buffer));
  } catch (error) {
    return [`container is unreadable: ${error.message}`];
  }
  if (declared !== buffer.length) {
    problems.push(`header declares ${declared} bytes, file is ${buffer.length}`);
  }

  const seen = new Set();
  for (const { code, length, payload } of entries) {
    if (seen.has(code)) problems.push(`${code} appears twice`);
    seen.add(code);
    const expectedPixels = SLOT_PIXELS[code];
    if (!expectedPixels) {
      problems.push(
        `${code} is not a slot this icon uses; macOS decodes icp4/icp5/icp6 as noise (#1942)`
      );
      continue;
    }
    if (length !== 8 + payload.length) {
      problems.push(`${code} declares ${length} bytes but carries ${payload.length}`);
      continue;
    }
    let png;
    try {
      png = decodePng(payload);
    } catch (error) {
      problems.push(`${code} payload is not a usable PNG: ${error.message}`);
      continue;
    }
    if (png.width !== expectedPixels || png.height !== expectedPixels) {
      problems.push(
        `${code} must carry a ${expectedPixels}x${expectedPixels} image, got ${png.width}x${png.height}`
      );
      continue;
    }

    const last = expectedPixels - 1;
    const corners = [
      pixelAt(png, 0, 0),
      pixelAt(png, last, 0),
      pixelAt(png, 0, last),
      pixelAt(png, last, last),
    ];
    if (!corners.every((pixel) => isNear(pixel, BRAND_NAVY, CORNER_TOLERANCE))) {
      problems.push(`${code} is not full-bleed navy at the corners`);
    }
    const centre = pixelAt(png, expectedPixels >> 1, expectedPixels >> 1);
    if (!centre.every((value) => value >= 200)) {
      problems.push(`${code} has no white photon at the centre (got ${centre.join(",")})`);
    }
    let navy = 0;
    for (let y = 0; y < expectedPixels; y += 1) {
      for (let x = 0; x < expectedPixels; x += 1) {
        if (isNear(pixelAt(png, x, y), BRAND_NAVY, CORNER_TOLERANCE)) navy += 1;
      }
    }
    const share = navy / (expectedPixels * expectedPixels);
    if (share < MIN_NAVY_SHARE) {
      problems.push(`${code} is only ${share.toFixed(3)} navy; looks like noise`);
    }
  }

  for (const code of Object.keys(SLOT_PIXELS)) {
    if (!seen.has(code)) problems.push(`missing ${code} (${SLOT_PIXELS[code]}px)`);
  }
  return problems;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const problems = checkIcns(readFileSync(ICNS_PATH));
  if (problems.length > 0) {
    for (const problem of problems) console.error(`icon.icns: ${problem}`);
    process.exit(1);
  }
  console.log(`icon.icns: ${Object.keys(SLOT_PIXELS).length} slots healthy`);
}
