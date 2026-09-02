use super::actor::RoomActor;
use super::operations::{classify_room_error, operation_failure_kind};
use koushi_protocol::event::{CoreEvent, RoomEvent};
use koushi_protocol::failure::{CoreFailure, RoomFailureKind};
use koushi_protocol::ids::RequestId;
use koushi_sdk::{
    MatrixRoomHistoryVisibility, MatrixRoomJoinRule, MatrixRoomMemberRole,
    MatrixRoomMemberRoleOption, MatrixRoomMemberSummary, MatrixRoomModerationAction,
    MatrixRoomPermissionFacts, MatrixRoomSettingChange, MatrixRoomSettingsSnapshot,
    MatrixUserTrustState,
};
use koushi_state::{
    AppAction, RoomHistoryVisibility, RoomJoinRule, RoomMemberRole, RoomMemberRoleOption,
    RoomMemberSummary, RoomModerationAction, RoomPermissionFacts, RoomSettingChange,
    RoomSettingsSnapshot, UserTrustState,
};

fn room_settings_snapshot_from_sdk(settings: MatrixRoomSettingsSnapshot) -> RoomSettingsSnapshot {
    let share_link = koushi_state::room_settings_share_link(
        &settings.room_id,
        settings.canonical_alias.as_deref(),
        &settings.alternate_aliases,
    );
    RoomSettingsSnapshot {
        room_id: settings.room_id,
        name: settings.name,
        topic: settings.topic,
        avatar_url: settings.avatar_url,
        canonical_alias: settings.canonical_alias,
        alternate_aliases: settings.alternate_aliases,
        share_link,
        join_rule: room_join_rule_from_sdk(settings.join_rule),
        history_visibility: room_history_visibility_from_sdk(settings.history_visibility),
        permissions: room_permission_facts_from_sdk(settings.permissions),
        members: settings
            .members
            .into_iter()
            .map(room_member_summary_from_sdk)
            .collect(),
    }
}

fn room_member_summary_from_sdk(member: MatrixRoomMemberSummary) -> RoomMemberSummary {
    let display_label = member
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|display_name| !display_name.is_empty())
        .unwrap_or(member.user_id.as_str())
        .to_owned();
    RoomMemberSummary {
        user_id: member.user_id,
        display_name: member.display_name,
        display_label: display_label.clone(),
        original_display_label: display_label,
        avatar_url: member.avatar_url,
        power_level: member.power_level,
        role: room_member_role_from_sdk(member.role),
        role_options: member
            .role_options
            .into_iter()
            .map(|option| RoomMemberRoleOption {
                power_level: option.power_level,
                role: room_member_role_from_sdk(option.role),
                requires_confirmation: option.requires_confirmation,
            })
            .collect(),
        user_trust: member.user_trust.map(user_trust_state_from_sdk),
    }
}

fn user_trust_state_from_sdk(state: MatrixUserTrustState) -> UserTrustState {
    match state {
        MatrixUserTrustState::Unverified => UserTrustState::Unverified,
        MatrixUserTrustState::Verified => UserTrustState::Verified,
        MatrixUserTrustState::IdentityReset => UserTrustState::IdentityReset,
    }
}

fn room_member_role_from_sdk(role: MatrixRoomMemberRole) -> RoomMemberRole {
    match role {
        MatrixRoomMemberRole::Creator => RoomMemberRole::Creator,
        MatrixRoomMemberRole::Administrator => RoomMemberRole::Administrator,
        MatrixRoomMemberRole::Moderator => RoomMemberRole::Moderator,
        MatrixRoomMemberRole::User => RoomMemberRole::User,
    }
}

fn room_join_rule_from_sdk(join_rule: MatrixRoomJoinRule) -> RoomJoinRule {
    match join_rule {
        MatrixRoomJoinRule::Public => RoomJoinRule::Public,
        MatrixRoomJoinRule::Invite => RoomJoinRule::Invite,
        MatrixRoomJoinRule::Knock => RoomJoinRule::Knock,
        MatrixRoomJoinRule::Restricted => RoomJoinRule::Restricted,
        MatrixRoomJoinRule::Private => RoomJoinRule::Private,
    }
}

fn room_join_rule_to_sdk(join_rule: RoomJoinRule) -> MatrixRoomJoinRule {
    match join_rule {
        RoomJoinRule::Public => MatrixRoomJoinRule::Public,
        RoomJoinRule::Invite => MatrixRoomJoinRule::Invite,
        RoomJoinRule::Knock => MatrixRoomJoinRule::Knock,
        RoomJoinRule::Restricted => MatrixRoomJoinRule::Restricted,
        RoomJoinRule::Private => MatrixRoomJoinRule::Private,
    }
}

