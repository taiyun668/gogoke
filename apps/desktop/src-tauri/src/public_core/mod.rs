//! Provider-neutral public contract for the S1 execution path.
//!
//! This module is deliberately IO-free. It must not depend on Tauri, provider
//! protocol types, provider home variables, process launch, or any native thread/turn
//! shape. The wire source of truth is `apps/desktop/contracts/s1/schema.json`.

mod codec;
mod error;
mod model;
mod scalar;
mod version;

pub use codec::{canonical_json, decode_json, CodecError, CodecErrorCode, PublicDocument};
pub use error::{
    CommandError, CommandFailure, CommandResponse, CommandSuccess, ErrorCode, SideEffectState,
};
pub use model::*;
pub use scalar::{DecimalU64, OpaqueId, UtcTimestamp};
pub use version::*;
