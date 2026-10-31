use std::time::Duration;

use koushi_state::{AppAction, CurrentDeviceTrustState};

use crate::account::actor::AccountMessage;
use crate::account::test_support::{
    consume_initial_unknown_trust_projection, inspect_session_runtime, login_gated_actor,
};

#[tokio::test]
async fn older_unknown_projection_ack_cannot_consume_pending_verified_promotion() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let (handle, mut actions) = login_gated_actor().await;
        consume_initial_unknown_trust_projection(&mut actions).await;
        assert!(
            handle
                .send(AccountMessage::CurrentDeviceTrustChanged {
                    generation: 2,
                    trust: CurrentDeviceTrustState::Verified,
                })
                .await
        );
        let (generation, transition_id) = loop {
            let batch = actions.recv().await.expect("verified projection");
            if let Some(ids) = batch.iter().find_map(|action| match action {
                AppAction::AuthoritativeDeviceTrustChanged {
                    generation,
                    transition_id,
                    trust: CurrentDeviceTrustState::Verified,
                } => Some((*generation, *transition_id)),
                _ => None,
            }) {
                break ids;
            }
        };
        assert!(
            handle
                .send(AccountMessage::TrustProjectionApplied {
                    generation,
                    transition_id: transition_id
                        .checked_sub(1)
                        .expect("older Unknown transition"),
                    ready: false,
                    locked: false,
                })
                .await
        );
        assert!(
            handle
                .send(AccountMessage::TrustProjectionApplied {
                    generation,
                    transition_id,
                    ready: true,
                    locked: false,
                })
                .await
        );
        assert_eq!(
            inspect_session_runtime(&handle).await,
            (true, true, true, true),
            "the matching Verified ack must still start normal children after a stale Unknown ack"
        );
        assert!(handle.send(AccountMessage::Shutdown).await);
    })
    .await
    .expect("stale acknowledgement admission deadline");
}
