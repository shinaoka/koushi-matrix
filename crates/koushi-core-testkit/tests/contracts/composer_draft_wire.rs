use std::sync::Arc;

use koushi_core::composer_draft_lifecycle::{
    ComposerDraftLeaseFailure, ComposerDraftLeaseRegistry, ComposerDraftScope,
    ComposerDraftWireError, ComposerRendererGeneration,
};
use koushi_protocol::SessionKeyId;
use koushi_state::ComposerTarget;

#[path = "../support/mod.rs"]
#[expect(
    clippy::duplicate_mod,
    reason = "suite aggregation preserves feature-local fixture scopes and registered helper tests"
)]
mod support;
use support::{ready_room_conn, session_key};

fn account(name: &str) -> SessionKeyId {
    SessionKeyId {
        homeserver: format!("https://{name}.invalid"),
        user_id: format!("@{name}:invalid"),
        device_id: format!("{name}-device"),
    }
}

fn main_scope(account: SessionKeyId, room_id: &str) -> ComposerDraftScope {
    ComposerDraftScope {
        account,
        target: ComposerTarget::Main {
            room_id: room_id.to_owned(),
        },
    }
}

#[test]
fn composer_identity_wire_values_are_canonical_nonzero_u64() {
    let generation =
        ComposerRendererGeneration::parse_wire("18446744073709551615").expect("u64 max is valid");
    assert_eq!(generation.to_wire_string(), "18446744073709551615");

    for (wire, error) in [
        ("", ComposerDraftWireError::Invalid),
        ("01", ComposerDraftWireError::NonCanonical),
        ("0", ComposerDraftWireError::Zero),
        ("18446744073709551616", ComposerDraftWireError::Overflow),
    ] {
        assert_eq!(ComposerRendererGeneration::parse_wire(wire), Err(error));
    }
}

#[test]
fn registry_counter_exhaustion_is_checked() {
    let registry = Arc::new(ComposerDraftLeaseRegistry::new());
    registry.set_next_generation_for_testing(u64::MAX);
    assert_eq!(
        registry.begin_renderer_generation(),
        Err(ComposerDraftLeaseFailure::CounterExhausted)
    );

    let registry = Arc::new(ComposerDraftLeaseRegistry::new());
    let generation = registry.begin_renderer_generation().expect("generation");
    registry.set_next_lease_id_for_testing(u64::MAX);
    assert_eq!(
        registry.acquire(generation, main_scope(account("counter"), "room")),
        Err(ComposerDraftLeaseFailure::CounterExhausted)
    );
}

#[tokio::test]
async fn core_lease_admission_requires_ready_exact_active_scope() {
    let (runtime, connection, _, _, _) = ready_room_conn("wire-room").await;
    let expected_account = session_key();
    let generation = connection
        .begin_composer_draft_renderer_generation()
        .expect("begin generation");
    let target = ComposerTarget::Main {
        room_id: "wire-room".to_owned(),
    };
    let lease = connection
        .acquire_composer_draft_lease_for_active_target(
            expected_account.clone(),
            generation,
            target.clone(),
        )
        .expect("active target lease");

    assert!(
        connection
            .acquire_composer_draft_command_permit_for_active_target(
                expected_account.clone(),
                target.clone(),
                generation,
                lease.lease_id,
            )
            .is_ok()
    );
    assert!(
        connection
            .acquire_composer_draft_command_permit_for_active_target(
                account("other"),
                target.clone(),
                generation,
                lease.lease_id,
            )
            .is_err()
    );
    assert!(
        connection
            .acquire_composer_draft_lease_for_active_target(
                expected_account,
                generation,
                ComposerTarget::Main {
                    room_id: "other-room".to_owned(),
                },
            )
            .is_err()
    );

    drop(connection);
    runtime.shutdown().await;
}

#[tokio::test]
async fn renderer_replacement_retires_old_numeric_lease() {
    let (runtime, connection, _, _, _) = ready_room_conn("replacement-room").await;
    let account = session_key();
    let target = ComposerTarget::Main {
        room_id: "replacement-room".to_owned(),
    };
    let old_generation = connection
        .begin_composer_draft_renderer_generation()
        .expect("old generation");
    let old_lease = connection
        .acquire_composer_draft_lease_for_active_target(
            account.clone(),
            old_generation,
            target.clone(),
        )
        .expect("old lease");
    let new_generation = connection
        .begin_composer_draft_renderer_generation()
        .expect("new generation");

    assert!(
        connection
            .acquire_composer_draft_command_permit_for_active_target(
                account.clone(),
                target.clone(),
                old_generation,
                old_lease.lease_id,
            )
            .is_err()
    );
    let new_lease = connection
        .acquire_composer_draft_lease_for_active_target(account, new_generation, target)
        .expect("new lease");
    assert_ne!(old_lease, new_lease);

    drop(connection);
    runtime.shutdown().await;
}
