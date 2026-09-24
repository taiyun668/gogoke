//! A bounded, test-only open RuntimeDriver identity. The registered identity
//! never supplies a path, command, model provider, or process argument.
use super::super::digest::content_hash;
use super::super::orchestration::OrchestrationError;
use super::super::same_open::VerifiedDatabaseConnection;
use super::model::denied;
use super::transaction::{self, Result, Transaction};

pub(crate) const FIXED_RUNTIME_INSTANCE_ID: &str = "runtime-r2-02-fixture";
pub(crate) const ADAPTER_VERSION: &str = "1.0.0";
// Must match the compile-pinned resource checked by controlled_fixture_request.
pub(crate) const LAUNCH_DIGEST_SHA256: &str =
    "sha256:2e66dac4ee497e023fd8d860178c77ef5b82e01b7bcd23dc868e9db80637f85c";
const DOMAIN_ID: &str = "domain-r2-02-test";
const SCHEMA: &str = "CREATE TABLE gogoke_r2_test_fixture_drivers (driver_id TEXT PRIMARY KEY,profile_id TEXT NOT NULL,adapter_version TEXT NOT NULL,runtime_instance_id TEXT NOT NULL UNIQUE,launch_digest_sha256 TEXT NOT NULL,content_hash TEXT NOT NULL) STRICT";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct R2FixtureDriverBinding {
    pub driver_id: String,
    pub adapter_version: String,
    pub runtime_instance_id: String,
    pub launch_digest_sha256: String,
    pub content_hash: String,
}

