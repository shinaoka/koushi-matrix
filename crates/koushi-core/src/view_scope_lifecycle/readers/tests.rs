use super::*;
use crate::view_scope_lifecycle::ViewScopeRegistry;

fn source_in(room_id: &str) -> ReceiptSourceRef {
    serde_json::from_value(serde_json::json!({
        "key": {"account_key": "account", "kind": {"Room": {"room_id": room_id}}},
        "projection_request_id": {"connection_id": "1", "sequence": "2"},
        "generation":"3", "event_id":"$event"
    }))
    .unwrap()
}

fn source() -> ReceiptSourceRef {
    source_in("!r:example.org")
}

async fn accept_profile_raw(registry: &ViewScopeRegistry, work: &mut ReaderWork, user_id: &str) {
    let source = work.source.clone();
    let mut raw = crate::timeline::RawReceiptWindow {
        total_count: 1,
        start: 0,
        receipts: vec![koushi_state::LiveReadReceipt {
            user_id: user_id.to_owned(),
            display_name: None,
            original_display_label: String::new(),
            avatar: None,
            timestamp_ms: None,
        }],
        profiles: vec![koushi_sdk::MatrixUserProfile {
            user_id: user_id.to_owned(),
            display_name: Some("Room profile".to_owned()),
            avatar_mxc_uri: Some(format!("mxc://example.org/{user_id}")),
        }],
        owner: None,
        epoch: std::sync::Weak::new(),
    };
    let _epoch = raw.bind_test_owner(&source).await;
    let retained = registry.retain_reader_raw(work, raw).unwrap();
    registry
        .accept_reader_raw(
            work,
            retained,
            std::iter::once(format!("mxc://example.org/{user_id}")),
        )
        .unwrap();
}

#[tokio::test]
async fn window_updates_refetch_changed_ranges_and_preserve_source_invalidation() {
    use koushi_protocol::view::{ReaderWindow, ResolvedReaderAnchor, ViewModel};
    for (start, limit, source_changed, refetch) in [
        (1, 1, false, true),
        (0, 2, false, true),
        (0, 1, true, true),
        (0, 1, false, false),
    ] {
        let registry = ViewScopeRegistry::default();
        let consumer = registry
            .consumer(koushi_protocol::RuntimeConnectionId(4))
            .unwrap();
        let mut scope = consumer
            .open_reader(source(), 0, ReaderWindowLimit::try_from(1).unwrap())
            .unwrap();
        let mut work = registry.take_reader_work().unwrap().unwrap();
        let mut raw = crate::timeline::RawReceiptWindow {
            total_count: 1,
            start: 0,
            receipts: vec![koushi_state::LiveReadReceipt {
                user_id: "@a:example.org".into(),
                display_name: None,
                original_display_label: String::new(),
                avatar: None,
                timestamp_ms: None,
            }],
            profiles: vec![],
            owner: None,
            epoch: std::sync::Weak::new(),
        };
        let _epoch = raw.bind_test_owner(&source()).await;
        let retained = registry.retain_reader_raw(&mut work, raw.clone()).unwrap();
        registry
            .accept_reader_raw(&mut work, retained, std::iter::empty())
            .unwrap();
        let resolved = raw.into_resolved(koushi_state::CatalogLocale::En);
        let model = ViewModel::ReaderReady(ReaderWindow {
            source: source(),
            total_count: 1,
            start: 0,
            rows: resolved.rows.clone(),
            window_sequence: 0,
            source_revision: resolved.source_revision().unwrap(),
            dependency_revision: 1,
            resolved_anchor: ResolvedReaderAnchor::NotRequested,
        });
        let revision = registry
            .publish_current(scope.id(), model, vec![], &resolved)
            .unwrap();
        let _delivery = scope.next_delivery().await.unwrap();
        consumer.ack_model(scope.id(), revision).unwrap();
        registry.finish_reader_work(&work).unwrap();
        drop(work);
        if source_changed {
            registry.dirty_reader(scope.id(), true).unwrap();
        }
        consumer
            .update_reader_window(
                scope.id(),
                revision,
                1,
                ReaderWindowTarget::Index { start },
                ReaderWindowLimit::try_from(limit).unwrap(),
            )
            .unwrap();
        let next = registry.take_reader_work().unwrap().unwrap();
        assert_eq!(
            next.raw.is_none(),
            refetch,
            "start={start} limit={limit} source_changed={source_changed}"
        );
    }
}

