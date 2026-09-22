//! Domain-qualified edges in the EXISTING Context graph, not another graph/store.
//! Call only inside the owning Context/Product Authority transaction. No grants
//! are issued here, and graph reachability never grants a reader access.
use super::atomic::Statement;
use super::context_state;
use super::orchestration::OrchestrationError;
use super::same_open::VerifiedDatabaseConnection;

type Result<T> = std::result::Result<T, OrchestrationError>;
const MAX_DESCENDANTS: usize = 16_384;

const LEGACY_SCHEMA: &str = "CREATE TABLE gogoke_context_edges (domain_id TEXT NOT NULL,from_ref TEXT NOT NULL,to_ref TEXT NOT NULL,edge_kind TEXT NOT NULL CHECK(edge_kind IN ('DERIVED','SUPERSEDES')),PRIMARY KEY(domain_id,from_ref,to_ref,edge_kind)) STRICT";
const GRAPH_SCHEMA: &str = "CREATE TABLE \"gogoke_context_edges\" (domain_id TEXT NOT NULL,source_domain_id TEXT NOT NULL,from_ref TEXT NOT NULL,to_ref TEXT NOT NULL,edge_kind TEXT NOT NULL CHECK(edge_kind IN ('DERIVED','SUPERSEDES')),PRIMARY KEY(source_domain_id,from_ref,domain_id,to_ref,edge_kind)) STRICT";
const CREATE_STAGING: &str = "CREATE TABLE gogoke_context_edges_r4_staging (domain_id TEXT NOT NULL,source_domain_id TEXT NOT NULL,from_ref TEXT NOT NULL,to_ref TEXT NOT NULL,edge_kind TEXT NOT NULL CHECK(edge_kind IN ('DERIVED','SUPERSEDES')),PRIMARY KEY(source_domain_id,from_ref,domain_id,to_ref,edge_kind)) STRICT";
const COPY_LEGACY: &str = "INSERT INTO gogoke_context_edges_r4_staging(domain_id,source_domain_id,from_ref,to_ref,edge_kind) SELECT domain_id,domain_id,from_ref,to_ref,edge_kind FROM gogoke_context_edges";
const DROP_LEGACY: &str = "DROP TABLE gogoke_context_edges";
const RENAME_STAGING: &str = "ALTER TABLE gogoke_context_edges_r4_staging RENAME TO gogoke_context_edges";
const READ_SCHEMA: &str = "SELECT type,sql FROM sqlite_schema WHERE name='gogoke_context_edges'";
const UNEXPECTED_DEPENDENCIES: &str = "SELECT name FROM sqlite_schema WHERE lower(name)='gogoke_context_edges_r4_staging' OR (type IN ('view','trigger') AND instr(lower(sql),'gogoke_context_edges')>0) OR (type='index' AND tbl_name='gogoke_context_edges' AND sql IS NOT NULL) LIMIT 1";
const INBOUND_FOREIGN_KEYS: &str = "SELECT s.name FROM sqlite_schema s JOIN pragma_foreign_key_list(s.name) f WHERE s.type='table' AND lower(f.\"table\")='gogoke_context_edges' LIMIT 1";
const INSERT_EDGE: &str = "INSERT INTO gogoke_context_edges(domain_id,source_domain_id,from_ref,to_ref,edge_kind) VALUES(?,?,?,?,?)";
const READ_DESCENDANTS: &str = "WITH RECURSIVE descendants(domain_id,ref) AS (SELECT domain_id,to_ref FROM gogoke_context_edges WHERE source_domain_id=? AND from_ref=? AND edge_kind='DERIVED' UNION SELECT edge.domain_id,edge.to_ref FROM gogoke_context_edges edge JOIN descendants parent ON edge.source_domain_id=parent.domain_id AND edge.from_ref=parent.ref WHERE edge.edge_kind='DERIVED' LIMIT 16385) SELECT domain_id,ref FROM descendants ORDER BY domain_id,ref";

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct ContextNode {
    pub domain_id: String,
    pub version_ref: String,
}

fn invalid<T>(detail: &'static str) -> Result<T> {
    Err(OrchestrationError::Invalid(detail))
}

// Preserve the existing native Context identifier language; authority admission
// applies its additional principal/domain constraints before invoking storage.
fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || b"._:/-@".contains(&b))
}

impl ContextNode {
    fn validate(&self) -> Result<()> {
        let Some((id, version)) = self.version_ref.rsplit_once('@') else {
            return invalid("qualified Context version reference");
        };
        if !valid_id(&self.domain_id) || !valid_id(id) || version.is_empty()
            || version.len() > 20 || (version.len() > 1 && version.starts_with('0'))
            || !version.bytes().all(|b| b.is_ascii_digit()) || version.parse::<u64>().is_err() {
            return invalid("qualified Context identity");
        }
        Ok(())
    }
}

