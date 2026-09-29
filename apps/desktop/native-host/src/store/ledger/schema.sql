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
COMMIT;