#[test]
fn retained_reader_avatar_identities_release_their_budget_with_the_owner() {
    let budget = crate::view_budget::ViewBudget::default();
    let context = koushi_state::AvatarDemandContext {
        account_id: "account".into(),
        session_generation: 1,
    };
    let ids = vec!["@visible:example.invalid".to_owned()];
    let observed = super::ObservedReaderAvatars::retain(&context, &ids, &[], &budget).unwrap();
    let bytes = observed._bytes.bytes();
    let _remaining = budget.reserve_bytes(256 * 1024 * 1024 - bytes).unwrap();
    assert!(matches!(
        super::ObservedReaderAvatars::retain(&context, &ids, &[], &budget),
        Err(ScopeError::Capacity)
    ));
    drop(observed);
    assert!(budget.reserve_bytes(bytes).is_some());
}

#[tokio::test]
async fn avatar_observation_requires_a_live_source_even_with_an_installed_model() {
    use koushi_protocol::view::{ReaderWindow, ResolvedReaderAnchor, ViewModel};
    let registry = ViewScopeRegistry::default();
    let consumer = registry
        .consumer(koushi_protocol::RuntimeConnectionId(4))
        .unwrap();
    let mut scope = consumer
        .open_reader(source(), 0, ReaderWindowLimit::try_from(3).unwrap())
        .unwrap();
    let mut work = registry.take_reader_work().unwrap().unwrap();
    let mut raw = crate::timeline::RawReceiptWindow {
        total_count: 1,
        start: 0,
        receipts: vec![koushi_state::LiveReadReceipt {
            user_id: "@a:example.org".into(),
            display_name: None,
            original_display_label: String::new(),
            avatar: Some(koushi_state::AvatarImage {
                mxc_uri: "mxc://example.invalid/observed-reader-avatar".into(),
                thumbnail: koushi_state::AvatarThumbnailState::NotRequested,
            }),
            timestamp_ms: None,
        }],
        profiles: vec![],
        owner: None,
        epoch: std::sync::Weak::new(),
    };
    let epoch = raw.bind_test_owner(&source()).await;
    let retained = registry.retain_reader_raw(&mut work, raw.clone()).unwrap();
    registry
        .accept_reader_raw(
            &mut work,
            retained,
            std::iter::once("mxc://example.invalid/observed-reader-avatar".to_owned()),
        )
        .unwrap();
    let mut resolved = raw.clone().into_resolved(koushi_state::CatalogLocale::En);
    let resources = std::mem::take(&mut resolved.avatar_resources);
    let mut model = ViewModel::ReaderReady(ReaderWindow {
        source: source(),
        total_count: 1,
        start: 0,
        rows: resolved.rows.clone(),
        window_sequence: 0,
        source_revision: resolved.source_revision().unwrap(),
        dependency_revision: 1,
        resolved_anchor: ResolvedReaderAnchor::NotRequested,
    });
    assert!(!serde_json::to_string(&model).unwrap().contains("mxc://"));
    let revision = registry
        .publish_current(scope.id(), model.clone(), resources, &resolved)
        .unwrap();
    let _delivery = scope.next_delivery().await.unwrap();
    consumer.ack_model(scope.id(), revision).unwrap();
    assert_eq!(
        consumer
            .with_live_reader_avatar_source(scope.id(), revision, |rows| {
                assert_eq!(
                    rows.avatar_mxc("@a:example.org")?,
                    Some("mxc://example.invalid/observed-reader-avatar")
                );
                Ok(42)
            })
            .unwrap(),
        42
    );
    let context = koushi_state::AvatarDemandContext {
        account_id: "account".into(),
        session_generation: 1,
    };
    assert!(
        registry
            .avatar_demand_for_context(Some(&context))
            .unwrap()
            .resources_by_priority()
            .is_empty()
    );
    consumer
        .observe_current_reader_avatars(scope.id(), revision, 1, &["@a:example.org".into()], &[])
        .unwrap();
    let installed = registry.avatar_demand_for_context(Some(&context)).unwrap();
    assert_eq!(
        installed.resources_by_priority(),
        ["mxc://example.invalid/observed-reader-avatar"]
    );
    assert_eq!(
        installed.scope_ids().collect::<Vec<_>>(),
        vec![scope.id().0]
    );
    let mut unchanged = raw.clone().into_resolved(koushi_state::CatalogLocale::En);
    let resources = std::mem::take(&mut unchanged.avatar_resources);
    registry
        .publish_current(scope.id(), model.clone(), resources, &unchanged)
        .unwrap();
    assert!(
        std::sync::Arc::ptr_eq(
            &installed,
            &registry.avatar_demand_for_context(Some(&context)).unwrap()
        ),
        "unchanged bindings must not republish demand on thumbnail/model updates"
    );
    // The next projection changes only private resource identity. The host has
    // not acknowledged it and must not be able to restore the old URI.
    raw.receipts[0].avatar.as_mut().unwrap().mxc_uri =
        "mxc://example.invalid/rebound-reader-avatar".into();
    let mut rebound = raw.into_resolved(koushi_state::CatalogLocale::En);
    let resources = std::mem::take(&mut rebound.avatar_resources);
    let next_revision = registry
        .publish_current(scope.id(), model.clone(), resources, &rebound)
        .unwrap();
    assert_eq!(
        registry
            .avatar_demand_for_context(Some(&context))
            .unwrap()
            .resources_by_priority(),
        ["mxc://example.invalid/rebound-reader-avatar"],
        "reprojection must refresh demand without host re-observation"
    );
    consumer
        .observe_reader_avatars(
            scope.id(),
            revision,
            2,
            &context,
            &["@a:example.org".into()],
            &[],
        )
        .unwrap();
    assert_eq!(
        registry
            .avatar_demand_for_context(Some(&context))
            .unwrap()
            .resources_by_priority(),
        ["mxc://example.invalid/rebound-reader-avatar"]
    );
    let _pending = scope.next_delivery().await.unwrap();
    consumer
        .observe_reader_avatars(
            scope.id(),
            revision,
            3,
            &context,
            &["@a:example.org".into()],
            &[],
        )
        .unwrap();
    assert_eq!(
        registry
            .avatar_demand_for_context(Some(&context))
            .unwrap()
            .resources_by_priority(),
        ["mxc://example.invalid/rebound-reader-avatar"]
    );
    // A newer removal must beat even the in-flight projection's old binding.
    if let ViewModel::ReaderReady(window) = &mut model {
        window.rows[0].avatar = None;
    }
    rebound.rows[0].avatar = None;
    registry
        .publish_current(scope.id(), model, vec![], &rebound)
        .unwrap();
    assert!(
        registry
            .avatar_demand_for_context(Some(&context))
            .unwrap()
            .resources_by_priority()
            .is_empty(),
        "avatar removal must withdraw demand without another observation"
    );
    consumer
        .observe_reader_avatars(
            scope.id(),
            revision,
            4,
            &context,
            &["@a:example.org".into()],
            &[],
        )
        .unwrap();
    assert!(
        registry
            .avatar_demand_for_context(Some(&context))
            .unwrap()
            .resources_by_priority()
            .is_empty()
    );
    // Keep the old installed revision for the remaining admission checks.
    assert_ne!(next_revision, revision);
    assert_eq!(
        consumer.observe_reader_avatars(scope.id(), revision, 1, &context, &[], &[]),
        Err(ScopeError::InvalidRevision)
    );
    assert_eq!(
        consumer.observe_reader_avatars(
            scope.id(),
            revision,
            2,
            &context,
            &["@foreign:example.org".into()],
            &[]
        ),
        Err(ScopeError::InvalidModel)
    );
    assert_eq!(
        consumer.observe_reader_avatars(
            scope.id(),
            revision,
            2,
            &context,
            &vec!["@a:example.org".into(); 257],
            &[]
        ),
        Err(ScopeError::Capacity)
    );
    epoch.lock().unwrap().valid = false;
    assert_eq!(
        consumer.observe_reader_avatars(scope.id(), revision, 2, &context, &[], &[]),
        Err(ScopeError::SourceUnavailable)
    );
    assert!(consumer.avatar_source(scope.id(), revision).is_ok());
    assert_eq!(
        consumer
            .with_live_reader_avatar_source::<()>(scope.id(), revision, |_| {
                panic!("retired source must not admit an observation")
            })
            .err(),
        Some(ScopeError::SourceUnavailable)
    );
    epoch.lock().unwrap().valid = true;
    registry.avatar_demand_for_context(None);
    assert_eq!(
        consumer.observe_reader_avatars(scope.id(), revision, 2, &context, &[], &[]),
        Err(ScopeError::InactiveSession)
    );
    let mut changed = context.clone();
    changed.session_generation += 1;
    registry.avatar_demand_for_context(Some(&changed));
    assert_eq!(
        consumer.observe_reader_avatars(scope.id(), revision, 2, &context, &[], &[]),
        Err(ScopeError::InactiveSession)
    );
}

