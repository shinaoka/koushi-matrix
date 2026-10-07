use crate::{
    AppAction, AppEffect, AppState, SessionState, SlidingSyncAdmission, SlidingSyncAdmissionSource,
    SlidingSyncCapabilityFailureKind, SlidingSyncCapabilityResult, SlidingSyncCapabilityState,
    SlidingSyncPositiveEvidence, SlidingSyncRevalidationState, SyncState, UiEvent,
};

fn same_session_identity(left: &crate::SessionInfo, right: &crate::SessionInfo) -> bool {
    left.homeserver == right.homeserver
        && left.user_id == right.user_id
        && left.device_id == right.device_id
}

fn initial_admission_matches_session(state: &AppState, admission: &SlidingSyncAdmission) -> bool {
    match (admission, &state.session) {
        (
            SlidingSyncAdmission::NewLogin { attempt_id },
            SessionState::Authenticating {
                attempt_id: active, ..
            },
        ) => attempt_id == active,
        (SlidingSyncAdmission::StoredSessionRestore { .. }, SessionState::Restoring) => true,
        (
            SlidingSyncAdmission::StoredSessionRestore { info },
            SessionState::SwitchingAccount { info: target },
        ) => same_session_identity(info, target),
        _ => false,
    }
}

fn blocked_admission_matches_session(state: &AppState, admission: &SlidingSyncAdmission) -> bool {
    match (admission, &state.session) {
        (
            SlidingSyncAdmission::NewLogin { attempt_id },
            SessionState::Authenticating {
                attempt_id: active, ..
            },
        ) => attempt_id == active,
        (
            SlidingSyncAdmission::StoredSessionRestore { info },
            SessionState::CapabilityBlocked { info: active, .. },
        ) => info == active,
        _ => false,
    }
}

fn revalidation_session_matches(state: &AppState, admission: &SlidingSyncAdmission) -> bool {
    let SlidingSyncAdmission::StoredSessionRestore { info } = admission else {
        return false;
    };
    match &state.session {
        SessionState::Ready(active) | SessionState::Locked(active) => info == active,
        _ => false,
    }
}

