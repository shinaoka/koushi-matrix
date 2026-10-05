//! Packager opt-out (#1063): a marker file installed by a distribution package
//! keeps the updater `Unsupported`, unselected, and free of update requests.

use super::*;
use std::cell::Cell;

fn policy(generation: u64, auto_check: bool, include_prereleases: bool) -> PolicySnapshot {
    PolicySnapshot {
        generation,
        settings: UpdatesSettings {
            auto_check,
            include_prereleases,
        },
    }
}

const PACKAGE_MANAGED: DesktopUpdateState = DesktopUpdateState::Unsupported {
    reason: DesktopUpdateUnsupportedReason::PackageManaged,
};

#[test]
fn package_managed_marker_is_unsupported_unselected_and_requests_nothing() {
    let root = tempfile::tempdir().expect("temporary marker root");
    let marker = root.path().join("package-managed");
    std::fs::write(&marker, b"").expect("write synthetic marker");

    let state = initial_state_for(Some(&marker));
    assert_eq!(state, PACKAGE_MANAGED);

    // The backend constructor is never reached, so no install backend exists
    // to spawn pkexec, sudo, dpkg, or rpm.
    let constructed = Cell::new(false);
    let backend = select_backend(&state, || {
        constructed.set(true);
        Some(())
    });
    assert!(backend.is_none());
    assert!(!constructed.get());

    // Every trigger (startup policy with auto-check, manual check, channel
    // change, download, install) is refused and no work is ever claimed.
    let mut lifecycle = Lifecycle::<()>::new(state);
    lifecycle.observe(policy(0, true, false));
    assert!(lifecycle.claim_work().is_none());
    assert!(!lifecycle.request_check(policy(1, true, false)));
    assert!(lifecycle.observe(policy(2, true, true)));
    assert_eq!(
        lifecycle.request_download(policy(3, true, true), 0),
        Err(())
    );
    assert_eq!(lifecycle.begin_install(), Err(()));
    assert!(lifecycle.claim_work().is_none());
    assert_eq!(lifecycle.state, PACKAGE_MANAGED);
}

#[test]
fn absent_marker_keeps_the_build_capability() {
    let root = tempfile::tempdir().expect("temporary marker root");
    let marker = root.path().join("package-managed");

    assert_eq!(initial_state_for(Some(&marker)), initial_state_for(None));
    assert_ne!(initial_state_for(None), PACKAGE_MANAGED);
}

#[test]
fn package_managed_state_serializes_its_reason() {
    assert_eq!(
        serde_json::to_value(PACKAGE_MANAGED).unwrap(),
        serde_json::json!({ "kind": "unsupported", "reason": "package_managed" })
    );
}

#[test]
fn only_linux_probes_the_documented_marker_path() {
    let expected =
        cfg!(target_os = "linux").then(|| Path::new("/usr/share/koushi-desktop/package-managed"));
    assert_eq!(package_managed_marker(), expected);
}
