use super::*;
use std::fs;
fn cache_test_lock() -> std::sync::MutexGuard<'static, ()> {
    super::test_cache_lock()
}

#[test]
fn resolved_reader_binds_ready_bytes_and_preserves_missing_avatar_demand() {
    let _guard = cache_test_lock();
    clear_renderable_thumbnail_cache();
    let thumbnail = store_renderable_thumbnail(
        RenderableThumbnailKind::Avatar,
        "mxc://example.test/reader-binding",
        vec![7; 16],
    )
    .unwrap();
    let make_window = || crate::timeline::RawReceiptWindow {
        total_count: 1,
        start: 0,
        receipts: vec![koushi_state::LiveReadReceipt {
            user_id: "@reader:example.test".into(),
            display_name: Some("Reader".into()),
            original_display_label: "Reader".into(),
            timestamp_ms: None,
            avatar: Some(koushi_state::AvatarImage {
                mxc_uri: "mxc://example.test/reader-binding".into(),
                thumbnail: thumbnail.clone(),
            }),
        }],
        profiles: vec![],
        owner: None,
        epoch: std::sync::Weak::new(),
    };
    let mut hinted = make_window();
    hinted.receipts[0].avatar.as_mut().unwrap().thumbnail = AvatarThumbnailState::NotRequested;
    let hinted = hinted.into_resolved(koushi_state::CatalogLocale::En);
    assert!(
        matches!(
            hinted.rows[0].avatar,
            Some(AvatarThumbnailState::Ready { .. })
        ),
        "already-cached bytes must not wait for a missed profile notification"
    );
    drop(hinted);
    let resolved = make_window().into_resolved(koushi_state::CatalogLocale::En);
    assert!(matches!(
        resolved.rows[0].avatar,
        Some(AvatarThumbnailState::Ready { .. })
    ));
    clear_renderable_thumbnail_cache();
    let binding = &resolved.avatar_resources[0];
    assert_eq!(binding.user_id, "@reader:example.test");
    assert_eq!(binding.mxc_uri, "mxc://example.test/reader-binding");
    assert_eq!(binding.lease.as_ref().unwrap().content().bytes, vec![7; 16]);
    let missing = make_window().into_resolved(koushi_state::CatalogLocale::En);
    assert!(matches!(
        missing.rows[0].avatar,
        Some(AvatarThumbnailState::NotRequested)
    ));
    assert_eq!(missing.avatar_resources[0].mxc_uri, binding.mxc_uri);
    assert_eq!(
        missing.avatar_resources[0].lease.as_ref().err(),
        Some(&ThumbnailLeaseError::Unavailable)
    );
    drop(resolved);
    drop(missing);
    let large = store_renderable_thumbnail(
        RenderableThumbnailKind::Avatar,
        "mxc://example.test/large-binding",
        vec![0; MAX_RENDERABLE_THUMBNAIL_BYTES],
    )
    .unwrap();
    let AvatarThumbnailState::Ready { source_ref, .. } = large else {
        panic!("ready")
    };
    let held: Vec<_> = (0..3)
        .map(|_| lease_renderable_thumbnail(&source_ref).unwrap())
        .collect();
    store_renderable_thumbnail(
        RenderableThumbnailKind::Avatar,
        "mxc://example.test/reader-binding",
        vec![7; 16],
    )
    .unwrap();
    let limited = make_window().into_resolved(koushi_state::CatalogLocale::En);
    assert!(matches!(
        limited.rows[0].avatar,
        Some(AvatarThumbnailState::NotRequested)
    ));
    assert_eq!(
        limited.avatar_resources[0].lease.as_ref().err(),
        Some(&ThumbnailLeaseError::Capacity)
    );
    assert_eq!(
        limited.avatar_resources[0].mxc_uri,
        "mxc://example.test/reader-binding"
    );
    drop(held);
    clear_renderable_thumbnail_cache();
}

