//! `profile` ownership for AccountActor.

use std::{collections::BTreeSet, future::Future, sync::Arc, time::Duration};

use koushi_sdk::MatrixClientSession;
use koushi_state::{
    AppAction, AvatarImage, AvatarThumbnailFailureKind, AvatarThumbnailState, OwnProfile,
    PresenceKind,
};
use matrix_sdk::media::{MediaFormat, MediaRequestParameters};
use matrix_sdk::ruma::events::room::MediaSource as SdkMediaSource;
use matrix_sdk::ruma::{MxcUri, OwnedMxcUri};
use tokio::sync::Semaphore;

use crate::renderable_thumbnail::{
    RenderableThumbnailKind, clear_renderable_thumbnail_cache, store_renderable_thumbnail,
};
use crate::room::RoomMessage;
use crate::timeline::TimelineMessage;
use koushi_protocol::event::{AccountEvent, CoreEvent, LiveSignalsEvent};
use koushi_protocol::failure::{CoreFailure, ProfileFailureKind};
use koushi_protocol::ids::{AccountKey, RequestId};

use super::actor::{AccountActor, AccountMessage};

/// Maximum number of concurrent avatar thumbnail downloads. Bounded to avoid
/// flooding the SDK media layer with parallel requests during large room joins.
pub(super) const AVATAR_DOWNLOAD_CONCURRENCY: usize = 6;
const AVATAR_DOWNLOAD_MAX_ATTEMPTS: usize = 2;

const ACCOUNT_HYDRATION_TIMEOUT: Duration = Duration::from_secs(10);

async fn account_hydration_actions_from_session(
    session: &MatrixClientSession,
) -> (Vec<AppAction>, Option<BTreeSet<String>>) {
    let mut actions = Vec::new();
    let mut ignored_user_ids = None;

    if let Some(action) = own_profile_action_from_session(session).await {
        actions.push(action);
    }
    if let Some(action) = local_user_aliases_action_from_session(session).await {
        actions.push(action);
    }
    if let Some(action) = ignored_user_ids_action_from_session(session).await {
        if let AppAction::IgnoredUsersLoaded { ref user_ids } = action {
            ignored_user_ids = Some(user_ids.clone());
        }
        actions.push(action);
    }

    (actions, ignored_user_ids)
}

async fn own_profile_action_from_session(session: &MatrixClientSession) -> Option<AppAction> {
    crate::executor::timeout(
        ACCOUNT_HYDRATION_TIMEOUT,
        koushi_sdk::get_own_profile(session),
    )
    .await
    .ok()?
    .ok()
    .map(map_matrix_own_profile)
    .map(|profile| AppAction::OwnProfileUpdated { profile })
}

async fn local_user_aliases_action_from_session(
    session: &MatrixClientSession,
) -> Option<AppAction> {
    crate::executor::timeout(
        ACCOUNT_HYDRATION_TIMEOUT,
        koushi_sdk::get_local_user_aliases(session),
    )
    .await
    .ok()?
    .ok()
    .map(|aliases| AppAction::LocalUserAliasesLoaded {
        aliases: aliases.aliases,
    })
}

async fn ignored_user_ids_action_from_session(session: &MatrixClientSession) -> Option<AppAction> {
    crate::executor::timeout(
        ACCOUNT_HYDRATION_TIMEOUT,
        koushi_sdk::get_ignored_user_list(session),
    )
    .await
    .ok()?
    .ok()
    .map(|user_ids| AppAction::IgnoredUsersLoaded { user_ids })
}

fn map_matrix_own_profile(profile: koushi_sdk::MatrixOwnProfile) -> OwnProfile {
    OwnProfile {
        display_name: profile.display_name,
        avatar: profile.avatar_mxc_uri.map(|mxc_uri| AvatarImage {
            mxc_uri,
            thumbnail: AvatarThumbnailState::NotRequested,
        }),
    }
}

async fn retry_avatar_thumbnail_fetch<F, Fut>(
    mut fetch: F,
) -> Result<AvatarThumbnailState, AvatarThumbnailFailureKind>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<AvatarThumbnailState, AvatarThumbnailFailureKind>>,
{
    for attempt in 1..=AVATAR_DOWNLOAD_MAX_ATTEMPTS {
        match fetch().await {
            Err(AvatarThumbnailFailureKind::Network) if attempt < AVATAR_DOWNLOAD_MAX_ATTEMPTS => {}
            result => return result,
        }
    }
    unreachable!("avatar retry loop always returns on its final attempt")
}