#[tokio::test]
async fn room_profile_changes_dirty_only_matching_room_and_user() {
    let registry = ViewScopeRegistry::default();
    let consumer = registry
        .consumer(koushi_protocol::RuntimeConnectionId(8))
        .unwrap();
    let same_room = consumer
        .open_reader(
            source_in("!room-a:example.org"),
            0,
            ReaderWindowLimit::try_from(1).unwrap(),
        )
        .unwrap();
    let mut same_room_work = registry.take_reader_work().unwrap().unwrap();
    accept_profile_raw(&registry, &mut same_room_work, "@a:example.org").await;
    registry.finish_reader_work(&same_room_work).unwrap();
    drop(same_room_work);

    let other_room = consumer
        .open_reader(
            source_in("!room-b:example.org"),
            0,
            ReaderWindowLimit::try_from(1).unwrap(),
        )
        .unwrap();
    let mut other_room_work = registry.take_reader_work().unwrap().unwrap();
    accept_profile_raw(&registry, &mut other_room_work, "@a:example.org").await;
    registry.finish_reader_work(&other_room_work).unwrap();
    drop(other_room_work);

    let other_user = consumer
        .open_reader(
            source_in("!room-a:example.org"),
            0,
            ReaderWindowLimit::try_from(1).unwrap(),
        )
        .unwrap();
    let mut other_user_work = registry.take_reader_work().unwrap().unwrap();
    accept_profile_raw(&registry, &mut other_user_work, "@b:example.org").await;
    registry.finish_reader_work(&other_user_work).unwrap();
    drop(other_user_work);

    registry.reader_room_profiles_changed("!room-a:example.org", &["@a:example.org".into()]);
    let refreshed = registry.take_reader_work().unwrap().unwrap();
    assert_eq!(refreshed.scope, same_room.id());
    assert!(
        refreshed.raw.is_some(),
        "profile-only changes reuse accepted raw"
    );
    registry.finish_reader_work(&refreshed).unwrap();
    drop(refreshed);
    assert!(registry.take_reader_work().unwrap().is_none());

    registry.reader_room_profiles_changed("!room-b:example.org", &["@b:example.org".into()]);
    assert!(registry.take_reader_work().unwrap().is_none());

    registry.reader_avatar_thumbnail_changed("mxc://example.org/@a:example.org");
    let avatar_refresh = registry.take_reader_work().unwrap().unwrap();
    let shared_avatar_refresh = registry.take_reader_work().unwrap().unwrap();
    assert_ne!(avatar_refresh.scope, shared_avatar_refresh.scope);
    assert!(
        [same_room.id(), other_room.id()].contains(&avatar_refresh.scope)
            && [same_room.id(), other_room.id()].contains(&shared_avatar_refresh.scope)
    );
    registry.finish_reader_work(&avatar_refresh).unwrap();
    registry.finish_reader_work(&shared_avatar_refresh).unwrap();
    drop((avatar_refresh, shared_avatar_refresh));
    assert!(registry.take_reader_work().unwrap().is_none());

    registry.reader_avatar_thumbnail_changed("mxc://example.org/@unknown:example.org");
    assert!(registry.take_reader_work().unwrap().is_none());
    drop((same_room, other_room, other_user));
}

