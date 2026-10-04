//! Native-only Design 37 request framing. The existing Node service pipe stays unwired.
mod wire;
mod admission;
mod episodes;
pub(crate) mod generation_change;
mod journal;
mod seat_io;
pub(crate) mod launch;
mod codex_component;
pub(crate) mod codex_rpc;
pub(crate) mod codex_output;
pub(crate) mod rpc_journal;
pub(crate) mod model_call;
pub(crate) mod host_health;
pub(crate) mod runtime;
pub(crate) mod provider_evidence;

pub(crate) use wire::{decode_receipt, decode_request, encode_receipt, V37Receipt, V37Request,
    V37Status, V37WireError};
pub(crate) use journal::{complete_stdin_request, mark_stdin_write_unknown,
    ClaudeSendInput, ClaudeSendIdentity, prepare_claude_send_request,
    mark_claude_send_written, mark_claude_send_write_unknown,
    observe_claude_send_echo_from_source, complete_claude_send_from_source,
    read_original_claude_send_completed,
    AcpSendInput, AcpSendIdentity, prepare_acp_send_request,
    complete_acp_send_from_source, read_acp_send_completed,
    mark_acp_send_written, mark_acp_send_write_unknown,
    complete_codex_turn_request,
    recover_codex_turn_request,
    reconcile_observed_codex_sends,
    prepare_codex_request,
    mark_codex_write_unknown,
    prepare_stdin_request, read_stdin_journal, JournalDecision, JournalError,
    JournalState, PrepareDisposition, StdinJournalKey, StdinJournalRecord, StdinRequest};
pub(crate) use seat_io::{run_seat_io, SeatChannelId, SeatIoAdmission, SeatIoError,
    SeatIoEvent, SeatIoReply};
pub(crate) use admission::{initialize_admission_schema, bind_owner_in_transaction,
    verify_home_owner_in_transaction, verify_home_stop_in_transaction,
    fence_home_admission_in_transaction, reserve_admission, commit_admission,
    release_admission, bind_process_operation_in_transaction,
    mark_start_unknown_in_transaction,
    record_session_stop_in_transaction,
    record_generation_change_stop_in_transaction,
    revoke_owner_binding_in_transaction, AdmissionError, AdmissionRequest,
    AdmissionResult, OwnerBinding, TrustedLimits};
pub(crate) use episodes::{record_initial, mark_active, begin_resume,
    attach_resume_process, mark_resume_unknown, promote_resume,
    mark_stopped as mark_episode_stopped};
