CREATE TABLE gogoke_v37_side_registry (
    domain_id TEXT NOT NULL,
    side_id TEXT NOT NULL,
    revision TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('ACTIVE','ARCHIVED','DELETED')),
    source_seat_id TEXT NOT NULL,
    source_seat_incarnation TEXT NOT NULL,
    source_session_id TEXT NOT NULL,
    seat_id TEXT NOT NULL,
    seat_incarnation TEXT NOT NULL,
    session_id TEXT NOT NULL,
    binding_generation TEXT NOT NULL,
    binding_instance_id TEXT NOT NULL,
    binding_process_operation_id TEXT NOT NULL,
    source_epoch TEXT NOT NULL,
    source_cursor TEXT NOT NULL,
    synced_cursor TEXT NOT NULL,
    PRIMARY KEY(domain_id,side_id),
    UNIQUE(session_id)
) STRICT;
CREATE TABLE gogoke_v37_side_operations (
    domain_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    request_hex TEXT NOT NULL,
    receipt TEXT NOT NULL,
    observed_epoch TEXT NOT NULL,
    observed_cursor TEXT NOT NULL,
    PRIMARY KEY(domain_id,request_id)
) STRICT;
CREATE TABLE gogoke_v37_side_pending (
    domain_id TEXT NOT NULL,
    side_id TEXT NOT NULL,
    epoch TEXT NOT NULL,
    after_cursor TEXT NOT NULL,
    through_cursor TEXT NOT NULL,
    PRIMARY KEY(domain_id,side_id,epoch,after_cursor),
    FOREIGN KEY(domain_id,side_id) REFERENCES gogoke_v37_side_registry(domain_id,side_id)
) STRICT;
CREATE TABLE gogoke_v37_side_sync (
    domain_id TEXT NOT NULL,
    sync_id TEXT NOT NULL,
    side_id TEXT NOT NULL,
    request_digest TEXT NOT NULL,
    origin_request_digest TEXT NOT NULL CHECK(length(origin_request_digest)=64 AND origin_request_digest NOT GLOB '*[^0-9a-f]*'),
    mode TEXT NOT NULL CHECK(mode IN ('APPEND','QUESTION')),
    session_id TEXT NOT NULL,
    process_operation_id TEXT NOT NULL,
    generation TEXT NOT NULL,
    epoch TEXT NOT NULL,
    after_cursor TEXT NOT NULL,
    through_cursor TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('PREPARED','UNKNOWN','DELIVERED','FAILED')),
    native_receipt_id TEXT NOT NULL,
    PRIMARY KEY(domain_id,sync_id),
    FOREIGN KEY(domain_id,side_id) REFERENCES gogoke_v37_side_registry(domain_id,side_id)
) STRICT;
