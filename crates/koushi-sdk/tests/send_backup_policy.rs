mod support;

#[test]
fn all_session_constructors_leave_the_per_send_backup_fence_disabled() {
    let sources = support::library_production_sources();
    assert_eq!(
        sources
            .iter()
            .map(|(_, source)| {
                source
                    .matches("require_secure_backup_for_encrypted_sends(false)")
                    .count()
            })
            .sum::<usize>(),
        3
    );
    assert!(sources.iter().all(|(_, source)| {
        !source.contains("require_secure_backup_for_encrypted_sends(true)")
    }));
}

#[test]
fn library_source_manifest_is_complete_and_unique() {
    let paths = support::library_production_sources()
        .iter()
        .map(|(path, _)| *path)
        .collect::<Vec<_>>();
    let mut unique = paths.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), paths.len());
    assert_eq!(
        unique,
        vec![
            "src/auth.rs",
            "src/client_session.rs",
            "src/e2ee.rs",
            "src/lib.rs",
            "src/profile.rs",
            "src/qa_reports.rs",
            "src/room_operations.rs",
            "src/room_projection.rs",
            "src/search.rs",
            "src/sliding_sync_discovery.rs",
            "src/sync.rs",
            "src/timeline.rs",
        ]
    );
}
