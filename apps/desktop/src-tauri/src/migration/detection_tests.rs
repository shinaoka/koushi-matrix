//! Detection answers off the platform: install paths, process output, and the
//! flag that keeps the Element Desktop import opt-in.

use super::element_desktop::{
    DetectionFacts, describe, import_flag_enabled, linux_install_path, tasklist_lists_element,
    windows_install_candidates,
};
use std::path::{Path, PathBuf};

fn facts(installed: bool, running: bool) -> DetectionFacts {
    DetectionFacts {
        installed,
        running,
        import_supported: true,
        import_enabled: true,
    }
}

#[test]
fn the_four_installed_and_running_combinations_are_reported_as_they_are() {
    let absent = describe(facts(false, false));
    assert!(!absent.installed && !absent.running);

    let stopped = describe(facts(true, false));
    assert!(stopped.installed && !stopped.running);
    assert_eq!(stopped.unavailable_reason, None);

    let running = describe(facts(true, true));
    assert!(running.installed && running.running);

    // A process without the application is not a source the UI may offer.
    let stray = describe(facts(false, true));
    assert!(!stray.installed && !stray.running);
}

#[test]
fn the_flag_gates_the_import_without_hiding_the_detection() {
    let off = describe(DetectionFacts {
        installed: true,
        running: false,
        import_supported: true,
        import_enabled: false,
    });
    assert!(off.installed);
    assert!(off.import_supported);
    assert!(!off.import_enabled);
    assert_eq!(
        off.unavailable_reason,
        Some("migration_element_desktop_import_disabled")
    );

    // Nothing installed is nothing to explain, flag or not.
    let nothing = describe(facts(false, false));
    assert_eq!(nothing.unavailable_reason, None);
}

#[test]
fn a_platform_without_the_cdp_export_says_so_instead_of_offering_it() {
    let unsupported = describe(DetectionFacts {
        installed: true,
        running: true,
        import_supported: false,
        import_enabled: true,
    });
    assert!(unsupported.installed && unsupported.running);
    assert!(!unsupported.import_supported);
    assert_eq!(
        unsupported.unavailable_reason,
        Some("migration_element_desktop_platform_unsupported")
    );
}

#[test]
fn only_the_exact_flag_value_enables_the_import() {
    assert!(import_flag_enabled(Some("1")));
    for value in [None, Some(""), Some("0"), Some("true"), Some("yes")] {
        assert!(!import_flag_enabled(value), "{value:?} must not enable it");
    }
}

#[test]
fn the_windows_candidates_cover_the_machine_and_user_installs() {
    let candidates = windows_install_candidates(Some(r"C:\Users\someone\AppData\Local"));
    assert_eq!(candidates.len(), 2);
    // The machine-wide path is written with Windows separators, so compare its
    // text rather than its components: this test runs on hosts where `\` is not
    // a separator and the whole path is one component.
    let machine = candidates[0].to_string_lossy().into_owned();
    assert!(machine.contains("Program Files"), "{machine}");
    assert!(machine.ends_with("Element.exe"), "{machine}");
    // The per-user path is built with `join`, so its components are comparable.
    let tail = candidates[1]
        .components()
        .rev()
        .take(3)
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(tail, ["Element.exe", "element-desktop", "Programs"]);
    // Without LOCALAPPDATA the machine-wide install is still worth checking.
    assert_eq!(windows_install_candidates(None).len(), 1);
}

#[test]
fn the_linux_install_path_takes_the_first_package_that_exists() {
    let exists = |path: &Path| path == Path::new("/opt/Element/element-desktop");
    assert_eq!(
        linux_install_path(exists),
        Some(PathBuf::from("/opt/Element/element-desktop"))
    );
    assert_eq!(linux_install_path(|_| false), None);
}

#[test]
fn the_disabled_import_reports_the_code_the_surface_maps() {
    // The UI hides the route, and a stale renderer's request still fails closed
    // with this code rather than starting a relaunch.
    assert_eq!(
        super::MigrationError::ImportDisabled.to_string(),
        "migration_element_desktop_import_disabled"
    );
}

#[test]
fn tasklist_output_is_read_for_element_and_nothing_else() {
    let listing = "\r\nImage Name                     PID Session Name        Session#    Mem Usage\r\n========================= ======== ================ =========== ============\r\nelement.exe                   8112 Console                    1    183,204 K\r\n";
    assert!(tasklist_lists_element(listing));
    assert!(tasklist_lists_element("ELEMENT.EXE 1"));
    assert!(!tasklist_lists_element(""));
    assert!(!tasklist_lists_element("explorer.exe 1 Console 1 80,000 K"));
}
