//! #2041: consecutive notices share one display row plus a count.
//!
//! Everything here is synthetic. The projection is the real one, so the rule is
//! pinned through the same path a live room timeline takes: canonical items
//! into [`DisplayProjectionState`], display items out.

use koushi_protocol::event::{
    TimelineItem, TimelineItemId, TimelineMessageKind, TimelineNoticeI18n, TimelineNoticeI18nKey,
    TimelineViewportObservation,
};

use super::super::test_support::{focused_key, room_key, thread_key, timeline_item};
use super::*;

const SENDER: &str = "@member:example.invalid";

fn notice(event_id: &str, key: TimelineNoticeI18nKey, at_ms: u64) -> TimelineItem {
    notice_from(event_id, SENDER, key, at_ms)
}

fn notice_from(
    event_id: &str,
    sender: &str,
    key: TimelineNoticeI18nKey,
    at_ms: u64,
) -> TimelineItem {
    let mut item = timeline_item(event_id, Some("sentence"), sender, false);
    item.message_kind = TimelineMessageKind::Notice;
    item.notice_i18n = Some(TimelineNoticeI18n {
        key,
        value: None,
        old_name: None,
        new_name: None,
        replacement_room_id: None,
    });
    item.timestamp_ms = Some(at_ms);
    item
}

fn read_marker() -> TimelineItem {
    timeline_item("synthetic", None, SENDER, true).tap_synthetic("read-marker")
}

fn date_divider(at_ms: u64) -> TimelineItem {
    let mut item = timeline_item("synthetic", None, SENDER, true)
        .tap_synthetic(&format!("date-divider-{at_ms}"));
    item.is_hidden = false;
    item.timestamp_ms = Some(at_ms);
    item
}

trait TapSynthetic {
    fn tap_synthetic(self, synthetic_id: &str) -> Self;
}

impl TapSynthetic for TimelineItem {
    fn tap_synthetic(mut self, synthetic_id: &str) -> Self {
        self.id = TimelineItemId::Synthetic {
            synthetic_id: synthetic_id.to_owned(),
        };
        self
    }
}

/// Project `canonical` the way a room timeline does and return its rows.
fn room_display_items(canonical: &[TimelineItem]) -> Vec<TimelineItem> {
    let observation = TimelineViewportObservation::default();
    let context = DisplayProjectionContext::for_timeline(&room_key().kind, &observation);
    let mut state = DisplayProjectionState::from_canonical_window(canonical, 0..canonical.len());
    state.reproject(&context);
    state.display_items().to_vec()
}

fn group_of<'a>(items: &'a [TimelineItem], event_id: &str) -> (u32, u32, Option<&'a str>) {
    let item = items
        .iter()
        .find(|item| matches!(&item.id, TimelineItemId::Event { event_id: id } if id == event_id))
        .unwrap_or_else(|| panic!("{event_id} must be a display row"));
    let metadata = item
        .display_metadata
        .as_ref()
        .expect("every display row carries metadata");
    (
        metadata.group_count,
        metadata.group_position,
        metadata.group_id.as_deref(),
    )
}

#[test]
fn four_consecutive_notices_are_one_group() {
    let canonical = vec![
        notice(
            "$a:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            1_000,
        ),
        notice(
            "$b:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            2_000,
        ),
        notice(
            "$c:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            3_000,
        ),
        notice(
            "$d:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            4_000,
        ),
    ];

    let items = room_display_items(&canonical);

    let head = group_of(&items, "$a:example.invalid");
    assert_eq!(head.0, 4, "the row stands for four notices");
    assert_eq!(head.1, 0, "the first notice is the row that is drawn");
    let group_id = head.2.expect("a grouped row has an expansion key");
    assert_eq!(group_id, "notice-group:$a:example.invalid");
    for (position, event_id) in ["$b", "$c", "$d"].into_iter().enumerate() {
        assert_eq!(
            group_of(&items, &format!("{event_id}:example.invalid")),
            (4, position as u32 + 1, Some(group_id)),
            "{event_id} belongs to the same group"
        );
    }
    assert_eq!(items.len(), 4, "grouping never drops a row");
}

