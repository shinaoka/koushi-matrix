#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
export const repositoryRoot = path.resolve(scriptDirectory, "..");
export const INLINE_TEST_MODULE_LIMIT = 200;
export const FIRST_PARTY_ROOTS = ["crates", "apps/desktop/src-tauri"];
export const ALLOWED_NON_RUST_TARGETS = new Set([
  "docs/architecture/state-machine.md",
  "apps/desktop/src-tauri/capabilities/windows-overlay.json",
  "apps/desktop/src/domain/coreEvents.generated.json"
]);

const identifierStart = (character) => /[A-Za-z_]/u.test(character ?? "");
const identifierPart = (character) => /[A-Za-z0-9_]/u.test(character ?? "");
const openingDelimiters = new Set(["(", "[", "{"]);
const closingDelimiters = new Set([")", "]", "}"]);
const matchingDelimiter = { "(": ")", "[": "]", "{": "}" };
const isOpeningDelimiter = (token) => token?.kind === "punctuation" && openingDelimiters.has(token.value);
const isClosingDelimiter = (token) => token?.kind === "punctuation" && closingDelimiters.has(token.value);

function decodeRustString(value) {
  return value.replace(/\\([\\"'nrt0])/gu, (_, escape) => ({
    "\\": "\\",
    '"': '"',
    "'": "'",
    n: "\n",
    r: "\r",
    t: "\t",
    0: "\0"
  })[escape]);
}

function readQuoted(source, start, prefixLength = 0) {
  const quote = start + prefixLength;
  let escaped = false;
  for (let index = quote + 1; index < source.length; index += 1) {
    const character = source[index];
    if (escaped) {
      escaped = false;
    } else if (character === "\\") {
      escaped = true;
    } else if (character === '"') {
      return {
        end: index + 1,
        value: decodeRustString(source.slice(quote + 1, index))
      };
    }
  }
  return { end: source.length, value: decodeRustString(source.slice(quote + 1)) };
}

function readRawString(source, start) {
  let quote = start;
  if (source[quote] === "b") quote += 1;
  if (source[quote] !== "r") return null;
  quote += 1;
  let hashes = 0;
  while (source[quote + hashes] === "#") hashes += 1;
  quote += hashes;
  if (source[quote] !== '"') return null;
  const terminator = `"${"#".repeat(hashes)}`;
  const close = source.indexOf(terminator, quote + 1);
  const end = close < 0 ? source.length : close + terminator.length;
  return {
    end,
    value: source.slice(quote + 1, close < 0 ? source.length : close)
  };
}

function readChar(source, start) {
  let escaped = false;
  for (let index = start + 1; index < source.length; index += 1) {
    const character = source[index];
    if (escaped) {
      escaped = false;
    } else if (character === "\\") {
      escaped = true;
    } else if (character === "'") {
      return index + 1;
    }
  }
  return -1;
}

/** Lex only enough Rust to safely skip literals/comments and balance items. */
export function lexRust(source) {
  const tokens = [];
  let index = 0;
  let line = 1;

  const advance = (end) => {
    for (; index < end; index += 1) {
      if (source[index] === "\n") line += 1;
    }
  };
  const add = (kind, value, start, end, startLine = line) => {
    tokens.push({ kind, value, start, end, line: startLine });
    advance(end);
  };

  while (index < source.length) {
    const character = source[index];
    const next = source[index + 1];
    if (/\s/u.test(character)) {
      advance(index + 1);
      continue;
    }
    if (character === "/" && next === "/") {
      const end = source.indexOf("\n", index + 2);
      advance(end < 0 ? source.length : end);
      continue;
    }
    if (character === "/" && next === "*") {
      const startLine = line;
      let depth = 1;
      let cursor = index + 2;
      while (cursor < source.length && depth > 0) {
        if (source[cursor] === "/" && source[cursor + 1] === "*") {
          depth += 1;
          cursor += 2;
        } else if (source[cursor] === "*" && source[cursor + 1] === "/") {
          depth -= 1;
          cursor += 2;
        } else {
          cursor += 1;
        }
      }
      add("comment", "", index, cursor, startLine);
      continue;
    }

    const raw = readRawString(source, index);
    if (raw) {
      add("string", raw.value, index, raw.end);
      continue;
    }
    if (character === '"') {
      const quoted = readQuoted(source, index);
      add("string", quoted.value, index, quoted.end);
      continue;
    }
    if (character === "b" && next === '"') {
      const quoted = readQuoted(source, index, 1);
      add("string", quoted.value, index, quoted.end);
      continue;
    }
    if (character === "b" && next === "'") {
      const end = readChar(source, index + 1);
      if (end > 0) {
        add("char", "", index, end);
        continue;
      }
    }
    if (character === "'") {
      const isSimpleChar = !identifierStart(next) || source[index + 2] === "'";
      const end = isSimpleChar ? readChar(source, index) : -1;
      if (end > 0) {
        add("char", "", index, end);
        continue;
      }
      if (identifierStart(next)) {
        let end = index + 2;
        while (identifierPart(source[end])) end += 1;
        add("lifetime", source.slice(index + 1, end), index, end);
        continue;
      }
    }
    if (identifierStart(character)) {
      let end = index + 1;
      while (identifierPart(source[end])) end += 1;
      add("identifier", source.slice(index, end), index, end);
      continue;
    }
    if (/[0-9]/u.test(character)) {
      let end = index + 1;
      while (/[A-Za-z0-9_\.]/u.test(source[end] ?? "")) end += 1;
      add("number", source.slice(index, end), index, end);
      continue;
    }
    add("punctuation", character, index, index + 1);
  }
  return tokens;
}

function delimiterPairs(tokens) {
  const openToClose = new Map();
  const closeToOpen = new Map();
  const stack = [];
  const errors = [];
  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index];
    if (isOpeningDelimiter(token)) {
      stack.push(index);
    } else if (isClosingDelimiter(token)) {
      const open = stack.pop();
      if (open === undefined || matchingDelimiter[tokens[open].value] !== token.value) {
        errors.push({ kind: "unbalanced delimiter", token: tokens[index] });
      } else {
        openToClose.set(open, index);
        closeToOpen.set(index, open);
      }
    }
  }
  for (const open of stack) errors.push({ kind: "unbalanced delimiter", token: tokens[open] });
  return { openToClose, closeToOpen, errors };
}

function braceDepths(tokens) {
  const depths = [];
  let depth = 0;
  for (const token of tokens) {
    depths.push(depth);
    if (token.kind === "punctuation" && token.value === "{") depth += 1;
    if (token.kind === "punctuation" && token.value === "}") depth -= 1;
  }
  return depths;
}

function cfgAttribute(tokens, hashIndex, pairs) {
  if (tokens[hashIndex]?.value !== "#" || tokens[hashIndex + 1]?.value !== "[") return null;
  const close = pairs.openToClose.get(hashIndex + 1);
  if (close === undefined) return null;
  if (tokens[hashIndex + 2]?.value !== "cfg") return null;
  const expression = tokens.slice(hashIndex + 3, close);
  let hasTest = false;
  let negated = false;
  for (let index = 0; index < expression.length; index += 1) {
    const token = expression[index];
    if (token.value === "not" && expression[index + 1]?.value === "(") {
      negated = true;
    } else if (token.kind === "identifier" && token.value === "test" && !negated) {
      hasTest = true;
    }
    if (token.value === ")") negated = false;
  }
  return { start: tokens[hashIndex].start, end: tokens[close].end, line: tokens[hashIndex].line, hasTest };
}

function attachedAttributes(tokens, moduleIndex, pairs) {
  let cursor = moduleIndex - 1;
  if (tokens[cursor]?.value === ")") {
    const open = pairs.closeToOpen.get(cursor);
    if (open !== undefined && tokens[open - 1]?.value === "pub") cursor = open - 2;
  }
  while (tokens[cursor]?.value === "pub" || tokens[cursor]?.value === "unsafe") cursor -= 1;

  const attributes = [];
  while (tokens[cursor]?.value === "]") {
    const open = pairs.closeToOpen.get(cursor);
    if (open === undefined || tokens[open - 1]?.value !== "#") break;
    const attribute = cfgAttribute(tokens, open - 1, pairs);
    if (attribute) attributes.unshift(attribute);
    cursor = open - 2;
  }
  return attributes;
}

function moduleInventory(source, fileName) {
  const tokens = lexRust(source);
  const pairs = delimiterPairs(tokens);
  const depths = braceDepths(tokens);
  const inline = [];
  const external = [];
  const nested = [];
  const errors = pairs.errors.map(({ kind, token }) => `${fileName}:${token.line}:${kind}`);

  for (let index = 0; index < tokens.length; index += 1) {
    if (tokens[index].value !== "mod" || tokens[index + 1]?.kind !== "identifier") continue;
    const nameToken = tokens[index + 1];
    const attributes = attachedAttributes(tokens, index, pairs);
    if (!attributes.some(({ hasTest }) => hasTest)) continue;
    const declaration = tokens[index + 2];
    const module = {
      file: fileName,
      name: nameToken.value,
      line: attributes[0]?.line ?? tokens[index].line,
      start: attributes[0]?.start ?? tokens[index].start,
      declarationLine: tokens[index].line,
      physicalLines: null,
      overThreshold: false
    };
    if (declaration?.value === ";") {
      external.push(module);
      continue;
    }
    if (declaration?.value !== "{") {
      errors.push(`${fileName}:${tokens[index].line}:ambiguous cfg(test) module`);
      continue;
    }
    const close = pairs.openToClose.get(index + 2);
    if (close === undefined) {
      errors.push(`${fileName}:${tokens[index].line}:unclosed cfg(test) module`);
      continue;
    }
    module.end = tokens[close].end;
    module.physicalLines = source.slice(module.start, module.end).split("\n").length;
    module.overThreshold = module.physicalLines >= INLINE_TEST_MODULE_LIMIT;
    if (depths[index] > 0) {
      nested.push(module);
    } else {
      inline.push(module);
    }
  }
  return { inline, external, nested, errors };
}

export function findInlineTestModules(source, fileName = "fixture.rs") {
  return moduleInventory(source, fileName).inline;
}

function splitArguments(tokens) {
  const argumentsList = [];
  let current = [];
  let depth = 0;
  for (const token of tokens) {
    if (isOpeningDelimiter(token)) depth += 1;
    if (isClosingDelimiter(token)) depth -= 1;
    if (token.value === "," && depth === 0) {
      argumentsList.push(current);
      current = [];
    } else {
      current.push(token);
    }
  }
  if (current.length > 0) argumentsList.push(current);
  return argumentsList;
}

function evaluateExpression(tokens, manifestDir) {
  if (tokens.length === 1 && tokens[0].kind === "string") return tokens[0].value;
  if (tokens[0]?.kind !== "identifier" || tokens[1]?.value !== "!" || tokens[2]?.value !== "(") return null;
  let depth = 0;
  let close = -1;
  for (let index = 2; index < tokens.length; index += 1) {
    if (tokens[index].kind === "punctuation" && tokens[index].value === "(") depth += 1;
    if (tokens[index].kind === "punctuation" && tokens[index].value === ")") {
      depth -= 1;
      if (depth === 0) {
        close = index;
        break;
      }
    }
  }
  if (close !== tokens.length - 1) return null;
  const args = splitArguments(tokens.slice(3, close));
  if (tokens[0].value === "env") {
    return evaluateExpression(args[0] ?? [], manifestDir) === "CARGO_MANIFEST_DIR" ? manifestDir : null;
  }
  if (tokens[0].value === "concat") {
    const values = args.map((argument) => evaluateExpression(argument, manifestDir));
    return values.every((value) => value !== null) ? values.join("") : null;
  }
  return null;
}

function findManifestDir(filePath, root) {
  let directory = path.dirname(filePath);
  const rootPath = path.resolve(root);
  while (directory.startsWith(rootPath)) {
    if (fs.existsSync(path.join(directory, "Cargo.toml"))) return directory;
    const parent = path.dirname(directory);
    if (parent === directory) break;
    directory = parent;
  }
  return rootPath;
}

function displayPath(absolutePath, root) {
  const relative = path.relative(root, absolutePath).split(path.sep).join("/");
  return relative && !relative.startsWith("../") && relative !== ".." ? relative : "<outside-repository>";
}

function normalizeFilePath(filePath, root) {
  return path.isAbsolute(filePath) ? path.normalize(filePath) : path.resolve(root, filePath);
}

export function findIncludeStrInvocations(source, filePath, options = {}) {
  const root = path.resolve(options.repositoryRoot ?? repositoryRoot);
  const absoluteFile = normalizeFilePath(filePath, root);
  const tokens = lexRust(source).map((token, index) => ({ ...token, index }));
  const pairs = delimiterPairs(tokens);
  const manifestDir = findManifestDir(absoluteFile, root);
  const includes = [];

  for (let index = 0; index < tokens.length; index += 1) {
    if (tokens[index].value !== "include_str" || tokens[index + 1]?.value !== "!" || tokens[index + 2]?.value !== "(") continue;
    const close = pairs.openToClose.get(index + 2);
    if (close === undefined) continue;
    const argumentTokens = tokens.slice(index + 3, close).filter(({ kind }) => kind !== "comment");
    const expression = evaluateExpression(argumentTokens, manifestDir);
    const targetPath = expression === null
      ? null
      : path.isAbsolute(expression)
        ? path.normalize(expression)
        : path.resolve(path.dirname(absoluteFile), expression);
    const target = targetPath ? displayPath(targetPath, root) : "<unresolved>";
    const exists = targetPath !== null && fs.existsSync(targetPath);
    includes.push({
      file: displayPath(absoluteFile, root),
      line: tokens[index].line,
      target,
      exists,
      rustSource: target.endsWith(".rs"),
      allowedNonRust: ALLOWED_NON_RUST_TARGETS.has(target),
      resolvedPath: targetPath
    });
    index = close;
  }
  return includes;
}

function readRustSource(relativePath) {
  return fs.readFileSync(path.join(repositoryRoot, relativePath), "utf8");
}

function readTauriSource(relativePath) {
  return readRustSource(`apps/desktop/src-tauri/src/${relativePath}`);
}

function productionOnly(source, fileName) {
  const modules = moduleInventory(source, fileName).inline.slice().sort((left, right) => left.start - right.start);
  let cursor = 0;
  let result = "";
  for (const module of modules) {
    result += source.slice(cursor, module.start);
    cursor = module.end;
  }
  return result + source.slice(cursor);
}

function tauriCommandsSource() {
  return [
    "commands/account.rs",
    "commands/activity.rs",
    "commands/diagnostics.rs",
    "commands/directory.rs",
    "commands/e2ee.rs",
    "commands/live_signals.rs",
    "commands/local_encryption.rs",
    "commands/mod.rs",
    "commands/native_attention.rs",
    "commands/navigation.rs",
    "commands/profile.rs",
    "commands/room.rs",
    "commands/search.rs",
    "commands/session.rs",
    "commands/settings.rs",
    "commands/timeline.rs",
    "commands/views.rs"
  ].map((relativePath) => productionOnly(readTauriSource(relativePath), relativePath)).join("\n");
}

function sourceSection(source, startMarker, endMarker) {
  const start = source.indexOf(startMarker);
  if (start < 0) return null;
  const rest = source.slice(start + startMarker.length);
  const end = endMarker ? rest.indexOf(endMarker) : -1;
  return end < 0 ? rest : rest.slice(0, end);
}

function orderedMarkers(rule, source, markers) {
  const positions = markers.map((marker) => source.indexOf(marker));
  const failures = [];
  if (positions.some((position) => position < 0)) {
    failures.push(sourceContractFailure(rule, "required source marker is missing"));
  } else if (positions.some((position, index) => index > 0 && positions[index - 1] >= position)) {
    failures.push(sourceContractFailure(rule, "required source markers are out of order"));
  }
  return failures;
}

function tauriCommandNames(source) {
  const tokens = lexRust(source);
  const names = [];
  for (let index = 0; index + 9 < tokens.length; index += 1) {
    const values = tokens.slice(index, index + 10).map((token) => token.value);
    if (values.slice(0, 7).join("") !== "#[tauri::command]" || values[7] !== "pub" || values[8] !== "async" || values[9] !== "fn") continue;
    const name = tokens[index + 10]?.value;
    if (tokens[index + 10]?.kind === "identifier") names.push(name);
  }
  return names;
}

export function checkDesktopTauriCommandRegistrationContract() {
  const rule = "desktop.commands.tauri_command_registration";
  const source = tauriCommandsSource();
  const libSource = productionOnly(readTauriSource("lib.rs"), "apps/desktop/src-tauri/src/lib.rs");
  const handlerStart = libSource.indexOf("tauri::generate_handler![");
  const handlerEnd = handlerStart < 0 ? -1 : libSource.indexOf("]", handlerStart);
  const handler = handlerStart >= 0 && handlerEnd >= 0 ? libSource.slice(handlerStart, handlerEnd) : null;
  const names = tauriCommandNames(source);
  const failures = [];
  if (!handler) failures.push(sourceContractFailure(rule, "generate_handler! is missing or unclosed"));
  if (names.length === 0) failures.push(sourceContractFailure(rule, "no Tauri commands found"));
  for (const name of names) {
    if (!handler?.split("\n").some((line) => line.includes("commands::") && line.includes(`::${name}`))) {
      failures.push(sourceContractFailure(rule, `Tauri command registration is missing for ${name}`));
    }
  }
  return failures;
}

export function checkDesktopSubmitCoreCommandContract() {
  const rule = "desktop.commands.submit_core_command_contract";
  const source = readTauriSource("commands/mod.rs");
  const body = rustItemBody(source, "pub(crate) async fn submit_core_command");
  const failures = [];
  for (const marker of ["const CORE_COMMAND_SUBMIT_TIMEOUT", "command_handle", "tokio::time::timeout(CORE_COMMAND_SUBMIT_TIMEOUT"]) {
    if (!source.includes(marker) && !body?.includes(marker)) failures.push(sourceContractFailure(rule, `missing ${marker}`));
  }
  if (body?.includes(".lock()\n        .await\n        .command(command)\n        .await") || body?.includes(".lock().await.command(command).await")) {
    failures.push(sourceContractFailure(rule, "submit_core_command holds the connection mutex while awaiting send"));
  }
  return failures;
}

export function checkDesktopEventWaitLagContract() {
  const rule = "desktop.commands.event_wait_lag_contract";
  const directory = readTauriSource("commands/directory.rs");
  const room = readTauriSource("commands/room.rs");
  const source = `${directory}\n${room}`;
  const failures = [];
  for (const marker of ["wait_for_request_outcome", "RequestOutcomeExpectation::DirectoryQuery", "RequestOutcomeExpectation::DirectoryPreview", "RequestOutcomeExpectation::RoomCreated", "RequestOutcomeExpectation::SpaceCreated", "RequestOutcomeExpectation::DirectMessageStarted", "RequestOutcomeExpectation::RoomJoined", "RequestOutcomeExpectation::InviteWorkflow", "RequestOutcomeExpectation::RoomOperation"]) {
    if (!source.includes(marker)) failures.push(sourceContractFailure(rule, `missing Core outcome delegation ${marker}`));
  }
  for (const marker of ["timeout_at", "recv_event", "InviteWorkflowSnapshotSource"]) {
    if (source.includes(marker)) failures.push(sourceContractFailure(rule, `adapter retains forbidden waiter marker ${marker}`));
  }
  return failures;
}

export function checkDesktopFailureWaiterContract() {
  const rule = "desktop.commands.failure_waiter_contract";
  const source = `${readTauriSource("commands/directory.rs")}\n${readTauriSource("commands/room.rs")}`;
  const failures = [];
  if (!source.includes("invoke_error_from_request_outcome")) failures.push(sourceContractFailure(rule, "room/directory outcome errors are not mapped through the Core outcome boundary"));
  if (source.includes("invoke_error_from_core_failure")) failures.push(sourceContractFailure(rule, "room/directory retains adapter failure mapping"));
  return failures;
}

export function checkDesktopActivityNavigationContract() {
  const rule = "desktop.activity.navigation_contract";
  const source = readTauriSource("commands/navigation.rs");
  const routes = [
    ["open_activity_event", rustItemBody(source, "pub async fn open_activity_event")],
    ["open_pinned_event", rustItemBody(source, "pub async fn open_pinned_event")],
    ["select_search_result", rustItemBody(source, "pub async fn select_search_result")]
  ];
  const helper = rustItemBody(source, "async fn navigate_to_event");
  const policy = rustItemBody(source, "fn event_navigation_policy");
  const failures = [];
  for (const [name, body] of routes) {
    if (!body?.includes("navigate_to_event(")) {
      failures.push(sourceContractFailure(rule, `${name} does not route through navigate_to_event`));
    }
  }
  if (!helper?.includes("navigate_to_event_and_wait")) {
    failures.push(sourceContractFailure(rule, "navigate_to_event lacks Core waiter delegation"));
  }
  for (const marker of [
    "EventNavigationSource::Activity",
    "EventNavigationSource::Search",
    "EventNavigationSource::Pinned",
    "EventNavigationMissingTargetPolicy::LiveFallback",
    "EventNavigationMissingTargetPolicy::Fail"
  ]) {
    if (!policy?.includes(marker)) failures.push(sourceContractFailure(rule, `event navigation policy lacks ${marker}`));
  }
  const eventPath = routes.map(([, body]) => body ?? "").concat(helper ?? "").join("\n");
  for (const marker of [
    "open_anchored_timeline",
    "CloseFocusedContext",
    "wait_for_focused_context_closed",
    "select_room_and_wait",
    "OpenAnchoredTimeline",
    "wait_for_main_timeline_anchor",
    "build_subscribe_timeline_command",
    "EnterAnchoredTimeline",
    "wait_for_focused_timeline_event",
    "build_update_navigation_scroll_anchor_command"
  ]) {
    if (eventPath.includes(marker)) failures.push(sourceContractFailure(rule, `event navigation contains forbidden ${marker}`));
  }
  return failures;
}

export function checkDesktopActivityCommandContract() {
  const rule = "desktop.activity.command_contract";
  const source = tauriCommandsSource();
  const libSource = productionOnly(readTauriSource("lib.rs"), "apps/desktop/src-tauri/src/lib.rs");
  const failures = [];
  for (const [command, builder, route] of [
    ["pub async fn open_activity", "build_open_activity_command", "commands::activity::open_activity"],
    ["pub async fn close_activity", "build_close_activity_command", "commands::activity::close_activity"],
    ["pub async fn set_activity_tab", "build_set_activity_tab_command", "commands::activity::set_activity_tab"],
    ["pub async fn paginate_activity", "build_paginate_activity_command", "commands::activity::paginate_activity"],
    ["pub async fn mark_activity_read", "build_mark_activity_read_command", "commands::activity::mark_activity_read"],
    ["pub async fn retry_activity_resolution", "build_retry_activity_resolution_command", "commands::activity::retry_activity_resolution"],
    ["pub async fn open_files_view", "build_open_files_view_command", "commands::views::open_files_view"],
    ["pub async fn close_files_view", "build_close_files_view_command", "commands::views::close_files_view"]
  ]) {
    if (!source.includes(command) || !source.includes(builder) || !libSource.includes(route)) failures.push(sourceContractFailure(rule, `missing ${command}, ${builder}, or ${route}`));
  }
  return failures;
}

export function checkDesktopLoginWaitContract() {
  const rule = "desktop.session.login_wait_contract";
  const source = readTauriSource("commands/session.rs");
  const helper = rustItemBody(source, "async fn submit_login_and_wait_for_authenticated");
  const oidcStart = rustItemBody(source, "pub async fn start_oidc_login");
  const oidcComplete = rustItemBody(source, "pub async fn complete_oidc_login");
  const saved = rustItemBody(source, "pub async fn list_saved_sessions");
  const failures = [];
  for (const marker of ["wait_for_request_outcome", "RequestOutcomeExpectation::Authenticated", "baseline_generation", "LOGIN_EVENT_TIMEOUT"]) {
    if (!helper?.includes(marker)) failures.push(sourceContractFailure(rule, `login helper lacks Core outcome marker ${marker}`));
  }
  if (helper?.includes("build_start_sync_command") || helper?.includes("wait_for_logged_in_authenticated")) failures.push(sourceContractFailure(rule, "login helper retains adapter-owned settlement or sync start"));
  for (const [body, expectation] of [[oidcStart, "OidcAuthorization"], [oidcComplete, "Authenticated"], [saved, "SavedSessions"]]) {
    if (!body?.includes("wait_for_request_outcome") || !body.includes(`RequestOutcomeExpectation::${expectation}`)) failures.push(sourceContractFailure(rule, `session command lacks Core ${expectation} outcome delegation`));
  }
  return failures;
}

export function checkDesktopE2eeCommandContract() {
  const rule = "desktop.e2ee.command_contract";
  const source = tauriCommandsSource();
  const libSource = productionOnly(readTauriSource("lib.rs"), "apps/desktop/src-tauri/src/lib.rs");
  const failures = [];
  for (const [command, builder, route] of [
    ["pub async fn bootstrap_cross_signing", "build_bootstrap_cross_signing_command", "commands::e2ee::bootstrap_cross_signing"],
    ["pub async fn enable_key_backup", "build_enable_key_backup_command", "commands::e2ee::enable_key_backup"],
    ["pub async fn export_room_keys", "build_export_room_keys_command", "commands::e2ee::export_room_keys"],
    ["pub async fn import_room_keys", "build_import_room_keys_command", "commands::e2ee::import_room_keys"],
    ["pub async fn bootstrap_secure_backup", "build_bootstrap_secure_backup_command", "commands::e2ee::bootstrap_secure_backup"],
    ["pub async fn change_secure_backup_passphrase", "build_change_secure_backup_passphrase_command", "commands::e2ee::change_secure_backup_passphrase"],
    ["pub async fn accept_verification", "build_accept_verification_command", "commands::e2ee::accept_verification"],
    ["pub async fn confirm_sas_verification", "build_confirm_sas_verification_command", "commands::e2ee::confirm_sas_verification"],
    ["pub async fn cancel_verification", "build_cancel_verification_command", "commands::e2ee::cancel_verification"],
    ["pub async fn reset_identity", "build_reset_identity_command", "commands::e2ee::reset_identity"],
    ["pub async fn cancel_identity_reset", "build_cancel_identity_reset_command", "commands::e2ee::cancel_identity_reset"],
    ["pub async fn submit_identity_reset_password", "build_submit_identity_reset_password_command", "commands::e2ee::submit_identity_reset_password"],
    ["pub async fn submit_identity_reset_oauth", "build_submit_identity_reset_oauth_command", "commands::e2ee::submit_identity_reset_oauth"]
  ]) {
    if (!source.includes(command) || !source.includes(builder) || !libSource.includes(route)) failures.push(sourceContractFailure(rule, `missing E2EE command contract for ${command}`));
  }
  if (!source.includes("intent: koushi_state::SecureBackupSetupIntent")) failures.push(sourceContractFailure(rule, "secure-backup bootstrap lacks typed intent transport"));
  if (source.includes("pub async fn reenable_secure_backup") || libSource.includes("commands::e2ee::reenable_secure_backup")) failures.push(sourceContractFailure(rule, "native secure-backup re-enable policy route remains"));
  if (source.includes("MessageDialogButtons") || source.includes("Secure Backupを再有効化")) failures.push(sourceContractFailure(rule, "native secure-backup confirmation copy remains"));
  return failures;
}

export function checkDesktopLocalEncryptionCommandContract() {
  const rule = "desktop.local_encryption.command_contract";
  const source = tauriCommandsSource();
  const libSource = productionOnly(readTauriSource("lib.rs"), "apps/desktop/src-tauri/src/lib.rs");
  const failures = [];
  for (const [command, builder, route, registration] of [
    ["pub async fn probe_local_encryption_health", "build_probe_local_encryption_health_command", "AccountCommand::ProbeLocalEncryptionHealth", "commands::local_encryption::probe_local_encryption_health"],
    ["pub async fn reset_local_data", "build_reset_local_data_command", "AccountCommand::ResetLocalData", "commands::local_encryption::reset_local_data"]
  ]) {
    if (!source.includes(command) || !source.includes(builder) || !source.includes(route) || !libSource.includes(registration)) failures.push(sourceContractFailure(rule, `missing local-encryption contract for ${command}`));
  }
  const reset = rustItemBody(readTauriSource("commands/local_encryption.rs"), "pub async fn reset_local_data");
  for (const marker of ["submit_core_command_with_admission", "FrontendCommandAdmission"]) {
    if (!reset?.includes(marker)) failures.push(sourceContractFailure(rule, `reset_local_data lacks Core admission marker ${marker}`));
  }
  for (const marker of ["wait_for_request_outcome", "RequestOutcomeExpectation::SignedOut", "current_snapshot"]) {
    if (reset?.includes(marker)) failures.push(sourceContractFailure(rule, `reset_local_data retains terminal snapshot marker ${marker}`));
  }
  return failures;
}

export function checkDesktopProfileCommandContract() {
  const rule = "desktop.profile.command_contract";
  const source = tauriCommandsSource();
  const libSource = productionOnly(readTauriSource("lib.rs"), "apps/desktop/src-tauri/src/lib.rs");
  const failures = [];
  for (const [command, builder, registration] of [["pub async fn set_display_name", "build_set_display_name_command", "commands::profile::set_display_name"], ["pub async fn set_local_user_alias", "build_set_local_user_alias_command", "commands::profile::set_local_user_alias"], ["pub async fn set_avatar", "build_set_avatar_command", "commands::profile::set_avatar"]]) {
    if (!source.includes(command) || !source.includes(builder) || !libSource.includes(registration)) failures.push(sourceContractFailure(rule, `missing profile contract for ${command}`));
  }
  return failures;
}

export function checkDesktopDirectoryStartDmContract() {
  const rule = "desktop.directory.start_dm_contract";
  const body = rustItemBody(readTauriSource("commands/room.rs"), "pub async fn start_direct_message");
  const failures = [];
  for (const marker of ["wait_for_request_outcome", "RequestOutcomeExpectation::DirectMessageStarted", "select_room_and_wait"]) if (!body?.includes(marker)) failures.push(sourceContractFailure(rule, `start_direct_message lacks Core outcome marker ${marker}`));
  for (const marker of ["wait_for_direct_message_started", "wait_for_room_in_state", "recv_event"]) if (body?.includes(marker)) failures.push(sourceContractFailure(rule, `start_direct_message retains adapter waiter ${marker}`));
  return failures;
}

export function checkDesktopDirectoryJoinRoomContract() {
  const rule = "desktop.directory.join_room_selection_contract";
  const body = rustItemBody(readTauriSource("commands/directory.rs"), "pub async fn join_directory_room");
  const failures = [];
  for (const marker of ["wait_for_request_outcome", "RequestOutcomeExpectation::RoomJoined", "select_room_and_wait", "joined_room_id", "SELECT_ROOM_EVENT_TIMEOUT"]) if (!body?.includes(marker)) failures.push(sourceContractFailure(rule, `join_directory_room lacks ${marker}`));
  if (body?.includes("wait_for_room_joined") || body?.includes("recv_event")) failures.push(sourceContractFailure(rule, "join_directory_room retains adapter waiter"));
  failures.push(...orderedMarkers(rule, body ?? "", ["RequestOutcomeExpectation::RoomJoined", "select_room_and_wait"]));
  return failures;
}

export function checkDesktopRoomOperationContract() {
  const rule = "desktop.room.operation_wait_contract";
  const source = readTauriSource("commands/room.rs");
  const failures = [];
  for (const [command, operation] of [["pub async fn load_room_settings", "RoomSettingsLoaded"], ["pub async fn update_room_setting", "RoomSettingUpdated"], ["pub async fn moderate_room_member", "MemberModerated"], ["pub async fn update_room_member_role", "MemberRoleUpdated"]]) {
    const body = rustItemBody(source, command);
    for (const marker of ["wait_for_room_operation", `RoomOperationKind::${operation}`, "command_settlement"]) if (!body?.includes(marker)) failures.push(sourceContractFailure(rule, `${command} lacks Core room outcome marker ${marker}`));
    if (body?.includes("recv_event") || body?.includes("timeout_at")) failures.push(sourceContractFailure(rule, `${command} retains adapter waiter`));
  }
  return failures;
}

export function checkDesktopSpaceOperationContract() {
  const rule = "desktop.room.space_operation_contract";
  const source = readTauriSource("commands/room.rs");
  const libSource = productionOnly(readTauriSource("lib.rs"), "apps/desktop/src-tauri/src/lib.rs");
  const failures = [];
  for (const [command, operation] of [["pub async fn load_space_members", "SpaceMembersLoaded"], ["pub async fn invite_user_to_space", "SpaceMemberInviteSettled"], ["pub async fn cancel_space_invite", "SpaceMemberInviteCancellationSettled"], ["pub async fn update_space_member_role", "SpaceMemberRoleUpdated"]]) {
    const body = rustItemBody(source, command);
    for (const marker of ["wait_for_room_operation", `RoomOperationKind::${operation}`, "command_settlement"]) if (!body?.includes(marker)) failures.push(sourceContractFailure(rule, `${command} lacks Core space outcome marker ${marker}`));
    if (body?.includes("recv_event") || body?.includes("timeout_at")) failures.push(sourceContractFailure(rule, `${command} retains adapter waiter`));
  }
  for (const registration of ["commands::room::cancel_space_invite", "commands::room::update_space_member_role"]) if (!libSource.includes(registration)) failures.push(sourceContractFailure(rule, `space operation registration is missing ${registration}`));
  return failures;
}

