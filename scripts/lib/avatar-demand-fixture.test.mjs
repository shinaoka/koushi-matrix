import assert from "node:assert/strict";
import test from "node:test";

import { classifyAvatarReceiptReadback } from "./avatar-demand-fixture.mjs";

const TUWUNEL = { supported: false, limitation: "sss_receipt_packing_single_reader" };
const SYNAPSE = { supported: true, limitation: "" };

test("declared tuwunel limitation is honored only while the packing loss is measured", () => {
  const classification = classifyAvatarReceiptReadback({
    serverKind: "tuwunel",
    capability: TUWUNEL,
    readback: { stored: 1500, packed: 1 },
    expected: 1500
  });
  assert.equal(classification.limited, true);
  assert.match(classification.token, /^avatar_demand=server_limited /);
  assert.match(classification.token, /reason=sss_receipt_packing_single_reader/);
  assert.match(classification.token, /server=tuwunel/);
  assert.match(classification.token, /receipt_readback=1/);
  assert.match(classification.token, /expected=1500/);
});

test("a server that starts reporting the full population must fail instead of staying skipped", () => {
  assert.throws(
    () =>
      classifyAvatarReceiptReadback({
        serverKind: "tuwunel",
        capability: TUWUNEL,
        readback: { stored: 1500, packed: 1500 },
        expected: 1500
      }),
    /avatar receipt limitation no longer applies/
  );
});

test("an unreadable population cannot be excused by a declared limitation", () => {
  assert.throws(
    () =>
      classifyAvatarReceiptReadback({
        serverKind: "tuwunel",
        capability: TUWUNEL,
        readback: { stored: 1500, packed: null },
        expected: 1500
      }),
    /receipt readback unreadable/
  );
});

test("a supported server must report the seeded population back", () => {
  assert.deepEqual(
    classifyAvatarReceiptReadback({
      serverKind: "synapse",
      capability: SYNAPSE,
      readback: { stored: 1500, packed: 1500 },
      expected: 1500
    }),
    { limited: false }
  );
  assert.throws(
    () =>
      classifyAvatarReceiptReadback({
        serverKind: "synapse",
        capability: SYNAPSE,
        readback: { stored: 1500, packed: 1 },
        expected: 1500
      }),
    /avatar fixture receipt readback failed server=synapse packed=1 stored=1500 expected=1500/
  );
});

test("a supported server may fall back to the stored count when the extension read fails", () => {
  assert.deepEqual(
    classifyAvatarReceiptReadback({
      serverKind: "synapse",
      capability: SYNAPSE,
      readback: { stored: 1500, packed: null },
      expected: 1500
    }),
    { limited: false }
  );
  assert.throws(
    () =>
      classifyAvatarReceiptReadback({
        serverKind: "synapse",
        capability: SYNAPSE,
        readback: { stored: null, packed: null },
        expected: 1500
      }),
    /avatar fixture receipt readback failed server=synapse/
  );
});
