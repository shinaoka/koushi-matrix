/// Internal transport envelope for causal SDK projection generations.
///
/// The Matrix SDK currently carries only a `u64` repair generation. Core owns
/// two independent serial namespaces, so the high bit is reserved as an
/// operation-kind discriminator: historical gap repair stays in the low
/// domain and live-tail refresh uses the high domain. Raw producer serials
/// must fit in the remaining 63 bits and never leave Core in encoded form.
pub(crate) const CAUSAL_PROJECTION_DOMAIN_BIT: u64 = 1 << 63;
pub(crate) const CAUSAL_PROJECTION_SERIAL_MAX: u64 = CAUSAL_PROJECTION_DOMAIN_BIT - 1;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum CausalProjectionDomain {
    HistoricalGap,
    LiveTail,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct CausalProjectionOperationId {
    pub(crate) domain: CausalProjectionDomain,
    pub(crate) serial: u64,
}

impl CausalProjectionOperationId {
    pub(crate) fn new(domain: CausalProjectionDomain, serial: u64) -> Option<Self> {
        (serial <= CAUSAL_PROJECTION_SERIAL_MAX).then_some(Self { domain, serial })
    }

    pub(crate) fn encode_transport(self) -> u64 {
        match self.domain {
            CausalProjectionDomain::HistoricalGap => self.serial,
            CausalProjectionDomain::LiveTail => CAUSAL_PROJECTION_DOMAIN_BIT | self.serial,
        }
    }

    pub(crate) fn decode_transport(encoded: u64) -> Self {
        let domain = if encoded & CAUSAL_PROJECTION_DOMAIN_BIT == 0 {
            CausalProjectionDomain::HistoricalGap
        } else {
            CausalProjectionDomain::LiveTail
        };
        Self {
            domain,
            serial: encoded & CAUSAL_PROJECTION_SERIAL_MAX,
        }
    }
}

/// Advance a raw producer serial without ever consuming the domain bit.
///
/// Exhaustion may restart at one only after the caller proves that no
/// operation in the same domain still owns a pending causal identity.
pub(crate) fn next_causal_projection_serial(
    current: u64,
    same_domain_identity_pending: bool,
) -> Option<u64> {
    match current.checked_add(1) {
        Some(next) if next <= CAUSAL_PROJECTION_SERIAL_MAX => Some(next),
        _ if !same_domain_identity_pending => Some(1),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct CausalProjectionId {
    pub(crate) actor_generation: u64,
    pub(crate) operation: CausalProjectionOperationId,
    pub(crate) projection_batch: u32,
}
