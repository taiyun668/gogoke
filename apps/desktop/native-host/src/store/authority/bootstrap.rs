//! Trusted native bootstrap: no caller-supplied issuer, principal or boolean.
use super::super::orchestration::OrchestrationError;
use super::super::same_open::VerifiedDatabaseConnection;
use super::model::{denied, identifier, revision};
use super::transaction::{self, Result, Transaction};
use crate::root::RootLock;
use std::ffi::c_void;

const SCHEMA_V1: &str = include_str!("schema_v1.sql");
const SCHEMA: &str = include_str!("schema.sql");

#[link(name = "bcrypt")]
unsafe extern "system" {
    fn BCryptGenRandom(algorithm: *mut c_void, buffer: *mut u8, length: u32, flags: u32) -> i32;
}

// Only this module can construct the root issuer capability. It is not serialized,
// sent to Node, returned over IPC, or reconstructed from a grant reference.
pub(crate) struct OwnerIssuer {
    profile_id: String,
    root_identity: String,
    principal_id: String,
    seat_id: String,
    issuer_id: String,
}

pub(super) struct Profile {
    pub profile_id: String,
    pub root_identity: String,
    pub principal_id: String,
    pub seat_id: String,
    pub issuer_id: String,
    pub policy_revision: String,
    pub revocation_head: String,
}

impl OwnerIssuer {
    pub(crate) fn principal_id(&self) -> &str {
        &self.principal_id
    }
    pub(crate) fn seat_id(&self) -> &str {
        &self.seat_id
    }

    pub(super) fn check(&self, current: &Profile) -> Result<()> {
        if self.profile_id != current.profile_id
            || self.root_identity != current.root_identity
            || self.principal_id != current.principal_id
            || self.seat_id != current.seat_id
            || self.issuer_id != current.issuer_id
        {
            return denied();
        }
        Ok(())
    }
}

pub(super) fn random_id(prefix: &str) -> Result<String> {
    let mut bytes = [0u8; 32];
    // NULL + BCRYPT_USE_SYSTEM_PREFERRED_RNG. An OS RNG failure has no fallback.
    let status = unsafe { BCryptGenRandom(std::ptr::null_mut(), bytes.as_mut_ptr(), 32, 2) };
    if status != 0 {
        return Err(OrchestrationError::Invalid(
            "authority operating-system randomness",
        ));
    }
    let hex = bytes.iter().map(|x| format!("{x:02x}")).collect::<String>();
    Ok(format!("{prefix}:{hex}"))
}

// Compare lexical tokens, not concatenated text. Whitespace inside a quoted
// literal/identifier is significant, and boundaries between bare words matter.
// This is deliberately conservative schema admission, not a general SQL parser.
fn compact(sql: &str) -> Vec<String> {
    fn word(ch: char) -> bool {
        ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' || !ch.is_ascii()
    }
    let mut tokens = Vec::new();
    let mut chars = sql.chars().peekable();
    while let Some(ch) = chars.next() {
        if matches!(ch, ' ' | '\t' | '\r' | '\n' | '\u{000c}') {
            continue;
        }
        let mut token = ch.to_string();
        if matches!(ch, '\'' | '"' | '`' | '[') {
            let close = if ch == '[' { ']' } else { ch };
            while let Some(next) = chars.next() {
                token.push(next);
                if next == close {
                    if close != ']' && chars.peek() == Some(&close) {
                        token.push(chars.next().expect("peeked quote"));
                    } else {
                        break;
                    }
                }
            }
        } else if word(ch) {
            while chars.peek().is_some_and(|next| word(*next)) {
                token.push(chars.next().expect("peeked word character"));
            }
        }
        tokens.push(token);
    }
    tokens
}

#[cfg(test)]
mod schema_comparison_tests {
    use super::{compact, SCHEMA};

    #[test]
    fn formatting_whitespace_is_not_a_schema_change() {
        assert_eq!(compact(SCHEMA), compact(&SCHEMA.replace("    ", "\t")));
        assert_eq!(compact("x='one two'"), compact("x = 'one two'"));
    }

    #[test]
    fn quoted_permission_whitespace_changes_the_schema() {
        let changed = SCHEMA.replace("'context.read'", "'context. read'");
        assert_ne!(compact(SCHEMA), compact(&changed));
    }

