use koushi_state::{
    ComposerDraftPersistenceProjection, ComposerDraftProtection, ComposerDraftRevision,
    ComposerDraftRevisionError, ComposerDraftStore, ComposerState, ComposerTarget,
    MAX_LIVE_COMPOSER_ROOM_TOMBSTONES, MAX_LIVE_COMPOSER_THREAD_TOMBSTONES,
};

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

#[test]
fn composer_state_serializes_revision_and_clear_token_as_strings() {
    let composer = ComposerState {
        draft_revision: ComposerDraftRevision::parse_wire("9007199254740993")
            .expect("valid revision"),
        last_accepted_clear_revision: ComposerDraftRevision::from_u64(7),
        ..ComposerState::default()
    };
    let value = serde_json::to_value(composer).expect("serialize composer");
    assert_eq!(value["draft_revision"], "9007199254740993");
    assert_eq!(value["last_accepted_clear_revision"], "7");
}

#[test]
fn exhausted_acceptance_does_not_mutate_the_draft() {
    let mut drafts = ComposerDraftStore::default();
    assert!(
        drafts
            .apply_room_draft(
                "room-a".to_owned(),
                "keep me".to_owned(),
                ComposerDraftRevision::MAX,
            )
            .expect("initial apply")
    );
    assert_eq!(
        drafts.advance_room_revision("room-a", ComposerDraftRevision::MAX),
        Err(ComposerDraftRevisionError::Exhausted)
    );
    assert_eq!(
        drafts.rooms.get("room-a").map(String::as_str),
        Some("keep me")
    );
}

#[test]
fn room_tombstones_evict_oldest_quiescent_not_lexical_first() {
    let mut drafts = ComposerDraftStore::default();
    assert!(
        drafts
            .apply_room_draft("z-oldest".to_owned(), String::new(), 1.into())
            .expect("oldest tombstone")
    );
    assert!(
        drafts
            .apply_room_draft("a-touched-middle".to_owned(), String::new(), 1.into())
            .expect("middle tombstone")
    );
    for index in 0..(MAX_LIVE_COMPOSER_ROOM_TOMBSTONES - 2) {
        assert!(
            drafts
                .apply_room_draft(format!("m-{index:03}"), String::new(), 1.into())
                .expect("fixture tombstone")
        );
    }
    let touched = ComposerTarget::Main {
        room_id: "a-touched-middle".to_owned(),
    };
    drafts.reconcile_lifecycle(&ComposerDraftProtection {
        active: [touched.clone()].into_iter().collect(),
        leased: Default::default(),
    });
    drafts.reconcile_lifecycle(&ComposerDraftProtection::default());
    assert!(
        drafts
            .apply_room_draft("b-newest".to_owned(), String::new(), 1.into())
            .expect("newest tombstone")
    );
    drafts.reconcile_lifecycle(&ComposerDraftProtection::default());

    assert_eq!(
        drafts.quiescent_room_tombstone_count(),
        MAX_LIVE_COMPOSER_ROOM_TOMBSTONES
    );
    assert!(drafts.room_revision("z-oldest").is_zero());
    assert!(!drafts.room_revision("a-touched-middle").is_zero());
    assert!(!drafts.room_revision("b-newest").is_zero());
}

#[test]
fn thread_tombstones_are_bounded_and_root_isolated() {
    let mut drafts = ComposerDraftStore::default();
    assert!(
        drafts
            .apply_thread_draft(
                "room".to_owned(),
                "z-root/z-oldest".to_owned(),
                String::new(),
                1.into(),
            )
            .expect("oldest thread tombstone")
    );
    for index in 0..(MAX_LIVE_COMPOSER_THREAD_TOMBSTONES - 1) {
        let root = if index % 2 == 0 { "z-root" } else { "a-root" };
        assert!(
            drafts
                .apply_thread_draft(
                    "room".to_owned(),
                    format!("{root}/m-{index:03}"),
                    String::new(),
                    1.into(),
                )
                .expect("fixture thread tombstone")
        );
    }
    assert!(
        drafts
            .apply_thread_draft(
                "room".to_owned(),
                "a-root/a-newest".to_owned(),
                String::new(),
                1.into(),
            )
            .expect("newest thread tombstone")
    );
    drafts.reconcile_lifecycle(&ComposerDraftProtection::default());

    assert_eq!(
        drafts.quiescent_thread_tombstone_count(),
        MAX_LIVE_COMPOSER_THREAD_TOMBSTONES
    );
    assert!(drafts.thread_revision("room", "z-root/z-oldest").is_zero());
    assert!(!drafts.thread_revision("room", "a-root/a-newest").is_zero());
    assert!(
        drafts
            .thread_revisions
            .get("room")
            .is_some_and(|revisions| revisions.keys().any(|root| root.starts_with("z-root/")))
    );
}

