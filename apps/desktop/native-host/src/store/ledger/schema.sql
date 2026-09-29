-- A.1 uses the already-open product database. Legacy event bodies remain in
-- orchestration_events; this index holds only their IDs and shared order.
BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS v37_ledger_meta (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    epoch TEXT NOT NULL
) STRICT;
INSERT OR IGNORE INTO v37_ledger_meta (singleton, epoch)
VALUES (1, lower(hex(randomblob(16))));

CREATE TABLE IF NOT EXISTS v37_ledger_index (
    cursor INTEGER PRIMARY KEY AUTOINCREMENT,
    source_event_id TEXT NOT NULL UNIQUE,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('legacy', 'v37')),
    source_cursor TEXT,
    source_epoch TEXT,
    domain_id TEXT,
    seat_id TEXT,
    session_id TEXT,
    tier TEXT CHECK (tier IN ('PROJECT', 'SEAT', 'SESSION', 'SIDE', 'GLOBAL')),
    side_id TEXT,
    occurred_at TEXT,
    update_json TEXT,
    CHECK (
        (source_kind = 'legacy' AND source_cursor IS NULL AND update_json IS NULL)
        OR
        (source_kind = 'v37' AND source_cursor IS NOT NULL AND source_epoch IS NOT NULL
         AND domain_id IS NOT NULL AND seat_id IS NOT NULL AND session_id IS NOT NULL
         AND tier IS NOT NULL AND occurred_at IS NOT NULL AND update_json IS NOT NULL
         AND ((tier = 'SIDE') = (side_id IS NOT NULL)))
    )
) STRICT;

-- A source cursor is contiguous within one adapter session epoch.  The
-- session is part of the key because a restarted adapter may reuse cursor 1
-- under a new epoch.  Tombstoned streams are retained after side-chat
-- deletion so an intentional deletion cannot be mistaken for a recovered
-- contiguous stream or accept a later append.
CREATE TABLE IF NOT EXISTS v37_ledger_source_stream (
    session_id TEXT NOT NULL,
    source_epoch TEXT NOT NULL,
    last_cursor INTEGER NOT NULL CHECK (last_cursor >= 1),
    state TEXT NOT NULL CHECK (state IN ('ACTIVE', 'TOMBSTONED')),
    PRIMARY KEY (session_id, source_epoch)
) STRICT;
INSERT OR IGNORE INTO v37_ledger_source_stream
    (session_id, source_epoch, last_cursor, state)
SELECT session_id, source_epoch, MAX(CAST(source_cursor AS INTEGER)), 'ACTIVE'
FROM v37_ledger_index
WHERE source_kind = 'v37'
GROUP BY session_id, source_epoch;

-- Keep the source index and its high-water row in one SQLite write.  The Rust
-- admission checks provide typed errors; these triggers also protect the
-- invariant if a future migration writes the index directly.
CREATE TRIGGER IF NOT EXISTS v37_ledger_source_stream_validate
BEFORE INSERT ON v37_ledger_index
WHEN NEW.source_kind = 'v37'
BEGIN
    SELECT CASE
        WHEN NOT EXISTS (
            SELECT 1 FROM v37_ledger_source_stream
            WHERE session_id = NEW.session_id AND source_epoch = NEW.source_epoch
        ) AND CAST(NEW.source_cursor AS INTEGER) <> 1
        THEN RAISE(ABORT, 'A.1 source stream must start at cursor 1')
        WHEN EXISTS (
            SELECT 1 FROM v37_ledger_source_stream
            WHERE session_id = NEW.session_id AND source_epoch = NEW.source_epoch
              AND (state <> 'ACTIVE' OR CAST(NEW.source_cursor AS INTEGER) <> last_cursor + 1)
        )
        THEN RAISE(ABORT, 'A.1 source stream cursor is not contiguous')
    END;
END;
CREATE TRIGGER IF NOT EXISTS v37_ledger_source_stream_advance
AFTER INSERT ON v37_ledger_index
WHEN NEW.source_kind = 'v37'
BEGIN
    INSERT INTO v37_ledger_source_stream
        (session_id, source_epoch, last_cursor, state)
    VALUES (NEW.session_id, NEW.source_epoch, CAST(NEW.source_cursor AS INTEGER), 'ACTIVE')
    ON CONFLICT (session_id, source_epoch) DO UPDATE SET
        last_cursor = excluded.last_cursor
    WHERE v37_ledger_source_stream.state = 'ACTIVE';
END;