fn avatar_thumbnail_for_request(
    thumbnail: &AvatarThumbnailState,
    request_id: RequestId,
) -> AvatarThumbnailState {
    match thumbnail {
        AvatarThumbnailState::Failed { kind, .. } => AvatarThumbnailState::Failed {
            request_id: request_id.sequence,
            kind: kind.clone(),
        },
        other => other.clone(),
    }
}

async fn download_avatar_thumbnail(
    session: &MatrixClientSession,
    mxc_uri: &str,
) -> Result<AvatarThumbnailState, AvatarThumbnailFailureKind> {
    let mxc = <&MxcUri>::from(mxc_uri);
    if !mxc.is_valid() {
        return Err(AvatarThumbnailFailureKind::Unsupported);
    }
    let uri: OwnedMxcUri = mxc.to_owned();
    let bytes = session
        .client()
        .media()
        .get_media_content(
            &MediaRequestParameters {
                source: SdkMediaSource::Plain(uri),
                format: MediaFormat::File,
            },
            true,
        )
        .await
        .map_err(|_| AvatarThumbnailFailureKind::Network)?;

    store_renderable_thumbnail(RenderableThumbnailKind::Avatar, mxc_uri, bytes)
        .map_err(|_| AvatarThumbnailFailureKind::Unsupported)
}

fn classify_profile_error(error: &koushi_sdk::MatrixProfileError) -> ProfileFailureKind {
    match error.failure_kind() {
        koushi_sdk::MatrixProfileFailureKind::Forbidden => ProfileFailureKind::Forbidden,
        koushi_sdk::MatrixProfileFailureKind::Network => ProfileFailureKind::Network,
        koushi_sdk::MatrixProfileFailureKind::InvalidMimeType => {
            ProfileFailureKind::InvalidMimeType
        }
        koushi_sdk::MatrixProfileFailureKind::Sdk => ProfileFailureKind::Sdk,
    }
}

fn classify_ignored_user_list_error(
    error: &koushi_sdk::MatrixIgnoredUserListError,
) -> koushi_protocol::failure::ReportFailureKind {
    use koushi_protocol::failure::ReportFailureKind;
    use koushi_sdk::MatrixIgnoredUserListFailureKind;
    match error.failure_kind() {
        MatrixIgnoredUserListFailureKind::Forbidden => ReportFailureKind::Forbidden,
        MatrixIgnoredUserListFailureKind::Network => ReportFailureKind::Network,
        MatrixIgnoredUserListFailureKind::InvalidUserId => ReportFailureKind::InvalidUserId,
        MatrixIgnoredUserListFailureKind::Sdk => ReportFailureKind::Sdk,
    }
}

