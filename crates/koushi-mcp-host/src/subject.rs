//! The identity a grant, an approval, an idempotency key, an activity cursor
//! and a draft are keyed on (#2132, D5).
//!
//! A connection is either an unprofiled client, known only by the name it
//! declared, or a profile the owner created. [`GrantSubject`] is the only thing
//! any authorization path accepts. The name the client declared is demoted to a
//! [`Label`]: it can be shown and nothing else. A `Label` is not `Hash`, `Eq`,
//! `Ord` or convertible to a string, and no key-taking function accepts one, so
//! the compiler refuses a grant keyed on a label.
//!
//! ```compile_fail,E0599
//! // A label cannot be used as a grant key: it is not `Hash` or `Eq`.
//! let identity = koushi_mcp_host::consent::ClientIdentity::unprofiled("codex", "1");
//! let mut keyed = std::collections::HashSet::new();
//! keyed.insert(identity.label());
//! ```
//!
//! ```compile_fail,E0308
//! // Nor can it stand in for the subject that grant lookups take.
//! fn needs_subject(_: &koushi_mcp_host::consent::GrantSubject) {}
//! let identity = koushi_mcp_host::consent::ClientIdentity::unprofiled("codex", "1");
//! needs_subject(&identity.label());
//! ```
//!
//! ```compile_fail,E0277
//! // A label is not a string, so it cannot be turned into a key.
//! let identity = koushi_mcp_host::consent::ClientIdentity::unprofiled("codex", "1");
//! let _key: String = identity.label().into();
//! ```
//!
//! ```compile_fail,E0308
//! // A profile id is not a client name: the two newtypes never convert.
//! let id = koushi_mcp_host::consent::ProfileId::parse("prof_00000000000000000000000000000000")
//!     .unwrap();
//! let _name: koushi_mcp_host::consent::ClientName = id;
//! ```
//!
//! Control: the subject itself is a key.
//!
//! ```
//! let identity = koushi_mcp_host::consent::ClientIdentity::unprofiled("codex", "1");
//! let mut keyed = std::collections::HashSet::new();
//! keyed.insert(identity.subject().clone());
//! assert_eq!(identity.label().to_string(), "codex");
//! ```

use std::fmt;

/// An unprofiled client's self-declared name. Not verified: any process of the
/// same OS user can claim any name.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ClientName(String);

impl ClientName {
    pub fn new(name: &str) -> Self {
        Self(name.to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An owner-created profile id: `prof_` and 128 random bits in hex.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProfileId(String);

impl fmt::Debug for ProfileId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("ProfileId").field(&self.0).finish()
    }
}

impl ProfileId {
    const PREFIX: &'static str = "prof_";

    /// Accepts only the exact shape the host generates, so an id is always a
    /// safe file name and nothing else can reach a path.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix(Self::PREFIX)?;
        (digits.len() == 32
            && digits
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
        .then(|| Self(text.to_owned()))
    }

    pub(crate) fn from_random(bytes: [u8; 16]) -> Self {
        let mut text = String::from(Self::PREFIX);
        for byte in bytes {
            text.push_str(&format!("{byte:02x}"));
        }
        Self(text)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a grant, an approval, a retry key, an activity cursor and a draft
/// belong to.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GrantSubject {
    Client(ClientName),
    Profile(ProfileId),
}

impl GrantSubject {
    pub fn client(name: &str) -> Self {
        Self::Client(ClientName::new(name))
    }

    pub fn profile(id: ProfileId) -> Self {
        Self::Profile(id)
    }

    pub fn profile_id(&self) -> Option<&ProfileId> {
        match self {
            Self::Profile(id) => Some(id),
            Self::Client(_) => None,
        }
    }

    /// A stable, namespaced key for the places that store a string: a profile
    /// id can never equal a client name's key because the namespaces differ
    /// and a client name cannot contain `:`.
    pub fn key(&self) -> String {
        match self {
            Self::Client(name) => format!("client:{}", name.0),
            Self::Profile(id) => format!("profile:{}", id.0),
        }
    }
}

/// Text shown to the owner. Deliberately not `Hash`, `Eq`, `Ord`, or
/// convertible to a string: it names a connection and keys nothing.
#[derive(Clone, Debug)]
pub struct Label(String);

impl fmt::Display for Label {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// The identity of one connection.
///
/// The assistant-declared name and version are unverified; the owner-chosen
/// profile label is trusted text. Only the [`GrantSubject`] decides access.
#[derive(Clone, Debug)]
pub struct ClientIdentity {
    label: Label,
    version: String,
    subject: GrantSubject,
    bound_account: Option<String>,
}

impl ClientIdentity {
    /// An unprofiled client: its declared name is both its label and its key.
    pub fn unprofiled(name: &str, version: &str) -> Self {
        Self {
            label: Label(name.to_owned()),
            version: version.to_owned(),
            subject: GrantSubject::client(name),
            bound_account: None,
        }
    }

    /// A profiled connection. `label` is the owner's text for the profile; the
    /// name the client declared never reaches the subject.
    pub(crate) fn profiled(
        id: ProfileId,
        label: &str,
        version: &str,
        bound_account: String,
    ) -> Self {
        Self {
            label: Label(label.to_owned()),
            version: version.to_owned(),
            subject: GrantSubject::Profile(id),
            bound_account: Some(bound_account),
        }
    }

    pub fn subject(&self) -> &GrantSubject {
        &self.subject
    }

    pub fn label(&self) -> &Label {
        &self.label
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    /// The one account a profiled connection may name; `None` when unprofiled.
    pub(crate) fn bound_account(&self) -> Option<&str> {
        self.bound_account.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "prof_00000000000000000000000000000001";

    #[test]
    fn keys_of_a_client_and_a_profile_never_meet() {
        let profile = GrantSubject::profile(ProfileId::parse(ID).expect("id"));
        // A client that declares the profile's raw id as its name is another
        // subject, and so is its key.
        let imposter = GrantSubject::client(ID);
        assert_ne!(profile, imposter);
        assert_ne!(profile.key(), imposter.key());
        assert_eq!(GrantSubject::client("codex").key(), "client:codex");
        assert_eq!(profile.key(), format!("profile:{ID}"));
    }

    #[test]
    fn only_the_exact_generated_shape_is_a_profile_id() {
        assert!(ProfileId::parse(ID).is_some());
        for bad in [
            "",
            "prof_",
            "prof_0",
            "PROF_00000000000000000000000000000001",
            "prof_0000000000000000000000000000000G",
            "prof_000000000000000000000000000000011",
            "prof_../../../etc/passwd",
            "codex",
        ] {
            assert!(ProfileId::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn a_profiled_identity_keys_on_the_profile_and_shows_the_owners_label() {
        let id = ProfileId::parse(ID).expect("id");
        let identity = ClientIdentity::profiled(id.clone(), "Reader", "1.0", "account:@a:x".into());
        assert_eq!(identity.subject(), &GrantSubject::Profile(id));
        assert_eq!(identity.label().to_string(), "Reader");
        assert_eq!(identity.bound_account(), Some("account:@a:x"));
        assert_eq!(
            ClientIdentity::unprofiled("codex", "1").bound_account(),
            None
        );
    }
}
