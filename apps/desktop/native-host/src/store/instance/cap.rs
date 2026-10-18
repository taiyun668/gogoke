//! F.1 Owner-set instance capacity. H reads the value on its verified product
//! connection inside the admission transaction; no capacity is inferred here.

use super::registry::valid_id;
use crate::store::atomic::Statement;
use crate::store::authority::{check_owner_in_current_transaction, OwnerIssuer};
use crate::store::orchestration::OrchestrationError;
use crate::store::same_open::VerifiedDatabaseConnection;

/// The native Owner issuer is checked in the same write transaction as the
/// update. The field has no schema or runtime default.
pub(crate) fn set_instance_concurrency_cap(
    connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    instance_id: &str,
    cap: i64,
) -> Result<(), OrchestrationError> {
    if !valid_id(instance_id) {
        return Err(OrchestrationError::Invalid("instance_id"));
    }
    if cap <= 0 {
        return Err(OrchestrationError::Invalid("instance_concurrency_cap"));
    }
    connection.execute("BEGIN IMMEDIATE")
        .map_err(|error| OrchestrationError::Atomic(error.into()))?;
    let result = (|| {
        check_owner_in_current_transaction(connection, owner)?;
        let exists = Statement::prepare(connection.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_instances WHERE instance_id=?1")?;
        exists.bind_text(1, instance_id)?;
        if !exists.step_row()? {
            return Err(OrchestrationError::AccessDenied);
        }
        let write = Statement::prepare(connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_caps(instance_id,concurrency_cap) VALUES(?1,?2) ON CONFLICT(instance_id) DO UPDATE SET concurrency_cap=excluded.concurrency_cap")?;
        write.bind_text(1, instance_id)?;
        write.bind_i64(2, cap)?;
        write.step_done()?;
        Ok(())
    })();
    match result {
        Ok(()) => connection.execute("COMMIT")
            .map_err(OrchestrationError::CommitUnknownWithCause),
        Err(error) => {
            connection.execute("ROLLBACK")
                .map_err(OrchestrationError::CommitUnknownWithCause)?;
            Err(error)
        }
    }
}

/// A missing cap denies admission. H calls this on the same
/// VerifiedDatabaseConnection and inside its admission transaction.
pub(crate) fn read_instance_concurrency_cap(
    connection: &VerifiedDatabaseConnection<'_>,
    instance_id: &str,
) -> Result<i64, OrchestrationError> {
    if !valid_id(instance_id) {
        return Err(OrchestrationError::Invalid("instance_id"));
    }
    let query = Statement::prepare(connection.as_ptr(),
        "SELECT c.concurrency_cap FROM main.gogoke_v37_instance_caps AS c INNER JOIN main.gogoke_v37_instances AS i ON i.instance_id=c.instance_id WHERE c.instance_id=?1")?;
    query.bind_text(1, instance_id)?;
    if !query.step_row()? {
        return Err(OrchestrationError::AccessDenied);
    }
    let cap = query.column_text(0)?.parse::<i64>()
        .map_err(|_| OrchestrationError::AccessDenied)?;
    if cap <= 0 || query.step_row()? {
        return Err(OrchestrationError::AccessDenied);
    }
    Ok(cap)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::instance::initialize_schema;
    use crate::store::same_open::{create_new, open_existing, route_b_test_guard};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn owner_cap_is_required_validated_and_survives_verified_reopen() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gogoke-v37-instance-cap-{}-{nonce}", std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let database = path.join("state.sqlite");
        let mut connection = create_new(&root, &database).unwrap();
        connection.execute("PRAGMA foreign_keys=ON").unwrap();
        let owner = crate::store::authority::initialize_profile(&mut connection, &root).unwrap();
        initialize_schema(&mut connection).unwrap();
        connection.execute("INSERT INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceA','codex','homeA','identityA','sha256:test','1','INSTALLED','LOGGED_IN',1)").unwrap();
        connection.execute("INSERT INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceB','codex','homeB','identityB','sha256:test','1','INSTALLED','LOGGED_IN',1)").unwrap();

        assert!(matches!(read_instance_concurrency_cap(&connection, "instanceA"),
            Err(OrchestrationError::AccessDenied)));
        assert!(matches!(set_instance_concurrency_cap(&mut connection, &owner, "instanceA", 0),
            Err(OrchestrationError::Invalid("instance_concurrency_cap"))));
        assert!(matches!(set_instance_concurrency_cap(&mut connection, &owner, "instanceA", -1),
            Err(OrchestrationError::Invalid("instance_concurrency_cap"))));
        assert!(matches!(set_instance_concurrency_cap(&mut connection, &owner, "missing", 4),
            Err(OrchestrationError::AccessDenied)));
        assert!(matches!(read_instance_concurrency_cap(&connection, "instanceA"),
            Err(OrchestrationError::AccessDenied)));

        // Four is explicit Owner test input, not a production default.
        set_instance_concurrency_cap(&mut connection, &owner, "instanceA", 4).unwrap();
        assert_eq!(read_instance_concurrency_cap(&connection, "instanceA").unwrap(), 4);
        assert!(matches!(set_instance_concurrency_cap(&mut connection, &owner, "instanceA", 0),
            Err(OrchestrationError::Invalid("instance_concurrency_cap"))));
        assert_eq!(read_instance_concurrency_cap(&connection, "instanceA").unwrap(), 4);
        assert!(matches!(read_instance_concurrency_cap(&connection, "instanceB"),
            Err(OrchestrationError::AccessDenied)));
        connection.close_checked().unwrap();

        let mut reopened = open_existing(&root, &database).unwrap();
        reopened.execute("PRAGMA foreign_keys=ON").unwrap();
        initialize_schema(&mut reopened).unwrap();
        assert_eq!(read_instance_concurrency_cap(&reopened, "instanceA").unwrap(), 4);
        assert!(matches!(read_instance_concurrency_cap(&reopened, "instanceB"),
            Err(OrchestrationError::AccessDenied)));
        set_instance_concurrency_cap(&mut reopened, &owner, "instanceA", 2).unwrap();
        assert_eq!(read_instance_concurrency_cap(&reopened, "instanceA").unwrap(), 2);
        reopened.close_checked().unwrap();
        drop(root);
        std::fs::remove_file(database).unwrap();
        if let Err(error) = std::fs::remove_dir(&path) {
            eprintln!("owned fixture retained: {} ({error})", path.display());
        }
    }
}
