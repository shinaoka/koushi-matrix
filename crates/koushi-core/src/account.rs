//! AccountActor ownership façade.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoomEventLookupResult {
    Located,
    Missing,
    Failed,
}

mod account_management;
mod actor;
mod local_data_cleanup;
mod profile;
#[cfg(test)]
mod profile_tests;
mod recovery_backup;
mod routing;
mod runtime_children;
mod scheduled_send;
mod session_lifecycle;
mod sliding_sync;
#[cfg(test)]
mod test_source;
#[cfg(test)]
mod test_support;
mod trust_gate;
mod verification;

pub(crate) use actor::AccountMessage;
pub use actor::{AccountActor, AccountActorHandle};
pub use trust_gate::VerificationMethodDiscoveryResult;
#[cfg(test)]
pub use verification::SyntheticVerificationTerminal;
