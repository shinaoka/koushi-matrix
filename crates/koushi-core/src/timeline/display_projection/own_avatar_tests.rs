//! #2059: the session user's own rows fall back to the account avatar when the
//! SDK reported no sender profile for them.
//!
//! The SDK's profile for the session user can be unavailable while the account
//! avatar is known, and the renderer's third `resolvedAvatar` argument only reuses
//! a thumbnail for a URI the first two already named. So Core's publish boundary
//! fills the gap — for the session user only, and only when nothing else set it.

use koushi_protocol::ids::{AccountKey, TimelineKey, TimelineKind};

use super::super::test_support::{room_key, thread_key, timeline_item};
use super::*;

const OWN_USER: &str = "@own:test";
const OWN_AVATAR: &str = "mxc://example.org/own";
const ROOM_AVATAR: &str = "mxc://example.org/room-specific";
const OTHER_USER: &str = "@other:test";
const OTHER_AVATAR: &str = "mxc://example.org/other";

fn avatar(mxc_uri: &str) -> AvatarImage {
    AvatarImage {
        mxc_uri: mxc_uri.to_owned(),
        thumbnail: AvatarThumbnailState::NotRequested,
    }
}

fn row(event_id: &str, sender: &str, sender_avatar: Option<AvatarImage>) -> TimelineItem {
    let mut item = timeline_item(event_id, Some("body"), sender, false);
    item.sender_avatar = sender_avatar;
    item
}

fn focused_key() -> TimelineKey {
    TimelineKey {
        account_key: AccountKey(OWN_USER.to_owned()),
        kind: TimelineKind::Focused {
            room_id: room_key().room_id().to_owned(),
            event_id: "$focus:test".to_owned(),
        },
    }
}

fn project(
    key: &TimelineKey,
    items: Vec<TimelineItem>,
    own_avatar: Option<&str>,
) -> Vec<TimelineItem> {
    let mut state = DisplayProjectionState::from_canonical_window(&items, 0..items.len());
    let context = DisplayProjectionContext::for_timeline(
        &key.kind,
        &TimelineViewportObservation {
            at_bottom: true,
            ..Default::default()
        },
    )
    .with_own_avatar(Some(OWN_USER), own_avatar);
    state.reproject(&context);
    state.display_items().to_vec()
}

fn sender_avatar_of<'a>(items: &'a [TimelineItem], event_id: &str) -> Option<&'a AvatarImage> {
    items
        .iter()
        .find(|item| {
            item.id
                == TimelineItemId::Event {
                    event_id: event_id.to_owned(),
                }
        })
        .and_then(|item| item.sender_avatar.as_ref())
}

/// The reported shape: the account avatar is known, the SDK reported no profile
/// for the own message, so the row rendered initials.
#[test]
fn an_own_row_without_an_sdk_avatar_renders_the_account_avatar() {
    for key in [room_key(), focused_key(), thread_key()] {
        let items = project(
            &key,
            vec![row("$own:test", OWN_USER, None)],
            Some(OWN_AVATAR),
        );
        assert_eq!(
            sender_avatar_of(&items, "$own:test"),
            Some(&avatar(OWN_AVATAR)),
            "the own row must show the account avatar in {:?}",
            key.kind
        );
    }
}

/// A room-specific avatar the SDK did resolve stays authoritative: the account
/// avatar only fills an empty slot.
#[test]
fn a_known_room_specific_avatar_is_not_replaced() {
    let items = project(
        &room_key(),
        vec![row("$own:test", OWN_USER, Some(avatar(ROOM_AVATAR)))],
        Some(OWN_AVATAR),
    );
    assert_eq!(
        sender_avatar_of(&items, "$own:test"),
        Some(&avatar(ROOM_AVATAR))
    );
}

/// Another sender never receives the session user's avatar, even with no profile.
#[test]
fn another_sender_never_takes_the_account_avatar() {
    let items = project(
        &room_key(),
        vec![
            row("$other:test", OTHER_USER, None),
            row("$other-silent:test", OTHER_USER, Some(avatar(OTHER_AVATAR))),
        ],
        Some(OWN_AVATAR),
    );
    assert_eq!(sender_avatar_of(&items, "$other:test"), None);
    assert_eq!(
        sender_avatar_of(&items, "$other-silent:test"),
        Some(&avatar(OTHER_AVATAR))
    );
}

/// Nothing is invented when the account avatar is not known yet.
#[test]
fn no_account_avatar_leaves_the_row_without_one() {
    let items = project(&room_key(), vec![row("$own:test", OWN_USER, None)], None);
    assert_eq!(sender_avatar_of(&items, "$own:test"), None);
}

/// Late hydration: the row is already published, and the avatar arriving later
/// updates it in place rather than requiring a reopen.
#[test]
fn a_late_account_avatar_updates_an_already_published_row() {
    let items = vec![row("$own:test", OWN_USER, None)];
    let mut state = DisplayProjectionState::from_canonical_window(&items, 0..items.len());
    let before = project(&room_key(), items, None);
    assert_eq!(sender_avatar_of(&before, "$own:test"), None);

    let context = DisplayProjectionContext::for_timeline(&room_key().kind, &Default::default())
        .with_own_avatar(Some(OWN_USER), Some(OWN_AVATAR));
    let diffs = state.reproject(&context);

    assert!(
        diffs
            .iter()
            .any(|diff| matches!(diff, TimelineDiff::Set { index: 0, .. })),
        "the visible row is republished: {diffs:?}"
    );
    assert_eq!(
        sender_avatar_of(state.display_items(), "$own:test"),
        Some(&avatar(OWN_AVATAR))
    );
}