export function checkDesktopSearchCommandContract() {
  const rule = "desktop.search.command_contract";
  const source = readTauriSource("commands/search.rs");
  const resolver = rustItemBody(source, "fn resolve_search_scope_from_active_room");
  const command = rustItemBody(source, "pub async fn submit_search");
  const helper = rustItemBody(source, "pub(crate) async fn submit_search_production_path");
  const failures = [];
  for (const marker of ["SearchScope::CurrentSpace", "SearchScope::CurrentRoom"]) if (!resolver?.includes(marker)) failures.push(sourceContractFailure(rule, `search scope resolver lacks ${marker}`));
  if (resolver?.includes("unwrap_or(SearchScope::AllRooms)")) failures.push(sourceContractFailure(rule, "search scope resolver collapses to allRooms"));
  for (const marker of ["submit_search_production_path", "FrontendCommandSettlement", "Ok(settlement)"]) if (!command?.includes(marker)) failures.push(sourceContractFailure(rule, `submit_search lacks ${marker}`));
  for (const marker of ["state.runtime.attach", "versioned_snapshot", "baseline_generation", "next_request_id(state).await", "submit_core_command", "wait_for_request_outcome", "RequestOutcomeExpectation::SearchStarted", "account_key", "query", "search_scope"]) {
    if (!helper?.includes(marker)) failures.push(sourceContractFailure(rule, `search path lacks Core outcome marker ${marker}`));
  }
  const close = rustItemBody(source, "pub async fn close_search");
  for (const marker of ["state.inner().runtime.attach", "versioned_snapshot", "baseline_generation", "next_request_id(state.inner()).await", "submit_core_command", "wait_for_request_outcome", "RequestOutcomeExpectation::SearchClosed", "account_key"]) {
    if (!close?.includes(marker)) failures.push(sourceContractFailure(rule, `close search lacks Core outcome marker ${marker}`));
  }
  for (const marker of ["wait_for_search_started", "wait_for_search_closed", "SearchPathIo", "timeout_at", "recv_event"]) {
    if (source.includes(marker)) failures.push(sourceContractFailure(rule, `search adapter retains forbidden waiter marker ${marker}`));
  }
  return failures;
}

export function checkDesktopSettingsCommandContract() {
  const rule = "desktop.settings.command_contract";
  const source = tauriCommandsSource();
  const libSource = productionOnly(readTauriSource("lib.rs"), "apps/desktop/src-tauri/src/lib.rs");
  const failures = [];
  for (const [command, builder, route, registration] of [["pub async fn update_settings", "build_update_settings_command", "AppCommand::UpdateSettings", "commands::settings::update_settings"], ["pub async fn set_room_url_preview_override", "build_set_room_url_preview_override_command", "AppCommand::SetRoomUrlPreviewOverride", "commands::settings::set_room_url_preview_override"], ["pub async fn rebuild_search_index", "build_rebuild_search_index_command", "AppCommand::RebuildSearchIndex", "commands::settings::rebuild_search_index"]]) {
    if (!source.includes(command) || !source.includes(builder) || !source.includes(route) || !libSource.includes(registration)) failures.push(sourceContractFailure(rule, `missing settings contract for ${command}`));
  }
  return failures;
}

export function checkDesktopNavigationContract() {
  const rule = "desktop.navigation.command_contract";
  const source = tauriCommandsSource();
  const failures = [];
  const select = rustItemBody(source, "pub async fn select_room");
  for (const marker of ["state.runtime.attach", "select_room_and_wait", "SELECT_ROOM_EVENT_TIMEOUT"]) if (!select?.includes(marker)) failures.push(sourceContractFailure(rule, `select_room lacks ${marker}`));
  for (const marker of ["build_select_room_command", "wait_for_selected_room", "build_subscribe_timeline_command", "account_key_from_snapshot"]) if (select?.includes(marker)) failures.push(sourceContractFailure(rule, `select_room contains forbidden ${marker}`));
  const trace = readTauriSource("commands/timeline.rs");
  const paginate = rustItemBody(readTauriSource("commands/timeline.rs"), "pub async fn paginate_timeline_backwards");
  const previews = rustItemBody(readTauriSource("commands/timeline.rs"), "pub async fn load_link_previews");
  if (!trace.includes("fn trace_tauri_timeline_command") || !trace.includes("desktop.timeline")) failures.push(sourceContractFailure(rule, "timeline trace helper lacks its private source token"));
  if (select?.includes("trace_tauri_timeline_command(\"submit\", \"select_room\"")) failures.push(sourceContractFailure(rule, "select_room emits duplicate adapter submit telemetry"));
  if (!paginate?.includes("trace_tauri_timeline_command(\"submit\", \"paginate_backwards\"")) failures.push(sourceContractFailure(rule, "backfill submit trace is missing"));
  if (!previews?.includes("trace_tauri_timeline_command(\"submit\", \"load_link_previews\"")) failures.push(sourceContractFailure(rule, "link-preview submit trace is missing"));
  const search = rustItemBody(source, "pub async fn select_search_result");
  const eventNavigation = rustItemBody(source, "async fn navigate_to_event");
  if (!search?.includes("navigate_to_event(")) failures.push(sourceContractFailure(rule, "search-result navigation lacks navigate_to_event"));
  if (!eventNavigation?.includes("navigate_to_event_and_wait")) failures.push(sourceContractFailure(rule, "event navigation lacks Core waiter delegation"));
  for (const marker of ["open_anchored_timeline", "CloseFocusedContext", "OpenAnchoredTimeline", "select_room_and_wait", "wait_for_main_timeline_anchor", "EnterAnchoredTimeline", "wait_for_focused_timeline_event", "build_subscribe_timeline_command"]) if (eventNavigation?.includes(marker)) failures.push(sourceContractFailure(rule, `event navigation contains forbidden ${marker}`));
  const close = rustItemBody(source, "pub async fn close_focused_context");
  for (const marker of ["CloseFocusedContext", "update_qa_window_title_from_state", "FrontendCommandSettlement::from_published_generation"]) if (!close?.includes(marker)) failures.push(sourceContractFailure(rule, `close_focused_context lacks ${marker}`));
  failures.push(...orderedMarkers(rule, close ?? "", ["CloseFocusedContext", "wait_for_focused_context_closed", "FrontendCommandSettlement::from_published_generation"]));
  return failures;
}

export function checkDesktopSpaceTraceContract() {
  const rule = "desktop.navigation.space_trace_contract";
  const body = rustItemBody(readTauriSource("commands/navigation.rs"), "pub async fn select_space");
  const failures = orderedMarkers(rule, body ?? "", ["\"desktop.space.transition\", \"submit\"", "build_select_space_command", "\"admitted\""]);
  for (const marker of ["DiagnosticField::request_id", "DiagnosticField::milliseconds", "DiagnosticField::boolean"]) if (!body?.includes(marker)) failures.push(sourceContractFailure(rule, `space transition trace lacks ${marker}`));
  return failures;
}

export function checkDesktopTimelineCommandContract() {
  const rule = "desktop.timeline.command_contract";
  const source = readTauriSource("commands/timeline.rs");
  const libSource = productionOnly(readTauriSource("lib.rs"), "apps/desktop/src-tauri/src/lib.rs");
  const body = rustItemBody(source, "pub async fn resolve_composer_key_action");
  const failures = [];
  for (const marker of ["koushi_state::resolve_composer_key_action", "settings.values.keyboard.composer_send_shortcut"]) if (!body?.includes(marker)) failures.push(sourceContractFailure(rule, `composer resolver command lacks ${marker}`));
  const threadCommand = rustItemBody(source, "pub async fn paginate_thread_timeline_backwards");
  const threadBuilder = rustItemBody(source, "build_paginate_thread_timeline_backwards_command");
  for (const marker of ["TimelineKind::Thread", "PaginationDirection::Backward", "event_count: TIMELINE_BACKWARDS_PAGE_EVENT_COUNT"]) if (!threadBuilder?.includes(marker)) failures.push(sourceContractFailure(rule, `thread pagination builder lacks ${marker}`));
  if (!threadCommand) failures.push(sourceContractFailure(rule, "thread pagination command is missing"));
  for (const registration of ["commands::timeline::resolve_composer_key_action", "commands::timeline::paginate_thread_timeline_backwards"]) {
    if (!libSource.includes(registration)) failures.push(sourceContractFailure(rule, `timeline command registration is missing ${registration}`));
  }
  return failures;
}

export function checkDesktopTimelineSignalContract() {
  const rule = "desktop.timeline.signal_contract";
  const source = readTauriSource("commands/timeline.rs") + readTauriSource("commands/live_signals.rs");
  const libSource = productionOnly(readTauriSource("lib.rs"), "apps/desktop/src-tauri/src/lib.rs");
  const failures = [];
  for (const [command, kind] of [["pub async fn send_reaction", "send_reaction"], ["pub async fn redact_reaction", "redact_reaction"], ["pub async fn send_read_receipt", "send_read_receipt"], ["pub async fn set_fully_read", "set_fully_read"]]) {
    const body = rustItemBody(source, command);
    for (const marker of [`trace_tauri_timeline_command(\"submit\", \"${kind}\"`, `trace_tauri_timeline_command_elapsed(\n        \"done\",\n        \"${kind}\"`]) if (!body?.includes(marker)) failures.push(sourceContractFailure(rule, `${command} lacks ${kind} trace`));
  }
  for (const registration of ["commands::timeline::send_reaction", "commands::timeline::redact_reaction"]) {
    if (!libSource.includes(registration)) failures.push(sourceContractFailure(rule, `timeline signal registration is missing ${registration}`));
  }
  return failures;
}

export function checkDesktopScheduledSendCommandContract() {
  const rule = "desktop.timeline.scheduled_send_contract";
  const source = readTauriSource("commands/timeline.rs");
  const libSource = productionOnly(readTauriSource("lib.rs"), "apps/desktop/src-tauri/src/lib.rs");
  const failures = [];
  for (const [command, builder, registration] of [["pub async fn schedule_send", "build_schedule_send_command", "commands::timeline::schedule_send"], ["pub async fn cancel_scheduled_send", "build_cancel_scheduled_send_command", "commands::timeline::cancel_scheduled_send"], ["pub async fn reschedule_scheduled_send", "build_reschedule_scheduled_send_command", "commands::timeline::reschedule_scheduled_send"]]) if (!source.includes(command) || !source.includes(builder) || !libSource.includes(registration)) failures.push(sourceContractFailure(rule, `missing scheduled-send contract for ${command}`));
  return failures;
}

export function checkDesktopSendQueueCommandContract() {
  const rule = "desktop.timeline.send_queue_contract";
  const source = readTauriSource("commands/timeline.rs");
  const libSource = productionOnly(readTauriSource("lib.rs"), "apps/desktop/src-tauri/src/lib.rs");
  const failures = [];
  for (const [command, builder, registration] of [["pub async fn retry_send", "build_retry_send_command", "commands::timeline::retry_send"], ["pub async fn cancel_send", "build_cancel_send_command", "commands::timeline::cancel_send"]]) if (!source.includes(command) || !source.includes(builder) || !libSource.includes(registration)) failures.push(sourceContractFailure(rule, `missing send-queue contract for ${command}`));
  return failures;
}

export function checkDesktopForwarderLagRecoveryContract() {
  const rule = "desktop.forwarder.lag_recovery_contract";
  const forwarder = productionOnly(readTauriSource("core_event_forwarder.rs"), "apps/desktop/src-tauri/src/core_event_forwarder.rs");
  const root = productionOnly(readTauriSource("lib.rs"), "apps/desktop/src-tauri/src/lib.rs");
  const lag = sourceSection(forwarder, "Err(lag)", "Ok(event)") ?? sourceSection(forwarder, "Err(lag)");
  const failures = [];
  for (const marker of ["TimelineCommand::ReplaySubscribed", "struct CoreEventForwarderTask"]) if (!forwarder.includes(marker)) failures.push(sourceContractFailure(rule, `forwarder lacks ${marker}`));
  if (!root.includes("forwarder_task: Some")) failures.push(sourceContractFailure(rule, "lib.rs does not retain the forwarder task"));
  if (forwarder.includes("Box::leak")) failures.push(sourceContractFailure(rule, "forwarder counter is leaked"));
  for (const marker of ["event_conn.command_handle()", "event_conn.next_request_id()", "emit_forwarded_webview_events", "submit_timeline_replay_after_forwarder_lag"]) if (!lag?.includes(marker)) failures.push(sourceContractFailure(rule, `lag recovery lacks ${marker}`));
  if (lag?.includes("async_runtime::spawn")) failures.push(sourceContractFailure(rule, "lag replay is detached"));
  failures.push(...orderedMarkers(rule, lag ?? "", ["emit_forwarded_webview_events", "submit_timeline_replay_after_forwarder_lag"]));
  return failures;
}

export function checkDesktopQaControlPipeContract() {
  const rule = "desktop.native.qa_control_pipe_cfg";
  const source = productionOnly(readTauriSource("lib.rs"), "apps/desktop/src-tauri/src/lib.rs");
  const failures = [];
  for (const token of ["const QA_CONTROL_PIPE_ENV", "fn qa_control_pipe_path_from_env()", "spawn_qa_control_pipe_reader"]) {
    const offset = source.indexOf(token);
    const gate = source.lastIndexOf("#[cfg(any(debug_assertions, test))]", offset);
    if (offset < 0 || gate < 0 || source.slice(gate, offset).includes("\n\n")) failures.push(sourceContractFailure(rule, `control-pipe item is not directly debug/test gated`));
  }
  if (source.split("std::env::var(QA_CONTROL_PIPE_ENV)").length - 1 !== 1) failures.push(sourceContractFailure(rule, "control-pipe env is read more than once"));
  return failures;
}

export function checkDesktopNativeWindowLifecycleContract() {
  const rule = "desktop.native.window_lifecycle_contract";
  const source = productionOnly(readTauriSource("lib.rs"), "apps/desktop/src-tauri/src/lib.rs");
  const failures = [];
  // A destroyed product window means the process is ending, so it must enter
  // the shared quit barrier rather than submit a shutdown of its own ahead of
  // the `ExitRequested` that follows it.
  const destroyed = sourceSection(source, "if window_event_should_stop_background_tasks(event)", ".invoke_handler");
  for (const marker of ["claim_core_shutdown", "begin_graceful_shutdown"]) if (!destroyed?.includes(marker)) failures.push(sourceContractFailure(rule, `window destruction path lacks ${marker}`));
  failures.push(...orderedMarkers(rule, destroyed ?? "", ["claim_core_shutdown", "begin_graceful_shutdown"]));
  if (destroyed?.includes("AppCommand::Shutdown {")) failures.push(sourceContractFailure(rule, "window destruction path submits shutdown outside the quit barrier"));
  // Exactly one owner submits the shutdown, awaits it, and only then exits.
  const shutdown = rustItemBody(source, "fn begin_graceful_shutdown");
  for (const marker of ["AppCommand::Shutdown { request_id }", "QuitStage::ShutdownComplete", "app.exit(0)"]) if (!shutdown?.includes(marker)) failures.push(sourceContractFailure(rule, `graceful shutdown lacks ${marker}`));
  failures.push(...orderedMarkers(rule, shutdown ?? "", ["AppCommand::Shutdown { request_id }", "QuitStage::ShutdownComplete", "app.exit(0)"]));
  if (source.split("AppCommand::Shutdown { request_id }").length - 1 !== 1) failures.push(sourceContractFailure(rule, "shutdown is submitted from more than one place"));
  // The barrier holds every exit request, whatever its code, until the single
  // claimant has finished shutting core down.
  const barrier = sourceSection(source, "tauri::RunEvent::ExitRequested", "QuitRequestAction::Exit");
  for (const marker of ["{ api, .. }", "quit_request_action", "QuitRequestAction::AwaitShutdown"]) if (!barrier?.includes(marker)) failures.push(sourceContractFailure(rule, `exit barrier lacks ${marker}`));
  failures.push(...orderedMarkers(rule, barrier ?? "", ["api.prevent_exit()", "claim_core_shutdown", "begin_graceful_shutdown"]));
  const claim = rustItemBody(source, "fn claim_core_shutdown");
  for (const marker of ["compare_exchange", "QuitStage::Idle.repr()", "QuitStage::ShuttingDown.repr()"]) if (!claim?.includes(marker)) failures.push(sourceContractFailure(rule, `shutdown claim lacks ${marker}`));
  const helper = rustItemBody(source, "fn window_event_should_stop_background_tasks");
  if (helper?.includes("CloseRequested")) failures.push(sourceContractFailure(rule, "close request stops background tasks"));
  // The section spans both close-request branches: it opens at the macOS
  // unconditional hide and closes at the shared persistence path.
  const close = sourceSection(source, "tauri::WindowEvent::CloseRequested", "if window_event_should_persist");
  for (const marker of ["prevent_close()", ".hide()", "window.is_fullscreen()", "window.set_fullscreen(false)", "= macos_close_requested_action(", "= close_requested_action(tray::tray_is_available(), close_to_tray)", "close_to_tray", "CloseRequestedAction::HideToTray"]) if (!close?.includes(marker)) failures.push(sourceContractFailure(rule, `close handler lacks ${marker}`));
  failures.push(...orderedMarkers(rule, close ?? "", ["window.set_fullscreen(false)", "window.hide()"]));
  if (close?.includes("AppCommand::Shutdown {")) failures.push(sourceContractFailure(rule, "close request submits shutdown"));
  return failures;
}

export function checkDesktopNativeReopenContract() {
  const rule = "desktop.native.reopen_contract";
  const source = productionOnly(readTauriSource("lib.rs"), "apps/desktop/src-tauri/src/lib.rs");
  const callback = sourceSection(source, "tauri_plugin_single_instance::init(", ".plugin(tauri_plugin_deep_link::init())");
  const run = sourceSection(source, "pub fn run()", "#[cfg(test)]");
  const failures = [];
  for (const marker of ["ensure_main_window_visible_for_handle", "desktop.lifecycle", "reopen_requested"]) if (!callback?.includes(marker)) failures.push(sourceContractFailure(rule, `single-instance callback lacks ${marker}`));
  for (const marker of [".build(tauri::generate_context!())", "tauri::RunEvent::Reopen", "ensure_main_window_visible_for_handle", "desktop.lifecycle", "reopen_requested"]) if (!run?.includes(marker)) failures.push(sourceContractFailure(rule, `run reopen path lacks ${marker}`));
  return failures;
}

export function checkDesktopViewportAdapterIsolationContract() {
  const rule = "desktop.viewport.native_adapter_isolation";
  const source = productionOnly(readTauriSource("viewport_sync.rs"), "apps/desktop/src-tauri/src/viewport_sync.rs");
  const failures = [];
  if (!source.includes("synchronize_now")) failures.push(sourceContractFailure(rule, "native adapter lacks synchronize_now"));
  for (const marker of ["set_size", "dispatchEvent"]) if (source.includes(marker)) failures.push(sourceContractFailure(rule, `native adapter contains forbidden ${marker}`));
  return failures;
}

function rustItemBody(source, marker) {
  const start = source.indexOf(marker);
  if (start < 0) return null;
  const tokens = lexRust(source);
  const pairs = delimiterPairs(tokens);
  const open = tokens.findIndex((token) => token.start >= start && token.kind === "punctuation" && token.value === "{");
  if (open < 0) return null;
  const close = pairs.openToClose.get(open);
  return close === undefined ? null : source.slice(start, tokens[close].end);
}

function sourceContractFailure(rule, message) {
  return { kind: "source-contract", rule, message };
}

// Issue #753 source-contract migration ledger.  Counts are structural
// assertions only; mixed tests retain their behavioral assertions in Rust.
// Old fully-qualified test identity | old assertions | replacement rule(s) | replacement assertions
// koushi_core::runtime::tests::role_command_reduces_pending_before_one_account_route | 2 | core.runtime.role_command_pending_route | 2
// koushi_core::runtime::tests::activity_mark_read_routes_persistent_room_mark_read_commands | 3 | core.runtime.activity_mark_read_route | 3
// koushi_core::runtime::tests::open_thread_command_must_execute_thread_timeline_effects | 2 | core.runtime.thread_effect_execution | 2
// koushi_core::runtime::tests::runtime_must_execute_start_sync_effects_from_session_reducer | 2 | core.runtime.start_sync_effect_execution | 2
// koushi_core::runtime::tests::runtime_must_execute_session_cleanup_effects_from_session_reducer | 2 | core.runtime.session_cleanup_effect_execution | 2
// koushi_core::runtime::tests::runtime_routes_current_device_trust_rechecks_in_both_effect_lanes | 1 | core.runtime.trust_recheck_effect_execution | 1
// koushi_core::runtime::tests::runtime_routes_current_session_status_in_both_effect_lanes | 1 | core.runtime.session_status_effect_execution | 1
// koushi_core::runtime::tests::app_actor_persistence_uses_blocking_store_port | 3 | core.runtime.persistence_blocking_port | 3
// koushi_core::runtime::tests::runtime_must_execute_subscribe_timeline_effects_from_navigation_reducers | 2 | core.runtime.subscribe_timeline_effect | 2
// koushi_core::runtime::tests::runtime_room_selection_replays_existing_room_timeline_for_empty_renderer_store | 3 | core.runtime.navigation_replay | 3
// koushi_core::runtime::tests::closed_account_actor_timeline_route_is_not_reported_as_queue_overflow | 2 | core.runtime.closed_timeline_route | 2
// koushi_core::runtime::tests::actor_projection_start_sync_effects_must_not_be_discarded | 1 | core.runtime.actor_start_sync_effect | 1
// koushi_core::runtime::tests::runtime_sync_trace_covers_start_sync_effect_boundaries | 2 | core.runtime.sync_trace | 2
// koushi_core::runtime::tests::opening_a_replacement_thread_unsubscribes_the_previous_thread_before_subscribe | 1 | core.runtime.thread_replacement | 1
// koushi_core::runtime::tests::opening_a_replacement_focused_context_unsubscribes_previous_focused_before_subscribe | 1 | core.runtime.focused_replacement | 1
// koushi_core::runtime::tests::opening_focused_context_repairs_target_event_cache_before_subscribe | 1 | core.runtime.focused_cache_repair | 1
// koushi_core::runtime::tests::selecting_a_replacement_room_cancels_previous_room_pagination_before_subscribe | 2 | core.runtime.room_switch_pagination | 2
// koushi_core::runtime::tests::selecting_a_replacement_room_cancels_previous_room_link_previews_before_subscribe | 2 | core.runtime.room_switch_link_previews | 2
// koushi_core::runtime::tests::timestamp_jump_uses_local_activity_projection_before_homeserver_fallback | 2 | core.runtime.timestamp_activity_projection | 2
// koushi_core::runtime::connection::tests::core_connection_command_handle_clones_submit_path | 4 | core.runtime.connection_command_handle | 8
// koushi_core::executor::tests::executor_exposes_blocking_task_port | 2 | core.runtime.executor_blocking_port | 2
// koushi_core::renderable_thumbnail::tests::avatar_and_preview_thumbnail_helpers_do_not_use_legacy_plaintext_paths | 8 | core.runtime.thumbnail_paths | 8
// koushi_core::search::tests::search_query_failures_are_classified_from_sdk_error | 3 | core.search.query_failure_classification | 3
// koushi_core::search::tests::search_actor_handles_new_queries_before_crawl_and_sdk_completions | 8 | core.search.query_priority | 8
// koushi_core::search::tests::empty_query_is_not_special_cased_in_runtime | 4 | core.search.empty_query_ownership | 4
// koushi_core::search::tests::search_actor_crawler_uses_element_style_round_robin_checkpoints | 3 | core.search.crawler_round_robin | 3
// koushi_core::search::tests::search_actor_prunes_crawler_queue_when_joined_rooms_change | 2 | core.search.crawler_pruning | 2
// koushi_core::search::tests::search_actor_history_crawler_uses_account_wide_account_work | 2 | core.search.crawler_account_work | 2
// koushi_core::search::tests::search_actor_room_availability_notifications_have_nonblocking_entrypoint | 3 | core.search.availability_nonblocking | 3
// koushi_core::search::tests::search_crawler_lifecycle_projects_actor_owned_stop_settles | 4 | core.search.crawler_lifecycle | 4
// koushi_core::search::tests::preempted_crawl_page_is_requeued | 2 | core.search.preempted_page_requeue | 2
// koushi_core::search::tests::automatic_crawl_starts_are_delayed_at_startup | 3 | core.search.startup_delay | 3
// koushi_core::search_crawler::tests::history_crawler_page_runner_fetches_only_one_messages_page | 2 | core.search.page_single_fetch | 2
// koushi_core::search_crawler::tests::history_crawler_page_runner_acquires_the_search_crawl_work_kind | 1 | core.search.page_work_kind | 1
// koushi_core::search_crawler::tests::crawler_page_emits_startup_trace | 1 | core.search.page_startup_trace | 1
// koushi_core::search_crawler::tests::crawler_page_yields_to_timeline_via_cancellation | 3 | core.search.page_cancellation | 3
// koushi_core::send_diagnostics::tests::distinguishes_http_timeouts_without_exposing_transport_details | 1 | core.runtime.send_http_timeout | 1 (mixed; behavioral assertions retained)
// koushi_core::sync::tests::sync_service_has_one_all_rooms_owner | 7 | core.sync.single_all_rooms_owner | 7
// koushi_core::sync::tests::running_state_is_not_the_committed_response_handoff | 2 | core.sync.committed_response_handoff | 2
// koushi_core::sync::tests::latest_observed_commit_is_forwarded_to_timeline_before_range_readiness | 4 | core.sync.timeline_commit_before_readiness | 4
// koushi_core::sync::tests::terminated_sync_owner_is_restarted_instead_of_settled_failed | 3 | core.sync.terminated_owner_restart | 3
// koushi_core::threads_list::tests::aggregate_refresh_has_production_manager_start_and_finish_callers | 5 | core.threads.aggregate_refresh_callers | 5
// koushi_core::threads_list::tests::thread_root_projection_source_never_uses_room_pagination_or_anchor_materialization | 1 | core.threads.root_projection_no_pagination | 3
// koushi_core::threads_list::tests::open_subscription_loads_initial_page_before_emitting_opened | 1 | core.threads.open_subscription_initial_page | 1
// koushi_core::threads_list::tests::paginate_updates_are_correlated_to_paginate_request_id | 4 | core.threads.pagination_request_correlation | 4
// koushi_core::threads_list::tests::thread_list_relays_are_reliable_and_paginate_errors_fail | 8 | core.threads.reliable_relays | 9
// koushi_core::room::actor::tests::room_actor_command_loop_never_awaits_room_list_refresh | 1 | core.room.actor_command_loop | 1
// koushi_core::room::actor::tests::sync_started_requires_one_live_room_list_service | 5 | core.room.sync_started_owner | 5
// koushi_core::room::directory::tests::directory_join_selects_room_before_room_joined_event_is_emitted | 1 | core.room.directory_join_order | 1
// koushi_core::room::list_observer::tests::live_direct_observer_subscribes_before_cached_account_data_read | 1 | core.room.live_direct_subscription_order | 1
// koushi_core::room::list_observer::tests::room_list_runtime_has_no_legacy_or_base_client_projection_path | 1 (5 predicates) | core.room.list_no_legacy_projection | 5
// koushi_core::room::list_observer::tests::room_list_observation_relays_parent_only_space_links_before_projection | 2 | core.room.list_relay_order | 2
// koushi_core::room::list_observer::tests::room_list_projection_updates_known_book_before_reliable_delivery | 2 | core.room.list_known_book_delivery | 2
// koushi_core::room::mentions::tests::existing_membership_change_message_routes_to_space_refresh | 1 | core.room.mention_membership_refresh | 1
// koushi_core::room::operations::tests::mark_room_as_read_success_updates_fully_read_marker_before_clearing_counts | 3 | core.room.mark_read_order | 3
// koushi_core::room::operations::tests::room_tag_success_path_does_not_refresh_from_stale_sdk_snapshot | 2 | core.room.tag_no_stale_refresh | 2
// koushi_core::room::operations::tests::create_room_links_parent_space_child_with_created_room_id_before_completion_event | 2 | core.room.create_links_before_completion | 2
// koushi_core::room::operations::tests::missing_space_child_repairs_are_actor_owned_and_retryable | 3 | core.room.missing_space_child_repair | 3
// koushi_core::room::pins::tests::pin_success_settles_pending_before_pinned_projection_reload | 4 | core.room.pin_settlement_order | 4
// koushi_core::room::pins::tests::pin_and_unpin_commands_require_actor_known_room_guard_before_sdk_call | 2 | core.room.pin_command_guard | 2
// koushi_core::room::space_members::tests::space_member_load_failure_does_not_construct_an_empty_projection | 2 | core.room.space_member_failure_projection | 2
// koushi_core::room::space_members::tests::background_space_member_lookup_failure_preserves_state_and_only_records_diagnostic | 3 | core.room.space_member_background_failure | 3
// koushi_core::room::space_members::tests::cancel_space_invite_reconciles_a_fresh_projection_before_settling | 3 | core.room.space_invite_cancellation_order | 3
// koushi_core::store::credential_backend::tests::file_credential_store_is_available_to_release_qa_binary_only | 2 | core.store.file_credential_cfg | 2 (vacuous self-matches in both old assertions)
// runtime_intent_lifecycle::select_room_routing_is_reliable_and_correlated | 3 | core.integration.select_room_routing | 4
// runtime_room_list_sync::production_runtime_requires_committed_all_rooms_readiness | 6 | core.integration.room_list_readiness | 6
// runtime_room_list_sync::production_core_has_no_legacy_or_mode_transition_vocabulary | 1 (7 predicates) | core.integration.no_legacy_mode_vocabulary | 7
// runtime_timeline::production_timeline_has_no_classic_sync_or_legacy_checkpoint_path | 1 | core.integration.timeline_no_legacy_checkpoint | 4
// send_queue_fast::fast_send_queue_lane_hard_bounds_generic_lifecycle_phases | 23 | core.qa.fast_send_queue_lifecycle | 23
// send_queue_fast::send_queue_stage_uses_exact_causal_waiter_for_both_subscriptions | 1 | core.qa.send_queue_causal_waiter | 1
// send_queue_fast::headless_send_queue_diagnostic_contract_counts_forwarded_and_completed_room_sends | 10 | core.qa.send_queue_diagnostic_counters | 10
// send_queue_fast::headless_send_queue_diagnostic_contract_wraps_fifo_failure_with_proxy_deltas | 6 | core.qa.send_queue_proxy_deltas | 6
// send_queue_fast::headless_send_queue_diagnostic_contract_arms_before_private_safe_not_sent_failure | 15 | core.qa.send_queue_private_safe_failure | 15
// send_queue_fast::fast_send_queue_restored_completion_cannot_finish_from_send_completed_alone | 2 | core.integration.fast_send_queue_completion | 2

export function checkStateFocusedContextReducerContract() {
  const rule = "state.focused_context_reducer_contract";
  const source = readRustSource("crates/koushi-state/src/reducer/mod.rs") + readRustSource("crates/koushi-state/src/reducer/thread.rs");
  const failures = [];
  for (const fragment of ["OpenFocusedContext", "FocusedContextSubscribed", "CloseFocusedContext", "OpenFocusedTimeline"]) {
    if (!source.includes(fragment)) failures.push(sourceContractFailure(rule, `missing focused-context reducer marker ${fragment}`));
  }
  return failures;
}

export function checkStateHasNoLegacySyncModeVocabulary() {
  const rule = "state.no_legacy_sync_mode_vocabulary";
  const source = [
    "crates/koushi-state/src/state/sync.rs",
    "crates/koushi-state/src/state/mod.rs",
    "crates/koushi-state/src/action.rs",
    "crates/koushi-state/src/effect.rs",
    "crates/koushi-state/src/reducer/sync.rs",
    "crates/koushi-state/src/reducer/mod.rs"
  ].map(readRustSource).join("\n");
  const failures = [];
  for (const fragment of ["SyncMode", "SyncModeFailureKind", "SyncModeChanged", "sync_mode", "LegacySync", "Transitioning"]) {
    if (source.includes(fragment)) failures.push(sourceContractFailure(rule, `forbidden sync vocabulary remains: ${fragment}`));
  }
  return failures;
}

export function checkSdkPasswordSmokeRuntimeSafety() {
  const rule = "sdk.password_smoke_runtime_safety";
  const source = readRustSource("crates/koushi-sdk/src/bin/password-login-smoke.rs");
  const failures = [];
  if (source.includes("fn restore_session_with_store_blocking(")) failures.push(sourceContractFailure(rule, "store-backed restore uses a blocking helper"));
  if (!source.includes("runtime.enter()")) failures.push(sourceContractFailure(rule, "store-backed session drop does not enter its runtime"));
  if (!source.includes("session.take()")) failures.push(sourceContractFailure(rule, "store-backed session drop does not take the session"));
  return failures;
}

export function checkSdkClientStoreConfigContract() {
  const rule = "sdk.client_store_config_contract";
  const source = readRustSource("crates/koushi-sdk/src/client_session.rs");
  const config = rustItemBody(source, "impl MatrixClientStoreConfig");
  const apply = rustItemBody(source, "fn apply_to_builder");
  const failures = [];
  if (!config?.includes("fn apply_to_builder")) failures.push(sourceContractFailure(rule, "MatrixClientStoreConfig must keep apply_to_builder"));
  if (!apply?.includes(".key(Some(self.key.expose_key()))")) failures.push(sourceContractFailure(rule, "apply_to_builder must pass the required store key"));
  if (!apply?.includes(".pool_max_size(DESKTOP_SQLITE_STORE_POOL_MAX_SIZE)")) failures.push(sourceContractFailure(rule, "apply_to_builder must cap the SDK SQLite pool"));
  return failures;
}

export function checkSdkDesktopClientBuilderDefaults() {
  const rule = "sdk.desktop_client_builder_defaults";
  const body = rustItemBody(readRustSource("crates/koushi-sdk/src/client_session.rs"), "fn desktop_client_builder_defaults");
  const failures = [];
  for (const fragment of ["with_threading_support", "ThreadingSupport::Enabled", "with_subscriptions: true", "with_enable_share_history_on_invite(true)"]) {
    if (!body?.includes(fragment)) failures.push(sourceContractFailure(rule, `desktop builder default is missing ${fragment}`));
  }
  return failures;
}

export function checkSdkBackupDownloadDefault() {
  const rule = "sdk.backup_download_default";
  const body = rustItemBody(readRustSource("crates/koushi-sdk/src/client_session.rs"), "fn desktop_client_builder_defaults");
  const failures = [];
  for (const fragment of ["with_encryption_settings", "BackupDownloadStrategy::AfterDecryptionFailure"]) {
    if (!body?.includes(fragment)) failures.push(sourceContractFailure(rule, `desktop builder default is missing ${fragment}`));
  }
  return failures;
}

export function checkSdkRecoveryUsesSdkSignaturePublication() {
  const rule = "sdk.recovery.uses_sdk_signature_publication";
  const body = rustItemBody(readRustSource("crates/koushi-sdk/src/e2ee.rs"), "pub async fn recover_e2ee");
  const failures = [];
  for (const fragment of ["prepare_current_device_registration", "force_upload_device_keys", ".recover(request.secret.expose_secret())", "republish_current_device_keys_after_recovery", "post_recovery_device_republish"]) {
    if (body?.includes(fragment)) failures.push(sourceContractFailure(rule, `recovery contains forbidden out-of-band publication ${fragment}`));
  }
  for (const fragment of ["recover_and_fix_backup", "get_own_device", "post_recovery_own_device_inspected", "inspect_current_device_signature_state", "is_cross_signed_by_owner", "record_recovery_verification_event"]) {
    if (!body?.includes(fragment)) failures.push(sourceContractFailure(rule, `recovery is missing ${fragment}`));
  }
  return failures;
}

