use super::*;
use koushi_diagnostics::DiagnosticValue;
use std::collections::HashMap;

fn records_since(
    cursor: koushi_diagnostics::test_support::DetailCursor,
) -> Vec<koushi_diagnostics::DiagnosticRecord> {
    koushi_diagnostics::test_support::detail_records_since(cursor)
        .into_iter()
        .filter(|record| record.event.source == SOURCE)
        .collect()
}

fn fields(
    record: &koushi_diagnostics::DiagnosticRecord,
) -> HashMap<&'static str, &DiagnosticValue> {
    record
        .event
        .fields
        .iter()
        .map(|field| (field.key, &field.value))
        .collect()
}

#[test]
fn request_path_records_carry_counts_and_closed_tokens_only() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let cursor = koushi_diagnostics::test_support::detail_cursor();

    record_window("open");
    record_issued("recovery_window", 3);
    record_requested("recovery_window", 3, 42);
    record_window("close");

    let records = records_since(cursor);
    let stages: Vec<_> = records.iter().map(|record| record.event.stage).collect();
    assert_eq!(stages, ["open", "issued", "requested", "close"]);

    let issued = fields(&records[1]);
    assert_eq!(
        issued.get("kind"),
        Some(&&DiagnosticValue::Token("history_key_request"))
    );
    assert_eq!(
        issued.get("trigger"),
        Some(&&DiagnosticValue::Token("recovery_window"))
    );
    assert_eq!(issued.get("recipients"), Some(&&DiagnosticValue::Count(3)));

    let requested = fields(&records[2]);
    assert_eq!(
        requested.get("recipients"),
        Some(&&DiagnosticValue::Count(3))
    );
    assert_eq!(
        requested.get("elapsed_ms"),
        Some(&&DiagnosticValue::Milliseconds(42))
    );
}

#[test]
fn request_path_records_never_carry_identifiers() {
    let _diagnostic_lock = koushi_diagnostics::test_support::lock();
    let cursor = koushi_diagnostics::test_support::detail_cursor();

    record_window("open");
    record_issued("startup", 128);
    record_requested("startup", 128, u128::MAX);
    record_window("close");

    for record in records_since(cursor) {
        assert!(
            record
                .event
                .stage
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '_'),
            "stage {} is not a closed token",
            record.event.stage
        );
        for field in &record.event.fields {
            match &field.value {
                // Every free-form value is a closed lowercase token, so no
                // room id, user id, device id or key material can reach a QA
                // artifact through a diagnostic field.
                DiagnosticValue::Token(token) => assert!(
                    token
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                    "field {} carries a non-token value",
                    field.key
                ),
                DiagnosticValue::Boolean(_)
                | DiagnosticValue::Count(_)
                | DiagnosticValue::Correlation(_)
                | DiagnosticValue::Milliseconds(_)
                | DiagnosticValue::RequestId { .. }
                | DiagnosticValue::OrdinalAlias { .. } => {}
            }
        }
    }
}
