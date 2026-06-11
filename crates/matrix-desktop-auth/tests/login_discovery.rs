use matrix_desktop_auth::parse_login_discovery;
use matrix_desktop_state::LoginFlowKind;

#[test]
fn parses_password_sso_and_token_flows() {
    let response = serde_json::json!({
        "flows": [
            { "type": "m.login.password" },
            {
                "type": "m.login.sso",
                "org.matrix.msc3824.delegated_oidc_compatibility": true
            },
            { "type": "m.login.token" }
        ]
    });

    let flows = parse_login_discovery(&response).expect("discovery should parse");

    assert_eq!(flows[0].kind, LoginFlowKind::Password);
    assert_eq!(flows[1].kind, LoginFlowKind::Sso);
    assert!(flows[1].delegated_oidc_compatibility);
    assert_eq!(flows[2].kind, LoginFlowKind::Token);
}

#[test]
fn keeps_unknown_flow_type_without_failing() {
    let response = serde_json::json!({
        "flows": [
            { "type": "com.example.login.custom" }
        ]
    });

    let flows = parse_login_discovery(&response).expect("unknown flow should parse");

    assert_eq!(
        flows[0].kind,
        LoginFlowKind::Unknown("com.example.login.custom".to_owned())
    );
}

#[test]
fn rejects_response_without_flows_array() {
    let response = serde_json::json!({
        "not_flows": []
    });

    let error = parse_login_discovery(&response).expect_err("missing flows should fail");

    assert_eq!(
        error.to_string(),
        "login discovery response is missing flows"
    );
}