#[tokio::test]
async fn installed_scope_owns_reader_leases_until_retirement_and_delivery_release() {
    use crate::view_scope_lifecycle::{ScopeDelivery, ViewScopeRegistry};
    use koushi_protocol::view::{ReaderWindow, ResolvedReaderAnchor, ViewModel};
    let _guard = cache_test_lock();
    clear_renderable_thumbnail_cache();
    for hold_delivery in [false, true] {
        let thumbnail = store_renderable_thumbnail(
            RenderableThumbnailKind::Avatar,
            "mxc://example.test/installed",
            vec![7; 16],
        )
        .unwrap();
        let mut raw = crate::timeline::RawReceiptWindow {
            total_count: 1,
            start: 0,
            receipts: vec![koushi_state::LiveReadReceipt {
                user_id: "@reader:example.test".into(),
                display_name: Some("Reader".into()),
                original_display_label: "Reader".into(),
                timestamp_ms: None,
                avatar: Some(koushi_state::AvatarImage {
                    mxc_uri: "mxc://example.test/installed".into(),
                    thumbnail,
                }),
            }],
            profiles: vec![],
            owner: None,
            epoch: std::sync::Weak::new(),
        };
        let source = serde_json::from_value(serde_json::json!({
            "key": {"account_key": "account", "kind": {"Room": {"room_id": "!r:example.org"}}},
            "projection_request_id": {"connection_id": "1", "sequence": "2"},
            "generation": "3", "event_id": "$event"
        }))
        .unwrap();
        let _epoch = raw.bind_test_owner(&source).await;
        let mut resolved = raw.into_resolved(koushi_state::CatalogLocale::En);
        let model = ViewModel::ReaderReady(ReaderWindow {
            source: source.clone(),
            total_count: 1,
            start: 0,
            rows: std::mem::take(&mut resolved.rows),
            window_sequence: 0,
            source_revision: 1,
            dependency_revision: 1,
            resolved_anchor: ResolvedReaderAnchor::NotRequested,
        });
        let registry = ViewScopeRegistry::default();
        let consumer = registry
            .consumer(koushi_protocol::RuntimeConnectionId(1))
            .unwrap();
        let mut scope = consumer
            .open_reader(
                source,
                0,
                koushi_protocol::view::ReaderWindowLimit::try_from(3).unwrap(),
            )
            .unwrap();
        assert_eq!(
            registry
                .publish_current(scope.id(), model.clone(), Vec::new(), &resolved)
                .err(),
            Some(crate::view_scope_lifecycle::ScopeError::InvalidModel)
        );
        let mut wrong_model = model.clone();
        if let ViewModel::ReaderReady(window) = &mut wrong_model {
            if let Some(AvatarThumbnailState::Ready { source_ref, .. }) = &mut window.rows[0].avatar
            {
                *source_ref = "avatar/0000000000000000".into();
            }
        }
        let original = &resolved.avatar_resources[0];
        let wrong_resources = vec![crate::timeline::ReaderAvatarResource {
            user_id: original.user_id.clone(),
            mxc_uri: original.mxc_uri.clone(),
            lease: lease_renderable_thumbnail(original.lease.as_ref().unwrap().source_ref()),
        }];
        assert_eq!(
            registry
                .publish_current(scope.id(), wrong_model, wrong_resources, &resolved)
                .err(),
            Some(crate::view_scope_lifecycle::ScopeError::InvalidModel)
        );
        let reference = original.lease.as_ref().unwrap().source_ref().to_owned();
        for mismatch in 0..4 {
            let mut foreign_model = model.clone();
            if let ViewModel::ReaderReady(window) = &mut foreign_model {
                if mismatch == 3 {
                    window.dependency_revision += 1;
                } else if mismatch == 2 {
                    window.window_sequence += 1;
                } else if mismatch == 1 {
                    window.source_revision += 1;
                } else {
                    window.source.event_id = "$foreign".into();
                }
            }
            let foreign_resources = vec![crate::timeline::ReaderAvatarResource {
                user_id: original.user_id.clone(),
                mxc_uri: original.mxc_uri.clone(),
                lease: lease_renderable_thumbnail(&reference),
            }];
            assert_eq!(
                registry
                    .publish_current(scope.id(), foreign_model, foreign_resources, &resolved)
                    .err(),
                Some(crate::view_scope_lifecycle::ScopeError::InvalidModel)
            );
        }
        let ViewModel::ReaderReady(ready) = &model else {
            unreachable!()
        };
        let wrong_window = consumer
            .open_reader(
                ready.source.clone(),
                1,
                koushi_protocol::view::ReaderWindowLimit::try_from(3).unwrap(),
            )
            .unwrap();
        let unregistered = consumer.open().unwrap();
        for rejected_scope in [&wrong_window, &unregistered] {
            let resources = vec![crate::timeline::ReaderAvatarResource {
                user_id: original.user_id.clone(),
                mxc_uri: original.mxc_uri.clone(),
                lease: lease_renderable_thumbnail(&reference),
            }];
            assert_eq!(
                registry
                    .publish_current(rejected_scope.id(), model.clone(), resources, &resolved)
                    .err(),
                Some(crate::view_scope_lifecycle::ScopeError::InvalidModel)
            );
        }
        let late_model = model.clone();
        let resources = std::mem::take(&mut resolved.avatar_resources);
        let revision = registry
            .publish_current(scope.id(), model, resources, &resolved)
            .unwrap();
        let delivery = scope.next_delivery().await.unwrap();
        assert!(
            matches!(&delivery, ScopeDelivery::Model { revision: observed, .. } if *observed == revision)
        );
        use crate::view_scope_lifecycle::ScopeError;
        assert_eq!(
            consumer.resource(scope.id(), revision, &reference).err(),
            Some(ScopeError::InvalidRevision)
        );
        consumer.ack_model(scope.id(), revision).unwrap();
        let foreign = registry
            .consumer(koushi_protocol::RuntimeConnectionId(1))
            .unwrap();
        assert_eq!(
            foreign.resource(scope.id(), revision, &reference).err(),
            Some(ScopeError::NotOwned)
        );
        assert_eq!(
            consumer
                .resource(
                    scope.id(),
                    koushi_protocol::view::ViewRevision(revision.0 + 1),
                    &reference
                )
                .err(),
            Some(ScopeError::InvalidRevision)
        );
        assert!(
            consumer
                .resource(scope.id(), revision, "avatar/unknown")
                .unwrap()
                .is_none()
        );
        let delivery = hold_delivery.then_some(delivery);
        clear_renderable_thumbnail_cache();
        assert_eq!(
            *renderable_thumbnail_cache()
                .lock()
                .unwrap()
                .lease_bytes
                .lock()
                .unwrap(),
            16
        );
        assert_eq!(
            consumer
                .resource(scope.id(), revision, &reference)
                .unwrap()
                .unwrap()
                .content()
                .bytes,
            vec![7; 16]
        );
        let late_resources = vec![crate::timeline::ReaderAvatarResource {
            user_id: "@reader:example.test".into(),
            mxc_uri: "mxc://example.test/installed".into(),
            lease: Ok(consumer
                .resource(scope.id(), revision, &reference)
                .unwrap()
                .unwrap()),
        }];
        let other_scope = consumer.open().unwrap();
        _epoch.lock().unwrap().valid = false;
        assert_eq!(
            registry
                .publish_current(other_scope.id(), late_model, late_resources, &resolved)
                .err(),
            Some(ScopeError::SourceUnavailable)
        );
        consumer.retire();
        assert_eq!(
            consumer.resource(scope.id(), revision, &reference).err(),
            Some(ScopeError::Closed)
        );
        assert_eq!(
            *renderable_thumbnail_cache()
                .lock()
                .unwrap()
                .lease_bytes
                .lock()
                .unwrap(),
            if hold_delivery { 16 } else { 0 }
        );
        drop(delivery);
        assert_eq!(
            *renderable_thumbnail_cache()
                .lock()
                .unwrap()
                .lease_bytes
                .lock()
                .unwrap(),
            0
        );
    }
}

