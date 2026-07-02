#!/usr/bin/env node
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { assertSdkSubmoduleSynced } from "./lib/sdk-submodule-status.mjs";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const fixturePath = optionValue("--status-fixture");
const expectedRevision = optionValue("--expected-rev");

try {
  assertSdkSubmoduleSynced({ repoRoot, fixturePath, expectedRevision });
  console.log("vendor Matrix SDK submodule is synced");
} catch (error) {
  console.error(error.message);
  process.exit(1);
}

function optionValue(name) {
  const prefix = `${name}=`;
  const args = process.argv.slice(2);
  const inline = args.find((value) => value.startsWith(prefix));
  if (inline) {
    return inline.slice(prefix.length);
  }
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
}
