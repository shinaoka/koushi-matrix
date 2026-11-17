import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import assert from "node:assert/strict";

import { SLOT_PIXELS } from "./check-icon-sizes.mjs";

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
  assert.match(generator, /for size in 32 64 128 256 512 1024/);
  assert.doesNotMatch(
    generator,
    /generate-icns\.py[\s\S]*"\$\{OUT_DIR\}\/32x32\.png"/,
    "the ICNS must not reuse the shared transparent raster"
  );
});

test("icon generation packs every macOS ICNS slot", async () => {
  const generator = await readFile(join(scriptsDir, "generate-koushi-icons.sh"), "utf8");

  // The built file itself is verified by scripts/check-icon-sizes.mjs.
  for (const [code, pixels] of Object.entries(SLOT_PIXELS)) {
    assert.match(
      generator,
      new RegExp(`"${code}=\\$\\{MACOS_ICON_DIR\\}/${pixels}x${pixels}\\.png"`),
      `${code} must be packed from the ${pixels}px render`
    );
  }
});
