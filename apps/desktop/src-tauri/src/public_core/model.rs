use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{DecimalU64, ErrorCode, OpaqueId, UtcTimestamp, MAX_CONTEXT_ITEMS, MAX_TITLE_BYTES};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionLifecycle {
    Active,
    Archived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinuationMode {
    Native,
    FixedContext,
    New,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionState {
    Created,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceState {
    Recorded,
    Queued,
    Dispatching,
    Accepted,
    AcceptanceUnknown,
    Rejected,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    Supported,
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanActionState {
    Pending,
    Resolved,
    Expired,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicEventKind {
    SessionUpdated,
    BindingUpdated,
    DeliveryUpdated,
    ExecutionStarted,
    ExecutionOutputDelta,
    ExecutionCompleted,
    ExecutionFailed,
    ExecutionCancelled,
    HumanActionRequested,
    HumanActionResolved,
    ContextCreated,
    DelegationResult,
    StreamGap,
    Diagnostic,
}

/// C2-F01 facts remain separate. These values describe evidence, not a
/// combined authorization verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationState {
    Observed,
    NotObserved,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordedState {
    Recorded,
    NotRecorded,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurabilityAckState {
    Confirmed,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustodyState {
    Retained,
    Released,
    Transferred,
    Unknown,
}

pub trait Validate {
    fn validate(&self) -> Result<(), &'static str>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Session {
    pub schema_version: u64,
    pub session_id: OpaqueId,
    pub scope_id: OpaqueId,
    pub privacy_domain_id: OpaqueId,
    pub title: String,
    pub lifecycle: SessionLifecycle,
    pub revision: DecimalU64,
}

impl Validate for Session {
    fn validate(&self) -> Result<(), &'static str> {
        if self.title.len() > MAX_TITLE_BYTES {
            return Err("TITLE_LIMIT_EXCEEDED");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeBinding {
    pub schema_version: u64,
    pub binding_id: OpaqueId,
    pub session_id: OpaqueId,
    pub driver_id: String,
    pub instance_id: OpaqueId,
    pub profile_revision: DecimalU64,
    pub auth_revision: DecimalU64,
    pub domain_id: OpaqueId,
    pub generation: DecimalU64,
    #[serde(deserialize_with = "required_nullable")]
    pub native_session_id: Option<String>,
    pub continuation_mode: ContinuationMode,
}

impl Validate for NativeBinding {
    fn validate(&self) -> Result<(), &'static str> {
        require_nonempty(&self.driver_id, "EMPTY_DRIVER_ID")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Execution {
    pub schema_version: u64,
    pub execution_id: OpaqueId,
    pub session_id: OpaqueId,
    pub binding_id: OpaqueId,
    pub generation: DecimalU64,
    pub state: ExecutionState,
    pub created_at: UtcTimestamp,
    #[serde(deserialize_with = "required_nullable")]
    pub completed_at: Option<UtcTimestamp>,
    #[serde(deserialize_with = "required_nullable")]
    pub result_ref: Option<String>,
}

impl Validate for Execution {
    fn validate(&self) -> Result<(), &'static str> {
        match self.state {
            ExecutionState::Completed | ExecutionState::Failed | ExecutionState::Cancelled => {
                if self.completed_at.is_none() {
                    return Err("TERMINAL_EXECUTION_REQUIRES_COMPLETED_AT");
                }
            }
            ExecutionState::Created | ExecutionState::Running => {
                if self.completed_at.is_some() {
                    return Err("NON_TERMINAL_EXECUTION_HAS_COMPLETED_AT");
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Delivery {
    pub schema_version: u64,
    pub operation_id: OpaqueId,
    pub execution_id: OpaqueId,
    pub session_id: OpaqueId,
    pub binding_id: OpaqueId,
    pub generation: DecimalU64,
    pub request_fingerprint: String,
    pub intent_kind: String,
    pub acceptance_state: AcceptanceState,
    #[serde(deserialize_with = "required_nullable")]
    pub durable_receipt_id: Option<OpaqueId>,
    #[serde(deserialize_with = "required_nullable")]
    pub native_receipt: Option<Value>,
}

impl Validate for Delivery {
    fn validate(&self) -> Result<(), &'static str> {
        require_nonempty(&self.request_fingerprint, "EMPTY_REQUEST_FINGERPRINT")?;
        require_nonempty(&self.intent_kind, "EMPTY_INTENT_KIND")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicEvent {
    pub schema_version: u64,
    pub event_id: OpaqueId,
    pub session_id: OpaqueId,
    #[serde(deserialize_with = "required_nullable")]
    pub execution_id: Option<OpaqueId>,
    #[serde(deserialize_with = "required_nullable")]
    pub binding_id: Option<OpaqueId>,
    pub generation: DecimalU64,
    pub stream_epoch: DecimalU64,
    pub sequence: DecimalU64,
    pub kind: PublicEventKind,
    pub payload: Value,
}

impl Validate for PublicEvent {
    fn validate(&self) -> Result<(), &'static str> {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutableId {
    pub path: String,
    pub hash: String,
    pub version: String,
    pub platform: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityEvidence {
    pub state: CapabilityState,
    pub reason: String,
    #[serde(deserialize_with = "required_nullable")]
    pub evidence_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilitySnapshot {
    pub schema_version: u64,
    pub snapshot_id: OpaqueId,
    pub instance_id: OpaqueId,
    pub executable_id: ExecutableId,
    pub mode: String,
    pub profile_revision: DecimalU64,
    pub auth_revision: DecimalU64,
    pub observed_at: UtcTimestamp,
    pub expires_at: UtcTimestamp,
    pub evidence_source: String,
    pub capabilities: BTreeMap<String, CapabilityEvidence>,
}

impl Validate for CapabilitySnapshot {
    fn validate(&self) -> Result<(), &'static str> {
        require_nonempty(&self.mode, "EMPTY_CAPABILITY_MODE")?;
        require_nonempty(&self.evidence_source, "EMPTY_EVIDENCE_SOURCE")?;
        if self.capabilities.keys().any(|name| name.is_empty()) {
            return Err("EMPTY_CAPABILITY_NAME");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextItem {
    #[serde(rename = "ref")]
    pub reference: String,
    pub digest: String,
    pub visibility: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextPackage {
    pub schema_version: u64,
    pub package_id: OpaqueId,
    pub recipient: String,
    pub scope_id: OpaqueId,
    pub domain_id: OpaqueId,
    pub task_id: String,
    pub source_version: String,
    pub items: Vec<ContextItem>,
    pub permission_ceiling: BTreeMap<String, Value>,
    pub expires_at: UtcTimestamp,
}

impl Validate for ContextPackage {
    fn validate(&self) -> Result<(), &'static str> {
        if self.items.len() > MAX_CONTEXT_ITEMS {
            return Err("CONTEXT_ITEM_LIMIT_EXCEEDED");
        }
        require_nonempty(&self.recipient, "EMPTY_RECIPIENT")?;
        require_nonempty(&self.task_id, "EMPTY_TASK_ID")?;
        require_nonempty(&self.source_version, "EMPTY_SOURCE_VERSION")?;
        for item in &self.items {
            require_nonempty(&item.reference, "EMPTY_CONTEXT_REF")?;
            require_nonempty(&item.digest, "EMPTY_CONTEXT_DIGEST")?;
            require_nonempty(&item.visibility, "EMPTY_CONTEXT_VISIBILITY")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HumanActionRequest {
    pub schema_version: u64,
    pub request_id: OpaqueId,
    pub execution_id: OpaqueId,
    pub binding_id: OpaqueId,
    pub generation: DecimalU64,
    pub continuation_id: String,
    pub domain_id: OpaqueId,
    pub expires_at: UtcTimestamp,
    pub allowed_answers: Vec<String>,
    pub permission_ceiling: BTreeMap<String, Value>,
    pub state: HumanActionState,
}

impl Validate for HumanActionRequest {
    fn validate(&self) -> Result<(), &'static str> {
        require_nonempty(&self.continuation_id, "EMPTY_CONTINUATION_ID")?;
        if self.allowed_answers.is_empty() || self.allowed_answers.iter().any(String::is_empty) {
            return Err("INVALID_ALLOWED_ANSWERS");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessIdentity {
    pub schema_version: u64,
    pub host_id: OpaqueId,
    pub instance_id: OpaqueId,
    pub process_handle_ref: String,
    #[serde(deserialize_with = "deserialize_pid")]
    pub pid: u32,
    #[serde(deserialize_with = "required_nullable")]
    pub started_at_ticks: Option<DecimalU64>,
    pub launch_epoch: DecimalU64,
    pub image_digest: String,
    #[serde(deserialize_with = "required_nullable")]
    pub containment_id: Option<String>,
}

fn deserialize_pid<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match Value::deserialize(deserializer)? {
        Value::Number(number) => number
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| serde::de::Error::custom("PID_VALUE_OVERFLOW")),
        _ => Err(serde::de::Error::custom("PID_SCHEMA")),
    }
}

impl Validate for ProcessIdentity {
    fn validate(&self) -> Result<(), &'static str> {
        require_nonempty(&self.process_handle_ref, "EMPTY_PROCESS_HANDLE_REF")?;
        require_nonempty(&self.image_digest, "EMPTY_IMAGE_DIGEST")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnershipIssue {
    pub code: ErrorCode,
    pub stage: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnershipResult {
    pub schema_version: u64,
    pub observation: ObservationState,
    pub in_memory_state: RecordedState,
    pub persisted_state: RecordedState,
    pub durability_ack: DurabilityAckState,
    pub custody_state: CustodyState,
    pub errors: Vec<OwnershipIssue>,
}

impl Validate for OwnershipResult {
    fn validate(&self) -> Result<(), &'static str> {
        Ok(())
    }
}

fn require_nonempty(value: &str, code: &'static str) -> Result<(), &'static str> {
    if value.is_empty() {
        Err(code)
    } else {
        Ok(())
    }
}

fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}
