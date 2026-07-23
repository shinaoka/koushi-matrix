use koushi_state::{ComposerDraftRevision, ComposerDraftRevisionError};

const ABOVE_JAVASCRIPT_SAFE_INTEGER: &str = "9007199254740993";
const U128_MAX_WIRE: &str = "340282366920938463463374607431768211455";

fn revision(wire: &str) -> ComposerDraftRevision {
    ComposerDraftRevision::parse_wire(wire).expect("synthetic revision must be valid")
}

#[test]
fn composer_revision_round_trips_canonical_strings_exactly() {
    for wire in ["0", ABOVE_JAVASCRIPT_SAFE_INTEGER, U128_MAX_WIRE] {
        let parsed = revision(wire);
        let serialized = serde_json::to_string(&parsed).expect("serialize revision");
        let deserialized = serde_json::from_str::<ComposerDraftRevision>(&serialized)
            .expect("deserialize revision");

        assert_eq!(parsed.to_wire_string(), wire);
        assert_eq!(serialized, format!("\"{wire}\""));
        assert_eq!(deserialized, parsed);
    }
}

#[test]
fn composer_revision_rejects_noncanonical_grammar_and_out_of_range_strings() {
    for invalid in [
        "",
        "00",
        "01",
        "-1",
        "+1",
        " 1",
        "1 ",
        "1.0",
        "1e3",
        "not-a-revision",
        "1000000000000000000000000000000000000000",
        "340282366920938463463374607431768211456",
    ] {
        assert_eq!(
            ComposerDraftRevision::parse_wire(invalid),
            Err(ComposerDraftRevisionError::InvalidWire),
            "{invalid:?} must be rejected by the direct parser"
        );
        assert!(
            serde_json::from_str::<ComposerDraftRevision>(&format!("\"{invalid}\"")).is_err(),
            "{invalid:?} must be rejected by serde"
        );
    }
}

#[test]
fn composer_revision_rejects_current_schema_numeric_json_tokens() {
    for numeric_json in ["0", ABOVE_JAVASCRIPT_SAFE_INTEGER, U128_MAX_WIRE] {
        assert!(
            serde_json::from_str::<ComposerDraftRevision>(numeric_json).is_err(),
            "numeric JSON token {numeric_json} must not enter the string-only wire contract"
        );
    }
}

#[test]
fn composer_revision_checked_successor_uses_the_greater_input_exactly() {
    let zero_next = ComposerDraftRevision::checked_successor(
        ComposerDraftRevision::ZERO,
        ComposerDraftRevision::ZERO,
    )
    .expect("zero must advance");
    assert_eq!(zero_next.to_wire_string(), "1");

    let above_safe = revision(ABOVE_JAVASCRIPT_SAFE_INTEGER);
    let above_safe_next =
        ComposerDraftRevision::checked_successor(above_safe, above_safe).expect("must advance");
    assert_eq!(above_safe_next.to_wire_string(), "9007199254740994");

    let five = revision("5");
    let seven = revision("7");
    assert_eq!(
        ComposerDraftRevision::checked_successor(five, seven)
            .expect("greater submitted revision must advance")
            .to_wire_string(),
        "8"
    );
    assert_eq!(
        ComposerDraftRevision::checked_successor(seven, five)
            .expect("greater authoritative revision must advance")
            .to_wire_string(),
        "8"
    );
}

#[test]
fn composer_revision_exhaustion_is_checked_without_wrap_or_saturation() {
    let maximum = revision(U128_MAX_WIRE);
    assert_eq!(maximum, ComposerDraftRevision::MAX);
    assert_eq!(
        ComposerDraftRevision::checked_successor(maximum, ComposerDraftRevision::ZERO),
        Err(ComposerDraftRevisionError::Exhausted)
    );
    assert_eq!(
        ComposerDraftRevision::checked_successor(ComposerDraftRevision::ZERO, maximum),
        Err(ComposerDraftRevisionError::Exhausted)
    );
    assert_eq!(
        ComposerDraftRevision::checked_successor(maximum, maximum),
        Err(ComposerDraftRevisionError::Exhausted)
    );
}

#[test]
fn composer_revision_debug_is_redacted() {
    let revision = revision(ABOVE_JAVASCRIPT_SAFE_INTEGER);
    let rendered = format!("{revision:?}");

    assert_eq!(rendered, "ComposerDraftRevision(REDACTED)");
    assert!(!rendered.contains(ABOVE_JAVASCRIPT_SAFE_INTEGER));
}