pub(crate) fn handle_started(
    state: &mut AppState,
    account_epoch: u64,
    request_id: u64,
    admission: SlidingSyncAdmission,
    positive_evidence: Option<SlidingSyncPositiveEvidence>,
) -> Vec<AppEffect> {
    if !matches!(
        state.sliding_sync_capability,
        SlidingSyncCapabilityState::Unknown
    ) || account_epoch <= state.sliding_sync_account_epoch
        || !initial_admission_matches_session(state, &admission)
    {
        return Vec::new();
    }
    state.sliding_sync_account_epoch = account_epoch;
    state.sliding_sync_capability = SlidingSyncCapabilityState::Checking {
        account_epoch,
        request_id,
        admission,
        positive_evidence,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

fn failure_for_result(
    result: &SlidingSyncCapabilityResult,
) -> Option<SlidingSyncCapabilityFailureKind> {
    match result {
        SlidingSyncCapabilityResult::Supported { .. } => None,
        SlidingSyncCapabilityResult::Unsupported => {
            Some(SlidingSyncCapabilityFailureKind::Unsupported)
        }
        SlidingSyncCapabilityResult::Unreachable => {
            Some(SlidingSyncCapabilityFailureKind::Unreachable)
        }
        SlidingSyncCapabilityResult::InvalidResponse => {
            Some(SlidingSyncCapabilityFailureKind::InvalidResponse)
        }
    }
}

pub(crate) fn handle_completed(
    state: &mut AppState,
    account_epoch: u64,
    request_id: u64,
    result: SlidingSyncCapabilityResult,
) -> Vec<AppEffect> {
    let SlidingSyncCapabilityState::Checking {
        account_epoch: active_epoch,
        request_id: active_request_id,
        admission,
        positive_evidence,
    } = &state.sliding_sync_capability
    else {
        return Vec::new();
    };
    if *active_epoch != account_epoch
        || *active_request_id != request_id
        || state.sliding_sync_account_epoch != account_epoch
        || !initial_admission_matches_session(state, admission)
    {
        return Vec::new();
    }
    let admission = admission.clone();
    let positive_evidence = positive_evidence.clone();
    let admission_kind = admission.kind();

    if let SlidingSyncCapabilityResult::Supported { evidence } = result {
        state.sliding_sync_capability = SlidingSyncCapabilityState::Supported {
            account_epoch,
            request_id,
            admission,
            evidence,
            revalidation: SlidingSyncRevalidationState::NotRequired,
        };
        return vec![
            AppEffect::ContinueSlidingSyncAdmission {
                account_epoch,
                request_id,
                admission: admission_kind,
                source: SlidingSyncAdmissionSource::Network,
            },
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
        ];
    }

    let failure = failure_for_result(&result).expect("non-supported result has a failure");
    if matches!(
        failure,
        SlidingSyncCapabilityFailureKind::Unreachable
            | SlidingSyncCapabilityFailureKind::InvalidResponse
    ) && matches!(admission, SlidingSyncAdmission::StoredSessionRestore { .. })
        && let Some(evidence) = positive_evidence.clone()
    {
        state.sliding_sync_capability = SlidingSyncCapabilityState::Supported {
            account_epoch,
            request_id,
            admission,
            evidence,
            revalidation: SlidingSyncRevalidationState::Pending { failure },
        };
        return vec![
            AppEffect::ContinueSlidingSyncAdmission {
                account_epoch,
                request_id,
                admission: admission_kind,
                source: SlidingSyncAdmissionSource::PositiveCache,
            },
            AppEffect::ScheduleSlidingSyncCapabilityRevalidation { account_epoch },
            AppEffect::EmitUiEvent(UiEvent::SessionChanged),
        ];
    }

    if let SlidingSyncAdmission::StoredSessionRestore { info } = &admission {
        state.session = SessionState::CapabilityBlocked {
            info: info.clone(),
            failure,
        };
        state.session_lock_reason = None;
    }
    state.sliding_sync_capability = SlidingSyncCapabilityState::Blocked {
        account_epoch,
        request_id,
        admission,
        failure,
        positive_evidence,
    };
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

pub(crate) fn handle_retry(
    state: &mut AppState,
    account_epoch: u64,
    blocked_request_id: u64,
    request_id: u64,
) -> Vec<AppEffect> {
    let SlidingSyncCapabilityState::Blocked {
        account_epoch: active_epoch,
        request_id: active_request_id,
        admission,
        positive_evidence,
        ..
    } = &state.sliding_sync_capability
    else {
        return Vec::new();
    };
    if *active_epoch != account_epoch
        || state.sliding_sync_account_epoch != account_epoch
        || *active_request_id != blocked_request_id
        || request_id <= blocked_request_id
        || !blocked_admission_matches_session(state, admission)
    {
        return Vec::new();
    }
    let admission = admission.clone();
    let positive_evidence = positive_evidence.clone();
    if matches!(admission, SlidingSyncAdmission::StoredSessionRestore { .. }) {
        state.session = SessionState::Restoring;
        state.session_lock_reason = None;
    }
    state.sliding_sync_capability = SlidingSyncCapabilityState::Checking {
        account_epoch,
        request_id,
        admission,
        positive_evidence,
    };
    vec![
        AppEffect::RetrySlidingSyncCapabilityDiscovery {
            account_epoch,
            blocked_request_id,
            request_id,
        },
        AppEffect::EmitUiEvent(UiEvent::SessionChanged),
    ]
}

pub(crate) fn handle_revalidation_started(
    state: &mut AppState,
    account_epoch: u64,
    request_id: u64,
) -> Vec<AppEffect> {
    let SlidingSyncCapabilityState::Supported {
        account_epoch: active_epoch,
        request_id: latest_request_id,
        admission,
        revalidation: SlidingSyncRevalidationState::Pending { .. },
        ..
    } = &state.sliding_sync_capability
    else {
        return Vec::new();
    };
    if *active_epoch != account_epoch
        || state.sliding_sync_account_epoch != account_epoch
        || request_id <= *latest_request_id
        || !matches!(state.session, SessionState::Ready(_))
        || !revalidation_session_matches(state, admission)
    {
        return Vec::new();
    }
    let SlidingSyncCapabilityState::Supported { revalidation, .. } =
        &mut state.sliding_sync_capability
    else {
        unreachable!("supported state checked above");
    };
    *revalidation = SlidingSyncRevalidationState::Checking { request_id };
    vec![AppEffect::EmitUiEvent(UiEvent::SessionChanged)]
}

pub(crate) fn handle_revalidation_completed(
    state: &mut AppState,
    account_epoch: u64,
    request_id: u64,
    result: SlidingSyncCapabilityResult,
) -> Vec<AppEffect> {
    let SlidingSyncCapabilityState::Supported {
        account_epoch: active_epoch,
        request_id: latest_request_id,
        admission,
        evidence,
        revalidation:
            SlidingSyncRevalidationState::Checking {
                request_id: active_request_id,
            },
    } = &state.sliding_sync_capability
    else {
        return Vec::new();
    };
    if *active_epoch != account_epoch
        || state.sliding_sync_account_epoch != account_epoch
        || *active_request_id != request_id
        || request_id <= *latest_request_id
        || !revalidation_session_matches(state, admission)
    {
        return Vec::new();
    }
    let admission = admission.clone();
    let prior_evidence = evidence.clone();
    match result {
        SlidingSyncCapabilityResult::Supported { evidence } => {
            let result = SlidingSyncCapabilityResult::Supported {
                evidence: evidence.clone(),
            };
            state.sliding_sync_capability = SlidingSyncCapabilityState::Supported {
                account_epoch,
                request_id,
                admission,
                evidence,
                revalidation: SlidingSyncRevalidationState::NotRequired,
            };
            vec![
                AppEffect::SettleSlidingSyncCapabilityRevalidation {
                    account_epoch,
                    request_id,
                    result,
                },
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            ]
        }
        SlidingSyncCapabilityResult::Unsupported => {
            let SlidingSyncAdmission::StoredSessionRestore { info } = &admission else {
                return Vec::new();
            };
            state.session = SessionState::CapabilityBlocked {
                info: info.clone(),
                failure: SlidingSyncCapabilityFailureKind::Unsupported,
            };
            state.session_lock_reason = None;
            state.sync = SyncState::Stopped;
            state.sliding_sync_capability = SlidingSyncCapabilityState::Blocked {
                account_epoch,
                request_id,
                admission,
                failure: SlidingSyncCapabilityFailureKind::Unsupported,
                positive_evidence: Some(prior_evidence),
            };
            let mut effects = vec![
                AppEffect::SettleSlidingSyncCapabilityRevalidation {
                    account_epoch,
                    request_id,
                    result: SlidingSyncCapabilityResult::Unsupported,
                },
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            ];
            // #1159: a blocked capability retirement also leaves the ready
            // session, so withdraw the scheduled-send durability notice.
            if let Some(effect) = super::withdraw_scheduled_send_persistence_failure(state) {
                effects.push(effect);
            }
            effects
        }
        SlidingSyncCapabilityResult::Unreachable | SlidingSyncCapabilityResult::InvalidResponse => {
            let failure = failure_for_result(&result).expect("retryable result has a failure");
            state.sliding_sync_capability = SlidingSyncCapabilityState::Supported {
                account_epoch,
                request_id,
                admission,
                evidence: prior_evidence,
                revalidation: SlidingSyncRevalidationState::Pending { failure },
            };
            vec![
                AppEffect::SettleSlidingSyncCapabilityRevalidation {
                    account_epoch,
                    request_id,
                    result,
                },
                AppEffect::EmitUiEvent(UiEvent::SessionChanged),
            ]
        }
    }
}

pub(crate) fn reduce(state: &mut AppState, action: AppAction) -> Vec<AppEffect> {
    match action {
        AppAction::SlidingSyncCapabilityCheckStarted {
            account_epoch,
            request_id,
            admission,
            positive_evidence,
        } => handle_started(
            state,
            account_epoch,
            request_id,
            admission,
            positive_evidence,
        ),
        AppAction::SlidingSyncCapabilityCheckCompleted {
            account_epoch,
            request_id,
            result,
        } => handle_completed(state, account_epoch, request_id, result),
        AppAction::SlidingSyncCapabilityRetryAccepted {
            account_epoch,
            blocked_request_id,
            request_id,
        } => handle_retry(state, account_epoch, blocked_request_id, request_id),
        AppAction::SlidingSyncCapabilityRevalidationStarted {
            account_epoch,
            request_id,
        } => handle_revalidation_started(state, account_epoch, request_id),
        AppAction::SlidingSyncCapabilityRevalidationCompleted {
            account_epoch,
            request_id,
            result,
        } => handle_revalidation_completed(state, account_epoch, request_id, result),
        _ => Vec::new(),
    }
}
