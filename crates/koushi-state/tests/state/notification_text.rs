//! Localized notification text rendered in Rust (#1476): the per-locale table,
//! and the OS banner reading the selected locale when it is built.

use koushi_state::{
    AppAction, AppState, CatalogLocale, NativeAttentionCapabilities, NativeAttentionCapability,
    NotificationPlural, NotificationText, RoomLatestEventSummary, RoomSummary, RoomTags,
    SessionInfo, SessionState, notification_counts, notification_plural, notification_text, reduce,
};

const LOCALES: [CatalogLocale; 6] = [
    CatalogLocale::En,
    CatalogLocale::Ja,
    CatalogLocale::ZhHans,
    CatalogLocale::ZhHant,
    CatalogLocale::Ko,
    CatalogLocale::Pseudo,
];

fn every_text() -> Vec<NotificationText<'static>> {
    vec![
        NotificationText::MentionFrom { name: "Member 1" },
        NotificationText::DirectMessageFrom { name: "Member 1" },
        NotificationText::MentionIn {
            place: "Synthetic Room",
        },
        NotificationText::DirectMessageIn {
            place: "Synthetic Room",
        },
        NotificationText::MessageIn {
            place: "Synthetic Room",
        },
        NotificationText::RoomFallback,
        NotificationText::SubjectMoreRooms {
            headline: "Synthetic headline",
            count: 2,
        },
        NotificationText::BodyMoreRooms { count: 7 },
        NotificationText::MentionCount { count: 3 },
        NotificationText::UnreadCount { count: 4 },
        NotificationText::TestDeliverySubject,
        NotificationText::TestDeliveryBody,
    ]
}

#[test]
fn every_locale_has_its_own_text_for_every_entry() {
    for text in every_text() {
        let english = notification_text(CatalogLocale::En, text);
        assert!(!english.trim().is_empty(), "{text:?}");
        assert_eq!(
            notification_text(CatalogLocale::Pseudo, text),
            english,
            "pseudo renders as en"
        );
        for locale in [
            CatalogLocale::Ja,
            CatalogLocale::ZhHans,
            CatalogLocale::ZhHant,
            CatalogLocale::Ko,
        ] {
            let localized = notification_text(locale, text);
            assert!(!localized.trim().is_empty(), "{locale:?} {text:?}");
            assert_ne!(localized, english, "{locale:?} {text:?}");
            assert!(!localized.contains('{'), "{locale:?} {text:?}");
        }
    }
}

#[test]
fn interpolated_values_are_kept_verbatim() {
    for locale in LOCALES {
        let headline =
            notification_text(locale, NotificationText::MentionFrom { name: "Member 1" });
        assert!(headline.contains("Member 1"), "{locale:?}");
        let place = notification_text(
            locale,
            NotificationText::MessageIn {
                place: "Synthetic Room · Synthetic Workspace",
            },
        );
        assert!(place.contains("Synthetic Room · Synthetic Workspace"));
        let counts = notification_text(locale, NotificationText::UnreadCount { count: 12 });
        assert!(counts.contains("12"), "{locale:?}");
    }
}

#[test]
fn english_distinguishes_one_and_other_while_cjk_uses_one_form() {
    assert_eq!(
        notification_plural(CatalogLocale::En, 1),
        NotificationPlural::One
    );
    assert_eq!(
        notification_plural(CatalogLocale::En, 2),
        NotificationPlural::Other
    );
    assert_eq!(
        notification_text(
            CatalogLocale::En,
            NotificationText::MentionCount { count: 1 }
        ),
        "1 mention"
    );
    assert_eq!(
        notification_text(
            CatalogLocale::En,
            NotificationText::MentionCount { count: 2 }
        ),
        "2 mentions"
    );
    assert_eq!(
        notification_text(
            CatalogLocale::En,
            NotificationText::SubjectMoreRooms {
                headline: "H",
                count: 1
            }
        ),
        "H and 1 more room"
    );
    for locale in [
        CatalogLocale::Ja,
        CatalogLocale::ZhHans,
        CatalogLocale::ZhHant,
        CatalogLocale::Ko,
    ] {
        assert_eq!(notification_plural(locale, 1), NotificationPlural::Other);
        let one = notification_text(locale, NotificationText::MentionCount { count: 1 });
        let two = notification_text(locale, NotificationText::MentionCount { count: 2 });
        assert_eq!(one.replace('1', "2"), two, "{locale:?}");
    }
}

#[test]
fn count_lists_use_the_locale_separator_and_omit_zero() {
    assert_eq!(
        notification_counts(CatalogLocale::En, 2, 3),
        "2 mentions, 3 unread"
    );
    assert_eq!(
        notification_counts(CatalogLocale::Ja, 2, 3),
        "メンション 2 件、未読 3 件"
    );
    assert_eq!(notification_counts(CatalogLocale::ZhHans, 0, 1), "1 条未读");
    assert_eq!(notification_counts(CatalogLocale::Ko, 0, 0), "");
}

