use serde::{Deserialize, Serialize};

use super::{OpaqueId, Validate, SCHEMA_VERSION};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    FeatureSealed,
    Unauthorized,
    ScopeMismatch,
    StaleBinding,
    Unsupported,
    CapabilityUnknown,
    ProtocolVersionMismatch,
    InvalidFrame,
    OperationConflict,
    AcceptanceUnknown,
    RootUnavailable,
    RootInUse,
    RootIdMismatch,
    UnsupportedRoot,
    StoreCorrupt,
    StoreWriteFailed,
    DurabilityUnknown,
    ProcessIdentityUnknown,
    StopUnconfirmed,
    LegacyRouteDisabled,
    QueueFull,
    ResyncRequired,
    ConfigChanged,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectState {
    None,
    Possible,
    Confirmed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandError {
    pub code: ErrorCode,
    pub stage: String,
    pub retryable: bool,
    pub side_effect_state: SideEffectState,
    pub correlation_id: OpaqueId,
    /// Must be public/redacted; domain-private diagnostics do not belong here.
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandSuccess<T> {
    pub schema_version: u64,
    pub ok: bool,
    pub value: T,
    #[serde(deserialize_with = "required_nullable")]
    pub receipt: Option<serde_json::Value>,
}

impl<T> CommandSuccess<T> {
    pub fn new(value: T, receipt: Option<serde_json::Value>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            ok: true,
            value,
            receipt,
        }
    }
}

impl<T> Validate for CommandSuccess<T> {
    fn validate(&self) -> Result<(), &'static str> {
        if self.ok {
            Ok(())
        } else {
            Err("SUCCESS_ENVELOPE_OK_MUST_BE_TRUE")
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandFailure {
    pub schema_version: u64,
    pub ok: bool,
    pub error: CommandError,
}

impl CommandFailure {
    pub fn new(error: CommandError) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            ok: false,
            error,
        }
    }
}

impl Validate for CommandFailure {
    fn validate(&self) -> Result<(), &'static str> {
        if self.ok {
            Err("FAILURE_ENVELOPE_OK_MUST_BE_FALSE")
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CommandResponse<T> {
    Success(CommandSuccess<T>),
    Failure(CommandFailure),
}

impl<T> Validate for CommandResponse<T> {
    fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Success(success) => success.validate(),
            Self::Failure(failure) => failure.validate(),
        }
    }
}

fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}