export function checkSdkRecoverySignatureRoundTripContract() {
  const rule = "sdk.recovery.signature_round_trip_contract";
  const devices = readRustSource("vendor/matrix-rust-sdk/crates/matrix-sdk/src/encryption/identities/devices.rs");
  const secretStore = readRustSource("vendor/matrix-rust-sdk/crates/matrix-sdk/src/encryption/secret_storage/secret_store.rs");
  const failures = [];
  if (!devices.includes("verify_with_diagnostics")) failures.push(sourceContractFailure(rule, "the SDK device target lacks diagnostic verification"));
  for (const fragment of ["standard_signature_round_trip_finished", "preupload_self_signing_signature_valid", "signed_content_matches_refreshed", "self_signing_key_id_matches_refreshed", "preupload_signature_matches_refreshed", "preupload_signature_valid_with_refreshed_key"]) {
    if (!secretStore.includes(fragment)) failures.push(sourceContractFailure(rule, `secret-storage recovery diagnostics are missing ${fragment}`));
  }
  if (secretStore.includes("preupload_signature_value")) failures.push(sourceContractFailure(rule, "secret-storage diagnostics expose a raw signature value"));
  return failures;
}

export function checkSdkRoomReadMarkerContract() {
  const rule = "sdk.room_read_marker_contract";
  const body = rustItemBody(readRustSource("crates/koushi-sdk/src/room_operations.rs"), "pub async fn mark_room_as_read");
  const failures = [];
  for (const fragment of ["send_multiple_receipts", "fully_read_marker", "private_read_receipt"]) {
    if (!body?.includes(fragment)) failures.push(sourceContractFailure(rule, `mark_room_as_read is missing ${fragment}`));
  }
  if (body?.includes("send_single_receipt(ReceiptType::FullyRead")) failures.push(sourceContractFailure(rule, "mark_room_as_read sends a standalone fully-read receipt"));
  return failures;
}

export function checkSdkSpaceInviteCancellationContract() {
  const rule = "sdk.space_invite_cancellation_contract";
  const body = rustItemBody(readRustSource("crates/koushi-sdk/src/room_operations.rs"), "pub async fn cancel_space_invite");
  const failures = [];
  const markers = [
    "members_no_sync(matrix_sdk::RoomMemberships::INVITE)",
    "MatrixSpaceInviteCancellationOutcome::NotInvited",
    ".kick_user(",
    "MatrixSpaceInviteCancellationOutcome::Cancelled"
  ];
  const positions = markers.map((marker) => body?.indexOf(marker) ?? -1);
  if (positions.some((position) => position < 0)) failures.push(sourceContractFailure(rule, "invite cancellation is missing its membership, no-op, kick, or success marker"));
  if (positions[0] >= 0 && positions[1] >= 0 && positions[0] >= positions[1]) failures.push(sourceContractFailure(rule, "invite membership is checked after the no-op outcome"));
  if (positions[1] >= 0 && positions[2] >= 0 && positions[1] >= positions[2]) failures.push(sourceContractFailure(rule, "invite cancellation kicks before the no-op outcome"));
  return failures;
}

export function checkSdkRoomTagMethods() {
  const rule = "sdk.room_tag_methods";
  const source = readRustSource("crates/koushi-sdk/src/room_operations.rs");
  const failures = [];
  for (const fragment of ["set_is_favourite(true", "set_is_favourite(false", "set_is_low_priority(true", "set_is_low_priority(false"]) {
    if (!source.includes(fragment)) failures.push(sourceContractFailure(rule, `room tag operation is missing ${fragment}`));
  }
  return failures;
}

export function checkSdkPinnedEventMethods() {
  const rule = "sdk.pinned_event_methods";
  const source = readRustSource("crates/koushi-sdk/src/room_operations.rs");
  const pin = rustItemBody(source, "pub async fn pin_event");
  const unpin = rustItemBody(source, "pub async fn unpin_event");
  const failures = [];
  if (!pin?.includes(".pin_event(&event_id)")) failures.push(sourceContractFailure(rule, "pin_event does not call the SDK pin method"));
  if (!unpin?.includes(".unpin_event(&event_id)")) failures.push(sourceContractFailure(rule, "unpin_event does not call the SDK unpin method"));
  return failures;
}

export function checkSdkRoomManagementMethods() {
  const rule = "sdk.room_management_methods";
  const source = readRustSource("crates/koushi-sdk/src/room_operations.rs");
  const failures = [];
  for (const fragment of [".set_name(", ".set_room_topic(", ".set_avatar_url(", ".remove_avatar(", ".privacy_settings()", ".update_join_rule(", ".update_room_history_visibility(", ".kick_user(", ".ban_user(", ".unban_user(", ".update_power_levels("]) {
    if (!source.includes(fragment)) failures.push(sourceContractFailure(rule, `room management is missing ${fragment}`));
  }
  return failures;
}

export function checkSdkJoinedRoomListDirectDetection() {
  const rule = "sdk.room_projection.async_direct_detection";
  const body = rustItemBody(readRustSource("crates/koushi-sdk/src/room_projection.rs"), "async fn matrix_room_list_snapshot_from_rooms");
  const failures = [];
  for (const fragment of ["room.is_direct().await", "unwrap_or_else(|_| room.is_dm())"]) {
    if (!body?.includes(fragment)) failures.push(sourceContractFailure(rule, `joined room projection is missing ${fragment}`));
  }
  return failures;
}

export function checkSdkJoinedRoomListAvoidsFullMemberScans() {
  const rule = "sdk.room_projection.no_full_member_scan";
  const body = rustItemBody(readRustSource("crates/koushi-sdk/src/room_projection.rs"), "async fn matrix_room_list_snapshot_from_rooms");
  const failures = [];
  for (const fragment of ["room.joined_members_count()", "matrix_space_member_user_ids_no_sync(&room).await"]) {
    if (!body?.includes(fragment)) failures.push(sourceContractFailure(rule, `room-list projection is missing ${fragment}`));
  }
  for (const fragment of ["collect_active_member_profiles", "room.members(matrix_sdk::RoomMemberships::ACTIVE)", "joined_user_ids"]) {
    if (body?.includes(fragment)) failures.push(sourceContractFailure(rule, `room-list projection contains forbidden full-member path ${fragment}`));
  }
  return failures;
}

export function checkSdkDmResolutionCandidates() {
  const rule = "sdk.room_projection.dm_resolution_candidates";
  const body = rustItemBody(readRustSource("crates/koushi-sdk/src/room_projection.rs"), "async fn matrix_room_list_dm_user_ids");
  const failures = [];
  for (const fragment of ["direct_targets_by_room.get(&room_id)", ".direct_targets()", "room.heroes()", "get_member_no_sync", "dm_user_ids.push(candidate_user_id_string.clone())"]) {
    if (!body?.includes(fragment)) failures.push(sourceContractFailure(rule, `DM resolution is missing ${fragment}`));
  }
  for (const fragment of ["room.members(matrix_sdk::RoomMemberships::ACTIVE)", "room.members_no_sync(matrix_sdk::RoomMemberships::ACTIVE)"]) {
    if (body?.includes(fragment)) failures.push(sourceContractFailure(rule, `DM resolution contains forbidden full-member path ${fragment}`));
  }
  return failures;
}

export function checkSdkSpaceMemberIdsNoSync() {
  const rule = "sdk.room_projection.space_member_ids_no_sync";
  const body = rustItemBody(readRustSource("crates/koushi-sdk/src/room_projection.rs"), "async fn matrix_space_member_user_ids_no_sync");
  const failures = [];
  if (!body?.includes("members_no_sync(matrix_sdk::RoomMemberships::JOIN)")) failures.push(sourceContractFailure(rule, "space membership does not use the joined no-sync view"));
  if (body?.includes("RoomMemberships::ACTIVE")) failures.push(sourceContractFailure(rule, "space membership uses the active membership view"));
  if (body?.includes("room.members(matrix_sdk::RoomMemberships::JOIN)")) failures.push(sourceContractFailure(rule, "space membership fetches the joined member list"));
  return failures;
}

export function checkSdkJoinedOnlySpaceMemberProjection() {
  const rule = "sdk.room_projection.joined_only_membership";
  const body = rustItemBody(readRustSource("crates/koushi-sdk/src/room_projection.rs"), "async fn matrix_space_members_projection");
  const failures = [];
  if (body?.includes("RoomMemberships::ACTIVE")) failures.push(sourceContractFailure(rule, "space member projection uses the active membership view"));
  for (const fragment of ["members_no_sync(matrix_sdk::RoomMemberships::JOIN)", "members_no_sync(matrix_sdk::RoomMemberships::INVITE"]) {
    if (!body?.includes(fragment)) failures.push(sourceContractFailure(rule, `space member projection is missing ${fragment}`));
  }
  return failures;
}

export function checkSdkSpaceLookupFailuresPropagate() {
  const rule = "sdk.room_projection.space_lookup_failures_propagate";
  const body = rustItemBody(readRustSource("crates/koushi-sdk/src/room_projection.rs"), "pub async fn matrix_space_members_projection");
  const failures = [];
  const joined = body?.split("let space_joined_members = match")[1]?.split("let space_invited_members")[0];
  const invited = body?.split("let space_invited_members = match")[1]?.split("let mut space_joined_by_user")[0];
  for (const lookup of [joined, invited]) {
    if (!lookup?.includes("Err(error)")) failures.push(sourceContractFailure(rule, "space lookup does not retain its structured error"));
    if (!lookup?.includes("return Err(MatrixRoomOperationError::from_sdk_error(error))")) failures.push(sourceContractFailure(rule, "space lookup does not abort on error"));
  }
  return failures;
}

export function checkSdkFailedSpaceMemberCountsUnavailable() {
  const rule = "sdk.room_projection.failed_counts_unavailable";
  const body = rustItemBody(readRustSource("crates/koushi-sdk/src/room_projection.rs"), "fn space_members_scope_diagnostic_event");
  const failures = [];
  for (const fragment of ["space_join_lookup_outcome", "space_invite_lookup_outcome", "counts_unavailable", "space_joined_lookup.observed_count()", "space_invited_lookup.observed_count()", "if let Some(count)"]) {
    if (!body?.includes(fragment)) failures.push(sourceContractFailure(rule, `space-member diagnostic is missing ${fragment}`));
  }
  return failures;
}

export function checkSdkRoomMemberSummariesUseFullMembers() {
  const rule = "sdk.room_projection.member_summaries_full_members";
  const body = rustItemBody(readRustSource("crates/koushi-sdk/src/room_projection.rs"), "async fn matrix_room_member_summaries");
  return body?.includes("room.members(matrix_sdk::RoomMemberships::ACTIVE)")
    ? []
    : [sourceContractFailure(rule, "member summaries no longer load the full active member list")];
}

export function checkSdkDirectAccountDataLoaderIsLocalOnly() {
  const rule = "sdk.room_projection.direct_account_data_local_only";
  const body = rustItemBody(readRustSource("crates/koushi-sdk/src/room_projection.rs"), "pub async fn cached_direct_account_data_targets_by_room");
  const failures = [];
  if (!body?.includes("account_data::<DirectEventContent>()")) failures.push(sourceContractFailure(rule, "direct account-data loader lacks its local account-data read"));
  if (body?.includes("fetch_account_data_static")) failures.push(sourceContractFailure(rule, "direct account-data loader fetches account data from the server"));
  return failures;
}

export function checkSdkDirectAccountDataServerFallback() {
  const rule = "sdk.room_projection.direct_account_data_server_fallback";
  const body = rustItemBody(readRustSource("crates/koushi-sdk/src/room_projection.rs"), "async fn matrix_direct_account_data_targets_by_room");
  const failures = [];
  for (const fragment of ["account_data::<DirectEventContent>()", "fetch_account_data_static::<DirectEventContent>()"]) {
    if (!body?.includes(fragment)) failures.push(sourceContractFailure(rule, `direct account-data resolution is missing ${fragment}`));
  }
  return failures;
}

export function checkSdkSlidingSyncInviteProbeContract() {
  const rule = "sdk.sync.sliding_sync_invite_probe_contract";
  const source = readRustSource("crates/koushi-sdk/src/sync.rs");
  const start = source.indexOf("pub async fn probe_sliding_sync_invite_list_support");
  const end = source.indexOf("pub fn sync_once_blocking", start);
  const implementation = start >= 0 && end >= 0 ? source.slice(start, end) : null;
  const helper = implementation?.indexOf("async fn build_sliding_sync_invite_probe_client");
  const body = helper === undefined || helper < 0 ? implementation : implementation?.slice(0, helper);
  const failures = [];
  const ordered = [
    "tokio::time::timeout(SYNC_INVITE_PROBE_TIMEOUT, async {",
    "build_sliding_sync_invite_probe_client(session).await",
    "send_sliding_sync_invite_list_probe(&probe).await"
  ].map((fragment) => body?.indexOf(fragment) ?? -1);
  if (ordered.some((position) => position < 0)) failures.push(sourceContractFailure(rule, "invite probe is missing its timeout, client, or request marker"));
  if (ordered.every((position) => position >= 0) && !(ordered[0] < ordered[1] && ordered[1] < ordered[2])) failures.push(sourceContractFailure(rule, "invite probe timeout does not enclose client setup and request"));
  for (const fragment of [".send(request)", "with_request_config", "SYNC_INVITE_PROBE_TIMEOUT", "disable_retry()"]) {
    if (!implementation?.includes(fragment)) failures.push(sourceContractFailure(rule, `invite probe is missing ${fragment}`));
  }
  for (const fragment of [".sliding_sync(", "RoomListService::"]) {
    if (implementation?.includes(fragment)) failures.push(sourceContractFailure(rule, `invite probe contains forbidden live-sync construction ${fragment}`));
  }
  return failures;
}

const sdkLibrarySourcePaths = [
  "src/auth.rs",
  "src/client_session.rs",
  "src/e2ee.rs",
  "src/lib.rs",
  "src/login_store.rs",
  "src/profile.rs",
  "src/qa_reports.rs",
  "src/room_operations.rs",
  "src/room_projection.rs",
  "src/search.rs",
  "src/sliding_sync_discovery.rs",
  "src/sync.rs",
  "src/timeline.rs"
];

export function checkSdkSessionBackupFence() {
  const rule = "sdk.sessions.no_per_send_backup_fence";
  // origin/main and this branch both have exactly three production constructors
  // setting the fence false (auth.rs twice, client_session.rs once). Older plan
  // prose said four, but the migrated executable baseline asserted three.
  const sources = sdkLibrarySourcePaths.map((relativePath) => readRustSource(`crates/koushi-sdk/${relativePath}`));
  const falseCount = sources.reduce((count, source) => count + source.split("require_secure_backup_for_encrypted_sends(false)").length - 1, 0);
  const failures = [];
  if (falseCount !== 3) failures.push(sourceContractFailure(rule, `expected three disabled per-send backup fences, found ${falseCount}`));
  if (sources.some((source) => source.includes("require_secure_backup_for_encrypted_sends(true)"))) failures.push(sourceContractFailure(rule, "a session constructor enables the per-send backup fence"));
  return failures;
}

function declaredSdkLibrarySourcePaths() {
  const source = readRustSource("crates/koushi-sdk/src/lib.rs");
  const tokens = lexRust(source);
  const pairs = delimiterPairs(tokens);
  const depths = braceDepths(tokens);
  const paths = ["src/lib.rs"];
  for (let index = 0; index < tokens.length; index += 1) {
    if (depths[index] !== 0 || tokens[index].value !== "mod" || tokens[index + 1]?.kind !== "identifier") continue;
    if (attachedAttributes(tokens, index, pairs).some(({ hasTest }) => hasTest)) continue;
    if (tokens[index + 2]?.value !== ";") continue;
    const name = tokens[index + 1].value;
    const flat = `src/${name}.rs`;
    const nested = `src/${name}/mod.rs`;
    if (fs.existsSync(path.join(repositoryRoot, "crates/koushi-sdk", flat))) paths.push(flat);
    else if (fs.existsSync(path.join(repositoryRoot, "crates/koushi-sdk", nested))) paths.push(nested);
    else paths.push(`<missing:${name}>`);
  }
  return paths.sort();
}

export function checkSdkLibrarySourceManifest() {
  const rule = "sdk.library_source_manifest";
  const paths = sdkLibrarySourcePaths.slice();
  const unique = [...new Set(paths)].sort();
  const failures = [];
  if (unique.length !== paths.length) failures.push(sourceContractFailure(rule, "SDK library source manifest contains duplicate paths"));
  if (JSON.stringify(unique) !== JSON.stringify(paths.slice().sort())) failures.push(sourceContractFailure(rule, "SDK library source manifest is not sorted completely"));
  const declared = declaredSdkLibrarySourcePaths();
  if (JSON.stringify(paths) !== JSON.stringify(declared)) {
    failures.push(sourceContractFailure(rule, `SDK library source manifest differs from lib.rs declarations: expected ${declared.length} files, found ${paths.length}`));
  }
  for (const relativePath of paths) {
    try {
      fs.readFileSync(path.join(repositoryRoot, "crates/koushi-sdk", relativePath), "utf8");
    } catch {
      failures.push(sourceContractFailure(rule, `SDK library source manifest target is missing: ${relativePath}`));
    }
  }
  return failures;
}

export function checkSdkCommittedRoomCheckpointHasNoLegacyApi() {
  const rule = "sdk.timeline.committed_room_checkpoint_no_legacy_api";
  const source = sdkLibrarySourcePaths.map((relativePath) => readRustSource(`crates/koushi-sdk/${relativePath}`)).join("\n");
  const failures = [];
  for (const fragment of ["MatrixCommittedRoomTimelineBackend", "MatrixCommittedRoomTimelineOrigin", "MatrixCommittedRoomUpdatesResponse", "from_committed_observation", "from_legacy_gap_for_testing", "from_legacy_room_absent", "is_room_absent"]) {
    if (source.includes(fragment)) failures.push(sourceContractFailure(rule, `legacy room checkpoint API remains: ${fragment}`));
  }
  return failures;
}

function readAccountSource(relativePath) {
  return readRustSource(`crates/koushi-core/src/account/${relativePath}`);
}

function accountProductionSource(relativePath) {
  const fileName = `crates/koushi-core/src/account/${relativePath}`;
  return productionOnly(readRustSource(fileName), fileName);
}

function accountItemBody(relativePath, marker) {
  return rustItemBody(accountProductionSource(relativePath), marker);
}

function accountSection(relativePath, startMarker, endMarker) {
  return sourceSection(accountProductionSource(relativePath), startMarker, endMarker);
}

function coreSource(relativePath) {
  const fileName = `crates/koushi-core/src/${relativePath}`;
  return productionOnly(readRustSource(fileName), fileName);
}

function coreItemBody(relativePath, marker) {
  return rustItemBody(coreSource(relativePath), marker);
}

function protocolSource(relativePath) {
  const fileName = `crates/koushi-protocol/src/${relativePath}`;
  return productionOnly(readRustSource(fileName), fileName);
}

export function checkCoreRuntimeRoleCommandPendingRoute() {
  const rule = "core.runtime.role_command_pending_route";
  const source = coreSource("runtime.rs");
  const branch = sourceSection(source, "RoomCommand::UpdateSpaceMemberRole {\n                        request_id", "CoreCommand::Timeline(timeline_command)");
  const routeMarker = ".account_actor\n                    .send";
  const pending = branch?.indexOf("SpaceMemberRoleUpdateRequested") ?? -1;
  const route = branch?.indexOf(routeMarker) ?? -1;
  const failures = [];
  if (pending < 0 || route < 0 || pending >= route) failures.push(sourceContractFailure(rule, "role command does not project pending state before account routing"));
  if ((branch?.split(routeMarker).length ?? 1) - 1 !== 1) failures.push(sourceContractFailure(rule, "role command does not have exactly one account route"));
  return failures;
}

export function checkCoreRuntimeActivityMarkReadRoute() {
  const rule = "core.runtime.activity_mark_read_route";
  const branch = sourceSection(coreSource("runtime.rs"), "AppCommand::MarkActivityRead", "AppCommand::OpenFilesView");
  const failures = [];
  for (const marker of ["RoomCommand::MarkRoomAsRead", "next_internal_request_id", "FullyReadMarkerUpdated"]) if (!branch?.includes(marker)) failures.push(sourceContractFailure(rule, `activity mark-read route lacks ${marker}`));
  return failures;
}

export function checkCoreRuntimeThreadEffectExecution() {
  const rule = "core.runtime.thread_effect_execution";
  const branch = sourceSection(coreSource("runtime.rs"), "AppCommand::OpenThread", "AppCommand::CloseThread");
  const failures = [];
  if (branch?.includes("let _ = effects;")) failures.push(sourceContractFailure(rule, "OpenThread discards reducer effects"));
  if (!branch?.includes("handle_app_effects") && !branch?.includes("TimelineCommand::Subscribe")) failures.push(sourceContractFailure(rule, "OpenThread does not execute its timeline effect"));
  return failures;
}

export function checkCoreRuntimeStartSyncEffectExecution() {
  const rule = "core.runtime.start_sync_effect_execution";
  const body = coreItemBody("runtime.rs", "async fn handle_app_effects");
  const failures = [];
  for (const marker of ["AppEffect::StartSync", "SyncCommand::Start"]) if (!body?.includes(marker)) failures.push(sourceContractFailure(rule, `StartSync effect route lacks ${marker}`));
  return failures;
}

export function checkCoreRuntimeSessionCleanupEffectExecution() {
  const rule = "core.runtime.session_cleanup_effect_execution";
  const source = coreSource("runtime.rs");
  const command = sourceSection(source, "async fn handle_app_effects", "async fn handle_post_projection_effects");
  const actor = sourceSection(source, "async fn handle_post_projection_effects", "async fn handle_ui_event_effects");
  const failures = [];
  for (const helper of [command, actor]) {
    for (const marker of ["AppEffect::StopSync", "SyncCommand::Stop"]) if (!helper?.includes(marker)) failures.push(sourceContractFailure(rule, `session cleanup route lacks ${marker}`));
  }
  return failures;
}

export function checkCoreRuntimeTrustRecheckEffectExecution() {
  const rule = "core.runtime.trust_recheck_effect_execution";
  const source = coreSource("runtime.rs");
  const command = sourceSection(source, "async fn handle_app_effects", "async fn handle_post_projection_effects");
  const actor = sourceSection(source, "async fn handle_post_projection_effects", "async fn handle_ui_event_effects");
  const failures = [];
  for (const helper of [command, actor]) {
    const recheck = helper?.split("AppEffect::CheckCurrentDeviceTrust").at(1)?.split("AppEffect::").at(0);
    if (!recheck?.includes("AccountMessage::CheckCurrentDeviceTrust")) failures.push(sourceContractFailure(rule, "trust recheck effect does not reach AccountActor"));
  }
  return failures;
}

export function checkCoreRuntimeSessionStatusEffectExecution() {
  const rule = "core.runtime.session_status_effect_execution";
  const source = coreSource("runtime.rs");
  const command = sourceSection(source, "async fn handle_app_effects", "async fn handle_post_projection_effects");
  const actor = sourceSection(source, "async fn handle_post_projection_effects", "async fn handle_ui_event_effects");
  const failures = [];
  for (const helper of [command, actor]) {
    const refresh = helper?.split("AppEffect::RefreshCurrentSessionStatus").at(1)?.split("AppEffect::").at(0);
    if (!refresh?.includes("AccountMessage::RefreshCurrentSessionStatus")) failures.push(sourceContractFailure(rule, "session-status refresh does not reach AccountActor"));
  }
  return failures;
}

export function checkCoreRuntimePersistenceBlockingPort() {
  const rule = "core.runtime.persistence_blocking_port";
  const runtime = coreSource("runtime.rs");
  const scheduled = coreSource("runtime/scheduled_send.rs");
  const navigation = coreSource("runtime/navigation.rs");
  const composer = coreSource("runtime/composer.rs");
  const sections = [
    sourceSection(scheduled, "async fn load_scheduled_sends_for_current_session", "async fn persist_scheduled_sends"),
    sourceSection(scheduled, "async fn persist_scheduled_sends", "fn scheduled_send_delay"),
    sourceSection(runtime, "async fn persist_room_preferences", "fn next_internal_request_id"),
    sourceSection(navigation, "async fn load_navigation_for_current_session", "async fn persist_navigation"),
    sourceSection(navigation, "async fn persist_navigation", "fn current_focused_context_timeline_key"),
    sourceSection(composer, "async fn flush_pending_composer_drafts", "fn composer_draft_session_key"),
    sourceSection(runtime, "AppEffect::PersistSettings", "AppEffect::PersistRoomPreferences")
  ];
  return sections.flatMap((section) => section?.includes("executor::spawn_blocking") ? [] : [sourceContractFailure(rule, "AppActor persistence is not offloaded through the blocking executor port")]);
}

export function checkCoreRuntimeSubscribeTimelineEffect() {
  const rule = "core.runtime.subscribe_timeline_effect";
  const body = coreItemBody("runtime.rs", "async fn handle_app_effects");
  const failures = [];
  for (const marker of ["AppEffect::SubscribeTimeline", "TimelineKind::Room"]) if (!body?.includes(marker)) failures.push(sourceContractFailure(rule, `SubscribeTimeline route lacks ${marker}`));
  return failures;
}

export function checkCoreRuntimeNavigationReplay() {
  const rule = "core.runtime.navigation_replay";
  const body = coreItemBody("runtime.rs", "async fn handle_post_projection_effects");
  const failures = [];
  for (const marker of ["NavigationProjectionIntent", "admit_navigation_projection", "replay_existing: true"]) if (!body?.includes(marker)) failures.push(sourceContractFailure(rule, `navigation replay route lacks ${marker}`));
  return failures;
}

export function checkCoreRuntimeClosedTimelineRoute() {
  const rule = "core.runtime.closed_timeline_route";
  const body = sourceSection(coreSource("runtime.rs"), "async fn send_timeline_command_or_fail", "fn default_data_dir_from_home");
  const failures = [];
  if (!body?.includes("CoreFailure::ShutdownFailed")) failures.push(sourceContractFailure(rule, "closed account route lacks shutdown failure"));
  if (body?.includes("TimelineFailureKind::QueueOverflow")) failures.push(sourceContractFailure(rule, "closed account route reports queue overflow"));
  return failures;
}

export function checkCoreRuntimeActorStartSyncEffect() {
  const rule = "core.runtime.actor_start_sync_effect";
  const body = sourceSection(coreSource("runtime.rs"), "actions = self.action_rx.recv()", "command = self.command_rx.recv()");
  return body?.includes("handle_post_projection_effects") ? [] : [sourceContractFailure(rule, "actor projection actions do not execute post-projection effects")];
}

export function checkCoreRuntimeSyncTrace() {
  const rule = "core.runtime.sync_trace";
  const source = coreSource("runtime.rs");
  const command = sourceSection(source, "async fn handle_app_effects", "async fn handle_post_projection_effects")?.replace(/\s/gu, "") ?? "";
  const actor = sourceSection(source, "async fn handle_post_projection_effects", "async fn handle_ui_event_effects")?.replace(/\s/gu, "") ?? "";
  const failures = [];
  for (const [body, marker] of [[command, 'trace_runtime_sync!("effect_start_sync",[DiagnosticField::token("source","command_effect")'], [actor, 'trace_runtime_sync!("effect_start_sync",[DiagnosticField::token("source","actor_projection")']]) if (!body.includes(marker)) failures.push(sourceContractFailure(rule, "StartSync effect boundary lacks its source trace"));
  return failures;
}

export function checkCoreRuntimeThreadReplacement() {
  const rule = "core.runtime.thread_replacement";
  const branch = sourceSection(coreSource("runtime.rs"), "AppCommand::OpenThread", "AppCommand::CloseThread");
  const replacement = branch?.indexOf("unsubscribe_replaced_thread_timeline") ?? -1;
  const effects = branch?.indexOf("handle_app_effects") ?? -1;
  return replacement >= 0 && effects >= 0 && replacement < effects ? [] : [sourceContractFailure(rule, "thread replacement is not unsubscribed before the new subscription effect")];
}

export function checkCoreRuntimeFocusedReplacement() {
  const rule = "core.runtime.focused_replacement";
  const branch = sourceSection(coreSource("runtime.rs"), "AppCommand::OpenFocusedContext", "AppCommand::CloseFocusedContext");
  const replacement = branch?.indexOf("unsubscribe_replaced_focused_context_timeline") ?? -1;
  const effects = branch?.indexOf("handle_app_effects") ?? -1;
  return replacement >= 0 && effects >= 0 && replacement < effects ? [] : [sourceContractFailure(rule, "focused replacement is not unsubscribed before the new subscription effect")];
}

export function checkCoreRuntimeFocusedCacheRepair() {
  const rule = "core.runtime.focused_cache_repair";
  const branch = sourceSection(coreSource("runtime.rs"), "AppCommand::OpenFocusedContext", "AppCommand::CloseFocusedContext");
  const repair = branch?.indexOf("ensure_room_event_cached") ?? -1;
  const effects = branch?.indexOf("handle_app_effects") ?? -1;
  return repair >= 0 && effects >= 0 && repair < effects ? [] : [sourceContractFailure(rule, "focused target cache repair does not precede subscription effects")];
}

export function checkCoreRuntimeRoomSwitchPagination() {
  const rule = "core.runtime.room_switch_pagination";
  const source = coreSource("runtime.rs");
  const arm = sourceSection(source, "actions = self.action_rx.recv()", "app_loop_trace(\"action\"");
  const cancel = arm?.indexOf("cancel_replaced_room_timeline_pagination") ?? -1;
  const effects = arm?.indexOf("handle_post_projection_effects") ?? -1;
  const failures = [];
  if (cancel < 0 || effects < 0 || cancel >= effects) failures.push(sourceContractFailure(rule, "room-switch pagination cancellation does not precede replacement effects"));
  return failures;
}

export function checkCoreRuntimeRoomSwitchLinkPreviews() {
  const rule = "core.runtime.room_switch_link_previews";
  const source = coreSource("runtime.rs");
  const arm = sourceSection(source, "actions = self.action_rx.recv()", "app_loop_trace(\"action\"");
  const cancel = arm?.indexOf("cancel_replaced_room_timeline_link_previews") ?? -1;
  const effects = arm?.indexOf("handle_post_projection_effects") ?? -1;
  const failures = [];
  if (cancel < 0 || effects < 0 || cancel >= effects) failures.push(sourceContractFailure(rule, "room-switch link-preview cancellation does not precede replacement effects"));
  return failures;
}

export function checkCoreRuntimeConnectionCommandHandle() {
  const rule = "core.runtime.connection_command_handle";
  const source = coreSource("runtime/connection.rs");
  const handle = rustItemBody(source, "impl CoreCommandHandle");
  const connection = rustItemBody(source, "impl CoreConnection {");
  const commandHandle = rustItemBody(connection ?? "", "pub fn command_handle");
  const command = rustItemBody(connection ?? "", "pub async fn command");
  const failures = [];
  if (!source.includes("#[derive(Clone)]\npub struct CoreCommandHandle")) failures.push(sourceContractFailure(rule, "command handle is not cloneable"));
  for (const marker of ["self.command_tx", ".send(CoreCommandEnvelope", "command,", "composer_permit", ".await"]) if (!handle?.includes(marker)) failures.push(sourceContractFailure(rule, `command handle lacks ${marker}`));
  if (!commandHandle?.includes("command_tx: self.command_tx.clone()")) failures.push(sourceContractFailure(rule, "connection clones more than the command sender"));
  if (!command?.includes("self.command_handle().command(command).await")) failures.push(sourceContractFailure(rule, "connection command does not delegate through the handle"));
  return failures;
}

export function checkCoreRuntimeCoalescerBaseline() {
  const rule = "core.runtime.coalescer_baseline";
  const source = coreSource("runtime.rs");
  const command = sourceSection(source, "command = self.command_rx.recv()", "focused_projection = self.focused_projection_rx.recv()");
  const failures = [];
  if (!command?.includes("self.snapshot_tx.borrow().state.clone()")) failures.push(sourceContractFailure(rule, "coalescer path does not publish the latest unpublished baseline"));
  return failures;
}

export function checkCoreRuntimeTimestampActivityProjection() {
  const rule = "core.runtime.timestamp_activity_projection";
  const arm = sourceSection(coreSource("runtime.rs"), "AppCommand::OpenTimelineAtTimestamp", "AppCommand::CloseFocusedContext");
  const local = arm?.indexOf("activity_projection") ?? -1;
  const fallback = arm?.indexOf("AccountMessage::OpenTimelineAtTimestamp") ?? -1;
  const failures = [];
  if (local < 0 || fallback < 0 || local >= fallback) failures.push(sourceContractFailure(rule, "timestamp navigation does not prefer local activity before homeserver fallback"));
  if (!arm?.includes("AppAction::OpenFocusedContext")) failures.push(sourceContractFailure(rule, "local timestamp resolution does not open focused context through the reducer"));
  return failures;
}

export function checkCoreRuntimeExecutorBlockingPort() {
  const rule = "core.runtime.executor_blocking_port";
  const source = coreSource("executor.rs");
  const failures = [];
  for (const marker of ["pub fn spawn_blocking", "tokio::task::spawn_blocking"]) if (!source.includes(marker)) failures.push(sourceContractFailure(rule, `executor blocking port lacks ${marker}`));
  return failures;
}

export function checkCoreRuntimeSendHttpTimeout() {
  const rule = "core.runtime.send_http_timeout";
  const source = coreSource("send_diagnostics.rs");
  return source.includes("matrix_sdk::HttpError::Cached(error) => http_error_is_timeout(error)") ? [] : [sourceContractFailure(rule, "send diagnostics do not classify cached HTTP timeout errors")];
}

export function checkCoreRuntimeThumbnailPaths() {
  const rule = "core.runtime.thumbnail_paths";
  const account = coreItemBody("account/profile.rs", "async fn download_avatar_thumbnail");
  const preview = coreItemBody("link_preview.rs", "async fn download_preview_image");
  const failures = [];
  for (const [body, required, forbidden] of [[account, ["get_media_content", "true,"], ["avatar_thumbnails", "file://"]], [preview, ["get_media_content", "false,"], ["link_preview_thumbnails", "file://"]]]) {
    for (const marker of required) if (!body?.includes(marker)) failures.push(sourceContractFailure(rule, `thumbnail helper lacks ${marker}`));
    for (const marker of forbidden) if (body?.includes(marker)) failures.push(sourceContractFailure(rule, `thumbnail helper contains forbidden ${marker}`));
  }
  return failures;
}