fn room_history_visibility_from_sdk(
    history_visibility: MatrixRoomHistoryVisibility,
) -> RoomHistoryVisibility {
    match history_visibility {
        MatrixRoomHistoryVisibility::WorldReadable => RoomHistoryVisibility::WorldReadable,
        MatrixRoomHistoryVisibility::Shared => RoomHistoryVisibility::Shared,
        MatrixRoomHistoryVisibility::Invited => RoomHistoryVisibility::Invited,
        MatrixRoomHistoryVisibility::Joined => RoomHistoryVisibility::Joined,
    }
}

fn room_history_visibility_to_sdk(
    history_visibility: RoomHistoryVisibility,
) -> MatrixRoomHistoryVisibility {
    match history_visibility {
        RoomHistoryVisibility::WorldReadable => MatrixRoomHistoryVisibility::WorldReadable,
        RoomHistoryVisibility::Shared => MatrixRoomHistoryVisibility::Shared,
        RoomHistoryVisibility::Invited => MatrixRoomHistoryVisibility::Invited,
        RoomHistoryVisibility::Joined => MatrixRoomHistoryVisibility::Joined,
    }
}

fn room_permission_facts_from_sdk(permissions: MatrixRoomPermissionFacts) -> RoomPermissionFacts {
    RoomPermissionFacts {
        can_edit_settings: permissions.can_edit_settings,
        can_edit_roles: permissions.can_edit_roles,
        can_invite: permissions.can_invite,
        can_kick: permissions.can_kick,
        can_ban: permissions.can_ban,
        can_unban: permissions.can_unban,
    }
}

fn room_setting_change_to_sdk(change: RoomSettingChange) -> MatrixRoomSettingChange {
    match change {
        RoomSettingChange::Name(name) => MatrixRoomSettingChange::Name(name),
        RoomSettingChange::Topic(topic) => MatrixRoomSettingChange::Topic(topic),
        RoomSettingChange::AvatarUrl(avatar_url) => MatrixRoomSettingChange::AvatarUrl(avatar_url),
        RoomSettingChange::JoinRule(join_rule) => {
            MatrixRoomSettingChange::JoinRule(room_join_rule_to_sdk(join_rule))
        }
        RoomSettingChange::HistoryVisibility(history_visibility) => {
            MatrixRoomSettingChange::HistoryVisibility(room_history_visibility_to_sdk(
                history_visibility,
            ))
        }
    }
}

fn room_moderation_action_to_sdk(action: RoomModerationAction) -> MatrixRoomModerationAction {
    match action {
        RoomModerationAction::Kick => MatrixRoomModerationAction::Kick,
        RoomModerationAction::Ban => MatrixRoomModerationAction::Ban,
        RoomModerationAction::Unban => MatrixRoomModerationAction::Unban,
    }
}

fn room_moderation_allowed(
    permissions: &RoomPermissionFacts,
    action: RoomModerationAction,
) -> bool {
    match action {
        RoomModerationAction::Kick => permissions.can_kick,
        RoomModerationAction::Ban => permissions.can_ban,
        RoomModerationAction::Unban => permissions.can_unban,
    }
}

