use std::sync::Arc;

use koushi_core::composer_draft_lifecycle::{
    ComposerDraftLeaseFailure, ComposerDraftLeaseRegistry, ComposerDraftScope,
};
use koushi_key::SessionKeyId;
use koushi_state::ComposerTarget;

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

#[tokio::test]
async fn retired_renderer_generation_cannot_submit_or_recreate_target() {
    let registry = Arc::new(ComposerDraftLeaseRegistry::new());
    let generation = registry.begin_renderer_generation().expect("generation");
    let scope = main_scope(account("retired"), "room-retired");
    let lease = registry
        .acquire(generation, scope.clone())
        .expect("active lease");
    let admitted = registry
        .try_command_permit(generation, lease, &scope)
        .expect("admitted command");

    registry.revoke_generation(generation);

    assert!(matches!(
        registry.try_command_permit(generation, lease, &scope),
        Err(ComposerDraftLeaseFailure::RendererGenerationRetired)
    ));
    assert!(
        registry
            .protected_targets(&scope.account)
            .contains(&scope.target)
    );
    drop(admitted);
    assert!(
        !registry
            .protected_targets(&scope.account)
            .contains(&scope.target)
    );
}

#[tokio::test]
async fn account_main_and_thread_leases_are_isolated() {
    let registry = Arc::new(ComposerDraftLeaseRegistry::new());
    let generation = registry.begin_renderer_generation().expect("generation");
    let account_a = account("a");
    let account_b = account("b");
    let scopes = [
        main_scope(account_a.clone(), "same-room"),
        ComposerDraftScope {
            account: account_a.clone(),
            target: ComposerTarget::Thread {
                room_id: "same-room".to_owned(),
                root_event_id: "same-root".to_owned(),
            },
        },
        main_scope(account_b.clone(), "same-room"),
        ComposerDraftScope {
            account: account_b.clone(),
            target: ComposerTarget::Thread {
                room_id: "same-room".to_owned(),
                root_event_id: "same-root".to_owned(),
            },
        },
    ];
    let leases = scopes
        .iter()
        .cloned()
        .map(|scope| registry.acquire(generation, scope).expect("lease"))
        .collect::<Vec<_>>();

    registry.release(generation, leases[0]).expect("release");

    let protected_a = registry.protected_targets(&account_a);
    let protected_b = registry.protected_targets(&account_b);
    assert_eq!(protected_a.len(), 1);
    assert!(matches!(
        protected_a.iter().next(),
        Some(ComposerTarget::Thread { .. })
    ));
    assert_eq!(protected_b.len(), 2);
}

#[tokio::test]
async fn persistence_guard_outlives_activation_release() {
    let registry = Arc::new(ComposerDraftLeaseRegistry::new());
    let mut changes = registry.subscribe();
    let generation = registry.begin_renderer_generation().expect("generation");
    let scope = main_scope(account("persist"), "room-persist");
    let lease = registry
        .acquire(generation, scope.clone())
        .expect("active lease");
    let mut persistence = registry
        .persistence_permits(&scope.account, [scope.target.clone()])
        .expect("persistence permit");

    registry.release(generation, lease).expect("release");
    assert!(
        registry
            .protected_targets(&scope.account)
            .contains(&scope.target)
    );
    changes.borrow_and_update();
    persistence.clear();
    changes
        .changed()
        .await
        .expect("permit release notification");
    assert!(
        !registry
            .protected_targets(&scope.account)
            .contains(&scope.target)
    );
}
