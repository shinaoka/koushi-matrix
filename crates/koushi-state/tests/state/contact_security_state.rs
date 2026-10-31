//! Reducer contract for contact security details in User info (#1024).
//!
//! The device/identity classification itself is the SDK adapter's
//! (`koushi-sdk` contact_security tests drive it with real signed keys); these
//! tests pin the reducer fences: checking, unavailable, stale responses after
//! switching contacts or accounts, and store-driven refreshes.

use koushi_state::{
    AppAction, AppState, ContactDeviceCounts, ContactDeviceSignature, ContactDevicesStatus,
    ContactIdentityVerification, ContactSecurityFailureKind, ContactSecurityLoadState,
    ContactSecuritySummary, SessionInfo, SessionState, reduce,
};

const ALICE: &str = "@alice:example.test";
const BOB: &str = "@bob:example.test";

fn session_info(user_id: &str) -> SessionInfo {
    SessionInfo {
        homeserver: "https://example.test".to_owned(),
        user_id: user_id.to_owned(),
        device_id: "DEVICE".to_owned(),
        authentication_method: koushi_state::SessionAuthenticationMethod::Unknown,
    }
}

fn ready_state() -> AppState {
    AppState {
        session: SessionState::Ready(session_info("@me:example.test")),
        ..AppState::default()
    }
}

fn summary(
    devices: ContactDevicesStatus,
    signatures: Vec<ContactDeviceSignature>,
    identity: ContactIdentityVerification,
) -> ContactSecuritySummary {
    let owner_signed = signatures
        .iter()
        .filter(|signature| **signature == ContactDeviceSignature::OwnerSigned)
        .count() as u32;
    let invalid = signatures
        .iter()
        .filter(|signature| **signature == ContactDeviceSignature::OwnerSignatureInvalid)
        .count() as u32;
    ContactSecuritySummary {
        devices,
        device_counts: ContactDeviceCounts {
            total: signatures.len() as u32,
            owner_signed,
            not_owner_signed: signatures.len() as u32 - owner_signed,
            owner_signature_invalid: invalid,
            excluded_dehydrated: 0,
        },
        device_signatures: signatures,
        identity,
        verification: koushi_state::ContactVerificationOffer::NotOffered,
    }
}

fn all_signed_not_verified() -> ContactSecuritySummary {
    summary(
        ContactDevicesStatus::AllOwnerSigned,
        vec![ContactDeviceSignature::OwnerSigned; 2],
        ContactIdentityVerification::NotVerifiedByYou,
    )
}

fn all_signed_verified() -> ContactSecuritySummary {
    summary(
        ContactDevicesStatus::AllOwnerSigned,
        vec![ContactDeviceSignature::OwnerSigned; 2],
        ContactIdentityVerification::VerifiedByYou,
    )
}

fn unsigned_device_verified() -> ContactSecuritySummary {
    summary(
        ContactDevicesStatus::SomeNotOwnerSigned,
        vec![
            ContactDeviceSignature::OwnerSigned,
            ContactDeviceSignature::NotOwnerSigned,
        ],
        ContactIdentityVerification::VerifiedByYou,
    )
}

fn request(state: &mut AppState, request_id: u64, user_id: &str) -> usize {
    reduce(
        state,
        AppAction::ContactSecurityLoadRequested {
            request_id,
            user_id: user_id.to_owned(),
        },
    )
    .len()
}

fn load(
    state: &mut AppState,
    request_id: u64,
    user_id: &str,
    summary: ContactSecuritySummary,
) -> usize {
    reduce(
        state,
        AppAction::ContactSecurityLoaded {
            request_id,
            user_id: user_id.to_owned(),
            summary,
        },
    )
    .len()
}

fn refresh(state: &mut AppState, user_id: &str, summary: ContactSecuritySummary) -> usize {
    reduce(
        state,
        AppAction::ContactSecurityRefreshed {
            user_id: user_id.to_owned(),
            summary,
        },
    )
    .len()
}

#[test]
fn opening_user_info_enters_checking_without_a_summary() {
    let mut state = ready_state();
    assert_eq!(request(&mut state, 1, ALICE), 1);
    assert_eq!(state.contact_security.user_id.as_deref(), Some(ALICE));
    assert_eq!(
        state.contact_security.load,
        ContactSecurityLoadState::Loading { request_id: 1 }
    );
    assert!(state.contact_security.summary.is_none());
}

#[test]
fn load_requires_a_ready_session() {
    let mut state = AppState::default();
    assert_eq!(request(&mut state, 1, ALICE), 0);
    assert_eq!(state.contact_security, Default::default());
}