#[test]
fn receipt_source_changes_dirty_only_matching_reader_scopes() {
    let registry = ViewScopeRegistry::default();
    let consumer = registry
        .consumer(koushi_protocol::RuntimeConnectionId(8))
        .unwrap();
    let scope = consumer
        .open_reader(source(), 0, ReaderWindowLimit::try_from(3).unwrap())
        .unwrap();
    let initial = registry.take_reader_work().unwrap().unwrap();
    registry.finish_reader_work(&initial).unwrap();

    registry.reader_receipt_source_changed("account", "!r:example.org", "$event");
    let refreshed = registry.take_reader_work().unwrap().unwrap();
    assert_eq!(refreshed.scope, scope.id());
    assert!(refreshed.raw.is_none());
    registry.finish_reader_work(&refreshed).unwrap();

    registry.reader_receipt_source_changed("other-account", "!r:example.org", "$event");
    assert!(registry.take_reader_work().unwrap().is_none());
}

#[test]
fn dirty_work_coalesces_while_queued_and_running_then_requeues_at_tail() {
    let registry = ViewScopeRegistry::default();
    let consumer = registry
        .consumer(koushi_protocol::RuntimeConnectionId(1))
        .unwrap();
    let a = consumer
        .open_reader(source(), 0, ReaderWindowLimit::try_from(3).unwrap())
        .unwrap();
    let b = consumer
        .open_reader(source(), 0, ReaderWindowLimit::try_from(3).unwrap())
        .unwrap();
    registry.dirty_reader(a.id(), true).unwrap();
    let first = registry.take_reader_work().unwrap().unwrap();
    assert_eq!(first.scope, a.id());
    registry.dirty_reader(a.id(), false).unwrap();
    registry.dirty_reader(a.id(), true).unwrap();
    assert_eq!(registry.state.lock().unwrap().reader_queue.len(), 1);
    registry.finish_reader_work(&first).unwrap();
    let second = registry.take_reader_work().unwrap().unwrap();
    assert_eq!(
        second.scope,
        b.id(),
        "requeued work goes behind already queued scopes"
    );
    let rerun = registry.take_reader_work().unwrap().unwrap();
    assert_eq!(rerun.scope, a.id());
    assert_eq!(rerun.dependency_revision, 2);
    assert!(registry.take_reader_work().unwrap().is_none());
    assert_eq!(
        registry.finish_reader_work(&first).err(),
        Some(crate::view_scope_lifecycle::ScopeError::InvalidRevision)
    );
    drop(first);
    registry.finish_reader_work(&rerun).unwrap();
    registry.finish_reader_work(&second).unwrap();
    assert!(registry.take_reader_work().unwrap().is_none());
}