impl AccountActor {
    pub(super) async fn handle_set_presence(&self, request_id: RequestId, presence: PresenceKind) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        let user_id = session.info.user_id.clone();
        let _ = self
            .action_tx
            .send(vec![AppAction::PresenceUpdated {
                user_id: user_id.clone(),
                presence,
            }])
            .await;
        self.emit(CoreEvent::LiveSignals(LiveSignalsEvent::PresenceSet {
            request_id,
            presence,
        }));
        self.emit(CoreEvent::LiveSignals(LiveSignalsEvent::PresenceUpdated {
            user_id,
            presence,
        }));
    }

    pub(super) async fn handle_set_display_name(
        &self,
        request_id: RequestId,
        display_name: Option<String>,
    ) {
        let Some(session) = &self.session else {
            self.send_actions(vec![AppAction::ProfileUpdateFailed {
                request_id: request_id.sequence,
                message: "profile update failed".to_owned(),
            }])
            .await;
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        match koushi_sdk::set_display_name(session, display_name.as_deref()).await {
            Ok(profile) => {
                let profile = map_matrix_own_profile(profile);
                self.send_actions(vec![AppAction::ProfileUpdateSucceeded {
                    request_id: request_id.sequence,
                    profile,
                }])
                .await;
                self.emit(CoreEvent::Account(AccountEvent::ProfileUpdated {
                    request_id,
                    account_key: AccountKey(session.info.user_id.clone()),
                }));
            }
            Err(error) => {
                self.send_actions(vec![AppAction::ProfileUpdateFailed {
                    request_id: request_id.sequence,
                    message: "profile update failed".to_owned(),
                }])
                .await;
                self.emit_failure(
                    request_id,
                    CoreFailure::ProfileOperationFailed {
                        kind: classify_profile_error(&error),
                    },
                );
            }
        }
    }

    pub(super) async fn handle_set_local_user_alias(
        &self,
        request_id: RequestId,
        user_id: String,
        alias: Option<String>,
    ) {
        let Some(session) = &self.session else {
            self.send_actions(vec![AppAction::LocalUserAliasUpdateFailed {
                request_id: request_id.sequence,
                message: "local alias update failed".to_owned(),
            }])
            .await;
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        match koushi_sdk::update_local_user_alias(session, &user_id, alias.as_deref()).await {
            Ok(aliases) => {
                let aliases = aliases.aliases;
                let _ = self
                    .room_actor
                    .send(RoomMessage::LocalUserAliasesUpdated {
                        aliases: aliases.clone(),
                    })
                    .await;
                self.send_actions(vec![
                    AppAction::LocalUserAliasUpdateSucceeded {
                        request_id: request_id.sequence,
                    },
                    AppAction::LocalUserAliasesLoaded { aliases },
                ])
                .await;
            }
            Err(error) => {
                if let Some(action) = local_user_aliases_action_from_session(session).await {
                    if let AppAction::LocalUserAliasesLoaded { aliases } = &action {
                        let _ = self
                            .room_actor
                            .send(RoomMessage::LocalUserAliasesUpdated {
                                aliases: aliases.clone(),
                            })
                            .await;
                    }
                    self.send_actions(vec![
                        AppAction::LocalUserAliasUpdateFailed {
                            request_id: request_id.sequence,
                            message: "local alias update failed".to_owned(),
                        },
                        action,
                    ])
                    .await;
                } else {
                    self.send_actions(vec![AppAction::LocalUserAliasUpdateFailed {
                        request_id: request_id.sequence,
                        message: "local alias update failed".to_owned(),
                    }])
                    .await;
                }
                self.emit_failure(
                    request_id,
                    CoreFailure::ProfileOperationFailed {
                        kind: classify_profile_error(&error),
                    },
                );
            }
        }
    }

    pub(super) async fn handle_set_avatar(
        &self,
        request_id: RequestId,
        request: koushi_protocol::command::SetAvatarRequest,
    ) {
        let Some(session) = &self.session else {
            self.send_actions(vec![AppAction::ProfileUpdateFailed {
                request_id: request_id.sequence,
                message: "profile update failed".to_owned(),
            }])
            .await;
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        match koushi_sdk::set_avatar(session, &request.mime_type, request.bytes).await {
            Ok(profile) => {
                let profile = map_matrix_own_profile(profile);
                self.send_actions(vec![AppAction::ProfileUpdateSucceeded {
                    request_id: request_id.sequence,
                    profile,
                }])
                .await;
                self.emit(CoreEvent::Account(AccountEvent::ProfileUpdated {
                    request_id,
                    account_key: AccountKey(session.info.user_id.clone()),
                }));
            }
            Err(error) => {
                self.send_actions(vec![AppAction::ProfileUpdateFailed {
                    request_id: request_id.sequence,
                    message: "profile update failed".to_owned(),
                }])
                .await;
                self.emit_failure(
                    request_id,
                    CoreFailure::ProfileOperationFailed {
                        kind: classify_profile_error(&error),
                    },
                );
            }
        }
    }

    /// Non-blocking, cache-first avatar thumbnail handler (Stage R1).
    ///
    /// 1. Cache hit (`Ready` or terminal `Failed`): emit immediately; no SDK call.
    /// 2. Already in-flight: return; the completing task will emit.
    /// 3. Otherwise: insert into `avatar_inflight`, spawn one bounded task that
    ///    owns at most two network attempts and posts `AvatarFetched` back.
    pub(super) async fn handle_download_avatar_thumbnail(
        &mut self,
        request_id: RequestId,
        mxc_uri: String,
    ) {
        // 1. Cache hit — Ready and terminal Failed states both settle without I/O.
        if let Some(cached) = self.avatar_cache.get(&mxc_uri) {
            let thumbnail = avatar_thumbnail_for_request(cached, request_id);
            self.send_actions(vec![AppAction::AvatarThumbnailUpdated {
                mxc_uri: mxc_uri.clone(),
                thumbnail: thumbnail.clone(),
            }])
            .await;
            self.emit(CoreEvent::Account(
                AccountEvent::AvatarThumbnailDownloaded {
                    request_id,
                    mxc_uri,
                    thumbnail,
                },
            ));
            return;
        }

        // 2. Single-flight dedup — a fetch is already running; record this
        //    request_id so the completing task will emit a terminal event for
        //    every waiter, then return without spawning a second task.
        if let Some(waiters) = self.avatar_inflight.get_mut(&mxc_uri) {
            waiters.push(request_id);
            return;
        }

        // 3. No session — emit failure synchronously rather than spawning.
        let Some(session) = self.session.clone() else {
            let thumbnail = AvatarThumbnailState::Failed {
                request_id: request_id.sequence,
                kind: AvatarThumbnailFailureKind::Sdk,
            };
            self.send_actions(vec![AppAction::AvatarThumbnailUpdated {
                mxc_uri: mxc_uri.clone(),
                thumbnail: thumbnail.clone(),
            }])
            .await;
            self.emit(CoreEvent::Account(
                AccountEvent::AvatarThumbnailDownloaded {
                    request_id,
                    mxc_uri,
                    thumbnail,
                },
            ));
            return;
        };

        // 4. Spawn a bounded fetch task; return immediately.
        // Record the originating request_id as the first waiter.
        self.avatar_inflight
            .insert(mxc_uri.clone(), vec![request_id]);
        let generation = self.avatar_session_generation;
        let semaphore = self.avatar_download_semaphore.clone();
        let tx = self.self_tx.clone();
        let mxc_uri_clone = mxc_uri.clone();

        self.avatar_fetch_tasks.spawn(async move {
            // Acquire a permit before hitting the SDK so at most
            // AVATAR_DOWNLOAD_CONCURRENCY fetches run concurrently.
            let _permit = semaphore.acquire().await;
            let thumbnail = retry_avatar_thumbnail_fetch(|| {
                download_avatar_thumbnail(&session, &mxc_uri_clone)
            })
            .await
            .unwrap_or_else(|kind| AvatarThumbnailState::Failed {
                request_id: request_id.sequence,
                kind,
            });
            // Best-effort: if the actor is already shut down, the send fails
            // silently — that is correct because the session is gone anyway.
            let _ = tx
                .send(AccountMessage::AvatarFetched {
                    mxc_uri: mxc_uri_clone,
                    generation,
                    thumbnail,
                })
                .await;
        });
    }

    /// Called when a spawned avatar-fetch task completes.  Updates the cache,
    /// removes the in-flight entry, and emits the same outputs as the old
    /// inline path (only the timing changed).
    ///
    /// Fix 1: stale-generation check — if `generation` does not match the
    /// current `avatar_session_generation` the completion belongs to a previous
    /// session; it is silently dropped.
    ///
    /// Fix 2: every waiter in the `avatar_inflight` Vec receives a terminal
    /// `AvatarThumbnailDownloaded` event; only one `AvatarThumbnailUpdated`
    /// action is reduced (one cache write).
    ///
    /// Fix 3: completed/aborted JoinSet entries are reaped non-blockingly at
    /// the start of each call so the JoinSet does not accumulate finished tasks.
    pub(super) async fn handle_avatar_fetched(
        &mut self,
        mxc_uri: String,
        generation: u64,
        thumbnail: AvatarThumbnailState,
    ) {
        // Fix 3: drain completed tasks non-blockingly so the JoinSet stays
        // bounded.  Only finished entries are removed; no async wait.
        self.reap_avatar_fetch_tasks();

        // Fix 1: drop stale completions from a prior session.
        if generation != self.avatar_session_generation {
            return;
        }

        // Remove and collect all waiting request_ids for this mxc.
        let waiters = self.avatar_inflight.remove(&mxc_uri).unwrap_or_default();

        // Cache the result so subsequent requests for the same URI are served
        // from memory. Ready and terminal Failed entries both settle duplicate
        // renderer demand without another SDK attempt; session clear resets them.
        self.avatar_cache.insert(mxc_uri.clone(), thumbnail.clone());

        // Emit one state-delta for the reducer (one cache write, regardless of
        // how many callers were waiting).
        self.send_actions(vec![AppAction::AvatarThumbnailUpdated {
            mxc_uri: mxc_uri.clone(),
            thumbnail: thumbnail.clone(),
        }])
        .await;

        // Fix 2: deliver a terminal event to every waiter. For a Failed
        // thumbnail, rebuild the payload with each waiter's own request_id so
        // the inner AvatarThumbnailState::Failed.request_id matches the outer
        // event request_id (the old inline path produced a per-request payload).
        for request_id in waiters {
            let per_waiter = avatar_thumbnail_for_request(&thumbnail, request_id);
            self.emit(CoreEvent::Account(
                AccountEvent::AvatarThumbnailDownloaded {
                    request_id,
                    mxc_uri: mxc_uri.clone(),
                    thumbnail: per_waiter,
                },
            ));
        }
    }

    /// Non-blocking reap of completed/aborted avatar-fetch JoinSet entries.
    /// Must not `.await`; called synchronously inside the actor message loop.
    fn reap_avatar_fetch_tasks(&mut self) {
        while self.avatar_fetch_tasks.try_join_next().is_some() {}
    }

    /// Abort all in-flight avatar fetch tasks and clear the per-session cache.
    /// Called on session clear (logout / account switch) and on shutdown.
    ///
    /// Fix 1: increment `avatar_session_generation` so that any `AvatarFetched`
    /// messages that were already queued before the abort are recognised as
    /// stale by `handle_avatar_fetched` and silently dropped.
    pub(super) fn abort_avatar_fetch_tasks(&mut self) {
        // Replace (drop) the JoinSet rather than only abort_all(): dropping a
        // JoinSet aborts all its tasks AND discards their entries, so cancelled
        // tasks do not linger across repeated request -> session-clear cycles.
        self.avatar_fetch_tasks = tokio::task::JoinSet::new();
        self.avatar_inflight.clear();
        self.avatar_cache.clear();
        clear_renderable_thumbnail_cache();
        // Replace the semaphore so any task that manages to run after abort
        // cannot accidentally re-use a poisoned permit count.
        self.avatar_download_semaphore = Arc::new(Semaphore::new(AVATAR_DOWNLOAD_CONCURRENCY));
        // Advance the generation counter so stale completions from tasks that
        // were spawned before this abort are silently rejected.
        self.avatar_session_generation = self.avatar_session_generation.wrapping_add(1);
    }

    pub(super) fn spawn_account_hydration(&mut self, session: Arc<MatrixClientSession>) {
        self.invalidate_account_hydration();
        let generation = self.account_hydration_generation;
        let self_tx = self.self_tx.clone();
        self.account_hydration_task = Some(crate::executor::spawn(async move {
            let (actions, ignored_user_ids) =
                account_hydration_actions_from_session(&session).await;
            if actions.is_empty() {
                return;
            }
            let _ = self_tx
                .send(AccountMessage::AccountHydrationLoaded {
                    generation,
                    actions,
                    ignored_user_ids,
                })
                .await;
        }));
    }

    pub(super) fn invalidate_account_hydration(&mut self) {
        self.account_hydration_generation = self.account_hydration_generation.wrapping_add(1);
        if let Some(task) = self.account_hydration_task.take() {
            task.abort();
        }
    }

    pub(super) async fn handle_account_hydration_loaded(
        &mut self,
        generation: u64,
        actions: Vec<AppAction>,
        ignored_user_ids: Option<BTreeSet<String>>,
    ) {
        if generation != self.account_hydration_generation || self.session.is_none() {
            return;
        }
        self.account_hydration_task = None;
        if let Some(user_ids) = ignored_user_ids {
            let _ = self
                .timeline_manager
                .send(TimelineMessage::IgnoredUsersUpdated { user_ids })
                .await;
        }
        if let Some(aliases) = actions.iter().find_map(|action| match action {
            AppAction::LocalUserAliasesLoaded { aliases } => Some(aliases.clone()),
            _ => None,
        }) {
            let _ = self
                .room_actor
                .send(RoomMessage::LocalUserAliasesUpdated { aliases })
                .await;
        }
        self.send_actions(actions).await;
    }

    pub(super) async fn handle_ignore_user(
        &mut self,
        request_id: RequestId,
        user_id: String,
        ignored: bool,
    ) {
        let Some(session) = &self.session else {
            self.send_actions(vec![AppAction::IgnoredUserUpdateFailed {
                request_id: request_id.sequence,
                user_id: user_id.clone(),
                ignored,
                message: "ignored user update failed".to_owned(),
            }])
            .await;
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        self.send_actions(vec![AppAction::IgnoredUserUpdateRequested {
            request_id: request_id.sequence,
            user_id: user_id.clone(),
            ignored,
        }])
        .await;

        let result = if ignored {
            koushi_sdk::ignore_user(session, &user_id).await
        } else {
            koushi_sdk::unignore_user(session, &user_id).await
        };

        match result {
            Ok(user_ids) => {
                self.send_actions(vec![
                    AppAction::IgnoredUserUpdateSucceeded {
                        request_id: request_id.sequence,
                    },
                    AppAction::IgnoredUsersLoaded {
                        user_ids: user_ids.clone(),
                    },
                ])
                .await;
                let _ = self
                    .timeline_manager
                    .send(TimelineMessage::IgnoredUsersUpdated { user_ids })
                    .await;
            }
            Err(error) => {
                // Reconcile with server state so the optimistic reducer update
                // does not drift after a failure.
                if let Some(action) = ignored_user_ids_action_from_session(session).await {
                    if let AppAction::IgnoredUsersLoaded { ref user_ids } = action {
                        let _ = self
                            .timeline_manager
                            .send(TimelineMessage::IgnoredUsersUpdated {
                                user_ids: user_ids.clone(),
                            })
                            .await;
                    }
                    self.send_actions(vec![
                        AppAction::IgnoredUserUpdateFailed {
                            request_id: request_id.sequence,
                            user_id: user_id.clone(),
                            ignored,
                            message: "ignored user update failed".to_owned(),
                        },
                        action,
                    ])
                    .await;
                } else {
                    self.send_actions(vec![AppAction::IgnoredUserUpdateFailed {
                        request_id: request_id.sequence,
                        user_id: user_id.clone(),
                        ignored,
                        message: "ignored user update failed".to_owned(),
                    }])
                    .await;
                }
                self.emit_failure(
                    request_id,
                    CoreFailure::ReportOperationFailed {
                        kind: classify_ignored_user_list_error(&error),
                    },
                );
            }
        }
    }

    pub(super) async fn handle_report_user(
        &mut self,
        request_id: RequestId,
        user_id: String,
        reason: String,
    ) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        match koushi_sdk::report_user(session, &user_id, reason).await {
            Ok(()) => {
                self.emit(CoreEvent::Account(AccountEvent::ReportCompleted {
                    request_id,
                    kind: koushi_protocol::event::ReportKind::User,
                }));
            }
            Err(error) => {
                self.emit_failure(
                    request_id,
                    CoreFailure::ReportOperationFailed {
                        kind: crate::report::classify_report_error(&error),
                    },
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use koushi_sdk::{MatrixClientSession, PersistableMatrixSession};
    use koushi_state::{AvatarThumbnailFailureKind, AvatarThumbnailState};

    use super::{
        avatar_thumbnail_for_request, download_avatar_thumbnail, retry_avatar_thumbnail_fetch,
    };

    use crate::renderable_thumbnail::clear_renderable_thumbnail_cache;
    use koushi_protocol::ids::{RequestId, RuntimeConnectionId};

    use matrix_sdk::test_utils::mocks::MatrixMockServer;
    use std::{fs, path::Path};
    use tempfile::tempdir;

    fn ready_thumbnail() -> AvatarThumbnailState {
        AvatarThumbnailState::Ready {
            source_ref: "avatar/0123456789abcdef".to_owned(),
            width: None,
            height: None,
            mime_type: Some("image/png".to_owned()),
        }
    }

    #[tokio::test]
    async fn avatar_fetch_retries_one_network_failure_inside_core() {
        let mut calls = 0;
        let result = retry_avatar_thumbnail_fetch(|| {
            calls += 1;
            std::future::ready(if calls == 1 {
                Err(AvatarThumbnailFailureKind::Network)
            } else {
                Ok(ready_thumbnail())
            })
        })
        .await;

        assert!(matches!(result, Ok(AvatarThumbnailState::Ready { .. })));
        assert_eq!(calls, 2);
    }

    #[tokio::test]
    async fn avatar_fetch_exhaustion_is_terminal_after_two_attempts() {
        let mut calls = 0;
        let result = retry_avatar_thumbnail_fetch(|| {
            calls += 1;
            std::future::ready(Err(AvatarThumbnailFailureKind::Network))
        })
        .await;

        assert_eq!(result, Err(AvatarThumbnailFailureKind::Network));
        assert_eq!(calls, 2);
    }

    #[test]
    fn cached_avatar_failure_is_recorrelated_without_another_fetch() {
        let request_id = RequestId {
            connection_id: RuntimeConnectionId(7),
            sequence: 42,
        };
        let cached = AvatarThumbnailState::Failed {
            request_id: 3,
            kind: AvatarThumbnailFailureKind::Network,
        };
        assert_eq!(
            avatar_thumbnail_for_request(&cached, request_id),
            AvatarThumbnailState::Failed {
                request_id: 42,
                kind: AvatarThumbnailFailureKind::Network,
            }
        );
    }

    async fn restore_media_test_session(
        server: &MatrixMockServer,
        data_dir: &Path,
    ) -> MatrixClientSession {
        let persisted = PersistableMatrixSession::from_json(
            &serde_json::json!({
                "homeserver": server.uri(),
                "access_token": "1234",
                "device_id": "AVATARCACHEDEVICE",
                "user_id": "@avatar-cache:localhost"
            })
            .to_string(),
        )
        .expect("synthetic Matrix session");
        let store_config = koushi_sdk::MatrixClientStoreConfig::new(
            data_dir.join("matrix-store"),
            koushi_sdk::MatrixClientStoreKey::new([41; 32]),
        )
        .with_cache_path(data_dir.join("matrix-cache"));

        koushi_sdk::restore_session_with_store(&persisted, Some(&store_config))
            .await
            .expect("restore media test session")
    }

    fn assert_directory_does_not_contain_plaintext(root: &Path, plaintext: &[u8]) {
        let mut pending = vec![root.to_path_buf()];
        while let Some(path) = pending.pop() {
            for entry in fs::read_dir(path).expect("read test store directory") {
                let entry = entry.expect("read test store entry");
                let file_type = entry.file_type().expect("read test store entry type");
                if file_type.is_dir() {
                    pending.push(entry.path());
                } else if file_type.is_file() {
                    let bytes = fs::read(entry.path()).expect("read test store file");
                    assert!(
                        !bytes
                            .windows(plaintext.len())
                            .any(|window| window == plaintext),
                        "keyed SDK media store must not persist renderable avatar plaintext"
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn avatar_download_survives_restart_and_offline_via_keyed_sdk_media_store() {
        let server = MatrixMockServer::new().await;
        server.mock_versions().ok().mount().await;
        server
            .mock_authed_media_download()
            .ok_image()
            .named("avatar fetched from network exactly once")
            .expect(1)
            .mount()
            .await;
        let data_dir = tempdir().expect("data tempdir");
        let mxc_uri = "mxc://localhost/persisted-avatar";

        let online_session = restore_media_test_session(&server, data_dir.path()).await;
        let online = download_avatar_thumbnail(&online_session, mxc_uri)
            .await
            .expect("online avatar fetch");
        let AvatarThumbnailState::Ready { source_ref, .. } = online else {
            panic!("online avatar should be ready");
        };
        assert!(source_ref.starts_with("avatar/"));
        assert!(!source_ref.contains("://"));
        drop(online_session);
        clear_renderable_thumbnail_cache();

        let offline_session = restore_media_test_session(&server, data_dir.path()).await;
        let offline = download_avatar_thumbnail(&offline_session, mxc_uri)
            .await
            .expect("cached avatar should load without a second network request");
        let AvatarThumbnailState::Ready { source_ref, .. } = offline else {
            panic!("offline cached avatar should be ready");
        };
        assert!(source_ref.starts_with("avatar/"));
        assert!(!source_ref.contains("://"));
        assert!(!data_dir.path().join("avatar_thumbnails").exists());
        assert_directory_does_not_contain_plaintext(data_dir.path(), b"binaryjpegfullimagedata");
    }

    #[tokio::test]
    async fn uncached_avatar_offline_preserves_network_failure() {
        let server = MatrixMockServer::new().await;
        server.mock_versions().ok().mount().await;
        let data_dir = tempdir().expect("data tempdir");
        let session = restore_media_test_session(&server, data_dir.path()).await;

        assert_eq!(
            download_avatar_thumbnail(&session, "mxc://localhost/uncached-avatar").await,
            Err(AvatarThumbnailFailureKind::Network)
        );
        assert!(!data_dir.path().join("avatar_thumbnails").exists());
    }
}
