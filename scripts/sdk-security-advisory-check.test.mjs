#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const here = dirname(fileURLToPath(import.meta.url));
const script = resolve(here, "sdk-security-advisory-check.mjs");
// Published repository advisories of matrix-org/matrix-rust-sdk,
// matrix-org/vodozemac and ruma/ruma, as the GitHub API returned them.
const realGithubAdvisories = resolve(here, "fixtures/sdk-security/github-advisories.json");

// The SDK-family versions the root Cargo.lock pins today.
const pinnedToday = {
  "matrix-sdk": "0.19.1",
  "matrix-sdk-base": "0.19.1",
  "matrix-sdk-crypto": "0.19.1",
  "matrix-sdk-ui": "0.19.1",
  ruma: "0.17.2",
  "ruma-common": "0.20.2",
  vodozemac: "0.11.1",
  "quinn-proto": "0.11.15",
};

const registry = "registry+https://github.com/rust-lang/crates.io-index";

/** A package value is a version string, or `{ version, source }` to pin the source line. */
function lockfile(packages) {
  return (
    "version = 4\n" +
    Object.entries(packages)
      .map(([name, entry]) => {
        const { version, source } =
          typeof entry === "string"
            ? { version: entry, source: name.startsWith("matrix-sdk") ? null : registry }
            : entry;
        return (
          `\n[[package]]\nname = "${name}"\nversion = "${version}"\n` +
          (source ? `source = "${source}"\n` : "")
        );
      })
      .join("")
  );
}

function githubAdvisory({ ghsa, name, vulnerable = "< 9.0.0", patched = "9.0.0" }) {
  return {
    ghsa_id: ghsa,
    cve_id: null,
    summary: `Fixture advisory ${ghsa}`,
    severity: "high",
    html_url: `https://github.com/advisories/${ghsa}`,
    withdrawn_at: null,
    vulnerabilities: [
      {
        package: { ecosystem: "rust", name },
        vulnerable_version_range: vulnerable,
        patched_versions: patched,
      },
    ],
  };
}

function auditVulnerability({ id, aliases = [], name, version, patched }) {
  return {
    advisory: {
      id,
      package: name,
      title: `Fixture advisory ${id}`,
      aliases,
      url: null,
      withdrawn: null,
      informational: null,
    },
    versions: { patched, unaffected: [] },
    package: { name, version },
  };
}

function run({
  packages = pinnedToday,
  githubAdvisories = null,
  auditList = [],
  issues = [],
}) {
  const root = mkdtempSync(join(tmpdir(), "koushi-sdk-security-"));
  const paths = {
    lockfile: join(root, "Cargo.lock"),
    audit: join(root, "audit.json"),
    github: join(root, "github-advisories.json"),
    issues: join(root, "issues.json"),
  };
  writeFileSync(paths.lockfile, lockfile(packages));
  writeFileSync(
    paths.audit,
    JSON.stringify({
      vulnerabilities: { found: auditList.length > 0, count: auditList.length, list: auditList },
      warnings: {},
    })
  );
  if (githubAdvisories) {
    writeFileSync(paths.github, JSON.stringify(githubAdvisories));
  }
  writeFileSync(paths.issues, JSON.stringify(issues));
  return spawnSync(
    process.execPath,
    [
      script,
      "--lockfile",
      paths.lockfile,
      "--audit-json",
      paths.audit,
      "--github-advisories-json",
      githubAdvisories ? paths.github : realGithubAdvisories,
      "--issues-json",
      paths.issues,
      "--dry-run",
    ],
    { encoding: "utf8" }
  );
}

test("unaffected: today's pinned versions match no published advisory", () => {
  const result = run({});
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^affected_advisories=0$/m);
  assert.match(result.stdout, /^issue_action=none$/m);
});

test("affected: an older matrix-sdk-crypto opens one issue for GHSA-45pr-7vv7-f64m", () => {
  const result = run({ packages: { ...pinnedToday, "matrix-sdk-crypto": "0.18.0" } });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^affected_advisories=1$/m);
  assert.match(
    result.stdout,
    /^issue_action=create SDK security advisory GHSA-45pr-7vv7-f64m \(matrix-sdk-crypto 0\.18\.0\)$/m
  );
  assert.match(result.stdout, /vulnerable `< 0\.19\.0`, patched `0\.19\.0`/);
});

