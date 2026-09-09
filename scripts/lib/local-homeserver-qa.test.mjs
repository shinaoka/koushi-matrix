import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  homeserverFixtureCapabilities,
  selectedServers,
  synapseDockerfile,
  synapseEntrypoint
} from "./local-homeserver-qa.mjs";

function extractedSlidingSyncBoolean(entrypoint) {
  const assignments = [
    ...[
      ...entrypoint.matchAll(
        /^\s*(?:export\s+)?KOUSHI_SYNAPSE_MSC3575_ENABLED=(true|false)\s*$/gm
      )
    ].map((match) => match[1]),
    ...[...entrypoint.matchAll(/^\s*msc3575_enabled:\s*(true|false)$/gm)].map(
      (match) => match[1]
    )
  ];
  assert.equal(assignments.length, 1, "expected exactly one Sliding Sync boolean");
  return assignments[0] === "true";
}

function extractedPythonUpdater(entrypoint) {
  const programs = [
    ...entrypoint.matchAll(/python3 - \/data\/homeserver\.yaml <<'PYTHON'\n([\s\S]*?)\nPYTHON/g)
  ];
  assert.equal(programs.length, 1, "expected exactly one structured YAML updater");
  return programs[0][1];
}

test("Synapse fixture pins matrixdotorg/synapse v1.157.0", () => {
  const dockerfile = synapseDockerfile();
  const baseImages = dockerfile.split("\n").filter((line) => /^\s*FROM\s+/i.test(line));

  assert.equal(baseImages.length, 1);
  assert.equal(baseImages[0].trim(), "FROM docker.io/matrixdotorg/synapse:v1.157.0");
});

test("Synapse fixture raises the separate per-room join limit for population QA", () => {
  assert.match(synapseEntrypoint(), /\nrc_joins_per_room:\n  per_second: 1000\n  burst_count: 1000\n/);
});

test("Synapse positive fixture enables simplified Sliding Sync", () => {
  const entrypoint = synapseEntrypoint();

  assert.equal(extractedSlidingSyncBoolean(entrypoint), true);
});

test("Synapse negative fixture disables simplified Sliding Sync", () => {
  const entrypoint = synapseEntrypoint({ slidingSyncEnabled: false });

  assert.equal(extractedSlidingSyncBoolean(entrypoint), false);
});

test("Synapse reapplies Sliding Sync to persisted config on every startup", () => {
  const runDir = mkdtempSync(join(tmpdir(), "koushi-synapse-config-test-"));
  try {
    const yamlShim = `import json
def safe_load(stream):
    return json.load(stream)
def safe_dump(value, stream, sort_keys=False):
    json.dump(value, stream)
`;
    writeFileSync(join(runDir, "yaml.py"), yamlShim);

    for (const requested of [true, false]) {
      const configPath = join(runDir, `homeserver-${requested}.yaml`);
      writeFileSync(
        configPath,
        JSON.stringify({ experimental_features: { msc3575_enabled: !requested } })
      );
      const entrypoint = synapseEntrypoint({ slidingSyncEnabled: requested });
      const updateStartsAfterCreation = entrypoint.indexOf("python3 - /data/homeserver.yaml");
      assert.ok(updateStartsAfterCreation > entrypoint.indexOf("\nfi\n"));
      assert.ok(entrypoint.indexOf("/start.py run") > updateStartsAfterCreation);

      const update = spawnSync("python3", ["-", configPath], {
        encoding: "utf8",
        env: {
          PATH: process.env.PATH,
          KOUSHI_SYNAPSE_MSC3575_ENABLED: String(requested),
          PYTHONPATH: runDir
        },
        input: extractedPythonUpdater(entrypoint)
      });
      assert.equal(update.status, 0, update.stderr);
      assert.equal(
        JSON.parse(readFileSync(configPath, "utf8")).experimental_features.msc3575_enabled,
        requested
      );
    }
  } finally {
    rmSync(runDir, { recursive: true, force: true });
  }
});

test("Tuwunel fixture intrinsically advertises simplified Sliding Sync", () => {
  const capabilities = homeserverFixtureCapabilities("tuwunel");

  assert.deepEqual(capabilities, {
    simplifiedSlidingSync: {
      unstableFeature: "org.matrix.simplified_msc3575",
      enabled: true,
      configuration: "intrinsic"
    }
  });
});

test("obsolete core backend selector is rejected", () => {
  const result = spawnSync(
    process.execPath,
    [
      "scripts/desktop-headless-local-qa.mjs",
      "--check-core-backend-map",
      "--core-backend=probed"
    ],
    {
      cwd: join(import.meta.dirname, "../.."),
      encoding: "utf8"
    }
  );

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /--core-backend is obsolete/);
});

test("forced backend environment is rejected", () => {
  const result = spawnSync(
    process.execPath,
    ["scripts/desktop-headless-local-qa.mjs", "--list"],
    {
      cwd: join(import.meta.dirname, "../.."),
      encoding: "utf8",
      env: { ...process.env, KOUSHI_QA_FORCE_SYNC_BACKEND: "legacy" }
    }
  );

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /KOUSHI_QA_FORCE_SYNC_BACKEND is obsolete/);
});

test("legacy fallback scenarios are rejected", () => {
  const result = spawnSync(
    process.execPath,
    [
      "scripts/desktop-headless-local-qa.mjs",
      "--run",
      "--server=tuwunel",
      "--core",
      "--scenario=timeline_legacy_fallback"
    ],
    {
      cwd: join(import.meta.dirname, "../.."),
      encoding: "utf8"
    }
  );

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /legacy fallback scenarios are obsolete/);
});

test("supported core QA has one engine-neutral run", () => {
  const runner = readFileSync(join(import.meta.dirname, "../desktop-headless-local-qa.mjs"), "utf8");

  assert.doesNotMatch(runner, /selectedCoreBackendLegs|coreBackendLegConfig|forceBackend|expectSyncBackend/);
  assert.doesNotMatch(runner, /env\.KOUSHI_QA_FORCE_SYNC_BACKEND\s*=/);
  assert.equal((runner.match(/runCoreHeadlessQa\(/g) ?? []).length, 2);
});

test("obsolete core backend values are no longer accepted", () => {
  const result = spawnSync(
    process.execPath,
    [
      "scripts/desktop-headless-local-qa.mjs",
      "--check-core-backend-map",
      "--core-backend=sync-service"
    ],
    {
      cwd: join(import.meta.dirname, "../.."),
      encoding: "utf8"
    }
  );

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /--core-backend is obsolete/);
});

test("server selection keeps individual Sliding Sync fixtures", () => {
  assert.deepEqual(selectedServers("tuwunel"), ["tuwunel"]);
  assert.deepEqual(selectedServers("synapse"), ["synapse"]);
});

test("both selects Tuwunel and Synapse without Conduit", () => {
  assert.deepEqual(selectedServers("both"), ["tuwunel", "synapse"]);
});

test("server selection rejects retired Conduit and compatibility aliases", () => {
  assert.throws(() => selectedServers("conduit"), /--server must be tuwunel, synapse, or both/);
  assert.throws(() => selectedServers("all"), /--server must be tuwunel, synapse, or both/);
  assert.throws(() => selectedServers("matrixorg"), /--server must be tuwunel, synapse, or both/);
});

test("server selection rejects unknown values", () => {
  assert.throws(
    () => selectedServers("unknown"),
    /--server must be tuwunel, synapse, or both/
  );
});
