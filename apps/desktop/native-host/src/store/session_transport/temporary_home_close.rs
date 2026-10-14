//! H's deliberately closed exact-gone proof seam.
//!
//! The durable C custody row identifies the original root process, but its
//! PID/creation pair is not a complete execution-set receipt. In particular,
//! it does not enumerate inheriting descendants or prove that every original
//! Job holder has gone. Counting C rows would count records, not processes.
//! Until a direct native producer persists and revalidates that complete set,
//! H cannot mint a typed absence fact for an F HOME transition.
use crate::store::same_open::VerifiedDatabaseConnection;

#[derive(Debug)]
pub(crate) enum ExactHomeGoneError {
    Missing(&'static str),
}

pub(crate) fn observe_exact_home_gone(
    _db: &VerifiedDatabaseConnection<'_>, _home: &str, _instance: &str,
    _domain: &str, _session: &str, _generation: &str, _binding: &str,
) -> Result<(), ExactHomeGoneError> {
    Err(ExactHomeGoneError::Missing(
        "complete original execution-set provenance",
    ))
}
