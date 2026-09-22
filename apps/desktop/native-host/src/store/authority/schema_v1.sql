CREATE TABLE gogoke_authority_profile (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    schema_revision TEXT NOT NULL CHECK(schema_revision='1'),
    profile_id TEXT NOT NULL UNIQUE,
    root_identity TEXT NOT NULL,
    owner_principal_id TEXT NOT NULL,
    owner_seat_id TEXT NOT NULL,
    issuer_id TEXT NOT NULL UNIQUE,
    policy_revision TEXT NOT NULL,
    revocation_head TEXT NOT NULL
) STRICT;
CREATE TABLE gogoke_authority_grants (
    grant_id TEXT NOT NULL,
    revision TEXT NOT NULL,
    principal_id TEXT NOT NULL,
    seat_id TEXT NOT NULL,
    permission TEXT NOT NULL CHECK(permission IN ('context.read','context.promote.source','context.promote.target')),
    promotion_kind TEXT NOT NULL,
    source_domain_id TEXT NOT NULL,
    destination_domain_id TEXT NOT NULL,
    destination_scope TEXT NOT NULL CHECK(destination_scope IN ('GLOBAL','PROJECT','SESSION')),
    issuer_id TEXT NOT NULL,
    parent_grant_id TEXT,
    parent_revision TEXT,
    policy_revision TEXT NOT NULL,
    issued_revocation_head TEXT NOT NULL,
    delegable_depth INTEGER NOT NULL CHECK(delegable_depth BETWEEN 0 AND 32),
    PRIMARY KEY(grant_id,revision),
    CHECK((parent_grant_id IS NULL)=(parent_revision IS NULL)),
    FOREIGN KEY(parent_grant_id,parent_revision) REFERENCES gogoke_authority_grants(grant_id,revision) ON DELETE RESTRICT ON UPDATE RESTRICT
) STRICT;
CREATE TABLE gogoke_authority_grant_heads (
    grant_id TEXT PRIMARY KEY,
    revision TEXT NOT NULL,
    revoked INTEGER NOT NULL CHECK(revoked IN (0,1)),
    FOREIGN KEY(grant_id,revision) REFERENCES gogoke_authority_grants(grant_id,revision) ON DELETE RESTRICT ON UPDATE RESTRICT
) STRICT;
CREATE TABLE gogoke_authority_events (
    event_id INTEGER PRIMARY KEY,
    event_kind TEXT NOT NULL CHECK(event_kind IN ('BOOTSTRAP','ISSUE','REVISE','REVOKE')),
    issuer_id TEXT NOT NULL,
    grant_id TEXT NOT NULL,
    grant_revision TEXT NOT NULL,
    policy_revision TEXT NOT NULL,
    revocation_head TEXT NOT NULL
) STRICT;