fn valid_driver_id(driver_id: &str) -> bool {
    driver_id.len() == 27
        && driver_id.starts_with("mock_novel_")
        && driver_id[11..].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn expected(driver_id: &str) -> Result<R2FixtureDriverBinding> {
    if !valid_driver_id(driver_id) {
        return denied();
    }
    let manifest = format!(
        "gogoke.r2-03.test-fixture-driver.v1\ndriverId={driver_id}\nadapterVersion={ADAPTER_VERSION}\ndomainId={DOMAIN_ID}\nkind=NON_MODEL\nlaunchResource=gogoke-service/fixtures/controlled-pi.mjs\nlaunchDigestSha256={LAUNCH_DIGEST_SHA256}\n"
    );
    let content_hash = content_hash(manifest.as_bytes());
    let runtime_instance_id = format!(
        "runtime-r2-03-{}-{}", &driver_id[11..], &content_hash[7..23]
    );
    Ok(R2FixtureDriverBinding {
        driver_id: driver_id.to_owned(),
        adapter_version: ADAPTER_VERSION.to_owned(),
        runtime_instance_id,
        launch_digest_sha256: LAUNCH_DIGEST_SHA256.to_owned(),
        content_hash,
    })
}

fn ensure_schema(tx: &mut Transaction<'_, '_>) -> Result<()> {
    let name = "gogoke_r2_test_fixture_drivers";
    if !tx.query("SELECT name FROM temp.sqlite_schema WHERE (lower(name)=lower(?) AND type IN ('table','view')) OR (lower(tbl_name)=lower(?) AND type='trigger')", &[name, name], 1)?.is_empty() {
        return denied();
    }
    let rows = tx.query("SELECT type,sql FROM main.sqlite_schema WHERE name=?", &[name], 2)?;
    if rows.is_empty() {
        tx.write(SCHEMA, &[])?;
    } else if rows.len() != 1 || rows[0][0] != "table" || rows[0][1] != SCHEMA {
        return denied();
    }
    if !tx.query("SELECT name FROM main.sqlite_schema WHERE type='trigger' AND lower(tbl_name)=lower(?)", &[name], 1)?.is_empty() {
        return denied();
    }
    Ok(())
}

fn validated_row(tx: &mut Transaction<'_, '_>, runtime_instance_id: &str) -> Result<R2FixtureDriverBinding> {
    ensure_schema(tx)?;
    let rows = tx.query("SELECT driver_id,profile_id,adapter_version,runtime_instance_id,launch_digest_sha256,content_hash FROM main.gogoke_r2_test_fixture_drivers WHERE runtime_instance_id=?", &[runtime_instance_id], 6)?;
    if rows.len() != 1 {
        return denied();
    }
    let row = &rows[0];
    let expected = expected(&row[0])?;
    if row[1] != super::catalog::current_profile(tx)?.profile_id
        || row[2] != expected.adapter_version
        || row[3] != expected.runtime_instance_id
        || row[4] != expected.launch_digest_sha256
        || row[5] != expected.content_hash
    {
        return denied();
    }
    Ok(expected)
}

pub(crate) fn register(
    connection: &mut VerifiedDatabaseConnection<'_>,
    profile_id: &str,
    driver_id: &str,
) -> Result<R2FixtureDriverBinding> {
    let binding = expected(driver_id)?;
    transaction::run(connection, |tx| {
        ensure_schema(tx)?;
        if super::catalog::current_profile(tx)?.profile_id != profile_id {
            return denied();
        }
        tx.write("INSERT INTO main.gogoke_r2_test_fixture_drivers(driver_id,profile_id,adapter_version,runtime_instance_id,launch_digest_sha256,content_hash) VALUES(?,?,?,?,?,?) ON CONFLICT(driver_id) DO NOTHING", &[&binding.driver_id, profile_id, &binding.adapter_version, &binding.runtime_instance_id, &binding.launch_digest_sha256, &binding.content_hash])?;
        let registered = validated_row(tx, &binding.runtime_instance_id)?;
        if registered != binding {
            return denied();
        }
        Ok(registered)
    })
}

pub(super) fn resolve_in_transaction(
    tx: &mut Transaction<'_, '_>,
    runtime_instance_id: &str,
) -> Result<R2FixtureDriverBinding> {
    validated_row(tx, runtime_instance_id)
}

pub(crate) fn resolve(
    connection: &mut VerifiedDatabaseConnection<'_>,
    runtime_instance_id: &str,
) -> Result<R2FixtureDriverBinding> {
    transaction::run(connection, |tx| validated_row(tx, runtime_instance_id))
}

pub(crate) fn read_action_binding(
    connection: &mut VerifiedDatabaseConnection<'_>,
    action_completion_ref: &str,
) -> Result<R2FixtureDriverBinding> {
    if action_completion_ref.is_empty() || action_completion_ref.len() > 256
        || !action_completion_ref.bytes().all(|b| b.is_ascii_alphanumeric() || b"._:/-".contains(&b)) {
        return Err(OrchestrationError::AccessDenied);
    }
    transaction::run(connection, |tx| {
        let rows = tx.query("SELECT c.runtime_instance_id,c.operation_id,c.reservation_id,c.semantic_digest,c.trusted_receipt_ref FROM main.gogoke_action_completion_receipts c JOIN main.gogoke_action_reservations a ON a.operation_id=c.operation_id AND a.reservation_id=c.reservation_id JOIN main.gogoke_action_native_receipts n ON n.domain_id=c.domain_id AND n.operation_id=c.operation_id AND n.receipt_ref=c.trusted_receipt_ref WHERE c.domain_id=? AND c.receipt_id=? AND c.disposition='COMPLETED' AND a.state='completed' AND a.runtime_instance_id=c.runtime_instance_id AND a.semantic_digest=c.semantic_digest AND n.runtime_instance_id=c.runtime_instance_id AND n.semantic_digest=c.semantic_digest AND n.reservation_id=c.reservation_id", &[DOMAIN_ID, action_completion_ref], 5)?;
        if rows.len() != 1 {
            return denied();
        }
        let completion = super::action_authority::load_validated_completion(tx, DOMAIN_ID, &rows[0][1])?
            .ok_or(OrchestrationError::AccessDenied)?;
        let native = super::action_authority::load_validated_native_receipt(tx, DOMAIN_ID, &rows[0][1])?
            .ok_or(OrchestrationError::AccessDenied)?;
        if completion[0] != rows[0][2] || completion[1] != rows[0][3]
            || completion[7] != rows[0][0] || completion[10] != rows[0][4]
            || completion[12] != "COMPLETED" || completion[13] != action_completion_ref
            || native[0] != rows[0][4] || native[1] != rows[0][2]
            || native[2] != rows[0][3] || native[8] != rows[0][0]
            || native[12] != "COMPLETED" {
            return denied();
        }
        validated_row(tx, &rows[0][0])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_identity_is_bounded_and_content_derived() {
        let binding = expected("mock_novel_0123456789abcdef").unwrap();
        assert_eq!(binding.driver_id, "mock_novel_0123456789abcdef");
        assert_eq!(binding.adapter_version, "1.0.0");
        assert!(binding.runtime_instance_id.starts_with("runtime-r2-03-0123456789abcdef-"));
        assert_eq!(binding.content_hash.len(), 71);
        for bad in ["mock_novel_0123456789abcde", "mock_novel_0123456789abcdeg", "mock_novel_0123456789ABCDEF", "mock_novel_0123456789abcdef_extra"] {
            assert!(expected(bad).is_err());
        }
    }
}