#[test]
fn charged_lease_survives_eviction_without_restoring_discoverability() {
    let mut cache = RenderableThumbnailCache::default();
    cache
        .insert("avatar/first".into(), vec![7; 1024], "image/png".into())
        .unwrap();
    let lease = cache.lease("avatar/first").unwrap();
    let clone = lease.clone();
    assert_eq!(*cache.lease_bytes.lock().unwrap(), 1024);
    for i in 0..MAX_RENDERABLE_THUMBNAIL_ENTRIES {
        cache
            .insert(format!("avatar/{i}"), vec![0], "image/png".into())
            .unwrap();
    }
    assert!(cache.get("avatar/first").is_none());
    assert_eq!(lease.content().bytes, vec![7; 1024]);
    cache.clear();
    assert!(cache.get("avatar/first").is_none());
    drop(lease);
    assert_eq!(*cache.lease_bytes.lock().unwrap(), 1024);
    drop(clone);
    assert_eq!(*cache.lease_bytes.lock().unwrap(), 0);
}

#[test]
fn independent_leases_are_conservatively_capped_without_releasing_existing_charge() {
    let mut cache = RenderableThumbnailCache::default();
    cache
        .insert(
            "avatar/large".into(),
            vec![0; MAX_RENDERABLE_THUMBNAIL_BYTES],
            "image/png".into(),
        )
        .unwrap();
    let leases: Vec<_> = (0..3)
        .map(|_| cache.lease("avatar/large").unwrap())
        .collect();
    assert_eq!(
        *cache.lease_bytes.lock().unwrap(),
        MAX_THUMBNAIL_LEASE_BYTES
    );
    assert_eq!(
        cache.lease("avatar/large").err(),
        Some(ThumbnailLeaseError::Capacity)
    );
    assert_eq!(
        *cache.lease_bytes.lock().unwrap(),
        MAX_THUMBNAIL_LEASE_BYTES
    );
    assert_eq!(
        cache.lease("missing").err(),
        Some(ThumbnailLeaseError::Unavailable)
    );
    drop(leases);
    assert_eq!(*cache.lease_bytes.lock().unwrap(), 0);
    assert!(cache.lease("avatar/large").is_ok());
}

