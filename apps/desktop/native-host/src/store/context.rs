use super::atomic::Statement;
use super::context_graph::{self, ContextNode};
use super::context_state;
use super::digest::content_hash;
use super::orchestration::OrchestrationError;
use super::same_open::VerifiedDatabaseConnection;

#[derive(Clone, Debug)]
pub struct ContextCommand {
    pub operation_id: String,
    pub context_id: String,
    pub version: String,
    pub scope: String,
    pub domain_id: String,
    pub kind: String,
    pub content_hash: String,
    pub source_ref: String,
    pub source_hash: String,
    pub source_authority_kind: String,
    pub source_authority_ref: String,
    pub derived_from: Vec<String>,
    pub supersedes: Vec<String>,
    pub access_policy_revision: String,
    pub visibility: String,
    pub read_grant_refs: Vec<String>,
    pub promotion: Option<PromotionEvidence>,
}

#[derive(Clone, Debug)]
pub struct PromotionEvidence {
    pub source_version_ref: String,
    pub source_grant_ref: String,
    pub target_grant_ref: String,
    pub provenance_refs: Vec<String>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct ContextReceipt {
    pub disposition: &'static str,
    pub operation_id: String,
    pub context_id: String,
    pub version: String,
    pub fingerprint: String,
    pub invalidated_version_refs: Vec<String>,
}

fn exec(
    connection: &mut VerifiedDatabaseConnection<'_>,
    sql: &str,
) -> Result<(), OrchestrationError> {
    connection
        .execute(sql)
        .map_err(|error| OrchestrationError::Atomic(error.into()))
}

pub fn apply_context_schema(
    connection: &mut VerifiedDatabaseConnection<'_>,
) -> Result<(), OrchestrationError> {
    for sql in [
        "CREATE TABLE IF NOT EXISTS gogoke_context_versions (domain_id TEXT NOT NULL,context_id TEXT NOT NULL,version TEXT NOT NULL,scope TEXT NOT NULL CHECK(scope IN ('GLOBAL','PROJECT','SESSION')),kind TEXT NOT NULL,content_hash TEXT NOT NULL CHECK(length(content_hash)=71),source_ref TEXT NOT NULL,source_hash TEXT NOT NULL CHECK(length(source_hash)=71),source_authority_kind TEXT NOT NULL,source_authority_ref TEXT NOT NULL,access_policy_revision TEXT NOT NULL,PRIMARY KEY(domain_id,context_id,version)) STRICT",
        "CREATE TABLE IF NOT EXISTS gogoke_context_states (domain_id TEXT NOT NULL,version_ref TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('ACTIVE','SUPERSEDED','CONFLICTED','STALE','REVOKED','ARCHIVED')),PRIMARY KEY(domain_id,version_ref)) STRICT",
        "CREATE TABLE IF NOT EXISTS gogoke_context_edges (domain_id TEXT NOT NULL,from_ref TEXT NOT NULL,to_ref TEXT NOT NULL,edge_kind TEXT NOT NULL CHECK(edge_kind IN ('DERIVED','SUPERSEDES')),PRIMARY KEY(domain_id,from_ref,to_ref,edge_kind)) STRICT",
        "CREATE TABLE IF NOT EXISTS gogoke_context_access (domain_id TEXT NOT NULL,version_ref TEXT NOT NULL,visibility TEXT NOT NULL CHECK(visibility IN ('OWNER_PRIVATE','DOMAIN_GRANTED')),read_grant_refs TEXT NOT NULL,PRIMARY KEY(domain_id,version_ref)) STRICT",
        "CREATE TABLE IF NOT EXISTS gogoke_context_promotions (domain_id TEXT NOT NULL,version_ref TEXT NOT NULL,source_version_ref TEXT NOT NULL,source_grant_ref TEXT NOT NULL,target_grant_ref TEXT NOT NULL,provenance_refs TEXT NOT NULL,PRIMARY KEY(domain_id,version_ref)) STRICT",
        "CREATE TABLE IF NOT EXISTS gogoke_context_operations (domain_id TEXT NOT NULL,operation_id TEXT NOT NULL,fingerprint TEXT NOT NULL CHECK(length(fingerprint)=71),context_id TEXT NOT NULL,version TEXT NOT NULL,invalidated_refs TEXT NOT NULL,PRIMARY KEY(domain_id,operation_id)) STRICT",
    ] {
        exec(connection, sql)?;
    }
    Ok(())
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-@".contains(&byte))
}

fn valid_u64(value: &str) -> bool {
    value == "0"
        || (!value.starts_with('0')
            && value.bytes().all(|byte| byte.is_ascii_digit())
            && value.parse::<u64>().is_ok())
}

fn valid_version_ref(value: &str) -> bool {
    value
        .rsplit_once('@')
        .is_some_and(|(context_id, version)| valid_id(context_id) && valid_u64(version))
}

fn valid_hash(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn list(value: &[String]) -> String {
    value.join(",")
}

fn version_ref(command: &ContextCommand) -> String {
    format!("{}@{}", command.context_id, command.version)
}

fn validate(command: &ContextCommand) -> Result<(), OrchestrationError> {
    validate_for_source_domain(command, &command.domain_id)
}

fn validate_for_source_domain(
    command: &ContextCommand, source_domain: &str,
) -> Result<(), OrchestrationError> {
    for value in [
        &command.operation_id,
        &command.context_id,
        &command.domain_id,
        &command.kind,
        &command.source_ref,
        &command.source_authority_kind,
        &command.source_authority_ref,
    ] {
        if !valid_id(value) {
            return Err(OrchestrationError::Invalid("context id"));
        }
    }
    if !valid_u64(&command.version) || !valid_u64(&command.access_policy_revision) {
        return Err(OrchestrationError::Invalid("context revision"));
    }
    if !valid_hash(&command.content_hash) || !valid_hash(&command.source_hash) {
        return Err(OrchestrationError::Invalid("context hash"));
    }
    if !matches!(command.scope.as_str(), "GLOBAL" | "PROJECT" | "SESSION")
        || !matches!(
            command.visibility.as_str(),
            "OWNER_PRIVATE" | "DOMAIN_GRANTED"
        )
    {
        return Err(OrchestrationError::Invalid("context enum"));
    }
    let current = version_ref(command);
    for reference in &command.derived_from {
        if !valid_version_ref(reference)
            || (source_domain == command.domain_id && reference == &current) {
            return Err(OrchestrationError::Invalid("context reference"));
        }
    }
    for reference in &command.supersedes {
        if !valid_version_ref(reference) || reference == &current {
            return Err(OrchestrationError::Invalid("context reference"));
        }
    }
    for reference in command.read_grant_refs.iter().chain(
        command
            .promotion
            .iter()
            .flat_map(|value| &value.provenance_refs),
    ) {
        if !valid_id(reference) {
            return Err(OrchestrationError::Invalid("context evidence reference"));
        }
    }
    if command.scope == "GLOBAL" {
        let promotion = command
            .promotion
            .as_ref()
            .ok_or(OrchestrationError::Invalid("promotion"))?;
        if !command.derived_from.contains(&promotion.source_version_ref)
            || promotion.provenance_refs.is_empty()
            || !valid_id(&promotion.source_grant_ref)
            || !valid_id(&promotion.target_grant_ref)
        {
            return Err(OrchestrationError::Invalid("promotion"));
        }
    } else if command.promotion.is_some() {
        return Err(OrchestrationError::Invalid("promotion"));
    }
    Ok(())
}

fn fingerprint(command: &ContextCommand) -> String {
    let promotion = command
        .promotion
        .as_ref()
        .map(|value| {
            format!(
                "{}\0{}\0{}\0{}",
                value.source_version_ref,
                value.source_grant_ref,
                value.target_grant_ref,
                list(&value.provenance_refs)
            )
        })
        .unwrap_or_default();
    content_hash(
        format!(
            "{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
            command.operation_id,
            command.context_id,
            command.version,
            command.scope,
            command.domain_id,
            command.kind,
            command.content_hash,
            command.source_ref,
            command.source_hash,
            command.source_authority_kind,
            command.source_authority_ref,
            list(&command.derived_from),
            list(&command.supersedes),
            command.access_policy_revision,
            command.visibility,
            list(&command.read_grant_refs),
            promotion
        )
        .as_bytes(),
    )
}

fn query_operation(
    connection: &mut VerifiedDatabaseConnection<'_>,
    domain: &str,
    operation: &str,
) -> Result<Option<(String, Vec<String>)>, OrchestrationError> {
    let statement = Statement::prepare(
        connection.as_ptr(),
        "SELECT fingerprint,invalidated_refs FROM gogoke_context_operations WHERE domain_id=? AND operation_id=?",
    )?;
    statement.bind_text(1, domain)?;
    statement.bind_text(2, operation)?;
    if statement.step_row()? {
        let invalidated = statement.column_text(1)?;
        Ok(Some((
            statement.column_text(0)?,
            if invalidated.is_empty() {
                vec![]
            } else {
                invalidated.split(',').map(str::to_owned).collect()
            },
        )))
    } else {
        Ok(None)
    }
}

fn require_version(
    connection: &mut VerifiedDatabaseConnection<'_>,
    domain: &str,
    reference: &str,
) -> Result<(), OrchestrationError> {
    let Some((context_id, version)) = reference.rsplit_once('@') else {
        return Err(OrchestrationError::Invalid("context reference"));
    };
    let statement = Statement::prepare(
        connection.as_ptr(),
        "SELECT 1 FROM gogoke_context_versions WHERE domain_id=? AND context_id=? AND version=?",
    )?;
    statement.bind_text(1, domain)?;
    statement.bind_text(2, context_id)?;
    statement.bind_text(3, version)?;
    if statement.step_row()? {
        Ok(())
    } else {
        Err(OrchestrationError::Invalid("context source missing"))
    }
}

fn require_active_version(
    connection: &mut VerifiedDatabaseConnection<'_>,
    domain_id: &str,
    reference: &str,
) -> Result<(), OrchestrationError> {
    require_version(connection, domain_id, reference)?;
    let snapshot = context_state::current(connection, domain_id, reference)?;
    if snapshot.state == "ACTIVE" {
        Ok(())
    } else {
        Err(OrchestrationError::Invalid("context source is not active"))
    }
}

fn insert_edge(
    connection: &mut VerifiedDatabaseConnection<'_>,
    source_domain: &str,
    target_domain: &str,
    from: &str,
    to: &str,
    kind: &str,
) -> Result<(), OrchestrationError> {
    context_graph::insert_edge(
        connection,
        &ContextNode { domain_id: source_domain.into(), version_ref: from.into() },
        &ContextNode { domain_id: target_domain.into(), version_ref: to.into() },
        kind,
    )
}

/// Legacy storage primitive; caller ingress must perform Product Authority admission.
/// GLOBAL IPC remains closed. This function alone is not authorization evidence.
pub fn commit_context_version(
    connection: &mut VerifiedDatabaseConnection<'_>,
    command: ContextCommand,
) -> Result<ContextReceipt, OrchestrationError> {
    validate(&command)?;
    exec(connection, "BEGIN IMMEDIATE")?;
    let result = apply_context_version_in_transaction(connection, command);
    match result {
        Ok(receipt) => {
            exec(connection, "COMMIT")?;
            Ok(receipt)
        }
        Err(error) => {
            let _ = exec(connection, "ROLLBACK");
            Err(error)
        }
    }
}

/// One existing storage write group, reused by the native authority transaction.
/// No grant decision, nested BEGIN, COMMIT, second graph, or second store here.
pub(super) fn apply_context_version_in_transaction(
    connection: &mut VerifiedDatabaseConnection<'_>,
    command: ContextCommand,
) -> Result<ContextReceipt, OrchestrationError> {
    // Preserve the exact historical meaning and fingerprint for legacy callers:
    // every unqualified reference belongs to the command's own domain.
    let source_domain = command.domain_id.clone();
    apply_context_version_from_source_domain(connection, command, &source_domain)
}

/// Internal storage composition only, not an authorization decision. The native
/// promotion authority supplies the explicit source after revalidating its grant.
/// One shared write group owns legacy and cross-domain storage; no alternate graph.
pub(super) fn apply_context_version_from_source_domain(
    connection: &mut VerifiedDatabaseConnection<'_>,
    command: ContextCommand,
    source_domain: &str,
) -> Result<ContextReceipt, OrchestrationError> {
    unsafe extern "C" {
        fn sqlite3_get_autocommit(database: *mut std::ffi::c_void) -> std::ffi::c_int;
    }
    // SAFETY: the verified connection exclusively owns this live SQLite handle.
    if unsafe { sqlite3_get_autocommit(connection.as_ptr()) } != 0 {
        return Err(OrchestrationError::Invalid("Context write group requires a transaction"));
    }
    validate_for_source_domain(&command, source_domain)?;
    if !valid_id(source_domain) {
        return Err(OrchestrationError::Invalid("context source domain"));
    }
    if source_domain != command.domain_id &&
        (command.scope != "GLOBAL" || command.derived_from.len() != 1
            || !command.supersedes.is_empty() || command.visibility != "OWNER_PRIVATE"
            || !command.read_grant_refs.is_empty()) {
        return Err(OrchestrationError::Invalid("qualified promotion storage shape"));
    }
    let legacy_fingerprint = fingerprint(&command);
    let fingerprint = if source_domain == command.domain_id {
        legacy_fingerprint
    } else {
        content_hash(format!("gogoke.context.source-domain.v1\0{source_domain}\0{legacy_fingerprint}").as_bytes())
    };
    // Schema conversion, edges, source state and receipts share this transaction.
    context_graph::ensure_schema(connection)?;
    (|| {
        if let Some((existing, invalidated_version_refs)) =
            query_operation(connection, &command.domain_id, &command.operation_id)?
        {
            if existing != fingerprint {
                return Err(OrchestrationError::OperationConflict);
            }
            return Ok(ContextReceipt {
                disposition: "RECONCILED",
                operation_id: command.operation_id.clone(),
                context_id: command.context_id.clone(),
                version: command.version.clone(),
                fingerprint,
                invalidated_version_refs,
            });
        }
        for reference in &command.derived_from {
            require_active_version(connection, source_domain, reference)?;
        }
        for reference in &command.supersedes {
            require_active_version(connection, &command.domain_id, reference)?;
        }
        let reference = version_ref(&command);
        let statement = Statement::prepare(connection.as_ptr(), "INSERT INTO gogoke_context_versions(domain_id,context_id,version,scope,kind,content_hash,source_ref,source_hash,source_authority_kind,source_authority_ref,access_policy_revision) VALUES (?,?,?,?,?,?,?,?,?,?,?)")?;
        for (index, value) in [
            command.domain_id.as_str(),
            command.context_id.as_str(),
            command.version.as_str(),
            command.scope.as_str(),
            command.kind.as_str(),
            command.content_hash.as_str(),
            command.source_ref.as_str(),
            command.source_hash.as_str(),
            command.source_authority_kind.as_str(),
            command.source_authority_ref.as_str(),
            command.access_policy_revision.as_str(),
        ]
        .iter()
        .enumerate()
        {
            statement.bind_text((index + 1) as i32, value)?;
        }
        statement.step_done()?;
        context_state::insert_initial(connection, &command.domain_id, &reference)?;
        let statement = Statement::prepare(connection.as_ptr(), "INSERT INTO gogoke_context_access(domain_id,version_ref,visibility,read_grant_refs) VALUES (?,?,?,?)")?;
        statement.bind_text(1, &command.domain_id)?;
        statement.bind_text(2, &reference)?;
        statement.bind_text(3, &command.visibility)?;
        statement.bind_text(4, &list(&command.read_grant_refs))?;
        statement.step_done()?;
        for source in &command.derived_from {
            insert_edge(
                connection,
                source_domain,
                &command.domain_id,
                source,
                &reference,
                "DERIVED",
            )?;
        }
        let mut invalidated_version_refs = Vec::new();
        for source in &command.supersedes {
            invalidated_version_refs.push(source.clone());
            insert_edge(
                connection,
                &command.domain_id,
                &command.domain_id,
                source,
                &reference,
                "SUPERSEDES",
            )?;
            context_state::transition(
                connection, &command.domain_id, source, "ACTIVE", "SUPERSEDED",
            )?;
            let descendants = context_graph::invalidate_descendants(
                connection,
                &ContextNode { domain_id: command.domain_id.clone(), version_ref: source.clone() },
            )?;
            // This public legacy receipt has no domain-qualified payload. Do not
            // leak foreign descendant identities, counts or grant information.
            invalidated_version_refs.extend(descendants.into_iter()
                .filter(|node| node.domain_id == command.domain_id)
                .map(|node| node.version_ref));
        }
        if let Some(promotion) = &command.promotion {
            let provenance = list(&promotion.provenance_refs);
            let statement = Statement::prepare(connection.as_ptr(), "INSERT INTO gogoke_context_promotions(domain_id,version_ref,source_version_ref,source_grant_ref,target_grant_ref,provenance_refs) VALUES (?,?,?,?,?,?)")?;
            for (index, value) in [
                command.domain_id.as_str(),
                reference.as_str(),
                promotion.source_version_ref.as_str(),
                promotion.source_grant_ref.as_str(),
                promotion.target_grant_ref.as_str(),
                provenance.as_str(),
            ]
            .iter()
            .enumerate()
            {
                statement.bind_text((index + 1) as i32, value)?;
            }
            statement.step_done()?;
        }
        invalidated_version_refs.sort();
        invalidated_version_refs.dedup();
        let invalidated = list(&invalidated_version_refs);
        let statement = Statement::prepare(connection.as_ptr(), "INSERT INTO gogoke_context_operations(domain_id,operation_id,fingerprint,context_id,version,invalidated_refs) VALUES (?,?,?,?,?,?)")?;
        for (index, value) in [
            command.domain_id.as_str(),
            command.operation_id.as_str(),
            fingerprint.as_str(),
            command.context_id.as_str(),
            command.version.as_str(),
            invalidated.as_str(),
        ]
        .iter()
        .enumerate()
        {
            statement.bind_text((index + 1) as i32, value)?;
        }
        statement.step_done()?;
        Ok(ContextReceipt {
            disposition: "COMMITTED",
            operation_id: command.operation_id.clone(),
            context_id: command.context_id.clone(),
            version: command.version.clone(),
            fingerprint,
            invalidated_version_refs,
        })
    })()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::same_open::{create_new, route_b_test_guard};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn command(id: &str, version: &str) -> ContextCommand {
        ContextCommand {
            operation_id: format!("operation-{id}-{version}"),
            context_id: id.into(),
            version: version.into(),
            scope: "PROJECT".into(),
            domain_id: "domain-one".into(),
            kind: "fact".into(),
            content_hash: format!("sha256:{}", "a".repeat(64)),
            source_ref: "source://one".into(),
            source_hash: format!("sha256:{}", "b".repeat(64)),
            source_authority_kind: "repository".into(),
            source_authority_ref: "authority://one".into(),
            derived_from: vec![],
            supersedes: vec![],
            access_policy_revision: "1".into(),
            visibility: "OWNER_PRIVATE".into(),
            read_grant_refs: vec![],
            promotion: None,
        }
    }
    fn state(connection: &mut VerifiedDatabaseConnection<'_>, reference: &str) -> String {
        let statement = Statement::prepare(connection.as_ptr(), "SELECT state FROM gogoke_context_states WHERE domain_id='domain-one' AND version_ref=?").unwrap();
        statement.bind_text(1, reference).unwrap();
        assert!(statement.step_row().unwrap());
        statement.column_text(0).unwrap()
    }
    #[test]
    fn immutable_versions_propagate_supersede_without_rewriting_originals() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root_path = std::env::temp_dir().join(format!("gogoke-context-{nonce}"));
        std::fs::create_dir(&root_path).unwrap();
        let root = RootLock::acquire(&root_path).unwrap();
        let database = root_path.join("state.sqlite");
        let mut connection = create_new(&root, &database).unwrap();
        apply_context_schema(&mut connection).unwrap();
        commit_context_version(&mut connection, command("source", "1")).unwrap();
        let mut derived = command("derived", "1");
        derived.derived_from = vec!["source@1".into()];
        commit_context_version(&mut connection, derived).unwrap();
        let mut replacement = command("source", "2");
        replacement.supersedes = vec!["source@1".into()];
        let receipt = commit_context_version(&mut connection, replacement).unwrap();
        assert_eq!(
            receipt.invalidated_version_refs,
            vec!["derived@1", "source@1"]
        );
        assert_eq!(state(&mut connection, "source@1"), "SUPERSEDED");
        assert_eq!(state(&mut connection, "derived@1"), "STALE");
        assert_eq!(state(&mut connection, "source@2"), "ACTIVE");
        let mut late = command("late-derived", "1");
        late.derived_from = vec!["source@1".into()];
        assert!(matches!(
            commit_context_version(&mut connection, late),
            Err(OrchestrationError::Invalid("context source is not active"))
        ));
        connection.close_checked().unwrap();
        drop(root);
        std::fs::remove_file(database).ok();
        std::fs::remove_dir(root_path).ok();
    }
    #[test]
    fn global_requires_promotion_and_remains_owner_private() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root_path = std::env::temp_dir().join(format!("gogoke-promotion-{nonce}"));
        std::fs::create_dir(&root_path).unwrap();
        let root = RootLock::acquire(&root_path).unwrap();
        let database = root_path.join("state.sqlite");
        let mut connection = create_new(&root, &database).unwrap();
        apply_context_schema(&mut connection).unwrap();
        commit_context_version(&mut connection, command("project", "1")).unwrap();
        let mut global = command("global", "1");
        global.scope = "GLOBAL".into();
        global.derived_from = vec!["project@1".into()];
        assert!(commit_context_version(&mut connection, global.clone()).is_err());
        global.promotion = Some(PromotionEvidence {
            source_version_ref: "project@1".into(),
            source_grant_ref: "grant://source".into(),
            target_grant_ref: "grant://target".into(),
            provenance_refs: vec!["evidence://review".into()],
        });
        assert_eq!(
            commit_context_version(&mut connection, global.clone())
                .unwrap()
                .disposition,
            "COMMITTED"
        );
        assert_eq!(
            commit_context_version(&mut connection, global)
                .unwrap()
                .disposition,
            "RECONCILED"
        );
        let statement = Statement::prepare(connection.as_ptr(), "SELECT visibility FROM gogoke_context_access WHERE domain_id='domain-one' AND version_ref='global@1'").unwrap();
        assert!(statement.step_row().unwrap());
        assert_eq!(statement.column_text(0).unwrap(), "OWNER_PRIVATE");
        drop(statement);
        connection.close_checked().unwrap();
        drop(root);
        std::fs::remove_file(database).ok();
        std::fs::remove_dir(root_path).ok();
    }
}
