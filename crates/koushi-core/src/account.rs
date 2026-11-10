//! AccountActor ownership façade.

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RoomEventLookupResult {
    /// #1854: the lookup also reports the thread the event replies into, so a
    /// threaded navigation target can be routed to the thread pane.
    Located {
        thread_root_event_id: Option<String>,
    },
    Missing,
    Failed,
}

mod account_management;
mod account_notifications;
#[cfg(test)]
mod account_notifications_tests;
mod actor;
mod avatar_demand;
mod contact_security;
#[cfg(test)]
mod contact_security_tests;
mod direct_action_ledger;
mod history_export;
mod local_data_cleanup;
mod own_sessions;
mod profile;
#[cfg(test)]
mod profile_tests;
mod recovery_backup;
mod routing;
mod runtime_children;
mod scheduled_send;
mod session_check;
mod session_lifecycle;
mod sliding_sync;
#[cfg(test)]
mod test_support;
mod trust_gate;
mod verification;

pub(crate) use actor::AccountMessage;
pub use actor::{AccountActor, AccountActorHandle};
pub use trust_gate::VerificationMethodDiscoveryResult;
#[cfg(test)]
pub use verification::SyntheticVerificationTerminal;
