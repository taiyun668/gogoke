//! Native-only Design 37 request framing. The existing Node service pipe stays unwired.
mod wire;
mod admission;
mod journal;
mod seat_io;
pub(crate) mod runtime;

pub(crate) use wire::{decode_receipt, decode_request, encode_receipt, V37Receipt, V37Request,
    V37Status, V37WireError};
pub(crate) use journal::{complete_stdin_request, mark_stdin_write_unknown,
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
    revoke_owner_binding_in_transaction, AdmissionError, AdmissionRequest,
    AdmissionResult, OwnerBinding, TrustedLimits};