#[test]
fn content_active_and_leased_targets_survive_tombstone_churn() {
    let mut drafts = ComposerDraftStore::default();
    assert!(
        drafts
            .apply_room_draft("protected-content".to_owned(), "draft".to_owned(), 1.into(),)
            .expect("content target")
    );
    for room_id in ["protected-active", "protected-leased"] {
        assert!(
            drafts
                .apply_room_draft(room_id.to_owned(), String::new(), 1.into())
                .expect("protected tombstone")
        );
    }
    for index in 0..=MAX_LIVE_COMPOSER_ROOM_TOMBSTONES {
        assert!(
            drafts
                .apply_room_draft(format!("churn-{index:03}"), String::new(), 1.into())
                .expect("churn tombstone")
        );
    }
    let protection = ComposerDraftProtection {
        active: [ComposerTarget::Main {
            room_id: "protected-active".to_owned(),
        }]
        .into_iter()
        .collect(),
        leased: [ComposerTarget::Main {
            room_id: "protected-leased".to_owned(),
        }]
        .into_iter()
        .collect(),
    };
    drafts.reconcile_lifecycle(&protection);

    assert_eq!(
        drafts.rooms.get("protected-content").map(String::as_str),
        Some("draft")
    );
    assert!(!drafts.room_revision("protected-active").is_zero());
    assert!(!drafts.room_revision("protected-leased").is_zero());
}

#[test]
fn released_empty_targets_become_collectible() {
    let mut drafts = ComposerDraftStore::default();
    for room_id in ["protected-active", "protected-leased"] {
        assert!(
            drafts
                .apply_room_draft(room_id.to_owned(), String::new(), 1.into())
                .expect("protected tombstone")
        );
    }
    for index in 0..=MAX_LIVE_COMPOSER_ROOM_TOMBSTONES {
        assert!(
            drafts
                .apply_room_draft(format!("released-{index:03}"), String::new(), 1.into())
                .expect("released fixture")
        );
    }
    let protection = ComposerDraftProtection {
        active: [ComposerTarget::Main {
            room_id: "protected-active".to_owned(),
        }]
        .into_iter()
        .collect(),
        leased: [ComposerTarget::Main {
            room_id: "protected-leased".to_owned(),
        }]
        .into_iter()
        .collect(),
    };
    drafts.reconcile_lifecycle(&protection);
    drafts.reconcile_lifecycle(&ComposerDraftProtection::default());

    assert_eq!(
        drafts.quiescent_room_tombstone_count(),
        MAX_LIVE_COMPOSER_ROOM_TOMBSTONES
    );
}

#[test]
fn persisted_projection_keeps_all_content_and_protected_targets_with_newest_lru_order() {
    let mut drafts = ComposerDraftStore::default();
    for index in 0..(MAX_LIVE_COMPOSER_ROOM_TOMBSTONES + 2) {
        assert!(
            drafts
                .apply_room_draft(format!("quiescent-{index:03}"), String::new(), 1.into())
                .expect("quiescent tombstone")
        );
    }
    for index in 0..(MAX_LIVE_COMPOSER_ROOM_TOMBSTONES + 2) {
        assert!(
            drafts
                .apply_room_draft(
                    format!("content-{index:03}"),
                    format!("draft-{index:03}"),
                    1.into(),
                )
                .expect("content draft")
        );
    }
    assert!(
        drafts
            .apply_room_draft("protected-empty".to_owned(), String::new(), 1.into())
            .expect("protected tombstone")
    );
    let protected = ComposerTarget::Main {
        room_id: "protected-empty".to_owned(),
    };
    let projection = drafts.persisted_projection(&ComposerDraftProtection {
        active: [protected].into_iter().collect(),
        leased: Default::default(),
    });

    assert_eq!(
        projection
            .rooms
            .values()
            .filter(|entry| entry.content.is_some())
            .count(),
        MAX_LIVE_COMPOSER_ROOM_TOMBSTONES + 2
    );
    assert_eq!(
        projection.quiescent_room_order.len(),
        MAX_LIVE_COMPOSER_ROOM_TOMBSTONES
    );
    assert_eq!(
        projection.quiescent_room_order.first().map(String::as_str),
        Some("quiescent-002")
    );
    assert_eq!(
        projection.quiescent_room_order.last().map(String::as_str),
        Some("quiescent-129")
    );
    assert_eq!(
        projection.protected_empty_rooms,
        vec!["protected-empty".to_owned()]
    );
    assert!(
        projection
            .rooms
            .get("protected-empty")
            .is_some_and(|entry| entry.content.is_none())
    );
}