#[test]
fn loaded_summaries_keep_devices_and_your_verification_independent() {
    for expected in [
        all_signed_not_verified(),
        all_signed_verified(),
        unsigned_device_verified(),
    ] {
        let mut state = ready_state();
        request(&mut state, 7, ALICE);
        assert_eq!(load(&mut state, 7, ALICE, expected.clone()), 1);
        assert_eq!(
            state.contact_security.load,
            ContactSecurityLoadState::Loaded { request_id: 7 }
        );
        assert_eq!(state.contact_security.summary.as_ref(), Some(&expected));
    }
}

#[test]
fn retrieval_failure_is_unavailable_and_never_a_confirmation() {
    let mut state = ready_state();
    request(&mut state, 1, ALICE);
    load(&mut state, 1, ALICE, all_signed_verified());
    // A retry clears the previous summary: a stale answer is not kept while
    // checking again.
    request(&mut state, 2, ALICE);
    assert!(state.contact_security.summary.is_none());
    let effects = reduce(
        &mut state,
        AppAction::ContactSecurityLoadFailed {
            request_id: 2,
            user_id: ALICE.to_owned(),
            failure_kind: ContactSecurityFailureKind::Network,
        },
    );
    assert_eq!(effects.len(), 1);
    assert_eq!(
        state.contact_security.load,
        ContactSecurityLoadState::Failed {
            request_id: 2,
            failure_kind: ContactSecurityFailureKind::Network,
        }
    );
    assert!(state.contact_security.summary.is_none());
    // A store refresh after a failed retrieval does not turn it into success.
    assert_eq!(refresh(&mut state, ALICE, all_signed_verified()), 0);
    assert!(state.contact_security.summary.is_none());
}

#[test]
fn stale_response_after_switching_contacts_is_dropped() {
    let mut state = ready_state();
    request(&mut state, 1, ALICE);
    request(&mut state, 2, BOB);
    // Alice's late answer, by request id and by user id.
    assert_eq!(load(&mut state, 1, ALICE, all_signed_verified()), 0);
    assert_eq!(load(&mut state, 2, ALICE, all_signed_verified()), 0);
    assert_eq!(
        reduce(
            &mut state,
            AppAction::ContactSecurityLoadFailed {
                request_id: 1,
                user_id: ALICE.to_owned(),
                failure_kind: ContactSecurityFailureKind::Sdk,
            },
        )
        .len(),
        0
    );
    assert_eq!(refresh(&mut state, ALICE, all_signed_verified()), 0);
    assert_eq!(state.contact_security.user_id.as_deref(), Some(BOB));
    assert_eq!(
        state.contact_security.load,
        ContactSecurityLoadState::Loading { request_id: 2 }
    );
    assert!(state.contact_security.summary.is_none());

    assert_eq!(load(&mut state, 2, BOB, unsigned_device_verified()), 1);
    assert_eq!(
        state.contact_security.summary,
        Some(unsigned_device_verified())
    );
}

#[test]
fn stale_response_after_switching_accounts_is_dropped() {
    let mut state = ready_state();
    request(&mut state, 1, ALICE);
    reduce(
        &mut state,
        AppAction::SwitchAccountRequested {
            info: session_info("@other:example.test"),
        },
    );
    assert_eq!(state.contact_security, Default::default());
    assert_eq!(load(&mut state, 1, ALICE, all_signed_verified()), 0);
    assert_eq!(state.contact_security, Default::default());
}

#[test]
fn store_refresh_tracks_device_additions_and_removals() {
    let mut state = ready_state();
    request(&mut state, 1, ALICE);
    load(&mut state, 1, ALICE, all_signed_verified());

    // The contact adds an unsigned device: your verification is preserved.
    assert_eq!(refresh(&mut state, ALICE, unsigned_device_verified()), 1);
    let refreshed = state.contact_security.summary.clone().expect("summary");
    assert_eq!(refreshed.devices, ContactDevicesStatus::SomeNotOwnerSigned);
    assert_eq!(
        refreshed.identity,
        ContactIdentityVerification::VerifiedByYou
    );
    assert_eq!(
        state.contact_security.load,
        ContactSecurityLoadState::Loaded { request_id: 1 }
    );

    // The unsigned device is removed again.
    assert_eq!(refresh(&mut state, ALICE, all_signed_verified()), 1);
    assert_eq!(state.contact_security.summary, Some(all_signed_verified()));

    // An unchanged re-read emits nothing.
    assert_eq!(refresh(&mut state, ALICE, all_signed_verified()), 0);
}

