//! RED contract tests for the #699 local QA scenario.

use super::registry::{QaScenario, final_tokens_for_scenario, scenario_report};

#[test]
fn e2ee_login_store_is_a_behavioral_dedicated_route() {
    let orchestrator = super::contracts::orchestrator_source();
    let identity = super::contracts::identity_source();
    let route = orchestrator
        .split("if scenario == QaScenario::E2eeLoginStore")
        .nth(1)
        .expect("login-store scenario must have a dedicated early-return branch");
    assert!(route.contains("run_e2ee_login_store_scenario(&config).await?"));
    assert!(route.contains("return Ok(scenario_report(&config.server_kind, scenario))"));
    assert!(identity.contains("async fn run_e2ee_login_store_scenario"));
    assert!(identity.contains("ForceNewOutboundSession"));
    assert!(identity.contains("RestoreLastSession"));
    assert!(identity.contains("SoftLogoutReauth"));
    assert!(identity.contains("wait_for_item_with_body_or_decryption_failure"));
    assert!(identity.contains("cleanup_owned_e2ee_participant_best_effort"));
    assert!(
        !identity
            .split("async fn run_e2ee_login_store_scenario")
            .nth(1)
            .expect("scenario body")
            .split("async fn ")
            .next()
            .expect("scenario body boundary")
            .contains("tokio::time::sleep")
    );
}

#[test]
fn e2ee_login_store_parses_with_exact_private_safe_tokens() {
    let scenario = QaScenario::E2eeLoginStore;

    assert_eq!(QaScenario::from_env_value("e2ee_login_store"), Ok(scenario));
    assert_eq!(
        final_tokens_for_scenario(scenario),
        [
            "safety=ok",
            "e2ee_login_store_fresh_offline_index0=ok",
            "e2ee_login_store_restore_offline_index0=ok",
            "e2ee_login_store_restart_offline_index0=ok",
            "e2ee_login_store_reauth_offline_index0=ok",
            "e2ee_login_store_online_index0=ok",
            "e2ee_login_store_group_index0=ok",
            "e2ee_login_store_identity_stable=ok",
            "e2ee_login_store=ok",
        ]
    );

    let report = scenario_report("local", scenario);
    assert!(!report.contains('@'));
    assert!(!report.contains('!'));
    assert!(!report.contains('$'));
}