test("affected: real ranges with two-part versions, exact lists and padded names", () => {
  const result = run({
    packages: {
      ...pinnedToday,
      "matrix-sdk": "0.12.0",
      "matrix-sdk-crypto": "0.9.0",
      vodozemac: "0.5.1",
    },
  });
  assert.equal(result.status, 0, result.stderr);
  // matrix-sdk `>= 0.11, < 0.13`
  assert.match(result.stdout, /^issue_action=create .*GHSA-275g-g844-73jh/m);
  // " matrix-sdk-crypto" `>= 0.8.0, < 0.11.1`
  assert.match(result.stdout, /^issue_action=create .*GHSA-x958-rvg6-956w/m);
  // vodozemac `0.5.0, 0.5.1`
  assert.match(result.stdout, /^issue_action=create .*GHSA-c3hm-hxwf-g5c6/m);
  // vodozemac `< 0.7.0`
  assert.match(result.stdout, /^issue_action=create .*GHSA-j8cm-g7r6-hfpq/m);
  // matrix-sdk-crypto `0.7.0` is an exact version, not a lower bound.
  assert.doesNotMatch(result.stdout, /GHSA-9ggc-845v-gcgv/);
});

test("a pre-release pin sorts below its release", () => {
  const result = run({
    packages: { ...pinnedToday, "matrix-sdk-crypto": "0.19.0-alpha.1" },
  });
  assert.equal(result.status, 0, result.stderr);
  // `< 0.19.0` covers 0.19.0-alpha.1.
  assert.match(result.stdout, /^issue_action=create .*GHSA-45pr-7vv7-f64m/m);
});

test("cargo audit findings for SDK crates file issues; other crates are left to cargo-deny", () => {
  const result = run({
    auditList: [
      auditVulnerability({
        id: "RUSTSEC-2099-0001",
        aliases: ["GHSA-aaaa-bbbb-cccc"],
        name: "ruma-common",
        version: "0.20.2",
        patched: [">=0.20.3"],
      }),
      auditVulnerability({
        id: "RUSTSEC-2026-0341",
        name: "quinn-proto",
        version: "0.11.15",
        patched: [">=0.11.17"],
      }),
    ],
  });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^affected_advisories=1$/m);
  assert.match(
    result.stdout,
    /^issue_action=create SDK security advisory RUSTSEC-2099-0001 \/ GHSA-aaaa-bbbb-cccc \(ruma-common 0\.20\.2\)$/m
  );
  assert.doesNotMatch(result.stdout, /quinn-proto/);
});

test("one issue when GitHub and RustSec report the same advisory", () => {
  const result = run({
    packages: { ...pinnedToday, "matrix-sdk-crypto": "0.18.0" },
    auditList: [
      auditVulnerability({
        id: "RUSTSEC-2026-0400",
        aliases: ["GHSA-45pr-7vv7-f64m"],
        name: "matrix-sdk-crypto",
        version: "0.18.0",
        patched: [">=0.19.0"],
      }),
    ],
  });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^affected_advisories=1$/m);
  assert.equal(result.stdout.match(/^issue_action=create/gm).length, 1);
  assert.match(result.stdout, /GHSA-45pr-7vv7-f64m \/ RUSTSEC-2026-0400/);
});

test("existing issue: an open or closed issue naming any alias suppresses a new one", () => {
  const packages = { ...pinnedToday, "matrix-sdk-crypto": "0.18.0" };
  for (const state of ["OPEN", "CLOSED"]) {
    const result = run({
      packages,
      issues: [
        {
          number: 1800,
          title: "SDK security advisory GHSA-45pr-7vv7-f64m (matrix-sdk-crypto 0.18.0)",
          state,
          body: "",
        },
      ],
    });
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /^affected_advisories=1$/m);
    assert.match(result.stdout, /^issue_action=none$/m);
  }
});

test("withdrawn advisories and other ecosystems are ignored", () => {
  const advisory = {
    ghsa_id: "GHSA-zzzz-zzzz-zzzz",
    cve_id: null,
    summary: "Withdrawn fixture",
    severity: "high",
    html_url: "https://github.com/advisories/GHSA-zzzz-zzzz-zzzz",
    withdrawn_at: "2026-01-01T00:00:00Z",
    vulnerabilities: [
      {
        package: { ecosystem: "rust", name: "matrix-sdk" },
        vulnerable_version_range: "< 1.0.0",
        patched_versions: "1.0.0",
      },
    ],
  };
  const otherEcosystem = {
    ...advisory,
    ghsa_id: "GHSA-yyyy-yyyy-yyyy",
    withdrawn_at: null,
    vulnerabilities: [{ ...advisory.vulnerabilities[0], package: { ecosystem: "npm", name: "matrix-sdk" } }],
  };
  const result = run({ githubAdvisories: [advisory, otherEcosystem] });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^affected_advisories=0$/m);
});