#[test]
fn insertion_moves_owned_thumbnail_bytes_without_copying() {
    let mut cache = RenderableThumbnailCache::default();
    let bytes = vec![7_u8; 64 * 1024];
    let allocation = bytes.as_ptr();
    cache
        .insert("avatar/test".into(), bytes, "image/png".into())
        .unwrap();
    assert_eq!(cache.entries["avatar/test"].bytes.as_ptr(), allocation);
    assert_eq!(cache.stats().retained_bytes, 64 * 1024);
    assert_eq!(
        cache.get("avatar/test").unwrap().bytes,
        vec![7_u8; 64 * 1024]
    );
}

#[test]
fn stores_avatar_and_link_preview_thumbnails_with_opaque_refs() {
    let _guard = cache_test_lock();
    clear_renderable_thumbnail_cache();

    let avatar = store_renderable_thumbnail(
        RenderableThumbnailKind::Avatar,
        "mxc://example.test/avatar",
        b"avatar-bytes".to_vec(),
    )
    .expect("avatar bytes are within the cache bound");
    let link_preview = store_renderable_thumbnail(
        RenderableThumbnailKind::LinkPreview,
        "https://example.test/page",
        b"preview-bytes".to_vec(),
    )
    .expect("link-preview bytes are within the cache bound");

    let AvatarThumbnailState::Ready {
        source_ref,
        mime_type,
        ..
    } = avatar
    else {
        panic!("avatar thumbnail should be ready");
    };
    assert!(source_ref.starts_with("avatar/"));
    assert!(!source_ref.contains("://"));
    assert_eq!(mime_type.as_deref(), Some("application/octet-stream"));

    let AvatarThumbnailState::Ready {
        source_ref,
        mime_type,
        ..
    } = link_preview
    else {
        panic!("link-preview thumbnail should be ready");
    };
    assert!(source_ref.starts_with("link-preview/"));
    assert!(!source_ref.contains("://"));
    assert_eq!(mime_type.as_deref(), Some("application/octet-stream"));
}

