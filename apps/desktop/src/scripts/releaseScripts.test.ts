import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { relative, sep } from "node:path";
import { describe, expect, test } from "vitest";

const repoRoot = new URL("../../../../", import.meta.url).pathname;

function runScript(script: string, args: string[] = []): string {
  return execFileSync(process.execPath, [script, ...args], {
    cwd: repoRoot,
    encoding: "utf8"
  });
}

function gitTrackedFiles(): string[] {
  return execFileSync("git", ["ls-files"], {
    cwd: repoRoot,
    encoding: "utf8"
  })
    .split("\n")
    .map((file) => file.trim())
    .filter(Boolean);
}

type DiagnosticSource = {
  relativePath: string;
  source: string;
};

type DiagnosticGateFinding = {
  relativePath: string;
  line: number;
  location: string;
  reason: string;
};

type SourceScope = {
  name: string;
  start: number;
  end: number;
};

const DIAGNOSTIC_ENV_PATTERN = /KOUSHI_[A-Z0-9_]*(?:TRACE|DIAGNOST)/;
const TEST_ATTRIBUTE_PATTERN = /^\s*#\[(?:(?:tokio|async_std)::)?test\]\s*$/;
const SYNTHETIC_TRACE_ENV = ["KOUSHI", "SYNTH_TRACE"].join("_");
const SYNTHETIC_TRACE_DECLARATION = `const SYNTHETIC_TRACE_ENV: &str = "${SYNTHETIC_TRACE_ENV}";`;
const GATED_DIAGNOSTIC_REASON =
  "env-gated diagnostic producer has no always-on structured collection";

function runtimeRustSources(): DiagnosticSource[] {
  const roots = ["crates/koushi-sdk/src", "crates/koushi-core/src", "apps/desktop/src-tauri/src"];
  const sources: DiagnosticSource[] = [];

  function visit(directory: string): void {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const file = `${directory}/${entry.name}`;
      const fileParts = relative(repoRoot, file).split(sep);
      if (fileParts.some((part) => ["bin", "build", "generated", "target"].includes(part))) {
        continue;
      }
      if (entry.isDirectory()) {
        visit(file);
      } else if (entry.isFile() && file.endsWith(".rs")) {
        sources.push({
          relativePath: fileParts.join("/"),
          source: readFileSync(file, "utf8")
        });
      }
    }
  }

  for (const root of roots) {
    visit(`${repoRoot}${root}`);
  }
  return sources;
}

function productionRustLines(source: string): string[] {
  const lines = source.split("\n");
  const productionLines = [...lines];

  for (let index = 0; index < lines.length; index += 1) {
    if (!isTestOnlyAttribute(lines[index])) {
      continue;
    }

    let itemStart = index + 1;
    while (itemStart < lines.length && lines[itemStart].trim() === "") {
      itemStart += 1;
    }
    const itemEnd = itemStart < lines.length ? testOnlyItemEnd(lines, itemStart) : index;
    for (let itemIndex = index; itemIndex <= itemEnd; itemIndex += 1) {
      productionLines[itemIndex] = "";
    }
    index = itemEnd;
  }

  return productionLines;
}

function isTestOnlyAttribute(line: string): boolean {
  if (TEST_ATTRIBUTE_PATTERN.test(line)) {
    return true;
  }
  const match = /^\s*#\[cfg\((.*)\)\]\s*$/.exec(line);
  return match ? isTestOnlyCfgExpression(match[1]) : false;
}

function isTestOnlyCfgExpression(expression: string): boolean {
  const trimmed = expression.trim();
  if (trimmed === "test") {
    return true;
  }

  const open = trimmed.indexOf("(");
  if (open <= 0 || !trimmed.endsWith(")")) {
    return false;
  }

  const name = trimmed.slice(0, open).trim();
  const argumentsText = trimmed.slice(open + 1, -1);
  const argumentsList = splitCfgArguments(argumentsText);
  if (argumentsList === null || argumentsList.length === 0) {
    return false;
  }

  if (name === "all") {
    return argumentsList.some((argument) => isTestOnlyCfgExpression(argument));
  }
  if (name === "any") {
    return argumentsList.every((argument) => isTestOnlyCfgExpression(argument));
  }
  return false;
}

function splitCfgArguments(expression: string): string[] | null {
  const argumentsList: string[] = [];
  let start = 0;
  let depth = 0;
  let inString = false;
  let escaped = false;

  for (let index = 0; index < expression.length; index += 1) {
    const character = expression[index];
    if (inString) {
      if (escaped) {
        escaped = false;
      } else if (character === "\\") {
        escaped = true;
      } else if (character === '"') {
        inString = false;
      }
      continue;
    }
    if (character === '"') {
      inString = true;
    } else if (character === "(") {
      depth += 1;
    } else if (character === ")") {
      depth -= 1;
      if (depth < 0) {
        return null;
      }
    } else if (character === "," && depth === 0) {
      argumentsList.push(expression.slice(start, index).trim());
      start = index + 1;
    }
  }

  if (inString || depth !== 0) {
    return null;
  }
  const last = expression.slice(start).trim();
  if (last.length > 0) {
    argumentsList.push(last);
  }
  return argumentsList;
}

function testOnlyItemEnd(lines: readonly string[], start: number): number {
  let depth = 0;
  let opened = false;
  for (let index = start; index < lines.length; index += 1) {
    const structural = structuralRustLine(lines[index]);
    const delta = braceDelta(lines[index]);
    if (delta > 0) {
      opened = true;
    }
    depth += delta;
    if (opened && depth <= 0) {
      return index;
    }
    if (!opened && (structural.includes(";") || structural.trimEnd().endsWith(","))) {
      return index;
    }
  }
  return lines.length - 1;
}

type RustLexicalView = {
  code: string;
  stringValues: string[];
};

function lexicalRustView(source: string): RustLexicalView {
  const code = [...source];
  const stringValues: string[] = [];

  function blank(start: number, endExclusive: number): void {
    for (let index = start; index < endExclusive; index += 1) {
      if (code[index] !== "\n" && code[index] !== "\r") {
        code[index] = " ";
      }
    }
  }

  function rawStringAt(start: number): { contentStart: number; hashes: number } | null {
    const previous = source[start - 1];
    if (previous && /[A-Za-z0-9_]/.test(previous)) {
      return null;
    }
    let cursor = start;
    if (source[cursor] === "b") {
      cursor += 1;
    }
    if (source[cursor] !== "r") {
      return null;
    }
    cursor += 1;
    let hashes = 0;
    while (source[cursor] === "#") {
      hashes += 1;
      cursor += 1;
    }
    return source[cursor] === '"' ? { contentStart: cursor + 1, hashes } : null;
  }

  let index = 0;
  while (index < source.length) {
    if (source[index] === "/" && source[index + 1] === "/") {
      const start = index;
      while (index < source.length && source[index] !== "\n") {
        index += 1;
      }
      blank(start, index);
      continue;
    }
    if (source[index] === "/" && source[index + 1] === "*") {
      const start = index;
      let depth = 1;
      index += 2;
      while (index < source.length && depth > 0) {
        if (source[index] === "/" && source[index + 1] === "*") {
          depth += 1;
          index += 2;
        } else if (source[index] === "*" && source[index + 1] === "/") {
          depth -= 1;
          index += 2;
        } else {
          index += 1;
        }
      }
      blank(start, index);
      continue;
    }

    const rawString = rawStringAt(index);
    if (rawString) {
      const start = index;
      const terminator = `"${"#".repeat(rawString.hashes)}`;
      const end = source.indexOf(terminator, rawString.contentStart);
      const contentEnd = end === -1 ? source.length : end;
      stringValues.push(source.slice(rawString.contentStart, contentEnd));
      index = end === -1 ? source.length : end + terminator.length;
      blank(start, index);
      continue;
    }

    if (source[index] === '"') {
      const start = index;
      let value = "";
      index += 1;
      let escaped = false;
      while (index < source.length) {
        const character = source[index];
        if (!escaped && character === '"') {
          index += 1;
          break;
        }
        value += character;
        if (escaped) {
          escaped = false;
        } else if (character === "\\") {
          escaped = true;
        }
        index += 1;
      }
      stringValues.push(value);
      blank(start, index);
      continue;
    }

    index += 1;
  }

  return { code: code.join(""), stringValues };
}

function structuralRustLine(line: string): string {
  return lexicalRustView(line).code;
}

function braceDelta(line: string): number {
  return [...structuralRustLine(line)].reduce(
    (delta, character) => delta + (character === "{" ? 1 : character === "}" ? -1 : 0),
    0
  );
}

function braceDepths(lines: readonly string[]): number[] {
  let depth = 0;
  return lines.map((line) => {
    const lineDepth = depth;
    depth += braceDelta(line);
    return lineDepth;
  });
}

function previousStatementRange(
  lines: readonly string[],
  endExclusive: number,
  parentDepth: number,
  depths: readonly number[],
  minimumStart: number
): [number, number] | null {
  let end = endExclusive - 1;
  while (end >= 0 && structuralRustLine(lines[end]).trim() === "") {
    end -= 1;
  }
  if (end < minimumStart || depths[end] < parentDepth) {
    return null;
  }

  const endStructural = structuralRustLine(lines[end]);
  if (endStructural.includes("}") && !endStructural.includes(";")) {
    const blockStart = matchingBlockStart(lines, end, minimumStart);
    if (blockStart === null) {
      return null;
    }
    for (let index = blockStart; index >= minimumStart; index -= 1) {
      if (
        depths[index] === parentDepth &&
        /^(?:if|for|match|while|loop)\b/.test(structuralRustLine(lines[index]).trim())
      ) {
        return [index, end];
      }
    }
    return [blockStart, end];
  }

  for (let index = end - 1; index >= minimumStart; index -= 1) {
    if (
      depths[index] + braceDelta(lines[index]) === parentDepth &&
      structuralRustLine(lines[index]).trimEnd().endsWith("}")
    ) {
      return [index + 1, end];
    }
    if (depths[index] === parentDepth && structuralRustLine(lines[index]).includes(";")) {
      return [index + 1, end];
    }
  }
  return [minimumStart, end];
}

function matchingBlockStart(
  lines: readonly string[],
  end: number,
  minimumStart: number
): number | null {
  let nestedClosures = 0;
  for (let index = end; index >= minimumStart; index -= 1) {
    const structural = structuralRustLine(lines[index]);
    for (let characterIndex = structural.length - 1; characterIndex >= 0; characterIndex -= 1) {
      const character = structural[characterIndex];
      if (character === "}") {
        nestedClosures += 1;
      } else if (character === "{") {
        nestedClosures -= 1;
        if (nestedClosures === 0) {
          return index;
        }
      }
    }
  }
  return null;
}

function blockEnd(lines: readonly string[], start: number): number {
  let depth = 0;
  let opened = false;
  for (let index = start; index < lines.length; index += 1) {
    const structural = structuralRustLine(lines[index]);
    const delta = braceDelta(structural);
    if (delta > 0) {
      opened = true;
    }
    depth += delta;
    if (opened && depth <= 0) {
      return index;
    }
    if (!opened && balancedBlockEndsLine(structural)) {
      return index;
    }
  }
  return lines.length - 1;
}

function balancedBlockEndsLine(structural: string): boolean {
  let depth = 0;
  let opened = false;
  for (let index = 0; index < structural.length; index += 1) {
    if (structural[index] === "{") {
      depth += 1;
      opened = true;
    } else if (structural[index] === "}") {
      depth -= 1;
      if (opened && depth === 0 && structural.slice(index + 1).trim() === "") {
        return true;
      }
    }
  }
  return false;
}

function sourceScopes(lines: readonly string[]): SourceScope[] {
  const scopes: SourceScope[] = [];
  const declarations = [
    /^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z0-9_]+)/,
    /^\s*macro_rules!\s+([A-Za-z0-9_]+)/
  ];
  for (let index = 0; index < lines.length; index += 1) {
    const declaration = declarations
      .map((pattern) => pattern.exec(lines[index]))
      .find((match) => match !== null);
    if (declaration) {
      scopes.push({
        name: declaration[1],
        start: index,
        end: blockEnd(lines, index)
      });
    }
  }
  return scopes;
}

type DiagnosticSourceAnalysis = {
  relativePath: string;
  rawLines: string[];
  codeLines: string[];
  depths: number[];
  scopes: SourceScope[];
  constants: Set<string>;
  moduleQualifiers: string[];
};

type HelperResolution = {
  localByPath: Map<string, Set<string>>;
  qualified: Set<string>;
};

function envConstants(rawLines: readonly string[], codeLines: readonly string[]): Set<string> {
  const constants = new Set<string>();
  for (let index = 0; index < codeLines.length; index += 1) {
    const match = /\bconst\s+([A-Z][A-Z0-9_]*)\s*:\s*&str\s*=/.exec(codeLines[index]);
    if (
      match &&
      lexicalRustView(rawLines[index]).stringValues.some((value) =>
        DIAGNOSTIC_ENV_PATTERN.test(value)
      )
    ) {
      constants.add(match[1]);
    }
  }
  return constants;
}

