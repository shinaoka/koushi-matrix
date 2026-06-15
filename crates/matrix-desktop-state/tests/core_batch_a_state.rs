use matrix_desktop_state::AppState;

#[test]
fn default_state_has_core_batch_a_skeletons_idle() {
    let state = AppState::default();

    assert_eq!(state.room_interactions.len(), 0);
    assert_eq!(state.activity.kind(), "closed");
    assert_eq!(state.local_encryption.kind(), "unknown");
    assert_eq!(state.native_attention.kind(), "idle");
}