    #[test]
    fn quoted_destination_scope_whitespace_changes_the_schema() {
        let changed = SCHEMA.replace("'GLOBAL'", "'GLO BAL'");
        assert_ne!(compact(SCHEMA), compact(&changed));
    }

    #[test]
    fn quoted_event_kind_whitespace_changes_the_schema() {
        let changed = SCHEMA.replace("'ISSUE'", "'IS SUE'");
        assert_ne!(compact(SCHEMA), compact(&changed));
    }

    #[test]
    fn quoted_schema_revision_whitespace_changes_the_schema() {
        let changed = SCHEMA.replace("schema_revision='2'", "schema_revision=' 2 '");
        assert_ne!(compact(SCHEMA), compact(&changed));
    }

    #[test]
    fn word_boundaries_and_escaped_literal_whitespace_are_preserved() {
        assert_ne!(compact("TEXT NOT NULL"), compact("TEXTNOTNULL"));
        assert_ne!(compact("x='two  spaces'"), compact("x='two spaces'"));
        assert_ne!(compact("x='a''  b'"), compact("x='a'' b'"));
        assert_ne!(compact("\"a b\""), compact("\"ab\""));
        assert_ne!(compact("[a b]"), compact("[ab]"));
    }
}

#[cfg(test)]
#[path = "bootstrap_migration_tests.rs"]
mod migration_tests;

fn verify_schema_version(tx: &mut Transaction<'_, '_>, schema: &str) -> Result<()> {
    let expected: Vec<&str> = schema
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    verify_temp_authority_placement(tx)?;
    let main = tx.query(
        "SELECT type,name,sql FROM main.sqlite_schema WHERE lower(substr(name,1,17))='gogoke_authority_' ORDER BY name",
        &[], 3)?;
    if main.len() != expected.len() {
        return denied();
    }
    for sql in expected {
        let name = sql
            .split_whitespace()
            .nth(2)
            .ok_or(OrchestrationError::AccessDenied)?;
        if !main
            .iter()
            .any(|row| row[0] == "table" && row[1] == name && compact(&row[2]) == compact(sql))
        {
            return denied();
        }
    }
    let main_triggers = tx.query(
        "SELECT t.name FROM main.sqlite_schema AS t WHERE t.type='trigger' AND EXISTS (SELECT 1 FROM main.sqlite_schema AS m WHERE m.type='table' AND lower(substr(m.name,1,17))='gogoke_authority_' AND t.tbl_name=m.name COLLATE NOCASE) LIMIT 1",
        &[], 1)?;
    if !main_triggers.is_empty() {
        return denied();
    }
    Ok(())
}

fn verify_temp_authority_placement(tx: &mut Transaction<'_, '_>) -> Result<()> {
    let shadows = tx.query(
        "SELECT name FROM temp.sqlite_schema WHERE lower(substr(name,1,17))='gogoke_authority_' LIMIT 1",
        &[], 1)?;
    if !shadows.is_empty() {
        return denied();
    }
    // TEMP trigger names are arbitrary. Inspect every TEMP trigger's resolved
    // target against all authority tables in main, case-insensitively.
    let triggers = tx.query(
        "SELECT t.name FROM temp.sqlite_schema AS t WHERE t.type='trigger' AND EXISTS (SELECT 1 FROM main.sqlite_schema AS m WHERE m.type='table' AND lower(substr(m.name,1,17))='gogoke_authority_' AND t.tbl_name=m.name COLLATE NOCASE) LIMIT 1",
        &[], 1)?;
    if !triggers.is_empty() {
        return denied();
    }
    Ok(())
}

pub(super) fn verify_schema(tx: &mut Transaction<'_, '_>) -> Result<()> {
    verify_schema_version(tx, SCHEMA)?;
    validate_foreign_keys(tx)?;
    validate_payload_kind_invariants(tx)
}

pub(super) fn verify_schema_v1(tx: &mut Transaction<'_, '_>) -> Result<()> {
    verify_schema_version(tx, SCHEMA_V1)
}

pub(super) fn profile(tx: &mut Transaction<'_, '_>) -> Result<Profile> {
    let rows = tx.query(
        "SELECT profile_id,root_identity,owner_principal_id,owner_seat_id,issuer_id,policy_revision,revocation_head,schema_revision FROM main.gogoke_authority_profile WHERE singleton=1",
        &[], 8)?;
    let row = rows.first().ok_or(OrchestrationError::AccessDenied)?;
    if rows.len() != 1 || row[7] != "2" || row[1] != tx.root_identity() {
        return denied();
    }
    for index in [0, 2, 3, 4] {
        identifier(&row[index])?;
    }
    revision(&row[5])?;
    revision(&row[6])?;
    Ok(Profile {
        profile_id: row[0].clone(),
        root_identity: row[1].clone(),
        principal_id: row[2].clone(),
        seat_id: row[3].clone(),
        issuer_id: row[4].clone(),
        policy_revision: row[5].clone(),
        revocation_head: row[6].clone(),
    })
}

