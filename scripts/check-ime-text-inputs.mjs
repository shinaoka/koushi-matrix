#!/usr/bin/env node

import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import ts from "../apps/desktop/node_modules/typescript/lib/typescript.js";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const desktopSourceRoot = join(repoRoot, "apps", "desktop", "src");
const nativePrimitivePath = "apps/desktop/src/components/ImeTextControl.tsx";
const composableInputTypes = new Set(["", "email", "password", "search", "tel", "text", "url"]);

function jsxTagName(node) {
  return ts.isIdentifier(node.tagName) ? node.tagName.text : null;
}

function literalInputType(node) {
  const typeAttribute = node.attributes.properties.find(
    (attribute) => ts.isJsxAttribute(attribute) && attribute.name.text === "type"
  );
  if (!typeAttribute) {
    return "";
  }
  if (!ts.isJsxAttribute(typeAttribute) || !typeAttribute.initializer) {
    return null;
  }
  if (ts.isStringLiteral(typeAttribute.initializer)) {
    return typeAttribute.initializer.text.toLowerCase();
  }
  if (
    ts.isJsxExpression(typeAttribute.initializer) &&
    typeAttribute.initializer.expression &&
    ts.isStringLiteralLike(typeAttribute.initializer.expression)
  ) {
    return typeAttribute.initializer.expression.text.toLowerCase();
  }
  return null;
}

function hasContentEditable(node) {
  return node.attributes.properties.some(
    (attribute) =>
      ts.isJsxAttribute(attribute) &&
      attribute.name.text.toLowerCase() === "contenteditable"
  );
}

export function findImeTextInputViolations(sourceText, fileName) {
  const normalizedFileName = fileName.replaceAll("\\", "/");
  if (normalizedFileName.endsWith(nativePrimitivePath)) {
    return [];
  }

  const sourceFile = ts.createSourceFile(
    fileName,
    sourceText,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX
  );
  const violations = [];

  function add(node, message) {
    const location = sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile));
    violations.push({
      file: fileName,
      line: location.line + 1,
      column: location.character + 1,
      message
    });
  }

  function visit(node) {
    if (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) {
      const tagName = jsxTagName(node);
      if (hasContentEditable(node)) {
        add(node, "contentEditable text surfaces require an approved IME-safe primitive");
      }
      if (tagName === "form") {
        add(node, "use ImeSafeForm instead of a raw form");
      } else if (tagName === "textarea") {
        add(node, "use ImeTextArea instead of a raw textarea");
      } else if (tagName === "input") {
        const inputType = literalInputType(node);
        if (inputType === null || composableInputTypes.has(inputType)) {
          add(
            node,
            inputType === "password"
              ? "use SecureImeTextField instead of a raw password input"
              : "use ImeTextField instead of a raw text input"
          );
        }
      }
    }
    ts.forEachChild(node, visit);
  }

  visit(sourceFile);
  return violations;
}

function collectProductionTsxFiles(directory) {
  const files = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const fullPath = join(directory, entry.name);
    if (entry.isDirectory()) {
      if (entry.name !== "__tests__" && entry.name !== "test") {
        files.push(...collectProductionTsxFiles(fullPath));
      }
    } else if (
      entry.name.endsWith(".tsx") &&
      !entry.name.includes(".test.") &&
      !entry.name.includes(".spec.")
    ) {
      files.push(fullPath);
    }
  }
  return files;
}

export function checkDesktopImeTextInputs() {
  return collectProductionTsxFiles(desktopSourceRoot).flatMap((filePath) => {
    const fileName = relative(repoRoot, filePath).replaceAll("\\", "/");
    return findImeTextInputViolations(readFileSync(filePath, "utf8"), fileName);
  });
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const violations = checkDesktopImeTextInputs();
  if (violations.length === 0) {
    console.log("check-ime-text-inputs: ok — all text-entry surfaces use IME-safe primitives.");
  } else {
    console.error(
      "check-ime-text-inputs: FAILED — text-entry surfaces must use the shared IME-safe primitives."
    );
    for (const violation of violations) {
      console.error(
        `  ${violation.file}:${violation.line}:${violation.column}: ${violation.message}`
      );
    }
    process.exitCode = 1;
  }
}