export function checkCoreRoomActorCommandLoop() {
  const rule = "core.room.actor_command_loop";
  const body = coreItemBody("room/actor.rs", "async fn run");
  return body?.includes("refresh_room_list().await")
    ? [sourceContractFailure(rule, "RoomActor command handling awaits room-list normalization")]
    : body
      ? []
      : [sourceContractFailure(rule, "RoomActor command loop is missing")];
}

export function checkCoreRoomSyncStartedOwner() {
  const rule = "core.room.sync_started_owner";
  const source = coreSource("room/actor.rs");
  const variant = sourceSection(source, "SyncStarted {", "ReconcileCommittedRange");
  const syncStarted = sourceSection(source, "RoomMessage::SyncStarted", "RoomMessage::ReconcileCommittedRange");
  const failures = [];
  for (const marker of ["Arc<matrix_sdk_ui::room_list_service::RoomListService>"]) if (!variant?.includes(marker)) failures.push(sourceContractFailure(rule, `SyncStarted variant lacks ${marker}`));
  if (variant?.includes("Option<")) failures.push(sourceContractFailure(rule, "SyncStarted variant makes its room-list service optional"));
  for (const marker of ["self.clear_known_rooms();", "match room_list_service"]) if (syncStarted?.includes(marker)) failures.push(sourceContractFailure(rule, `SyncStarted body contains forbidden ${marker}`));
  if (!syncStarted?.includes("self.start_live_observation(")) failures.push(sourceContractFailure(rule, "SyncStarted body does not start live observation"));
  return failures;
}

export function checkCoreRoomDirectoryJoinOrder() {
  const rule = "core.room.directory_join_order";
  const body = coreItemBody("room/directory.rs", "async fn handle_join_directory_room");
  return orderedMarkers(rule, body ?? "", ["AppAction::DirectoryJoinSucceeded", "RoomEvent::RoomJoined"]);
}

export function checkCoreRoomLiveDirectSubscriptionOrder() {
  const rule = "core.room.live_direct_subscription_order";
  const body = coreItemBody("room/list_observer.rs", "async fn run_live_room_list_observation(");
  return orderedMarkers(rule, body ?? "", ["observe_events::<DirectEvent, ()>", "cached_direct_account_data_targets_by_room"]);
}

export function checkCoreRoomListNoLegacyProjection() {
  const rule = "core.room.list_no_legacy_projection";
  const source = coreSource("room/list_observer.rs");
  const failures = [];
  for (const marker of ["run_legacy_room_list_observation", "start_legacy_observation", "refresh_room_list_from_joined_rooms", ".joined_rooms()", ".invited_rooms()"])
    if (source.includes(marker)) failures.push(sourceContractFailure(rule, `RoomActor contains forbidden room-list path ${marker}`));
  return failures;
}

export function checkCoreRoomListRelayOrder() {
  const rule = "core.room.list_relay_order";
  const body = coreItemBody("room/list_observer.rs", "async fn normalize_and_project_entries");
  const failures = orderedMarkers(rule, body ?? "", ["relay_missing_space_child_links", "project_room_list_snapshot"]);
  if (body?.includes("koushi_sdk::set_space_child")) failures.push(sourceContractFailure(rule, "room-list observer performs the space-child write directly"));
  return failures;
}

export function checkCoreRoomListKnownBookDelivery() {
  const rule = "core.room.list_known_book_delivery";
  const body = coreItemBody("room/list_observer.rs", "async fn project_room_list_snapshot");
  const failures = orderedMarkers(rule, body ?? "", ["replace_known_room_ids", ".send(vec!["]);
  if (body?.includes("try_send(vec![")) failures.push(sourceContractFailure(rule, "room-list projection uses lossy action delivery"));
  return failures;
}

export function checkCoreRoomMentionMembershipRefresh() {
  const rule = "core.room.mention_membership_refresh";
  const body = coreItemBody("room/mentions.rs", "async fn handle_mention_membership_changed");
  return body?.includes("handle_space_membership_changed")
    ? []
    : [sourceContractFailure(rule, "mention membership changes do not refresh demanded Space members")];
}

export function checkCoreRoomMarkReadOrder() {
  const rule = "core.room.mark_read_order";
  const body = coreItemBody("room/operations.rs", "async fn handle_mark_room_as_read");
  const success = body?.split("Ok(()) => {").at(1)?.split("Err(error) => {").at(0);
  const failures = [];
  for (const marker of ["AppAction::FullyReadMarkerUpdated", "AppAction::RoomMarkedAsReadSucceeded"]) if (!success?.includes(marker)) failures.push(sourceContractFailure(rule, `mark-read success arm lacks ${marker}`));
  const marker = success?.indexOf("FullyReadMarkerUpdated") ?? -1;
  const cleared = success?.indexOf("RoomMarkedAsReadSucceeded") ?? -1;
  if (marker < 0 || cleared < 0 || marker >= cleared) failures.push(sourceContractFailure(rule, "mark-read clears counts before updating the fully-read marker"));
  return failures;
}

export function checkCoreRoomTagNoStaleRefresh() {
  const rule = "core.room.tag_no_stale_refresh";
  const failures = [];
  for (const marker of ["async fn handle_set_tag", "async fn handle_remove_tag"]) {
    const body = coreItemBody("room/operations.rs", marker);
    if (!body) failures.push(sourceContractFailure(rule, `missing ${marker}`));
    else if (body.includes("refresh_room_list().await")) failures.push(sourceContractFailure(rule, `${marker} refreshes from a stale room-list snapshot`));
  }
  return failures;
}

export function checkCoreRoomCreateLinksBeforeCompletion() {
  const rule = "core.room.create_links_before_completion";
  const body = coreItemBody("room/operations.rs", "async fn handle_create_room");
  const failures = orderedMarkers(rule, body ?? "", ["link_created_room_to_parent_space", "RoomEvent::RoomCreated"]);
  const linkHelper = coreItemBody("room/operations.rs", "async fn link_created_room_to_parent_space");
  if (linkHelper?.includes("emit_failure")) failures.push(sourceContractFailure(rule, "best-effort parent-space linking emits a room-creation failure"));
  return failures;
}

export function checkCoreRoomMissingSpaceChildRepair() {
  const rule = "core.room.missing_space_child_repair";
  const actor = coreItemBody("room/operations.rs", "async fn handle_missing_space_child_links");
  const relay = coreItemBody("room/list_observer.rs", "async fn relay_missing_space_child_links");
  const failures = [];
  if (!relay?.includes("RoomMessage::MissingSpaceChildLinks")) failures.push(sourceContractFailure(rule, "room-list observation does not relay missing links"));
  if (!actor?.includes("classify_room_error(&error)")) failures.push(sourceContractFailure(rule, "space-child repair failures are not classified"));
  const call = actor?.indexOf("koushi_sdk::set_space_child") ?? -1;
  const recorded = actor?.indexOf("attempts.insert(key)") ?? -1;
  if (call < 0 || recorded < 0 || call >= recorded) failures.push(sourceContractFailure(rule, "space-child repair records its dedupe key before the write succeeds"));
  return failures;
}

export function checkCoreRoomPinSettlementOrder() {
  const rule = "core.room.pin_settlement_order";
  const pin = coreItemBody("room/pins.rs", "async fn handle_pin_event");
  const unpin = coreItemBody("room/pins.rs", "async fn handle_unpin_event");
  const projection = coreItemBody("room/pins.rs", "async fn project_pinned_events_after_success");
  const failures = [];
  for (const [body, completion] of [[pin, "self.reduce_reliable(vec![AppAction::PinEventCompleted"], [unpin, "self.reduce_reliable(vec![AppAction::UnpinEventCompleted"]]) {
    const settled = body?.indexOf(completion) ?? -1;
    const reload = body?.indexOf("project_pinned_events_after_success") ?? -1;
    if (settled < 0 || reload < 0 || settled >= reload) failures.push(sourceContractFailure(rule, "pin completion is not reduced before pinned projection reload"));
  }
  for (const marker of ["AppAction::PinEventCompleted", "AppAction::UnpinEventCompleted"]) if (projection?.includes(marker)) failures.push(sourceContractFailure(rule, `pinned projection emits ${marker}`));
  return failures;
}

export function checkCoreRoomPinCommandGuard() {
  const rule = "core.room.pin_command_guard";
  const failures = [];
  for (const [fileMarker, sdkMarker] of [["async fn handle_pin_event", "koushi_sdk::pin_event"], ["async fn handle_unpin_event", "koushi_sdk::unpin_event"]]) {
    const body = coreItemBody("room/pins.rs", fileMarker);
    const guard = body?.indexOf("ensure_known_room_for_message_interaction") ?? -1;
    const sdk = body?.indexOf(sdkMarker) ?? -1;
    if (guard < 0 || sdk < 0 || guard >= sdk) failures.push(sourceContractFailure(rule, `${fileMarker} calls the SDK before the known-room guard`));
  }
  return failures;
}

export function checkCoreRoomSpaceMemberFailureProjection() {
  const rule = "core.room.space_member_failure_projection";
  const body = coreItemBody("room/space_members.rs", "async fn handle_load_space_members");
  const failure = body?.split("Err(error) =>").at(1)?.split("self.reduce_reliable").at(0);
  const failures = [];
  if (failure?.includes("SpaceMembersProjection {")) failures.push(sourceContractFailure(rule, "failed Space lookup fabricates an empty projection"));
  if (!failure?.includes("record_core_space_members_load_failure")) failures.push(sourceContractFailure(rule, "failed Space lookup loses unavailable-count diagnostics"));
  return failures;
}

export function checkCoreRoomSpaceMemberBackgroundFailure() {
  const rule = "core.room.space_member_background_failure";
  const body = coreItemBody("room/space_members.rs", "async fn handle_space_members_projection_refreshed");
  const failure = body?.split("Err(_error) =>").at(1);
  const failures = [];
  if (!failure?.includes("record_core_space_members_load_failure")) failures.push(sourceContractFailure(rule, "background Space lookup failure lacks diagnostics"));
  for (const marker of ["SpaceMembersBackgroundProjectionReconciled", "SpaceMembersLoadFailed"]) if (failure?.includes(marker)) failures.push(sourceContractFailure(rule, `background lookup failure emits forbidden ${marker}`));
  return failures;
}

export function checkCoreRoomSpaceInviteCancellationOrder() {
  const rule = "core.room.space_invite_cancellation_order";
  const body = coreItemBody("room/space_members.rs", "async fn handle_cancel_space_invite");
  const failures = orderedMarkers(rule, body ?? "", ["koushi_sdk::cancel_space_invite", "reconcile_space_invite_cancellation", "SpaceMemberInviteCancellationSettled"]);
  const reconciliation = coreItemBody("room/space_members.rs", "async fn reconcile_space_invite_cancellation");
  if (!reconciliation?.includes("koushi_sdk::matrix_space_members_projection")) failures.push(sourceContractFailure(rule, "invite cancellation reconciliation does not request a fresh Space projection"));
  return failures;
}

export function checkCoreStoreFileCredentialCfg() {
  const rule = "core.store.file_credential_cfg";
  const source = readRustSource("crates/koushi-store/src/credential_backend.rs");
  const failures = [];
  for (const marker of [
    'cfg(any(debug_assertions, test, feature = "test-hooks"))',
    "// --- File-based credential store (debug/test/test-hooks only) ---"
  ])
    if (!source.includes(marker)) failures.push(sourceContractFailure(rule, `file credential backend lacks ${marker}`));
  return failures;
}

export function checkCoreSearchQueryFailureClassification() {
  const rule = "core.search.query_failure_classification";
  const source = coreSource("search.rs");
  const query = sourceSection(source, "async fn handle_query", "async fn handle_index_message");
  const failures = [];
  if (!source.includes("fn classify_matrix_search_error")) failures.push(sourceContractFailure(rule, "search failure classifier is missing"));
  if (!query?.includes("classify_matrix_search_error(&error)")) failures.push(sourceContractFailure(rule, "query failures bypass SDK error classification"));
  if (query?.includes("kind: SearchFailureKind::IndexUnavailable")) failures.push(sourceContractFailure(rule, "query failures hardcode IndexUnavailable"));
  return failures;
}

export function checkCoreSearchQueryPriority() {
  const rule = "core.search.query_priority";
  const source = coreSource("search.rs");
  const run = coreItemBody("search.rs", "async fn run");
  const failures = [];
  if (!run?.includes("biased;")) failures.push(sourceContractFailure(rule, "search actor loop is not biased toward actor messages"));
  const actor = run?.indexOf("msg = self.msg_rx.recv()") ?? -1;
  for (const marker of ["crawl_result = async", "sdk_result = async"]) {
    const position = run?.indexOf(marker) ?? -1;
    if (actor < 0 || position < 0 || actor >= position) failures.push(sourceContractFailure(rule, `actor messages do not precede ${marker}`));
  }
  const query = coreItemBody("search.rs", "async fn handle_query");
  for (const marker of ["self.active_sdk_search.take()", "abort_and_await_task(task).await", "record_stale_sdk_drop"]) if (!query?.includes(marker)) failures.push(sourceContractFailure(rule, `query replacement lacks ${marker}`));
  const result = coreItemBody("search.rs", "async fn handle_sdk_query_result");
  for (const marker of ["result.generation != self.active_query_generation", "record_stale_sdk_drop"]) if (!result?.includes(marker)) failures.push(sourceContractFailure(rule, `SDK result fencing lacks ${marker}`));
  return failures;
}

export function checkCoreSearchEmptyQueryOwnership() {
  const rule = "core.search.empty_query_ownership";
  const runtime = coreSource("runtime.rs");
  const search = coreSource("search.rs");
  const failures = [];
  for (const marker of ["is_empty_query", "results: Vec::new()"]) if (runtime.includes(marker)) failures.push(sourceContractFailure(rule, `runtime owns forbidden empty-query marker ${marker}`));
  for (const marker of ["query.trim().is_empty()", "CoreEvent::Search(SearchEvent::Results"]) if (!search.includes(marker)) failures.push(sourceContractFailure(rule, `search actor lacks ${marker}`));
  return failures;
}

export function checkCoreSearchCrawlerRoundRobin() {
  const rule = "core.search.crawler_round_robin";
  const source = coreSource("search.rs");
  const failures = [];
  for (const marker of ["VecDeque<HistoryCrawlCheckpoint>", "start_next_history_crawl_page", "push_back(next_checkpoint)"]) if (!source.includes(marker)) failures.push(sourceContractFailure(rule, `crawler round-robin path lacks ${marker}`));
  return failures;
}

export function checkCoreSearchCrawlerPruning() {
  const rule = "core.search.crawler_pruning";
  const source = coreSource("search.rs");
  const failures = [];
  for (const marker of ["retain_history_crawl_rooms", "abort_active_history_crawl_if_retired"]) if (!source.includes(marker)) failures.push(sourceContractFailure(rule, `crawler pruning path lacks ${marker}`));
  return failures;
}

export function checkCoreSearchCrawlerAccountWork() {
  const rule = "core.search.crawler_account_work";
  const source = coreSource("search.rs");
  const starter = sourceSection(source, "fn start_next_history_crawl_page", "async fn handle_history_crawl_page_result");
  const failures = [];
  if (!source.includes("AccountWorkScheduler")) failures.push(sourceContractFailure(rule, "SearchActor lacks shared account work scheduler"));
  if (!starter?.includes("account_work.clone()")) failures.push(sourceContractFailure(rule, "crawler page starter does not pass shared account work"));
  return failures;
}

export function checkCoreSearchAvailabilityNonblocking() {
  const rule = "core.search.availability_nonblocking";
  const body = coreItemBody("search.rs", "impl SearchActorHandle");
  const failures = [];
  for (const marker of ["pub fn try_notify_rooms_available", ".try_send(SearchActorMessage::RoomsAvailable"]) if (!body?.includes(marker)) failures.push(sourceContractFailure(rule, `room availability path lacks ${marker}`));
  if (body?.includes("TrySendError::Closed(_)) => Ok(())")) failures.push(sourceContractFailure(rule, "closed room availability delivery is reported as success"));
  return failures;
}

export function checkCoreSearchCrawlerLifecycle() {
  const rule = "core.search.crawler_lifecycle";
  const source = coreSource("search.rs");
  const start = coreItemBody("search.rs", "async fn handle_start_history_crawl");
  const stop = coreItemBody("search.rs", "async fn handle_stop_history_crawl");
  const available = coreItemBody("search.rs", "async fn handle_rooms_available");
  const retain = sourceSection(source, "fn retain_history_crawl_rooms", "fn abort_active_history_crawl_if_retired");
  const failures = [];
  if (!start?.includes("settings.speed == SearchCrawlerSpeed::Paused") || !start?.includes("self.emit_history_crawl_stopped(room_id).await")) failures.push(sourceContractFailure(rule, "paused manual crawl start does not settle stopped state"));
  if (!stop?.includes("self.emit_history_crawl_stopped(room_id).await")) failures.push(sourceContractFailure(rule, "manual crawl stop does not settle stopped state"));
  if (!available?.includes("self.stop_all_history_crawls().await") || !available?.includes("self.emit_history_crawl_stopped(room_id).await")) failures.push(sourceContractFailure(rule, "room availability pruning does not settle stopped state"));
  if (!retain?.includes("-> Vec<String>")) failures.push(sourceContractFailure(rule, "crawler pruning does not return retired room ids"));
  return failures;
}

export function checkCoreSearchPreemptedPageRequeue() {
  const rule = "core.search.preempted_page_requeue";
  const body = coreItemBody("search.rs", "fn handle_history_crawl_page_result");
  const failures = [];
  for (const marker of ["HistoryCrawlPageResult::Preempted", "push_front"]) if (!body?.includes(marker)) failures.push(sourceContractFailure(rule, `preempted page handling lacks ${marker}`));
  return failures;
}

export function checkCoreSearchStartupDelay() {
  const rule = "core.search.startup_delay";
  const source = coreSource("search.rs");
  const starter = coreItemBody("search.rs", "fn start_next_history_crawl_page");
  const failures = [];
  if (!source.includes("CRAWLER_STARTUP_DELAY")) failures.push(sourceContractFailure(rule, "crawler startup-delay constant is missing"));
  for (const marker of ["crawl_delay_elapsed", "manual"]) if (!starter?.includes(marker)) failures.push(sourceContractFailure(rule, `crawler startup-delay gate lacks ${marker}`));
  return failures;
}

export function checkCoreSearchPageSingleFetch() {
  const rule = "core.search.page_single_fetch";
  const body = coreItemBody("search_crawler.rs", "async fn run_history_crawl_page");
  const failures = [];
  if (!body?.includes("result = room.messages(options)")) failures.push(sourceContractFailure(rule, "crawler page does not fetch one messages page"));
  if (body?.includes("loop {")) failures.push(sourceContractFailure(rule, "crawler page loops through room history"));
  return failures;
}

export function checkCoreSearchPageWorkKind() {
  const rule = "core.search.page_work_kind";
  const body = coreItemBody("search_crawler.rs", "async fn run_history_crawl_page");
  const acquire = body?.indexOf("AccountWorkKind::SearchCrawl") ?? -1;
  const messages = body?.indexOf("result = room.messages(options)") ?? -1;
  return acquire >= 0 && messages >= 0 && acquire < messages ? [] : [sourceContractFailure(rule, "crawler page acquires search-crawl work after room.messages")];
}

export function checkCoreSearchPageStartupTrace() {
  const rule = "core.search.page_startup_trace";
  return coreSource("search_crawler.rs").includes("StartupPhase::CrawlerPage") ? [] : [sourceContractFailure(rule, "crawler page does not emit startup trace" )];
}

export function checkCoreSearchPageCancellation() {
  const rule = "core.search.page_cancellation";
  const source = coreSource("search_crawler.rs");
  const failures = [];
  for (const marker of ["permit.cancelled()", "HistoryCrawlPageResult::Preempted", "trace_crawler_preempted"]) if (!source.includes(marker)) failures.push(sourceContractFailure(rule, `crawler cancellation path lacks ${marker}`));
  return failures;
}

export function checkCoreSyncSingleAllRoomsOwner() {
  const rule = "core.sync.single_all_rooms_owner";
  const source = coreSource("sync.rs");
  const failures = [];
  if ((source.match(/SyncService::builder/gu) ?? []).length !== 1) failures.push(sourceContractFailure(rule, "sync service has more than one all-rooms builder"));
  for (const marker of ["committed_all_rooms_response", "room_list_service: Arc<", "room_list_service,"]) if (!source.includes(marker)) failures.push(sourceContractFailure(rule, `single sync owner lacks ${marker}`));
  for (const marker of ["KOUSHI_QA_FORCE_SYNC_BACKEND", "probe_backend", "run_legacy_sync_loop"]) if (source.includes(marker)) failures.push(sourceContractFailure(rule, `legacy sync marker remains: ${marker}`));
  return failures;
}

export function checkCoreSyncCommittedResponseHandoff() {
  const rule = "core.sync.committed_response_handoff";
  const observer = coreItemBody("sync.rs", "async fn observe_sync_service");
  const committed = observer?.indexOf("Signal::Committed(committed)") ?? -1;
  const handoff = observer?.indexOf("reconcile_committed_room_list") ?? -1;
  const failures = [];
  if (committed < 0 || handoff < 0 || handoff <= committed) failures.push(sourceContractFailure(rule, "committed response is not handed to RoomActor after observation"));
  if (observer?.includes("if !committed.range_fully_loaded()")) failures.push(sourceContractFailure(rule, "range readiness gates committed response handoff"));
  return failures;
}

export function checkCoreSyncTimelineCommitBeforeReadiness() {
  const rule = "core.sync.timeline_commit_before_readiness";
  const observer = coreItemBody("sync.rs", "async fn observe_sync_service");
  const committed = observer?.indexOf("Signal::Committed(committed)") ?? -1;
  const forwarding = observer?.indexOf("forward_latest_timeline_response_commit(") ?? -1;
  const handoff = forwarding >= 0 ? observer.slice(forwarding) : "";
  const failures = [];
  if (committed < 0 || forwarding < 0 || forwarding <= committed) failures.push(sourceContractFailure(rule, "timeline commit is not forwarded after committed response"));
  for (const marker of ["run_generation", "committed.sequence()"]) if (!handoff.includes(marker)) failures.push(sourceContractFailure(rule, `timeline commit handoff lacks ${marker}`));
  if (handoff.includes("backend")) failures.push(sourceContractFailure(rule, "timeline commit handoff exposes backend selection"));
  return failures;
}

export function checkCoreSyncTerminatedOwnerRestart() {
  const rule = "core.sync.terminated_owner_restart";
  const observer = coreItemBody("sync.rs", "async fn observe_sync_service");
  const terminated = observer?.split("State::Terminated =>").at(1)?.split("_ => {}").at(0);
  const failures = [];
  for (const marker of ["ReplacementRecoveryProof::new", "sync_service.start().await"]) if (!terminated?.includes(marker)) failures.push(sourceContractFailure(rule, `terminated owner recovery lacks ${marker}`));
  if (terminated?.includes("SyncTaskOutcome::Failed")) failures.push(sourceContractFailure(rule, "terminated owner settles as failed instead of restarting"));
  return failures;
}

export function checkCoreThreadsAggregateRefreshCallers() {
  const rule = "core.threads.aggregate_refresh_callers";
  const manager = coreSource("timeline/manager.rs");
  const projection = coreSource("timeline/thread_projection.rs");
  const commit = sourceSection(projection, "async fn commit_prepared_thread_root_hydration_for_generation(", "fn thread_root_projection_action_from_record");
  const failures = [];
  for (const marker of ["StartAggregateRefresh", "AggregateRefreshFinished", "handle_aggregate_refresh"]) if (!manager.includes(marker)) failures.push(sourceContractFailure(rule, `thread manager lacks ${marker}`));
  for (const marker of ["schedule_aggregate_refresh", "StartAggregateRefresh"]) if (!commit?.includes(marker)) failures.push(sourceContractFailure(rule, `thread projection commit lacks ${marker}`));
  return failures;
}

export function checkCoreThreadsRootProjectionNoPagination() {
  const rule = "core.threads.root_projection_no_pagination";
  const source = sourceSection(coreSource("threads_list.rs"), "pub struct ThreadRootProjectionService");
  const failures = [];
  for (const marker of ["paginate_backwards", "PaginateBackward", "RestoreTimelineAnchor"]) if (source?.includes(marker)) failures.push(sourceContractFailure(rule, `root projection contains forbidden ${marker}`));
  return failures;
}

export function checkCoreThreadsOpenSubscriptionInitialPage() {
  const rule = "core.threads.open_subscription_initial_page";
  const body = sourceSection(coreSource("threads_list.rs"), "async fn open_subscription", "async fn emit_opened");
  const paginate = body?.indexOf("service.paginate().await") ?? -1;
  const emit = body?.indexOf("self.emit_opened") ?? -1;
  return paginate >= 0 && emit >= 0 && paginate < emit ? [] : [sourceContractFailure(rule, "thread subscription emits opened before its initial page")];
}

export function checkCoreThreadsPaginationRequestCorrelation() {
  const rule = "core.threads.pagination_request_correlation";
  const source = coreSource("threads_list.rs");
  const paginate = sourceSection(source, "async fn paginate(&self, request_id: RequestId)", "fn project_item");
  const updates = sourceSection(source, "Some((_, _state)) = pagination_rx.recv()", "else => break");
  const failures = [];
  if (!paginate?.includes("send(request_id)")) failures.push(sourceContractFailure(rule, "pagination does not send its fresh request id"));
  if (paginate?.includes("let _ = request_id")) failures.push(sourceContractFailure(rule, "pagination discards its fresh request id"));
  if (!updates?.includes("current_request_id.sequence")) failures.push(sourceContractFailure(rule, "pagination updates do not use current request id"));
  if (updates?.includes("request_id: request_id.sequence")) failures.push(sourceContractFailure(rule, "pagination updates use the opening request id"));
  return failures;
}

export function checkCoreThreadsReliableRelays() {
  const rule = "core.threads.reliable_relays";
  const source = coreSource("threads_list.rs");
  const open = sourceSection(source, "async fn open_subscription", "async fn emit_opened");
  const paginate = sourceSection(source, "async fn paginate(&self, request_id: RequestId)", "fn project_item");
  const failures = [];
  if (open?.includes("try_send")) failures.push(sourceContractFailure(rule, "thread-list relay uses lossy try_send"));
  for (const marker of ["items_tx.send(room_id.clone()).await", "pagination_tx.send((room_id.clone(), state)).await", "AppAction::ThreadsListFailed", "failed_pagination_request_id"]) if (!open?.includes(marker)) failures.push(sourceContractFailure(rule, `thread-list relay lacks ${marker}`));
  for (const marker of ["classify_thread_list_error(&error)", "pagination_failure_tx"]) if (!paginate?.includes(marker)) failures.push(sourceContractFailure(rule, `pagination failure relay lacks ${marker}`));
  for (const marker of ["self.emit_failed(&scope, request_id, OperationFailureKind::Invalid)", "self.emit_failed(&scope, request_id, OperationFailureKind::NotFound)"]) if (!open?.includes(marker)) failures.push(sourceContractFailure(rule, `open failure relay lacks ${marker}`));
  return failures;
}

function timelineSource(relativePath) {
  return coreSource(`timeline/${relativePath}`);
}

function timelineItemBody(relativePath, marker) {
  return rustItemBody(timelineSource(relativePath), marker);
}

function timelineSection(relativePath, startMarker, endMarker) {
  return sourceSection(timelineSource(relativePath), startMarker, endMarker);
}

export function checkCoreTimelineUnsubscribeCleanupOrder() {
  const rule = "core.timeline.unsubscribe_cleanup_order";
  const branch = timelineSection("manager.rs", "TimelineCommand::Unsubscribe { request_id, key } => {", "TimelineCommand::Paginate");
  const clear = branch?.indexOf("self.clear_thread_root_projections_for_room(&key).await") ?? -1;
  const remove = branch?.indexOf("self.timelines.remove(&key)") ?? -1;
  return clear >= 0 && remove >= 0 && clear < remove
    ? []
    : [sourceContractFailure(rule, "Room unsubscribe does not clear projection state before dropping the actor")];
}

export function checkCoreTimelineStartupTrace() {
  const rule = "core.timeline.startup_trace";
  const failures = [];
  if (!timelineItemBody("manager.rs", "async fn build_timeline_actor_handle")?.includes("StartupPhase::TimelineBuild")) failures.push(sourceContractFailure(rule, "timeline build lacks its startup phase"));
  if (!timelineItemBody("actor.rs", "pub(super) async fn spawn(")?.includes("StartupPhase::TimelineSubscribe")) failures.push(sourceContractFailure(rule, "timeline subscribe lacks its startup phase"));
  if (!timelineItemBody("navigation.rs", "async fn paginate_once_for")?.includes("trace_paginate")) failures.push(sourceContractFailure(rule, "pagination lacks its startup trace"));
  return failures;
}

export function checkCoreTimelineTraceTokens() {
  const rule = "core.timeline.trace_tokens";
  const source = timelineSource("diagnostics.rs");
  const failures = [];
  for (const marker of ["fn trace_timeline_route", "fn trace_timeline_paginate", '"core.timeline"', '"manager_received"', '"actor_paginate_start"', '"gate_acquired"', '"sdk_finish"', "DiagnosticField::request_id", 'DiagnosticField::token("timeline"']) {
    if (!source.includes(marker)) failures.push(sourceContractFailure(rule, `timeline diagnostics lack ${marker}`));
  }
  return failures;
}

export function checkCoreTimelineGapRepairFailureResume() {
  const rule = "core.timeline.gap_repair_failure_resume";
  const handler = timelineItemBody("gap_repair.rs", "async fn handle_timeline_gap_repair_finished");
  const helper = timelineItemBody("gap_repair.rs", "async fn emit_gap_repair_failure_and_resume");
  const failures = [];
  if (!handler || (handler.match(/emit_gap_repair_failure_and_resume/gu) ?? []).length < 3) failures.push(sourceContractFailure(rule, "gap-repair terminal outcomes do not all release queued work"));
  const resume = helper?.indexOf("start_pending_timeline_gap_inspection().await") ?? -1;
  const wake = helper?.indexOf("emit_gap_repair_released_if_idle") ?? -1;
  if (resume < 0 || wake < 0 || resume >= wake) failures.push(sourceContractFailure(rule, "gap-repair resume does not precede the release wake"));
  return failures;
}

export function checkCoreTimelineGapInspectionResume() {
  const rule = "core.timeline.gap_inspection_resume";
  const handler = timelineItemBody("gap_repair.rs", "async fn handle_timeline_gap_inspection_finished");
  const resume = handler?.lastIndexOf("start_pending_timeline_gap_inspection().await") ?? -1;
  const wake = handler?.lastIndexOf("emit_gap_repair_released_if_idle") ?? -1;
  return resume >= 0 && wake >= 0 && resume < wake
    ? []
    : [sourceContractFailure(rule, "gap inspection does not resume queued work before the release wake")];
}

export function checkCoreTimelineGapRepairScheduler() {
  const rule = "core.timeline.gap_repair_scheduler";
  const source = timelineSource("gap_repair.rs");
  const repair = sourceSection(source, "async fn start_timeline_gap_repair", "async fn handle_timeline_gap_repair_finished");
  const acquire = repair?.indexOf("account_work.acquire(work_kind)") ?? -1;
  const call = repair?.indexOf("repair_room_timeline_gap(") ?? -1;
  const yieldOffset = repair?.indexOf("permit.record_yield(1,") ?? -1;
  const settlement = repair?.indexOf("wait_for_gap_repair_projection_with_timeout") ?? -1;
  const failures = [];

  if (acquire < 0 || call < 0 || yieldOffset < 0 || !(acquire < call && call < yieldOffset)) failures.push(sourceContractFailure(rule, "gap repair does not acquire, run one batch, then yield"));
  if (settlement < 0 || yieldOffset >= settlement) failures.push(sourceContractFailure(rule, "gap repair releases its permit after projection settlement"));
  return failures;
}

export function checkCoreTimelineProfileChangeProjection() {
  const rule = "core.timeline.profile_change_projection";
  const branch = timelineSection("item_projection.rs", "TimelineItemContent::ProfileChange(change)", "_ => {}");
  const failures = [];
  if (!branch?.includes("profile_change_projection(change)")) failures.push(sourceContractFailure(rule, "profile changes lack notice projection"));
  if (branch?.includes("change.user_id()")) failures.push(sourceContractFailure(rule, "profile projection exposes a raw user id"));
  return failures;
}

export function checkCoreTimelineSearchReliableDelivery() {
  const rule = "core.timeline.search_reliable_delivery";
  const body = timelineSection("item_projection.rs", "async fn forward_diff_to_search", "fn search_index_messages_for_diff");
  const failures = [];
  if (!body?.includes("emit_search_messages_reliable")) failures.push(sourceContractFailure(rule, "timeline search mutations do not use reliable delivery"));
  if (body?.includes("try_send(SearchIndexMessage")) failures.push(sourceContractFailure(rule, "timeline search mutations use lossy delivery"));
  return failures;
}

export function checkCoreTimelineMediaAttentionReliableDelivery() {
  const rule = "core.timeline.media_attention_reliable_delivery";
  const failures = [];
  if (!timelineItemBody("actor.rs", "pub(super) async fn spawn(")?.includes("action_tx.send(vec![action]).await")) failures.push(sourceContractFailure(rule, "initial media gallery projection is not reliable"));
  if (!timelineItemBody("relay.rs", "async fn handle_diff_batch")?.includes("self.emit_action_reliable(action).await")) failures.push(sourceContractFailure(rule, "thread attention projection is not reliable"));
  const gallery = timelineItemBody("media.rs", "async fn emit_media_gallery_if_changed");
  if (!gallery?.includes("self.emit_action_reliable(action).await") || gallery.includes("try_send(vec![action])")) failures.push(sourceContractFailure(rule, "media gallery projection can advance behind a dropped action"));
  return failures;
}