#[test]
fn a_different_notice_kind_splits_the_run() {
    let canonical = vec![
        notice(
            "$a:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            1_000,
        ),
        notice(
            "$b:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            2_000,
        ),
        notice(
            "$pin:example.invalid",
            TimelineNoticeI18nKey::RoomPinnedEvents,
            3_000,
        ),
        notice(
            "$c:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            4_000,
        ),
        notice(
            "$d:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            5_000,
        ),
    ];

    let items = room_display_items(&canonical);

    assert_eq!(group_of(&items, "$a:example.invalid").0, 2);
    assert_eq!(group_of(&items, "$b:example.invalid").1, 1);
    assert_eq!(
        group_of(&items, "$pin:example.invalid"),
        (1, 0, None),
        "a pin between profile changes is a row of its own"
    );
    assert_eq!(group_of(&items, "$c:example.invalid").0, 2);
    assert_eq!(group_of(&items, "$d:example.invalid").1, 1);
}

#[test]
fn a_message_between_notices_splits_the_run() {
    let mut message = timeline_item("$message:example.invalid", Some("hello"), SENDER, false);
    message.timestamp_ms = Some(2_000);
    let canonical = vec![
        notice(
            "$a:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            1_000,
        ),
        message,
        notice(
            "$b:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            3_000,
        ),
    ];

    let items = room_display_items(&canonical);

    assert_eq!(group_of(&items, "$a:example.invalid"), (1, 0, None));
    assert_eq!(group_of(&items, "$b:example.invalid"), (1, 0, None));
}

#[test]
fn the_gap_threshold_is_five_minutes() {
    assert_eq!(NOTICE_GROUP_MAX_GAP_MS, 5 * 60 * 1_000);

    let joined = vec![
        notice(
            "$a:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            1_000,
        ),
        notice(
            "$b:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            1_000 + NOTICE_GROUP_MAX_GAP_MS,
        ),
    ];
    let items = room_display_items(&joined);
    assert_eq!(
        group_of(&items, "$a:example.invalid").0,
        2,
        "five minutes apart is still one burst"
    );

    let split = vec![
        notice(
            "$a:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            1_000,
        ),
        notice(
            "$b:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            1_001 + NOTICE_GROUP_MAX_GAP_MS,
        ),
    ];
    let items = room_display_items(&split);
    assert_eq!(group_of(&items, "$a:example.invalid"), (1, 0, None));
    assert_eq!(group_of(&items, "$b:example.invalid"), (1, 0, None));
}

#[test]
fn a_date_divider_splits_the_run() {
    let canonical = vec![
        notice(
            "$a:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            1_000,
        ),
        notice(
            "$b:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            2_000,
        ),
        date_divider(2_500),
        notice(
            "$c:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            3_000,
        ),
        notice(
            "$d:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            4_000,
        ),
    ];

    let items = room_display_items(&canonical);

    assert_eq!(group_of(&items, "$a:example.invalid").0, 2);
    assert_eq!(group_of(&items, "$c:example.invalid").0, 2);
    assert_eq!(
        group_of(&items, "$c:example.invalid").1,
        0,
        "a group never starts before a date divider"
    );
}

#[test]
fn the_read_marker_splits_the_run() {
    let canonical = vec![
        notice(
            "$a:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            1_000,
        ),
        notice(
            "$b:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            2_000,
        ),
        read_marker(),
        notice(
            "$c:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            3_000,
        ),
        notice(
            "$d:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            4_000,
        ),
    ];

    let items = room_display_items(&canonical);

    assert_eq!(group_of(&items, "$a:example.invalid").0, 2);
    assert_eq!(
        group_of(&items, "$c:example.invalid").1,
        0,
        "the unread side starts its own group, so the divider keeps its row"
    );
    assert_eq!(group_of(&items, "$d:example.invalid").0, 2);
}