pub(crate) fn initialize_profile(
    connection: &mut VerifiedDatabaseConnection<'_>,
    root: &RootLock,
) -> Result<OwnerIssuer> {
    // A RootLock argument is not sufficient: it must be the root retained by
    // this exact same-open database pin, before any schema or issuer is created.
    if connection.root_identity() != &root.canonical_root().identity {
        return denied();
    }
    let root_identity = connection.root_identity().opaque();
    transaction::run(connection, |tx| {
        verify_temp_authority_placement(tx)?;
        let objects = tx.query(
            "SELECT name FROM main.sqlite_schema WHERE lower(substr(name,1,17))='gogoke_authority_' LIMIT 1",
            &[], 1)?;
        if objects.is_empty() {
            create_schema_v2(tx)?;
            create_profile(tx, &root_identity)?;
        } else {
            let version = tx.query(
                "SELECT schema_revision FROM main.gogoke_authority_profile WHERE singleton=1",
                &[],
                1,
            );
            match version {
                Ok(rows) if rows.len() == 1 && rows[0][0] == "1" => {
                    verify_schema_v1(tx)?;
                    validate_v1_profile(tx, &root_identity)?;
                    validate_foreign_keys(tx)?;
                    migrate_v1_to_v2(tx)?;
                }
                Ok(rows) if rows.len() == 1 && rows[0][0] == "2" => verify_schema(tx)?,
                _ => return denied(),
            }
        }
        // Partial schema, malformed preflight, or missing profile is corruption,
        // never a new identity. v1 migration and v2 verification share this txn.
        verify_schema(tx)?;
        let current = profile(tx)?;
        if current.root_identity != root_identity {
            return denied();
        }
        Ok(OwnerIssuer {
            profile_id: current.profile_id,
            root_identity: current.root_identity,
            principal_id: current.principal_id,
            seat_id: current.seat_id,
            issuer_id: current.issuer_id,
        })
    })
}

fn create_schema_v2(tx: &mut Transaction<'_, '_>) -> Result<()> {
    for sql in SCHEMA.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        tx.write(sql, &[])?;
    }
    Ok(())
}

fn create_profile(tx: &mut Transaction<'_, '_>, root_identity: &str) -> Result<()> {
    let profile_id = random_id("profile")?;
    let principal_id = random_id("owner")?;
    let seat_id = random_id("owner-seat")?;
    let issuer_id = random_id("issuer")?;
    tx.write(
        "INSERT INTO main.gogoke_authority_profile(singleton,schema_revision,profile_id,root_identity,owner_principal_id,owner_seat_id,issuer_id,policy_revision,revocation_head) VALUES(1,'2',?,?,?,?,?,'1','0')",
        &[&profile_id, root_identity, &principal_id, &seat_id, &issuer_id])?;
    tx.write(
        "INSERT INTO main.gogoke_authority_events(event_kind,issuer_id,grant_id,grant_revision,policy_revision,revocation_head) VALUES('BOOTSTRAP',?,'','0','1','0')",
        &[&issuer_id])
}

pub(super) fn validate_v1_profile(tx: &mut Transaction<'_, '_>, root_identity: &str) -> Result<()> {
    let rows = tx.query(
        "SELECT profile_id,root_identity,owner_principal_id,owner_seat_id,issuer_id,policy_revision,revocation_head,schema_revision FROM main.gogoke_authority_profile WHERE singleton=1",
        &[], 8)?;
    let row = rows.first().ok_or(OrchestrationError::AccessDenied)?;
    if rows.len() != 1 || row[7] != "1" || row[1] != root_identity {
        return denied();
    }
    for index in [0, 2, 3, 4] {
        identifier(&row[index])?;
    }
    revision(&row[5])?;
    revision(&row[6])?;
    Ok(())
}

