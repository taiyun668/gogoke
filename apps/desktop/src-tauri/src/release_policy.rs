use std::net::SocketAddr;

pub(crate) const FEATURE_SEALED_CODE: &str = "FEATURE_SEALED";

const REMOTE_EXTERNAL_ENABLED: bool = false;
const VOICE_ENABLED: bool = false;
const DEFAULT_RUNTIME_DRIVERS_ENABLED: bool = false;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReleaseCapability {
    LocalNonModel,
    RemoteAccess,
    MobileClient,
    Tailscale,
    Voice,
    Update,
    Install,
    Telemetry,
    AutomaticProbe,
    TitleGeneration,
    AccountAccess,
    NetworkListener,
    NetworkEgress,
    ModelRuntime,
}

impl ReleaseCapability {
    fn label(self) -> &'static str {
        match self {
            Self::LocalNonModel => "local non-model",
            Self::RemoteAccess => "remote access",
            Self::MobileClient => "mobile client",
            Self::Tailscale => "Tailscale",
            Self::Voice => "voice input",
            Self::Update => "update",
            Self::Install => "install",
            Self::Telemetry => "telemetry",
            Self::AutomaticProbe => "automatic probe",
            Self::TitleGeneration => "title generation",
            Self::AccountAccess => "account access",
            Self::NetworkListener => "network listener",
            Self::NetworkEgress => "network egress",
            Self::ModelRuntime => "model runtime",
        }
    }
}

pub(crate) fn remote_external_enabled() -> bool {
    REMOTE_EXTERNAL_ENABLED
}

pub(crate) fn voice_enabled() -> bool {
    VOICE_ENABLED
}

pub(crate) fn default_runtime_drivers_enabled() -> bool {
    DEFAULT_RUNTIME_DRIVERS_ENABLED
}

pub(crate) fn require_capability(
    capability: ReleaseCapability,
    operation: &str,
) -> Result<(), String> {
    if capability == ReleaseCapability::LocalNonModel {
        return Ok(());
    }
    Err(sealed_error(capability.label(), operation))
}

pub(crate) fn construct_if_allowed<T>(
    capability: ReleaseCapability,
    operation: &str,
    constructor: impl FnOnce() -> T,
) -> Result<T, String> {
    require_capability(capability, operation)?;
    Ok(constructor())
}

pub(crate) fn require_remote_external(operation: &str) -> Result<(), String> {
    require_capability(ReleaseCapability::RemoteAccess, operation)
}

pub(crate) fn require_voice(operation: &str) -> Result<(), String> {
    require_capability(ReleaseCapability::Voice, operation)
}

pub(crate) fn validate_legacy_loopback(
    endpoint: &str,
    token: Option<&str>,
    insecure_no_auth: bool,
) -> Result<(), String> {
    if insecure_no_auth {
        return Err(format!(
            "{FEATURE_SEALED_CODE}: legacy TCP requires authentication in release builds"
        ));
    }
    if token
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        return Err(format!(
            "{FEATURE_SEALED_CODE}: legacy loopback TCP requires an authentication token"
        ));
    }
    if !is_explicit_loopback(endpoint) {
        return require_remote_external("legacy TCP endpoint");
    }
    Ok(())
}

pub(crate) fn is_explicit_loopback(endpoint: &str) -> bool {
    endpoint
        .trim()
        .parse::<SocketAddr>()
        .is_ok_and(|address| address.port() != 0 && address.ip().is_loopback())
}

fn sealed_error(feature: &str, operation: &str) -> String {
    format!("{FEATURE_SEALED_CODE}: {feature} is sealed in this release; refusing {operation}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_features_are_compile_time_sealed() {
        assert!(!remote_external_enabled());
        assert!(!voice_enabled());
        assert!(!default_runtime_drivers_enabled());
    }

    #[test]
    fn every_forbidden_capability_is_rejected_before_construction() {
        let forbidden = [
            ReleaseCapability::RemoteAccess,
            ReleaseCapability::MobileClient,
            ReleaseCapability::Tailscale,
            ReleaseCapability::Voice,
            ReleaseCapability::Update,
            ReleaseCapability::Install,
            ReleaseCapability::Telemetry,
            ReleaseCapability::AutomaticProbe,
            ReleaseCapability::TitleGeneration,
            ReleaseCapability::AccountAccess,
            ReleaseCapability::NetworkListener,
            ReleaseCapability::NetworkEgress,
            ReleaseCapability::ModelRuntime,
        ];

        for capability in forbidden {
            let mut constructed = false;
            let error = construct_if_allowed(capability, "test construction", || {
                constructed = true;
            })
            .expect_err("forbidden capability must be sealed");
            assert!(error.starts_with(FEATURE_SEALED_CODE));
            assert!(!constructed);
        }
    }

    #[test]
    fn local_non_model_construction_remains_available() {
        let mut constructed = false;
        construct_if_allowed(
            ReleaseCapability::LocalNonModel,
            "local service construction",
            || {
                constructed = true;
            },
        )
        .expect("local non-model behavior must remain available");
        assert!(constructed);
    }

    #[test]
    fn external_and_wildcard_endpoints_are_rejected_before_use() {
        for endpoint in ["0.0.0.0:4732", "[::]:4732", "device.example:4732"] {
            let error = validate_legacy_loopback(endpoint, Some("token"), false)
                .expect_err("external endpoint must be sealed");
            assert!(error.starts_with(FEATURE_SEALED_CODE));
        }
    }

    #[test]
    fn authenticated_explicit_loopback_is_the_only_legacy_exception() {
        for endpoint in ["127.0.0.1:4732", "[::1]:4732"] {
            validate_legacy_loopback(endpoint, Some("token"), false)
                .expect("authenticated loopback should remain available");
        }
        for endpoint in ["localhost:4732", "127.0.0.1:0", "[::1]:0"] {
            assert!(validate_legacy_loopback(endpoint, Some("token"), false).is_err());
        }
        assert!(validate_legacy_loopback("127.0.0.1:4732", None, false).is_err());
        assert!(validate_legacy_loopback("127.0.0.1:4732", Some("token"), true).is_err());
    }

    #[test]
    fn voice_operations_are_sealed() {
        let error = require_voice("microphone permission").expect_err("voice must be sealed");
        assert!(error.starts_with(FEATURE_SEALED_CODE));
    }
}
