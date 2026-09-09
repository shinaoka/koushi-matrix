use super::*;
use koushi_protocol::view::{ReceiptCompactSummary, ReceiptSourceRef};

#[test]
fn installed_avatar_targets_are_room_qualified_and_cannot_impersonate_own_profile() {
    let source: ReceiptSourceRef = serde_json::from_value(serde_json::json!({
        "key": {"account_key": "account", "kind": {"Room": {"room_id": "!r:example.org"}}},
        "projection_request_id": {"connection_id": "1", "sequence": "2"},
        "generation": "3", "event_id": "$event"
    }))
    .unwrap();
    let model = ViewModel::TimelineReceipts {
        source: source.timeline.clone(),
        summaries: vec![ReceiptCompactSummary {
            source,
            total_count: 1,
            overflow_count: 0,
            readers: vec![ReaderRow {
                user_id: "@reader:example.org".into(),
                display_label: "Reader".into(),
                original_display_label: "Reader".into(),
                initials: "R".into(),
                timestamp: None,
                avatar: Some(koushi_state::AvatarThumbnailState::NotRequested),
            }],
        }],
    };
    assert!(!serde_json::to_string(&model).unwrap().contains("mxc://"));
    let prepared = prepare(
        model,
        &ViewBudget::default(),
        vec![crate::timeline::ReaderAvatarResource {
            user_id: "@reader:example.org".into(),
            mxc_uri: "mxc://example.org/private-avatar".into(),
            lease: Err(crate::renderable_thumbnail::ThumbnailLeaseError::Unavailable),
        }],
    )
    .unwrap();
    let target = |room: &str| koushi_state::AvatarTarget::User {
        room_id: room.into(),
        user_id: "@reader:example.org".into(),
    };
    assert_eq!(
        prepared
            .installed
            .avatar_target_mxc(&target("!r:example.org")),
        Ok(Some("mxc://example.org/private-avatar"))
    );
    assert_eq!(
        prepared
            .installed
            .avatar_target_mxc(&target("!other:example.org")),
        Err(ScopeError::InvalidModel)
    );
    assert_eq!(
        prepared
            .installed
            .avatar_target_mxc(&koushi_state::AvatarTarget::OwnProfile),
        Err(ScopeError::InvalidModel)
    );
}