#[test]
fn store_renderable_thumbnail_uses_media_owned_image_mime_detection() {
    let _guard = cache_test_lock();
    clear_renderable_thumbnail_cache();

    for (source, bytes, expected_mime) in [
        (
            "mxc://example.test/png",
            &b"\x89PNG\r\n\x1a\nrest"[..],
            "image/png",
        ),
        (
            "mxc://example.test/jpeg",
            &b"\xff\xd8\xff\xe0rest"[..],
            "image/jpeg",
        ),
    ] {
        let ready =
            store_renderable_thumbnail(RenderableThumbnailKind::Avatar, source, bytes.to_vec())
                .expect("image bytes are within the cache bound");
        let AvatarThumbnailState::Ready {
            source_ref,
            mime_type,
            ..
        } = ready
        else {
            panic!("image thumbnail should be ready");
        };
        assert_eq!(mime_type.as_deref(), Some(expected_mime));
        assert_eq!(
            lookup_renderable_thumbnail(&source_ref)
                .expect("image thumbnail should be cached")
                .mime_type
                .as_deref(),
            Some(expected_mime)
        );
    }
}

#[test]
fn lookup_renderable_thumbnail_returns_bytes_for_opaque_ref() {
    let _guard = cache_test_lock();
    clear_renderable_thumbnail_cache();

    let ready = store_renderable_thumbnail(
        RenderableThumbnailKind::Avatar,
        "mxc://example.test/lookup",
        b"lookup-bytes".to_vec(),
    )
    .expect("thumbnail bytes are within the cache bound");
    let AvatarThumbnailState::Ready { source_ref, .. } = ready else {
        panic!("thumbnail should be ready");
    };

    let content = lookup_renderable_thumbnail(&source_ref).expect("thumbnail should be cached");
    assert_eq!(content.bytes, b"lookup-bytes");
    assert_eq!(
        content.mime_type.as_deref(),
        Some("application/octet-stream")
    );
}

#[test]
fn ready_thumbnail_refs_survive_session_cache_churn() {
    let _guard = cache_test_lock();
    clear_renderable_thumbnail_cache();

    let ready = store_renderable_thumbnail(
        RenderableThumbnailKind::Avatar,
        "mxc://example.test/pinned",
        b"pinned-bytes".to_vec(),
    )
    .expect("pinned thumbnail bytes are within the cache bound");
    let AvatarThumbnailState::Ready { source_ref, .. } = ready else {
        panic!("thumbnail should be ready");
    };
    for index in 0..=128 {
        let source = format!("mxc://example.test/churn/{index}");
        let bytes = format!("bytes-{index}").into_bytes();
        store_renderable_thumbnail(RenderableThumbnailKind::Avatar, &source, bytes)
            .expect("churn thumbnail bytes are within the cache bound");
    }

    let content = lookup_renderable_thumbnail(&source_ref)
        .expect("Ready thumbnail ref must remain available until session clear");
    assert_eq!(content.bytes, b"pinned-bytes");
}

#[test]
fn lookup_rejects_uri_and_traversal_instead_of_parsing_adapter_schemes() {
    assert!(lookup_renderable_thumbnail("../avatar/0123456789abcdef").is_none());
    assert!(lookup_renderable_thumbnail("avatar/not-hex").is_none());
}

