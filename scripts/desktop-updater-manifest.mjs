#!/usr/bin/env node
import { readFileSync, writeFileSync } from "node:fs";

const values = new Map();
for (let index = 2; index < process.argv.length; index += 2) {
  const name = process.argv[index];
  const value = process.argv[index + 1];
  if (!name?.startsWith("--") || value === undefined) {
    fail("arguments must be --name value pairs");
  }
  values.set(name, value);
}

const version = required("--version");
const target = required("--target");
const url = required("--url");
const signature = readFileSync(required("--signature-file"), "utf8").trim();
const output = required("--output");

if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(version)) {
  fail(`invalid SemVer: ${version}`);
}
if (!target || !signature) {
  fail("target and signature must not be empty");
}
if (!url.startsWith("https://")) {
  fail("update URL must use HTTPS");
}

writeFileSync(
  output,
  `${JSON.stringify({
    version,
    pub_date: new Date().toISOString(),
    platforms: {
      [target]: { signature, url }
    }
  }, null, 2)}\n`,
  "utf8"
);

function required(name) {
  const value = values.get(name);
  if (!value) fail(`missing ${name}`);
  return value;
}

function fail(message) {
  console.error(`desktop-updater-manifest: ${message}`);
  process.exit(1);
}
