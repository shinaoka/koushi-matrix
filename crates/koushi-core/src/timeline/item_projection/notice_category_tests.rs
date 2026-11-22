//! #2041: the classes of state change the "show room and member changes"
//! preference may hide.

use crate::timeline::item_projection::membership_notice_category;
use koushi_protocol::event::TimelineNoticeCategory;
use matrix_sdk_ui::timeline::MembershipChange;

#[test]
fn a_membership_change_is_a_class_only_when_it_changes_membership() {
    // The hideable set is explicit. A join, a leave, an invite or a knock may be
    // hidden; nothing else may, because the row this switch hides is the one a
    // reader has to be able to find.
    for change in [
        MembershipChange::Joined,
        MembershipChange::Left,
        MembershipChange::Invited,
        MembershipChange::InvitationAccepted,
        MembershipChange::InvitationRejected,
        MembershipChange::InvitationRevoked,
        MembershipChange::Knocked,
        MembershipChange::KnockAccepted,
        MembershipChange::KnockRetracted,
        MembershipChange::KnockDenied,
    ] {
        assert_eq!(
            membership_notice_category(Some(change)),
            Some(TimelineNoticeCategory::Membership),
            "{change:?} is a membership change the preference hides"
        );
    }
}

#[test]
fn moderation_and_unclassified_changes_stay_visible() {
    // A ban, an unban or a kick is a moderation action: hiding one makes the
    // member's absence look unexplained, and hiding an unban makes the ban look
    // permanent. `None` means no membership change happened at all (an
    // `m.room.member` event that only touched a power level or a reason), so
    // hiding it would silence e.g. someone being made a moderator. `Error` and
    // `NotImplemented` are rows this build does not understand, and a row we
    // cannot classify must never be the one we silently drop.
    for change in [
        MembershipChange::Banned,
        MembershipChange::Unbanned,
        MembershipChange::Kicked,
        MembershipChange::KickedAndBanned,
        MembershipChange::None,
        MembershipChange::Error,
        MembershipChange::NotImplemented,
    ] {
        assert_eq!(
            membership_notice_category(Some(change)),
            None,
            "{change:?} must stay visible"
        );
    }
    assert_eq!(membership_notice_category(None), None);
}
