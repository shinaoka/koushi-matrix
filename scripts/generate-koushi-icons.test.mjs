import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import assert from "node:assert/strict";

const scriptsDir = dirname(fileURLToPath(import.meta.url));
const projectRoot = dirname(scriptsDir);

test("macOS icon source is full-bleed and opaque", async () => {
  const source = await readFile(
    join(projectRoot, "assets/branding/koushi-photon-macos.svg"),
    "utf8"
  );

  assert.match(
    source,
    /<rect\s+width="128"\s+height="128"\s+fill="#13233F"\s*\/>/,
    "the macOS source must paint the whole canvas"
  );
  assert.doesNotMatch(
    source,
    /fill-opacity|opacity="0"|fill="none"/,
    "the macOS background must not introduce transparent corners"
  );
});

test("icon generation keeps shared rasters separate from the macOS ICNS source", async () => {
  const generator = await readFile(join(scriptsDir, "generate-koushi-icons.sh"), "utf8");

  assert.match(generator, /MACOS_SRC=.*koushi-photon-macos\.svg/);
  assert.match(generator, /convert[\s\S]*"\$\{SRC\}"[\s\S]*icon\.png/);
  assert.match(generator, /for size in 16 32 64 128 256 512 1024/);
  assert.match(generator, /generate-icns\.py[\s\S]*"\$\{MACOS_ICON_DIR\}\/1024x1024\.png"/);
  assert.doesNotMatch(
    generator,
    /generate-icns\.py[\s\S]*"\$\{OUT_DIR\}\/32x32\.png"/,
    "the ICNS must not reuse the shared transparent raster"
  );
});

test("macOS ICNS contains the complete modern icon family", async () => {
  const icns = await readFile(
    join(projectRoot, "apps/desktop/src-tauri/icons/icon.icns")
  );
  assert.equal(icns.subarray(0, 4).toString("ascii"), "icns");
  assert.equal(icns.readUInt32BE(4), icns.length);

  const entryTypes = new Set();
  for (let offset = 8; offset < icns.length; ) {
    const type = icns.subarray(offset, offset + 4).toString("ascii");
    const length = icns.readUInt32BE(offset + 4);
    assert.ok(length > 8, `invalid ${type} ICNS entry length`);
    entryTypes.add(type);
    offset += length;
  }

  assert.deepEqual(
    entryTypes,
    new Set(["icp4", "icp5", "icp6", "ic07", "ic13", "ic09", "ic10"])
  );
});
