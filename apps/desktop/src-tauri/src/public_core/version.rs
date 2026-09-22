/// Only supported public schema major. There is no implicit legacy fallback.
pub const SCHEMA_VERSION: u64 = 1;

/// C02/C05 public frame ceiling.
pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
/// Appendix A title ceiling, measured as UTF-8 bytes.
pub const MAX_TITLE_BYTES: usize = 256;
/// Appendix A text content ceiling, measured as UTF-8 bytes.
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
/// Appendix A ContextPackage reference ceiling.
pub const MAX_CONTEXT_ITEMS: usize = 128;
/// C05 control request queue ceiling.
pub const MAX_CONTROL_QUEUE_ITEMS: usize = 64;
/// C05 ordinary per-Session request queue ceiling.
pub const MAX_SESSION_QUEUE_ITEMS: usize = 128;
/// C05 output cache item ceiling.
pub const MAX_OUTPUT_CACHE_ITEMS: usize = 1024;
/// C05 output cache byte ceiling.
pub const MAX_OUTPUT_CACHE_BYTES: usize = 8 * 1024 * 1024;
/// C04 append-only journal hard ceiling.
pub const MAX_JOURNAL_BYTES: u64 = 1024 * 1024 * 1024;
/// C04 control-record reserve within the journal ceiling.
pub const JOURNAL_CONTROL_RESERVE_BYTES: u64 = 16 * 1024 * 1024;
/// C08 conservative capability snapshot TTL.
pub const CAPABILITY_TTL_SECONDS: u64 = 5 * 60;
/// C07 default graceful-stop interval.
pub const STOP_GRACEFUL_SECONDS: u64 = 10;
/// C07 termination action interval.
pub const STOP_TERMINATION_SECONDS: u64 = 5;
/// C07 post-termination observation interval.
pub const STOP_OBSERVATION_SECONDS: u64 = 5;
/// C07 total monotonic stop budget.
pub const STOP_TOTAL_SECONDS: u64 = 30;