fn capabilities() -> NativeAttentionCapabilities {
    NativeAttentionCapabilities {
        notifications: NativeAttentionCapability::Available,
        badge: NativeAttentionCapability::Available,
        overlay_icon: NativeAttentionCapability::Available,
        sound: NativeAttentionCapability::Available,
        tray: NativeAttentionCapability::Available,
        activation: NativeAttentionCapability::Available,
    }
}

fn room(unread: u64, event_id: &str) -> RoomSummary {
    RoomSummary {
        display_name_placeholder: None,
        display_label_placeholder: None,
        room_id: "!room:example.invalid".to_owned(),
        display_name: "Synthetic Room".to_owned(),
        display_label: "Synthetic Room".to_owned(),
        original_display_label: "Synthetic Room".to_owned(),
        avatar: None,
        is_dm: false,
        dm_user_ids: Vec::new(),
        tags: RoomTags::default(),
        unread_count: unread,
        notification_count: unread,
        highlight_count: 0,
        thread_unread_count: 0,
        thread_highlight_count: 0,
        marked_unread: false,
        recency_stamp: Some(42),
        conversation_activity: None,
        latest_event: Some(RoomLatestEventSummary {
            event_id: event_id.to_owned(),
            relation_type: None,
            relation_event_id: None,
            thread_root_event_id: None,
            sender_id: Some("@member2:example.invalid".to_owned()),
            sender_label: Some("Member 2".to_owned()),
            sender_avatar: None,
            timestamp_ms: 1,
            preview: Some("preview".to_owned()),
            message_text: Some("decrypted private text".to_owned()),
            is_redacted: false,
        }),
        parent_space_ids: Vec::new(),
        dm_space_ids: Vec::new(),
        is_encrypted: false,
        joined_members: 3,
    }
}

fn signed_in(language_tag: &str) -> AppState {
    let mut state = AppState {
        session: SessionState::Ready(SessionInfo {
            homeserver: "https://example.invalid".to_owned(),
            user_id: "@member1:example.invalid".to_owned(),
            device_id: "DEVICE".to_owned(),
            authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
        }),
        ..AppState::default()
    };
    state.settings.values.locale.language_tag = Some(language_tag.to_owned());
    state.native_attention.summary.capabilities = capabilities();
    state.native_attention_context.window_focused = false;
    state
}

/// The banner raised by one live unread rise: (title, body).
fn banner(state: &mut AppState, from_event: &str, to_event: &str, unread: u64) -> (String, String) {
    reduce(
        state,
        AppAction::RoomListUpdated {
            spaces: vec![],
            rooms: vec![room(unread - 1, from_event)],
        },
    );
    reduce(
        state,
        AppAction::RoomListUpdated {
            spaces: vec![],
            rooms: vec![room(unread, to_event)],
        },
    );
    let payload = state
        .native_attention
        .notification
        .as_ref()
        .expect("a banner");
    (payload.title.clone(), payload.body.clone())
}

#[test]
fn the_os_banner_follows_the_selected_locale() {
    let mut state = signed_in("ja");
    let (title, body) = banner(&mut state, "$e1:example.invalid", "$e2:example.invalid", 1);
    assert_eq!(title, "Synthetic Room のメッセージ");
    assert_eq!(body, "未読 1 件");

    let mut state = signed_in("ko");
    let (title, body) = banner(&mut state, "$e1:example.invalid", "$e2:example.invalid", 2);
    assert_eq!(title, "Synthetic Room의 메시지");
    assert_eq!(body, "읽지 않음 2개");
}

#[test]
fn a_locale_change_applies_to_the_next_banner() {
    let mut state = signed_in("en");
    let (title, _) = banner(&mut state, "$e1:example.invalid", "$e2:example.invalid", 1);
    assert_eq!(title, "Message in Synthetic Room");
    state.settings.values.locale.language_tag = Some("zh-Hant".to_owned());
    let (title, body) = banner(&mut state, "$e3:example.invalid", "$e4:example.invalid", 1);
    assert_eq!(title, "Synthetic Room 中的訊息");
    assert_eq!(body, "1 則未讀");
}

#[test]
fn localization_never_adds_message_text_when_previews_are_off() {
    for tag in ["en", "ja", "zh-Hans", "zh-Hant", "ko"] {
        let mut state = signed_in(tag);
        state.settings.values.notifications.message_previews = false;
        let (title, body) = banner(&mut state, "$e1:example.invalid", "$e2:example.invalid", 1);
        assert!(!title.contains("decrypted"), "{tag}");
        assert!(!body.contains("decrypted"), "{tag}");
    }
}