#[test]
fn store_refresh_projects_identity_change_attention_only_after_verification() {
    let mut state = ready_state();
    request(&mut state, 1, ALICE);
    load(&mut state, 1, ALICE, all_signed_verified());
    let changed = summary(
        ContactDevicesStatus::SomeNotOwnerSigned,
        vec![ContactDeviceSignature::OwnerSignatureInvalid],
        ContactIdentityVerification::ChangedAfterVerification,
    );
    assert_eq!(refresh(&mut state, ALICE, changed.clone()), 1);
    assert_eq!(state.contact_security.summary, Some(changed));
}

#[test]
fn store_refresh_is_ignored_while_checking() {
    let mut state = ready_state();
    request(&mut state, 1, ALICE);
    assert_eq!(refresh(&mut state, ALICE, all_signed_verified()), 0);
    assert!(state.contact_security.summary.is_none());
}

#[test]
fn closing_user_info_drops_later_answers() {
    let mut state = ready_state();
    request(&mut state, 1, ALICE);
    assert_eq!(
        reduce(&mut state, AppAction::ContactSecurityClosed).len(),
        1
    );
    assert_eq!(state.contact_security, Default::default());
    assert_eq!(load(&mut state, 1, ALICE, all_signed_verified()), 0);
    assert_eq!(refresh(&mut state, ALICE, all_signed_verified()), 0);
    assert_eq!(state.contact_security, Default::default());
    // Closing twice is a no-op.
    assert_eq!(
        reduce(&mut state, AppAction::ContactSecurityClosed).len(),
        0
    );
}

#[test]
fn contact_security_debug_omits_the_contact_user_id() {
    let mut state = ready_state();
    request(&mut state, 1, ALICE);
    let debug = format!("{:?}", state.contact_security);
    assert!(!debug.contains("alice"), "{debug}");
}

// ── Verify user: initiator in the shared verification state ────────────────

fn contact_target() -> koushi_state::VerificationTarget {
    koushi_state::VerificationTarget {
        user_id: BOB.to_owned(),
        device_id: String::new(),
    }
}

#[test]
fn our_verify_user_request_waits_for_them_instead_of_offering_accept() {
    use koushi_state::{SasEmoji, VerificationFlowState, VerificationInitiator};
    let mut state = ready_state();
    assert!(
        !reduce(
            &mut state,
            AppAction::VerificationRequestSent {
                request_id: 21,
                target: contact_target(),
            },
        )
        .is_empty()
    );
    assert_eq!(
        state.e2ee_trust.verification,
        VerificationFlowState::Requested {
            request_id: 21,
            target: contact_target(),
            initiator: VerificationInitiator::Us,
        }
    );
    // A second request while one is active is refused, so the runtime does
    // not start another SDK flow.
    assert!(
        reduce(
            &mut state,
            AppAction::VerificationRequestSent {
                request_id: 22,
                target: contact_target(),
            },
        )
        .is_empty()
    );
    // The contact accepted (projected by the actor from the SDK request).
    reduce(
        &mut state,
        AppAction::VerificationAccepted { request_id: 21 },
    );
    assert_eq!(
        state.e2ee_trust.verification,
        VerificationFlowState::Accepted {
            request_id: 21,
            target: contact_target(),
            initiator: VerificationInitiator::Us,
        }
    );
    let emojis = vec![SasEmoji {
        symbol: "🐶".to_owned(),
        description: "Dog".to_owned(),
    }];
    reduce(
        &mut state,
        AppAction::VerificationSasPresented {
            request_id: 21,
            emojis,
        },
    );
    reduce(
        &mut state,
        AppAction::VerificationConfirmed { request_id: 21 },
    );
    reduce(
        &mut state,
        AppAction::VerificationCompleted { request_id: 21 },
    );
    assert_eq!(
        state.e2ee_trust.verification,
        VerificationFlowState::Done {
            request_id: 21,
            target: contact_target(),
        }
    );
}

#[test]
fn incoming_requests_stay_acceptable_from_them() {
    use koushi_state::{VerificationFlowState, VerificationInitiator};
    let mut state = ready_state();
    reduce(
        &mut state,
        AppAction::VerificationRequested {
            request_id: 31,
            target: contact_target(),
        },
    );
    assert!(matches!(
        state.e2ee_trust.verification,
        VerificationFlowState::Requested {
            initiator: VerificationInitiator::Them,
            ..
        }
    ));
    reduce(
        &mut state,
        AppAction::VerificationAccepted { request_id: 31 },
    );
    assert!(matches!(
        state.e2ee_trust.verification,
        VerificationFlowState::Accepted {
            initiator: VerificationInitiator::Them,
            ..
        }
    ));
}