pub(super) fn validate_foreign_keys(tx: &mut Transaction<'_, '_>) -> Result<()> {
    let enabled = tx.query("PRAGMA foreign_keys", &[], 1)?;
    if enabled.len() != 1 || enabled[0][0] != "1" {
        return denied();
    }
    let violations = tx.query("PRAGMA main.foreign_key_check", &[], 4)?;
    if !violations.is_empty() {
        return denied();
    }
    Ok(())
}

fn validate_payload_kind_invariants(tx: &mut Transaction<'_, '_>) -> Result<()> {
    let rows = tx.query(
        "SELECT EXISTS(SELECT 1 FROM main.gogoke_authority_grants AS g WHERE (g.grant_kind='CONTEXT' AND ((SELECT count(*) FROM main.gogoke_authority_context_grant_payloads AS c WHERE c.grant_id=g.grant_id AND c.revision=g.revision)<>1 OR (SELECT count(*) FROM main.gogoke_authority_delegation_grant_payloads AS d WHERE d.grant_id=g.grant_id AND d.revision=g.revision)<>0)) OR (g.grant_kind='DELEGATION' AND ((SELECT count(*) FROM main.gogoke_authority_context_grant_payloads AS c WHERE c.grant_id=g.grant_id AND c.revision=g.revision)<>0 OR (SELECT count(*) FROM main.gogoke_authority_delegation_grant_payloads AS d WHERE d.grant_id=g.grant_id AND d.revision=g.revision)<>1)) OR g.grant_kind NOT IN ('CONTEXT','DELEGATION'))",
        &[], 1)?;
    if rows.len() != 1 || rows[0][0] != "0" {
        return denied();
    }
    Ok(())
}

fn migration_equal(tx: &mut Transaction<'_, '_>, sql: &str) -> Result<()> {
    let rows = tx.query(sql, &[], 1)?;
    if rows.len() != 1 || rows[0][0] != "0" {
        return denied();
    }
    Ok(())
}

