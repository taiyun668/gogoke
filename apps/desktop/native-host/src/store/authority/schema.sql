CREATE TABLE gogoke_authority_profile (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    schema_revision TEXT NOT NULL CHECK(schema_revision='2'),
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
    grant_kind TEXT NOT NULL CHECK(grant_kind IN ('CONTEXT','DELEGATION')),
    issuer_id TEXT NOT NULL,
    parent_grant_id TEXT,
    parent_revision TEXT,
    policy_revision TEXT NOT NULL,
    issued_revocation_head TEXT NOT NULL,
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
CREATE TABLE gogoke_authority_context_grant_payloads (
    grant_id TEXT NOT NULL,
    revision TEXT NOT NULL,
    principal_id TEXT NOT NULL,
    seat_id TEXT NOT NULL,
    permission TEXT NOT NULL CHECK(permission IN ('context.read','context.promote.source','context.promote.target')),
    promotion_kind TEXT NOT NULL,
    source_domain_id TEXT NOT NULL,
    destination_domain_id TEXT NOT NULL,
    destination_scope TEXT NOT NULL CHECK(destination_scope IN ('GLOBAL','PROJECT','SESSION')),
    delegable_depth INTEGER NOT NULL CHECK(delegable_depth BETWEEN 0 AND 32),
    PRIMARY KEY(grant_id,revision),
    FOREIGN KEY(grant_id,revision) REFERENCES gogoke_authority_grants(grant_id,revision) ON DELETE RESTRICT ON UPDATE RESTRICT
) STRICT;
CREATE TABLE gogoke_authority_delegation_grant_payloads (
    grant_id TEXT NOT NULL,
    revision TEXT NOT NULL,
    principal_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    domain_id TEXT NOT NULL,
    role TEXT NOT NULL,
    seat_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    execution_id TEXT NOT NULL,
    generation TEXT NOT NULL,
    expires_at_epoch_ms INTEGER NOT NULL CHECK(expires_at_epoch_ms BETWEEN 0 AND 9007199254740991),
    max_material_items INTEGER NOT NULL CHECK(max_material_items BETWEEN 0 AND 9007199254740991),
    max_material_bytes INTEGER NOT NULL CHECK(max_material_bytes BETWEEN 0 AND 9007199254740991),
    max_response_bytes INTEGER NOT NULL CHECK(max_response_bytes BETWEEN 0 AND 9007199254740991),
    PRIMARY KEY(grant_id,revision),
    FOREIGN KEY(grant_id,revision) REFERENCES gogoke_authority_grants(grant_id,revision) ON DELETE RESTRICT ON UPDATE RESTRICT
) STRICT;
CREATE TABLE gogoke_authority_delegation_ceiling_entries (
    grant_id TEXT NOT NULL,
    revision TEXT NOT NULL,
    axis TEXT NOT NULL CHECK(axis IN ('allowed_actions','allowed_target_principal_ids','allowed_target_domain_ids','allowed_sinks','allowed_material_classes','explicit_private_material_ids','allowed_continuation_responses')),
    ordinal INTEGER NOT NULL CHECK(ordinal>=0),
    value TEXT NOT NULL,
    PRIMARY KEY(grant_id,revision,axis,ordinal),
    FOREIGN KEY(grant_id,revision) REFERENCES gogoke_authority_delegation_grant_payloads(grant_id,revision) ON DELETE RESTRICT ON UPDATE RESTRICT
) STRICT;
