//! Native program catalog boundary. The running host uses only registered
//! MANAGED copies; observing a user-global CLI is forbidden for every driver.

use super::registry::{ProgramObservation, RegistryError};
use crate::store::atomic::AtomicError;
use std::{io, path::PathBuf};

const CODEX_VERSION: &str = "0.160.0";
/// The product's verified catalog is the only source of an upgrade notice.
/// A registry value, PATH lookup or network response cannot silently replace
/// the binary pinned by the running host. An unknown version is not ordered.
pub(crate) fn known_new_version(driver: &str, registered_version: &str) -> Option<&'static str> {
    fn parts(version: &str) -> Option<[u64; 3]> {
        let mut fields = version.split('.');
        let values = [fields.next()?, fields.next()?, fields.next()?];
        if fields.next().is_some() || values.iter().any(|value| value.is_empty()
            || (value.len() > 1 && value.starts_with('0'))
            || !value.bytes().all(|byte| byte.is_ascii_digit())) { return None; }
        Some([values[0].parse().ok()?, values[1].parse().ok()?, values[2].parse().ok()?])
    }
    let current = match driver {
        "codex" => CODEX_VERSION,
        "claude" => "2.1.196",
        "opencode" => "1.18.32",
        "grok" => "1.0.41",
        "antigravity" => "1.2.11",
        _ => return None,
    };
    if parts(registered_version)? >= parts(current)? {
        return None;
    }
    Some(current)
}
#[derive(Debug)]
pub(crate) enum CatalogError {
    GlobalCliObservationForbidden,
    UnknownDriver,
    UnsupportedVersion,
    PackageIdentity,
    PackageFormat,
    IdentityChanged,
    KnownFolder(i32),
    Io(io::Error),
    Json(AtomicError),
    Utf8(std::str::Utf8Error),
    Provider(super::provider_catalog::ProviderCatalogError),
    Program(RegistryError),
}
impl From<io::Error> for CatalogError {
    fn from(error: io::Error) -> Self { Self::Io(error) }
}
impl From<AtomicError> for CatalogError {
    fn from(error: AtomicError) -> Self { Self::Json(error) }
}
impl From<RegistryError> for CatalogError {
    fn from(error: RegistryError) -> Self { Self::Program(error) }
}
impl From<super::provider_catalog::ProviderCatalogError> for CatalogError {
    fn from(error: super::provider_catalog::ProviderCatalogError) -> Self {
        use super::provider_catalog::ProviderCatalogError as Provider;
        match error {
            Provider::UnknownDriver => Self::UnknownDriver,
            Provider::NotInstalled(error) | Provider::Io(error) => Self::Io(error),
            Provider::UnsupportedVersion => Self::UnsupportedVersion,
            Provider::PackageIdentity => Self::PackageIdentity,
            Provider::PackageFormat => Self::PackageFormat,
            Provider::IdentityChanged => Self::IdentityChanged,
            Provider::KnownFolder(code) => Self::KnownFolder(code),
            Provider::Json(error) => Self::Json(error),
            Provider::Utf8(error) => Self::Utf8(error),
            Provider::Program(error) => Self::Program(error),
            error @ Provider::UnsupportedObservation(_) => Self::Provider(error),
        }
    }
}

/// Registration cannot inspect a global Codex/provider executable. A private
/// managed CLI must first be installed through the independent official path.
pub(crate) fn discover_program(_driver_id: &str) -> Result<ProgramObservation, CatalogError> {
    Err(CatalogError::GlobalCliObservationForbidden)
}

/// H cannot recover an unbound source by searching the user's global CLI.
pub(crate) fn locate_pinned_program(_driver_id: &str, _digest: &str,
    _version: &str) -> Result<PathBuf, CatalogError> {
    Err(CatalogError::GlobalCliObservationForbidden)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn catalog_notice_only_names_a_known_newer_pinned_version() {
        assert_eq!(known_new_version("codex", "0.149.0"), Some("0.160.0"));
        for version in ["0.160.0", "0.161.0", "0.16x.0", "00.149.0", "0.149.0-beta"] {
            assert_eq!(known_new_version("codex", version), None);
        }
        assert_eq!(known_new_version("claude", "0.149.0"), Some("2.1.196"));
        assert_eq!(known_new_version("opencode", "1.18.31"), Some("1.18.32"));
        assert_eq!(known_new_version("grok", "1.0.40"), Some("1.0.41"));
        assert_eq!(known_new_version("antigravity", "1.2.10"), Some("1.2.11"));
        assert_eq!(known_new_version("unknown-driver", "0.149.0"), None);
    }

    #[test]
    fn global_cli_discovery_and_location_are_forbidden_for_every_driver() {
        for driver in ["codex", "claude", "opencode", "grok", "antigravity", "unknown"] {
            assert!(matches!(discover_program(driver),
                Err(CatalogError::GlobalCliObservationForbidden)), "{driver}");
            assert!(matches!(locate_pinned_program(driver, "sha256:any", "0.0.0"),
                Err(CatalogError::GlobalCliObservationForbidden)), "{driver}");
        }
    }
}