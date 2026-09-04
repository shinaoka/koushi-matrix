use koushi_search::{SearchCandidate, SearchDocumentStore, SearchableEvent, SensitiveString};
use koushi_state::{SearchMatchField, TextRange, normalize_cjk_search_text};

fn body_result(body: &str, query: &str) -> Option<koushi_state::SearchResult> {
    let mut store = SearchDocumentStore::default();
    store.upsert_message(SearchableEvent {
        room_id: "!room:example.invalid".into(),
        event_id: "$event".into(),
        sender: "@sender:example.invalid".into(),
        timestamp_ms: 1_700_000_000_000,
        body: Some(SensitiveString::new(body)),
        attachment_filename: None,
        attachment: None,
    });
    store.verify_candidate(
        SearchCandidate {
            room_id: "!room:example.invalid".into(),
            event_id: "$event".into(),
            score_millis: 1,
        },
        query,
    )
}

#[test]
fn common_dash_and_minus_variants_match_ascii_hyphen() {
    for dash in ['‐', '‑', '‒', '–', '—', '―', '−', '﹘', '－'] {
        let result = body_result(&format!("before{dash}after"), "before-after")
            .expect("dash variant should match ASCII hyphen");
        assert_eq!(result.match_field, SearchMatchField::MessageBody);
        assert_eq!(
            result.highlights,
            vec![TextRange {
                start_utf16: 0,
                end_utf16: 12
            }]
        );
    }
}

#[test]
fn casefold_expansions_map_back_to_the_exact_utf16_source_range() {
    let result = body_result("Straße", "STRASSE").expect("full casefold should match");
    assert_eq!(
        result.highlights,
        vec![TextRange {
            start_utf16: 0,
            end_utf16: 6
        }]
    );
}

#[test]
fn the_katakana_long_vowel_mark_remains_distinct() {
    assert_eq!(normalize_cjk_search_text("ー"), "ー");
    assert!(body_result("カタカナ", "カタカナー").is_none());
}