#[test]
fn persisted_import_appends_formerly_protected_group_after_saved_lru_order() {
    let mut drafts = ComposerDraftStore::default();
    assert!(
        drafts
            .apply_room_draft("z-oldest".to_owned(), String::new(), 1.into())
            .expect("oldest tombstone")
    );
    assert!(
        drafts
            .apply_room_draft("a-newer".to_owned(), String::new(), 1.into())
            .expect("nonlexical newer tombstone")
    );
    for index in 0..(MAX_LIVE_COMPOSER_ROOM_TOMBSTONES - 2) {
        assert!(
            drafts
                .apply_room_draft(format!("middle-{index:03}"), String::new(), 1.into())
                .expect("middle tombstone")
        );
    }
    assert!(
        drafts
            .apply_room_draft("protected-empty".to_owned(), String::new(), 1.into())
            .expect("protected tombstone")
    );
    let projection = drafts.persisted_projection(&ComposerDraftProtection {
        active: [ComposerTarget::Main {
            room_id: "protected-empty".to_owned(),
        }]
        .into_iter()
        .collect(),
        leased: Default::default(),
    });

    let restarted =
        ComposerDraftStore::from_persisted_projection(ComposerDraftPersistenceProjection {
            ..projection
        })
        .expect("valid persisted projection");

    assert!(
        !restarted.room_revisions.contains_key("z-oldest"),
        "the saved oldest target must be evicted"
    );
    assert!(restarted.room_revisions.contains_key("a-newer"));
    assert!(restarted.room_revisions.contains_key("protected-empty"));
    let restarted_projection = restarted.persisted_projection(&ComposerDraftProtection::default());
    assert_eq!(
        restarted_projection
            .quiescent_room_order
            .first()
            .map(String::as_str),
        Some("a-newer")
    );
    assert_eq!(
        restarted_projection
            .quiescent_room_order
            .last()
            .map(String::as_str),
        Some("protected-empty")
    );
}

#[test]
fn persisted_projection_classifies_empty_room_strings_as_quiescent() {
    let mut drafts = ComposerDraftStore::default();
    for index in 0..(MAX_LIVE_COMPOSER_ROOM_TOMBSTONES + 2) {
        let room_id = format!("empty-room-{index:03}");
        drafts.rooms.insert(room_id.clone(), String::new());
        drafts.room_revisions.insert(room_id, 1.into());
    }

    let projection = drafts.persisted_projection(&ComposerDraftProtection::default());
    assert_eq!(
        projection.quiescent_room_order.len(),
        MAX_LIVE_COMPOSER_ROOM_TOMBSTONES
    );
    assert!(
        projection
            .rooms
            .get("empty-room-002")
            .is_some_and(|entry| entry.content.is_none())
    );
    assert!(!projection.rooms.contains_key("empty-room-000"));
    assert!(!projection.rooms.contains_key("empty-room-001"));
    ComposerDraftStore::from_persisted_projection(projection)
        .expect("bounded empty room strings must produce a valid tombstone projection");
}

#[test]
fn accepted_clear_token_changes_only_when_current_content_clears() {
    let mut drafts = ComposerDraftStore::default();
    assert!(
        drafts
            .apply_room_draft("room".to_owned(), "first".to_owned(), 7.into())
            .expect("first draft")
    );
    assert_eq!(
        drafts
            .advance_room_revision("room", 7.into())
            .expect("current acceptance"),
        8.into()
    );
    assert_eq!(
        drafts.room_last_accepted_clear_revisions.get("room"),
        Some(&ComposerDraftRevision::from_u64(8))
    );
    assert!(
        drafts
            .apply_room_draft("room".to_owned(), "newer".to_owned(), 10.into())
            .expect("newer draft")
    );
    assert_eq!(
        drafts
            .advance_room_revision("room", 8.into())
            .expect("stale acceptance"),
        11.into()
    );
    assert_eq!(drafts.rooms.get("room").map(String::as_str), Some("newer"));
    assert_eq!(
        drafts.room_last_accepted_clear_revisions.get("room"),
        Some(&ComposerDraftRevision::from_u64(8))
    );
}
