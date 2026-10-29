#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";

import { applyIssueActions, listIssues } from "./lib/github-issues.mjs";

// Opens one `security` issue per advisory that affects an SDK crate pinned in
// the root Cargo.lock. Sources: the published repository advisories of the
// SDK's upstream repositories, and RustSec through `cargo audit`. The scope is
// the SDK family only: `cargo audit` reads the whole lockfile, including crates
// that no enabled feature builds, while `cargo deny check` in CI already owns
// advisories for the resolved dependency graph.
const sdkCrate = /^(matrix-sdk(-[a-z0-9-]+)?|ruma(-[a-z0-9-]+)?|vodozemac)$/;
const advisoryRepositories = ["matrix-org/matrix-rust-sdk", "matrix-org/vodozemac", "ruma/ruma"];
const issueLabel = "security";

const options = parseArguments(process.argv.slice(2));

try {
  const pinned = pinnedSdkVersions(readFileSync(options.lockfile, "utf8"));
  const unevaluable = [];
  const githubAdvisories = readGithubAdvisories();
  const audit = readAudit();
  console.log(
    `pinned_sdk_crates=${pinned.size} github_advisories=${githubAdvisories.length} ` +
      `rustsec_vulnerabilities=${audit.vulnerabilities?.list?.length ?? 0}`
  );
  const findings = [
    ...githubFindings(githubAdvisories, pinned, unevaluable),
    ...auditFindings(audit),
  ];
  const advisories = mergeAliases(findings);
  console.log(`affected_advisories=${advisories.length}`);
  for (const advisory of advisories) {
    console.log(`affected=${advisory.ids.join(",")} ${packageList(advisory)}`);
  }

  const existing = listIssues({ label: issueLabel, issuesJson: options.issuesJson });
  const actions = advisories
    .filter((advisory) => !existing.some((issue) => advisory.ids.some((id) => issue.title.includes(id))))
    .map((advisory) => ({
      kind: "create",
      title: `SDK security advisory ${advisory.ids.join(" / ")} (${packageList(advisory)})`,
      body: issueBody(advisory),
      labels: [issueLabel],
    }));
  applyIssueActions(actions, { dryRun: options.dryRun });

  if (unevaluable.length > 0) {
    throw new Error(
      `cannot evaluate the affected range of ${unevaluable
        .map(({ id, range }) => `${id} (\`${range}\`)`)
        .join(", ")}; review it by hand`
    );
  }
} catch (error) {
  console.error(`sdk-security-advisory-check: ${error.message}`);
  process.exit(1);
}

function parseArguments(argumentsList) {
  const parsed = {
    lockfile: "Cargo.lock",
    auditJson: null,
    githubAdvisoriesJson: null,
    issuesJson: null,
    dryRun: false,
  };
  const valued = ["--lockfile", "--audit-json", "--github-advisories-json", "--issues-json"];
  for (let index = 0; index < argumentsList.length; index += 1) {
    const argument = argumentsList[index];
    if (valued.includes(argument)) {
      const value = argumentsList[index + 1];
      if (!value) {
        throw new Error(`${argument} requires a value`);
      }
      parsed[argument.slice(2).replace(/-([a-z])/g, (_, letter) => letter.toUpperCase())] = value;
      index += 1;
    } else if (argument === "--dry-run") {
      parsed.dryRun = true;
    } else if (argument === "--help") {
      console.log(
        "Usage: node scripts/sdk-security-advisory-check.mjs [--lockfile PATH] " +
          "[--audit-json PATH] [--github-advisories-json PATH] [--issues-json PATH] [--dry-run]"
      );
      process.exit(0);
    } else {
      throw new Error(`unknown argument: ${argument}`);
    }
  }
  return parsed;
}

/** SDK-family crate name → versions the lockfile pins (path crates included). */
function pinnedSdkVersions(lockfile) {
  const pinned = new Map();
  for (const block of lockfile.split(/^\[\[package\]\]\s*$/m).slice(1)) {
    const name = /^name = "([^"]+)"$/m.exec(block)?.[1];
    const version = /^version = "([^"]+)"$/m.exec(block)?.[1];
    if (name && version && sdkCrate.test(name)) {
      pinned.set(name, [...(pinned.get(name) ?? []), version]);
    }
  }
  if (!pinned.has("matrix-sdk")) {
    throw new Error("the lockfile pins no matrix-sdk package");
  }
  return pinned;
}

function readGithubAdvisories() {
  if (options.githubAdvisoriesJson) {
    return JSON.parse(readFileSync(options.githubAdvisoriesJson, "utf8"));
  }
  return advisoryRepositories.flatMap((repository) => {
    const result = spawnSync(
      "gh",
      ["api", "--paginate", "--slurp", `repos/${repository}/security-advisories?state=published&per_page=100`],
      { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 }
    );
    if (result.status !== 0) {
      throw new Error(
        `gh api failed for ${repository}: ${result.stderr?.trim() || `exit ${result.status}`}`
      );
    }
    return JSON.parse(result.stdout).flat();
  });
}

/** `cargo audit --json` output; it exits 1 when it finds a vulnerability. */
function readAudit() {
  if (options.auditJson) {
    return JSON.parse(readFileSync(options.auditJson, "utf8"));
  }
  const result = spawnSync("cargo", ["audit", "--json", "--file", options.lockfile], {
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  });
  if (result.status !== 0 && result.status !== 1) {
    throw new Error(`cargo audit failed: ${result.stderr?.trim() || `exit ${result.status}`}`);
  }
  return JSON.parse(result.stdout);
}

function githubFindings(advisories, pinned, unevaluable) {
  const findings = [];
  for (const advisory of advisories) {
    if (advisory.withdrawn_at) {
      continue;
    }
    for (const vulnerability of advisory.vulnerabilities ?? []) {
      const name = vulnerability.package?.name?.trim();
      if (vulnerability.package?.ecosystem !== "rust" || !pinned.has(name)) {
        continue;
      }
      const range = vulnerability.vulnerable_version_range ?? "";
      for (const version of pinned.get(name)) {
        let affected;
        try {
          affected = rangeContains(range, version);
        } catch {
          unevaluable.push({ id: advisory.ghsa_id, range });
          break;
        }
        if (affected) {
          findings.push({
            ids: [advisory.ghsa_id, advisory.cve_id].filter(Boolean),
            name,
            version,
            vulnerable: range,
            patched: vulnerability.patched_versions ?? "none",
            severity: advisory.severity,
            summary: advisory.summary,
            url: advisory.html_url,
          });
        }
      }
    }
  }
  return findings;
}

function auditFindings(audit) {
  return (audit.vulnerabilities?.list ?? [])
    .filter(({ advisory, package: crate }) => sdkCrate.test(crate.name) && !advisory.withdrawn)
    .map(({ advisory, versions, package: crate }) => ({
      ids: [advisory.id, ...(advisory.aliases ?? [])],
      name: crate.name,
      version: crate.version,
      vulnerable: "see advisory",
      patched: versions?.patched?.join(", ") || "none",
      severity: null,
      summary: advisory.title,
      url: `https://rustsec.org/advisories/${advisory.id}.html`,
    }));
}