test("a crates.io advisory asks for a lock bump and names the source", () => {
  const result = run({
    packages: { ...pinnedToday, vodozemac: { version: "0.11.1", source: registry } },
    githubAdvisories: [
      githubAdvisory({ ghsa: "GHSA-regi-stry-0001", name: "vodozemac", vulnerable: "< 0.11.2", patched: "0.11.2" }),
    ],
  });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^issue_action=create .*GHSA-regi-stry-0001/m);
  assert.match(result.stdout, /comes from crates.io \(`registry\+https:\/\/github\.com\/rust-lang\/crates\.io-index`\)/);
  assert.match(result.stdout, /`cargo update -p vodozemac --precise 0\.11\.2`/);
  assert.match(result.stdout, /The fork and its submodule are not involved/);
  assert.doesNotMatch(result.stdout, /gitlink/);
});

test("a vendored fork advisory asks for the upstream merge and the gitlink move", () => {
  const result = run({ packages: { ...pinnedToday, "matrix-sdk-crypto": "0.18.0" } });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^issue_action=create .*GHSA-45pr-7vv7-f64m/m);
  assert.match(result.stdout, /The pinned fork can trail unreleased upstream fixes/);
  assert.match(result.stdout, /move the gitlink as described in/);
  assert.doesNotMatch(result.stdout, /cargo update/);
});

test("an unrecognized lockfile source states both routes instead of guessing one", () => {
  const result = run({
    packages: {
      ...pinnedToday,
      vodozemac: { version: "0.11.1", source: "sparse+https://example.invalid/index/" },
    },
    githubAdvisories: [
      githubAdvisory({ ghsa: "GHSA-unkn-own0-0001", name: "vodozemac", vulnerable: "< 0.11.2", patched: "0.11.2" }),
    ],
  });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /`cargo update -p vodozemac --precise 0\.11\.2` if it comes from/);
  assert.match(result.stdout, /merge the fix \(or upstream `main`\) into the fork and/);
  assert.match(result.stdout, /move the gitlink as described in/);
});

test("a git revision dependency also asks for its pinned revision to move", () => {
  const result = run({
    packages: {
      ...pinnedToday,
      ruma: { version: "0.17.2", source: `git+https://github.com/ruma/ruma?rev=${"0".repeat(40)}` },
    },
    githubAdvisories: [
      githubAdvisory({ ghsa: "GHSA-git0-rev0-0001", name: "ruma", vulnerable: "< 0.18.0", patched: "0.18.0" }),
    ],
  });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /The pinned fork can trail unreleased upstream fixes/);
  assert.match(result.stdout, /is a pinned revision \(`git\+https:\/\/github\.com\/ruma\/ruma\?rev=0+`\)/);
  assert.match(result.stdout, /source `git\+https:\/\/github\.com\/ruma\/ruma\?rev=0+`/);
});

test("a strict lower bound never becomes a --precise version", () => {
  const result = run({
    packages: { ...pinnedToday, vodozemac: { version: "0.11.1", source: registry } },
    auditList: [
      auditVulnerability({
        id: "RUSTSEC-2099-0002",
        name: "vodozemac",
        version: "0.11.1",
        patched: [">0.11.1"],
      }),
    ],
  });
  assert.equal(result.status, 0, result.stderr);
  assert.doesNotMatch(result.stdout, /--precise 0\.11\.1/);
  assert.match(result.stdout, /`cargo update -p vodozemac` to the first release outside the vulnerable range/);
});

test("an unparseable range fails the run after filing the advisories it could evaluate", () => {
  const base = {
    cve_id: null,
    summary: "Fixture",
    severity: "high",
    html_url: "https://github.com/advisories/GHSA-fixture",
    withdrawn_at: null,
  };
  const result = run({
    githubAdvisories: [
      {
        ...base,
        ghsa_id: "GHSA-bad0-bad0-bad0",
        vulnerabilities: [
          {
            package: { ecosystem: "rust", name: "matrix-sdk" },
            vulnerable_version_range: "~> 0.19",
            patched_versions: null,
          },
        ],
      },
      {
        ...base,
        ghsa_id: "GHSA-good-good-good",
        vulnerabilities: [
          {
            package: { ecosystem: "rust", name: "matrix-sdk" },
            vulnerable_version_range: "<= 0.19.1",
            patched_versions: "0.19.2",
          },
        ],
      },
    ],
  });
  assert.equal(result.status, 1);
  assert.match(result.stdout, /^issue_action=create .*GHSA-good-good-good/m);
  assert.match(result.stderr, /GHSA-bad0-bad0-bad0.*~> 0\.19/);
});
