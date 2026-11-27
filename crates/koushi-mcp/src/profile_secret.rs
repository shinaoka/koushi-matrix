//! Reading a profile's secret file (#2132, D4).
//!
//! The host writes the secret once to a `0600` file in a `0700` directory and
//! the launcher reads it; the secret is never in argv. The launcher refuses a
//! file it should not trust: one that is a symlink (it could point anywhere),
//! not a regular file, owned by another user, or readable by group or others.
//! On Windows the directory ACL is the protection and only the read is done.

use std::io::Read;
use std::path::Path;

/// The longest secret file read. A real one is a few dozen bytes.
const MAX_SECRET_BYTES: u64 = 4096;

/// Why a secret file was not used. No variant carries the path or any content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecretFileError {
    /// The path is a symbolic link; it is never followed.
    Symlink,
    /// Not a regular file.
    NotRegular,
    /// Owned by a different user.
    WrongOwner,
    /// Readable or writable by group or others.
    TooPermissive,
    /// Missing, unreadable, empty, or too large.
    Unusable,
}

impl SecretFileError {
    /// What the launcher tells the person.
    pub fn explanation(self) -> &'static str {
        match self {
            Self::Symlink => "the profile secret file is a symbolic link",
            Self::NotRegular => "the profile secret file is not a regular file",
            Self::WrongOwner => "the profile secret file belongs to another user",
            Self::TooPermissive => {
                "the profile secret file can be read by other users; it must be mode 0600"
            }
            Self::Unusable => "cannot read the profile secret file",
        }
    }
}

/// Reads and trims the secret.
pub fn read(path: &Path) -> Result<String, SecretFileError> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Refuses a symlink at the final component, atomically with the open.
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|error| {
        // ELOOP is how O_NOFOLLOW reports a symlink.
        if cfg!(unix) && error.raw_os_error() == Some(libc_eloop()) {
            SecretFileError::Symlink
        } else {
            SecretFileError::Unusable
        }
    })?;
    let metadata = file.metadata().map_err(|_| SecretFileError::Unusable)?;
    if !metadata.is_file() {
        return Err(SecretFileError::NotRegular);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: `geteuid` has no preconditions and cannot fail.
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(SecretFileError::WrongOwner);
        }
        if metadata.mode() & 0o077 != 0 {
            return Err(SecretFileError::TooPermissive);
        }
    }
    let mut secret = String::new();
    file.take(MAX_SECRET_BYTES + 1)
        .read_to_string(&mut secret)
        .map_err(|_| SecretFileError::Unusable)?;
    if secret.len() as u64 > MAX_SECRET_BYTES || secret.trim().is_empty() {
        return Err(SecretFileError::Unusable);
    }
    Ok(secret.trim().to_owned())
}

#[cfg(unix)]
fn libc_eloop() -> i32 {
    libc::ELOOP
}

#[cfg(not(unix))]
fn libc_eloop() -> i32 {
    0
}

#[cfg(all(test, unix))]
#[path = "profile_secret_tests.rs"]
mod tests;