#[test]
fn thumbnail_cache_is_bounded_by_entry_count_and_retained_bytes() {
    let mut cache = RenderableThumbnailCache::default();
    for index in 0..=(MAX_RENDERABLE_THUMBNAIL_ENTRIES + 8) {
        cache
            .insert(
                format!("avatar/{index}"),
                vec![u8::try_from(index % 251).unwrap(); 1024],
                "image/png".to_owned(),
            )
            .expect("test entry is within the cache bound");
    }

    let stats = cache.stats();
    assert!(stats.entry_count <= MAX_RENDERABLE_THUMBNAIL_ENTRIES);
    assert!(stats.retained_bytes <= MAX_RENDERABLE_THUMBNAIL_BYTES);
    assert!(stats.eviction_count > 0);
    assert!(cache.get("avatar/0").is_none(), "oldest entry is evicted");
}

#[test]
fn oversized_thumbnail_is_rejected_without_publishing_a_ready_url() {
    let _guard = cache_test_lock();
    clear_renderable_thumbnail_cache();
    let rejection_count_before = renderable_thumbnail_cache_stats().oversize_rejection_count;

    let result = store_renderable_thumbnail(
        RenderableThumbnailKind::Avatar,
        "mxc://example.test/oversized",
        vec![0; MAX_RENDERABLE_THUMBNAIL_BYTES + 1],
    );

    assert_eq!(
        result,
        Err(RenderableThumbnailStoreError::TooLarge {
            byte_count: MAX_RENDERABLE_THUMBNAIL_BYTES + 1,
            max_bytes: MAX_RENDERABLE_THUMBNAIL_BYTES,
        })
    );
    let stats = renderable_thumbnail_cache_stats();
    assert_eq!(stats.entry_count, 0);
    assert_eq!(stats.retained_bytes, 0);
    assert_eq!(
        stats.oversize_rejection_count,
        rejection_count_before.saturating_add(1)
    );
}

#[test]
fn clear_renderable_thumbnail_cache_drops_previous_session_bytes() {
    let _guard = cache_test_lock();
    clear_renderable_thumbnail_cache();

    let ready = store_renderable_thumbnail(
        RenderableThumbnailKind::Avatar,
        "mxc://example.test/session-scoped",
        b"session-bytes".to_vec(),
    )
    .expect("thumbnail bytes are within the cache bound");
    let AvatarThumbnailState::Ready { source_ref, .. } = ready else {
        panic!("thumbnail should be ready");
    };
    assert!(lookup_renderable_thumbnail(&source_ref).is_some());

    clear_renderable_thumbnail_cache();

    assert!(lookup_renderable_thumbnail(&source_ref).is_none());
}

#[test]
fn cleanup_legacy_plaintext_thumbnail_dirs_preserves_media_downloads() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let data_dir = tempdir.path();

    fs::create_dir_all(data_dir.join("avatar_thumbnails")).expect("seed avatar dir");
    fs::write(
        data_dir.join("avatar_thumbnails").join("thumb.bin"),
        b"avatar",
    )
    .expect("seed avatar file");
    fs::create_dir_all(data_dir.join("link_preview_thumbnails")).expect("seed preview dir");
    fs::write(
        data_dir.join("link_preview_thumbnails").join("preview.bin"),
        b"preview",
    )
    .expect("seed preview file");
    fs::create_dir_all(data_dir.join("media_downloads")).expect("seed media dir");
    fs::write(
        data_dir.join("media_downloads").join("download.bin"),
        b"download",
    )
    .expect("seed download file");

    cleanup_legacy_plaintext_thumbnail_dirs(data_dir).expect("cleanup should succeed");

    assert!(!data_dir.join("avatar_thumbnails").exists());
    assert!(!data_dir.join("link_preview_thumbnails").exists());
    assert!(data_dir.join("media_downloads").exists());
    assert_eq!(
        fs::read(data_dir.join("media_downloads").join("download.bin")).expect("media file"),
        b"download"
    );
}