/** One advisory per alias set, so GHSA, CVE and RUSTSEC ids of one flaw share an issue. */
function mergeAliases(findings) {
  const merged = [];
  for (const finding of findings) {
    const match = merged.find((advisory) => finding.ids.some((id) => advisory.ids.includes(id)));
    if (!match) {
      merged.push({ ids: [...finding.ids], findings: [finding] });
      continue;
    }
    match.ids.push(...finding.ids.filter((id) => !match.ids.includes(id)));
    match.findings.push(finding);
  }
  return merged;
}

function packageList(advisory) {
  return [...new Set(advisory.findings.map(({ name, version }) => `${name} ${version}`))].join(", ");
}

function issueBody(advisory) {
  const sources = advisory.findings
    .map(
      (finding) =>
        `- [${finding.ids[0]}](${finding.url})` +
        `${finding.severity ? ` (${finding.severity})` : ""}: ${finding.summary}\n` +
        `  \`${finding.name}\` ${finding.version} is pinned (vulnerable \`${finding.vulnerable}\`, patched \`${finding.patched}\`)`
    )
    .join("\n");
  return `A published security advisory affects an SDK crate version pinned by the root \`Cargo.lock\`.

Advisory ids: ${advisory.ids.join(", ")}

${sources}

The pinned fork can trail unreleased upstream fixes behind its crate version,
so first check whether the pinned fork commit already carries the fix. If it
does not, this pre-empts the every-other-day upstream sync: merge the fix (or
upstream \`main\`) into the fork and move the gitlink as described in
\`docs/agents/environment.md\` (Following upstream).

Opened by the \`sdk-security-advisory-check\` workflow from public advisory data.
`;
}

/**
 * Whether `version` falls in a GitHub advisory range: comma-separated
 * comparators that must all hold (`>= 0.12.0, < 0.16.1`), or a list of exact
 * versions (`0.5.0, 0.5.1`). Throws on anything else.
 */
function rangeContains(range, version) {
  const parts = range
    .split(",")
    .map((part) => part.trim())
    .filter(Boolean)
    .map((part) => {
      const match = /^(>=|<=|>|<|=)?\s*(\d+(?:\.\d+){0,2})$/.exec(part);
      if (!match) {
        throw new Error(`unsupported range: ${range}`);
      }
      return { operator: match[1] ?? null, bound: match[2] };
    });
  if (parts.length === 0) {
    throw new Error("empty range");
  }
  const holds = ({ operator, bound }) => {
    const order = compareVersions(version, bound);
    return {
      ">=": order >= 0,
      "<=": order <= 0,
      ">": order > 0,
      "<": order < 0,
      "=": order === 0,
    }[operator ?? "="];
  };
  return parts.every(({ operator }) => operator === null) ? parts.some(holds) : parts.every(holds);
}

function compareVersions(left, right) {
  const leftParts = versionParts(left);
  const rightParts = versionParts(right);
  for (let index = 0; index < 3; index += 1) {
    if (leftParts[index] !== rightParts[index]) {
      return leftParts[index] < rightParts[index] ? -1 : 1;
    }
  }
  return 0;
}

/** Major, minor, patch; pre-release and build metadata do not move a pinned crate across a bound. */
function versionParts(version) {
  const match = /^(\d+)(?:\.(\d+))?(?:\.(\d+))?/.exec(version);
  if (!match) {
    throw new Error(`unsupported version: ${version}`);
  }
  return [match[1], match[2] ?? "0", match[3] ?? "0"].map(Number);
}
