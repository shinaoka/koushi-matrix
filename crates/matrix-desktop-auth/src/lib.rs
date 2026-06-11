use matrix_desktop_state::{LoginFlow, LoginFlowKind};
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum LoginDiscoveryError {
    #[error("login discovery response is missing flows")]
    MissingFlows,
    #[error("login discovery response is invalid: {0}")]
    InvalidResponse(String),
}

#[derive(Deserialize)]
struct LoginDiscoveryResponse {
    flows: Vec<RawLoginFlow>,
}

#[derive(Deserialize)]
struct RawLoginFlow {
    #[serde(rename = "type")]
    flow_type: String,
    #[serde(default, rename = "org.matrix.msc3824.delegated_oidc_compatibility")]
    delegated_oidc_compatibility: bool,
}

pub fn parse_login_discovery(
    value: &serde_json::Value,
) -> Result<Vec<LoginFlow>, LoginDiscoveryError> {
    if !value.get("flows").is_some_and(serde_json::Value::is_array) {
        return Err(LoginDiscoveryError::MissingFlows);
    }

    let response = serde_json::from_value::<LoginDiscoveryResponse>(value.clone())
        .map_err(|error| LoginDiscoveryError::InvalidResponse(error.to_string()))?;

    Ok(response
        .flows
        .into_iter()
        .map(|flow| LoginFlow {
            kind: parse_flow_kind(flow.flow_type),
            delegated_oidc_compatibility: flow.delegated_oidc_compatibility,
        })
        .collect())
}

fn parse_flow_kind(flow_type: String) -> LoginFlowKind {
    match flow_type.as_str() {
        "m.login.password" => LoginFlowKind::Password,
        "m.login.sso" => LoginFlowKind::Sso,
        "m.login.token" => LoginFlowKind::Token,
        _ => LoginFlowKind::Unknown(flow_type),
    }
}
