import assert from "node:assert/strict";
import childProcess, { spawnSync } from "node:child_process";
import { syncBuiltinESMExports } from "node:module";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  homeserverFixtureCapabilities,
  selectedServers,
  synapseDockerfile,
  synapseEntrypoint,
  waitForHomeserver
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

function mockReadiness(t, probe) {
  t.mock.method(globalThis, "fetch", async () => ({ ok: true }));
  t.mock.method(childProcess, "spawnSync", probe);
  syncBuiltinESMExports();
  t.after(() => {
    t.mock.restoreAll();
    syncBuiltinESMExports();
  });
}

test("Synapse readiness waits for its native membership-table migrations", async (t) => {
  const runPython = spawnSync;
  let probes = 0;
  let program;
  mockReadiness(t, (command, args, options) => {
    assert.equal(command, "docker");
    assert.deepEqual(args.slice(0, 4), ["exec", "synthetic-owned", "python3", "-c"]);
    assert.ok(options.timeout > 0 && options.timeout <= 2000);
    program = args[4];
    return { status: ++probes === 1 ? 2 : 0 };
  });
  await waitForHomeserver("http://example.invalid", {
    exitCode: null, koushiDockerContainerName: "synthetic-owned"
  }, 2000, "synthetic.log");
  assert.equal(probes, 2, "versions alone must not declare native Sliding Sync ready");

  const runDir = mkdtempSync(join(tmpdir(), "koushi-synapse-readiness-test-"));
  try {
    const database = join(runDir, "homeserver.db");
    const setup = runPython("python3", ["-c", `import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
c.execute("CREATE TABLE background_updates (update_name TEXT)")
c.commit()
`, database]);
    assert.equal(setup.status, 0);
    assert.match(program, /mode=ro/);
    const probe = program.replace("/data/homeserver.db", database);
    for (const name of [
      "sliding_sync_prefill_joined_rooms_to_recalculate_table_bg_update",
      "sliding_sync_joined_rooms_bg_update",
      "sliding_sync_membership_snapshots_bg_update"
    ]) {
      const insert = runPython("python3", ["-c", `import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
c.execute("INSERT INTO background_updates VALUES (?)", (sys.argv[2],))
c.commit()
`, database, name]);
      assert.equal(insert.status, 0);
      assert.equal(runPython("python3", ["-c", probe]).status, 2);
      const remove = runPython("python3", ["-c", `import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
c.execute("DELETE FROM background_updates")
c.commit()
`, database]);
      assert.equal(remove.status, 0);
    }
    const unrelated = runPython("python3", ["-c", `import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
c.execute("INSERT INTO background_updates VALUES ('synthetic_unrelated_job')")
c.commit()
`, database]);
    assert.equal(unrelated.status, 0);
    assert.equal(runPython("python3", ["-c", probe]).status, 0);
  } finally {
    rmSync(runDir, { recursive: true, force: true });
  }
});

test("Synapse readiness probe errors fail without exposing child output", async (t) => {
  mockReadiness(t, () => ({ status: 1, stderr: "synthetic-private-output" }));
  await assert.rejects(waitForHomeserver("http://example.invalid", {
    exitCode: null, koushiDockerContainerName: "synthetic-owned"
  }, 1000, "synthetic.log"), { message: "Synapse Sliding Sync readiness probe failed" });
});

test("Synapse pending readiness expires on one monotonic deadline", { timeout: 1000 }, async (t) => {
  let probes = 0;
  let now = 0;
  mockReadiness(t, () => { probes++; now = 20; return { status: 2 }; });
  t.mock.method(Date, "now", () => 0);
  t.mock.method(performance, "now", () => now);
  await assert.rejects(waitForHomeserver("http://example.invalid", {
    exitCode: null, koushiDockerContainerName: "synthetic-owned"
  }, 20, "synthetic.log"), /Synapse Sliding Sync table migrations pending/);
  assert.equal(probes, 1);
});

test("Synapse readiness rejects a probe that finishes beyond its deadline", async (t) => {
  let now = 0;
  mockReadiness(t, () => { now = 20; return { status: 0 }; });
  t.mock.method(performance, "now", () => now);
  await assert.rejects(waitForHomeserver("http://example.invalid", {
    exitCode: null, koushiDockerContainerName: "synthetic-owned"
  }, 20, "synthetic.log"), /Synapse Sliding Sync readiness deadline expired/);
});

test("Tuwunel readiness never invokes the Synapse probe", async (t) => {
  mockReadiness(t, () => assert.fail("unexpected container probe"));
  await waitForHomeserver("http://example.invalid", { exitCode: null }, 1000, "synthetic.log");
});

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
    },
    multiReaderReceipts: {
      supported: false,
      limitation: "sss_receipt_packing_single_reader"
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
