#!/usr/bin/env node

import assert from "node:assert/strict";
import { test } from "node:test";

import { findImeTextInputViolations } from "./check-ime-text-inputs.mjs";

function messages(source) {
  return findImeTextInputViolations(source, "apps/desktop/src/components/Example.tsx")
    .map((violation) => violation.message);
}

test("rejects raw text-entry surfaces and forms", () => {
  assert.deepEqual(messages("export const View = () => <input />;"), [
    "use ImeTextField instead of a raw text input"
  ]);
  assert.deepEqual(messages('export const View = () => <input type="password" />;'), [
    "use SecureImeTextField instead of a raw password input"
  ]);
  assert.deepEqual(messages("export const View = () => <textarea />;"), [
    "use ImeTextArea instead of a raw textarea"
  ]);
  assert.deepEqual(messages("export const View = () => <form />;"), [
    "use ImeSafeForm instead of a raw form"
  ]);
  assert.deepEqual(messages("export const View = () => <div contentEditable />;"), [
    "contentEditable text surfaces require an approved IME-safe primitive"
  ]);
});

test("rejects every composable input type and dynamic type attributes", () => {
  for (const type of ["text", "search", "email", "url", "tel"]) {
    assert.equal(messages(`export const View = () => <input type="${type}" />;`).length, 1);
  }
  assert.equal(messages("export const View = ({ type }) => <input type={type} />;").length, 1);
});

test("allows non-text input controls and shared primitives", () => {
  const source = `
    export const View = () => <>
      <input type="file" />
      <input type="checkbox" />
      <input type="radio" />
      <input type="datetime-local" />
      <ImeTextField />
      <SecureImeTextField />
      <ImeTextArea />
      <ImeSafeForm />
    </>;
  `;
  assert.deepEqual(messages(source), []);
});

test("permits native elements only inside the shared primitive implementation", () => {
  const source = "export const Native = () => <form><input/><textarea/></form>;";
  assert.deepEqual(
    findImeTextInputViolations(
      source,
      "apps/desktop/src/components/ImeTextControl.tsx"
    ),
    []
  );
});