#[tokio::test]
async fn failed_admission_and_abandoned_work_terminate_without_requeueing() {
    use crate::view_scope_lifecycle::{ScopeDelivery, ScopeError};
    use koushi_protocol::view::ViewRetirement;
    for case in 0..3 {
        let registry = ViewScopeRegistry::default();
        let consumer = registry
            .consumer(koushi_protocol::RuntimeConnectionId(2))
            .unwrap();
        let mut scope = consumer
            .open_reader(source(), 0, ReaderWindowLimit::try_from(3).unwrap())
            .unwrap();
        let _held = if case == 0 {
            Some(registry.budget.reserve_bytes(200 * 1024 * 1024).unwrap())
        } else {
            None
        };
        let reason = match case {
            0 => {
                assert_eq!(
                    registry.take_reader_work().err(),
                    Some(ScopeError::Capacity)
                );
                ViewRetirement::Capacity
            }
            1 => {
                scope
                    .control
                    .reader
                    .lock()
                    .unwrap()
                    .as_mut()
                    .unwrap()
                    .dependency_revision = u64::MAX;
                assert_eq!(
                    registry.dirty_reader(scope.id(), false).err(),
                    Some(ScopeError::CounterExhausted)
                );
                ViewRetirement::CounterExhausted
            }
            _ => {
                drop(registry.take_reader_work().unwrap().unwrap());
                ViewRetirement::ProducerFailed
            }
        };
        assert!(
            matches!(scope.next_delivery().await, Some(ScopeDelivery::Retired(observed)) if observed == reason)
        );
        assert!(registry.take_reader_work().unwrap().is_none());
    }
}