impl RoomActor {
    pub(super) async fn handle_load_room_settings(&self, request_id: RequestId, room_id: String) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        match koushi_sdk::get_room_settings_snapshot(session, &room_id).await {
            Ok(settings) => {
                let settings = room_settings_snapshot_from_sdk(settings);
                // Request-outcome settlement may accept this exact baseline generation for an
                // idempotent reload, so the authoritative reduction must complete before the
                // correlated RoomSettingsLoaded event is emitted.
                self.reduce_reliable(vec![AppAction::RoomSettingsSnapshotLoaded {
                    room_id,
                    settings: settings.clone(),
                }])
                .await;
                self.emit(CoreEvent::Room(RoomEvent::RoomSettingsLoaded {
                    request_id,
                    settings,
                }));
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    pub(super) async fn handle_update_room_setting(
        &self,
        request_id: RequestId,
        room_id: String,
        change: RoomSettingChange,
    ) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        let settings = match koushi_sdk::get_room_settings_snapshot(session, &room_id).await {
            Ok(settings) => room_settings_snapshot_from_sdk(settings),
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
                return;
            }
        };
        self.reduce_reliable(vec![AppAction::RoomSettingsSnapshotLoaded {
            room_id: room_id.clone(),
            settings: settings.clone(),
        }])
        .await;
        if !settings.permissions.can_edit_settings {
            self.reduce_reliable(vec![AppAction::RoomSettingUpdateRequested {
                request_id: request_id.sequence,
                room_id,
                change,
            }])
            .await;
            self.emit_failure(
                request_id,
                CoreFailure::RoomOperationFailed {
                    kind: RoomFailureKind::Forbidden,
                },
            );
            return;
        }

        self.reduce_reliable(vec![AppAction::RoomSettingUpdateRequested {
            request_id: request_id.sequence,
            room_id: room_id.clone(),
            change: change.clone(),
        }])
        .await;

        match koushi_sdk::update_room_setting(session, &room_id, room_setting_change_to_sdk(change))
            .await
        {
            Ok(settings) => {
                let settings = room_settings_snapshot_from_sdk(settings);
                self.reduce_reliable(vec![AppAction::RoomSettingUpdateSucceeded {
                    request_id: request_id.sequence,
                    room_id,
                    settings: settings.clone(),
                }])
                .await;
                self.emit(CoreEvent::Room(RoomEvent::RoomSettingUpdated {
                    request_id,
                    settings,
                }));
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.reduce_reliable(vec![AppAction::RoomSettingUpdateFailed {
                    request_id: request_id.sequence,
                    room_id,
                    kind: operation_failure_kind(kind),
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    pub(super) async fn handle_moderate_room_member(
        &self,
        request_id: RequestId,
        room_id: String,
        target_user_id: String,
        action: RoomModerationAction,
        reason: Option<String>,
    ) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        let settings = match koushi_sdk::get_room_settings_snapshot(session, &room_id).await {
            Ok(settings) => room_settings_snapshot_from_sdk(settings),
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
                return;
            }
        };
        self.reduce_reliable(vec![AppAction::RoomSettingsSnapshotLoaded {
            room_id: room_id.clone(),
            settings: settings.clone(),
        }])
        .await;
        if !room_moderation_allowed(&settings.permissions, action) {
            self.reduce_reliable(vec![AppAction::RoomModerationRequested {
                request_id: request_id.sequence,
                room_id,
                target_user_id,
                action,
                reason,
            }])
            .await;
            self.emit_failure(
                request_id,
                CoreFailure::RoomOperationFailed {
                    kind: RoomFailureKind::Forbidden,
                },
            );
            return;
        }

        self.reduce_reliable(vec![AppAction::RoomModerationRequested {
            request_id: request_id.sequence,
            room_id: room_id.clone(),
            target_user_id: target_user_id.clone(),
            action,
            reason: reason.clone(),
        }])
        .await;

        match koushi_sdk::moderate_room_member(
            session,
            &room_id,
            &target_user_id,
            room_moderation_action_to_sdk(action),
            reason.as_deref(),
        )
        .await
        {
            Ok(()) => {
                self.reduce_reliable(vec![AppAction::RoomModerationSucceeded {
                    request_id: request_id.sequence,
                    room_id: room_id.clone(),
                    target_user_id: target_user_id.clone(),
                    action,
                }])
                .await;
                self.emit(CoreEvent::Room(RoomEvent::RoomMemberModerated {
                    request_id,
                    room_id,
                    target_user_id,
                    action,
                }));
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.reduce_reliable(vec![AppAction::RoomModerationFailed {
                    request_id: request_id.sequence,
                    room_id,
                    target_user_id,
                    action,
                    kind: operation_failure_kind(kind),
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }

    pub(super) async fn handle_update_room_member_role(
        &self,
        request_id: RequestId,
        room_id: String,
        target_user_id: String,
        power_level: i64,
    ) {
        let Some(session) = &self.session else {
            self.emit_failure(request_id, CoreFailure::SessionRequired);
            return;
        };

        let settings = match koushi_sdk::get_room_settings_snapshot(session, &room_id).await {
            Ok(settings) => room_settings_snapshot_from_sdk(settings),
            Err(error) => {
                let kind = classify_room_error(&error);
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
                return;
            }
        };
        self.reduce_reliable(vec![AppAction::RoomSettingsSnapshotLoaded {
            room_id: room_id.clone(),
            settings: settings.clone(),
        }])
        .await;
        if !settings.permissions.can_edit_roles {
            self.reduce_reliable(vec![AppAction::RoomMemberRoleUpdateRequested {
                request_id: request_id.sequence,
                room_id,
                target_user_id,
                power_level,
            }])
            .await;
            self.emit_failure(
                request_id,
                CoreFailure::RoomOperationFailed {
                    kind: RoomFailureKind::Forbidden,
                },
            );
            return;
        }

        self.reduce_reliable(vec![AppAction::RoomMemberRoleUpdateRequested {
            request_id: request_id.sequence,
            room_id: room_id.clone(),
            target_user_id: target_user_id.clone(),
            power_level,
        }])
        .await;

        match koushi_sdk::update_room_member_power_level(
            session,
            &room_id,
            &target_user_id,
            power_level,
        )
        .await
        {
            Ok(settings) => {
                let settings = room_settings_snapshot_from_sdk(settings);
                self.reduce_reliable(vec![
                    AppAction::RoomSettingsSnapshotLoaded {
                        room_id: room_id.clone(),
                        settings,
                    },
                    AppAction::RoomMemberRoleUpdateRequested {
                        request_id: request_id.sequence,
                        room_id: room_id.clone(),
                        target_user_id: target_user_id.clone(),
                        power_level,
                    },
                    AppAction::RoomMemberRoleUpdateSucceeded {
                        request_id: request_id.sequence,
                        room_id: room_id.clone(),
                        target_user_id: target_user_id.clone(),
                        power_level,
                    },
                ])
                .await;
                self.emit(CoreEvent::Room(RoomEvent::RoomMemberRoleUpdated {
                    request_id,
                    room_id,
                    target_user_id,
                    power_level,
                }));
            }
            Err(error) => {
                let kind = classify_room_error(&error);
                self.reduce_reliable(vec![AppAction::RoomMemberRoleUpdateFailed {
                    request_id: request_id.sequence,
                    room_id,
                    target_user_id,
                    kind: operation_failure_kind(kind),
                }])
                .await;
                self.emit_failure(request_id, CoreFailure::RoomOperationFailed { kind });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::room_settings_snapshot_from_sdk;

    use koushi_sdk::{
        MatrixRoomHistoryVisibility, MatrixRoomJoinRule, MatrixRoomMemberRole,
        MatrixRoomMemberRoleOption, MatrixRoomMemberSummary, MatrixRoomPermissionFacts,
        MatrixRoomSettingsSnapshot,
    };

    use koushi_state::RoomMemberRole;

    #[test]
    fn room_settings_snapshot_mapping_preserves_role_power_and_role_permission_facts() {
        let settings = MatrixRoomSettingsSnapshot {
            room_id: "!room:example.invalid".to_owned(),
            name: Some("Private room".to_owned()),
            topic: Some("Private topic".to_owned()),
            avatar_url: Some("mxc://example.invalid/avatar".to_owned()),
            canonical_alias: Some("#private:example.invalid".to_owned()),
            alternate_aliases: vec!["#alternate:example.invalid".to_owned()],
            join_rule: MatrixRoomJoinRule::Invite,
            history_visibility: MatrixRoomHistoryVisibility::Shared,
            permissions: MatrixRoomPermissionFacts {
                can_edit_settings: true,
                can_edit_roles: true,
                can_invite: true,
                can_kick: true,
                can_ban: false,
                can_unban: false,
            },
            members: vec![MatrixRoomMemberSummary {
                user_id: "@member:example.invalid".to_owned(),
                display_name: Some("Private member".to_owned()),
                avatar_url: Some("mxc://example.invalid/member-avatar".to_owned()),
                power_level: Some(50),
                role: MatrixRoomMemberRole::Moderator,
                role_options: vec![MatrixRoomMemberRoleOption {
                    power_level: 0,
                    role: MatrixRoomMemberRole::User,
                    requires_confirmation: false,
                }],
                user_trust: None,
            }],
        };

        let mapped = room_settings_snapshot_from_sdk(settings);

        assert!(mapped.permissions.can_edit_roles);
        assert_eq!(mapped.members[0].role_options[0].power_level, 0);
        assert!(mapped.permissions.can_invite);
        assert_eq!(
            mapped.share_link.as_deref(),
            Some("https://matrix.to/#/%23private%3Aexample.invalid")
        );
        let member = mapped.members.first().expect("member summary");
        assert_eq!(member.power_level, Some(50));
        assert_eq!(member.role, RoomMemberRole::Moderator);
        let debug = format!("{mapped:?}");
        assert!(!debug.contains("Private room"), "{debug}");
        assert!(!debug.contains("Private topic"), "{debug}");
        assert!(!debug.contains("@member:example.invalid"), "{debug}");
        assert!(!debug.contains("mxc://example.invalid"), "{debug}");
    }
}
