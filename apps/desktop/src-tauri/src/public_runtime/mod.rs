//! Public-runtime side of the private native-host contract.
//!
//! Shared Tauri wiring is intentionally absent in this preparatory package.
//! Callers must provide the current-user named-pipe endpoint and the complete
//! runtime binding. Network URLs and incomplete identities are rejected before
//! any connection attempt.

use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeBinding {
    pub root_identity: String,
    pub profile_id: String,
    pub runtime_instance_id: String,
    pub auth_revision: u64,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupPhase {
    Prepared,
    DurableOwner,
    Activated,
    Initialized,
    StartupCustody,
}

#[derive(Debug, Eq, PartialEq)]
pub enum PublicRuntimeError {
    InvalidBinding(&'static str),
    NonPrivateEndpoint,
    InvalidTransition {
        from: StartupPhase,
        requested: StartupPhase,
    },
}

impl fmt::Display for PublicRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBinding(field) => {
                write!(formatter, "PUBLIC_RUNTIME_INVALID_BINDING: {field}")
            }
            Self::NonPrivateEndpoint => write!(formatter, "PUBLIC_RUNTIME_NON_PRIVATE_ENDPOINT"),
            Self::InvalidTransition { from, requested } => write!(
                formatter,
                "PUBLIC_RUNTIME_INVALID_TRANSITION: {from:?} -> {requested:?}"
            ),
        }
    }
}

impl std::error::Error for PublicRuntimeError {}

fn canonical(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && !value.chars().any(char::is_control)
}

impl RuntimeBinding {
    pub fn validate(&self) -> Result<(), PublicRuntimeError> {
        for (field, value) in [
            ("root_identity", self.root_identity.as_str()),
            ("profile_id", self.profile_id.as_str()),
            ("runtime_instance_id", self.runtime_instance_id.as_str()),
        ] {
            if !canonical(value) {
                return Err(PublicRuntimeError::InvalidBinding(field));
            }
        }
        Ok(())
    }

    pub fn instance_key(&self) -> String {
        format!(
            "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
            self.root_identity,
            self.profile_id,
            self.runtime_instance_id,
            self.auth_revision,
            self.generation
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrivateHostEndpoint(String);

impl PrivateHostEndpoint {
    pub fn parse(value: impl Into<String>) -> Result<Self, PublicRuntimeError> {
        let value = value.into();
        let suffix = value
            .strip_prefix(r"\\.\pipe\gogoke.current-user.v1.")
            .ok_or(PublicRuntimeError::NonPrivateEndpoint)?;
        if suffix.is_empty()
            || suffix.len() > 120
            || !suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(PublicRuntimeError::NonPrivateEndpoint);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Mechanical guard for prepare -> persist -> activate -> initialize.
/// Any failure after prepare moves to retained startup custody.
#[derive(Debug)]
pub struct StartupGuard {
    binding: RuntimeBinding,
    endpoint: PrivateHostEndpoint,
    phase: StartupPhase,
}

impl StartupGuard {
    pub fn prepared(
        binding: RuntimeBinding,
        endpoint: PrivateHostEndpoint,
    ) -> Result<Self, PublicRuntimeError> {
        binding.validate()?;
        Ok(Self {
            binding,
            endpoint,
            phase: StartupPhase::Prepared,
        })
    }

    pub fn binding(&self) -> &RuntimeBinding {
        &self.binding
    }

    pub fn endpoint(&self) -> &PrivateHostEndpoint {
        &self.endpoint
    }

    pub fn phase(&self) -> StartupPhase {
        self.phase
    }

    pub fn durable_owner_persisted(&mut self) -> Result<(), PublicRuntimeError> {
        self.transition(StartupPhase::Prepared, StartupPhase::DurableOwner)
    }

    pub fn activated(&mut self) -> Result<(), PublicRuntimeError> {
        self.transition(StartupPhase::DurableOwner, StartupPhase::Activated)
    }

    pub fn initialized(&mut self) -> Result<(), PublicRuntimeError> {
        self.transition(StartupPhase::Activated, StartupPhase::Initialized)
    }

    pub fn retain_startup_custody(&mut self) {
        self.phase = StartupPhase::StartupCustody;
    }

    pub fn is_ready(&self) -> bool {
        self.phase == StartupPhase::Initialized
    }

    fn transition(
        &mut self,
        expected: StartupPhase,
        requested: StartupPhase,
    ) -> Result<(), PublicRuntimeError> {
        if self.phase != expected {
            return Err(PublicRuntimeError::InvalidTransition {
                from: self.phase,
                requested,
            });
        }
        self.phase = requested;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding() -> RuntimeBinding {
        RuntimeBinding {
            root_identity: "volume:1/file:2".into(),
            profile_id: "current-user".into(),
            runtime_instance_id: "runtime-a".into(),
            auth_revision: 7,
            generation: 9,
        }
    }

    #[test]
    fn rejects_tcp_http_and_noncanonical_pipe_endpoints() {
        for endpoint in [
            "http://127.0.0.1:7740",
            "tcp://127.0.0.1:7740",
            r"\\.\pipe\other-service",
            r"\\.\pipe\gogoke.current-user.v1...\escape",
        ] {
            assert_eq!(
                PrivateHostEndpoint::parse(endpoint),
                Err(PublicRuntimeError::NonPrivateEndpoint)
            );
        }
    }

    #[test]
    fn binds_every_runtime_identity_dimension_into_the_instance_key() {
        let original = binding();
        let original_key = original.instance_key();
        let mutations = [
            RuntimeBinding {
                root_identity: "volume:other/file:2".into(),
                ..original.clone()
            },
            RuntimeBinding {
                profile_id: "other-profile".into(),
                ..original.clone()
            },
            RuntimeBinding {
                runtime_instance_id: "runtime-b".into(),
                ..original.clone()
            },
            RuntimeBinding {
                auth_revision: 8,
                ..original.clone()
            },
            RuntimeBinding {
                generation: 10,
                ..original.clone()
            },
        ];
        for mutation in mutations {
            assert_ne!(mutation.instance_key(), original_key);
        }
    }

    #[test]
    fn ready_requires_prepare_persist_activate_and_initialize_in_order() {
        let endpoint =
            PrivateHostEndpoint::parse(r"\\.\pipe\gogoke.current-user.v1.fixture-runtime")
                .expect("private endpoint");
        let mut guard = StartupGuard::prepared(binding(), endpoint).expect("prepared");
        assert!(!guard.is_ready());
        assert!(guard.activated().is_err());
        guard.durable_owner_persisted().expect("durable owner");
        assert!(!guard.is_ready());
        guard.activated().expect("activated");
        assert!(!guard.is_ready());
        guard.initialized().expect("initialized");
        assert!(guard.is_ready());
    }

    #[test]
    fn failure_retains_startup_custody_and_cannot_be_reactivated() {
        let endpoint =
            PrivateHostEndpoint::parse(r"\\.\pipe\gogoke.current-user.v1.fixture-runtime")
                .expect("private endpoint");
        let mut guard = StartupGuard::prepared(binding(), endpoint).expect("prepared");
        guard.durable_owner_persisted().expect("durable owner");
        guard.retain_startup_custody();
        assert_eq!(guard.phase(), StartupPhase::StartupCustody);
        assert!(!guard.is_ready());
        assert!(guard.activated().is_err());
    }
}


mod product_entry;
pub(crate) use product_entry::gogoke_r2_goal_probe;