#[tokio::test]
async fn accepted_raw_is_scope_owned_and_reused_without_mutating_its_hints() {
    let registry = ViewScopeRegistry::default();
    let consumer = registry
        .consumer(koushi_protocol::RuntimeConnectionId(4))
        .unwrap();
    let scope = consumer
        .open_reader(source(), 0, ReaderWindowLimit::try_from(3).unwrap())
        .unwrap();
    let mut work = registry.take_reader_work().unwrap().unwrap();
    let mut raw = crate::timeline::RawReceiptWindow {
        total_count: 3,
        start: 0,
        receipts: ["@a:example.org", "@b:example.org", "@c:example.org"]
            .into_iter()
            .map(|user_id| koushi_state::LiveReadReceipt {
                user_id: user_id.into(),
                display_name: None,
                original_display_label: String::new(),
                avatar: None,
                timestamp_ms: None,
            })
            .collect(),
        profiles: vec![koushi_sdk::MatrixUserProfile {
            user_id: "@a:example.org".into(),
            display_name: Some("Original".into()),
            avatar_mxc_uri: None,
        }],
        owner: None,
        epoch: std::sync::Weak::new(),
    };
    let _epoch = raw.bind_test_owner(&source()).await;
    let retained = registry.retain_reader_raw(&mut work, raw).unwrap();
    registry
        .accept_reader_raw(&mut work, retained.clone(), std::iter::empty())
        .unwrap();
    {
        let mut replacement = retained.raw.clone();
        let _replacement_epoch = replacement.bind_test_owner(&source()).await;
        let replacement = registry.retain_reader_raw(&mut work, replacement).unwrap();
        assert_eq!(
            registry.validate_reader_owner(&work, &replacement).err(),
            Some(crate::view_scope_lifecycle::ScopeError::SourceRetired)
        );
        assert_eq!(
            registry
                .accept_reader_raw(&mut work, replacement, std::iter::empty())
                .err(),
            Some(crate::view_scope_lifecycle::ScopeError::SourceRetired),
            "same public source cannot silently adopt a replacement actor"
        );
    }
    {
        let narrow = consumer
            .open_reader(source(), 0, ReaderWindowLimit::try_from(1).unwrap())
            .unwrap();
        let mut other = registry.take_reader_work().unwrap().unwrap();
        assert_eq!(other.scope, narrow.id());
        assert_eq!(
            registry
                .accept_reader_raw(&mut other, retained.clone(), std::iter::empty())
                .err(),
            Some(crate::view_scope_lifecycle::ScopeError::InvalidRevision)
        );
        registry.finish_reader_work(&other).unwrap();
    }
    {
        let _pressure = registry.budget.reserve_bytes(190 * 1024 * 1024).unwrap();
        let mut candidate = retained.raw.clone();
        candidate.profiles[0].display_name = Some("x".repeat(3 * 1024 * 1024));
        assert!(
            registry.retain_reader_raw(&mut work, candidate).is_ok(),
            "already-admitted bytes transfer without re-admission under pressure"
        );
    }
    registry.finish_reader_work(&work).unwrap();
    drop(work);
    registry.dirty_reader(scope.id(), false).unwrap();
    let next = registry.take_reader_work().unwrap().unwrap();
    let cached = next.raw.as_ref().unwrap();
    assert!(std::sync::Arc::ptr_eq(cached, &retained));
    let mut projection = cached.raw.clone();
    projection.profiles[0].display_name = Some("Alias".into());
    assert_eq!(
        retained.raw.profiles[0].display_name.as_deref(),
        Some("Original")
    );
    let weak = std::sync::Arc::downgrade(&retained);
    drop(retained);
    drop(scope);
    assert!(
        weak.upgrade().is_some(),
        "in-flight work retains its charged input"
    );
    drop(next);
    assert!(weak.upgrade().is_none());
}

#[test]
fn closed_scopes_do_not_accumulate_queued_ids() {
    let registry = ViewScopeRegistry::default();
    let consumer = registry
        .consumer(koushi_protocol::RuntimeConnectionId(3))
        .unwrap();
    for _ in 0..1000 {
        drop(
            consumer
                .open_reader(source(), 0, ReaderWindowLimit::try_from(3).unwrap())
                .unwrap(),
        );
        assert!(registry.state.lock().unwrap().reader_queue.is_empty());
    }
}
