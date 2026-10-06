#!/usr/bin/env node

import assert from "node:assert/strict";
import { test } from "node:test";

import { findDiagnosticTestIsolationViolations } from "./check-diagnostic-test-isolation.mjs";

test("detects a diagnostic snapshot test without the shared lock", () => {
  const source = `
    #[test]
    fn reads_diagnostics() {
      assert!(!koushi_diagnostics::snapshot().records.is_empty());
    }
  `;
  assert.deepEqual(findDiagnosticTestIsolationViolations(source, "fixture.rs"), [
    "fixture.rs:2:reads_diagnostics"
  ]);
});

test("accepts diagnostic snapshots under the shared lock", () => {
  const source = `
    #[test]
    fn reads_diagnostics() {
      let _diagnostic_lock = koushi_diagnostics::test_support::lock();
      assert!(!koushi_diagnostics::snapshot().records.is_empty());
    }
  `;
  assert.deepEqual(findDiagnosticTestIsolationViolations(source, "fixture.rs"), []);
});

test("accepts async diagnostic snapshots under the awaited shared lock", () => {
  const source = `
    #[tokio::test]
    async fn reads_diagnostics() {
      let _diagnostic_lock = koushi_diagnostics::test_support::lock_async().await;
      assert!(!koushi_diagnostics::snapshot().records.is_empty());
    }
  `;
  assert.deepEqual(findDiagnosticTestIsolationViolations(source, "fixture.rs"), []);
});

test("does not confuse a nested ignored child with its parent test", () => {
  const source = `
    #[test]
    fn launches_child() {
      std::process::Command::new("test").status().unwrap();
    }

    #[test]
    #[ignore]
    fn child() {
      println!("{}", koushi_diagnostics::snapshot().records.len());
    }
  `;
  assert.deepEqual(findDiagnosticTestIsolationViolations(source, "fixture.rs"), [
    "fixture.rs:7:child"
  ]);
});

test("detects detail-ring and cursor readers without the shared lock", () => {
  for (const read of [
    "koushi_diagnostics::test_support::detail_snapshot().records.len()",
    "koushi_diagnostics::test_support::detail_cursor()",
    "test_support::detail_records_since(cursor).len()",
    "test_support::rotation_snapshot().records.len()"
  ]) {
    const source = `
    #[test]
    fn reads_diagnostics() {
      let _ = ${read};
    }
  `;
    assert.deepEqual(
      findDiagnosticTestIsolationViolations(source, "fixture.rs"),
      ["fixture.rs:2:reads_diagnostics"],
      read
    );
  }
});

test("accepts cursor readers under an imported shared lock", () => {
  const source = `
    #[test]
    fn reads_diagnostics() {
      let _guard = test_support::lock();
      let cursor = test_support::detail_cursor();
      assert!(test_support::detail_records_since(cursor).is_empty());
    }
  `;
  assert.deepEqual(findDiagnosticTestIsolationViolations(source, "fixture.rs"), []);
});
