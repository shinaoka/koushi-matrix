use thiserror::Error;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum SearchVerificationError {
    #[error("candidate event is not available")]
    MissingCandidate,
}

pub fn verify_candidate() -> Result<(), SearchVerificationError> {
    Ok(())
}