-- Provider stdout is retained only as an internal recovery journal.  It has
-- no ledger cursor of its own and is never joined by the user-facing query
-- or receipt paths.  The source cursor is the adapter's cursor, while the
-- operation/ticket/session/generation columns bind one exact frame to H's
-- durable process custody. A valid protocol reply/notification without a
-- normalized K-LEDGER event is terminal NO_EVENT with a bounded reason code.
CREATE TABLE IF NOT EXISTS v37_ledger_raw_source (
    operation_id TEXT NOT NULL,
    process_ticket TEXT NOT NULL,
    custodian_nonce TEXT NOT NULL,
    domain_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    generation TEXT NOT NULL,
    source_epoch TEXT NOT NULL,
    source_cursor TEXT NOT NULL CHECK (
        length(source_cursor) BETWEEN 1 AND 20
        AND source_cursor NOT GLOB '*[^0-9]*'
        AND source_cursor <> '0'
        AND substr(source_cursor, 1, 1) <> '0'
    ),
    raw_bytes BLOB NOT NULL CHECK (
        length(raw_bytes) BETWEEN 1 AND 1048576
        AND substr(raw_bytes, -1, 1) = X'0A'
    ),
    state TEXT NOT NULL CHECK (state IN ('PENDING', 'RESOLVED', 'NO_EVENT')),
    resolved_event_id TEXT,
    no_event_reason TEXT,
    PRIMARY KEY (operation_id, source_epoch, source_cursor),
    UNIQUE (process_ticket, source_epoch, source_cursor),
    CHECK (
        (state = 'PENDING' AND resolved_event_id IS NULL AND no_event_reason IS NULL)
        OR
        (state = 'RESOLVED' AND resolved_event_id IS NOT NULL AND no_event_reason IS NULL)
        OR
        (state = 'NO_EVENT' AND resolved_event_id IS NULL
         AND no_event_reason IS NOT NULL
         AND length(no_event_reason) BETWEEN 1 AND 128)
    )
) STRICT;

-- After the initial backfill, every old write receives a cursor inside its
-- original transaction. This table is a reference index, not a second event log.
INSERT OR IGNORE INTO v37_ledger_index (source_event_id, source_kind)
SELECT event_id, 'legacy' FROM orchestration_events ORDER BY sequence;
CREATE TRIGGER IF NOT EXISTS v37_ledger_legacy_insert
AFTER INSERT ON orchestration_events
BEGIN
    INSERT INTO v37_ledger_index (source_event_id, source_kind)
    VALUES (NEW.event_id, 'legacy');
END;

-- Native reconciliation must bind each old thread to an actual seat/session;
-- an unbound old event is not silently attributed to a made-up seat.
CREATE TABLE IF NOT EXISTS v37_ledger_legacy_binding (
    thread_id TEXT PRIMARY KEY,
    domain_id TEXT NOT NULL,
    seat_id TEXT NOT NULL,
    session_id TEXT NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS v37_ledger_subscription (
    subscription_id TEXT PRIMARY KEY,
    reader_kind TEXT NOT NULL CHECK (reader_kind IN ('PROJECT', 'SIDE', 'GLOBAL')),
    domain_id TEXT,
    reader_id TEXT,
    cursor INTEGER NOT NULL CHECK (cursor >= 0),
    epoch TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    state TEXT NOT NULL CHECK (state IN ('ACTIVE', 'ENDED'))
) STRICT;

CREATE TABLE IF NOT EXISTS v37_ledger_receipt (
    family TEXT NOT NULL,
    domain_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    request_bytes BLOB NOT NULL,
    receipt_bytes BLOB NOT NULL,
    PRIMARY KEY (family, domain_id, request_id)
) STRICT;

-- Session purpose is fixed at registration. A formal review must start with a
-- fresh native session and cannot gain an old ledger source by resume/fork.
CREATE TABLE IF NOT EXISTS v37_ledger_session (
    session_id TEXT PRIMARY KEY,
    domain_id TEXT NOT NULL,
    seat_id TEXT NOT NULL,
    purpose TEXT NOT NULL CHECK (purpose IN ('WORK', 'HANDOFF', 'SIDE_CHAT', 'FORMAL_REVIEW')),
    side_id TEXT,
    CHECK ((purpose = 'SIDE_CHAT') = (side_id IS NOT NULL))
) STRICT;
CREATE INDEX IF NOT EXISTS v37_ledger_scope_cursor
ON v37_ledger_index(domain_id, cursor);
COMMIT;