#[test]
fn verify_user_failure_and_cancel_settle_the_shared_flow() {
    use koushi_state::{
        TrustOperationFailureKind, VerificationCancelReason, VerificationFlowState,
    };
    let mut state = ready_state();
    let send = |state: &mut AppState, request_id| {
        reduce(
            state,
            AppAction::VerificationRequestSent {
                request_id,
                target: contact_target(),
            },
        )
    };
    send(&mut state, 41);
    reduce(
        &mut state,
        AppAction::VerificationFailed {
            request_id: 41,
            kind: TrustOperationFailureKind::Network,
        },
    );
    assert!(matches!(
        state.e2ee_trust.verification,
        VerificationFlowState::Failed {
            request_id: 41,
            kind: TrustOperationFailureKind::Network,
            ..
        }
    ));
    // Retrying after a failure is allowed; cancelling returns to idle.
    assert!(!send(&mut state, 42).is_empty());
    reduce(
        &mut state,
        AppAction::VerificationCancelled {
            request_id: 42,
            reason: VerificationCancelReason::User,
        },
    );
    assert_eq!(state.e2ee_trust.verification, VerificationFlowState::Idle);
}

#[test]
fn session_verification_gate_is_not_affected_by_verify_user() {
    use koushi_state::{
        ProvisionalPhase, VerificationAccountKind, VerificationFlowState, VerificationGateState,
        VerificationMethod, VerificationMethodCapability,
    };
    let mut state = AppState {
        session: SessionState::Verifying {
            info: session_info("@me:example.test"),
            gate: VerificationGateState {
                methods: vec![VerificationMethodCapability::ExistingDeviceSas],
                account_kind: VerificationAccountKind::ExistingIdentity,
                failure: None,
            },
            method: VerificationMethod::ExistingDeviceSas,
            flow_id: 51,
            sas_emojis: vec![],
        },
        ..AppState::default()
    };
    // Verify user is refused while this session is still being verified.
    assert!(
        reduce(
            &mut state,
            AppAction::VerificationRequestSent {
                request_id: 52,
                target: contact_target(),
            },
        )
        .is_empty()
    );
    assert_eq!(state.e2ee_trust.verification, VerificationFlowState::Idle);
    // The own-device SAS gate still completes as before.
    reduce(
        &mut state,
        AppAction::VerificationCompleted { request_id: 51 },
    );
    assert!(matches!(
        state.session,
        SessionState::Provisional {
            phase: ProvisionalPhase::RecheckingTrust { .. },
            ..
        }
    ));
}

#[test]
fn verify_user_is_busy_while_another_verification_is_in_progress() {
    use koushi_state::{AppEffect, TrustOperationFailureKind, UiEvent, VerificationTarget};
    let contact_changed = |effects: &[AppEffect]| {
        effects.contains(&AppEffect::EmitUiEvent(UiEvent::ContactSecurityChanged))
    };
    let mut state = ready_state();
    request(&mut state, 1, BOB);
    load(&mut state, 1, BOB, all_signed_not_verified());
    assert!(!state.contact_security.verification_busy);

    // Another person's request arrives while Bob's User info is open.
    let effects = reduce(
        &mut state,
        AppAction::VerificationRequested {
            request_id: 61,
            target: VerificationTarget {
                user_id: ALICE.to_owned(),
                device_id: "ALICEDEVICE".to_owned(),
            },
        },
    );
    assert!(state.contact_security.verification_busy);
    assert!(contact_changed(&effects));
    // A store refresh keeps the projection.
    refresh(&mut state, BOB, all_signed_verified());
    assert!(state.contact_security.verification_busy);

    let effects = reduce(
        &mut state,
        AppAction::VerificationFailed {
            request_id: 61,
            kind: TrustOperationFailureKind::Cancelled,
        },
    );
    assert!(!state.contact_security.verification_busy);
    assert!(contact_changed(&effects));

    // Opening User info while a flow is active starts busy; closing clears.
    reduce(
        &mut state,
        AppAction::VerificationRequested {
            request_id: 62,
            target: contact_target(),
        },
    );
    request(&mut state, 2, ALICE);
    assert!(state.contact_security.verification_busy);
    reduce(&mut state, AppAction::ContactSecurityClosed);
    assert_eq!(
        state.contact_security,
        koushi_state::ContactSecurityState::default()
    );
}
