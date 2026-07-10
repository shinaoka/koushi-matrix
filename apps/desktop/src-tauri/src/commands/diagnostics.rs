use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendDiagnosticLogEntry {
    timestamp_ms: u64,
    source: &'static str,
    message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendDiagnosticLogSnapshot {
    entries: Vec<FrontendDiagnosticLogEntry>,
    dropped_entries: u64,
}

fn map_snapshot(snapshot: koushi_diagnostics::DiagnosticSnapshot) -> FrontendDiagnosticLogSnapshot {
    FrontendDiagnosticLogSnapshot {
        entries: snapshot
            .records
            .into_iter()
            .map(|record| FrontendDiagnosticLogEntry {
                timestamp_ms: record.timestamp_ms,
                source: record.event.source,
                message: koushi_diagnostics::format_event(&record.event),
            })
            .collect(),
        dropped_entries: snapshot.dropped_records,
    }
}

#[tauri::command]
pub fn get_diagnostic_snapshot() -> FrontendDiagnosticLogSnapshot {
    map_snapshot(koushi_diagnostics::snapshot())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_snapshot_maps_structured_snapshot_to_camel_case_frontend_contract() {
        let snapshot = koushi_diagnostics::DiagnosticSnapshot {
            records: vec![koushi_diagnostics::DiagnosticRecord {
                timestamp_ms: 42,
                event: koushi_diagnostics::DiagnosticEvent::new(
                    koushi_diagnostics::DiagnosticLevel::Debug,
                    "desktop.timeline",
                    "submit",
                )
                .field(koushi_diagnostics::DiagnosticField::token(
                    "operation",
                    "send_reaction",
                )),
            }],
            dropped_records: 7,
        };

        let json = serde_json::to_value(map_snapshot(snapshot)).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "entries": [{
                    "timestampMs": 42,
                    "source": "desktop.timeline",
                    "message": "stage=submit operation=send_reaction"
                }],
                "droppedEntries": 7
            })
        );
    }

    #[test]
    fn diagnostic_snapshot_command_is_registered_in_generate_handler() {
        let source = include_str!("../lib.rs");
        assert!(source.contains("commands::diagnostics::get_diagnostic_snapshot"));
    }

    #[test]
    fn diagnostic_snapshot_serialization_excludes_synthetic_private_values() {
        let snapshot = koushi_diagnostics::DiagnosticSnapshot {
            records: vec![koushi_diagnostics::DiagnosticRecord {
                timestamp_ms: 42,
                event: koushi_diagnostics::DiagnosticEvent::new(
                    koushi_diagnostics::DiagnosticLevel::Debug,
                    "desktop.search",
                    "submit",
                )
                .field(koushi_diagnostics::DiagnosticField::count(
                    "query_bytes",
                    23,
                ))
                .field(koushi_diagnostics::DiagnosticField::count(
                    "query_chars",
                    17,
                )),
            }],
            dropped_records: 0,
        };
        let serialized = serde_json::to_string(&map_snapshot(snapshot)).unwrap();
        for forbidden in [
            "!room:synthetic.invalid",
            "@user:synthetic.invalid",
            "$event:synthetic.invalid",
            "/Users/alice/private",
            "secret message",
            "synthetic search query",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "serialized diagnostics leaked {forbidden}"
            );
        }
        assert!(serialized.contains("query_bytes"));
        assert!(serialized.contains("query_chars"));
    }
}