pub(super) fn migrate_v1_to_v2(tx: &mut Transaction<'_, '_>) -> Result<()> {
    // Existing v1 grant rows may reference older rows in the same table. Defer
    // constraints only until this transaction ends; the complete v2 graph is
    // checked before commit and foreign_keys remains enabled throughout.
    tx.write("PRAGMA defer_foreign_keys=ON", &[])?;
    // Rename all four v1 tables inside the active BEGIN IMMEDIATE. SQLite keeps
    // the original FK graph on the renamed objects while the v2 graph is built.
    for (old, renamed) in [
        ("gogoke_authority_grants", "gogoke_authority_grants_v1"),
        (
            "gogoke_authority_grant_heads",
            "gogoke_authority_grant_heads_v1",
        ),
        ("gogoke_authority_events", "gogoke_authority_events_v1"),
        ("gogoke_authority_profile", "gogoke_authority_profile_v1"),
    ] {
        tx.write(&format!("ALTER TABLE main.{old} RENAME TO {renamed}"), &[])?;
    }
    create_schema_v2(tx)?;
    tx.write(
        "INSERT INTO main.gogoke_authority_profile(singleton,schema_revision,profile_id,root_identity,owner_principal_id,owner_seat_id,issuer_id,policy_revision,revocation_head) SELECT singleton,'2',profile_id,root_identity,owner_principal_id,owner_seat_id,issuer_id,policy_revision,revocation_head FROM main.gogoke_authority_profile_v1", &[])?;
    tx.write(
        "INSERT INTO main.gogoke_authority_grants(grant_id,revision,grant_kind,issuer_id,parent_grant_id,parent_revision,policy_revision,issued_revocation_head) SELECT grant_id,revision,'CONTEXT',issuer_id,parent_grant_id,parent_revision,policy_revision,issued_revocation_head FROM main.gogoke_authority_grants_v1", &[])?;
    tx.write(
        "INSERT INTO main.gogoke_authority_context_grant_payloads(grant_id,revision,principal_id,seat_id,permission,promotion_kind,source_domain_id,destination_domain_id,destination_scope,delegable_depth) SELECT grant_id,revision,principal_id,seat_id,permission,promotion_kind,source_domain_id,destination_domain_id,destination_scope,delegable_depth FROM main.gogoke_authority_grants_v1", &[])?;
    tx.write(
        "INSERT INTO main.gogoke_authority_grant_heads(grant_id,revision,revoked) SELECT grant_id,revision,revoked FROM main.gogoke_authority_grant_heads_v1", &[])?;
    tx.write(
        "INSERT INTO main.gogoke_authority_events(event_id,event_kind,issuer_id,grant_id,grant_revision,policy_revision,revocation_head) SELECT event_id,event_kind,issuer_id,grant_id,grant_revision,policy_revision,revocation_head FROM main.gogoke_authority_events_v1", &[])?;

    migration_equal(tx, "SELECT EXISTS(SELECT grant_id,revision,'CONTEXT',issuer_id,parent_grant_id,parent_revision,policy_revision,issued_revocation_head FROM main.gogoke_authority_grants_v1 EXCEPT SELECT grant_id,revision,grant_kind,issuer_id,parent_grant_id,parent_revision,policy_revision,issued_revocation_head FROM main.gogoke_authority_grants) OR EXISTS(SELECT grant_id,revision,grant_kind,issuer_id,parent_grant_id,parent_revision,policy_revision,issued_revocation_head FROM main.gogoke_authority_grants EXCEPT SELECT grant_id,revision,'CONTEXT',issuer_id,parent_grant_id,parent_revision,policy_revision,issued_revocation_head FROM main.gogoke_authority_grants_v1)")?;
    migration_equal(tx, "SELECT EXISTS(SELECT grant_id,revision,principal_id,seat_id,permission,promotion_kind,source_domain_id,destination_domain_id,destination_scope,delegable_depth FROM main.gogoke_authority_grants_v1 EXCEPT SELECT grant_id,revision,principal_id,seat_id,permission,promotion_kind,source_domain_id,destination_domain_id,destination_scope,delegable_depth FROM main.gogoke_authority_context_grant_payloads) OR EXISTS(SELECT grant_id,revision,principal_id,seat_id,permission,promotion_kind,source_domain_id,destination_domain_id,destination_scope,delegable_depth FROM main.gogoke_authority_context_grant_payloads EXCEPT SELECT grant_id,revision,principal_id,seat_id,permission,promotion_kind,source_domain_id,destination_domain_id,destination_scope,delegable_depth FROM main.gogoke_authority_grants_v1)")?;
    migration_equal(tx, "SELECT EXISTS(SELECT grant_id,revision,revoked FROM main.gogoke_authority_grant_heads_v1 EXCEPT SELECT grant_id,revision,revoked FROM main.gogoke_authority_grant_heads) OR EXISTS(SELECT grant_id,revision,revoked FROM main.gogoke_authority_grant_heads EXCEPT SELECT grant_id,revision,revoked FROM main.gogoke_authority_grant_heads_v1)")?;
    migration_equal(tx, "SELECT EXISTS(SELECT event_id,event_kind,issuer_id,grant_id,grant_revision,policy_revision,revocation_head FROM main.gogoke_authority_events_v1 EXCEPT SELECT event_id,event_kind,issuer_id,grant_id,grant_revision,policy_revision,revocation_head FROM main.gogoke_authority_events) OR EXISTS(SELECT event_id,event_kind,issuer_id,grant_id,grant_revision,policy_revision,revocation_head FROM main.gogoke_authority_events EXCEPT SELECT event_id,event_kind,issuer_id,grant_id,grant_revision,policy_revision,revocation_head FROM main.gogoke_authority_events_v1)")?;
    migration_equal(tx, "SELECT EXISTS(SELECT singleton,profile_id,root_identity,owner_principal_id,owner_seat_id,issuer_id,policy_revision,revocation_head FROM main.gogoke_authority_profile_v1 EXCEPT SELECT singleton,profile_id,root_identity,owner_principal_id,owner_seat_id,issuer_id,policy_revision,revocation_head FROM main.gogoke_authority_profile) OR EXISTS(SELECT singleton,profile_id,root_identity,owner_principal_id,owner_seat_id,issuer_id,policy_revision,revocation_head FROM main.gogoke_authority_profile EXCEPT SELECT singleton,profile_id,root_identity,owner_principal_id,owner_seat_id,issuer_id,policy_revision,revocation_head FROM main.gogoke_authority_profile_v1)")?;
    validate_foreign_keys(tx)?;

    for table in [
        "gogoke_authority_grant_heads_v1",
        "gogoke_authority_grants_v1",
        "gogoke_authority_events_v1",
        "gogoke_authority_profile_v1",
    ] {
        tx.write(&format!("DROP TABLE main.{table}"), &[])?;
    }
    validate_foreign_keys(tx)?;
    verify_schema(tx)
}