fn require_transaction(connection: &VerifiedDatabaseConnection<'_>) -> Result<()> {
    unsafe extern "C" {
        fn sqlite3_get_autocommit(database: *mut std::ffi::c_void) -> std::ffi::c_int;
    }
    // SAFETY: the caller exclusively owns this live verified SQLite connection.
    if unsafe { sqlite3_get_autocommit(connection.as_ptr()) } != 0 {
        return invalid("Context graph requires owning transaction");
    }
    Ok(())
}

fn execute(connection: &VerifiedDatabaseConnection<'_>, sql: &str) -> Result<()> {
    Statement::prepare(connection.as_ptr(), sql)?.step_done()?;
    Ok(())
}

fn schema(connection: &VerifiedDatabaseConnection<'_>) -> Result<String> {
    let statement = Statement::prepare(connection.as_ptr(), READ_SCHEMA)?;
    if !statement.step_row()? || statement.column_text(0)? != "table" {
        return invalid("Context graph schema missing or not a table");
    }
    let sql = statement.column_text(1)?;
    if statement.step_row()? { return invalid("ambiguous Context graph schema"); }
    Ok(sql)
}

/// Extend one internal table without reinterpreting legacy facts. The old graph
/// ALWAYS meant both ends were in domain_id, so those rows are copied with that
/// exact source domain. Only exact known DDL is admitted, never guessed/repaired.
/// No PRAGMA bypass, foreign-key disabling, second connection or external I/O.
/// Caller must rollback on any error; DDL and data remain one atomic write group.
pub(super) fn ensure_schema(connection: &mut VerifiedDatabaseConnection<'_>) -> Result<()> {
    require_transaction(connection)?;
    for query in [UNEXPECTED_DEPENDENCIES, INBOUND_FOREIGN_KEYS] {
        if Statement::prepare(connection.as_ptr(), query)?.step_row()? {
            return invalid("unqualified Context graph schema dependency");
        }
    }
    let before = schema(connection)?;
    if before == GRAPH_SCHEMA { return Ok(()); }
    if before != LEGACY_SCHEMA { return invalid("unrecognized Context graph schema"); }
    for sql in [CREATE_STAGING, COPY_LEGACY, DROP_LEGACY, RENAME_STAGING] {
        execute(connection, sql)?;
    }
    if schema(connection)? != GRAPH_SCHEMA {
        return invalid("Context graph schema verification failed");
    }
    Ok(())
}

pub(super) fn insert_edge(
    connection: &mut VerifiedDatabaseConnection<'_>,
    source: &ContextNode, target: &ContextNode, kind: &str,
) -> Result<()> {
    require_transaction(connection)?;
    source.validate()?;
    target.validate()?;
    if source == target || !matches!(kind, "DERIVED" | "SUPERSEDES")
        || (kind == "SUPERSEDES" && source.domain_id != target.domain_id) {
        return invalid("Context graph edge kind or identity");
    }
    let statement = Statement::prepare(connection.as_ptr(), INSERT_EDGE)?;
    for (index, value) in [target.domain_id.as_str(), source.domain_id.as_str(), source.version_ref.as_str(),
        target.version_ref.as_str(), kind].iter().enumerate() {
        statement.bind_text((index + 1) as i32, value)?;
    }
    statement.step_done()?;
    Ok(())
}

/// Return complete compound identities or an error, never a silently truncated
/// invalidation set. UNION de-duplicates compound nodes even in a corrupt cycle.
pub(super) fn descendants(
    connection: &mut VerifiedDatabaseConnection<'_>, source: &ContextNode,
) -> Result<Vec<ContextNode>> {
    require_transaction(connection)?;
    source.validate()?;
    let statement = Statement::prepare(connection.as_ptr(), READ_DESCENDANTS)?;
    statement.bind_text(1, &source.domain_id)?;
    statement.bind_text(2, &source.version_ref)?;
    let mut nodes = Vec::new();
    while statement.step_row()? {
        if nodes.len() >= MAX_DESCENDANTS { return invalid("Context graph traversal bound"); }
        let node = ContextNode { domain_id: statement.column_text(0)?, version_ref: statement.column_text(1)? };
        node.validate()?;
        if &node == source { return invalid("Context graph cycle reaches source"); }
        nodes.push(node);
    }
    Ok(nodes)
}

/// Updates the SAME lifecycle index, preserving immutable Context versions and
/// non-ACTIVE states. Reachability is not read permission; caller-visible receipts
/// must filter these internal identities to their already-authorized domain.
pub(super) fn invalidate_descendants(
    connection: &mut VerifiedDatabaseConnection<'_>, source: &ContextNode,
) -> Result<Vec<ContextNode>> {
    let nodes = descendants(connection, source)?;
    // Obtain/validate the complete set before the first mutation.
    for node in &nodes {
        context_state::transition_active_to_stale(
            connection, &node.domain_id, &node.version_ref,
        )?;
    }
    Ok(nodes)
}