export function checkCoreTimelineRetryQueueOrder() {
  const rule = "core.timeline.retry_queue_order";
  const body = timelineSection("outbound_send.rs", "async fn handle_retry_send", "async fn handle_cancel_send");
  const enable = body?.indexOf("set_enabled(true)") ?? -1;
  const unwedge = body?.indexOf("unwedge().await") ?? -1;
  return enable >= 0 && unwedge >= 0 && enable < unwedge
    ? []
    : [sourceContractFailure(rule, "retry unwedges the SDK queue before re-enabling it")];
}

export function checkCoreTimelineCancelQueueOrder() {
  const rule = "core.timeline.cancel_queue_order";
  const body = timelineSection("outbound_send.rs", "async fn handle_cancel_send", "fn sdk_room_for_key");
  const abort = body?.indexOf("abort().await") ?? -1;
  const enable = body?.indexOf("set_enabled(true)") ?? -1;
  return abort >= 0 && enable >= 0 && abort < enable
    ? []
    : [sourceContractFailure(rule, "cancel does not re-enable the SDK queue after abort")];
}

export function checkCoreTimelineSignalTraces() {
  const rule = "core.timeline.signal_traces";
  const sources = ["item_projection.rs", "diagnostics.rs", "manager.rs", "read_state.rs"].map(timelineSource);
  const failures = [];
  for (const kind of ["send_reaction", "redact_reaction"]) {
    if (!sources.some((source) => new RegExp(`trace_timeline_actor_operation\\(\\s*\\"actor_start\\",\\s*\\"${kind}\\"`).test(source))) failures.push(sourceContractFailure(rule, `${kind} lacks actor-start tracing`));
    if (!sources.some((source) => new RegExp(`trace_timeline_actor_operation\\(\\s*\\"actor_finish\\",\\s*\\"${kind}\\"`).test(source))) failures.push(sourceContractFailure(rule, `${kind} lacks actor-finish tracing`));
  }
  for (const kind of ["send_read_receipt", "set_fully_read"]) {
    if (!sources.some((source) => new RegExp(`trace_timeline_route\\(\\s*\\"manager_received\\",\\s*\\"${kind}\\"`).test(source))) failures.push(sourceContractFailure(rule, `${kind} lacks manager-admission tracing`));
  }
  const read = sources.join("\n");
  for (const marker of ["ReadWorkerCompletion::Network", "ReadWorkerCompletion::ActorApplied"]) if (!read.includes(marker)) failures.push(sourceContractFailure(rule, `read operations lack ${marker}`));
  if (!sources.some((source) => /trace_timeline_actor_scan\(\s*"target_scan"/.test(source))) failures.push(sourceContractFailure(rule, "reaction target scans lack tracing"));
  return failures;
}

export function checkCoreTimelineLinkPreviewTrace() {
  const rule = "core.timeline.link_preview_trace";
  const source = timelineSource("diagnostics.rs");
  const failures = [];
  for (const marker of ["fn trace_timeline_link_preview", '"link_preview"', '"lookup_miss"', '"no_previews"', '"start"', '"complete"', '"load_link_previews"', 'DiagnosticField::count("pending"', 'DiagnosticField::milliseconds("duration"', "DiagnosticField::request_id"]) if (!source.includes(marker)) failures.push(sourceContractFailure(rule, `link-preview diagnostics lack ${marker}`));
  return failures;
}

export function checkCoreTimelineLinkPreviewOffLoop() {
  const rule = "core.timeline.link_preview_off_loop";
  const production = timelineSource("item_projection.rs");
  const load = timelineItemBody("item_projection.rs", "async fn handle_load_link_previews");
  const failures = [];
  if (load?.includes("fetch_link_preview(")) failures.push(sourceContractFailure(rule, "link-preview fetch runs on the actor command loop"));
  for (const marker of ["spawn_link_preview_fetch", "LinkPreviewsFetched"]) if (!production.includes(marker)) failures.push(sourceContractFailure(rule, `link-preview worker path lacks ${marker}`));
  return failures;
}

export function checkCoreTimelineLinkPreviewCancellation() {
  const rule = "core.timeline.link_preview_cancellation";
  const item = timelineSource("item_projection.rs");
  const actor = timelineSource("actor.rs");
  const cancel = timelineItemBody("item_projection.rs", "fn handle_cancel_link_previews");
  const fetched = timelineItemBody("item_projection.rs", "async fn handle_link_previews_fetched");
  const failures = [];
  if (!actor.includes("CancelLinkPreviews")) failures.push(sourceContractFailure(rule, "timeline actor lacks link-preview cancellation"));
  if (!cancel?.includes(".abort()")) failures.push(sourceContractFailure(rule, "link-preview cancellation does not abort the worker"));
  if (!cancel?.includes("reset_loading_link_previews_to_pending")) failures.push(sourceContractFailure(rule, "link-preview cancellation does not reset loading state"));
  if (!fetched?.includes("remove(&event_id).is_none()")) failures.push(sourceContractFailure(rule, "late cancelled link-preview results are not ignored"));
  return failures;
}

export function checkCoreTimelineInitialSearchForward() {
  const rule = "core.timeline.initial_search_forward";
  const spawn = timelineItemBody("actor.rs", "pub(super) async fn spawn(");
  const forward = spawn?.indexOf("forward_initial_items_to_search") ?? -1;
  const run = spawn?.indexOf("actor.run()") ?? -1;
  return forward >= 0 && run >= 0 && forward < run
    ? []
    : [sourceContractFailure(rule, "initial timeline items are not forwarded before the actor loop")];
}

export function checkCoreTimelineSubscribeSuccess() {
  const rule = "core.timeline.subscribe_success";
  const source = timelineSource("manager.rs");
  const subscribe = timelineItemBody("manager.rs", "async fn handle_subscribe");
  const build = timelineSection("manager.rs", "async fn build_timeline_actor_handle", "async fn route_to_actor_or_fail");
  const success = subscribe?.split("Ok(handle) =>").at(1) ?? "";
  const action = success.indexOf("emit_timeline_subscribed_action");
  const failures = [];
  for (const marker of ["TimelineActor::spawn", "TimelineKind::Room"]) if (!source.includes(marker)) failures.push(sourceContractFailure(rule, `subscribe success lacks ${marker}`));
  if (!build?.includes("TimelineActor::spawn")) failures.push(sourceContractFailure(rule, "subscribe success does not spawn the actor"));
  if (action < 0) failures.push(sourceContractFailure(rule, "subscribe success does not reduce TimelineSubscribed"));
  return failures;
}

export function checkCoreTimelineSubscribeReliableSettles() {
  const rule = "core.timeline.subscribe_reliable_settles";
  const subscribed = timelineItemBody("manager.rs", "async fn emit_timeline_subscribed_action");
  const failure = timelineItemBody("diagnostics.rs", "fn timeline_subscription_failed_action");
  const subscribe = timelineItemBody("manager.rs", "async fn handle_subscribe");
  const failures = [];
  if (!subscribed?.includes("self.action_tx.send(vec![action]).await") || subscribed.includes("try_send(vec![action])")) failures.push(sourceContractFailure(rule, "timeline subscribe success is not reliably delivered"));
  for (const marker of ["TimelineKind::Room { .. } => None", "AppAction::ThreadSubscriptionFailed", "AppAction::FocusedContextSubscriptionFailed"]) if (!failure?.includes(marker)) failures.push(sourceContractFailure(rule, `subscription failure path lacks ${marker}`));
  if (!subscribe?.includes("self.emit_subscription_failure")) failures.push(sourceContractFailure(rule, "subscribe failure branches do not settle reducer state"));
  return failures;
}

export function checkCoreTimelineThreadFocus() {
  const rule = "core.timeline.thread_focus";
  const focus = timelineItemBody("manager.rs", "async fn build_timeline_actor_handle")?.split("let focus = match &key.kind").at(1)?.split("let timeline_result").at(0);
  const thread = focus?.split("TimelineKind::Thread").at(1)?.split("TimelineKind::Focused").at(0);
  const failures = [];
  if (!thread?.includes("TimelineFocus::Thread")) failures.push(sourceContractFailure(rule, "thread timelines do not use SDK thread focus"));
  if (thread?.includes("TimelineFocus::Event")) failures.push(sourceContractFailure(rule, "thread timelines use event focus"));
  return failures;
}

export function checkCoreTimelineIdempotentSubscribe() {
  const rule = "core.timeline.idempotent_subscribe";
  const source = timelineSource("manager.rs");
  const subscribe = timelineSection("manager.rs", "async fn handle_subscribe", "async fn route_to_actor_or_fail");
  const existing = subscribe?.split("let replay_result = if let Some(handle) = self.timelines.get(&key)").at(1)?.split("let client = session.client()").at(0);
  const build = timelineSection("manager.rs", "async fn build_timeline_actor_handle", "async fn route_to_actor_or_fail");
  const failures = [];
  for (const marker of ["ReplayInitialItems", "return;"]) if (!existing?.includes(marker)) failures.push(sourceContractFailure(rule, `existing timeline replay path lacks ${marker}`));
  if (!build || build.includes("subscribe_to_rooms_with_generation")) failures.push(sourceContractFailure(rule, "timeline actor construction mutates room subscriptions"));
  for (const marker of ["lease_room", "reconcile_subscriptions"]) if (!subscribe?.includes(marker)) failures.push(sourceContractFailure(rule, `new timeline subscription lacks ${marker}`));
  if (!source.includes("let replay_result = if let Some(handle) = self.timelines.get(&key)")) failures.push(sourceContractFailure(rule, "subscribe does not detect an existing key"));
  return failures;
}

export function checkCoreTimelineSyncStartedRebuild() {
  const rule = "core.timeline.sync_started_rebuild";
  const manager = timelineSource("manager.rs");
  const run = timelineItemBody("manager.rs", "async fn run(mut self)");
  const started = timelineItemBody("residency.rs", "async fn handle_sync_started");
  const rebuild = timelineItemBody("residency.rs", "async fn rebuild_existing_room_timelines_after_sync_started");
  const failures = [];
  if (!run?.includes("self.handle_sync_started(room_list_service, core_generation)")) failures.push(sourceContractFailure(rule, "SyncStarted is not routed by the manager"));
  for (const marker of ["self.room_list_service = Some(room_list_service.clone());", "self.subscribe_existing_timeline_rooms(&room_list_service)", "rebuild_existing_room_timelines_after_sync_started"]) if (!started?.includes(marker)) failures.push(sourceContractFailure(rule, `SyncStarted handler lacks ${marker}`));
  for (const marker of ["session_subscribed_rooms", "reconcile_room_subscriptions_with_generation"]) if (!timelineSource("residency.rs").includes(marker)) failures.push(sourceContractFailure(rule, `residency lacks ${marker}`));
  if (!rebuild?.includes("matches!(key.kind, TimelineKind::Room { .. })")) failures.push(sourceContractFailure(rule, "SyncStarted rebuild is not room-only"));
  if (rebuild?.includes("self.timelines.remove(&key);")) failures.push(sourceContractFailure(rule, "SyncStarted rebuild drops the existing actor before replacement"));
  if (!rebuild?.includes("replace_existing_room_timeline_after_sync_started")) failures.push(sourceContractFailure(rule, "SyncStarted rebuild lacks replacement swap"));
  if (!manager.includes("handle_sync_started")) failures.push(sourceContractFailure(rule, "manager source lacks SyncStarted handling"));
  return failures;
}

export function checkCoreTimelineEnsureSubscribed() {
  const rule = "core.timeline.ensure_subscribed";
  const source = timelineSource("manager.rs");
  const command = timelineSection("manager.rs", "async fn handle_command(&mut self, command: TimelineCommand)", "async fn handle_command_with_permit");
  const permitted = timelineSection("manager.rs", "async fn handle_command_with_permit", "async fn route_send_to_worker_or_fail");
  const subscribe = timelineSection("manager.rs", "async fn handle_subscribe", "let client = session.client()");
  const failures = [];
  if (!command?.includes("self.handle_command_with_permit(command, None).await")) failures.push(sourceContractFailure(rule, "plain timeline commands bypass the permit-aware helper"));
  for (const marker of ["TimelineCommand::EnsureSubscribed", "replay_existing"]) if (!permitted?.includes(marker)) failures.push(sourceContractFailure(rule, `ensure-subscription route lacks ${marker}`));
  if (!subscribe?.includes("if replay_existing")) failures.push(sourceContractFailure(rule, "existing actors do not honor replay_existing"));
  if (!source.includes("async fn handle_command_with_permit")) failures.push(sourceContractFailure(rule, "permit-aware command helper is missing"));
  return failures;
}

export function checkCoreTimelineReplaySubscribed() {
  const rule = "core.timeline.replay_subscribed";
  const command = timelineSection("manager.rs", "async fn handle_command_with_permit", "async fn route_send_to_worker_or_fail");
  const replay = timelineSection("manager.rs", "async fn handle_replay_subscribed", "async fn handle_subscribe");
  const failures = [];
  for (const marker of ["TimelineCommand::ReplaySubscribed { request_id }", "self.handle_replay_subscribed(request_id).await"]) if (!command?.includes(marker)) failures.push(sourceContractFailure(rule, `replay command route lacks ${marker}`));
  for (const marker of ["for handle in self.timelines.values()", "TimelineActorMessage::ReplayInitialItems", "cause_request_id: None", ".await"]) if (!replay?.includes(marker)) failures.push(sourceContractFailure(rule, `replay handler lacks ${marker}`));
  return failures;
}

export function checkCoreTimelineMediaDownloadLifecycle() {
  const rule = "core.timeline.media_download_lifecycle";
  const source = timelineSource("media.rs");
  const handler = timelineItemBody("media.rs", "async fn handle_download_media")?.split("async fn download_media_for").at(0);
  const worker = timelineSection("media.rs", "async fn download_media_for", "async fn handle_media_download_finished");
  const failures = [];
  for (const marker of ["TimelineActorMessage::MediaDownloadFinished", "executor::spawn(async move", "emit_media_download_current_state"]) if (!handler?.includes(marker)) failures.push(sourceContractFailure(rule, `media download handler lacks ${marker}`));
  if (handler?.includes(".get_media_content(")) failures.push(sourceContractFailure(rule, "media transfer runs inline on the actor loop"));
  for (const marker of ["executor::timeout(", "MEDIA_DOWNLOAD_TIMEOUT", "classify_media_download_error(&error)", "TimelineFailureKind::Timeout"]) if (!worker?.includes(marker)) failures.push(sourceContractFailure(rule, `media download worker lacks ${marker}`));
  if (!source.includes("async fn download_media_for")) failures.push(sourceContractFailure(rule, "media download worker is missing"));
  return failures;
}

export function checkCoreTimelineMediaDownloadDiagnostics() {
  const rule = "core.timeline.media_download_diagnostics";
  const source = timelineSource("media.rs");
  const failures = [];
  for (const marker of ["core.media_download", "request_received", "request_rejected", "cache_hit", "sdk_fetch_started", "sdk_fetch_failed", "file_write_failed", "completed", "selection", "source_encrypted", "thumbnail_source_present", "failure", "raw_os_error", "data_dir_present", "target_dir_exists", "target_path_exists", "target_path_is_file", "target_path_is_dir"]) if (!source.includes(marker)) failures.push(sourceContractFailure(rule, `media download diagnostics lack ${marker}`));
  return failures;
}

export function checkCoreTimelinePaginationScheduler() {
  const rule = "core.timeline.pagination_scheduler";
  const source = timelineSource("navigation.rs");
  const admission = timelineSection("navigation.rs", "async fn acquire_pagination_permit_and_emit_paginating", "/// Emits an already-authorized group");
  const operation = timelineSection("navigation.rs", "async fn paginate_once_for", "fn emit_pagination_completion");
  const acquire = operation?.indexOf("acquire_pagination_permit_and_emit_paginating") ?? -1;
  const paginate = operation?.indexOf("paginate_backwards") ?? -1;
  const failures = [];
  if (!source.includes("AccountWorkScheduler")) failures.push(sourceContractFailure(rule, "timeline actor lacks account work scheduler"));
  if (!admission?.includes("AccountWorkKind::ExplicitPagination")) failures.push(sourceContractFailure(rule, "pagination lacks explicit-pagination work kind"));
  if (acquire < 0 || paginate < 0 || acquire >= paginate) failures.push(sourceContractFailure(rule, "pagination calls the SDK before scheduler admission"));
  return failures;
}

export function checkCoreTimelinePaginationCancellation() {
  const rule = "core.timeline.pagination_cancellation";
  const actor = timelineSource("actor.rs");
  const actorStruct = timelineItemBody("actor.rs", "pub(super) struct TimelineActor {");
  const paginate = timelineItemBody("navigation.rs", "pub(super) async fn handle_paginate");
  const cancel = timelineItemBody("navigation.rs", "pub(super) fn handle_cancel_pagination");
  const failures = [];
  for (const [body, marker] of [[actor, "CancelPagination"], [actorStruct, "pagination_task"], [paginate, "executor::spawn"], [cancel, ".abort()"]]) if (!body?.includes(marker)) failures.push(sourceContractFailure(rule, `pagination cancellation lacks ${marker}`));
  return failures;
}

export function checkCoreTimelinePaginationTerminalRelease() {
  const rule = "core.timeline.pagination_terminal_release";
  const handler = timelineItemBody("actor.rs", "async fn handle_msg");
  const branch = handler?.split("TimelineActorMessage::PaginationFinished {").at(1);
  const release = branch?.indexOf("self.pagination_task = None") ?? -1;
  const terminal = branch?.indexOf("self.emit_pagination_completion") ?? -1;
  return release >= 0 && terminal >= 0 && release < terminal
    ? []
    : [sourceContractFailure(rule, "pagination terminal is emitted before active-task release")];
}

export function checkCoreTimelineRestoreRoomBounded() {
  const rule = "core.timeline.restore_room_bounded";
  const handler = timelineSection("navigation.rs", "async fn handle_restore_timeline_anchor(", "async fn handle_restore_timeline_anchor_continue");
  const continuation = timelineSection("navigation.rs", "async fn handle_restore_timeline_anchor_continue", "async fn schedule_restore_anchor_continue");
  const failures = [];
  if (!handler?.includes("TimelineKind::Room")) failures.push(sourceContractFailure(rule, "restore anchor is not room-only"));
  if (!continuation?.includes("PaginationDirection::Backward")) failures.push(sourceContractFailure(rule, "restore anchor does not paginate backward"));
  for (const marker of ["max_batches", "event_count"]) if (!handler?.includes(marker)) failures.push(sourceContractFailure(rule, `restore anchor lacks ${marker}`));
  if (handler?.includes("TimelineKind::Focused")) failures.push(sourceContractFailure(rule, "restore anchor uses focused timeline path"));
  return failures;
}

export function checkCoreTimelineRestoreBudget() {
  const rule = "core.timeline.restore_budget";
  const production = timelineSource("navigation.rs");
  const construct = production.split("let restore = RestoreTimelineAnchorState {").at(1);
  const existing = production.split("if existing.event_id == event_id {").at(1);
  const failures = [];
  if (!construct?.includes("max_batches_remaining: max_batches,")) failures.push(sourceContractFailure(rule, "restore initialization ignores the frontend budget"));
  if (!existing?.includes(".max(max_batches);")) failures.push(sourceContractFailure(rule, "restore replacement inflates the in-flight budget"));
  return failures;
}

export function checkCoreTimelineRestoreCoalescing() {
  const rule = "core.timeline.restore_coalescing";
  const actor = timelineItemBody("actor.rs", "pub(super) struct TimelineActor {");
  const diff = timelineItemBody("relay.rs", "async fn handle_diff_batch");
  const navigation = timelineSource("navigation.rs");
  const finish = timelineItemBody("navigation.rs", "fn finish_anchor_restore");
  const settlement = timelineItemBody("navigation.rs", "pub(super) fn publish_restore_settlement_for_generation(");
  const handler = timelineItemBody("navigation.rs", "async fn handle_restore_timeline_anchor(");
  const continuation = timelineItemBody("navigation.rs", "async fn handle_restore_timeline_anchor_continue(");
  const failures = [];
  for (const marker of ["restore_emit_buffer: Vec<TimelineDiff>"]) if (!actor?.includes(marker)) failures.push(sourceContractFailure(rule, `restore actor lacks ${marker}`));
  for (const marker of ["restore_anchor.is_some()", "restore_emit_buffer", "ItemsUpdated"]) if (!diff?.includes(marker)) failures.push(sourceContractFailure(rule, `restore diff relay lacks ${marker}`));
  if (!finish?.includes("publish_restore_settlement(Some((request_id, status)))")) failures.push(sourceContractFailure(rule, "restore finish bypasses atomic settlement"));
  for (const marker of ["std::mem::take", "TimelineEvent::NavigationUpdated", "TimelineEvent::AnchorRestoreFinished"]) if (!settlement?.includes(marker)) failures.push(sourceContractFailure(rule, `restore settlement lacks ${marker}`));
  if (finish?.includes("emit_anchor_restore_finished")) failures.push(sourceContractFailure(rule, "restore finish emits a second raw terminal"));
  if ((handler?.match(/self\.emit_anchor_restore_finished\(/gu) ?? []).length > 1) failures.push(sourceContractFailure(rule, "restore handler emits too many raw terminals"));
  if (continuation?.includes("self.emit_anchor_restore_finished(")) failures.push(sourceContractFailure(rule, "restore continuation bypasses finish_anchor_restore"));
  if (!navigation.includes("publish_restore_settlement_for_generation")) failures.push(sourceContractFailure(rule, "restore settlement helper is missing"));
  return failures;
}

export function checkCoreTimelineRestoreTerminal() {
  const rule = "core.timeline.restore_terminal";
  const production = timelineSource("navigation.rs");
  const state = production.split("struct RestoreTimelineAnchorState {").at(1)?.split("}").at(0);
  const continuation = timelineSection("navigation.rs", "async fn handle_restore_timeline_anchor_continue(", "async fn maybe_continue_restore_anchor_after_diff");
  const handler = timelineSection("navigation.rs", "async fn handle_restore_timeline_anchor(", "async fn handle_restore_timeline_anchor_continue");
  const failures = [];
  if (!state?.includes("anchor_relay_wait")) failures.push(sourceContractFailure(rule, "restore state lacks relay wait"));
  for (const marker of ["anchor_relay_wait", "outcome.anchor_present", "outcome.reached_start"]) if (!continuation?.includes(marker)) failures.push(sourceContractFailure(rule, `restore continuation lacks ${marker}`));
  for (const marker of ["settle_last_seen_seq", "settle_awaiting_first_diff", "RESTORE_ANCHOR_SETTLE_TICK_DELAY_MS", "schedule_restore_anchor_settle_tick"]) if (production.includes(marker)) failures.push(sourceContractFailure(rule, `restore retains timing heuristic ${marker}`));
  if (!handler?.includes("emit_anchor_restore_finished")) failures.push(sourceContractFailure(rule, "invalid restore requests do not emit their terminal"));
  return failures;
}

export function checkCoreTimelineSendAdmissionGuard() {
  const rule = "core.timeline.send_admission_guard";
  const source = timelineSource("outbound_send.rs");
  const spawn = timelineSection("outbound_send.rs", "fn spawn_send_enqueue(", "fn handle_send_enqueue_worker_completion");
  const guard = spawn?.indexOf("begin_interactive(AccountWorkKind::MessageSend)") ?? -1;
  const enqueue = spawn?.indexOf("enqueue_timeline_send(context, payload)") ?? -1;
  const preflight = spawn?.indexOf("preflight_started_tx.send(())") ?? -1;
  const failures = [];
  if (guard < 0 || enqueue < 0 || guard >= enqueue) failures.push(sourceContractFailure(rule, "send enqueue is not held under the interactive guard"));
  if (preflight < 0 || preflight >= guard) failures.push(sourceContractFailure(rule, "send admission waits for the scheduler"));
  if (!source.includes("fn spawn_send_enqueue(")) failures.push(sourceContractFailure(rule, "send enqueue worker is missing"));
  return failures;
}

export function checkCoreTimelineSendCompletionGuard() {
  const rule = "core.timeline.send_completion_guard";
  const source = timelineSource("outbound_send.rs");
  const pending = source.split("struct CoordinatedPendingSend").at(1)?.split("enum SendCompletionObservation").at(0);
  const spawn = timelineSection("outbound_send.rs", "fn spawn_send_enqueue(", "fn handle_send_enqueue_worker_completion");
  const terminal = timelineSection("outbound_send.rs", "fn apply_terminal(", "fn media_upload_progress_identity");
  const guard = spawn?.indexOf("begin_interactive(AccountWorkKind::MessageSend)") ?? -1;
  const retain = spawn?.indexOf("registration.hold_interactive_guard") ?? -1;
  const enqueue = spawn?.indexOf("enqueue_timeline_send(context, payload)") ?? -1;
  const failures = [];
  if (!pending?.includes("interactive_guard: Option<InteractiveWorkGuard>")) failures.push(sourceContractFailure(rule, "pending send does not retain its interactive guard"));
  if (guard < 0 || retain < 0 || enqueue < 0 || !(guard < retain && retain < enqueue)) failures.push(sourceContractFailure(rule, "send guard is not retained before SDK enqueue"));
  if (!terminal?.includes("pending.interactive_guard.take()")) failures.push(sourceContractFailure(rule, "send terminal does not release its interactive guard"));
  return failures;
}

export function checkCoreTimelineSendSubmissionRoute() {
  const rule = "core.timeline.send_submission_route";
  const helper = timelineItemBody("outbound_send.rs", "async fn route_send_to_worker_or_fail");
  const lookup = helper?.indexOf("handle.enqueue_context.clone()") ?? -1;
  const submitted = helper?.indexOf("send_submitted_action") ?? -1;
  const worker = helper?.indexOf("self.spawn_send_enqueue") ?? -1;
  const failures = [];
  if (lookup < 0 || submitted < 0 || worker < 0 || !(lookup < submitted && submitted < worker)) failures.push(sourceContractFailure(rule, "send submission is reduced outside the manager worker route"));
  if (!timelineSource("outbound_send.rs").includes("AppAction::SendTextSubmitted")) failures.push(sourceContractFailure(rule, "room send projection lacks SendTextSubmitted"));
  return failures;
}

export function checkCoreTimelineThreadReplySubmissionRoute() {
  const rule = "core.timeline.thread_reply_submission_route";
  const helper = timelineItemBody("outbound_send.rs", "async fn route_send_to_worker_or_fail");
  const lookup = helper?.indexOf("handle.enqueue_context.clone()") ?? -1;
  const submitted = helper?.indexOf("send_submitted_action") ?? -1;
  const source = timelineSource("outbound_send.rs");
  const failures = [];
  if (lookup < 0 || submitted < 0 || lookup >= submitted) failures.push(sourceContractFailure(rule, "thread submission is reduced before manager route resolution"));
  if (!source.includes("AppAction::ThreadReplySubmitted")) failures.push(sourceContractFailure(rule, "thread projection lacks ThreadReplySubmitted"));
  return failures;
}

export function checkCoreTimelineThreadComposerRoute() {
  const rule = "core.timeline.thread_composer_route";
  const source = timelineSource("outbound_send.rs");
  const helper = timelineItemBody("outbound_send.rs", "async fn route_send_to_worker_or_fail");
  const projection = timelineSection("outbound_send.rs", "fn send_submitted_action", "fn send_finished_action");
  const terminal = timelineSection("outbound_send.rs", "fn send_terminal_action", "fn timeline_send_terminal_handoff");
  const failures = [];
  if (!helper?.includes("send_submitted_action") || !projection?.includes("TimelineKind::Thread") || !projection?.includes("ThreadReplySubmitted")) failures.push(sourceContractFailure(rule, "thread reply does not submit thread composer state"));
  if (!source.includes("ThreadReplyFailed")) failures.push(sourceContractFailure(rule, "thread reply failure does not clear pending state"));
  for (const marker of ["ComposerSubmissionSettled", "ComposerSubmissionTerminalOutcome", "submission_target"]) if (!terminal?.includes(marker)) failures.push(sourceContractFailure(rule, `thread send terminal lacks ${marker}`));
  if (!source.includes("TimelineKind::Focused { .. } => Self::None") || !source.includes("TimelineKind::Focused { .. } => None")) failures.push(sourceContractFailure(rule, "focused timelines own composer state"));
  return failures;
}

export function checkCoreTimelineOutboundState() {
  const rule = "core.timeline.outbound_state";
  const outbound = timelineSource("outbound_send.rs");
  const manager = timelineSource("manager.rs");
  const monitor = timelineItemBody("outbound_send.rs", "async fn run_send_queue_monitor");
  const projection = timelineItemBody("item_projection.rs", "fn sdk_item_to_timeline_item_with_send_states");
  const boundary = timelineItemBody("outbound_send.rs", "fn apply_send_completion_observation_and_handoff");
  const observer = timelineItemBody("outbound_send.rs", "async fn run_global_send_completion_observer");
  const actor = timelineItemBody("outbound_send.rs", "async fn handle_send_queue_update");
  const delivery = timelineItemBody("outbound_send.rs", "async fn handle_send_terminal_handoff");
  const run = timelineItemBody("manager.rs", "async fn run(mut self)");
  const failures = [];
  if (!monitor?.includes("TimelineActorMessage::SendQueueLagged") || monitor.includes("not critical for send completion tracking")) failures.push(sourceContractFailure(rule, "send queue lag is not treated as terminally relevant"));
  const sdk = projection?.indexOf("timeline_send_state_from_sdk") ?? -1;
  const mirror = projection?.indexOf("send_statuses.get") ?? -1;
  if (sdk < 0 || mirror < 0 || sdk >= mirror) failures.push(sourceContractFailure(rule, "SDK send state does not win over the actor mirror"));
  if (!boundary?.includes("terminal_ingress.admit(handoff)") || boundary.includes(".await") || boundary.includes("executor::spawn")) failures.push(sourceContractFailure(rule, "terminal handoff is not synchronously admitted"));
  for (const marker of ["SendQueueUpdate", "RecvError::Lagged", "apply_send_completion_observation_loss_and_handoff"]) if (!observer?.includes(marker)) failures.push(sourceContractFailure(rule, `global send observer lacks ${marker}`));
  if (actor?.includes("apply_send_completion_observation_and_handoff") || actor?.includes("SendCompleted")) failures.push(sourceContractFailure(rule, "replaceable actor consumes send terminals"));
  for (const marker of ["deliver_submission_terminal_action", ".await"]) if (!delivery?.includes(marker)) failures.push(sourceContractFailure(rule, `terminal delivery lacks ${marker}`));
  if (delivery?.includes("try_send") || delivery?.includes("schedule_room_key_reshares")) failures.push(sourceContractFailure(rule, "terminal delivery is lossy or schedules a forbidden reshare"));
  if (!run?.includes("tokio::select!") || !run.includes("biased;") || !run.includes("terminal_rx.recv()")) failures.push(sourceContractFailure(rule, "timeline manager does not prioritize terminal ingress"));
  if ((manager.match(/session\.client\(\)\.send_queue\(\)\.subscribe\(\)/gu) ?? []).length !== 1) failures.push(sourceContractFailure(rule, "timeline manager does not have one global send terminal subscription"));
  if (outbound.includes("SharedSendCompletionTracker") || manager.includes("SharedSendCompletionTracker")) failures.push(sourceContractFailure(rule, "obsolete shared send tracker remains"));
  return failures;
}

export function checkCoreTimelineSendSupervision() {
  const rule = "core.timeline.send_queue_supervision";
  const outbound = timelineSource("outbound_send.rs");
  const manager = timelineSource("manager.rs");
  const route = timelineItemBody("outbound_send.rs", "async fn route_submission_to_worker");
  const run = timelineItemBody("manager.rs", "async fn run(mut self)");
  const supervisor = timelineItemBody("outbound_send.rs", "impl SendEnqueueWorkerSupervisor");
  const supervisorDrop = timelineItemBody("outbound_send.rs", "impl Drop for SendEnqueueWorkerSupervisor");
  const managerDrop = timelineItemBody("manager.rs", "impl Drop for TimelineManagerActor");
  const runner = timelineItemBody("outbound_send.rs", "fn spawn_send_enqueue_future");
  const spawn = route?.indexOf("self.spawn_send_enqueue") ?? -1;
  const activate = route?.indexOf("activate_registration") ?? -1;
  const action = route?.indexOf("self.action_tx.send") ?? -1;
  const release = route?.indexOf("permit_tx.send") ?? -1;
  const failures = [];
  if (spawn < 0 || activate < 0 || action < 0 || release < 0 || !(spawn < activate && activate < action && action < release)) failures.push(sourceContractFailure(rule, "supervised send worker is not permit-blocked through reducer delivery"));
  for (const marker of ["TimelineActorMessage::SendText", "TimelineActorMessage::SendReply", "TimelineActorMessage::UploadAndSendMedia"]) if (outbound.includes(marker) || manager.includes(marker)) failures.push(sourceContractFailure(rule, `replaceable actor owns ${marker}`));
  for (const marker of ["route_send_to_worker_or_fail", "route_submission_to_worker", "route_media_send_to_worker_or_fail"]) if (!outbound.includes(marker)) failures.push(sourceContractFailure(rule, `send path lacks ${marker}`));
  if (!outbound.includes("enqueue_timeline_send(context, payload).await")) failures.push(sourceContractFailure(rule, "send payloads bypass supervised enqueue"));
  const workerPoll = run?.indexOf("worker = self.send_enqueue_workers.tasks.next()") ?? -1;
  const mailbox = run?.indexOf("msg = self.msg_rx.recv()") ?? -1;
  const join = run?.indexOf("self.join_send_enqueue_workers().await") ?? -1;
  const observer = run?.indexOf("self.global_send_completion_observer_future.take()") ?? -1;
  const actors = run?.indexOf("let timeline_actors = self") ?? -1;
  if (!run?.includes("if !self.send_enqueue_workers.tasks.is_empty()") || workerPoll < 0 || mailbox < 0 || workerPoll >= mailbox) failures.push(sourceContractFailure(rule, "manager does not poll nonempty worker futures before its mailbox"));
  if (join < 0 || observer < 0 || actors < 0 || !(join < observer && observer < actors)) failures.push(sourceContractFailure(rule, "manager shutdown order drops observer or actors too early"));
  for (const marker of ["fn cancel_all(&mut self)", "self.tasks = FuturesUnordered::new()"]) if (!supervisor?.includes(marker)) failures.push(sourceContractFailure(rule, `worker supervisor lacks ${marker}`));
  for (const marker of ["self.terminal_ingress.stop_accepting()", "self.cancel_all()"]) if (!supervisorDrop?.includes(marker)) failures.push(sourceContractFailure(rule, `worker supervisor drop lacks ${marker}`));
  const admission = managerDrop?.indexOf("self.terminal_ingress.stop_accepting()") ?? -1;
  const cancel = managerDrop?.indexOf("self.send_enqueue_workers.cancel_all()") ?? -1;
  const dropObserver = managerDrop?.indexOf("self.global_send_completion_observer_future.take()") ?? -1;
  if (admission < 0 || cancel < 0 || dropObserver < 0 || !(admission < cancel && cancel < dropObserver)) failures.push(sourceContractFailure(rule, "manager drop does not close admission, workers, then observer"));
  if (!runner?.includes("AssertUnwindSafe") || !runner?.includes(".catch_unwind()")) failures.push(sourceContractFailure(rule, "enqueue future lacks panic isolation"));
  return failures;
}

export function checkCoreTimelineRoomReadMarker() {
  const rule = "core.timeline.room_read_marker";
  const source = timelineSource("read_state.rs");
  const network = timelineSection("read_state.rs", "async fn perform_read_network_operation", "async fn run_send_enqueue_future");
  const actor = timelineSection("read_state.rs", "async fn handle_read_success", "async fn handle_own_read_receipt_changed");
  const settlement = timelineSection("read_state.rs", "async fn settle_read_operation", "async fn route_to_actor_or_fail");
  const failures = [];
  for (const marker of ["send_multiple_receipts", "room.send_multiple_receipts", "fully_read_marker", "private_read_receipt"]) if (!network?.includes(marker)) failures.push(sourceContractFailure(rule, `room read marker lacks ${marker}`));
  if (network?.includes("send_single_receipt(ReceiptType::FullyRead")) failures.push(sourceContractFailure(rule, "room read marker uses standalone fully-read receipt"));
  for (const marker of ["AppAction::FullyReadMarkerUpdated", "emit_action_reliable"]) if (!actor?.includes(marker)) failures.push(sourceContractFailure(rule, `read success lacks ${marker}`));
  if (!settlement?.includes("AppAction::RoomMarkedAsReadSucceeded")) failures.push(sourceContractFailure(rule, "read settlement does not clear room unread state"));
  if (!source.includes("async fn perform_read_network_operation")) failures.push(sourceContractFailure(rule, "read network worker is missing"));
  return failures;
}

export function checkCoreTimelineThreadReadReceipts() {
  const rule = "core.timeline.thread_read_receipts";
  const worker = timelineSection("read_state.rs", "async fn perform_read_network_operation", "async fn run_send_enqueue_future");
  const failures = [];
  for (const marker of ["ReadStateKey::ThreadRead", "ReceiptThread::Thread", "send_single_receipt"]) if (!worker?.includes(marker)) failures.push(sourceContractFailure(rule, `thread read receipt lacks ${marker}`));
  return failures;
}

export function checkCoreTimelineReadCompletionPriority() {
  const rule = "core.timeline.read_completion_priority";
  const run = timelineItemBody("manager.rs", "async fn run(mut self)");
  const completion = run?.indexOf("completion = self.read_workers.tasks.next()") ?? -1;
  const mailbox = run?.indexOf("msg = self.msg_rx.recv()") ?? -1;
  return completion >= 0 && mailbox >= 0 && completion < mailbox
    ? []
    : [sourceContractFailure(rule, "read completion lane does not precede the ordinary mailbox")];
}

export function checkCoreTimelineReplayAttention() {
  const rule = "core.timeline.replay_attention";
  const replay = timelineItemBody("navigation.rs", "fn handle_replay_initial_items");
  return replay?.includes("ThreadAttentionObservation::Replay") && !replay.includes("ThreadAttentionTracker::default()")
    ? []
    : [sourceContractFailure(rule, "thread replay resets semantic attention tracking")];
}

export function checkCoreTimelineReceiptTracking() {
  const rule = "core.timeline.receipt_tracking";
  const builder = timelineSection("relay.rs", "fn koushi_timeline_builder", "struct PreparedRelayRecovery");
  const failures = [];
  if (!builder?.includes("TimelineReadReceiptTracking::MessageLikeEvents")) failures.push(sourceContractFailure(rule, "timeline builder does not use message-like receipt tracking"));
  if (builder?.includes("TimelineReadReceiptTracking::AllEvents")) failures.push(sourceContractFailure(rule, "timeline builder tracks state-event receipts"));
  return failures;
}

export function checkCoreTimelineReceiptObservationDelivery() {
  const rule = "core.timeline.receipt_observation_delivery";
  const diff = timelineItemBody("relay.rs", "async fn handle_diff_batch");
  const delivery = timelineItemBody("item_projection.rs", "async fn emit_receipt_observation_actions");
  const failures = [];
  if (!diff?.includes("emit_live_receipt_observation_actions")) failures.push(sourceContractFailure(rule, "receipt diffs bypass the production observation path"));
  if (!delivery?.includes("send_generation_fenced")) failures.push(sourceContractFailure(rule, "receipt actions lack generation fencing"));
  if (diff?.includes("try_send(vec![action])")) failures.push(sourceContractFailure(rule, "receipt actions use lossy delivery"));
  return failures;
}

export function checkCoreTimelineInitialReceiptObservation() {
  const rule = "core.timeline.initial_receipt_observation";
  const startup = timelineSection("actor.rs", "let initial_receipts = live_event_receipts_from_sdk_items", "let thread_attention = ThreadAttentionTracker::hydrate");
  const failures = [];
  if (!startup?.includes("emit_receipt_observation_actions")) failures.push(sourceContractFailure(rule, "initial receipts bypass local profile observation"));
  if (startup?.includes("LiveRoomReceiptsUpdated {")) failures.push(sourceContractFailure(rule, "initial receipts bypass the ordered receipt batch"));
  if (startup?.includes("try_send(actions)")) failures.push(sourceContractFailure(rule, "initial receipts use lossy delivery"));
  return failures;
}

export function checkCoreTimelineRecoveryReceiptObservation() {
  const rule = "core.timeline.recovery_receipt_observation";
  const recovery = timelineSection("relay.rs", "async fn handle_relay_overflow", "// ---------------------------------------------------------------------------\n// Relay task");
  const failures = [];
  if (!recovery?.includes("emit_receipt_observation_actions")) failures.push(sourceContractFailure(rule, "receipt recovery bypasses local profile observation"));
  if (recovery?.includes("if let Some(action) = receipts_action")) failures.push(sourceContractFailure(rule, "receipt recovery publishes an unobserved action"));
  return failures;
}

export function checkCoreTimelineOriginObserver() {
  const rule = "core.timeline.origin_observer";
  const spawn = timelineItemBody("actor.rs", "pub(super) async fn spawn(");
  const origin = timelineItemBody("diagnostics.rs", "fn event_cache_origin_trace_token");
  const failures = [];
  for (const [body, marker] of [[spawn, "startup_trace::trace_origin"], [spawn, "event_cache()"], [origin, "EventsOrigin"]]) if (!body?.includes(marker)) failures.push(sourceContractFailure(rule, `timeline origin observer lacks ${marker}`));
  return failures;
}

export function checkCoreTimelineRoomFocus() {
  const rule = "core.timeline.room_focus";
  const build = timelineItemBody("manager.rs", "async fn build_timeline_actor_handle");
  const focus = build?.split("let focus = match &key.kind").at(1)?.split("let timeline_result").at(0);
  const room = focus?.split("TimelineKind::Room").at(1)?.split("TimelineKind::Thread").at(0);
  return room?.includes("hide_threaded_events: false")
    ? []
    : [sourceContractFailure(rule, "room live timelines hide threaded events")];
}

export function checkCoreTimelineThreadRootHydration() {
  const rule = "core.timeline.thread_root_hydration";
  const source = timelineSource("thread_projection.rs");
  const hydration = sourceSection(source, "fn maybe_hydrate_missing_thread_roots", "async fn handle_ignored_users_updated");
  const commit = sourceSection(source, "async fn commit_prepared_thread_root_hydration_for_generation", "fn thread_root_projection_action_from_record");
  const failures = [];
  if (!hydration?.includes("missing_activities") || !hydration.includes("commit_prepared_thread_root_hydration_for_generation")) failures.push(sourceContractFailure(rule, "missing roots do not request bounded hydration"));
  for (const marker of ["reserve_owned().await", "ThreadRootProjectionDecision::StartFetch", "schedule_aggregate_refresh", "TimelineMessage::StartAggregateRefresh", "actor_generation"]) if (!commit?.includes(marker)) failures.push(sourceContractFailure(rule, `root hydration commit lacks ${marker}`));
  if (commit?.includes("TimelineMessage::StartThreadRootProjectionFetch") || commit?.includes("try_send")) failures.push(sourceContractFailure(rule, "root hydration commit uses an obsolete or lossy path"));
  if (hydration?.includes("paginate_backwards(") || hydration?.includes("handle_restore_timeline_anchor(")) failures.push(sourceContractFailure(rule, "root hydration initiates pagination or anchor materialization"));
  return failures;
}

export function checkCoreTimelineSdkProjectionAccessors() {
  const rule = "core.timeline.sdk_projection_accessors";
  const projection = timelineItemBody("item_projection.rs", "fn sdk_item_to_timeline_item_with_send_states");
  const compact = projection?.replace(/\s/gu, "") ?? "";
  const failures = [];
  for (const marker of ["content().thread_root()", "content().thread_summary()"]) if (!compact.includes(marker)) failures.push(sourceContractFailure(rule, `SDK projection lacks ${marker}`));
  return failures;
}

export function checkCoreTimelineReceiptAttentionOrdering() {
  const rule = "core.timeline.receipt_attention_ordering";
  const recovery = timelineItemBody("relay.rs", "async fn handle_relay_overflow");
  const startup = timelineItemBody("actor.rs", "pub(super) async fn spawn(");
  const managerCompletion = timelineItemBody("read_state.rs", "async fn handle_read_worker_completion");
  const actorApply = timelineItemBody("read_state.rs", "async fn handle_read_success");
  const settlement = timelineItemBody("read_state.rs", "async fn settle_read_waiters");
  const subscribe = startup?.indexOf("subscribe_own_user_read_receipts_changed") ?? -1;
  const query = startup?.indexOf("latest_user_read_receipt_timeline_event_id") ?? -1;
  const acknowledge = actorApply?.indexOf("thread_attention.acknowledge") ?? -1;
  const reliable = actorApply?.indexOf("emit_action_reliable(action).await") ?? -1;
  const failures = [];
  if (!recovery?.includes("if let Some(action) = self.thread_attention.reconcile") || !recovery.includes("self.emit_action_reliable(action).await")) failures.push(sourceContractFailure(rule, "recovery attention is not reliably projected"));
  if (subscribe < 0 || query < 0 || subscribe >= query) failures.push(sourceContractFailure(rule, "receipt observer subscribes after its initial query"));
  if (!managerCompletion?.includes("self.spawn_read_actor_apply(operation.clone())") || managerCompletion.includes("LiveSignalsEvent::ReadReceiptSent")) failures.push(sourceContractFailure(rule, "read success settles before actor application"));
  if (acknowledge < 0 || reliable < 0 || acknowledge >= reliable) failures.push(sourceContractFailure(rule, "thread attention is emitted before acknowledgement"));
  if (!settlement?.includes("LiveSignalsEvent::ReadReceiptSent")) failures.push(sourceContractFailure(rule, "read receipt success is not owned by manager settlement"));
  return failures;
}

export function checkCoreAccountSessionReplacementTeardown() {
  const rule = "core.account.session_replacement_teardown";
  const install = accountItemBody("session_lifecycle.rs", "async fn install_provisional_session");
  const teardown = accountItemBody("runtime_children.rs", "async fn stop_current_session_runtime");
  const failures = [];
  if (!install?.includes("stop_current_session_runtime().await")) failures.push(sourceContractFailure(rule, "provisional session installation lacks runtime teardown"));
  if (!teardown?.includes("stop_active_session_account_management_discovery")) failures.push(sourceContractFailure(rule, "runtime teardown lacks account-management discovery cancellation"));
  return failures;
}

export function checkCoreAccountReliableReducerDelivery() {
  const rule = "core.account.reliable_reducer_delivery";
  const sources = [
    "account_management.rs", "actor.rs", "local_data_cleanup.rs", "profile.rs",
    "recovery_backup.rs", "routing.rs", "runtime_children.rs", "scheduled_send.rs",
    "session_lifecycle.rs", "sliding_sync.rs", "trust_gate.rs", "verification.rs"
  ].map(accountProductionSource);
  const failures = [];
  const sendActions = accountItemBody("actor.rs", "async fn send_actions");
  if (!sendActions?.includes("self.action_tx.send(actions).await")) failures.push(sourceContractFailure(rule, "send_actions does not await reliable action delivery"));
  if (sources.some((source) => source.includes("self.reduce("))) failures.push(sourceContractFailure(rule, "AccountActor command-result actions use the lossy reduce helper"));
  if (sources.some((source) => source.includes("action_tx.try_send(actions)"))) failures.push(sourceContractFailure(rule, "AccountActor actions use drop-on-full try_send"));
  return failures;
}

export function checkCoreAccountLoginHydrationOrder() {
  const rule = "core.account.login_hydration_order";
  const login = accountItemBody("session_lifecycle.rs", "async fn handle_login_password");
  const promotion = accountItemBody("trust_gate.rs", "async fn handle_trust_projection_applied");
  const failures = [];
  const loggedIn = login?.indexOf("AccountEvent::LoggedIn") ?? -1;
  if (loggedIn < 0) failures.push(sourceContractFailure(rule, "login handler does not emit LoggedIn"));
  for (const marker of [
    "own_profile_action_from_session(&session_arc).await",
    "local_user_aliases_action_from_session(&session_arc).await",
    "ignored_user_ids_action_from_session(&session_arc).await"
  ]) {
    const position = login?.indexOf(marker) ?? -1;
    if (position >= 0 && position <= loggedIn) failures.push(sourceContractFailure(rule, `optional hydration precedes LoggedIn: ${marker}`));
  }
  if (login?.includes("spawn_account_hydration")) failures.push(sourceContractFailure(rule, "login handler spawns account hydration"));
  if (!promotion?.includes("spawn_account_hydration")) failures.push(sourceContractFailure(rule, "trust promotion does not spawn account hydration"));
  return failures;
}

export function checkCoreAccountHydrationGenerationFence() {
  const rule = "core.account.hydration_generation_fence";
  const actor = accountProductionSource("actor.rs");
  const profile = accountProductionSource("profile.rs");
  const failures = [];
  if (!actor.includes("AccountHydrationLoaded {")) failures.push(sourceContractFailure(rule, "account hydration does not return through the actor mailbox"));
  if (!profile.includes("generation != self.account_hydration_generation")) failures.push(sourceContractFailure(rule, "account hydration lacks its generation fence"));
  if (!profile.includes("fn invalidate_account_hydration(&mut self)")) failures.push(sourceContractFailure(rule, "account hydration invalidation helper is missing"));
  return failures;
}

export function checkCoreAccountAliasFailureReconciliation() {
  const rule = "core.account.alias_failure_reconciliation";
  const body = accountItemBody("profile.rs", "async fn handle_set_local_user_alias");
  const failures = [];
  if (!body?.includes("local_user_aliases_action_from_session(session).await")) failures.push(sourceContractFailure(rule, "alias failure does not reload authoritative aliases"));
  for (const marker of ["AppAction::LocalUserAliasUpdateFailed", "AppAction::LocalUserAliasesLoaded"]) if (!body?.includes(marker)) failures.push(sourceContractFailure(rule, `alias failure reconciliation lacks ${marker}`));
  return failures;
}

export function checkCoreAccountSecureBackupMonitorOwner() {
  const rule = "core.account.secure_backup_monitor_owner";
  const recovery = accountProductionSource("recovery_backup.rs");
  const actor = accountProductionSource("actor.rs");
  const scheduler = accountItemBody("recovery_backup.rs", "fn schedule_secure_backup_monitor");
  const retire = accountItemBody("recovery_backup.rs", "fn retire_secure_backup_monitor");
  const inspection = accountItemBody("recovery_backup.rs", "fn start_secure_backup_inspection");
  const failures = [];
  for (const [source, marker] of [[recovery, "const SECURE_BACKUP_MONITOR_INTERVAL: Duration = Duration::from_secs(60);"], [actor, "secure_backup_monitor_task: Option<crate::executor::JoinHandle<()>>"], [retire, "secure_backup_monitor_task.take()"], [scheduler, "SECURE_BACKUP_MONITOR_INTERVAL"], [scheduler, "monitor_serial"], [inspection, "retire_secure_backup_monitor()"]]) if (!source?.includes(marker)) failures.push(sourceContractFailure(rule, `secure-backup monitor is missing ${marker}`));
  return failures;
}

export function checkCoreAccountE2eeTypedFailureClassification() {
  const rule = "core.account.e2ee_typed_failure_classification";
  const recovery = accountProductionSource("recovery_backup.rs");
  const failures = [];
  for (const marker of ["async fn handle_export_room_keys", "async fn handle_import_room_keys", "async fn handle_bootstrap_secure_backup", "async fn handle_change_secure_backup_passphrase"]) {
    const body = accountItemBody("recovery_backup.rs", marker);
    if (!body?.includes("classify_e2ee_trust_error(&error)")) failures.push(sourceContractFailure(rule, `${marker} does not preserve typed SDK failure classification`));
    if (!body?.includes("native_artifacts")) failures.push(sourceContractFailure(rule, `${marker} bypasses the native artifact port`));
  }
  if (!recovery.includes("InvalidPassphrase")) failures.push(sourceContractFailure(rule, "recovery source lacks InvalidPassphrase classification"));
  return failures;
}

export function checkCoreAccountRecoveryKeyHydrationOrder() {
  const rule = "core.account.recovery_key_hydration_order";
  const submit = accountItemBody("recovery_backup.rs", "async fn handle_submit_recovery");
  const complete = accountItemBody("recovery_backup.rs", "async fn complete_recovery_after_verified");
  const failures = [];
  if (!submit?.includes("koushi_sdk::recover_e2ee")) failures.push(sourceContractFailure(rule, "recovery submission does not recover the secret"));
  const request = complete?.indexOf("AppAction::RestoreKeyBackupRequested") ?? -1;
  const restore = complete?.indexOf("koushi_sdk::download_joined_room_keys_from_backup") ?? -1;
  if (request < 0 || restore < 0 || request >= restore) failures.push(sourceContractFailure(rule, "joined-room key hydration does not follow restore-state projection"));
  return failures;
}

export function checkCoreAccountCrawlerNotificationLatestWins() {
  const rule = "core.account.crawler_notification_latest_wins";
  const actor = accountProductionSource("actor.rs");
  const notification = actor.split("AccountMessage::NotifySearchCrawlerRoomsAvailable")[1]?.split("AccountMessage::CurrentDeviceTrustChanged")[0];
  const failures = [];
  if (!notification?.includes("self.pending_crawler_notification = Some")) failures.push(sourceContractFailure(rule, "crawler notification is not retained latest-wins"));
  if (!notification?.includes("self.flush_pending_crawler_notification();")) failures.push(sourceContractFailure(rule, "crawler notification is not flushed without blocking"));
  if (notification?.includes("notify_rooms_available(room_ids, settings).await")) failures.push(sourceContractFailure(rule, "crawler notification awaits background capacity"));
  return failures;
}

export function checkCoreAccountSyncStopRouting() {
  const rule = "core.account.sync_stop_routing";
  const body = accountItemBody("routing.rs", "async fn route_sync_command");
  const failures = [];
  const gate = body?.indexOf("!matches!(command, SyncCommand::Stop { .. })") ?? -1;
  const spawn = body?.indexOf("self.spawn_sync_actor(session.clone()).await") ?? -1;
  const noActor = body?.indexOf("action=no_sync_actor") ?? -1;
  if (gate < 0 || spawn < 0 || noActor < 0 || !(gate < spawn && spawn < noActor)) failures.push(sourceContractFailure(rule, "Sync Stop routing does not separate the missing-actor path"));
  return failures;
}

export function checkCoreAccountManualSyncOnceGuard() {
  const rule = "core.account.manual_sync_once_guard";
  const productionRoute = accountItemBody("routing.rs", "async fn route_sync_command");
  const qaRoute = accountItemBody("routing.rs", "async fn route_sync_once_for_qa");
  const failures = [];
  if (productionRoute?.includes("SyncCommand::SyncOnce")) failures.push(sourceContractFailure(rule, "public Sync command routing still exposes SyncOnce"));
  for (const marker of ["sync_once_for_qa", "CoreFailure::SyncFailed", "SyncFailureKind::Internal"]) if (!qaRoute?.includes(marker)) failures.push(sourceContractFailure(rule, `private SyncOnce QA route lacks ${marker}`));
  return failures;
}

export function checkCoreAccountSessionEstablishedHandoff() {
  const rule = "core.account.session_established_handoff";
  const body = accountItemBody("runtime_children.rs", "async fn spawn_sync_actor");
  const failures = [];
  const handoff = body?.indexOf(".send(RoomMessage::SessionEstablished") ?? -1;
  if (handoff < 0 || !body?.slice(handoff).includes(".await")) failures.push(sourceContractFailure(rule, "RoomActor session handoff is not reliably awaited"));
  if (body?.includes("room_actor.try_send(RoomMessage::SessionEstablished")) failures.push(sourceContractFailure(rule, "RoomActor session handoff uses try_send"));
  return failures;
}

export function checkCoreAccountSecureBackupContentBarrier() {
  const rule = "core.account.secure_backup_content_barrier";
  const cases = [
    ["routing.rs", "async fn route_timeline_command_with_permit_and_formatting_options"],
    ["scheduled_send.rs", "async fn handle_schedule_server_delayed_send"],
    ["scheduled_send.rs", "async fn handle_dispatch_local_scheduled_send"],
    ["scheduled_send.rs", "async fn handle_reschedule_server_delayed_send"]
  ];
  const failures = [];
  for (const [file, marker] of cases) if (!accountItemBody(file, marker)?.includes("admit_secure_backup_user_content")) failures.push(sourceContractFailure(rule, `${marker} lacks the secure-backup barrier`));
  const reschedule = accountItemBody("scheduled_send.rs", "async fn handle_reschedule_server_delayed_send");
  const barrier = reschedule?.indexOf("admit_secure_backup_user_content") ?? -1;
  const cancel = reschedule?.indexOf("UpdateAction::Cancel") ?? -1;
  if (barrier < 0 || cancel < 0 || barrier >= cancel) failures.push(sourceContractFailure(rule, "reschedule cancels before secure-backup admission"));
  return failures;
}

export function checkCoreAccountLocalScheduledSendNoBackupFence() {
  const rule = "core.account.local_scheduled_send_no_backup_fence";
  const body = accountItemBody("scheduled_send.rs", "async fn handle_dispatch_local_scheduled_send");
  return body?.includes(".require_backed_up_session()")
    ? [sourceContractFailure(rule, "local scheduled send has a per-session backup durability fence")]
    : [];
}

export function checkCoreAccountExplicitLogoutTeardown() {
  const rule = "core.account.explicit_logout_teardown";
  const logout = accountItemBody("session_lifecycle.rs", "pub(super) async fn handle_logout");
  const continuation = accountItemBody("session_lifecycle.rs", "match pending.continuation");
  const failures = [];
  if (!logout?.includes("perform_logout(request_id, true, false)")) failures.push(sourceContractFailure(rule, "explicit logout does not select non-preserving teardown"));
  for (const marker of ["preserve_persistence", "forget_last_session_pointer_if_matches(key_id)", "clear_account_persistence(key_id)", "session_persistence_deleted"]) if (!continuation?.includes(marker)) failures.push(sourceContractFailure(rule, `logout continuation lacks ${marker}`));
  return failures;
}

export function checkCoreAccountRestoreEventCacheStatus() {
  const rule = "core.account.restore_event_cache_status";
  const restore = accountItemBody("session_lifecycle.rs", "async fn restore_into_store");
  const helper = accountItemBody("actor.rs", "fn emit_event_cache_status(");
  const prepare = accountItemBody("session_lifecycle.rs", "async fn prepare_store_backed_session");
  const compact = (value) => value?.replace(/\s/gu, "") ?? "";
  const body = compact(restore);
  const helperBody = compact(helper);
  const prepareBody = compact(prepare);
  const failures = [];
  const storeConfig = body.indexOf("self.store.existing_account_store_config(key_id)");
  const restoreCall = body.indexOf("koushi_sdk::restore_session_with_verified_store");
  const encryptedStore = body.indexOf("letencrypted_store=store_config.store_config.encrypted_at_rest_configured();");
  const prepareCall = body.indexOf("self.prepare_store_backed_session(&session,encrypted_store).await");
  const returnOk = body.lastIndexOf("Ok(session)");
  if ([storeConfig, restoreCall, encryptedStore, prepareCall, returnOk].some((position) => position < 0) || !(storeConfig < restoreCall && storeConfig < encryptedStore && restoreCall < prepareCall && encryptedStore < prepareCall && prepareCall < returnOk)) failures.push(sourceContractFailure(rule, "store-backed restore ordering is incomplete"));
  for (const marker of ["koushi_sdk::enable_event_cache(session).await", "self.emit_event_cache_status(encrypted_store,&event_cache_result);"]) if (!prepareBody.includes(marker)) failures.push(sourceContractFailure(rule, `store-backed preparation lacks ${marker}`));
  for (const marker of ["EventCacheSubscribeStatus::Enabled,None", "EventCacheSubscribeStatus::AlreadyEnabled,None", "EventCacheSubscribeStatus::SubscribeFailed,Some(EventCacheFailureReasonClass::SubscribeFailed),"]) if (!helperBody.includes(marker)) failures.push(sourceContractFailure(rule, `event-cache diagnostic lacks ${marker}`));
  if ((prepareBody.match(/self\.emit_event_cache_status\(encrypted_store,&event_cache_result\);/gu) ?? []).length !== 1) failures.push(sourceContractFailure(rule, "event-cache status is not emitted exactly once"));
  for (const marker of ["enable_event_cache(&session).await.map_err", "enable_event_cache(&session).await?", "encrypted_store:true", "cache_path().is_some()"]) if (body.includes(marker) || helperBody.includes(marker)) failures.push(sourceContractFailure(rule, `restore event-cache path contains forbidden ${marker}`));
  return failures;
}

export function checkCoreAccountHomeserverChangeLoginAbort() {
  const rule = "core.account.homeserver_change_login_abort";
  const logout = accountItemBody("session_lifecycle.rs", "async fn perform_logout");
  const abort = accountItemBody("session_lifecycle.rs", "async fn abort_login");
  const failures = [];
  if (!logout?.includes("self.abort_login(login_session, &key_id, false, server_logout)")) failures.push(sourceContractFailure(rule, "logout does not pass server_logout to login abort"));
  for (const marker of ["if server_logout", "koushi_sdk::logout"]) if (!abort?.includes(marker)) failures.push(sourceContractFailure(rule, `login abort lacks ${marker}`));
  return failures;
}

export function checkCoreAccountAuthenticationQuarantine() {
  const rule = "core.account.authentication_quarantine";
  const password = accountItemBody("session_lifecycle.rs", "async fn handle_login_password");
  const restore = accountItemBody("session_lifecycle.rs", "async fn restore_account");
  const continuation = accountItemBody("sliding_sync.rs", "async fn continue_sliding_sync_admission");
  const completion = accountItemBody("sliding_sync.rs", "async fn finish_sliding_sync_capability_discovery");
  const failures = [];
  const before = password?.split("AppAction::LoginSucceeded")[0] ?? "";
  for (const marker of ["begin_sliding_sync_capability_discovery"]) if (!before.includes(marker)) failures.push(sourceContractFailure(rule, `password login lacks ${marker}`));
  for (const marker of ["persist_session(", "spawn_sync_actor(", "install_provisional_session"]) if (before.includes(marker)) failures.push(sourceContractFailure(rule, `password login performs premature ${marker}`));
  const beforeRestore = restore?.split("AppAction::RestoreSessionSucceeded")[0] ?? "";
  if (!beforeRestore.includes("begin_sliding_sync_capability_discovery")) failures.push(sourceContractFailure(rule, "restore lacks capability discovery"));
  for (const marker of ["spawn_sync_actor(", "install_provisional_session"]) if (beforeRestore.includes(marker)) failures.push(sourceContractFailure(rule, `restore performs premature ${marker}`));
  if (!continuation?.includes("install_provisional_session")) failures.push(sourceContractFailure(rule, "sliding-sync admission does not install the provisional session"));
  if (completion?.includes("self.continue_sliding_sync_admission(")) failures.push(sourceContractFailure(rule, "capability completion bypasses reducer continuation"));
  return failures;
}

export function checkCoreAccountRestoreTrace() {
  const rule = "core.account.restore_trace";
  const restoreLast = accountItemBody("session_lifecycle.rs", "async fn handle_restore_last_session");
  const restore = accountItemBody("session_lifecycle.rs", "async fn restore_account");
  const continuation = accountItemBody("sliding_sync.rs", "async fn continue_sliding_sync_admission");
  const actor = accountProductionSource("actor.rs");
  const failures = [];
  for (const marker of ["trace_account_request(\"restore_last_session\", request_id, \"load_pointer\")", "executor::spawn_blocking", "trace_account_request(\"restore_last_session\", request_id, \"pointer_found\")"]) if (!restoreLast?.includes(marker)) failures.push(sourceContractFailure(rule, `startup restore lacks ${marker}`));
  if (!restore?.includes("trace_account_request(\"restore_account\", request_id, \"load_session\")")) failures.push(sourceContractFailure(rule, "restore lacks load-session trace"));
  for (const marker of ["trace_account_request(", "\"restore_account\"", "core_request_id", "\"store_restore_ok\"", "install_provisional_session"]) if (!continuation?.includes(marker)) failures.push(sourceContractFailure(rule, `restore continuation lacks ${marker}`));
  if (restore?.includes("sync_actor_spawned")) failures.push(sourceContractFailure(rule, "restore reports sync actor spawn"));
  if (!actor.includes("DiagnosticField::request_id")) failures.push(sourceContractFailure(rule, "account diagnostics lack request correlation"));
  for (const source of [restoreLast, restore, continuation]) if (source?.includes("account_name()")) failures.push(sourceContractFailure(rule, "restore diagnostics expose an account identifier"));
  return failures;
}

export function checkCoreAccountRestoreDiagnostics() {
  const rule = "core.account.restore_diagnostics";
  const restore = accountItemBody("session_lifecycle.rs", "async fn restore_into_store");
  const recovery = accountItemBody("recovery_backup.rs", "async fn handle_recovery_finished");
  const promotion = accountItemBody("trust_gate.rs", "async fn promote_recovered_session_runtime");
  const failures = [];
  for (const marker of ["\"store_config_ready\"", "\"sdk_restore_begin\"", "\"sdk_restore_ok\""]) if (!restore?.includes(marker)) failures.push(sourceContractFailure(rule, `restore diagnostics lack ${marker}`));
  if (!recovery?.includes("\"post_recovery_trust_read\"")) failures.push(sourceContractFailure(rule, "recovery diagnostics lack post-recovery trust read"));
  for (const marker of ["\"persisted\"", "\"promoted\"", "current_device_trust_token"]) if (!promotion?.includes(marker)) failures.push(sourceContractFailure(rule, `recovery promotion diagnostics lack ${marker}`));
  return failures;
}

export function checkCoreAccountPasswordStoreFirst() {
  const rule = "core.account.password_store_first";
  const login = accountItemBody("session_lifecycle.rs", "async fn handle_login_password");
  const failures = [];
  for (const marker of ["Homeserver::parse", "existing_account_store_config", "pending_login_owner()", "login_with_password_with_new_device", "login_with_password_with_store_and_device"]) if (!login?.includes(marker)) failures.push(sourceContractFailure(rule, `password login lacks ${marker}`));
  for (const marker of ["login_with_existing_device", "fallback_to_fresh_device"]) if (login?.includes(marker)) failures.push(sourceContractFailure(rule, `password login contains forbidden ${marker}`));
  return failures;
}

export function checkCoreAccountSessionChangeObserver() {
  const rule = "core.account.session_change_observer";
  const start = accountItemBody("session_lifecycle.rs", "fn start_session_change_observer");
  const run = accountItemBody("session_lifecycle.rs", "async fn run_session_change_observation");
  const handler = accountItemBody("session_lifecycle.rs", "async fn handle_session_invalidated");
  const failures = [];
  for (const [body, marker] of [[start, "subscribe_to_session_changes()"], [run, "matrix_sdk::SessionChange::UnknownToken(data)"], [run, "soft_logout: data.soft_logout"], [handler, "AppAction::SessionAuthenticationInvalidated"], [handler, "self.stop_current_session_runtime().await"]]) if (!body?.includes(marker)) failures.push(sourceContractFailure(rule, `session-change observer lacks ${marker}`));
  return failures;
}

export function checkCoreAccountSoftLogoutReauth() {
  const rule = "core.account.soft_logout_reauth";
  const reauth = accountItemBody("session_lifecycle.rs", "async fn handle_soft_logout_reauth");
  const logout = accountItemBody("session_lifecycle.rs", "async fn perform_logout");
  const failures = [];
  const positions = ["drop(self.session.take())", "preflight_saved_crypto_store", "login_with_password_with_store_and_device"].map((marker) => reauth?.indexOf(marker) ?? -1);
  if (positions.some((position) => position < 0) || !(positions[0] < positions[1] && positions[1] < positions[2])) failures.push(sourceContractFailure(rule, "reauth does not retire, preflight, and replace in order"));
  for (const marker of ["locked_session_record = Some", "prepare_store_backed_session(&login_session, true)"]) if (!reauth?.includes(marker)) failures.push(sourceContractFailure(rule, `reauth lacks ${marker}`));
  for (const marker of ["locked_session_record.take()", "AppAction::LogoutFinished"]) if (!logout?.includes(marker)) failures.push(sourceContractFailure(rule, `logout lacks ${marker}`));
  return failures;
}

export function checkCoreAccountCredentialStoreBlocking() {
  const rule = "core.account.credential_store_blocking";
  const cases = [
    ["session_lifecycle.rs", "async fn persist_session"],
    ["session_lifecycle.rs", "async fn clear_account_persistence"],
    ["session_lifecycle.rs", "async fn lookup_session_key_id"],
    ["session_lifecycle.rs", "async fn handle_query_saved_sessions"],
    ["local_data_cleanup.rs", "async fn handle_probe_local_encryption_health"]
  ];
  const failures = [];
  for (const [file, marker] of cases) if (!accountItemBody(file, marker)?.includes("executor::spawn_blocking")) failures.push(sourceContractFailure(rule, `${marker} does not use the blocking port`));
  return failures;
}

export function checkCoreAccountSecureBackupLatch() {
  const rule = "core.account.secure_backup_latch";
  const inspection = accountItemBody("recovery_backup.rs", "fn start_secure_backup_inspection");
  const stateChange = accountItemBody("recovery_backup.rs", "async fn handle_secure_backup_state_changed");
  const teardown = accountItemBody("runtime_children.rs", "async fn stop_current_session_runtime");
  const completion = accountItemBody("recovery_backup.rs", "async fn finish_secure_backup_inspection");
  const failures = [];
  if (inspection?.includes("set_secure_backup_send_admitted(false)")) failures.push(sourceContractFailure(rule, "periodic backup inspection closes established admission"));
  for (const [body, marker] of [[stateChange, "set_secure_backup_send_admitted(false)"], [teardown, "set_secure_backup_send_admitted(false)"], [completion, "set_secure_backup_send_admitted(admitted)"]]) if (!body?.includes(marker)) failures.push(sourceContractFailure(rule, `backup latch lacks ${marker}`));
  return failures;
}

export function checkCoreAccountSessionStatusRefreshTeardown() {
  const rule = "core.account.session_status_refresh_teardown";
  const body = accountItemBody("runtime_children.rs", "async fn stop_current_session_runtime");
  return body?.includes("cancel_current_session_status_refresh().await") ? [] : [sourceContractFailure(rule, "runtime teardown does not cancel session-status refresh")];
}

export function checkCoreAccountProvisionalSyncRetry() {
  const rule = "core.account.provisional_sync_retry";
  const owner = accountSection("trust_gate.rs", "fn start_provisional_encryption_sync", "pub(super) async fn stop_provisional_encryption_sync");
  const failures = [];
  const branch = owner?.indexOf("if !first_response_seen.load(Ordering::Acquire)") ?? -1;
  const sleep = owner?.indexOf("executor::sleep(Duration::from_millis(250)).await;") ?? -1;
  const continueAt = sleep >= 0 ? owner.indexOf("continue;", sleep) : -1;
  const failed = continueAt >= 0 ? owner.indexOf("AccountMessage::ProvisionalEncryptionSyncFailed", continueAt) : -1;
  if (branch < 0 || sleep < 0 || continueAt < 0 || failed < 0 || !(branch < sleep && sleep < continueAt && continueAt < failed)) failures.push(sourceContractFailure(rule, "provisional sync retry does not remain under its owner before terminal failure"));
  return failures;
}

export function checkCoreAccountProvisionalSyncFirstResponse() {
  const rule = "core.account.provisional_sync_first_response";
  const owner = accountSection("trust_gate.rs", "fn start_provisional_encryption_sync", "pub(super) async fn stop_provisional_encryption_sync");
  const send = owner?.indexOf("callback_tx.send(message).await.is_ok()") ?? -1;
  const publish = owner?.indexOf("callback_first_response_seen.store(true, Ordering::Release)") ?? -1;
  return send >= 0 && publish >= 0 && send < publish ? [] : [sourceContractFailure(rule, "provisional first-response publication precedes actor delivery")];
}

export function checkCoreAccountAdmissionTimeoutTeardown() {
  const rule = "core.account.admission_timeout_teardown";
  const body = accountItemBody("session_lifecycle.rs", "pub(super) async fn stop_provisional_runtime");
  return body?.includes("cancel_verification_method_discovery_admission_timeout()") ? [] : [sourceContractFailure(rule, "provisional runtime teardown does not cancel admission timeout")];
}

export function checkCoreAccountProvisionalEncryptionSyncService() {
  const rule = "core.account.provisional_encryption_sync_service";
  const body = accountItemBody("trust_gate.rs", "fn start_provisional_encryption_sync");
  const failures = [];
  if (!body?.includes("provisional_encryption_sync_loop")) failures.push(sourceContractFailure(rule, "provisional verification lacks EncryptionSyncService loop"));
  if (body?.includes("restricted_verification_sync_once_with_token")) failures.push(sourceContractFailure(rule, "provisional verification constructs classic sync"));
  return failures;
}

export function checkCoreAccountQaDeviceKeyRefresh() {
  const rule = "core.account.qa_device_key_refresh";
  const helper = accountItemBody("trust_gate.rs", "async fn refresh_device_keys_and_assert_known");
  const query = helper?.indexOf("request_user_identity(&user_id)") ?? -1;
  const device = helper?.indexOf("get_device(&user_id, &device_id)") ?? -1;
  const failures = [];
  if (query < 0 || device < 0 || query >= device) failures.push(sourceContractFailure(rule, "QA device refresh does not query before exact-device assertion"));
  if (device < 0 || !helper?.slice(device).includes(".ok_or(())?")) failures.push(sourceContractFailure(rule, "QA device refresh does not require the exact device"));
  return failures;
}

export function checkCoreAccountVerificationDiscoveryCompletion() {
  const rule = "core.account.verification_discovery_completion";
  const actor = accountProductionSource("actor.rs");
  const completion = actor.split("AccountMessage::VerificationMethodsDiscovered")[1]?.split("AccountMessage::RecoveryFinished")[0];
  const failures = [];
  if (completion?.includes("owned.task.await")) failures.push(sourceContractFailure(rule, "verification discovery completion awaits its sender task"));
  if (!completion?.includes("success_projected")) failures.push(sourceContractFailure(rule, "verification discovery completion lacks success projection diagnostic"));
  return failures;
}

export function checkCoreAccountSasAdoption() {
  const rule = "core.account.sas_adoption";
  const body = accountItemBody("verification.rs", "async fn store_sas_verification(");
  const failures = [];
  const classify = body?.indexOf("resolve_sas_adoption(") ?? -1;
  const earlyReturn = body?.indexOf("return;") ?? -1;
  if (classify < 0 || earlyReturn < 0 || classify >= earlyReturn) failures.push(sourceContractFailure(rule, "SAS adoption classifies after its early return"));
  if (!body?.includes("koushi_sdk::cancel_sas_verification(&handle)")) failures.push(sourceContractFailure(rule, "conflicting SAS handle is not cancelled"));
  for (const marker of ["self.stop_sas_verification_observer().await", "self.sas_verification = Some", "self.start_sas_timeout(", "self.observe_sas_verification(", "koushi_sdk::accept_sas_verification("]) {
    const position = body?.indexOf(marker) ?? -1;
    if (position < 0 || position <= earlyReturn) failures.push(sourceContractFailure(rule, `SAS adoption guard does not precede ${marker}`));
  }
  return failures;
}

export function checkCoreAccountIncomingVerificationAdmission() {
  const rule = "core.account.incoming_verification_admission";
  const body = accountItemBody("verification.rs", "async fn handle_incoming_verification_request");
  const failures = [];
  const positions = ["own_user_active: self.own_user_verification.is_some()", "match decision", "koushi_sdk::cancel_verification_request(&handle).await", "self.verification_request = Some", "self.observe_verification_request("].map((marker) => body?.indexOf(marker) ?? -1);
  if (positions.some((position) => position < 0) || positions[0] >= positions[1] || positions[2] >= positions[3] || positions[2] >= positions[4]) failures.push(sourceContractFailure(rule, "incoming verification admission ordering is incomplete"));
  return failures;
}

export function checkCoreAccountIdentityResetAuthLifecycle() {
  const rule = "core.account.identity_reset_auth_lifecycle";
  const fields = accountItemBody("actor.rs", "pub struct AccountActor {");
  const route = accountItemBody("actor.rs", "async fn handle_command");
  const cancel = accountItemBody("verification.rs", "async fn handle_cancel_identity_reset");
  const required = accountItemBody("recovery_backup.rs", "IdentityResetOutcome::AuthRequired(handle)");
  const timeout = accountItemBody("verification.rs", "async fn handle_identity_reset_auth_timeout");
  const cleanup = accountItemBody("verification.rs", "async fn cancel_identity_reset_handle");
  const failures = [];
  for (const [body, marker] of [[fields, "identity_reset_timeout_task"], [route, "AccountCommand::CancelIdentityReset"], [cancel, "AppAction::ResetIdentityCancelled"], [required, "spawn_identity_reset_auth_timeout"], [timeout, "AppAction::ResetIdentityTimedOut"], [cleanup, "identity_reset_timeout_task"]]) if (!body?.includes(marker)) failures.push(sourceContractFailure(rule, `identity-reset lifecycle lacks ${marker}`));
  return failures;
}

const coreQaSourcePaths = [
  "bin/headless-core-qa.rs",
  "bin/headless_core_qa/registry.rs",
  "bin/headless_core_qa/event_wait.rs",
  "bin/headless_core_qa/participants.rs",
  "bin/headless_core_qa/fixtures.rs",
  "bin/headless_core_qa/cleanup.rs",
  "bin/headless_core_qa/diagnostics.rs",
  "bin/headless_core_qa/orchestrator.rs",
  "bin/headless_core_qa/scenarios/identity.rs",
  "bin/headless_core_qa/scenarios/rooms.rs",
  "bin/headless_core_qa/scenarios/timeline.rs",
  "bin/headless_core_qa/scenarios/search.rs"
];

function coreQaSource(relativePath) {
  const file = `crates/koushi-qa/src/${relativePath}`;
  return productionOnly(readRustSource(file), file);
}

function coreQaProductionSource() {
  return coreQaSourcePaths.map(coreQaSource).join("\n");
}

function coreQaItemBody(relativePath, marker) {
  return rustItemBody(coreQaSource(relativePath), marker);
}

function coreIntegrationSource(relativePath) {
  return readRustSource(`crates/koushi-core-testkit/tests/${relativePath}`);
}

export function checkCoreQaReconnectEncryptionGate() {
  const rule = "core.qa.reconnect_encryption_gate";
  const source = coreQaProductionSource();
  const stage = coreQaItemBody("bin/headless_core_qa/scenarios/timeline.rs", "async fn run_timeline_reconnect_scenario_impl");
  const helper = coreQaItemBody("bin/headless_core_qa/event_wait.rs", "async fn wait_for_encrypted_room_projection_for_qa");
  const failures = [];
  for (const marker of ["create_room_for_qa", "wait_for_encrypted_room_projection_for_qa(", "subscribe_active_timeline_projection_for_qa(", "TimelineCommand::SendText"]) {
    if (!stage?.includes(marker)) failures.push(sourceContractFailure(rule, `reconnect stage lacks ${marker}`));
  }
  for (const marker of ["ROOM_LIST_EVENT_TIMEOUT", "room.room_id == expected_room_id && room.is_encrypted", "RoomEvent::RoomListUpdated", "CoreEvent::StateDelta(_)", "tokio::time::timeout_at(deadline, conn.recv_event())"]) {
    if (!helper?.includes(marker)) failures.push(sourceContractFailure(rule, `encrypted-room waiter lacks ${marker}`));
  }
  if (helper?.includes("tokio::time::sleep")) failures.push(sourceContractFailure(rule, "encrypted-room waiter uses a fixed sleep"));
  const subscribe = stage?.indexOf("subscribe_active_timeline_projection_for_qa(") ?? -1;
  const send = stage?.indexOf("TimelineCommand::SendText") ?? -1;
  const gateA = stage?.indexOf("wait_for_encrypted_room_projection_for_qa(\n            &mut conn_a") ?? -1;
  const gateB = stage?.indexOf("wait_for_encrypted_room_projection_for_qa(\n            &mut conn_b") ?? -1;
  if (subscribe < 0 || send < 0 || gateA < 0 || gateB < 0 || gateA >= subscribe || gateB >= subscribe || gateA >= send || gateB >= send) {
    failures.push(sourceContractFailure(rule, "reconnect encryption gates do not precede timeline work"));
  }
  if (!source.includes("create_room_for_qa")) failures.push(sourceContractFailure(rule, "reconnect source is missing"));
  return failures;
}

export function checkCoreQaPrivateSafeInviteTimeout() {
  const rule = "core.qa.private_safe_invite_timeout";
  const body = coreQaItemBody("bin/headless_core_qa/event_wait.rs", "async fn wait_for_invite_in_snapshot");
  const failures = [];
  for (const marker of ["invite_observer_diagnostic_summary(&koushi_diagnostics::snapshot())", "{observer_diagnostics}"]) if (!body?.includes(marker)) failures.push(sourceContractFailure(rule, `invite timeout lacks ${marker}`));
  if (body?.includes("expected_room_id:?")) failures.push(sourceContractFailure(rule, "invite timeout exposes the expected room id"));
  return failures;
}

export function checkCoreQaNoManualSyncOnce() {
  const rule = "core.qa.no_manual_sync_once";
  const source = coreQaProductionSource();
  const failures = [];
  for (const marker of ["SyncCommand::SyncOnce", "sync_once_for_qa("]) if (source.includes(marker)) failures.push(sourceContractFailure(rule, `manual sync marker remains: ${marker}`));
  return failures;
}

export function checkCoreQaE2eeWaiterDeadline() {
  const rule = "core.qa.e2ee_waiter_deadline";
  const body = coreQaItemBody("bin/headless_core_qa/event_wait.rs", "async fn wait_for_item_with_body_or_decryption_failure");
  const failures = [];
  if (!body?.includes("E2EE_EVENT_TIMEOUT")) failures.push(sourceContractFailure(rule, "E2EE body waiter lacks its extended deadline"));
  if (!body?.includes("tokio::time::timeout_at(deadline, conn.recv_event())")) failures.push(sourceContractFailure(rule, "E2EE body waiter lacks an absolute deadline"));
  if (body?.includes("SyncCommand::SyncOnce")) failures.push(sourceContractFailure(rule, "E2EE body waiter issues manual sync"));
  return failures;
}

export function checkCoreQaMultiDeviceOrder() {
  const rule = "core.qa.multi_device_order";
  const stage = coreQaItemBody("bin/headless_core_qa/scenarios/identity.rs", "async fn verify_multi_user_multi_device_room_key_delivery_for_qa");
  const failures = [];
  const ordered = ["refresh_device_keys_and_assert_known_for_qa(", "TimelineCommand::SendText", "qa_set_local_device_blacklisted(", "let blocked_send"];
  const positions = ordered.map((marker) => stage?.indexOf(marker) ?? -1);
  if (positions.some((position) => position < 0) || positions.some((position, index) => index > 0 && positions[index - 1] >= position)) failures.push(sourceContractFailure(rule, "multi-device verification checkpoints are out of order"));
  for (const marker of ["wait_for_send_flow_completion_with_timeout(", "E2EE_EVENT_TIMEOUT", "wait_for_withheld_event_projection_from_source(", "room_id.clone()", "blocked QA promote B3"]) if (!stage?.includes(marker)) failures.push(sourceContractFailure(rule, `multi-device stage lacks ${marker}`));
  for (const marker of ["AccountCommand::RequestVerification", "SyncCommand::SyncOnce"]) if (stage?.includes(marker)) failures.push(sourceContractFailure(rule, `multi-device stage contains forbidden ${marker}`));
  const helper = coreQaItemBody("bin/headless_core_qa/participants.rs", "async fn refresh_device_keys_and_assert_known_for_qa");
  for (const marker of ["qa_refresh_device_keys_and_assert_known", "tokio::time::timeout(", "E2EE_EVENT_TIMEOUT"]) if (!helper?.includes(marker)) failures.push(sourceContractFailure(rule, `device refresh helper lacks ${marker}`));
  for (const marker of ["AccountCommand::RequestVerification", "tokio::time::sleep"]) if (helper?.includes(marker)) failures.push(sourceContractFailure(rule, `device refresh helper contains forbidden ${marker}`));
  return failures;
}

export function checkCoreQaInviteBeforeOptionalLogin() {
  const rule = "core.qa.invite_before_optional_login";
  const stage = coreQaItemBody("bin/headless_core_qa/scenarios/identity.rs", "async fn verify_multi_user_multi_device_room_key_delivery_for_qa");
  const failures = [];
  const ordered = ["let room_id = create_room_for_qa(", "invite_user_for_qa(", "login_synced_participant_for_qa(", "wait_for_invite_in_snapshot(", "cleanup_e2ee_multi_device_participants"];
  const positions = ordered.map((marker) => stage?.indexOf(marker) ?? -1);
  if (positions.some((position) => position < 0) || positions.some((position, index) => index > 0 && positions[index - 1] >= position)) failures.push(sourceContractFailure(rule, "E2EE invite/login/cleanup order changed"));
  if ((stage?.match(/login_synced_participant_for_qa\(/gu) ?? []).length !== 1) failures.push(sourceContractFailure(rule, "E2EE invite stage owns more than one optional login"));
  if ((stage?.match(/cleanup_e2ee_multi_device_participants/gu) ?? []).length !== 1) failures.push(sourceContractFailure(rule, "E2EE invite stage owns more than one cleanup"));
  for (const marker of ["SyncCommand::SyncOnce", "sync_once_for_qa(", "tokio::time::sleep"]) if (stage?.includes(marker)) failures.push(sourceContractFailure(rule, `E2EE invite stage contains forbidden ${marker}`));
  return failures;
}

export function checkCoreQaTracingAndDeviceLabels() {
  const rule = "core.qa.tracing_and_device_labels";
  const source = coreQaProductionSource();
  const failures = [];
  for (const marker of ["init_headless_qa_tracing_from_env();", "tracing_subscriber::EnvFilter"]) if (!source.includes(marker)) failures.push(sourceContractFailure(rule, `headless QA tracing lacks ${marker}`));
  for (const marker of ["e2ee gated self verification A/A2", "e2ee recipient verification B/B2", "primary incoming request"]) if (!source.includes(marker)) failures.push(sourceContractFailure(rule, `verification labels lack ${marker}`));
  if (source.includes("request secondary to primary")) failures.push(sourceContractFailure(rule, "verification labels retain the obsolete secondary request"));
  return failures;
}

export function checkCoreQaSecondaryRuntimeIsolation() {
  const rule = "core.qa.secondary_runtime_isolation";
  const source = coreQaProductionSource();
  const failures = [];
  for (const label of ["gate-negative-a2", "gate-negative-a3", "gate-negative-a4", "gate-negative-a5", "gate-negative-a6", "a2", "e2ee-b2", "e2ee-b3-unverified"]) {
    if (!source.includes(`start_isolated_qa_runtime("${label}")`)) failures.push(sourceContractFailure(rule, `secondary runtime is not isolated for ${label}`));
  }
  return failures;
}

export function checkCoreQaSendQueueRoute() {
  const rule = "core.qa.send_queue_route";
  const source = coreQaProductionSource();
  const beforeFixture = source.split("async fn run_async").at(1)?.split("// One CoreRuntime per synthetic user").at(0) ?? "";
  const route = source.split("async fn run_focused_send_queue_scenario").at(1)?.split("async fn run_send_queue_stage").at(0) ?? "";
  const failures = [];
  for (const marker of ["should_run_focused_send_queue_route(scenario)", "run_focused_send_queue_scenario(&config).await?", "return Ok(scenario_report(&config.server_kind, scenario))"]) if (!beforeFixture.includes(marker)) failures.push(sourceContractFailure(rule, `focused SendQueue dispatch lacks ${marker}`));
  for (const marker of ["QaParticipantLoginGate::BootstrapNewIdentity", "bootstrap_recovery_secret", "run_send_queue_stage(config, &recovery_secret).await", "runtime.shutdown()", "drop(conn)"]) if (!route.includes(marker)) failures.push(sourceContractFailure(rule, `focused SendQueue route lacks ${marker}`));
  for (const marker of ["user_b", "password_b"]) if (route.includes(marker)) failures.push(sourceContractFailure(rule, `focused SendQueue route exposes ${marker}`));
  return failures;
}

export function checkCoreQaLoginGateLifecycle() {
  const rule = "core.qa.login_gate_lifecycle";
  const source = coreQaProductionSource();
  const failures = [];
  const shared = source.split("--- Login A (persistent store selected before authentication) ---").at(1)?.split("wait_for_logged_in(&mut conn_a, login_a_id").at(0) ?? "";
  if (!shared.includes("complete_new_identity_gate_for_qa(&mut conn_a")) failures.push(sourceContractFailure(rule, "shared primary login does not complete the identity gate"));
  if (shared.includes("should_bootstrap_new_identity_before_logged_in")) failures.push(sourceContractFailure(rule, "shared identity gate is scenario-gated"));
  const helper = source.split("async fn complete_new_identity_gate_for_qa").at(1)?.split("async fn wait_for_existing_identity_gate").at(0) ?? "";
  const confirmation = ["ConfirmSessionBootstrapSaved", "failed == confirm_id", "Ok(Some(recovery_secret))"].map((marker) => helper.indexOf(marker));
  if (confirmation.some((position) => position < 0) || !(confirmation[0] < confirmation[1] && confirmation[1] < confirmation[2])) failures.push(sourceContractFailure(rule, "identity-gate confirmation is not settled before return"));
  if (!helper.includes("timed out settling bootstrap confirmation; phase=")) failures.push(sourceContractFailure(rule, "identity-gate timeout omits its phase"));
  const waiter = source.split("async fn wait_for_logged_in").at(1)?.split("async fn ").at(0) ?? "";
  if (!waiter.includes("timed out waiting for LoggedIn event; phase=") || !waiter.includes("gate_session_phase(&conn.snapshot().session)")) failures.push(sourceContractFailure(rule, "login timeout omits authoritative session phase"));
  return failures;
}

export function checkCoreQaSecondaryLifecycle() {
  const rule = "core.qa.secondary_lifecycle";
  const source = coreQaProductionSource();
  const failures = [];
  const beforeRoom = source.split("async fn run_async").at(1)?.split("// --- Phase 4: Room operations").at(0) ?? "";
  if (!beforeRoom.includes("let mut normal_secondary = if should_run_normal_secondary_participant(scenario)")) failures.push(sourceContractFailure(rule, "central secondary owner is missing"));
  if ((beforeRoom.match(/login_synced_participant_for_qa\(/gu) ?? []).length !== 1) failures.push(sourceContractFailure(rule, "normal secondary login is not centralized"));
  if ((beforeRoom.match(/cleanup_normal_secondary_participant_for_qa\(/gu) ?? []).length !== 2) failures.push(sourceContractFailure(rule, "normal secondary cleanup paths changed"));
  const invites = source.split("async fn run_invites_dm_stage").at(1)?.split("async fn run_directory_stage").at(0) ?? "";
  const directory = source.split("async fn run_directory_stage").at(1)?.split("async fn join_directory_room_for_qa").at(0) ?? "";
  for (const [label, stage] of [["InvitesDm", invites], ["Directory", directory]]) {
    if (!stage.includes("conn_b: &mut CoreConnection")) failures.push(sourceContractFailure(rule, `${label} does not borrow the central connection`));
    for (const marker of ["CoreRuntime::", "AccountCommand::LoginPassword", "wait_for_logged_in", "login_synced_participant_for_qa(", "cleanup_logged_in_runtime"]) if (stage.includes(marker)) failures.push(sourceContractFailure(rule, `${label} owns forbidden lifecycle ${marker}`));
  }
  const cleanup = coreQaSource("bin/headless_core_qa/cleanup.rs").split("async fn cleanup_logged_in_runtime").at(1)?.split("async fn cleanup_normal_secondary_participant_for_qa").at(0) ?? "";
  if (!cleanup.includes("runtime.shutdown().await") || cleanup.includes("drop(runtime)") || cleanup.includes("tokio::time::sleep")) failures.push(sourceContractFailure(rule, "secondary cleanup lacks ordered shutdown"));
  return failures;
}

export function checkCoreQaSendQueueSecretAndCleanup() {
  const rule = "core.qa.send_queue_secret_and_cleanup";
  const source = coreQaProductionSource();
  const stage = source.split("async fn run_send_queue_stage").at(1)?.split("async fn unsubscribe_timeline_for_qa").at(0) ?? "";
  const route = source.split("async fn run_focused_send_queue_scenario").at(1)?.split("async fn run_send_queue_stage").at(0) ?? "";
  const failures = [];
  for (const marker of ["login_synced_participant_for_qa(", "proxy.homeserver_url()", "recovery_secret: &AuthSecret", "QaParticipantLoginGate::RecoverExistingIdentity(recovery_secret)"]) if (!stage.includes(marker)) failures.push(sourceContractFailure(rule, `SendQueue stage lacks ${marker}`));
  for (const marker of ["\n        true,", "AccountCommand::LoginPassword", "wait_for_logged_in"]) if (stage.includes(marker)) failures.push(sourceContractFailure(rule, `SendQueue stage contains forbidden ${marker}`));
  const ordered = ["SyncCommand::Stop", "wait_for_sync_stopped", "AccountCommand::Logout", "wait_for_logged_out", "drop(conn)", "runtime.shutdown()", "send_queue bootstrap recovery secret unavailable", "run_send_queue_stage(config, &recovery_secret).await"].map((marker) => route.indexOf(marker));
  if (ordered.some((position) => position < 0) || ordered.some((position, index) => index > 0 && ordered[index - 1] >= position)) failures.push(sourceContractFailure(rule, "focused SendQueue bootstrap cleanup order changed"));
  return failures;
}

export function checkCoreQaStrictWaiters() {
  const rule = "core.qa.strict_waiters";
  const waiters = [
    ["wait_for_existing_identity_gate", "participants.rs"],
    ["wait_for_room_in_room_list", "event_wait.rs"],
    ["subscribe_active_timeline_projection_for_qa", "scenarios/timeline.rs"],
    ["wait_for_verification_requested_event_only", "participants.rs"],
    ["wait_for_verification_accepted", "participants.rs"],
    ["wait_for_initial_items_from_source", "event_wait.rs"],
    ["wait_for_send_flow_completion_with_timeout", "event_wait.rs"],
    ["wait_for_item_with_body_or_decryption_failure", "event_wait.rs"],
    ["wait_for_withheld_event_projection_from_source", "event_wait.rs"]
  ];
  const failures = [];
  for (const [waiter, file] of waiters) {
    const body = coreQaItemBody(`bin/headless_core_qa/${file}`, `async fn ${waiter}`);
    if (!body?.includes(".recv(") && !body?.includes("recv_event()")) failures.push(sourceContractFailure(rule, `strict waiter is missing its receive loop: ${waiter}`));
    if (body?.includes("tokio::time::timeout(")) failures.push(sourceContractFailure(rule, `strict waiter resets its timeout: ${waiter}`));
  }
  return failures;
}

export function checkCoreQaDeviceCleanup() {
  const rule = "core.qa.device_cleanup";
  const source = coreQaProductionSource();
  const route = source.split("if scenario == QaScenario::DeviceCleanup").at(1)?.split("if scenario == QaScenario::E2eeTrust").at(0) ?? "";
  const proof = source.split("async fn run_provisional_device_cleanup_qa").at(1)?.split("async fn login_until_device_cleanup_offered").at(0) ?? "";
  const failures = [];
  if (!route.includes("run_provisional_device_cleanup_qa(&config).await?")) failures.push(sourceContractFailure(rule, "device-cleanup scenario is not routed"));
  if (!proof.includes("audit_removed_device_absent_from_server")) failures.push(sourceContractFailure(rule, "device-cleanup proof lacks remote audit"));
  for (const token of ["device_cleanup_remote_first=ok", "device_cleanup_relogin_new_device=ok"]) if (!source.includes(token)) failures.push(sourceContractFailure(rule, `device-cleanup token is missing: ${token}`));
  return failures;
}

export function checkCoreQaBackupCausalWaiters() {
  const rule = "core.qa.backup_causal_waiters";
  const source = coreQaProductionSource();
  const seed = source.split("async fn seed_encrypted_room_key_for_qa(").at(1)?.split("async fn enable_key_backup_for_qa(").at(0) ?? "";
  const delivery = source.split("async fn verify_second_device_room_key_delivery_for_qa(").at(1)?.split("async fn verify_multi_user_multi_device_room_key_delivery_for_qa(").at(0) ?? "";
  const secondary = source.split("// B subscribes and receives both messages").at(1)?.split("// Paginate backward on B").at(0) ?? "";
  const failures = [];
  if (seed.includes("sync_once_for_qa(") || !seed.includes("wait_for_room_in_room_list(") || !seed.includes("wait_for_initial_items(") || !seed.includes("subscribe encrypted backup seed")) failures.push(sourceContractFailure(rule, "backup seed does not use live causal waiters"));
  for (const [name, body] of [["second-device", delivery], ["generic-secondary", secondary]]) if (!body.includes("wait_for_initial_items(")) failures.push(sourceContractFailure(rule, `${name} subscription lacks the causal waiter`));
  return failures;
}

export function checkCoreQaTimeoutAndDirectoryOrder() {
  const rule = "core.qa.timeout_and_directory_order";
  const source = coreQaProductionSource();
  const waitBody = source.split("async fn wait_for_send_completions_in_order").at(1)?.split("async fn wait_for_cancelled_or_removed_send").at(0) ?? "";
  const login = source.split("fn ready_account_key").at(1)?.split("async fn wait_for_logged_in").at(0) ?? "";
  const loggedIn = source.split("async fn wait_for_logged_in").at(1)?.split("/// Wait for `AccountEvent::SessionRestored`").at(0) ?? "";
  const run = source.split("async fn run_async").at(1)?.split("async fn cleanup_after_full_flow").at(0) ?? "";
  const directoryCall = "run_directory_stage(&config, &mut conn_a, conn_b).await?";
  const failures = [];
  if (!waitBody.includes("tokio::time::timeout(SEND_QUEUE_EVENT_TIMEOUT, conn.recv_event())") || !waitBody.includes("first_completed={first_completed}")) failures.push(sourceContractFailure(rule, "SendQueue FIFO waiter lacks its dedicated timeout"));
  if (!source.includes("const SEND_QUEUE_EVENT_TIMEOUT: Duration = Duration::from_secs(300);")) failures.push(sourceContractFailure(rule, "SendQueue timeout is not 300 seconds"));
  if (!source.includes("const LOGIN_EVENT_TIMEOUT: Duration = Duration::from_secs(180);") || !loggedIn.includes("QaEventDeadline::after(LOGIN_EVENT_TIMEOUT)") || !loggedIn.includes(".recv(conn)")) failures.push(sourceContractFailure(rule, "login waiter lacks its dedicated timeout"));
  if (!login.includes("SessionState::Ready(info)") || !login.includes("Some(AccountKey(info.user_id))") || (loggedIn.match(/ready_account_key\(conn\)/gu) ?? []).length < 3) failures.push(sourceContractFailure(rule, "login waiter does not accept the authoritative Ready snapshot"));
  const directory = run.indexOf(directoryCall);
  const roomSpace = run.indexOf("// --- Phase 4: Room operations");
  if (directory < 0 || roomSpace < 0 || directory >= roomSpace || run.slice(roomSpace).includes(directoryCall)) failures.push(sourceContractFailure(rule, "directory QA does not precede RoomSpace exactly once"));
  return failures;
}

export function checkCoreQaRuntimeReopenOrder() {
  const rule = "core.qa.runtime_reopen_order";
  const source = coreQaProductionSource();
  const cases = [
    ["cleanup_after_full_flow", "drop(conn_a)", "runtime_a.shutdown().await", "CoreRuntime::start_with_data_dir(data_dir_a)"],
    ["run_async All restore", "drop(conn_a)", "runtime_a.shutdown().await", "CoreRuntime::start_with_data_dir(data_dir_a)"],
    ["cache restore", "drop(conn)", "runtime.shutdown().await", "CoreRuntime::start_with_data_dir(data_dir)"]
  ];
  const failures = [];
  for (const [label, dropConnection, shutdown, reopen] of cases) {
    const start = label === "cleanup_after_full_flow" ? "async fn cleanup_after_full_flow" : label === "cache restore" ? "async fn run_cache_restore_scenario" : "// --- Sync stop A + store-backed restore A + logout A ---";
    const end = label === "cache restore" ? "let mut conn2 = runtime2.attach();" : "let mut conn_a2 = runtime_a2.attach();";
    const slice = source.split(start).at(1)?.split(end).at(0) ?? "";
    const positions = [dropConnection, shutdown, reopen].map((marker) => slice.indexOf(marker));
    if (positions.some((position) => position < 0) || !(positions[0] < positions[1] && positions[1] < positions[2])) failures.push(sourceContractFailure(rule, `${label} does not drop, shutdown, and reopen in order`));
    if (slice.includes("drop(runtime)") || slice.includes("Duration::from_millis(500)")) failures.push(sourceContractFailure(rule, `${label} uses an invalid shutdown shortcut`));
  }
  return failures;
}

export function checkCoreQaTimelineStress() {
  const rule = "core.qa.timeline_stress";
  const source = coreQaProductionSource();
  const wait = source.split("async fn wait_for_stress_bodies_and_no_blank_rows").at(1)?.split("async fn submit_stress_backfill_paginate").at(0) ?? "";
  const replay = source.split("async fn run_timeline_stress_replay_stage").at(1)?.split("struct StressRoomCoordinates").at(0) ?? "";
  const failures = [];
  if (!wait.includes("request_id: ev_id") || !wait.includes("ev_id == &Some(current_paginate_request_id)")) failures.push(sourceContractFailure(rule, "stress backfill does not fence stale pagination state"));
  for (const marker of ["run_timeline_stress_replay_stage", "Subscribe", "submit_stress_backfill_paginate"]) if (!source.includes(marker)) failures.push(sourceContractFailure(rule, `timeline stress lacks ${marker}`));
  for (const marker of ["CreateRoom", "CreateSpace", "SendText"]) if (replay.includes(marker)) failures.push(sourceContractFailure(rule, `timeline replay mutates state with ${marker}`));
  const stress = source.split("async fn run_timeline_stress_stage").at(1)?.split("async fn run_timeline_stress_room_messages").at(0) ?? "";
  if (stress.includes("sync_once_for_qa") || !stress.includes("wait_for_invite_in_snapshot")) failures.push(sourceContractFailure(rule, "timeline stress does not use live event waiters"));
  return failures;
}

export function checkCoreQaRestoreScopeAndPrivacy() {
  const rule = "core.qa.restore_scope_and_privacy";
  const source = coreQaProductionSource();
  const failures = [];
  if (!source.includes('println!("joined_room_restore=ok")') || source.includes("e2ee_key_backup_restore_success=ok")) failures.push(sourceContractFailure(rule, "backup restore scope token is incorrect"));
  for (const token of ["e2ee_second_device_decrypt=ok", "e2ee_multi_user_multi_device_decrypt=ok"]) if (!source.includes(`println!("${token}")`)) failures.push(sourceContractFailure(rule, `restore token is missing: ${token}`));
  if (!source.includes("KOUSHI_QA_ALLOW_IDENTITY_RESET") || !source.includes("if config.allow_identity_reset") || !source.includes('println!("e2ee_identity_reset=skipped")')) failures.push(sourceContractFailure(rule, "identity reset is not explicitly opt-in"));
  for (const marker of ["println!(\"room_id={", "println!(\"space_id={", "println!(\"event_id={", "println!(\"sdk_txn={", "println!(\"transaction_id={"]) if (source.includes(marker)) failures.push(sourceContractFailure(rule, "core QA stdout formats a Matrix identifier"));
  return failures;
}

export function checkCoreQaProvisionalVerification() {
  const rule = "core.qa.provisional_verification";
  const source = coreQaProductionSource();
  const helper = source.split("async fn verify_provisional_second_device_for_qa").at(1)?.split("fn verification_closed_summary").at(0) ?? "";
  const failures = [];
  const refresh = helper.indexOf("refresh_device_keys_and_assert_known_for_qa(");
  const start = helper.indexOf("AccountCommand::StartOwnUserSas");
  if (refresh < 0 || start < 0 || refresh >= start) failures.push(sourceContractFailure(rule, "provisional verification does not refresh before SAS"));
  for (const marker of ["target_a2.clone()", "primary incoming request", "SasQaOutcome::Timeout", "SasQaOutcome::Mismatch", "AccountCommand::CancelVerification", "AccountCommand::ConfirmSasVerification", "timed out waiting for authoritative Ready"]) if (!helper.includes(marker)) failures.push(sourceContractFailure(rule, `provisional verification lacks ${marker}`));
  for (const marker of ["stop_sync_for_qa(conn_a", "start_sync_for_qa(conn_a", "sync_once_for_qa(conn_a", "stop_sync_for_qa(conn_a2", "start_sync_for_qa(conn_a2"]) if (helper.includes(marker)) failures.push(sourceContractFailure(rule, `provisional verification stops normal sync: ${marker}`));
  return failures;
}

export function checkCoreQaIncomingVerificationWaiter() {
  const rule = "core.qa.incoming_verification_waiter";
  const source = coreQaSource("bin/headless_core_qa/participants.rs");
  const guard = source.split("fn ensure_incoming_verification_receiver_sync_not_stopped").at(1)?.split("async fn wait_for_verification_requested_event_only").at(0) ?? "";
  const waiter = source.split("async fn wait_for_verification_requested_event_only").at(1)?.split("fn requested_verification_flow_id").at(0) ?? "";
  const failures = [];
  for (const marker of ["koushi_state::SyncState::Stopped", "receiver sync is stopped; cannot await an incoming verification request"]) if (!guard.includes(marker)) failures.push(sourceContractFailure(rule, `incoming verification guard lacks ${marker}`));
  if (guard.includes("{sync:?}")) failures.push(sourceContractFailure(rule, "incoming verification guard formats sync state"));
  const syncGuard = waiter.indexOf("ensure_incoming_verification_receiver_sync_not_stopped(&conn.snapshot().sync, label)?");
  const deadline = waiter.indexOf("let deadline");
  if (syncGuard < 0 || deadline < 0 || syncGuard >= deadline) failures.push(sourceContractFailure(rule, "incoming verification waiter checks sync after its deadline"));
  return failures;
}

export function checkCoreQaNoObsoleteVerificationCascade() {
  const rule = "core.qa.no_obsolete_verification_cascade";
  const source = coreQaProductionSource();
  const failures = [];
  for (const marker of ["async fn verify_second_device_for_qa", "enum VerificationRequestAttempt", "async fn request_device_verification_for_qa", "async fn wait_for_verification_requested_or_failed", "async fn wait_for_verification_accepted_with_sync_once", "async fn drive_until_both_verification_sas", "async fn wait_for_verification_done", "fn verification_state_done"]) if (source.includes(marker)) failures.push(sourceContractFailure(rule, `obsolete verification orchestration remains: ${marker}`));
  return failures;
}

export function checkCoreIntegrationSelectRoomRouting() {
  const rule = "core.integration.select_room_routing";
  const runtime = coreSource("runtime.rs");
  const room = protocolSource("command/room.rs");
  const failures = [];
  for (const marker of ["User-intent lane: for SelectRoom, record the request_id→room_id", "terminal IntentLifecycle outcome", "AccountMessage::RoomCommand(room_command)", ".await;"]) if (!runtime.includes(marker)) failures.push(sourceContractFailure(rule, `SelectRoom route lacks ${marker}`));
  if (runtime.includes("try_send(crate::account::AccountMessage::RoomCommand")) failures.push(sourceContractFailure(rule, "SelectRoom uses lossy routing"));
  if (!room.includes("User-intent lane: room selection is request-id correlated")) failures.push(sourceContractFailure(rule, "RoomCommand SelectRoom lacks its correlation comment"));
  return failures;
}

export function checkCoreIntegrationRoomListReadiness() {
  const rule = "core.integration.room_list_readiness";
  const source = coreSource("sync.rs");
  const failures = [];
  for (const marker of ["committed_all_rooms_response", "room_list_service: Arc<", "room_list_service,"]) if (!source.includes(marker)) failures.push(sourceContractFailure(rule, `room-list readiness lacks ${marker}`));
  for (const marker of ["note_room_list_service_state", "probe_backend", "run_legacy_sync_loop"]) if (source.includes(marker)) failures.push(sourceContractFailure(rule, `room-list readiness contains forbidden ${marker}`));
  return failures;
}

export function checkCoreIntegrationNoLegacyModeVocabulary() {
  const rule = "core.integration.no_legacy_mode_vocabulary";
  const source = [
    protocolSource("event.rs"),
    ...["state_delta.rs", "sync.rs", "room.rs"].map(coreSource)
  ].join("\n");
  const failures = [];
  for (const marker of ["SyncBackendKind", "LegacySync", "ModeChanged", "SyncMode", "sync_mode", "RoomListSource::Legacy", "RoomListSource::SyncService"]) if (source.includes(marker)) failures.push(sourceContractFailure(rule, `legacy sync vocabulary remains: ${marker}`));
  return failures;
}

export function checkCoreIntegrationTimelineNoLegacyCheckpoint() {
  const rule = "core.integration.timeline_no_legacy_checkpoint";
  const source = coreSource("timeline.rs");
  const failures = [];
  for (const marker of ["/_matrix/client/v3/sync", "LegacyResponseCommitted", "MatrixCommittedRoomTimelineBackend", "RoomAbsent"]) if (source.includes(marker)) failures.push(sourceContractFailure(rule, `legacy timeline checkpoint remains: ${marker}`));
  return failures;
}

export function checkCoreQaFastSendQueueLifecycle() {
  const rule = "core.qa.fast_send_queue_lifecycle";
  const source = coreIntegrationSource("send_queue_fast.rs");
  const lane = source.split("async fn run_fast_send_queue_feedback").at(-1)?.split("async fn fast_send_queue_feedback_runs_production_runtime_without_homeserver").at(0) ?? "";
  const compact = lane.split(/\s+/u).join(" ").replaceAll("( ", "(");
  const failures = [];
  if (!/timeout\s*\(\s*FAST_SEND_QUEUE_TOTAL_TIMEOUT/u.test(source)) failures.push(sourceContractFailure(rule, "fast SendQueue lane lacks its whole-test timeout"));
  for (const phase of ["initial trust configure", "login command", "LoggedIn event", "ready snapshot", "room-list snapshot", "initial stop sync", "initial subscribe command", "initial subscribe replay", "replacement sync start", "stop replacement sync", "first retry command", "cancel command", "first shutdown barrier", "restored trust configure", "restore command", "SessionRestored event", "restored ready snapshot", "restored stop sync", "restored subscribe command", "restored subscribe replay", "restored retry command", "final shutdown barrier"]) if (!compact.includes(`fast_send_queue_phase("fast_send_queue ${phase}",`)) failures.push(sourceContractFailure(rule, `fast SendQueue phase is not bounded: ${phase}`));
  return failures;
}

export function checkCoreQaSendQueueCausalWaiter() {
  const rule = "core.qa.send_queue_causal_waiter";
  const source = coreQaSource("bin/headless_core_qa/scenarios/timeline.rs");
  const stage = source.split("\npub(super) async fn run_send_queue_stage(").at(1)?.split("\npub(super) async fn unsubscribe_timeline_for_qa(").at(0) ?? "";
  return (stage.match(/wait_for_initial_items\(/gu) ?? []).length === 2 ? [] : [sourceContractFailure(rule, "SendQueue subscriptions do not both use the causal waiter")];
}

export function checkCoreQaSendQueueDiagnosticCounters() {
  const rule = "core.qa.send_queue_diagnostic_counters";
  const source = coreQaSource("bin/headless_core_qa/diagnostics.rs");
  const classifier = source.split("\nfn qa_proxy_request_kind(").at(1)?.split("\nfn qa_messages_proxy_action(").at(0) ?? "";
  const proxy = source.split("\nfn proxy_single_http_request(").at(1)?.split("\nfn qa_proxy_request_kind(").at(0) ?? "";
  const compact = proxy.split(/\s+/u).join(" ");
  const failures = [];
  for (const marker of ['("PUT", path)', 'path.contains("/rooms/")', 'path.contains("/send/")', "QaProxyRequestKind::RoomSend"]) if (!classifier.includes(marker)) failures.push(sourceContractFailure(rule, `proxy classifier lacks ${marker}`));
  if (!compact.includes("request_kind == QaProxyRequestKind::RoomSend && action == QaProxyRequestAction::Forward")) failures.push(sourceContractFailure(rule, "RoomSend forwarding predicate is missing"));
  const counter = proxy.indexOf("room_send_forwarded.fetch_add(1, Ordering::SeqCst);");
  const write = proxy.indexOf("io::Write::write_all(&mut server, &request)?;");
  const copy = proxy.indexOf("io::copy(&mut server, client)?;");
  const completed = proxy.indexOf("room_send_responses_completed.fetch_add(1, Ordering::SeqCst);");
  if (counter < 0 || write < 0 || counter >= write || copy < 0 || completed < 0 || copy >= completed) failures.push(sourceContractFailure(rule, "proxy counters are not fenced around forwarding"));
  for (const marker of ["room_send_forwarded: Arc<AtomicUsize>", "fn room_send_forwarded_count(&self) -> usize", "room_send_responses_completed: Arc<AtomicUsize>", "fn room_send_responses_completed_count(&self) -> usize"]) if (!source.includes(marker)) failures.push(sourceContractFailure(rule, `proxy counter API lacks ${marker}`));
  return failures;
}

export function checkCoreQaSendQueueProxyDeltas() {
  const rule = "core.qa.send_queue_proxy_deltas";
  const source = coreQaSource("bin/headless_core_qa/scenarios/timeline.rs");
  const stage = source.split("\npub(super) async fn run_send_queue_stage(").at(1)?.split("\npub(super) async fn unsubscribe_timeline_for_qa(").at(0) ?? "";
  const retry = stage.split("    proxy.enable();").at(1)?.split('    println!("resend=ok");').at(0) ?? "";
  const compact = retry.split(/\s+/u).join(" ");
  const failures = [];
  const baseline = compact.indexOf("let room_send_forwarded_before_retry = proxy.room_send_forwarded_count();");
  const completed = compact.indexOf("let room_send_responses_completed_before_retry = proxy.room_send_responses_completed_count();");
  const retryCommand = compact.indexOf("let retry_id = retry_send_queue_item(");
  if (baseline < 0 || completed < 0 || retryCommand < 0 || baseline >= retryCommand || completed >= retryCommand) failures.push(sourceContractFailure(rule, "FIFO retry lacks pre-command proxy baselines"));
  for (const marker of ["room_send_forwarded_after_retry={}", "room_send_responses_completed_after_retry={}", "saturating_sub(room_send_forwarded_before_retry)", "saturating_sub(room_send_responses_completed_before_retry)"]) if (!retry.includes(marker)) failures.push(sourceContractFailure(rule, `FIFO retry lacks ${marker}`));
  return failures;
}

export function checkCoreQaSendQueuePrivateSafeFailure() {
  const rule = "core.qa.send_queue_private_safe_failure";
  const source = coreQaSource("bin/headless_core_qa/scenarios/timeline.rs");
  const observer = source.split("\nfn observe_send_queue_retry_item_state(").at(1)?.split("\npub(super) async fn wait_for_send_completions_in_order(").at(0) ?? "";
  const waiter = source.split("\npub(super) async fn wait_for_send_completions_in_order(").at(1)?.split("\npub(super) async fn wait_for_cancelled_or_removed_send(").at(0) ?? "";
  const failures = [];
  for (const marker of ["first_left_not_sent_after_retry: &mut bool", "if *first_left_not_sent_after_retry", "*first_left_not_sent_after_retry = true;", "TimelineSendFailureReason::Recoverable", 'Some("recoverable")', "TimelineSendFailureReason::Unrecoverable", 'Some("unrecoverable")']) if (!observer.includes(marker)) failures.push(sourceContractFailure(rule, `SendQueue observer lacks ${marker}`));
  if (observer.includes("format!(")) failures.push(sourceContractFailure(rule, "SendQueue observer formats a dynamic failure"));
  for (const marker of ["TimelineEvent::InitialItems", "TimelineEvent::ItemsUpdated", "visit_timeline_diff_items(&diffs", "let mut first_left_not_sent_after_retry = false;"]) if (!waiter.includes(marker)) failures.push(sourceContractFailure(rule, `SendQueue waiter lacks ${marker}`));
  if ((waiter.match(/observe_send_queue_retry_item_state\(/gu) ?? []).length !== 2) failures.push(sourceContractFailure(rule, "SendQueue waiter does not share its observer"));
  if (waiter.split('"{label}: first queued send returned to NotSent reason={reason}"').length - 1 !== 2) failures.push(sourceContractFailure(rule, "SendQueue waiter does not share its fixed diagnostic"));
  for (const marker of ["request_id == retry_request_id", '"{label}: retry operation failed"', '"{label}: queued send operation failed"']) if (!waiter.includes(marker)) failures.push(sourceContractFailure(rule, `SendQueue waiter lacks ${marker}`));
  for (const marker of ["{failure:?}", "{transaction_id}", "sdk_transaction_id}"]) if (waiter.includes(marker)) failures.push(sourceContractFailure(rule, `SendQueue waiter exposes ${marker}`));
  return failures;
}

export function checkCoreIntegrationFastSendQueueCompletion() {
  const rule = "core.integration.fast_send_queue_completion";
  const source = coreIntegrationSource("send_queue_fast.rs");
  const helper = source.split("async fn wait_for_fast_send_queue_authoritative_completion").at(1)?.split("async fn wait_for_fast_send_queue_flow_completion").at(0) ?? "";
  const failures = [];
  if (!helper.includes("fast_send_queue_authoritative_projection")) failures.push(sourceContractFailure(rule, "restored completion does not validate the projection"));
  const sendCompleted = helper.split("TimelineEvent::SendCompleted").at(1)?.split("CoreEvent::OperationFailed").at(0) ?? "";
  if (sendCompleted.includes("return Ok(event_id)")) failures.push(sourceContractFailure(rule, "SendCompleted alone settles restored completion"));
  return failures;
}

export function runSourceContractRules() {
  return [
    checkCoreAccountSessionReplacementTeardown(),
    checkCoreAccountReliableReducerDelivery(),
    checkCoreAccountLoginHydrationOrder(),
    checkCoreAccountHydrationGenerationFence(),
    checkCoreAccountAliasFailureReconciliation(),
    checkCoreAccountSecureBackupMonitorOwner(),
    checkCoreAccountE2eeTypedFailureClassification(),
    checkCoreAccountRecoveryKeyHydrationOrder(),
    checkCoreAccountCrawlerNotificationLatestWins(),
    checkCoreAccountSyncStopRouting(),
    checkCoreAccountManualSyncOnceGuard(),
    checkCoreAccountSessionEstablishedHandoff(),
    checkCoreAccountSecureBackupContentBarrier(),
    checkCoreAccountLocalScheduledSendNoBackupFence(),
    checkCoreAccountExplicitLogoutTeardown(),
    checkCoreAccountRestoreEventCacheStatus(),
    checkCoreAccountHomeserverChangeLoginAbort(),
    checkCoreAccountAuthenticationQuarantine(),
    checkCoreAccountRestoreTrace(),
    checkCoreAccountRestoreDiagnostics(),
    checkCoreAccountPasswordStoreFirst(),
    checkCoreAccountSessionChangeObserver(),
    checkCoreAccountSoftLogoutReauth(),
    checkCoreAccountCredentialStoreBlocking(),
    checkCoreAccountSecureBackupLatch(),
    checkCoreAccountSessionStatusRefreshTeardown(),
    checkCoreAccountProvisionalSyncRetry(),
    checkCoreAccountProvisionalSyncFirstResponse(),
    checkCoreAccountAdmissionTimeoutTeardown(),
    checkCoreAccountProvisionalEncryptionSyncService(),
    checkCoreAccountQaDeviceKeyRefresh(),
    checkCoreAccountVerificationDiscoveryCompletion(),
    checkCoreAccountSasAdoption(),
    checkCoreAccountIncomingVerificationAdmission(),
    checkCoreAccountIdentityResetAuthLifecycle(),
    checkCoreRuntimeRoleCommandPendingRoute(),
    checkCoreRuntimeActivityMarkReadRoute(),
    checkCoreRuntimeThreadEffectExecution(),
    checkCoreRuntimeStartSyncEffectExecution(),
    checkCoreRuntimeSessionCleanupEffectExecution(),
    checkCoreRuntimeTrustRecheckEffectExecution(),
    checkCoreRuntimeSessionStatusEffectExecution(),
    checkCoreRuntimePersistenceBlockingPort(),
    checkCoreRuntimeSubscribeTimelineEffect(),
    checkCoreRuntimeNavigationReplay(),
    checkCoreRuntimeClosedTimelineRoute(),
    checkCoreRuntimeActorStartSyncEffect(),
    checkCoreRuntimeSyncTrace(),
    checkCoreRuntimeThreadReplacement(),
    checkCoreRuntimeFocusedReplacement(),
    checkCoreRuntimeFocusedCacheRepair(),
    checkCoreRuntimeRoomSwitchPagination(),
    checkCoreRuntimeRoomSwitchLinkPreviews(),
    checkCoreRuntimeConnectionCommandHandle(),
    checkCoreRuntimeCoalescerBaseline(),
    checkCoreRuntimeTimestampActivityProjection(),
    checkCoreRuntimeExecutorBlockingPort(),
    checkCoreRuntimeSendHttpTimeout(),
    checkCoreRuntimeThumbnailPaths(),
    checkCoreRoomActorCommandLoop(),
    checkCoreRoomSyncStartedOwner(),
    checkCoreRoomDirectoryJoinOrder(),
    checkCoreRoomLiveDirectSubscriptionOrder(),
    checkCoreRoomListNoLegacyProjection(),
    checkCoreRoomListRelayOrder(),
    checkCoreRoomListKnownBookDelivery(),
    checkCoreRoomMentionMembershipRefresh(),
    checkCoreRoomMarkReadOrder(),
    checkCoreRoomTagNoStaleRefresh(),
    checkCoreRoomCreateLinksBeforeCompletion(),
    checkCoreRoomMissingSpaceChildRepair(),
    checkCoreRoomPinSettlementOrder(),
    checkCoreRoomPinCommandGuard(),
    checkCoreRoomSpaceMemberFailureProjection(),
    checkCoreRoomSpaceMemberBackgroundFailure(),
    checkCoreRoomSpaceInviteCancellationOrder(),
    checkCoreStoreFileCredentialCfg(),
    checkCoreSearchQueryFailureClassification(),
    checkCoreSearchQueryPriority(),
    checkCoreSearchEmptyQueryOwnership(),
    checkCoreSearchCrawlerRoundRobin(),
    checkCoreSearchCrawlerPruning(),
    checkCoreSearchCrawlerAccountWork(),
    checkCoreSearchAvailabilityNonblocking(),
    checkCoreSearchCrawlerLifecycle(),
    checkCoreSearchPreemptedPageRequeue(),
    checkCoreSearchStartupDelay(),
    checkCoreSearchPageSingleFetch(),
    checkCoreSearchPageWorkKind(),
    checkCoreSearchPageStartupTrace(),
    checkCoreSearchPageCancellation(),
    checkCoreSyncSingleAllRoomsOwner(),
    checkCoreSyncCommittedResponseHandoff(),
    checkCoreSyncTimelineCommitBeforeReadiness(),
    checkCoreSyncTerminatedOwnerRestart(),
    checkCoreThreadsAggregateRefreshCallers(),
    checkCoreThreadsRootProjectionNoPagination(),
    checkCoreThreadsOpenSubscriptionInitialPage(),
    checkCoreThreadsPaginationRequestCorrelation(),
    checkCoreThreadsReliableRelays(),
    checkCoreTimelineUnsubscribeCleanupOrder(),
    checkCoreTimelineStartupTrace(),
    checkCoreTimelineTraceTokens(),
    checkCoreTimelineGapRepairFailureResume(),
    checkCoreTimelineGapInspectionResume(),
    checkCoreTimelineGapRepairScheduler(),
    checkCoreTimelineProfileChangeProjection(),
    checkCoreTimelineSearchReliableDelivery(),
    checkCoreTimelineMediaAttentionReliableDelivery(),
    checkCoreTimelineRetryQueueOrder(),
    checkCoreTimelineCancelQueueOrder(),
    checkCoreTimelineSignalTraces(),
    checkCoreTimelineLinkPreviewTrace(),
    checkCoreTimelineLinkPreviewOffLoop(),
    checkCoreTimelineLinkPreviewCancellation(),
    checkCoreTimelineInitialSearchForward(),
    checkCoreTimelineSubscribeSuccess(),
    checkCoreTimelineSubscribeReliableSettles(),
    checkCoreTimelineThreadFocus(),
    checkCoreTimelineIdempotentSubscribe(),
    checkCoreTimelineSyncStartedRebuild(),
    checkCoreTimelineEnsureSubscribed(),
    checkCoreTimelineReplaySubscribed(),
    checkCoreTimelineMediaDownloadLifecycle(),
    checkCoreTimelineMediaDownloadDiagnostics(),
    checkCoreTimelinePaginationScheduler(),
    checkCoreTimelinePaginationCancellation(),
    checkCoreTimelinePaginationTerminalRelease(),
    checkCoreTimelineRestoreRoomBounded(),
    checkCoreTimelineRestoreBudget(),
    checkCoreTimelineRestoreCoalescing(),
    checkCoreTimelineRestoreTerminal(),
    checkCoreTimelineSendAdmissionGuard(),
    checkCoreTimelineSendCompletionGuard(),
    checkCoreTimelineSendSubmissionRoute(),
    checkCoreTimelineThreadReplySubmissionRoute(),
    checkCoreTimelineThreadComposerRoute(),
    checkCoreTimelineOutboundState(),
    checkCoreTimelineSendSupervision(),
    checkCoreTimelineRoomReadMarker(),
    checkCoreTimelineThreadReadReceipts(),
    checkCoreTimelineReadCompletionPriority(),
    checkCoreTimelineReplayAttention(),
    checkCoreTimelineReceiptTracking(),
    checkCoreTimelineReceiptObservationDelivery(),
    checkCoreTimelineInitialReceiptObservation(),
    checkCoreTimelineRecoveryReceiptObservation(),
    checkCoreTimelineOriginObserver(),
    checkCoreTimelineRoomFocus(),
    checkCoreTimelineThreadRootHydration(),
    checkCoreTimelineSdkProjectionAccessors(),
    checkCoreTimelineReceiptAttentionOrdering(),
    checkStateFocusedContextReducerContract(),
    checkStateHasNoLegacySyncModeVocabulary(),
    checkSdkPasswordSmokeRuntimeSafety(),
    checkSdkClientStoreConfigContract(),
    checkSdkDesktopClientBuilderDefaults(),
    checkSdkBackupDownloadDefault(),
    checkSdkRecoveryUsesSdkSignaturePublication(),
    checkSdkRecoverySignatureRoundTripContract(),
    checkSdkRoomReadMarkerContract(),
    checkSdkSpaceInviteCancellationContract(),
    checkSdkRoomTagMethods(),
    checkSdkPinnedEventMethods(),
    checkSdkRoomManagementMethods(),
    checkSdkJoinedRoomListDirectDetection(),
    checkSdkJoinedRoomListAvoidsFullMemberScans(),
    checkSdkDmResolutionCandidates(),
    checkSdkSpaceMemberIdsNoSync(),
    checkSdkJoinedOnlySpaceMemberProjection(),
    checkSdkSpaceLookupFailuresPropagate(),
    checkSdkFailedSpaceMemberCountsUnavailable(),
    checkSdkRoomMemberSummariesUseFullMembers(),
    checkSdkDirectAccountDataLoaderIsLocalOnly(),
    checkSdkDirectAccountDataServerFallback(),
    checkSdkSlidingSyncInviteProbeContract(),
    checkSdkSessionBackupFence(),
    checkSdkLibrarySourceManifest(),
    checkSdkCommittedRoomCheckpointHasNoLegacyApi(),
    checkDesktopTauriCommandRegistrationContract(),
    checkDesktopSubmitCoreCommandContract(),
    checkDesktopEventWaitLagContract(),
    checkDesktopFailureWaiterContract(),
    checkDesktopActivityNavigationContract(),
    checkDesktopActivityCommandContract(),
    checkDesktopLoginWaitContract(),
    checkDesktopE2eeCommandContract(),
    checkDesktopLocalEncryptionCommandContract(),
    checkDesktopProfileCommandContract(),
    checkDesktopDirectoryStartDmContract(),
    checkDesktopDirectoryJoinRoomContract(),
    checkDesktopRoomOperationContract(),
    checkDesktopSpaceOperationContract(),
    checkDesktopSearchCommandContract(),
    checkDesktopSettingsCommandContract(),
    checkDesktopNavigationContract(),
    checkDesktopSpaceTraceContract(),
    checkDesktopTimelineCommandContract(),
    checkDesktopTimelineSignalContract(),
    checkDesktopScheduledSendCommandContract(),
    checkDesktopSendQueueCommandContract(),
    checkDesktopForwarderLagRecoveryContract(),
    checkDesktopQaControlPipeContract(),
    checkDesktopNativeWindowLifecycleContract(),
    checkDesktopNativeReopenContract(),
    checkDesktopViewportAdapterIsolationContract(),
    checkCoreQaReconnectEncryptionGate(),
    checkCoreQaPrivateSafeInviteTimeout(),
    checkCoreQaNoManualSyncOnce(),
    checkCoreQaE2eeWaiterDeadline(),
    checkCoreQaMultiDeviceOrder(),
    checkCoreQaInviteBeforeOptionalLogin(),
    checkCoreQaTracingAndDeviceLabels(),
    checkCoreQaSecondaryRuntimeIsolation(),
    checkCoreQaSendQueueRoute(),
    checkCoreQaLoginGateLifecycle(),
    checkCoreQaSecondaryLifecycle(),
    checkCoreQaSendQueueSecretAndCleanup(),
    checkCoreQaStrictWaiters(),
    checkCoreQaDeviceCleanup(),
    checkCoreQaBackupCausalWaiters(),
    checkCoreQaTimeoutAndDirectoryOrder(),
    checkCoreQaRuntimeReopenOrder(),
    checkCoreQaTimelineStress(),
    checkCoreQaRestoreScopeAndPrivacy(),
    checkCoreQaProvisionalVerification(),
    checkCoreQaIncomingVerificationWaiter(),
    checkCoreQaNoObsoleteVerificationCascade(),
    checkCoreIntegrationSelectRoomRouting(),
    checkCoreIntegrationRoomListReadiness(),
    checkCoreIntegrationNoLegacyModeVocabulary(),
    checkCoreIntegrationTimelineNoLegacyCheckpoint(),
    checkCoreQaFastSendQueueLifecycle(),
    checkCoreQaSendQueueCausalWaiter(),
    checkCoreQaSendQueueDiagnosticCounters(),
    checkCoreQaSendQueueProxyDeltas(),
    checkCoreQaSendQueuePrivateSafeFailure(),
    checkCoreIntegrationFastSendQueueCompletion()
  ].flat();
}

export function analyzeRustSource(source, options = {}) {
  const root = path.resolve(options.repositoryRoot ?? repositoryRoot);
  const fileName = displayPath(normalizeFilePath(options.filePath ?? "fixture.rs", root), root);
  const modules = moduleInventory(source, fileName);
  const includes = findIncludeStrInvocations(source, options.filePath ?? "fixture.rs", { ...options, repositoryRoot: root });
  const rustSourceIncludes = includes.filter(({ rustSource }) => rustSource);
  const nonRustArtifacts = includes.filter(({ allowedNonRust }) => allowedNonRust);
  const unexpectedArtifacts = includes.filter(({ target, rustSource, allowedNonRust }) => target !== "<unresolved>" && !rustSource && !allowedNonRust);
  const violations = [...modules.errors.map((message) => ({ kind: "parse", message }))];
  for (const module of modules.nested) {
    violations.push({ kind: "nested-module", file: module.file, line: module.line, name: module.name });
  }
  for (const module of modules.inline.filter(({ overThreshold }) => overThreshold)) {
    violations.push({ kind: "inline-module", file: module.file, line: module.line, name: module.name, physicalLines: module.physicalLines });
  }
  for (const include of rustSourceIncludes) violations.push({ kind: "rust-source-include", ...include });
  for (const include of unexpectedArtifacts) violations.push({ kind: "unexpected-include", ...include });
  for (const include of includes.filter(({ exists }) => !exists)) violations.push({ kind: "unresolved-include", ...include });
  return {
    ...modules,
    inlineTestModules: modules.inline,
    externalTestModules: modules.external,
    nestedTestModules: modules.nested,
    includes,
    rustSourceIncludes,
    nonRustArtifacts,
    unexpectedArtifacts,
    violations
  };
}

function rustFiles(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const entryPath = path.join(directory, entry.name);
    if (entry.isDirectory()) return rustFiles(entryPath);
    return entry.isFile() && entry.name.endsWith(".rs") ? [entryPath] : [];
  }).sort();
}

export function scanRepository(root = repositoryRoot) {
  const repository = path.resolve(root);
  const files = FIRST_PARTY_ROOTS.flatMap((relativeRoot) => rustFiles(path.join(repository, relativeRoot))).sort();
  const analyses = files.map((filePath) => analyzeRustSource(fs.readFileSync(filePath, "utf8"), {
    filePath,
    repositoryRoot: repository
  }));
  const result = {
    rustFileCount: files.length,
    files,
    analyses,
    inlineTestModules: analyses.flatMap(({ inlineTestModules }) => inlineTestModules),
    externalTestModules: analyses.flatMap(({ externalTestModules }) => externalTestModules),
    nestedTestModules: analyses.flatMap(({ nestedTestModules }) => nestedTestModules),
    includes: analyses.flatMap(({ includes }) => includes),
    rustSourceIncludes: analyses.flatMap(({ rustSourceIncludes }) => rustSourceIncludes),
    nonRustArtifacts: analyses.flatMap(({ nonRustArtifacts }) => nonRustArtifacts),
    unexpectedArtifacts: analyses.flatMap(({ unexpectedArtifacts }) => unexpectedArtifacts),
    violations: analyses.flatMap(({ violations }) => violations)
  };
  return result;
}

export function formatViolation(violation) {
  if (typeof violation === "string") return violation;
  if (violation.kind === "parse") return violation.message;
  if (violation.kind === "nested-module") return `${violation.file}:${violation.line}:nested inline cfg(test) module ${violation.name}`;
  if (violation.kind === "inline-module") return `${violation.file}:${violation.line}:inline cfg(test) module ${violation.name} has ${violation.physicalLines} physical lines (limit ${INLINE_TEST_MODULE_LIMIT})`;
  if (violation.kind === "rust-source-include") return `${violation.file}:${violation.line}:include_str! targets Rust source ${violation.target}`;
  if (violation.kind === "unexpected-include") return `${violation.file}:${violation.line}:include_str! targets unapproved artifact ${violation.target}`;
  if (violation.kind === "unresolved-include") return `${violation.file}:${violation.line}:include_str! target could not be resolved`;
  if (violation.kind === "source-contract") return `${violation.rule}: ${violation.message}`;
  return "Rust test structure violation";
}

function groupedTargets(includes) {
  const counts = new Map();
  for (const include of includes) counts.set(include.target, (counts.get(include.target) ?? 0) + 1);
  return [...counts.entries()].sort(([left], [right]) => left.localeCompare(right));
}

export function inventoryReport(result) {
  const threshold = result.inlineTestModules.filter(({ overThreshold }) => overThreshold);
  const lines = [
    "Rust test structure inventory (transition mode)",
    `Rust files: ${result.rustFileCount}`,
    `include_str! invocations: ${result.includes.length}`,
    `Rust-source include invocations: ${result.rustSourceIncludes.length}`,
    `Non-Rust artifact invocations: ${result.nonRustArtifacts.length}`,
    `Inline cfg(test) modules: ${result.inlineTestModules.length}`,
    `Inline cfg(test) modules at/over ${INLINE_TEST_MODULE_LIMIT} lines: ${threshold.length}`,
    `External/path cfg(test) modules: ${result.externalTestModules.length}`,
    `Nested cfg(test) modules rejected from top-level inventory: ${result.nestedTestModules.length}`,
    "Include targets:"
  ];
  for (const [target, count] of groupedTargets(result.includes)) lines.push(`- ${target}: ${count}`);
  lines.push("Allowed non-Rust artifacts:");
  for (const [target, count] of groupedTargets(result.nonRustArtifacts)) lines.push(`- ${target}: ${count}`);
  lines.push(`Threshold list (${threshold.length}):`);
  for (const module of threshold.sort((left, right) => `${left.file}:${left.line}`.localeCompare(`${right.file}:${right.line}`))) {
    lines.push(`- ${module.file}:${module.line}:${module.name}: ${module.physicalLines} lines`);
  }
  return `${lines.join("\n")}\n`;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const inventory = process.argv.includes("--inventory");
  const result = scanRepository();
  result.violations.push(...runSourceContractRules());
  if (inventory) {
    process.stdout.write(inventoryReport(result));
  } else {
    const strictViolations = result.violations;
    if (strictViolations.length > 0) {
      console.error("Rust test structure violations:");
      for (const violation of strictViolations) console.error(`- ${formatViolation(violation)}`);
      process.exitCode = 1;
    }
  }
}