function directDiagnosticEnvCheck(
  rawText: string,
  codeText: string,
  constants: Set<string>
): boolean {
  if (!/std::env::(?:var_os|var)\s*\(/.test(codeText)) {
    return false;
  }
  if (lexicalRustView(rawText).stringValues.some((value) => DIAGNOSTIC_ENV_PATTERN.test(value))) {
    return true;
  }
  return [...constants].some((constant) => new RegExp(`\\b${constant}\\b`).test(codeText));
}

function moduleQualifiers(relativePath: string): string[] {
  const parts = relativePath.split("/").filter((part) => part.length > 0);
  if (parts.length === 0) {
    return [];
  }
  parts[parts.length - 1] = parts.at(-1)!.replace(/\.rs$/, "");
  if (parts.at(-1) === "mod") {
    parts.pop();
  }
  const sourceRoot = Math.max(parts.lastIndexOf("src"), parts.lastIndexOf("fixtures"));
  const moduleParts = parts.slice(sourceRoot + 1);
  return moduleParts.map((_, index) => moduleParts.slice(index).join("::"));
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function normalizedHelperCode(codeText: string): string {
  return codeText.replace(/\b(?:crate|self|super|Self)::/g, (prefix) => " ".repeat(prefix.length));
}

function normalizedHelperName(name: string): string {
  return name.replace(/^(?:(?:crate|self|super|Self)::)+/, "");
}

function callArguments(rawText: string, codeText: string, name: string): string[] {
  const argumentsList: string[] = [];
  const normalizedCode = normalizedHelperCode(codeText);
  const pattern = new RegExp(
    `(?:^|[^A-Za-z0-9_:.])${escapeRegExp(normalizedHelperName(name))}\\s*\\(`,
    "g"
  );
  for (const match of normalizedCode.matchAll(pattern)) {
    const open = normalizedCode.indexOf("(", match.index);
    let depth = 0;
    for (let index = open; index < normalizedCode.length; index += 1) {
      if (normalizedCode[index] === "(") {
        depth += 1;
      } else if (normalizedCode[index] === ")") {
        depth -= 1;
        if (depth === 0) {
          argumentsList.push(rawText.slice(open + 1, index));
          break;
        }
      }
    }
  }
  return argumentsList;
}

function hasNamedCall(codeText: string, name: string): boolean {
  return new RegExp(
    `(?:^|[^A-Za-z0-9_:.])${escapeRegExp(normalizedHelperName(name))}\\s*\\(`
  ).test(normalizedHelperCode(codeText));
}

function hasHelperCall(codeText: string, helpers: Set<string>, currentScopeName = ""): boolean {
  return [...helpers]
    .filter((name) => name !== currentScopeName)
    .some((name) => hasNamedCall(codeText, name));
}

function resolveHelpers(
  analyses: readonly DiagnosticSourceAnalysis[],
  directMatch: (analysis: DiagnosticSourceAnalysis, scope: SourceScope) => boolean,
  transitive: boolean,
  wrapperMatch: (analysis: DiagnosticSourceAnalysis, scope: SourceScope) => boolean = () => true
): HelperResolution {
  const localByPath = new Map<string, Set<string>>();
  for (const analysis of analyses) {
    localByPath.set(
      analysis.relativePath,
      new Set(
        analysis.scopes.filter((scope) => directMatch(analysis, scope)).map((scope) => scope.name)
      )
    );
  }

  let changed = transitive;
  while (changed) {
    changed = false;
    const qualified = new Set<string>();
    for (const analysis of analyses) {
      for (const name of localByPath.get(analysis.relativePath) ?? []) {
        for (const qualifier of analysis.moduleQualifiers) {
          qualified.add(`${qualifier}::${name}`);
        }
      }
    }
    for (const analysis of analyses) {
      const local = localByPath.get(analysis.relativePath)!;
      const visible = new Set([...local, ...qualified]);
      for (const scope of analysis.scopes) {
        if (local.has(scope.name) || !wrapperMatch(analysis, scope)) {
          continue;
        }
        const codeText = analysis.codeLines.slice(scope.start, scope.end + 1).join("\n");
        if (hasHelperCall(codeText, visible, scope.name)) {
          local.add(scope.name);
          changed = true;
        }
      }
    }
  }

  const qualified = new Set<string>();
  for (const analysis of analyses) {
    for (const name of localByPath.get(analysis.relativePath) ?? []) {
      for (const qualifier of analysis.moduleQualifiers) {
        qualified.add(`${qualifier}::${name}`);
      }
    }
  }
  return { localByPath, qualified };
}

function scopeReturnsBool(analysis: DiagnosticSourceAnalysis, scope: SourceScope): boolean {
  const text = analysis.codeLines.slice(scope.start, scope.end + 1).join("\n");
  const openingBrace = text.indexOf("{");
  return /->\s*bool\b/.test(openingBrace === -1 ? text : text.slice(0, openingBrace));
}

function visibleHelpers(resolution: HelperResolution, relativePath: string): Set<string> {
  const local = resolution.localByPath.get(relativePath) ?? [];
  return new Set([...local, ...[...local].map((name) => `Self::${name}`), ...resolution.qualified]);
}

function statementEnd(
  codeLines: readonly string[],
  start: number,
  maximumEnd: number,
  depths: readonly number[]
): number {
  const parentDepth = depths[start];
  for (let index = start; index <= maximumEnd; index += 1) {
    if (depths[index] === parentDepth && codeLines[index].includes(";")) {
      return index;
    }
  }
  return start;
}

function bindingInitializer(rawText: string, codeText: string): { raw: string; code: string } {
  const equals = codeText.indexOf("=");
  const semicolon = codeText.lastIndexOf(";");
  const end = semicolon > equals ? semicolon : codeText.length;
  return equals === -1
    ? { raw: "", code: "" }
    : {
        raw: rawText.slice(equals + 1, end),
        code: codeText.slice(equals + 1, end)
      };
}

function localEnvironmentAliases(
  analysis: DiagnosticSourceAnalysis,
  scope: SourceScope,
  envHelpers: Set<string>
): Set<string> {
  const aliases = new Set<string>();
  const depths = analysis.depths;
  let changed = true;
  while (changed) {
    changed = false;
    for (let index = scope.start; index <= scope.end; index += 1) {
      const declaration = /\blet\s+(?:mut\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*(?::[^=;]+)?=/.exec(
        analysis.codeLines[index]
      );
      if (!declaration || aliases.has(declaration[1])) {
        continue;
      }
      const end = statementEnd(analysis.codeLines, index, scope.end, depths);
      const rawText = analysis.rawLines.slice(index, end + 1).join("\n");
      const codeText = analysis.codeLines.slice(index, end + 1).join("\n");
      const initializer = bindingInitializer(rawText, codeText);
      if (
        directDiagnosticEnvCheck(initializer.raw, initializer.code, analysis.constants) ||
        hasHelperCall(initializer.code, envHelpers) ||
        [...aliases].some((alias) => new RegExp(`\\b${alias}\\b`).test(initializer.code))
      ) {
        aliases.add(declaration[1]);
        changed = true;
      }
      index = end;
    }
  }
  return aliases;
}

function diagnosticGateLine(
  rawText: string,
  codeText: string,
  constants: Set<string>,
  envHelpers: Set<string>,
  localAliases: Set<string>
): boolean {
  if (!/\bif\b/.test(codeText)) {
    return false;
  }
  if (directDiagnosticEnvCheck(rawText, codeText, constants)) {
    return true;
  }
  if (hasHelperCall(codeText, envHelpers)) {
    return true;
  }
  const aliasConditionText = /\bif\s+let\b/.test(codeText)
    ? bindingInitializer(codeText, codeText).code
    : codeText;
  return [...localAliases].some((name) =>
    new RegExp(`\\b${name}\\b`).test(aliasConditionText)
  );
}

function gateHeaderEnd(codeLines: readonly string[], start: number, maximumEnd: number): number {
  const startIf = codeLines[start].search(/\bif\b/);
  if (startIf !== -1 && codeLines[start].slice(startIf).includes("{")) {
    return start;
  }
  for (let index = start; index <= maximumEnd; index += 1) {
    if (braceDelta(codeLines[index]) > 0 || codeLines[index].trimEnd().endsWith("{")) {
      return index;
    }
  }
  return start;
}

function hasStructuredCollection(
  rawLines: readonly string[],
  codeLines: readonly string[],
  depths: readonly number[],
  start: number,
  gateLine: number,
  gateEnd: number,
  structuredHelpers: Set<string>,
  stderrHelpers: Set<string>,
  currentScopeName: string
): boolean {
  if (gateLine <= start) {
    return false;
  }
  const mirrorRawText = rawLines.slice(gateLine, gateEnd + 1).join("\n");
  const mirrorCodeText = codeLines.slice(gateLine, gateEnd + 1).join("\n");
  const mirrorHeaderEnd = gateHeaderEnd(codeLines, gateLine, gateEnd);
  const mirrorHeaderCodeText = codeLines.slice(gateLine, mirrorHeaderEnd + 1).join("\n");
  const aliases = iteratorAliases(codeLines, depths, start, gateLine, depths[gateLine]);
  const mirrorTokens = expandSemanticTokensThroughBindings(
    diagnosticSideEffectTokens(mirrorRawText, mirrorCodeText, stderrHelpers),
    rawLines,
    codeLines,
    depths,
    start,
    gateLine,
    depths[gateLine]
  );
  let endExclusive = gateLine;
  while (endExclusive > start) {
    const range = previousStatementRange(codeLines, endExclusive, depths[gateLine], depths, start);
    if (range === null || range[0] < start) {
      return false;
    }
    const rawText = rawLines.slice(range[0], range[1] + 1).join("\n");
    const codeText = codeLines.slice(range[0], range[1] + 1).join("\n");
    const structuredProducer = hasStructuredProducer(
      codeLines,
      range[0],
      range[1],
      structuredHelpers,
      currentScopeName
    );
    if (
      isAssociationBarrier(codeText) &&
      (!structuredProducer ||
        !barrierControlAllowsMirror(codeText, mirrorHeaderCodeText, mirrorCodeText, aliases))
    ) {
      return false;
    }
    if (structuredProducer) {
      const collectionTokens = expandSemanticTokensThroughBindings(
        producerTokens(rawText, codeText, structuredHelpers, currentScopeName),
        rawLines,
        codeLines,
        depths,
        start,
        range[0],
        depths[gateLine]
      );
      if (hasSemanticAssociation(collectionTokens, mirrorTokens)) {
        return true;
      }
    }
    endExclusive = range[0];
  }
  return false;
}

function barrierControlAllowsMirror(
  barrierText: string,
  gateHeaderText: string,
  gateBlockText: string,
  aliases: ReadonlyMap<string, string>
): boolean {
  const firstLine = barrierText
    .split("\n")
    .map((line) => line.trim())
    .find((line) => line.length > 0);
  if (firstLine?.startsWith("if ")) {
    return gateConditionImpliesCollector(barrierText, gateHeaderText);
  }
  if (firstLine?.startsWith("for ")) {
    const collectorIterators = loopIteratorExpressions(barrierText);
    const mirrorIterators = loopIteratorExpressions(gateBlockText);
    return (
      collectorIterators.length > 0 &&
      collectorIterators.some((collectorIterator) =>
        mirrorIterators.some((mirrorIterator) =>
          equivalentIteratorDataFlow(collectorIterator, mirrorIterator, aliases)
        )
      )
    );
  }
  return false;
}

type NormalizedCondition = {
  hasOr: boolean;
  terms: Map<string, boolean>;
  valid: boolean;
};

function gateConditionImpliesCollector(
  collectorControlText: string,
  gateHeaderText: string
): boolean {
  const collector = normalizedCondition(collectorControlText);
  const gate = normalizedCondition(gateHeaderText);
  if (!collector.valid || !gate.valid || collector.hasOr || gate.hasOr) {
    return false;
  }
  return [...collector.terms].every(
    ([term, polarity]) => gate.terms.get(term) === polarity
  );
}

function normalizedCondition(controlText: string): NormalizedCondition {
  const lines = controlText.split("\n");
  const headerEnd = gateHeaderEnd(lines, 0, lines.length - 1);
  const header = lines.slice(0, headerEnd + 1).join("\n");
  const ifIndex = header.search(/\bif\b/);
  const openingBrace = header.lastIndexOf("{");
  if (ifIndex === -1 || openingBrace === -1 || openingBrace <= ifIndex) {
    return { hasOr: false, terms: new Map(), valid: false };
  }
  const expression = header.slice(ifIndex + 2, openingBrace).trim();
  const split = splitTopLevelBooleanExpression(expression);
  const terms = new Map<string, boolean>();
  let valid = split.parts.length > 0;
  for (const part of split.parts) {
    let atom = part.trim();
    let polarity = true;
    while (atom.startsWith("!") && !atom.startsWith("!=")) {
      polarity = !polarity;
      atom = atom.slice(1).trim();
    }
    atom = stripBalancedOuterParentheses(atom).replace(/\s+/g, "");
    if (atom.length === 0 || (terms.has(atom) && terms.get(atom) !== polarity)) {
      valid = false;
      continue;
    }
    terms.set(atom, polarity);
  }
  return { hasOr: split.hasOr, terms, valid };
}

function splitTopLevelBooleanExpression(expression: string): {
  hasOr: boolean;
  parts: string[];
} {
  const parts: string[] = [];
  let start = 0;
  let parentheses = 0;
  let brackets = 0;
  let braces = 0;
  let hasOr = false;
  for (let index = 0; index < expression.length - 1; index += 1) {
    const character = expression[index];
    if (character === "(") parentheses += 1;
    else if (character === ")") parentheses -= 1;
    else if (character === "[") brackets += 1;
    else if (character === "]") brackets -= 1;
    else if (character === "{") braces += 1;
    else if (character === "}") braces -= 1;
    if (parentheses !== 0 || brackets !== 0 || braces !== 0) {
      continue;
    }
    const operator = expression.slice(index, index + 2);
    if (operator === "&&" || operator === "||") {
      parts.push(expression.slice(start, index));
      hasOr ||= operator === "||";
      start = index + 2;
      index += 1;
    }
  }
  parts.push(expression.slice(start));
  return { hasOr, parts: parts.filter((part) => part.trim().length > 0) };
}

function stripBalancedOuterParentheses(value: string): string {
  let result = value.trim();
  while (result.startsWith("(") && result.endsWith(")")) {
    let depth = 0;
    let wrapsWholeValue = true;
    for (let index = 0; index < result.length; index += 1) {
      if (result[index] === "(") depth += 1;
      else if (result[index] === ")") depth -= 1;
      if (depth === 0 && index < result.length - 1) {
        wrapsWholeValue = false;
        break;
      }
    }
    if (!wrapsWholeValue) break;
    result = result.slice(1, -1).trim();
  }
  return result;
}

function loopIteratorExpressions(text: string): string[] {
  const iterators: string[] = [];
  for (const match of text.matchAll(/\bfor\s+[A-Za-z_][A-Za-z0-9_]*\s+in\s+/g)) {
    const expressionStart = (match.index ?? 0) + match[0].length;
    let parentheses = 0;
    let brackets = 0;
    for (let index = expressionStart; index < text.length; index += 1) {
      const character = text[index];
      if (character === "(") parentheses += 1;
      else if (character === ")") parentheses -= 1;
      else if (character === "[") brackets += 1;
      else if (character === "]") brackets -= 1;
      else if (character === "{" && parentheses === 0 && brackets === 0) {
        iterators.push(text.slice(expressionStart, index).replace(/\s+/g, ""));
        break;
      }
    }
  }
  return iterators;
}

function iteratorAliases(
  codeLines: readonly string[],
  depths: readonly number[],
  start: number,
  end: number,
  scopeDepth: number
): Map<string, string> {
  const aliases = new Map<string, string>();
  for (let index = start; index < end; index += 1) {
    if (depths[index] !== scopeDepth) {
      continue;
    }
    const match = /\blet\s+(?:mut\s+)?([A-Za-z_][A-Za-z0-9_]*)(?:\s*:\s*[^=;]+)?\s*=\s*&?\s*([A-Za-z_][A-Za-z0-9_]*)\s*;/.exec(
      codeLines[index]
    );
    if (match) {
      aliases.set(match[1], resolvedIteratorAlias(match[2], aliases));
    }
  }
  return aliases;
}

function resolvedIteratorAlias(name: string, aliases: ReadonlyMap<string, string>): string {
  const seen = new Set<string>();
  let current = name;
  while (aliases.has(current) && !seen.has(current)) {
    seen.add(current);
    current = aliases.get(current)!;
  }
  return current;
}

function normalizedIteratorExpression(
  expression: string,
  aliases: ReadonlyMap<string, string>
): string {
  return expression
    .replace(/\b[A-Za-z_][A-Za-z0-9_]*\b/g, (name) => resolvedIteratorAlias(name, aliases))
    .replace(/\s+/g, "");
}

function iteratorDataRoots(
  expression: string,
  aliases: ReadonlyMap<string, string>
): Set<string> {
  const closureBindings = new Set<string>();
  for (const closure of expression.matchAll(/\|([^|]*)\|/g)) {
    for (const binding of closure[1].matchAll(/\b[A-Za-z_][A-Za-z0-9_]*\b/g)) {
      closureBindings.add(binding[0]);
    }
  }
  const ignored = new Set([
    "as",
    "async",
    "await",
    "const",
    "else",
    "false",
    "for",
    "if",
    "in",
    "let",
    "loop",
    "match",
    "move",
    "mut",
    "ref",
    "return",
    "static",
    "true",
    "unsafe",
    "while"
  ]);
  const roots = new Set<string>();
  for (const match of expression.matchAll(/\b[A-Za-z_][A-Za-z0-9_]*\b/g)) {
    const name = match[0];
    const index = match.index ?? 0;
    const before = expression.slice(0, index).trimEnd();
    const after = expression.slice(index + name.length).trimStart();
    if (
      ignored.has(name) ||
      closureBindings.has(name) ||
      before.endsWith(".") ||
      after.startsWith("::") ||
      after.startsWith("(")
    ) {
      continue;
    }
    if (name === "self") {
      const field = /^\.([A-Za-z_][A-Za-z0-9_]*)/.exec(after);
      roots.add(field ? `self.${field[1]}` : name);
    } else {
      roots.add(resolvedIteratorAlias(name, aliases));
    }
  }
  return roots;
}

function equivalentIteratorDataFlow(
  collectorIterator: string,
  mirrorIterator: string,
  aliases: ReadonlyMap<string, string>
): boolean {
  if (
    normalizedIteratorExpression(collectorIterator, aliases) ===
    normalizedIteratorExpression(mirrorIterator, aliases)
  ) {
    return true;
  }
  const collectorRoots = iteratorDataRoots(collectorIterator, aliases);
  const mirrorRoots = iteratorDataRoots(mirrorIterator, aliases);
  const helperIterator = (expression: string): boolean =>
    /^(?:[A-Za-z_][A-Za-z0-9_]*::)*[A-Za-z_][A-Za-z0-9_]*\s*\(/.test(
      expression.trim()
    );
  return (
    (helperIterator(collectorIterator) || helperIterator(mirrorIterator)) &&
    collectorRoots.size > 0 &&
    collectorRoots.size === mirrorRoots.size &&
    [...collectorRoots].every((root) => mirrorRoots.has(root))
  );
}

function isAssociationBarrier(text: string): boolean {
  const firstLine = text
    .split("\n")
    .map((line) => line.trim())
    .find((line) => line.length > 0);
  return firstLine !== undefined && /^(?:if|for|match|while|loop)\b/.test(firstLine);
}

function isBindingBarrier(text: string): boolean {
  const firstLine = text
    .split("\n")
    .map((line) => line.trim())
    .find((line) => line.length > 0);
  return firstLine !== undefined && /^(?:if|match|while|loop)\b/.test(firstLine);
}

function recordArguments(rawText: string, codeText: string): string[] {
  return [
    ...callArguments(rawText, codeText, "record"),
    ...callArguments(rawText, codeText, "koushi_diagnostics::record"),
    ...callArguments(rawText, codeText, "record_batch"),
    ...callArguments(rawText, codeText, "koushi_diagnostics::record_batch")
  ];
}

function hasStructuredRecord(codeText: string): boolean {
  return recordArguments(codeText, codeText).length > 0;
}

function producerTokens(
  rawText: string,
  codeText: string,
  helpers: Set<string>,
  currentScopeName: string
): Set<string> {
  const tokens = new Set<string>();
  for (const argumentsText of recordArguments(rawText, codeText)) {
    addAll(tokens, semanticTokens(argumentsText));
  }
  for (const helper of helpers) {
    if (helper === currentScopeName || !hasNamedCall(codeText, helper)) {
      continue;
    }
    for (const argumentsText of callArguments(rawText, codeText, helper)) {
      addAll(tokens, semanticTokens(argumentsText));
    }
  }
  return expandInlineLoopBindings(tokens, rawText, codeText);
}

function expandInlineLoopBindings(
  initialTokens: Set<string>,
  rawText: string,
  codeText: string
): Set<string> {
  const tokens = new Set(initialTokens);
  for (const loop of codeText.matchAll(/\bfor\s+([A-Za-z_][A-Za-z0-9_]*)\s+in\s+/g)) {
    if (!tokens.has(loop[1])) {
      continue;
    }
    const expressionStart = (loop.index ?? 0) + loop[0].length;
    const expressionEnd = codeText.indexOf("{", expressionStart);
    if (expressionEnd !== -1) {
      addAll(tokens, semanticTokens(rawText.slice(expressionStart, expressionEnd)));
    }
  }
  return tokens;
}

function semanticTokens(text: string): Set<string> {
  const tokens = new Set<string>();
  const lexical = lexicalRustView(text);
  for (const token of lexical.code.matchAll(/\b[A-Za-z_][A-Za-z0-9_]*\b/g)) {
    tokens.add(token[0]);
  }

  for (const value of lexical.stringValues) {
    if (/^[a-z][a-z0-9_]*$/.test(value)) {
      tokens.add(value);
    }
    for (const assignment of value.matchAll(
      /(?:^|\s)[A-Za-z_][A-Za-z0-9_]*=\{?([A-Za-z_][A-Za-z0-9_]*)\}?/g
    )) {
      tokens.add(assignment[1]);
    }
    for (const placeholder of value.matchAll(/\{([A-Za-z_][A-Za-z0-9_]*)\}/g)) {
      tokens.add(placeholder[1]);
    }
  }

  return tokens;
}

function addAll(target: Set<string>, source: Set<string>): void {
  for (const value of source) {
    target.add(value);
  }
}

function expandSemanticTokensThroughBindings(
  initialTokens: Set<string>,
  rawLines: readonly string[],
  codeLines: readonly string[],
  depths: readonly number[],
  minimumStart: number,
  endExclusive: number,
  parentDepth: number
): Set<string> {
  const tokens = new Set(initialTokens);
  let cursor = endExclusive;
  while (cursor > minimumStart) {
    const range = previousStatementRange(codeLines, cursor, parentDepth, depths, minimumStart);
    if (range === null || range[0] < minimumStart) {
      break;
    }
    const codeText = codeLines.slice(range[0], range[1] + 1).join("\n");
    if (isBindingBarrier(codeText)) {
      break;
    }
    const declaration = /\blet\s+([\s\S]*?)=/.exec(codeText);
    const rawText = rawLines.slice(range[0], range[1] + 1).join("\n");
    for (const token of [...tokens]) {
      for (const method of ["push", "extend"]) {
        for (const argumentsText of callArguments(rawText, codeText, `${token}.${method}`)) {
          addAll(tokens, semanticTokens(argumentsText));
        }
      }
    }
    const boundNames = declaration
      ? [...declaration[1].matchAll(/\b[A-Za-z_][A-Za-z0-9_]*\b/g)].map((match) => match[0])
      : [];
    if (declaration && boundNames.some((name) => tokens.has(name))) {
      const equals = codeText.indexOf("=", declaration.index);
      const semicolon = codeText.lastIndexOf(";");
      if (equals !== -1 && semicolon > equals) {
        addAll(tokens, semanticTokens(rawText.slice(equals + 1, semicolon)));
      }
    }
    cursor = range[0];
  }
  return tokens;
}

function diagnosticSideEffectTokens(
  rawText: string,
  codeText: string,
  stderrHelpers: Set<string>
): Set<string> {
  const tokens = new Set<string>();
  for (const argumentsText of callArguments(rawText, codeText, "eprintln!")) {
    addAll(tokens, semanticTokens(argumentsText));
  }
  for (const helper of stderrHelpers) {
    if (!hasNamedCall(codeText, helper)) {
      continue;
    }
    for (const argumentsText of callArguments(rawText, codeText, helper)) {
      addAll(tokens, semanticTokens(argumentsText));
    }
  }
  return expandInlineLoopBindings(tokens, rawText, codeText);
}

function hasSemanticAssociation(collectionTokens: Set<string>, mirrorTokens: Set<string>): boolean {
  return [...collectionTokens].some((token) => mirrorTokens.has(token));
}

function hasStructuredProducer(
  codeLines: readonly string[],
  start: number,
  end: number,
  helpers: Set<string>,
  currentScopeName: string
): boolean {
  if (end < start) {
    return false;
  }
  const text = codeLines.slice(start, end + 1).join("\n");
  if (hasStructuredRecord(text)) {
    return true;
  }
  return hasHelperCall(text, helpers, currentScopeName);
}

function hasDiagnosticSideEffect(
  codeLines: readonly string[],
  start: number,
  end: number,
  stderr: Set<string>,
  structuredHelpers: Set<string>,
  currentScopeName: string
): boolean {
  const text = codeLines.slice(start, end + 1).join("\n");
  if (/\beprintln!\s*\(/.test(text)) {
    return true;
  }
  if (hasHelperCall(text, stderr, currentScopeName)) {
    return true;
  }
  return hasStructuredProducer(codeLines, start, end, structuredHelpers, currentScopeName);
}

function scanDiagnosticSources(sources: readonly DiagnosticSource[]): DiagnosticGateFinding[] {
  const findings: DiagnosticGateFinding[] = [];
  const analyses = sources.map(({ relativePath, source }): DiagnosticSourceAnalysis => {
    const rawLines = productionRustLines(source);
    const codeLines = lexicalRustView(rawLines.join("\n")).code.split("\n");
    return {
      relativePath,
      rawLines,
      codeLines,
      depths: braceDepths(codeLines),
      scopes: sourceScopes(codeLines),
      constants: envConstants(rawLines, codeLines),
      moduleQualifiers: moduleQualifiers(relativePath)
    };
  });
  const envResolution = resolveHelpers(
    analyses,
    (analysis, scope) =>
      scopeReturnsBool(analysis, scope) &&
      directDiagnosticEnvCheck(
        analysis.rawLines.slice(scope.start, scope.end + 1).join("\n"),
        analysis.codeLines.slice(scope.start, scope.end + 1).join("\n"),
        analysis.constants
      ),
    true,
    scopeReturnsBool
  );
  const structuredResolution = resolveHelpers(
    analyses,
    (analysis, scope) =>
      hasStructuredRecord(analysis.codeLines.slice(scope.start, scope.end + 1).join("\n")),
    true
  );
  const stderrResolution = resolveHelpers(
    analyses,
    (analysis, scope) =>
      /\beprintln!\s*\(/.test(analysis.codeLines.slice(scope.start, scope.end + 1).join("\n")),
    true
  );

  for (const analysis of analyses) {
    const envHelpers = visibleHelpers(envResolution, analysis.relativePath);
    const structuredHelpers = visibleHelpers(structuredResolution, analysis.relativePath);
    const stderr = visibleHelpers(stderrResolution, analysis.relativePath);
    for (const scope of analysis.scopes) {
      const localAliases = localEnvironmentAliases(analysis, scope, envHelpers);
      for (let lineIndex = scope.start; lineIndex <= scope.end; lineIndex += 1) {
        if (!/\bif\b/.test(analysis.codeLines[lineIndex])) {
          continue;
        }
        const headerEnd = gateHeaderEnd(analysis.codeLines, lineIndex, scope.end);
        const headerRawText = analysis.rawLines.slice(lineIndex, headerEnd + 1).join("\n");
        const headerCodeText = analysis.codeLines.slice(lineIndex, headerEnd + 1).join("\n");
        if (
          !diagnosticGateLine(
            headerRawText,
            headerCodeText,
            analysis.constants,
            envHelpers,
            localAliases
          )
        ) {
          continue;
        }
        const blockGateEnd = Math.min(blockEnd(analysis.codeLines, lineIndex), scope.end);
        const gateBlockCode = analysis.codeLines.slice(lineIndex, blockGateEnd + 1).join("\n");
        const negativeEarlyExit =
          /\bif\s*!/.test(headerCodeText) && /\breturn\b/.test(gateBlockCode);
        const diagnosticEnd = negativeEarlyExit ? scope.end : blockGateEnd;
        if (
          !hasDiagnosticSideEffect(
            analysis.codeLines,
            lineIndex,
            diagnosticEnd,
            stderr,
            structuredHelpers,
            scope.name
          )
        ) {
          continue;
        }
        const gatedStructuredProducer = hasStructuredProducer(
          analysis.codeLines,
          lineIndex,
          diagnosticEnd,
          structuredHelpers,
          scope.name
        );
        const hasCollection = hasStructuredCollection(
          analysis.rawLines,
          analysis.codeLines,
          analysis.depths,
          scope.start,
          lineIndex,
          diagnosticEnd,
          structuredHelpers,
          stderr,
          scope.name
        );
        if (gatedStructuredProducer || !hasCollection) {
          findings.push({
            relativePath: analysis.relativePath,
            line: lineIndex + 1,
            location: `${analysis.relativePath}:${lineIndex + 1}`,
            reason: GATED_DIAGNOSTIC_REASON
          });
        }
      }
    }
  }
  return findings;
}

describe("desktop release scripts", () => {
  test("always-on diagnostic collection rejects trace-only producers and accepts stderr mirrors", () => {
    const badFixture = `
fn gated_only() {
  if std::env::var_os("KOUSHI_SYNTH_TRACE").is_some() {
    record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", "gated"));
    eprintln!("synthetic stderr mirror");
  }
}
`;
    const goodFixture = `
fn collected_first() {
  let stage = "collected";
  record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", stage));
  if std::env::var_os("KOUSHI_SYNTH_TRACE").is_some() {
    eprintln!("synthetic stderr stage={stage}");
  }
}

#[cfg(test)]
mod tests {
  fn test_only_environment_probe() {
    if std::env::var_os("KOUSHI_SYNTH_TRACE").is_some() {
      eprintln!("test-only");
    }
  }
}
`;
    const helperAndAliasFixture = `
const SYNTHETIC_TRACE_ENV: &str = "KOUSHI_SYNTH_TRACE";

fn stderr_enabled() -> bool {
  std::env::var_os(SYNTHETIC_TRACE_ENV).is_some()
}

fn helper_gated_only() {
  if stderr_enabled() {
    record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", "helper"));
    eprintln!("synthetic helper stderr mirror");
  }
}

fn boolean_alias_gated_only() {
  let trace = std::env::var_os("KOUSHI_SYNTH_TRACE").is_some();
  if trace {
    record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", "alias"));
    eprintln!("synthetic alias stderr mirror");
  }
}

fn collected_helper_mirror(stage: &'static str) {
  record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", stage));
  if stderr_enabled() {
    eprintln!("synthetic helper stderr stage={stage}");
  }
}

fn collected_alias_mirror() {
  let trace = std::env::var_os("KOUSHI_SYNTH_TRACE").is_some();
  let stage = "alias_collected";
  record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", stage));
  if trace {
    eprintln!("synthetic alias stderr stage={stage}");
  }
}
`;

    const badFindings = scanDiagnosticSources([
      { relativePath: "fixtures/bad.rs", source: badFixture }
    ]);
    expect(badFindings).toHaveLength(1);
    expect(badFindings[0]).toMatchObject({
      relativePath: "fixtures/bad.rs",
      line: 3,
      location: "fixtures/bad.rs:3"
    });
    expect(badFindings[0].reason).toContain("structured collection");

    const helperAndAliasFindings = scanDiagnosticSources([
      {
        relativePath: "fixtures/helper-and-alias.rs",
        source: helperAndAliasFixture
      }
    ]);
    expect(helperAndAliasFindings).toHaveLength(2);
    expect(
      helperAndAliasFindings.every((finding) => finding.relativePath.includes("fixtures/"))
    ).toBe(true);
    expect(helperAndAliasFindings.every((finding) => finding.line > 0)).toBe(true);
    expect(helperAndAliasFindings.every((finding) => finding.location.includes(":"))).toBe(true);
    expect(
      helperAndAliasFindings.every((finding) => finding.reason === GATED_DIAGNOSTIC_REASON)
    ).toBe(true);

    expect(
      scanDiagnosticSources([{ relativePath: "fixtures/good.rs", source: goodFixture }])
    ).toEqual([]);

    const runtimeFindings = scanDiagnosticSources(runtimeRustSources());
    expect(runtimeFindings).toEqual([]);
  });

  test("scanner rejects structured producers inside every recognized gate form without stderr", () => {
    const directGateFixture = `
fn direct_gate_only() {
  if std::env::var_os("KOUSHI_SYNTH_TRACE").is_some() {
    record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", "direct"));
  }
}
`;
    const helperGateFixture = `
fn record_helper() {
  record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", "helper"));
}

fn helper_gate_only() {
  if stderr_enabled() {
    record_helper();
  }
}

fn stderr_enabled() -> bool {
  std::env::var_os("KOUSHI_SYNTH_TRACE").is_some()
}
`;
    const booleanAliasGateFixture = `
fn boolean_alias_gate_only() {
  let trace = std::env::var_os("KOUSHI_SYNTH_TRACE").is_some();
  if trace {
    record_helper();
  }
}

fn record_helper() {
  record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", "alias"));
}
`;

    for (const [relativePath, source, line] of [
      ["fixtures/direct-gate-only.rs", directGateFixture, 3],
      ["fixtures/helper-gate-only.rs", helperGateFixture, 7],
      ["fixtures/boolean-alias-gate-only.rs", booleanAliasGateFixture, 4]
    ] as const) {
      const findings = scanDiagnosticSources([{ relativePath, source }]);
      expect(findings).toHaveLength(1);
      expect(findings[0]).toMatchObject({
        relativePath,
        line,
        location: `${relativePath}:${line}`
      });
      expect(findings[0].reason).toContain("structured collection");
    }
  });

  test("scanner does not let an unrelated record hide a later gated-only diagnostic", () => {
    const fixture = `
${SYNTHETIC_TRACE_DECLARATION}

fn trace_enabled() -> bool {
  std::env::var_os(SYNTHETIC_TRACE_ENV).is_some()
}

fn unrelated_canonical_record_before_gate() {
  record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", "unrelated"));
  if trace_enabled() {
    eprintln!("synthetic stderr mirror");
  }
}

fn unrelated_canonical_record_helper() {
  record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", "helper_unrelated"));
}

fn unrelated_canonical_helper_before_gate() {
  unrelated_canonical_record_helper();
  if trace_enabled() {
    eprintln!("synthetic stderr mirror");
  }
}
`;

    const findings = scanDiagnosticSources([
      { relativePath: "fixtures/unrelated-record.rs", source: fixture }
    ]);
    expect(findings).toHaveLength(2);
    expect(findings.map((finding) => finding.line)).toEqual([10, 21]);
    expect(findings).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          relativePath: "fixtures/unrelated-record.rs",
          location: "fixtures/unrelated-record.rs:10"
        }),
        expect.objectContaining({
          relativePath: "fixtures/unrelated-record.rs",
          location: "fixtures/unrelated-record.rs:21"
        })
      ])
    );
  });

  test("scanner association uses producer arguments and stops at control-flow barriers", () => {
    const reboundStageFixture = `
${SYNTHETIC_TRACE_DECLARATION}

fn trace_enabled() -> bool {
  std::env::var_os(SYNTHETIC_TRACE_ENV).is_some()
}

fn unrelated_then_rebound_stage() {
  record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", "unrelated"));
  let stage = "actual_mirror";
  if trace_enabled() {
    eprintln!("synthetic stderr stage={stage}");
  }
}
`;
    const gateNameCollisionFixture = `
${SYNTHETIC_TRACE_DECLARATION}

fn trace_enabled() -> bool {
  std::env::var_os(SYNTHETIC_TRACE_ENV).is_some()
}

fn unrelated_trace_stage() {
  record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", "trace"));
  if trace_enabled() {
    eprintln!("synthetic stderr mirror");
  }
}
`;
    const conditionalCollectorFixture = `
${SYNTHETIC_TRACE_DECLARATION}

fn trace_enabled() -> bool {
  std::env::var_os(SYNTHETIC_TRACE_ENV).is_some()
}

fn conditionally_collected(collect: bool) {
  let stage = "conditional";
  if collect {
    record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", stage));
  }
  if trace_enabled() {
    eprintln!("synthetic stderr stage={stage}");
  }
}
`;

    const findings = scanDiagnosticSources([
      {
        relativePath: "fixtures/rebound-stage.rs",
        source: reboundStageFixture
      },
      {
        relativePath: "fixtures/gate-name-collision.rs",
        source: gateNameCollisionFixture
      },
      {
        relativePath: "fixtures/conditional-collector.rs",
        source: conditionalCollectorFixture
      }
    ]);
    expect(findings.map((finding) => finding.relativePath)).toEqual([
      "fixtures/rebound-stage.rs",
      "fixtures/gate-name-collision.rs",
      "fixtures/conditional-collector.rs"
    ]);
    expect(findings.every((finding) => finding.reason === GATED_DIAGNOSTIC_REASON)).toBe(true);
  });

  test("scanner rejects opposite-polarity conditional collection", () => {
    const fixture = `
fn trace_enabled() -> bool {
  std::env::var_os("KOUSHI_SYNTH_TRACE").is_some()
}

fn negated_condition(collect: bool) {
  let stage = "negated";
  if collect {
    record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", stage));
  }
  if !collect && trace_enabled() {
    eprintln!("synthetic stderr stage={stage}");
  }
}
`;

    const findings = scanDiagnosticSources([
      { relativePath: "fixtures/negated-condition.rs", source: fixture }
    ]);
    expect(findings).toHaveLength(1);
    expect(findings[0]).toMatchObject({
      relativePath: "fixtures/negated-condition.rs",
      reason: GATED_DIAGNOSTIC_REASON
    });
  });

  test("scanner rejects disjunctive gates and accepts implied conjunctions", () => {
    const fixture = `
fn trace_enabled() -> bool {
  std::env::var_os("KOUSHI_SYNTH_TRACE").is_some()
}

fn disjunctive_gate(collect: bool) {
  let stage = "disjunction";
  if collect {
    record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", stage));
  }
  if trace_enabled() || collect {
    eprintln!("synthetic stderr stage={stage}");
  }
}

fn paired_condition(collect: bool, ready: bool) {
  let stage = "paired_condition";
  if ready && collect {
    record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", stage));
  }
  if trace_enabled() && collect && ready {
    eprintln!("synthetic stderr stage={stage}");
  }
}
`;

    const findings = scanDiagnosticSources([
      { relativePath: "fixtures/disjunctive-and-implied.rs", source: fixture }
    ]);
    expect(findings).toHaveLength(1);
    expect(findings[0]).toMatchObject({
      relativePath: "fixtures/disjunctive-and-implied.rs",
      reason: GATED_DIAGNOSTIC_REASON
    });
  });

  test("scanner treats collector loops as barriers unless iterators are paired", () => {
    const fixture = `
fn trace_enabled() -> bool {
  std::env::var_os("KOUSHI_SYNTH_TRACE").is_some()
}

fn independent_loop(items: &[&str]) {
  let stage = "independent_loop";
  for _item in items {
    record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", stage));
  }
  if trace_enabled() {
    eprintln!("synthetic stderr stage={stage}");
  }
}

fn paired_loop(items: &[&str]) {
  let stage = "paired_loop";
  for item in items {
    record(make_diagnostic_event(stage, item));
  }
  if trace_enabled() {
    for item in items {
      eprintln!("synthetic stderr stage={stage} item={item}");
    }
  }
}
`;

    const findings = scanDiagnosticSources([
      { relativePath: "fixtures/paired-and-independent-loops.rs", source: fixture }
    ]);
    expect(findings).toHaveLength(1);
    expect(findings[0]).toMatchObject({
      relativePath: "fixtures/paired-and-independent-loops.rs",
      reason: GATED_DIAGNOSTIC_REASON
    });
  });

  test("scanner pairs collector loops only through equivalent iterator data flow", () => {
    const fixture = `
fn trace_enabled() -> bool {
  std::env::var_os("KOUSHI_SYNTH_TRACE").is_some()
}

fn different_iterators(collected_items: &[&str], mirrored_items: &[&str]) {
  let stage = "different_iterators";
  for item in collected_items.iter() {
    record(make_diagnostic_event(stage, item));
  }
  if trace_enabled() {
    for item in mirrored_items.iter() {
      eprintln!("synthetic stderr stage={stage} item={item}");
    }
  }
}

fn same_iterator(items: &[&str]) {
  let stage = "same_iterator";
  for item in items.iter() {
    record(make_diagnostic_event(stage, item));
  }
  if trace_enabled() {
    for item in items.iter() {
      eprintln!("synthetic stderr stage={stage} item={item}");
    }
  }
}

fn aliased_iterator(collected_items: &[&str]) {
  let stage = "aliased_iterator";
  let mirrored_items = collected_items;
  for item in collected_items.iter() {
    record(make_diagnostic_event(stage, item));
  }
  if trace_enabled() {
    for item in mirrored_items.iter() {
      eprintln!("synthetic stderr stage={stage} item={item}");
    }
  }
}
`;

    const findings = scanDiagnosticSources([
      { relativePath: "fixtures/equivalent-loop-iterators.rs", source: fixture }
    ]);
    expect(findings).toHaveLength(1);
    expect(findings[0]).toMatchObject({
      relativePath: "fixtures/equivalent-loop-iterators.rs",
      reason: GATED_DIAGNOSTIC_REASON
    });
  });

  test("scanner recognizes generic gated record producers without stderr", () => {
    const fixture = `
${SYNTHETIC_TRACE_DECLARATION}

fn trace_enabled() -> bool {
  std::env::var_os(SYNTHETIC_TRACE_ENV).is_some()
}

fn make_diagnostic_event() -> DiagnosticEvent {
  todo!()
}

fn generic_direct_record_only() {
  if trace_enabled() {
    record(make_diagnostic_event());
  }
}

fn generic_record_helper() {
  record(make_diagnostic_event());
}

fn generic_helper_record_only() {
  if trace_enabled() {
    generic_record_helper();
  }
}
`;

    const findings = scanDiagnosticSources([
      { relativePath: "fixtures/generic-record-only.rs", source: fixture }
    ]);
    expect(findings).toHaveLength(2);
    expect(findings.map((finding) => finding.line)).toEqual([13, 23]);
    expect(findings.every((finding) => finding.reason === GATED_DIAGNOSTIC_REASON)).toBe(true);
  });

  test("scanner accepts semantically linked generic records, wrappers, and event bindings", () => {
    const fixture = `
${SYNTHETIC_TRACE_DECLARATION}

fn trace_enabled() -> bool {
  std::env::var_os(SYNTHETIC_TRACE_ENV).is_some()
}

fn make_diagnostic_event(stage: &'static str) -> DiagnosticEvent {
  todo!()
}

fn record_generic(stage: &'static str) {
  record(make_diagnostic_event(stage));
}

fn record_wrapper(stage: &'static str) {
  record_generic(stage);
}

fn direct_generic_mirror() {
  let stage = "direct_generic";
  record(make_diagnostic_event(stage));
  if trace_enabled() {
    eprintln!("synthetic stderr stage={stage}");
  }
}

fn wrapped_generic_mirror() {
  let stage = "wrapped_generic";
  record_wrapper(stage);
  if trace_enabled() {
    eprintln!("synthetic stderr stage={stage}");
  }
}

fn arbitrary_event_binding_mirror() {
  let stage = "bound_generic";
  let diagnostic_entry = make_diagnostic_event(stage);
  record(diagnostic_entry);
  if trace_enabled() {
    eprintln!("synthetic stderr stage={stage}");
  }
}
`;

    expect(
      scanDiagnosticSources([{ relativePath: "fixtures/generic-mirrors.rs", source: fixture }])
    ).toEqual([]);
  });

  test("scanner recognizes record batches and event-vector data flow", () => {
    const goodFixture = `
fn trace_enabled() -> bool {
  std::env::var_os("KOUSHI_SYNTH_TRACE").is_some()
}

fn collected_batch_before_early_exit() {
  let stage = "batch";
  let mut diagnostic_events = vec![
    DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", stage),
  ];
  diagnostic_events.push(make_diagnostic_event(stage));
  record_batch(diagnostic_events);
  if !trace_enabled() {
    return;
  }
  eprintln!("synthetic stderr stage={stage}");
}
`;
    const badFixture = `
fn trace_enabled() -> bool {
  std::env::var_os("KOUSHI_SYNTH_TRACE").is_some()
}

fn gated_batch_only() {
  let stage = "gated_batch";
  if trace_enabled() {
    let diagnostic_events = vec![make_diagnostic_event(stage)];
    record_batch(diagnostic_events);
  }
}
`;

    expect(
      scanDiagnosticSources([
        { relativePath: "fixtures/good-record-batch.rs", source: goodFixture }
      ])
    ).toEqual([]);
    const badFindings = scanDiagnosticSources([
      { relativePath: "fixtures/bad-record-batch.rs", source: badFixture }
    ]);
    expect(badFindings).toHaveLength(1);
    expect(badFindings[0]).toMatchObject({
      relativePath: "fixtures/bad-record-batch.rs",
      reason: GATED_DIAGNOSTIC_REASON
    });
  });

  test("stderr helper discovery follows two-hop chains without masking gated-only output", () => {
    const fixture = `
fn stderr_leaf(stage: &'static str) {
  eprintln!("synthetic stderr stage={stage}");
}

fn stderr_middle(stage: &'static str) {
  stderr_leaf(stage);
}

fn trace_enabled() -> bool {
  std::env::var_os("KOUSHI_SYNTH_TRACE").is_some()
}

fn gated_two_hop_only() {
  if trace_enabled() {
    stderr_middle("gated");
  }
}

fn collected_two_hop_mirror() {
  let stage = "collected";
  record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", stage));
  if trace_enabled() {
    stderr_middle(stage);
  }
}
`;

    const findings = scanDiagnosticSources([
      { relativePath: "fixtures/two-hop-stderr.rs", source: fixture }
    ]);
    expect(findings).toHaveLength(1);
    expect(findings[0]).toMatchObject({
      relativePath: "fixtures/two-hop-stderr.rs",
      line: 15,
      location: "fixtures/two-hop-stderr.rs:15"
    });
  });

  test("scanner recognizes balanced multiline gates and negative early-exit gates", () => {
    const multilineBadFixture = `
fn multiline_gate_only() {
  let stage = "multiline";
  if std::env::var_os(
    "KOUSHI_SYNTH_TRACE"
  ).is_some() {
    eprintln!("synthetic stderr stage={stage}");
  }
}
`;
    const negativeEprintlnBadFixture = `
fn trace_enabled() -> bool {
  std::env::var_os("KOUSHI_SYNTH_TRACE").is_some()
}

fn early_exit_gate_only() {
  let stage = "early_exit";
  if !trace_enabled() {
    return;
  }
  eprintln!("synthetic stderr stage={stage}");
}
`;
    const negativeHelperBadFixture = `
fn trace_enabled() -> bool {
  std::env::var_os("KOUSHI_SYNTH_TRACE").is_some()
}

fn stderr_leaf(stage: &'static str) {
  eprintln!("synthetic stderr stage={stage}");
}

fn early_exit_helper_only() {
  let stage = "early_exit_helper";
  if !trace_enabled() {
    return;
  }
  stderr_leaf(stage);
}
`;
    const goodFixture = `
fn trace_enabled() -> bool {
  std::env::var_os("KOUSHI_SYNTH_TRACE").is_some()
}

fn stderr_leaf(stage: &'static str) {
  eprintln!("synthetic stderr stage={stage}");
}

fn collected_before_multiline_gate() {
  let stage = "multiline_collected";
  record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", stage));
  if std::env::var_os(
    "KOUSHI_SYNTH_TRACE"
  ).is_some() {
    eprintln!("synthetic stderr stage={stage}");
  }
}

fn collected_before_early_exit() {
  let stage = "early_exit_collected";
  record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", stage));
  if !trace_enabled() {
    return;
  }
  stderr_leaf(stage);
}
`;

    const badFindings = scanDiagnosticSources([
      {
        relativePath: "fixtures/multiline-gate-only.rs",
        source: multilineBadFixture
      },
      {
        relativePath: "fixtures/negative-eprintln-gate-only.rs",
        source: negativeEprintlnBadFixture
      },
      {
        relativePath: "fixtures/negative-helper-gate-only.rs",
        source: negativeHelperBadFixture
      }
    ]);
    expect(badFindings.map((finding) => finding.relativePath)).toEqual([
      "fixtures/multiline-gate-only.rs",
      "fixtures/negative-eprintln-gate-only.rs",
      "fixtures/negative-helper-gate-only.rs"
    ]);
    expect(badFindings.every((finding) => finding.reason === GATED_DIAGNOSTIC_REASON)).toBe(true);
    expect(
      scanDiagnosticSources([
        {
          relativePath: "fixtures/multiline-and-early-good.rs",
          source: goodFixture
        }
      ])
    ).toEqual([]);
  });

  test("scanner resolves module-qualified environment and diagnostic helpers across files", () => {
    const unreadTraceFixture = `
const ENV_VAR: &str = "KOUSHI_UNREAD_TRACE";

pub(crate) fn enabled() -> bool {
  std::env::var_os(ENV_VAR).is_some()
}

pub(crate) fn trace_room_list_applied(stage: &'static str) {
  record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", stage));
}
`;
    const runtimeFixture = `
fn reduce_app_action(action: Action) {
  let stage = "room_list_applied";
  let raw_room_list_trace = if unread_trace::enabled()
    && matches!(action, Action::RoomListUpdated)
  {
    Some(stage)
  } else {
    None
  };
  if let Some(raw_stage) = raw_room_list_trace {
    unread_trace::trace_room_list_applied(raw_stage);
  }
}
`;

    const findings = scanDiagnosticSources([
      { relativePath: "fixtures/unread_trace.rs", source: unreadTraceFixture },
      { relativePath: "fixtures/runtime.rs", source: runtimeFixture }
    ]);
    expect(findings).toHaveLength(1);
    expect(findings[0]).toMatchObject({
      relativePath: "fixtures/runtime.rs",
      location: "fixtures/runtime.rs:11",
      reason: GATED_DIAGNOSTIC_REASON
    });
  });

  test("scanner follows wrapped environment helpers and two-hop Self record wrappers", () => {
    const fixture = `
fn direct_env_enabled() -> bool {
  std::env::var_os("KOUSHI_SYNTH_TRACE").is_some()
}

fn trace_enabled() -> bool {
  direct_env_enabled()
}

fn record_leaf(stage: &'static str) {
  record(make_diagnostic_event(stage));
}

fn record_wrapper(stage: &'static str) {
  Self::record_leaf(stage);
}

fn wrapped_gate_only() {
  let stage = "wrapped_gate";
  if trace_enabled() {
    Self::record_wrapper(stage);
  }
}
`;

    const findings = scanDiagnosticSources([
      { relativePath: "fixtures/wrapped-helper-chain.rs", source: fixture }
    ]);
    expect(findings).toHaveLength(1);
    expect(findings[0]).toMatchObject({
      relativePath: "fixtures/wrapped-helper-chain.rs",
      reason: GATED_DIAGNOSTIC_REASON
    });
  });

  test("scanner follows arbitrary and transitive environment aliases", () => {
    const fixture = `
fn arbitrary_alias_gate_only() {
  let stage = "arbitrary_alias";
  let gate = std::env::var_os("KOUSHI_SYNTH_TRACE").is_some();
  let forwarded = gate;
  if forwarded {
    record(make_diagnostic_event(stage));
  }
}
`;

    const findings = scanDiagnosticSources([
      { relativePath: "fixtures/arbitrary-alias-chain.rs", source: fixture }
    ]);
    expect(findings).toHaveLength(1);
    expect(findings[0]).toMatchObject({
      relativePath: "fixtures/arbitrary-alias-chain.rs",
      reason: GATED_DIAGNOSTIC_REASON
    });
  });

  test("scanner normalizes crate-qualified cross-file environment helpers", () => {
    const crossFileHelperFixture = `
pub(crate) fn enabled() -> bool {
  std::env::var_os("KOUSHI_SYNTH_TRACE").is_some()
}
`;
    const crossFileRuntimeFixture = `
fn crate_qualified_gate_only() {
  let stage = "crate_qualified";
  if crate::trace_gate::enabled() {
    record(make_diagnostic_event(stage));
  }
}
`;

    const findings = scanDiagnosticSources([
      { relativePath: "fixtures/trace_gate.rs", source: crossFileHelperFixture },
      { relativePath: "fixtures/qualified-runtime.rs", source: crossFileRuntimeFixture }
    ]);
    expect(findings).toHaveLength(1);
    expect(findings[0]).toMatchObject({
      relativePath: "fixtures/qualified-runtime.rs",
      reason: GATED_DIAGNOSTIC_REASON
    });
  });

  test("scanner preserves nested module identity for cross-file helpers", () => {
    const nestedHelperFixture = `
pub(crate) fn enabled() -> bool {
  std::env::var_os("KOUSHI_SYNTH_TRACE").is_some()
}
`;
    const crateNestedRuntimeFixture = `
fn crate_nested_gate_only() {
  if crate::diagnostics::trace_gate::enabled() {
    record(make_diagnostic_event("crate_nested"));
  }
}
`;
    const selfNestedRuntimeFixture = `
fn self_nested_gate_only() {
  if self::diagnostics::trace_gate::enabled() {
    record(make_diagnostic_event("self_nested"));
  }
}
`;
    const superNestedRuntimeFixture = `
fn super_nested_gate_only() {
  if super::trace_gate::enabled() {
    record(make_diagnostic_event("super_nested"));
  }
}
`;

    const findings = scanDiagnosticSources([
      {
        relativePath: "fixtures/diagnostics/trace_gate.rs",
        source: nestedHelperFixture
      },
      { relativePath: "fixtures/crate-runtime.rs", source: crateNestedRuntimeFixture },
      { relativePath: "fixtures/lib.rs", source: selfNestedRuntimeFixture },
      {
        relativePath: "fixtures/diagnostics/super-runtime.rs",
        source: superNestedRuntimeFixture
      }
    ]);
    expect(findings).toHaveLength(3);
    expect(findings.every((finding) => finding.reason === GATED_DIAGNOSTIC_REASON)).toBe(true);

    const wrongModuleFindings = scanDiagnosticSources([
      { relativePath: "fixtures/other/trace_gate.rs", source: nestedHelperFixture },
      {
        relativePath: "fixtures/wrong-module-runtime.rs",
        source: `
fn wrong_module_gate_only() {
  if crate::diagnostics::trace_gate::enabled() {
    record(make_diagnostic_event("wrong_module"));
  }
}
`
      }
    ]);
    expect(wrongModuleFindings).toHaveLength(0);
  });

  test("scanner keeps balanced one-line scopes from duplicating findings", () => {
    const fixture = `
fn trace_enabled() -> bool { std::env::var_os("KOUSHI_SYNTH_TRACE").is_some() }
fn one_line_gate_only() { if trace_enabled() { record(make_diagnostic_event("one_line")); } }
`;

    const findings = scanDiagnosticSources([
      { relativePath: "fixtures/one-line-scopes.rs", source: fixture }
    ]);
    expect(findings).toHaveLength(1);
    expect(findings[0]).toMatchObject({
      relativePath: "fixtures/one-line-scopes.rs",
      line: 3,
      location: "fixtures/one-line-scopes.rs:3",
      reason: GATED_DIAGNOSTIC_REASON
    });
  });

  test("scanner ignores record and helper spellings inside comments and strings", () => {
    const lineCommentFixture = `
fn trace_enabled() -> bool {
  std::env::var_os("KOUSHI_SYNTH_TRACE").is_some()
}

fn comment_is_not_collection() {
  let stage = "comment";
  // record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", stage));
  if trace_enabled() {
    eprintln!("synthetic stderr stage={stage}");
  }
}
`;
    const blockCommentFixture = `
fn trace_enabled() -> bool {
  std::env::var_os("KOUSHI_SYNTH_TRACE").is_some()
}

fn block_comment_is_not_collection() {
  let stage = "block_comment";
  /* record(DiagnosticEvent::new(
    DiagnosticLevel::Debug,
    "synthetic",
    stage,
  )); */
  if trace_enabled() {
    eprintln!("synthetic stderr stage={stage}");
  }
}
`;
    const stringHelperFixture = `
fn trace_enabled() -> bool {
  std::env::var_os("KOUSHI_SYNTH_TRACE").is_some()
}

fn fake_record_helper(stage: &'static str) {
  let example = "record(DiagnosticEvent::new(DiagnosticLevel::Debug, synthetic, stage))";
}

fn string_is_not_helper_collection() {
  let stage = "string_helper";
  fake_record_helper(stage);
  if trace_enabled() {
    eprintln!("synthetic stderr stage={stage}");
  }
}
`;

    const findings = scanDiagnosticSources([
      {
        relativePath: "fixtures/line-comment-record.rs",
        source: lineCommentFixture
      },
      {
        relativePath: "fixtures/block-comment-record.rs",
        source: blockCommentFixture
      },
      {
        relativePath: "fixtures/string-helper-record.rs",
        source: stringHelperFixture
      }
    ]);
    expect(findings.map((finding) => finding.relativePath)).toEqual([
      "fixtures/line-comment-record.rs",
      "fixtures/block-comment-record.rs",
      "fixtures/string-helper-record.rs"
    ]);
    expect(findings.every((finding) => finding.reason === GATED_DIAGNOSTIC_REASON)).toBe(true);
  });

  test("scanner accepts direct, helper, loop, and transformed mirror siblings", () => {
    const fixture = `
${SYNTHETIC_TRACE_DECLARATION}

fn trace_enabled() -> bool {
  std::env::var_os(SYNTHETIC_TRACE_ENV).is_some()
}

fn record_helper(stage: &'static str) {
  record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", stage));
}

fn direct_mirror() {
  let stage = "direct";
  record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", stage));
  if trace_enabled() {
    eprintln!("synthetic stderr stage={stage}");
  }
}

fn helper_mirror() {
  let stage = "helper";
  record_helper(stage);
  if trace_enabled() {
    eprintln!("synthetic stderr stage={stage}");
  }
}

fn loop_mirror(items: &[&'static str]) {
  let stage = "loop";
  for item in items {
    record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", stage));
  }
  if trace_enabled() {
    for item in items {
      eprintln!("synthetic stderr stage={stage} item={item}");
    }
  }
}

fn transformed_mirror() {
  let stage = "transformed";
  let event = make_diagnostic_event(stage);
  record(event);
  let line = format!("stage={stage}");
  if trace_enabled() {
    eprintln!("{line}");
  }
}
`;

    expect(
      scanDiagnosticSources([{ relativePath: "fixtures/mirror-shapes.rs", source: fixture }])
    ).toEqual([]);
  });

  test("scanner masks only cfg items that are provably test-only", () => {
    const fixture = `
#[cfg(test)]
fn exact_test_only() {
  if std::env::var_os("KOUSHI_SYNTH_TRACE").is_some() {
    record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", "exact-test"));
    eprintln!("test-only");
  }
}

#[cfg(all(test, feature = "diagnostic-runtime"))]
fn all_test_only() {
  if std::env::var_os("KOUSHI_SYNTH_TRACE").is_some() {
    record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", "all-test"));
    eprintln!("test-only");
  }
}

#[cfg(any(test, feature = "diagnostic-runtime"))]
fn conditional_runtime() {
  if std::env::var_os("KOUSHI_SYNTH_TRACE").is_some() {
    record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", "conditional"));
    eprintln!("synthetic stderr mirror");
  }
}

#[cfg(all(any(test, feature = "diagnostic-runtime"), test))]
fn nested_all_test_only() {
  if std::env::var_os("KOUSHI_SYNTH_TRACE").is_some() {
    record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", "nested-test"));
    eprintln!("test-only");
  }
}
`;

    const findings = scanDiagnosticSources([
      { relativePath: "fixtures/cfg-conditions.rs", source: fixture }
    ]);
    expect(findings).toHaveLength(1);
    expect(findings[0]).toMatchObject({
      relativePath: "fixtures/cfg-conditions.rs",
      line: 20,
      location: "fixtures/cfg-conditions.rs:20"
    });
  });

  test("scanner keeps production code after a balanced test-only module", () => {
    const fixture = `
#[cfg(test)]
mod tests {
  fn test_only_environment_probe() {
    if std::env::var_os("KOUSHI_SYNTH_TRACE").is_some() {
      eprintln!("test-only");
    }
  }
}

fn production_after_tests() {
  if std::env::var_os("KOUSHI_SYNTH_TRACE").is_some() {
    record(DiagnosticEvent::new(DiagnosticLevel::Debug, "synthetic", "after-tests"));
    eprintln!("synthetic stderr mirror");
  }
}
`;

    const findings = scanDiagnosticSources([
      { relativePath: "fixtures/production-after-tests.rs", source: fixture }
    ]);
    expect(findings).toHaveLength(1);
    expect(findings[0]).toMatchObject({
      relativePath: "fixtures/production-after-tests.rs",
      line: 12,
      location: "fixtures/production-after-tests.rs:12"
    });
  });

  test("tracked text artifacts contain no previous branding residue", () => {
    const oldLatinBrand = "Ru" + "ri";
    const oldLowerBrand = oldLatinBrand.toLowerCase();
    const oldJapaneseBrand = "瑠" + "璃";
    const pattern = new RegExp(`${oldLatinBrand}|${oldLowerBrand}|${oldJapaneseBrand}`);
    const binaryExtensions = new Set([
      ".png",
      ".jpg",
      ".jpeg",
      ".gif",
      ".webp",
      ".ico",
      ".icns",
      ".woff",
      ".woff2",
      ".ttf",
      ".otf",
      ".zst"
    ]);
    // Files that intentionally mention prior branding for documentation/history.
    const intentionalPreviousBrandReferences = new Set(["README.md"]);
    const findings: string[] = [];

    for (const file of gitTrackedFiles()) {
      const extension = file.includes(".") ? file.slice(file.lastIndexOf(".")).toLowerCase() : "";
      if (binaryExtensions.has(extension)) {
        continue;
      }
      if (intentionalPreviousBrandReferences.has(file)) {
        continue;
      }
      let contents: string;
      try {
        contents = readFileSync(new URL(`../../../../${file}`, import.meta.url), "utf8");
      } catch {
        continue;
      }
      if (pattern.test(contents)) {
        findings.push(file);
      }
    }

    expect(findings).toEqual([]);
  });

  test("release preflight validates installer and signing preparation", () => {
    const output = runScript("scripts/desktop-release-preflight.mjs", ["--check-config"]);

    expect(output).toContain("bundle.active");
    expect(output).toContain("dmg");
    expect(output).toContain("msi");
    expect(output).toContain("nsis");
    expect(output).toContain("macOS.hardenedRuntime");
    expect(output).toContain("windows.signCommand");
    expect(output).toContain("windows.wix.upgradeCode");
    expect(output).toContain("security.assetProtocol.enable");
    expect(output).toContain("security.assetProtocol.scope.noBroadAppdata");
    expect(output).toContain("security.assetProtocol.scope.mediaDownloads");
    expect(output).toContain("security.csp.img-src.koushiThumbnail");
  });

  test("manual QA script lists every Milestone 9 flow", () => {
    const output = runScript("scripts/desktop-manual-qa.mjs", ["--list"]);

    for (const flow of [
      "login",
      "restore",
      "recovery",
      "search",
      "edit",
      "redaction",
      "logout",
      "account switch",
      "shortcut parity",
      "right-panel behavior",
      "settings placement",
      "Space info/settings"
    ]) {
      expect(output).toContain(flow);
    }
  });

  test("mac GUI smoke script lists automated first-run checks", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", ["--list"]);

    for (const check of [
      "launch Tauri dev shell",
      "verify main window",
      "optional real login from stdin",
      "optional reusable QA profile for restored sync state",
      "optional synthetic send smoke message",
      "verify QA title panel token after shortcuts",
      "open Keyboard settings shortcut",
      "open User settings shortcut",
      "capture private-data-free screenshots",
      "stop app process group"
    ]) {
      expect(output).toContain(check);
    }
  });

  test("mac GUI smoke script parses the QA panel token without launching the GUI", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-panel=koushi-desktop qa session=ready sync=running rooms=1 spaces=0 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=keyboardSettings"
    ]);

    expect(output.trim()).toBe("keyboardSettings");
  });

  test("mac GUI smoke only skips panel checks while recovery owns the panel", () => {
    const readyRecoveryPanel = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-panel-ready=koushi-desktop qa session=ready sync=running rooms=1 spaces=0 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=recovery",
      "--required-panel=keyboardSettings"
    ]);
    const recoveryPanel = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-panel-ready=koushi-desktop qa session=needsRecovery sync=running rooms=1 spaces=0 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=recovery",
      "--required-panel=keyboardSettings"
    ]);
    const keyboardPanel = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-panel-ready=koushi-desktop qa session=ready sync=running rooms=1 spaces=0 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=keyboardSettings",
      "--required-panel=keyboardSettings"
    ]);
    const erroredPanel = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-panel-ready=koushi-desktop qa session=ready sync=running rooms=1 spaces=0 active_room=true timeline_subscribed=true timeline_items=1 errors=1 panel=keyboardSettings",
      "--required-panel=keyboardSettings"
    ]);

    expect(readyRecoveryPanel.trim()).toBe("not-ready");
    expect(recoveryPanel.trim()).toBe("ready");
    expect(keyboardPanel.trim()).toBe("ready");
    expect(erroredPanel.trim()).toBe("not-ready");
  });

  test("release preflight validates mac GUI smoke entry", () => {
    const output = runScript("scripts/desktop-release-preflight.mjs", ["--check-config"]);

    expect(output).toContain("package.scripts.qa:mac-gui");
  });

  test("macOS Keychain Tier 2 workflow stays disabled while retaining the temporary-keychain recipe", () => {
    const workflowUrl = new URL(
      "../../../../.github/workflows/macos-keychain-tier2.yml",
      import.meta.url
    );
    const disabledWorkflowUrl = new URL(
      "../../../../.github/workflows.disabled/macos-keychain-tier2.yml",
      import.meta.url
    );

    expect(existsSync(workflowUrl)).toBe(false);
    expect(existsSync(disabledWorkflowUrl)).toBe(true);

    const workflow = readFileSync(disabledWorkflowUrl, "utf8");

    for (const token of [
      "workflow_dispatch:",
      "runs-on: macos-latest",
      "uses: actions/checkout@v6",
      "Prepare standalone key crate",
      'cp -R crates/koushi-key/. "$RUNNER_TEMP/koushi-key/"',
      'KOUSHI_MACOS_KEYCHAIN_QA: "1"',
      'cargo test --manifest-path "$RUNNER_TEMP/koushi-key/Cargo.toml" credential_backend_macos_temporary_keychain_round_trip_is_env_gated -- --nocapture',
      'cargo test --manifest-path "$RUNNER_TEMP/koushi-key/Cargo.toml" credential_backend'
    ]) {
      expect(workflow).toContain(token);
    }

    expect(workflow).not.toContain("KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR");
    expect(workflow).not.toContain("submodules:");
  });

  test("release preflight validates linux GUI smoke entry", () => {
    const output = runScript("scripts/desktop-release-preflight.mjs", ["--check-config"]);

    expect(output).toContain("package.scripts.qa:linux-gui");
  });

  test("release preflight validates real account QA entry", () => {
    const output = runScript("scripts/desktop-release-preflight.mjs", ["--check-config"]);

    expect(output).toContain("package.scripts.qa:real-account");
  });

  test("real homeserver QA runner forwards scenario selection to the binary", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-real-homeserver-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("--scenario");
    expect(source).toContain("KOUSHI_REAL_QA_SCENARIO");
    expect(source).toContain("compat|space_compat|all");
  });

  test("real homeserver QA binary names the staged real-server scenarios", () => {
    const source = readFileSync(
      new URL("../../../../crates/koushi-core/src/bin/real-homeserver-qa.rs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("KOUSHI_REAL_QA_SCENARIO");
    expect(source).toContain("RealQaScenario");
    expect(source).toContain("SpaceCompat");
    expect(source).toContain("All");
  });

  test("real homeserver QA treats space projection as an observation token", () => {
    const source = readFileSync(
      new URL("../../../../crates/koushi-core/src/bin/real-homeserver-qa.rs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("real_space_projection=observed");
    expect(source).toContain("real_space_projection=not_observed");
  });

  test("real homeserver QA runner enforces the private-data-free token contract", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-real-homeserver-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("./lib/qa-token-contract.mjs");
    expect(source).toContain("assertNoMatrixIdentifiers");
    expect(source).toContain("assertNoLocalPaths");
    expect(source).toContain("assertNoRawSdkErrors");
    expect(source).toContain("assertRequiredTokens");
    expect(source).toContain("requiredTokensForScenario");
  });

  test("real homeserver QA runner checks private data before writing artifacts", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-real-homeserver-qa.mjs", import.meta.url),
      "utf8"
    );

    const writeLogOffset = source.indexOf("writeFileSync(logPath");
    const matrixIdCheckOffset = source.indexOf("assertNoMatrixIdentifiers(combinedOutput");
    const localPathCheckOffset = source.indexOf("assertNoLocalPaths(combinedOutput");

    expect(matrixIdCheckOffset).toBeGreaterThan(-1);
    expect(localPathCheckOffset).toBeGreaterThan(-1);
    expect(writeLogOffset).toBeGreaterThan(-1);
    expect(matrixIdCheckOffset).toBeLessThan(writeLogOffset);
    expect(localPathCheckOffset).toBeLessThan(writeLogOffset);
  });

  test("real homeserver QA runner stdout omits local paths and raw child output", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-real-homeserver-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).not.toContain("run dir = ${runDir}");
    expect(source).not.toContain("credentials file = ${credentialsPath}");
    expect(source).not.toContain("stdout: ${stdout");
    expect(source).not.toContain("stderr: ${stderr");
    expect(source).not.toContain("log: ${logPath}");
    expect(source).not.toContain("PASSED. Log");
    expect(source).toContain("child output omitted after private-data validation");
  });

  test("real homeserver QA binary emits private-data-free tokens (no Matrix ids)", () => {
    const source = readFileSync(
      new URL("../../../../crates/koushi-core/src/bin/real-homeserver-qa.rs", import.meta.url),
      "utf8"
    );

    // No token line or summary may interpolate a Matrix identifier.
    expect(source).not.toContain("event_id={");
    expect(source).not.toContain("user_id={");
    expect(source).not.toContain("room_id={");
    expect(source).not.toContain("space_id={");
    expect(source).not.toContain("user={user_id}");
    expect(source).not.toContain("{expected_event_id}");
    expect(source).not.toContain("{space_id}");
    expect(source).not.toContain("{child_room_id}");
    expect(source).not.toContain("space={ev_space}");
    expect(source).not.toContain("child={ev_child}");
  });

  test("qa token contract helper exposes token and private-data assertions", () => {
    const source = readFileSync(
      new URL("../../../../scripts/lib/qa-token-contract.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("export function tokensFromOutput");
    expect(source).toContain("export function assertRequiredTokens");
    expect(source).toContain("export function assertNoMatrixIdentifiers");
    expect(source).toContain("export function assertNoLocalPaths");
    expect(source).toContain("export function assertNoRawSdkErrors");
    expect(source).not.toContain("${match[1]}");
  });

  test("release preflight validates headless local QA entry", () => {
    const output = runScript("scripts/desktop-release-preflight.mjs", ["--check-config"]);

    expect(output).toContain("package.scripts.qa:headless-local");
  });

  test("package scripts expose the headless basic QA aggregators", () => {
    const packageJson = JSON.parse(
      readFileSync(new URL("../../../../apps/desktop/package.json", import.meta.url), "utf8")
    );
    const localHeadlessCoreReleaseQa =
      "node ../../scripts/desktop-headless-local-qa.mjs --run --server=both --core --scenario=login_sync,directory,timeline_reconnect --timeout-ms=600000 --cargo-profile=release && node ../../scripts/desktop-headless-local-qa.mjs --run --server=conduit --core --scenario=send_queue --timeout-ms=600000 --cargo-profile=release";

    expect(packageJson.scripts?.["qa:headless-basic:local"]).toBe(localHeadlessCoreReleaseQa);
    expect(packageJson.scripts?.["qa:headless-basic:real"]).toBe(
      "node ../../scripts/desktop-real-homeserver-qa.mjs --run --scenario=space_compat"
    );
  });

  test("headless basic operations docs list the default real space_compat tokens", () => {
    const docs = readFileSync(
      new URL("../../../../docs/qa/headless-basic-operations.md", import.meta.url),
      "utf8"
    );

    for (const token of [
      "login=ok",
      "sync=running",
      "real_reply=ok",
      "real_space_create=ok",
      "real_space_child=ok",
      "real_space_cleanup=ok",
      "logout=ok",
      "post_logout_restore=not_found"
    ]) {
      expect(docs).toContain(token);
    }
  });

  test("headless basic operations docs list the Phase 11 local thread tokens", () => {
    const docs = readFileSync(
      new URL("../../../../docs/qa/headless-basic-operations.md", import.meta.url),
      "utf8"
    );

    for (const token of [
      "thread_hidden=ok",
      "thread_summary=ok",
      "thread_recv=ok",
      "thread_paginate=end_reached"
    ]) {
      expect(docs).toContain(token);
    }
    expect(docs).not.toContain("thread=ok");
  });

  test("package scripts expose the linux GUI smoke runner", () => {
    const packageJson = JSON.parse(
      readFileSync(new URL("../../../../apps/desktop/package.json", import.meta.url), "utf8")
    );

    expect(packageJson.scripts?.["qa:linux-gui"]).toBe(
      "node ../../scripts/desktop-linux-gui-qa.mjs --run"
    );
  });

  test("linux GUI smoke script lists the expected foundation checks", () => {
    const output = runScript("scripts/desktop-linux-gui-qa.mjs", ["--list"]);

    for (const check of [
      "verify Xvfb virtual display",
      "verify tauri-driver and WebKitWebDriver",
      "verify debug Tauri build",
      "drive WebdriverIO session",
      "exercise real IPC and DOM smoke",
      "optional local homeserver login via FIFO",
      "clean process teardown"
    ]) {
      expect(output).toContain(check);
    }
  });

  test("linux GUI smoke lists the local-login and local-send scenarios", () => {
    const output = runScript("scripts/desktop-linux-gui-qa.mjs", ["--list"]);

    for (const token of ["signed-out", "local-login", "local-send"]) {
      expect(output).toContain(token);
    }
  });

  test("linux GUI smoke lists the local basic-operation scenarios", () => {
    const output = runScript("scripts/desktop-linux-gui-qa.mjs", ["--list"]);

    for (const token of [
      "scenario local-create-room",
      "scenario local-create-space",
      "scenario local-invites-dm",
      "scenario local-reply",
      "scenario local-media",
      "scenario local-room-tags",
      "scenario local-room-management",
      "scenario local-explore",
      "scenario local-message-actions",
      "scenario local-pins",
      "scenario local-composer",
      "scenario local-scheduled-send",
      "scenario local-timeline-navigation",
      "scenario local-alias",
      "scenario local-cjk",
      "scenario local-settings",
      "verify local-settings trust section"
    ]) {
      expect(output).toContain(token);
    }
  });

  test("linux GUI smoke supports the fast skip-build inner loop", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("--skip-build");
    expect(source).toContain("--app-binary");
    expect(source).toContain("async function ensureAppBinary(");
  });

  test("linux GUI smoke source emits the basic-operation success tokens", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("gui_local_create_room=ok");
    expect(source).toContain("gui_local_create_space=ok");
    expect(source).toContain("gui_local_invite_accept=ok");
    expect(source).toContain("gui_local_dm_start=ok");
    expect(source).toContain("gui_local_reply=ok");
    expect(source).toContain("gui_local_media=ok");
    expect(source).toContain("gui_local_room_tag_set=ok");
    expect(source).toContain("gui_local_room_tag_removed=ok");
    expect(source).toContain("gui_local_room_topic=ok");
    expect(source).toContain("gui_local_room_kick=ok");
    expect(source).toContain("gui_local_message_source=ok");
    expect(source).toContain("gui_local_message_forward=ok");
    expect(source).toContain("gui_local_hide_redacted=ok");
    expect(source).toContain("gui_local_mention=ok");
    expect(source).toContain("gui_local_markdown=ok");
    expect(source).toContain("gui_local_slash=ok");
    expect(source).toContain("gui_local_scheduled_create=ok");
    expect(source).toContain("gui_local_scheduled_reschedule=ok");
    expect(source).toContain("gui_local_scheduled_cancel=ok");
    expect(source).toContain("gui_local_settings=ok");
    expect(source).toContain("gui_local_trust_settings=ok");
  });

  test("linux GUI composer smoke drives real controls without IPC mocking", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("async function runLocalComposerScenario()");
    expect(source).toContain('textarea[aria-label="Message composer"]');
    expect(source).toContain('button[role="option"]');
    expect(source).toContain('button[aria-label="Bold"]');
    expect(source).toContain("Mention Helper");
    expect(source).toContain("sendRoomMessage(");
    expect(source).not.toContain("installTauriInvokeRecorder(");
  });

  test("linux GUI room-tag smoke drives context menu and Rust-owned section movement", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("async function runLocalRoomTagsScenario()");
    expect(source).toContain('button[data-testid="room-item"]');
    expect(source).toContain('button[role="menuitem"]');
    expect(source).toContain("Add to Favourites");
    expect(source).toContain("Remove from Favourites");
    expect(source).toContain('data-room-section="favourites"');
    expect(source).toContain('data-room-section="rooms"');
    expect(source).toContain("waitForRoomInSection(");
    expect(source).not.toContain("installTauriInvokeRecorder(");
  });

  test("linux GUI room-management smoke drives Rust-owned settings and member state", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("async function runLocalRoomManagementScenario()");
    expect(source).toContain('textarea[aria-label="Room topic"]');
    expect(source).toContain("Save topic");
    expect(source).toContain(".settings-detail-row");
    expect(source).toContain(".room-member-row");
    expect(source).toContain('button[data-action="kick"]');
    expect(source).toContain("waitForRoomManagementTopic(");
    expect(source).not.toContain("installTauriInvokeRecorder(");
  });

  test("linux GUI message-action smoke drives real action menu controls", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("async function runLocalMessageActionsScenario()");
    expect(source).toContain("waitForLatestMessageActionButton(");
    expect(source).toContain('button[aria-label="Message actions"]');
    expect(source).toContain("View source");
    expect(source).toContain("Message source");
    expect(source).toContain("Forward");
    expect(source).toContain("Redact message");
    expect(source).toContain("Hide deleted messages");
    expect(source).toContain('.message[data-redacted="true"]');
    expect(source).toContain("QA Seed Room");
    expect(source).toContain("QA message action seed");
    expect(source).not.toContain("installTauriInvokeRecorder(");
  });

  test("linux GUI media smoke drives the hidden file input without a native dialog", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("setSyntheticFileInput(");
    expect(source).toContain("makeFileInputInteractable(");
    expect(source).toContain("dispatchFileInputChange(");
    expect(source).toContain("DataTransfer");
    expect(source).toContain(".message-media");
    expect(source).toContain("Download ${filename}");
    expect(source).not.toContain("verifyTauriInvokeRecorder(");
    expect(source).not.toContain("installTauriInvokeRecorder(");
  });

  test("headless basic operations docs mention the local create, reply, and media GUI scenarios", () => {
    const docs = readFileSync(
      new URL("../../../../docs/qa/headless-basic-operations.md", import.meta.url),
      "utf8"
    );

    expect(docs).toContain("--scenario=local-create-room");
    expect(docs).toContain("--scenario=local-create-space");
    expect(docs).toContain("--scenario=local-invites-dm");
    expect(docs).toContain("--scenario=local-reply");
    expect(docs).toContain("--scenario=local-media");
    expect(docs).toContain("--scenario=local-room-tags");
    expect(docs).toContain("--scenario=local-room-management");
    expect(docs).toContain("--scenario=local-explore");
    expect(docs).toContain("--scenario=local-message-actions");
    expect(docs).toContain("--scenario=local-pins");
    expect(docs).toContain("--scenario=local-composer");
    expect(docs).toContain("--scenario=local-scheduled-send");
    expect(docs).toContain("--scenario=local-timeline-navigation");
    expect(docs).toContain("--scenario=local-alias");
    expect(docs).toContain("--scenario=local-cjk");
    expect(docs).toContain("--scenario=local-settings");
    expect(docs).toContain("gui_local_create_room=ok");
    expect(docs).toContain("gui_local_invite_accept=ok");
    expect(docs).toContain("gui_local_dm_start=ok");
    expect(docs).toContain("gui_local_reply=ok");
    expect(docs).toContain("gui_local_media=ok");
    expect(docs).toContain("gui_local_room_tag_set=ok");
    expect(docs).toContain("gui_local_room_tag_removed=ok");
    expect(docs).toContain("gui_local_room_topic=ok");
    expect(docs).toContain("gui_local_room_kick=ok");
    expect(docs).toContain("gui_local_message_source=ok");
    expect(docs).toContain("gui_local_message_forward=ok");
    expect(docs).toContain("gui_local_hide_redacted=ok");
    expect(docs).toContain("gui_local_mention=ok");
    expect(docs).toContain("gui_local_scheduled_create=ok");
    expect(docs).toContain("gui_local_scheduled_reschedule=ok");
    expect(docs).toContain("gui_local_scheduled_cancel=ok");
    expect(docs).toContain("gui_local_markdown=ok");
    expect(docs).toContain("gui_local_slash=ok");
    expect(docs).toContain("gui_local_alias_set=ok");
    expect(docs).toContain("gui_local_alias_clear=ok");
    expect(docs).toContain("gui_local_cjk=ok");
    expect(docs).toContain("gui_local_settings=ok");
    expect(docs).toContain("gui_local_trust_settings=ok");
  });

  test("linux GUI smoke resolves relative artifact dirs from the repo root", () => {
    const output = execFileSync(
      process.execPath,
      [
        "../../scripts/desktop-linux-gui-qa.mjs",
        "--print-artifact-root",
        "--artifact-dir=artifacts/linux-gui-local-login"
      ],
      {
        cwd: `${repoRoot}apps/desktop`,
        encoding: "utf8"
      }
    );

    expect(output.trim()).toBe(
      new URL("../../../../artifacts/linux-gui-local-login", import.meta.url).pathname
    );
  });

  test("linux GUI smoke source emits the local scenario success tokens", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("gui_local_login=ok");
    expect(source).toContain("gui_local_send=ok");
    expect(source).toContain("gui_local_logout=ok");
    expect(source).toContain("gui_local_relogin=ok");
    expect(source).toContain("gui_local_spaces_home=ok");
    expect(source).toContain("gui_local_spaces_nav=ok");
    expect(source).toContain("gui_local_spaces_info=ok");
    expect(source).toContain("gui_local_explore_query=ok");
    expect(source).toContain("gui_local_explore_join=ok");
    expect(source).toContain("gui_local_room_topic=ok");
    expect(source).toContain("gui_local_room_kick=ok");
    expect(source).toContain("gui_local_alias_set=ok");
    expect(source).toContain("gui_local_alias_clear=ok");
    expect(source).toContain("gui_local_scheduled_create=ok");
    expect(source).toContain("gui_local_scheduled_cancel=ok");
    expect(source).toContain("gui_local_timeline_unread_jump=ok");
    expect(source).toContain("gui_local_timeline_date_jump=ok");
    expect(source).toContain("waitForTimelineFocusedContextReady");
    expect(source).toContain("timelineDateJumpDiagnostics");
    expect(source).toContain("setDatetimeLocalValue");
    expect(source).toContain("gui_local_cjk=ok");
  });

  test("linux GUI local logout/relogin uses the gated QA control pipe", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("local-logout-relogin");
    expect(source).toContain("KOUSHI_QA_CONTROL_PIPE");
    expect(source).toContain("qa-control.pipe");
    expect(source).toContain('JSON.stringify({ command: "logout" })');
    expect(source).toContain("requestQaLogout");
    expect(source).toContain("submitLoginForm");
    expect(source).toMatch(
      /function childEnvironment\(dataDir, qaLoginPipePath = null, qaControlPipePath = null\)/
    );
    expect(source).toMatch(
      /if \(qaControlPipePath\) \{[\s\S]*env\.KOUSHI_QA_CONTROL_PIPE = qaControlPipePath;/
    );
  });

  test("linux GUI local spaces navigation checks rail selection and space info", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("local-spaces-nav");
    expect(source).toContain("waitForWorkspaceActive");
    expect(source).toContain("clickWorkspaceButton");
    expect(source).toContain("gui_local_spaces_home=ok");
    expect(source).toContain("gui_local_spaces_nav=ok");
    expect(source).toContain("gui_local_spaces_info=ok");
  });

  test("linux GUI local scenarios also emit DBus and window-state evidence", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("recordLocalGuiEvidence");
    expect(source).toContain("notification_dbus=ok");
    expect(source).toContain("window_state_path_contract=ok");
    expect(source).toContain("run_dir=artifact");
    expect(source).not.toContain("window_state_path=${");
    expect(source).not.toContain("run_dir=${");
    expect(source).toMatch(
      /async function runLocalLoginScenario\(\)[\s\S]*await recordLocalGuiEvidence\(session\);[\s\S]*gui_local_login=ok/
    );
    expect(source).toMatch(
      /async function runLocalSendScenario\(\)[\s\S]*await recordLocalGuiEvidence\(session\);[\s\S]*gui_local_send=ok/
    );
  });

  test("linux GUI local login selects the first room when timeline subscription is still missing", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("shouldSelectFirstRoom(status, selectedRoom)");
    expect(source).toMatch(
      /function shouldSelectFirstRoom\(status, selectedRoom\)[\s\S]*status\.active_room === false \|\| status\.timeline_subscribed === false/
    );
    expect(source).toMatch(
      /if \(shouldSelectFirstRoom\(status, selectedRoom\)\) \{[\s\S]*selectedRoom = await selectFirstRoom\(browser\);/
    );
  });

  test("linux GUI smoke parses the attention baseline title token", () => {
    const output = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--qa-title-attention-ready=koushi-desktop qa session=signedOut sync=stopped rooms=0 spaces=0 active_room=false timeline_subscribed=false timeline_items=0 errors=0 unread=0 badge=0 notify=none"
    ]);

    expect(output.trim()).toBe("ready");
  });

  test("linux GUI smoke validates the persisted window-state path contract", () => {
    const output = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--qa-window-state-ready=/tmp/koushi-desktop/app-shell/window-state.json"
    ]);

    expect(output.trim()).toBe("ready");
  });

  test("linux GUI smoke wires dbus notification evidence into the signed-out run path", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("dbus-daemon");
    expect(source).toContain("--session");
    expect(source).toContain("--address");
    expect(source).toContain("dbus-monitor");
    expect(source).toContain("NSS_WRAPPER_PASSWD");
    expect(source).toContain("notification_dbus=ok");
    expect(source).toContain("triggerNotificationSmoke");
  });

  test("linux GUI smoke child environment filters secrets and enables QA file credentials", () => {
    const output = execFileSync(
      process.execPath,
      ["scripts/desktop-linux-gui-qa.mjs", "--child-env"],
      {
        cwd: repoRoot,
        encoding: "utf8",
        env: {
          ...process.env,
          DEEPSEEK_API_KEY: "synthetic-secret",
          KOUSHI_CORE_ACTOR_TRACE: "1",
          KOUSHI_TEST_SECRET: "synthetic-secret"
        }
      }
    );

    expect(output).toContain("KOUSHI_DATA_DIR=");
    expect(output).toContain("KOUSHI_QA_TITLE=1");
    expect(output).toContain("VITE_KOUSHI_QA_TITLE=1");
    expect(output).toContain("KOUSHI_SKIP_SAVED_SESSIONS=1");
    expect(output).toContain("KOUSHI_SKIP_KEYCHAIN_PERSISTENCE=1");
    expect(output).toContain("KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR=");
    expect(output).toContain("KOUSHI_CORE_ACTOR_TRACE=1");
    expect(output).toContain("/qa-credential-store");
    expect(output).toContain("NO_COLOR=1");
    expect(output).not.toContain("DEEPSEEK_API_KEY");
    expect(output).not.toContain("KOUSHI_TEST_SECRET");
  });

  test("linux GUI smoke child environment exposes only safe QA keys for local login", () => {
    const output = execFileSync(
      process.execPath,
      ["scripts/desktop-linux-gui-qa.mjs", "--child-env-keys", "--real-login-from-stdin"],
      {
        cwd: repoRoot,
        encoding: "utf8",
        env: {
          ...process.env,
          DEEPSEEK_API_KEY: "synthetic-secret",
          KOUSHI_TEST_SECRET: "synthetic-secret"
        }
      }
    );

    expect(output).toContain("KOUSHI_DATA_DIR");
    expect(output).toContain("KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR");
    expect(output).toContain("KOUSHI_QA_LOGIN_PIPE");
    expect(output).toContain("KOUSHI_QA_CONTROL_PIPE");
    expect(output).not.toContain("DEEPSEEK_API_KEY");
    expect(output).not.toContain("KOUSHI_TEST_SECRET");
  });

  test("Tauri crate is owned by the root Cargo workspace", () => {
    const rootCargo = readFileSync(new URL("../../../../Cargo.toml", import.meta.url), "utf8");
    const tauriCargo = readFileSync(
      new URL("../../../../apps/desktop/src-tauri/Cargo.toml", import.meta.url),
      "utf8"
    );
    const releaseGate = readFileSync(
      new URL("../../../../scripts/desktop-release-gate-check.mjs", import.meta.url),
      "utf8"
    );

    expect(rootCargo).toContain('"apps/desktop/src-tauri"');
    expect(tauriCargo).not.toMatch(/^\[workspace\]$/m);
    expect(releaseGate).toContain('"koushi-desktop"');
    expect(releaseGate).not.toContain('"apps", "desktop", "src-tauri"');
  });

  test("local and real homeserver QA preserve shared Cargo target dir", () => {
    const localQaSource = readFileSync(
      new URL("../../../../scripts/lib/local-homeserver-qa.mjs", import.meta.url),
      "utf8"
    );
    const realQaSource = readFileSync(
      new URL("../../../../scripts/desktop-real-homeserver-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(localQaSource).toMatch(/"CARGO_TARGET_DIR"/);
    expect(realQaSource).toMatch(/"CARGO_TARGET_DIR"/);
  });

  test("linux GUI smoke source wires the shared local homeserver helper module", () => {
    const guiSource = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );
    const sharedSource = readFileSync(
      new URL("../../../../scripts/lib/local-homeserver-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(guiSource).toContain("local-homeserver-qa.mjs");
    expect(guiSource).toContain("local-login");
    expect(guiSource).toContain("local-send");
    expect(guiSource).not.toContain("--password");
    expect(sharedSource).toContain("checkInstalledHomeserver");
    expect(sharedSource).toContain("registerUser");
    expect(sharedSource).toContain("stopProcess");
  });

  test("local Synapse QA config relaxes room creation limits for synthetic stress seeds", () => {
    const sharedSource = readFileSync(
      new URL("../../../../scripts/lib/local-homeserver-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(sharedSource).toContain("rc_room_creation:");
    expect(sharedSource).toMatch(/rc_room_creation:\n\s+per_second: 1000\n\s+burst_count: 1000/);
  });

  test("local Synapse QA config allows synthetic public room directory publication", () => {
    const sharedSource = readFileSync(
      new URL("../../../../scripts/lib/local-homeserver-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(sharedSource).toContain("room_list_publication_rules:");
    expect(sharedSource).toMatch(/room_list_publication_rules:\n\s+- action: allow/);
  });

  test("linux GUI local setup keeps homeserver data separate and cleanup covers setup failures", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("serverDataDir");
    expect(source).toContain("homeserver-data");
    expect(source).toContain("const session = {");
    expect(source).toContain("await cleanupLocalGuiScenario(session)");
  });

  test("linux GUI local setup defines the safe timestamp helper it uses for synthetic users", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("const userSuffix = safeTimestamp();");
    expect(source).toContain("function safeTimestamp()");
    expect(source).toContain('replaceAll("-", "_")');
  });

  test("linux GUI smoke real login transport is FIFO and the script avoids password args", () => {
    const transport = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--print-real-login-transport"
    ]);
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(transport.trim()).toBe("fifo");
    expect(source).toContain("readRealLoginCredentials");
    expect(source).toContain("writeRealLoginPipe");
    expect(source).toContain("requestQaLogout(qaControlPipePath)");
    expect(source).toContain("KOUSHI_QA_LOGIN_PIPE");
    expect(source).toContain("KOUSHI_QA_CONTROL_PIPE");
    expect(source).not.toContain("--password");
  });

  test("linux GUI smoke prints WebDriver capabilities for the app binary", () => {
    const output = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--print-webdriver-capabilities",
      "--app-binary=/tmp/app"
    ]);

    expect(JSON.parse(output)).toEqual(
      expect.objectContaining({
        browserName: "wry",
        "wdio:enforceWebDriverClassic": true,
        "tauri:options": expect.objectContaining({
          application: "/tmp/app"
        })
      })
    );
    expect(JSON.parse(output)["tauri:options"]).not.toHaveProperty("args");
  });

  test("linux GUI smoke run path now wires WebdriverIO and the signed-out screenshot", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("webdriverio");
    expect(source).toContain('createRequire(new URL("../apps/desktop/package.json"');
    expect(source).toContain("importDesktopWebdriverio");
    expect(source).toContain("remote({");
    expect(source).toContain("screenshots/01-signed-out.png");
    expect(source).not.toContain("foundation is wired, but the WebDriver session");
  });

  test("linux GUI smoke launches Xvfb with the sanitized child environment", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("const xvfb = await startXvfb(logPath, buildEnv);");
    expect(source).toContain("async function startXvfb(logPath, buildEnv)");
    expect(source).toContain("env: buildEnv");
    expect(source).not.toContain("env: process.env");
  });

  test("linux GUI Docker recipe pins Rust 1.96.0 and keeps the tauri-driver mitigation", () => {
    const dockerfile = readFileSync(
      new URL("../../../../docker/linux-gui.Dockerfile", import.meta.url),
      "utf8"
    );

    for (const token of [
      "node:22.22.3-bookworm",
      "ARG RUST_TOOLCHAIN=1.96.0",
      "ARG CONDUIT_URL=https://gitlab.com/api/v4/projects/famedly%2Fconduit/jobs/artifacts/master/raw/x86_64-unknown-linux-musl?job=artifacts",
      "ARG TUWUNEL_VERSION=v1.7.1",
      "ARG TUWUNEL_ZST_URL=https://github.com/matrix-construct/tuwunel/releases/download/v1.7.1/v1.7.1-release-all-x86_64-v1-linux-gnu-tuwunel.zst",
      "RUST_TOOLCHAIN=${RUST_TOOLCHAIN}",
      '--default-toolchain "${RUST_TOOLCHAIN}"',
      'rustup default "${RUST_TOOLCHAIN}"',
      'RUSTUP_TOOLCHAIN="${RUST_TOOLCHAIN}"',
      "libwebkit2gtk-4.1-dev",
      "libayatana-appindicator3-dev",
      "zstd",
      "webkit2gtk-driver",
      "xvfb",
      "fonts-noto-color-emoji",
      "cargo install tauri-driver --locked",
      "curl --proto '=https' --tlsv1.2 -fsSLo /usr/local/bin/conduit",
      "curl --proto '=https' --tlsv1.2 -fsSLo /tmp/tuwunel.zst",
      "unzstd -f -o /usr/local/bin/tuwunel /tmp/tuwunel.zst",
      "conduit --version",
      "tuwunel --version",
      'RUSTC="$(rustup which rustc)"',
      'RUSTDOC="$(rustup which rustdoc)"'
    ]) {
      expect(dockerfile).toContain(token);
    }
  });

  test("linux GUI container docs use bash -c and the audited artifact lane", () => {
    const agents = readFileSync(new URL("../../../../AGENTS.md", import.meta.url), "utf8");

    expect(agents).toContain("bash -c");
    expect(agents).not.toContain("bash -lc");
    expect(agents).toContain('-u "$(id -u):$(id -g)"');
    expect(agents).toContain("-v /tmp/koushi-desktop-cargo-home:/tmp/cargo-home");
    expect(agents).toContain("-v /tmp/koushi-desktop-gui-target:/tmp/koushi-desktop-gui-target");
    expect(agents).toContain("-v /tmp/koushi-desktop-npm-cache:/tmp/npm-cache");
    expect(agents).toContain("CARGO_HOME=/tmp/cargo-home");
    expect(agents).toContain("CARGO_TARGET_DIR=/tmp/koushi-desktop-gui-target");
    expect(agents).toContain("NPM_CONFIG_CACHE=/tmp/npm-cache");
    expect(agents).toContain("koushi-desktop-linux-gui:basic-ops");
    expect(agents).toContain("--scenario=local-send");
    expect(agents).toContain("--server=conduit");
    expect(agents).toContain("--artifact-dir=/work/artifacts/linux-gui-local-send-docker");
    expect(agents).toContain("--timeout-ms=180000");
    expect(agents).toContain("conduit");
    expect(agents).toContain("tuwunel");
    expect(agents).toContain("zstd");
    expect(agents).toContain(
      "PATH=/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
    );
    expect(agents).toContain('RUSTC="$(rustup which rustc)"');
    expect(agents).toContain('RUSTDOC="$(rustup which rustdoc)"');
  });

  test("linux GUI smoke QA title helpers match the mac runner contract", () => {
    const ready = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--qa-title-ready=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=closed"
    ]);
    const readyRecovered = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--qa-title-ready-require-recovered=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=closed"
    ]);
    const panel = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--qa-title-panel=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=keyboardSettings"
    ]);
    const panelReady = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--qa-title-panel-ready=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=0 panel=keyboardSettings",
      "--required-panel=keyboardSettings"
    ]);
    const sendReady = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--qa-title-send-ready=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=0 send=sent panel=closed"
    ]);
    const mismatchedTimeline = runScript("scripts/desktop-linux-gui-qa.mjs", [
      "--qa-title-ready=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_room=true timeline_matches_active=false timeline_subscribed=true timeline_items=1 errors=0 panel=closed"
    ]);

    expect(ready.trim()).toBe("ready");
    expect(readyRecovered.trim()).toBe("ready");
    expect(panel.trim()).toBe("keyboardSettings");
    expect(panelReady.trim()).toBe("ready");
    expect(sendReady.trim()).toBe("ready");
    expect(mismatchedTimeline.trim()).toBe("not-ready");
  });

  test("linux GUI smoke QA title contract uses the local send statuses", () => {
    const titleSource = readFileSync(
      new URL("../../../../apps/desktop/src/domain/qaTitle.ts", import.meta.url),
      "utf8"
    );
    const sendSource = readFileSync(
      new URL("../../../../apps/desktop/src/domain/qaSendSmoke.ts", import.meta.url),
      "utf8"
    );

    expect(titleSource).toContain("send=");
    expect(sendSource).toContain('"idle"');
    expect(sendSource).toContain('"pending"');
    expect(sendSource).toContain('"sent"');
    expect(sendSource).toContain('"failed"');
  });

  test("app wires Tauri CoreEvent send completion into the QA send title token", () => {
    const source = readFileSync(
      new URL("../../../../apps/desktop/src/App.tsx", import.meta.url),
      "utf8"
    );

    expect(source).toContain("qaSendCompletionStatusFromCoreEvent");
    expect(source).toContain("SendCompleted");
    expect(source).toContain("OperationFailed");
    expect(source).toContain("setQaSendStatus(eventStatus)");
  });

  test("app lets Tauri snapshot errors fail the QA send title token", () => {
    const source = readFileSync(
      new URL("../../../../apps/desktop/src/App.tsx", import.meta.url),
      "utf8"
    );

    expect(source).toContain('completionStatus !== "failed"');
    expect(source).toMatch(/isTauriRuntime\(\) &&\s*completionStatus !== "failed"[\s\S]*return;/);
  });

  test("app keeps Tauri send completion listener mounted and gates events with a pending ref", () => {
    const source = readFileSync(
      new URL("../../../../apps/desktop/src/App.tsx", import.meta.url),
      "utf8"
    );

    expect(source).toContain("const qaSendPending = useRef(false)");
    expect(source).toMatch(
      /useEffect\(\(\) => \{[\s\S]*if \(!isTauriRuntime\(\)\) \{[\s\S]*listen<CoreEventPayload>\(CORE_EVENT_NAME,[\s\S]*qaSendPending\.current[\s\S]*qaSendCompletionStatusFromCoreEvent[\s\S]*setQaSendStatus\(eventStatus\);[\s\S]*\}, \[\]\);/
    );
    expect(source).toMatch(
      /qaSendStarted\.current = true;[\s\S]*qaSendPending\.current = true;[\s\S]*setQaSendStatus\("pending"\);/
    );
    expect(source).toMatch(
      /async function sendText\([^)]*\)[\s\S]*qaSendPending\.current = true;[\s\S]*setQaSendStatus\("pending"\);/
    );
  });

  test("linux GUI local login retries room selection until a displayed row is clicked", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("selectedRoom = await selectFirstRoom(browser);");
    expect(source).toMatch(
      /async function selectFirstRoom\(browser\)[\s\S]*return false;[\s\S]*await roomItems\[0\]\.click\(\);[\s\S]*return true;/
    );
  });

  test("headless local QA script lists homeserver and SDK checks", () => {
    const output = runScript("scripts/desktop-headless-local-qa.mjs", ["--list"]);

    for (const check of [
      "verify installed Conduit binary",
      "verify installed Tuwunel binary",
      "start disposable local homeserver",
      "register synthetic local users",
      "run headless Matrix SDK operations",
      "stop disposable local homeserver"
    ]) {
      expect(output).toContain(check);
    }
  });

  test("headless local QA script imports the shared local homeserver helper module", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-headless-local-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("local-homeserver-qa.mjs");
    expect(source).toContain("checkInstalledHomeserver");
    expect(source).toContain("registerUser");
    expect(source).toContain("stopProcess");
  });

  test("headless local QA script lists staged scenarios", () => {
    const output = runScript("scripts/desktop-headless-local-qa.mjs", ["--list"]);

    for (const scenario of [
      "scenario safety",
      "scenario login_sync",
      "scenario room_space",
      "scenario directory",
      "scenario room_management",
      "scenario timeline",
      "scenario composer",
      "scenario credential_health",
      "scenario reply",
      "scenario media",
      "scenario thread",
      "scenario edit_redact_search",
      "scenario restore_cleanup"
    ]) {
      expect(output).toContain(scenario);
    }
  });

  test("headless local QA forwards the selected scenario to core QA", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-headless-local-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("--scenario");
    expect(source).toContain("KOUSHI_QA_SCENARIO");
  });

  test("headless local QA forwards explicit Rust diagnostics env to core QA", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-headless-local-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("KOUSHI_QA_RUST_LOG");
    expect(source).toContain("KOUSHI_QA_RUST_BACKTRACE");
    expect(source).toContain("env.RUST_LOG = process.env.KOUSHI_QA_RUST_LOG");
    expect(source).not.toContain('"RUST_LOG",');
    expect(source).not.toContain('"RUST_BACKTRACE",');
    expect(source).toContain("KOUSHI_QA_E2EE_PAUSE_SYNC_BEFORE_MULTI_DEVICE_SEND");
  });

  test("headless local QA exposes strict E2EE multi-device options", () => {
    const usage = runScript("scripts/desktop-headless-local-qa.mjs");
    const source = readFileSync(
      new URL("../../../../scripts/desktop-headless-local-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(usage).toContain("--e2ee-recipient-second-device");
    expect(usage).toContain("--e2ee-pause-sync-before-multi-device-send");
    expect(source).toContain("e2eeRecipientSecondDeviceOption");
    expect(source).toContain('env.KOUSHI_QA_E2EE_RECIPIENT_SECOND_DEVICE = "true"');
    expect(source).toContain('env.KOUSHI_QA_E2EE_PAUSE_SYNC_BEFORE_MULTI_DEVICE_SEND = "true"');
    expect(source.indexOf("if (e2eeRecipientSecondDeviceOption)")).toBeGreaterThan(
      source.indexOf("for (const name of [")
    );
  });

  test("headless local QA can replay a saved Synapse fixture without mutating the source data", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-headless-local-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("--fixture-run");
    expect(source).toContain("loadQaFixture");
    expect(source).toContain("copyFixtureDataDir");
    expect(source).toContain("KOUSHI_QA_STRESS_REPLAY_EXISTING");
    expect(source).toMatch(/cpSync\(fixture\.dataDir,\s*dataDir,\s*\{[\s\S]*recursive: true/);
    expect(source).not.toContain("-v `${fixture.dataDir}:/data`");
  });

  test("headless local QA stores fixture credentials only under the ignored local secrets run dir", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-headless-local-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("fixture.json");
    expect(source).toContain("writeQaFixture");
    expect(source).toContain("serverName");
    expect(source).toContain("passwordA");
    expect(source).toContain("passwordB");
    expect(source).toContain(".local-secrets");
    expect(source).not.toContain("console.log(fixture");
  });

  test("headless local QA runner preserves raw child logs before public privacy validation", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-headless-local-qa.mjs", import.meta.url),
      "utf8"
    );

    const firstValidation = source.indexOf("assertQaOutputIsPrivate(");
    const firstWrite = source.indexOf("writeQaOutputFiles(");

    expect(source).toContain("./lib/qa-token-contract.mjs");
    expect(source).toContain("assertNoMatrixIdentifiers");
    expect(source).toContain("assertNoLocalPaths");
    expect(source).toContain("assertNoRawSdkErrors");
    expect(firstValidation).toBeGreaterThan(-1);
    expect(firstWrite).toBeGreaterThan(-1);
    expect(firstWrite).toBeLessThan(firstValidation);
  });

  test("headless local QA failure messages do not replay raw child output or paths", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-headless-local-qa.mjs", import.meta.url),
      "utf8"
    );

    expect(source).not.toContain("stdout=${stdout");
    expect(source).not.toContain("stderr=${stderr");
    expect(source).not.toContain("see ${logPath}");
    expect(source).toContain("child output omitted after private-data validation");
  });

  test("headless local QA configs bind only to loopback disposable stores", () => {
    const conduit = runScript("scripts/desktop-headless-local-qa.mjs", ["--print-conduit-config"]);
    const tuwunel = runScript("scripts/desktop-headless-local-qa.mjs", ["--print-tuwunel-config"]);

    expect(conduit).toContain('address = "127.0.0.1"');
    expect(conduit).toContain('database_path = "/tmp/conduit-data"');
    expect(conduit).toContain("allow_federation = false");
    expect(tuwunel).toContain('address = ["127.0.0.1"]');
    expect(tuwunel).toContain('database_path = "/tmp/tuwunel-data"');
    expect(tuwunel).toContain("allow_federation = false");
  });

  test("headless basic operations docs mention the Linux GUI local scenarios and aggregators", () => {
    const docs = readFileSync(
      new URL("../../../../docs/qa/headless-basic-operations.md", import.meta.url),
      "utf8"
    );

    expect(docs).toContain("qa:headless-basic:local");
    expect(docs).toContain("qa:linux-gui");
    expect(docs).toContain("--scenario=local-login");
    expect(docs).toContain("--scenario=local-send");
    expect(docs).toContain("gui_local_login=ok");
    expect(docs).toContain("gui_local_send=ok");
  });

  test("headless basic operations docs describe the bundled Linux GUI homeserver binaries", () => {
    const docs = readFileSync(
      new URL("../../../../docs/qa/headless-basic-operations.md", import.meta.url),
      "utf8"
    );

    expect(docs).toContain("conduit");
    expect(docs).toContain("tuwunel");
    expect(docs).toContain("zstd");
    expect(docs).toContain("unzstd");
  });

  test("mac GUI smoke child environment excludes secret-like variables", () => {
    const output = execFileSync(
      process.execPath,
      ["scripts/desktop-mac-gui-smoke.mjs", "--child-env-keys"],
      {
        cwd: repoRoot,
        encoding: "utf8",
        env: {
          ...process.env,
          DEEPSEEK_API_KEY: "synthetic-secret",
          KOUSHI_TEST_SECRET: "synthetic-secret"
        }
      }
    );

    expect(output).toContain("PATH");
    expect(output).toContain("KOUSHI_RESTORE_SESSION");
    expect(output).toContain("KOUSHI_SKIP_SAVED_SESSIONS");
    expect(output).not.toContain("DEEPSEEK_API_KEY");
    expect(output).not.toContain("KOUSHI_TEST_SECRET");
  });

  test("mac GUI smoke preserves shared Cargo target dir without exposing secrets", () => {
    const output = execFileSync(
      process.execPath,
      ["scripts/desktop-mac-gui-smoke.mjs", "--child-env-keys"],
      {
        cwd: repoRoot,
        encoding: "utf8",
        env: {
          ...process.env,
          CARGO_TARGET_DIR: "/tmp/koushi-desktop-shared-target",
          DEEPSEEK_API_KEY: "synthetic-secret",
          KOUSHI_TEST_SECRET: "synthetic-secret"
        }
      }
    );

    expect(output).toContain("CARGO_TARGET_DIR");
    expect(output).not.toContain("DEEPSEEK_API_KEY");
    expect(output).not.toContain("KOUSHI_TEST_SECRET");
  });

  test("mac GUI smoke can opt into SDK error diagnostics without forwarding secret env values", () => {
    const output = execFileSync(
      process.execPath,
      ["scripts/desktop-mac-gui-smoke.mjs", "--child-env"],
      {
        cwd: repoRoot,
        encoding: "utf8",
        env: {
          ...process.env,
          KOUSHI_DEBUG_SDK_ERROR: "synthetic-secret-value"
        }
      }
    );

    expect(output).toContain("KOUSHI_DEBUG_SDK_ERROR=1");
    expect(output).not.toContain("synthetic-secret-value");
  });

  test("mac GUI smoke real login mode enables QA title without exposing credentials in args", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--child-env-keys",
      "--real-login-from-stdin"
    ]);
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    expect(output).toContain("VITE_KOUSHI_QA_TITLE");
    expect(output).toContain("KOUSHI_QA_TITLE");
    expect(source).toContain("--real-login-from-stdin");
    expect(source).not.toContain("--password");
  });

  test("mac GUI smoke real login uses FIFO transport instead of credential keystrokes", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", ["--print-real-login-transport"]);
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    expect(output.trim()).toBe("fifo");
    expect(source).toContain("KOUSHI_QA_LOGIN_PIPE");
    expect(source).not.toContain("clickAndReplace");
  });

  test("mac GUI smoke real login avoids post-login screenshot artifacts", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("skip real login screenshot");
    expect(source).toContain("skip profile screenshot");
    expect(source).toContain("allowPrivateScreenshots");
    expect(source).toContain("postLoginScreenshotsAreAllowed");
    expect(source).not.toContain("02-real-login.png");
  });

  test("mac GUI smoke can update the native QA title from the frontend", () => {
    const capability = readFileSync(
      new URL("../../../../apps/desktop/src-tauri/capabilities/default.json", import.meta.url),
      "utf8"
    );

    expect(capability).toContain("core:window:allow-set-title");
  });

  test("mac GUI smoke has a frontend boot error title before App imports", () => {
    const mainSource = readFileSync(
      new URL("../../../../apps/desktop/src/main.tsx", import.meta.url),
      "utf8"
    );
    const bootCaptureSource = readFileSync(
      new URL("../../../../apps/desktop/src/bootErrorCapture.ts", import.meta.url),
      "utf8"
    );
    const bootImportOffset = mainSource.indexOf("./bootErrorCapture");
    const appImportOffset = mainSource.indexOf("./App");

    expect(bootImportOffset).toBeGreaterThanOrEqual(0);
    expect(appImportOffset).toBeGreaterThanOrEqual(0);
    expect(bootImportOffset).toBeLessThan(appImportOffset);
    expect(bootCaptureSource).toContain("session=booting");
    expect(bootCaptureSource).toContain("session=boot_error");
    expect(bootCaptureSource).toContain("error_kind=");
  });

  test("Tauri dev capability explicitly grants the Vite dev URL", () => {
    const capability = JSON.parse(
      readFileSync(
        new URL("../../../../apps/desktop/src-tauri/capabilities/default.json", import.meta.url),
        "utf8"
      )
    );

    expect(capability.remote.urls).toContain("http://127.0.0.1:5173/*");
  });

  test("Tauri opener capability grants both URL command and http scopes", () => {
    const capability = JSON.parse(
      readFileSync(
        new URL("../../../../apps/desktop/src-tauri/capabilities/default.json", import.meta.url),
        "utf8"
      )
    );

    expect(capability.permissions).toContain("opener:allow-open-url");
    expect(capability.permissions).toContain("opener:allow-default-urls");
  });

  test("Tauri window capability grants custom titlebar dragging", () => {
    const capability = JSON.parse(
      readFileSync(
        new URL("../../../../apps/desktop/src-tauri/capabilities/default.json", import.meta.url),
        "utf8"
      )
    );

    expect(capability.permissions).toContain("core:window:allow-start-dragging");
  });

  test("Tauri launch explicitly makes the main WebView window visible", () => {
    const source = readFileSync(
      new URL("../../../../apps/desktop/src-tauri/src/lib.rs", import.meta.url),
      "utf8"
    );
    const setupSource = source.split(".setup(move |app|").at(1)?.split(".on_window_event").at(0);

    expect(source).toContain("ensure_main_window_visible");
    expect(setupSource).toContain("ensure_main_window_visible(app)");
    expect(source).toContain("set_activation_policy");
    expect(source).toContain("run_on_main_thread");
    expect(source).toContain("activateIgnoringOtherApps");
    expect(source).toContain("makeKeyAndOrderFront");
    expect(source).toContain("orderFrontRegardless");
    expect(source).toContain("qa_window_visibility_mode_enabled");
    expect(source).toContain("set_visible_on_all_workspaces(true)");
    expect(source).toContain("window.unminimize()");
    expect(source).toContain("window.show()");
    expect(source).toContain("window.set_focus()");
  });

  test("Tauri repeats main window activation after the WebView page loads", () => {
    const source = readFileSync(
      new URL("../../../../apps/desktop/src-tauri/src/lib.rs", import.meta.url),
      "utf8"
    );
    const pageLoadSource = source.split(".on_page_load(").at(1)?.split(".on_window_event").at(0);

    expect(pageLoadSource).toContain("ensure_main_window_visible");
    expect(pageLoadSource).toContain('webview.label() == "main"');
  });

  test("mac GUI smoke real login uses the QA file store instead of macOS Keychain", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--child-env",
      "--real-login-from-stdin"
    ]);

    expect(output).toContain("KOUSHI_SKIP_KEYCHAIN_PERSISTENCE=1");
    expect(output).toContain("KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR=");
    expect(output).toContain("qa-credential-store");
  });

  test("mac GUI smoke drives a logout cleanup over the QA control pipe for real login", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    // A second debug/test-only FIFO carries control commands to the app.
    expect(source).toContain("KOUSHI_QA_CONTROL_PIPE");
    expect(source).toContain("qa-control.pipe");
    // The runner writes a logout command and waits for a signed-out QA title
    // before terminating the process group (no stale device survives the run).
    expect(source).toContain('JSON.stringify({ command: "logout" })');
    expect(source).toContain("requestQaLogout");
    expect(source).toContain("waitForQaSignedOut");
    expect(source).toContain("--keep-session");
    // The cleanup runs in teardown after credentials were handed to the app:
    // a failed ready gate can still leave a real device/session behind.
    expect(source).toMatch(
      /finally \{[\s\S]*if \(qaControlPipePath && realLoginCleanupRequired && !keepSession\)[\s\S]*requestQaLogout\(qaControlPipePath\);[\s\S]*waitForQaSignedOut\(timeoutMs, diagnostics\);[\s\S]*terminateProcessGroup\(child, "SIGTERM"\);/
    );
  });

  test("mac GUI smoke control pipe rides the filtered child environment, not the parent env", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    // The control pipe path is threaded through the allow-listed childEnvironment
    // helper, never via process.env passthrough.
    expect(source).toContain("childEnvironment(dataDir, qaLoginPipePath, qaControlPipePath)");
    expect(source).toMatch(
      /function childEnvironment\(dataDir, qaLoginPipePath = null, qaControlPipePath = null\)/
    );
    expect(source).toMatch(
      /if \(qaControlPipePath\) \{[\s\S]*env\.KOUSHI_QA_CONTROL_PIPE = qaControlPipePath;/
    );
  });

  test("mac GUI smoke reusable profile keeps restore and saved sessions enabled", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--child-env",
      "--qa-profile=agent-sync"
    ]);

    expect(output).toContain("KOUSHI_RESTORE_SESSION=1");
    expect(output).toContain("KOUSHI_SKIP_SAVED_SESSIONS=0");
    expect(output).toContain(".local-secrets/qa-profiles/agent-sync/data");
    expect(output).toContain("KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR=");
    expect(output).toContain(".local-secrets/qa-profiles/agent-sync/data/qa-credential-store");
    expect(output).not.toContain("KOUSHI_SKIP_KEYCHAIN_PERSISTENCE");
  });

  test("Tauri debug runtime honors the keychain persistence bypass env", () => {
    const source = readFileSync(
      new URL("../../../../apps/desktop/src-tauri/src/lib.rs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("KOUSHI_SKIP_KEYCHAIN_PERSISTENCE");
    expect(source).toContain("keychain_persistence_disabled_from_env");
    expect(source).toContain("CoreRuntime::start_with_data_dir(data_dir.clone())");
    expect(source).toContain("CoreRuntime::start_with_data_dir_and_os_backend");
  });

  test("Tauri production adapter does not depend on the fixture backend crate", () => {
    const tauriCargo = readFileSync(
      new URL("../../../../apps/desktop/src-tauri/Cargo.toml", import.meta.url),
      "utf8"
    );
    const tauriLib = readFileSync(
      new URL("../../../../apps/desktop/src-tauri/src/lib.rs", import.meta.url),
      "utf8"
    );

    expect(tauriCargo).not.toContain("koushi-backend");
    expect(tauriLib).not.toContain("koushi_backend");
    expect(tauriLib).not.toContain("BackendState");
    expect(tauriLib).not.toContain("TimelineTaskHandle");
    expect(tauriLib).not.toContain("TimelinePaginationRequest");
  });

  test("desktop package exposes a local DMG build script", () => {
    const packageJson = JSON.parse(
      readFileSync(new URL("../../../../apps/desktop/package.json", import.meta.url), "utf8")
    );
    const scriptPath = new URL("../../../../scripts/desktop-build-dmg.mjs", import.meta.url);
    const source = readFileSync(scriptPath, "utf8");

    expect(packageJson.scripts["build:dmg"]).toBe("node ../../scripts/desktop-build-dmg.mjs");
    expect(source).toContain("tauri");
    expect(source).toContain("build");
    expect(source).toContain("--bundles");
    expect(source).toContain("dmg");
    expect(source).toContain("Application Support/koushi-desktop");
    expect(source).toContain("koushi-desktop");
    expect(source).not.toContain("Application Support/matrix-desktop");
  });

  test("active runtime storage identifiers use Koushi without matrix-desktop compatibility", () => {
    const activeSourceFiles = [
      "apps/desktop/src/App.tsx",
      "apps/desktop/src/bootErrorCapture.ts",
      "apps/desktop/src-tauri/src/lib.rs",
      "apps/desktop/src-tauri/src/commands/mod.rs",
      "crates/koushi-core/src/store.rs",
      "crates/koushi-core/src/runtime.rs",
      "crates/koushi-core/src/sync.rs",
      "crates/koushi-core/src/bin/headless-core-qa.rs",
      "crates/koushi-core/src/bin/real-homeserver-qa.rs",
      "crates/koushi-sdk/src/lib.rs",
      "crates/koushi-key/src/lib.rs",
      "scripts/desktop-build-dmg.mjs",
      "scripts/desktop-headless-local-qa.mjs",
      "scripts/desktop-linux-gui-qa.mjs",
      "scripts/desktop-mac-gui-smoke.mjs",
      "scripts/desktop-real-homeserver-qa.mjs"
    ];

    for (const file of activeSourceFiles) {
      const source = readFileSync(new URL(`../../../../${file}`, import.meta.url), "utf8");
      expect(source, file).not.toContain("MATRIX_DESKTOP_");
      expect(source, file).not.toContain("VITE_MATRIX_DESKTOP_");
      expect(source, file).not.toContain("matrix-desktop://");
      expect(source, file).not.toContain("matrix-desktop:");
      expect(source, file).not.toContain("LEGACY_DATA_DIR_NAME");
      expect(source, file).not.toContain("LEGACY_CREDENTIAL_STORE_SERVICE_NAME");
      expect(source, file).not.toContain("migrate_app_data_dir_if_needed");
      expect(source, file).not.toContain("app.kagome");
      expect(source, file).not.toContain("RURI-");
    }
  });

  test("mac GUI smoke send smoke mode passes only a synthetic body through child env", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--child-env",
      "--send-smoke-message=Koushi synthetic QA send"
    ]);
    const sendLine = output
      .split("\n")
      .find((line) => line.startsWith("VITE_KOUSHI_QA_SEND_SMOKE_MESSAGE="));

    expect(sendLine).toBe("VITE_KOUSHI_QA_SEND_SMOKE_MESSAGE=Koushi synthetic QA send");
    expect(sendLine).not.toContain("password");
  });

  test("mac GUI smoke can target a real DM user for synthetic send smoke", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--child-env",
      "--send-smoke-message=Koushi synthetic QA send",
      "--send-smoke-user-id=@hiroshi.shinaoka:matrix.org"
    ]);
    const source = readFileSync(
      new URL("../../../../apps/desktop/src/App.tsx", import.meta.url),
      "utf8"
    );

    expect(output).toContain("VITE_KOUSHI_QA_SEND_SMOKE_USER_ID=@hiroshi.shinaoka:matrix.org");
    expect(source).toContain("qaSendSmokeTargetUserId");
    expect(source).toContain("api.startDirectMessage(targetUserId)");
    expect(source).toContain("api.selectRoom(targetRoom.room_id)");
  });

  test("mac GUI smoke send smoke uses a bounded send timeout separate from login", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("const sendTimeoutMs");
    expect(source).toContain('optionValue("--send-timeout-ms") ?? "30000"');
    expect(source).toContain("waitForQaSend(sendTimeoutMs, diagnostics)");
  });

  test("mac GUI smoke defaults the real-login wait to thirty seconds", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain('optionValue("--timeout-ms") ?? "30000"');
  });

  test("mac GUI smoke fails fast when QA title reports errors during ready wait", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("qaStatusHasBlockingError");
    expect(source).toContain("QA reported an error before ready");
  });

  test("mac GUI smoke verbose mode records private-data-free QA diagnostics", () => {
    const usage = runScript("scripts/desktop-mac-gui-smoke.mjs");
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    expect(usage).toContain("--verbose");
    expect(source).toContain('const verbose = args.has("--verbose")');
    expect(source).toContain("qa-diagnostics.log");
    expect(source).toContain("recordQaPoll");
    expect(source).toContain("diagnostics path:");
  });

  test("mac GUI smoke keeps target DM encryption diagnostics in summaries", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain('"target_dm"');
    expect(source).toContain('"target_selected"');
    expect(source).toContain('"target_members"');
  });

  test("mac GUI smoke keeps timeline and crawler counters in diagnostics summaries", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    for (const key of [
      "timeline_visible",
      "timeline_dl",
      "timeline_backfill",
      "crawler_running",
      "crawler_completed",
      "crawler_failed",
      "crawler_processed",
      "crawler_indexed"
    ]) {
      expect(source).toContain(`"${key}"`);
    }
  });

  test("mac GUI smoke keeps rendered DOM counters in diagnostics summaries", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    for (const key of ["dom_screen", "dom_root_children", "dom_text_len"]) {
      expect(source).toContain(`"${key}"`);
    }
  });

  test("Tauri dev uses a refresh-free Vite mode compatible with the desktop CSP", () => {
    const tauriConfig = JSON.parse(
      readFileSync(
        new URL("../../../../apps/desktop/src-tauri/tauri.conf.json", import.meta.url),
        "utf8"
      )
    );
    const packageJson = JSON.parse(
      readFileSync(new URL("../../../../apps/desktop/package.json", import.meta.url), "utf8")
    );
    const viteConfig = readFileSync(
      new URL("../../../../apps/desktop/vite.config.ts", import.meta.url),
      "utf8"
    );

    expect(tauriConfig.build.beforeDevCommand).toBe("npm run dev:tauri");
    expect(packageJson.scripts["dev:tauri"]).toContain("--mode tauri");
    expect(viteConfig).toContain('mode === "tauri"');
    expect(viteConfig).toContain("hmr: false");
    expect(tauriConfig.app.security.devCsp).toContain("http://127.0.0.1:5173");
    expect(tauriConfig.app.security.devCsp).toContain("ws://127.0.0.1:5173");
    for (const csp of [tauriConfig.app.security.csp, tauriConfig.app.security.devCsp]) {
      expect(csp).toContain("img-src");
      expect(csp).toContain("asset:");
      expect(csp).toContain("http://asset.localhost");
      expect(csp).toContain("koushi-thumbnail:");
      expect(csp).toContain("http://koushi-thumbnail.localhost");
    }
    expect(tauriConfig.app.security.assetProtocol.scope).toEqual([
      "$LOCALDATA/koushi-desktop/media_downloads/**"
    ]);
  });

  test("QA file credential store is gated to debug, test, and qa-bin builds in core", () => {
    // The credential store moved into koushi-core (StoreActor) when
    // src-tauri became a pure transport adapter; the compile-time gate lives
    // there now.
    const coreStore = readFileSync(
      new URL("../../../../crates/koushi-core/src/store.rs", import.meta.url),
      "utf8"
    );

    expect(coreStore).toContain("const ENV_FILE_CREDENTIAL_STORE_DIR");
    expect(coreStore).toMatch(
      /#\[cfg\(any\(debug_assertions, test, feature = "qa-bin"\)\)\]\nconst ENV_FILE_CREDENTIAL_STORE_DIR/
    );
    expect(coreStore).toMatch(
      /#\[cfg\(any\(debug_assertions, test, feature = "qa-bin"\)\)\]\n(?:#\[derive\([^\n]+\)\]\n)?pub struct FileCredentialStore/
    );

    // The transport adapter must not read the credential store at all — not
    // even the QA file-dir override env.
    const adapter = readFileSync(
      new URL("../../../../apps/desktop/src-tauri/src/lib.rs", import.meta.url),
      "utf8"
    );
    expect(adapter).not.toContain("KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR");
    expect(adapter).not.toContain("CredentialStore");
  });

  test("mac GUI smoke rejects unsafe reusable profile names", () => {
    for (const profileName of ["", "../secret"]) {
      const result = spawnSync(
        process.execPath,
        ["scripts/desktop-mac-gui-smoke.mjs", "--child-env", `--qa-profile=${profileName}`],
        {
          cwd: repoRoot,
          encoding: "utf8"
        }
      );

      expect(result.status).not.toBe(0);
      expect(result.stderr).toContain(
        "qa profile must be 1-64 characters of letters, numbers, underscore, or dash"
      );
    }
  });

  test("mac GUI smoke accepts recovery-required sessions after room timeline QA is ready", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-ready=koushi-desktop qa session=needsRecovery sync=running rooms=109 spaces=4 active_room=true timeline_subscribed=true timeline_items=8 errors=0 panel=recovery"
    ]);

    expect(output.trim()).toBe("ready");
  });

  test("mac GUI smoke can relax timeline item count for sparse QA accounts", () => {
    const strict = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-ready=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=0 errors=0 panel=closed"
    ]);
    const relaxed = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--allow-empty-timeline",
      "--qa-title-ready=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=0 errors=0 panel=closed"
    ]);

    expect(strict.trim()).toBe("not-ready");
    expect(relaxed.trim()).toBe("ready");
  });

  test("mac GUI smoke rejects active/timeline room mismatches", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-ready=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_room=true timeline_matches_active=false timeline_subscribed=true timeline_items=1 errors=0 panel=closed"
    ]);

    expect(output.trim()).toBe("not-ready");
  });

  test("mac GUI smoke rejects ready titles with backend errors", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-ready=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=1 panel=closed"
    ]);

    expect(output.trim()).toBe("not-ready");
  });

  test("mac GUI smoke waits for send smoke success token", () => {
    const pending = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-send-ready=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=0 send=pending panel=closed"
    ]);
    const sent = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-send-ready=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=2 errors=0 send=sent panel=closed"
    ]);
    const failed = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-send-ready=koushi-desktop qa session=ready sync=running rooms=2 spaces=1 active_room=true timeline_subscribed=true timeline_items=1 errors=1 send=failed panel=closed"
    ]);

    expect(pending.trim()).toBe("not-ready");
    expect(sent.trim()).toBe("ready");
    expect(failed.trim()).toBe("not-ready");
  });

  test("mac GUI smoke requires ready session when recovery code is supplied", () => {
    const waiting = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-ready-require-recovered=koushi-desktop qa session=needsRecovery sync=running rooms=109 spaces=4 active_room=true timeline_subscribed=true timeline_items=8 errors=0 panel=recovery"
    ]);
    const recovered = runScript("scripts/desktop-mac-gui-smoke.mjs", [
      "--qa-title-ready-require-recovered=koushi-desktop qa session=ready sync=running rooms=109 spaces=4 active_room=true timeline_subscribed=true timeline_items=8 errors=0 panel=keyboardSettings"
    ]);

    expect(waiting.trim()).toBe("not-ready");
    expect(recovered.trim()).toBe("ready");
  });

  test("mac GUI smoke uses whose clauses for variable process names", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", ["--print-window-query-script"]);

    expect(output).toContain("first process whose name is candidateName");
    expect(output).not.toContain("exists process candidateName");
    expect(output).not.toContain("tell process candidateName");
  });

  test("mac GUI smoke captures only the app window bounds", () => {
    const output = runScript("scripts/desktop-mac-gui-smoke.mjs", ["--print-screenshot-args"]);

    expect(output).toContain("-R");
    expect(output).toContain("10,20,300,400");
    expect(output).not.toContain("fullscreen");
  });

  test("mac GUI smoke does not send Cmd+Q while cleaning up", () => {
    const source = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    expect(source).toContain("terminateProcessGroup");
    expect(source).not.toContain('keystroke "q" using command down');
  });

  test("GUI smoke FIFO writers use node:fs/promises open and never spawn tee", () => {
    const linuxSource = readFileSync(
      new URL("../../../../scripts/desktop-linux-gui-qa.mjs", import.meta.url),
      "utf8"
    );
    const macSource = readFileSync(
      new URL("../../../../scripts/desktop-mac-gui-smoke.mjs", import.meta.url),
      "utf8"
    );

    for (const source of [linuxSource, macSource]) {
      // The sensitive-payload writer must use a direct node:fs/promises FIFO
      // write so no child process inherits the parent environment.
      expect(source).toContain('import { open } from "node:fs/promises";');
      expect(source).toContain(
        "async function writeSensitivePayloadToPath(path, payload, timeout)"
      );
      expect(source).toContain("await open(path, ");
      // No `tee` helper process anywhere (it would inherit the parent env).
      expect(source).not.toContain('spawn("tee"');
      expect(source).not.toContain('"tee"');
    }
  });

  test("app icon family is consistent and the SVG avoids an unintended white rounded frame", () => {
    const tauriDir = new URL("../../../../apps/desktop/src-tauri/", import.meta.url);
    const conf = JSON.parse(readFileSync(new URL("tauri.conf.json", tauriDir), "utf8")) as {
      bundle: { icon: string[] };
    };

    for (const iconPath of conf.bundle.icon) {
      expect(existsSync(new URL(iconPath, tauriDir))).toBe(true);
    }

    const svgPath = new URL("icons/icon.svg", tauriDir);
    expect(existsSync(svgPath)).toBe(true);
    const svg = readFileSync(svgPath, "utf8");

    // The icon must not use a plain white rounded rectangle as its outer frame.
    const whiteFramePattern = /<rect[^>]*\sfill="(#FFFFFF|white|#fff)"[^>]*\brx="/i;
    expect(whiteFramePattern.test(svg)).toBe(false);

    // The icon set referenced by Tauri must include the source SVG.
    expect(conf.bundle.icon).toContain("icons/icon.svg");
  });
});
