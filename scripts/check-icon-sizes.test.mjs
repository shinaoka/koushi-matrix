import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import assert from "node:assert/strict";
import { deflateSync } from "node:zlib";

import { SLOT_PIXELS, checkIcns, readEntries } from "./check-icon-sizes.mjs";

const projectRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const icnsPath = join(projectRoot, "apps/desktop/src-tauri/icons/icon.icns");

/** Rebuild a container from mutated entries so each test changes exactly one thing. */
function mutated(mutate) {
  const entries = readEntries(readFileSync(icnsPath)).entries.map((entry) => ({ ...entry }));
  mutate(entries);
  const chunks = entries.map(({ code, payload }) => {
    const header = Buffer.alloc(8);
    header.write(code, 0, "ascii");
    header.writeUInt32BE(8 + payload.length, 4);
    return Buffer.concat([header, payload]);
  });
  const header = Buffer.alloc(8);
  header.write("icns", 0, "ascii");
  header.writeUInt32BE(8 + chunks.reduce((total, chunk) => total + chunk.length, 0), 4);
  return Buffer.concat([header, ...chunks]);
}

function encodePng(size, pixelAt) {
  const stride = size * 3;
  const raw = Buffer.alloc((stride + 1) * size);
  for (let y = 0; y < size; y += 1) {
    for (let x = 0; x < size; x += 1) {
      const index = y * (stride + 1) + 1 + x * 3;
      raw.set(pixelAt(x, y), index);
    }
  }
  const chunk = (type, data) => {
    const length = Buffer.alloc(4);
    length.writeUInt32BE(data.length);
    // The decoder never reads chunk CRCs, so the fixture writes none.
    return Buffer.concat([
      length,
      Buffer.from(type, "ascii"),
      data,
      Buffer.alloc(4),
    ]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 2; // truecolor
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

function noisePixel(size) {
  let state = 0x2f6b1a3d;
  const values = [];
  for (let i = 0; i < size * size * 3; i += 1) {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    values.push((state >>> 0) & 0xff);
  }
  return (x, y) => values.slice((y * size + x) * 3, (y * size + x) * 3 + 3);
}

test("the checked-in icon.icns has no problems", () => {
  assert.deepEqual(checkIcns(readFileSync(icnsPath)), []);
});

test("legacy icp4/icp5/icp6 slots are rejected", () => {
  const problems = checkIcns(
    mutated((entries) => {
      for (const entry of entries) {
        if (entry.code === "ic11") entry.code = "icp5";
        if (entry.code === "ic12") entry.code = "icp6";
      }
    })
  );
  assert.ok(
    problems.some((problem) => problem.includes("icp5") && problem.includes("#1942")),
    problems.join("\n")
  );
  assert.ok(problems.some((problem) => problem.includes("missing ic11")));
});

test("a slot carrying the wrong pixel size is rejected", () => {
  const problems = checkIcns(
    mutated((entries) => {
      const source = entries.find((entry) => entry.code === "ic07");
      entries.find((entry) => entry.code === "ic10").payload = source.payload;
    })
  );
  assert.ok(
    problems.some((problem) => problem === "ic10 must carry a 1024x1024 image, got 128x128"),
    problems.join("\n")
  );
});

test("a noise payload is rejected even when its type code is right", () => {
  const problems = checkIcns(
    mutated((entries) => {
      entries.find((entry) => entry.code === "ic11").payload = encodePng(
        SLOT_PIXELS.ic11,
        noisePixel(SLOT_PIXELS.ic11)
      );
    })
  );
  assert.ok(
    problems.some((problem) => problem.startsWith("ic11") && problem.includes("noise")),
    problems.join("\n")
  );
});

test("a flat payload is rejected", () => {
  const problems = checkIcns(
    mutated((entries) => {
      entries.find((entry) => entry.code === "ic12").payload = encodePng(
        SLOT_PIXELS.ic12,
        () => [0x13, 0x23, 0x3f]
      );
    })
  );
  assert.ok(
    problems.some(
      (problem) => problem.startsWith("ic12") && problem.includes("white photon")
    ),
    problems.join("\n")
  );
});

test("a missing slot is rejected", () => {
  const problems = checkIcns(
    mutated((entries) => {
      entries.splice(
        entries.findIndex((entry) => entry.code === "ic14"),
        1
      );
    })
  );
  assert.ok(problems.includes("missing ic14 (512px)"), problems.join("\n"));
});

test("a mangled container is a reported problem, not a hang", () => {
  const healthy = readFileSync(icnsPath);
  const zeroLength = Buffer.from(healthy);
  zeroLength.writeUInt32BE(0, 12); // first entry's length field
  assert.deepEqual(checkIcns(zeroLength), [
    "container is unreadable: ic11 at byte 8 declares 0 bytes",
  ]);

  const truncated = healthy.subarray(0, healthy.length - 100);
  assert.ok(
    checkIcns(truncated).some((problem) => problem.startsWith("container is unreadable")),
    checkIcns(truncated).join("\n")
  );
});