#[test]
fn different_senders_are_not_one_group() {
    let canonical = vec![
        notice_from(
            "$a:example.invalid",
            "@a:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            1_000,
        ),
        notice_from(
            "$b:example.invalid",
            "@b:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            2_000,
        ),
        notice_from(
            "$c:example.invalid",
            "@a:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            3_000,
        ),
    ];

    let items = room_display_items(&canonical);

    for event_id in ["$a", "$b", "$c"] {
        assert_eq!(
            group_of(&items, &format!("{event_id}:example.invalid")),
            (1, 0, None)
        );
    }
}

#[test]
fn notices_with_per_event_work_are_never_counted() {
    // An undecryptable message keeps its key-request control and a call keeps
    // its own wording; #2041 groups state notices only.
    let canonical = vec![
        notice(
            "$utd1:example.invalid",
            TimelineNoticeI18nKey::UnableToDecrypt,
            1_000,
        ),
        notice(
            "$utd2:example.invalid",
            TimelineNoticeI18nKey::UnableToDecrypt,
            2_000,
        ),
        notice("$call1:example.invalid", TimelineNoticeI18nKey::Call, 3_000),
        notice("$call2:example.invalid", TimelineNoticeI18nKey::Call, 4_000),
    ];

    let items = room_display_items(&canonical);

    for item in &items {
        assert_eq!(
            item.display_metadata
                .as_ref()
                .map(|metadata| metadata.group_count),
            Some(1),
            "a notice with per-event work stays its own row"
        );
    }
}

#[test]
fn membership_lines_are_not_a_group_kind() {
    // A membership line is a notice with no localized kind (`notice_i18n` is
    // None), so #2041 leaves it alone: "Alice joined the room" twice is two
    // events, not one line with a count.
    let joined = |event_id: &str, at_ms: u64| {
        let mut item = timeline_item(event_id, Some("Alice joined the room"), SENDER, false);
        item.message_kind = TimelineMessageKind::Notice;
        item.timestamp_ms = Some(at_ms);
        item
    };
    let canonical = vec![
        joined("$join1:example.invalid", 1_000),
        joined("$join2:example.invalid", 2_000),
    ];

    let items = room_display_items(&canonical);

    assert_eq!(group_of(&items, "$join1:example.invalid"), (1, 0, None));
    assert_eq!(group_of(&items, "$join2:example.invalid"), (1, 0, None));
}

#[test]
fn only_room_timelines_group() {
    let canonical = vec![
        notice(
            "$a:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            1_000,
        ),
        notice(
            "$b:example.invalid",
            TimelineNoticeI18nKey::ProfileAvatarChanged,
            2_000,
        ),
    ];

    for kind in [thread_key().kind, focused_key().kind] {
        let observation = TimelineViewportObservation::default();
        let context = DisplayProjectionContext::for_timeline(&kind, &observation);
        let mut state =
            DisplayProjectionState::from_canonical_window(&canonical, 0..canonical.len());
        state.reproject(&context);
        assert_eq!(
            state
                .display_items()
                .first()
                .and_then(|item| item.display_metadata.as_ref())
                .map(|metadata| metadata.group_count),
            Some(1),
            "a thread or focused timeline keeps one row per notice"
        );
    }
}

#[test]
fn the_window_head_is_the_row_that_is_drawn() {
    // The published window may start in the middle of a burst: the head shown is
    // the oldest row of the window, and the count covers only what is loaded.
    let canonical = (0..4)
        .map(|index| {
            notice(
                &format!("$n{index}:example.invalid"),
                TimelineNoticeI18nKey::ProfileAvatarChanged,
                1_000 + index as u64,
            )
        })
        .collect::<Vec<_>>();
    let observation = TimelineViewportObservation::default();
    let context = DisplayProjectionContext::for_timeline(&room_key().kind, &observation);
    let mut state = DisplayProjectionState::from_canonical_window(&canonical, 2..4);
    state.reproject(&context);

    let items = state.display_items();
    assert_eq!(
        group_of(items, "$n2:example.invalid"),
        (2, 0, Some("notice-group:$n2:example.invalid"))
    );
    assert_eq!(group_of(items, "$n3:example.invalid").1, 1);
}
