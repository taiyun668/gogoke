//! Native F source and linked-worktree authority. No caller supplied cwd or
//! Git result is accepted as a binding. The crate-private native child route
//! rechecks E dispatch authority inside each F write transaction.

use crate::process::{
    DurableStopConfirmation, NativeBinding, PrepareRequest, ProcessCustodian, ProcessLaunch,
    StopBudgets,
};
use crate::root::{inspect_root, RootIdentity, RootLock};
use crate::store::atomic::{AtomicError, Json, JsonString, Statement};
use crate::store::authority::{
    check_owner_in_current_transaction, mark_process_active, mark_process_stopped,
    mark_process_unknown, read_product_identity, record_prepared_process, OwnerIssuer,
};
use crate::store::digest::{content_hash, sha256_hex};
use crate::store::same_open::{SameOpenError, VerifiedDatabaseConnection};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SOURCE_REMOTE: &str = "https://github.com/taiyun668/gogoke-seat-testbed.git";
const SOURCE_REMOTE_SSH: &str = "git@github.com:taiyun668/gogoke-seat-testbed.git";

mod f2;
pub(crate) use f2::{cleanup_stop_gate, cleanup_worktree, graph_query, merge_worktree,
    merge_worktree_request, readback_create_receipt, register_created_worktree,
    register_created_worktree_request, repository_for_worktree,
    resolve_group_for_launch, CleanupReceipt, CreateHistory, ExactStopFact, GraphMember,
    MergeReceipt, RegisterReceipt, WorktreeGraph};

fn remote_kind(remote: &str) -> Option<&'static str> {
    // Classify only. Never store the remote text, contact it, invoke a
    // credential helper, or allow inline URL credentials into native logs.
    if remote.is_empty() || remote.len() > 2048 || remote.chars().any(char::is_control)
        || remote.bytes().any(|byte| byte.is_ascii_whitespace()) { return None; }
    if let Some(rest) = remote.strip_prefix("https://") {
        let (authority, path) = rest.split_once('/')?;
        if authority.is_empty() || authority.contains('@') || authority.contains(':') ||
            path.is_empty() || path.contains('?') || path.contains('#') { return None; }
        return Some("HTTPS");
    }
    if let Some(rest) = remote.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        if host.is_empty() || host.contains('/') || path.is_empty() || path.contains('@') {
            return None;
        }
        return Some("SSH");
    }
    if let Some(rest) = remote.strip_prefix("ssh://git@") {
        let (host, path) = rest.split_once('/')?;
        if host.is_empty() || host.contains(':') || path.is_empty() || path.contains('@') {
            return None;
        }
        return Some("SSH");
    }
    None
}
const REPARSE_POINT: u32 = 0x400;
const GIT_TIMEOUT: Duration = Duration::from_secs(25);
const SCHEMA: [(&str, &str); 8] = [
    ("gogoke_v37_worktree_sources", "CREATE TABLE gogoke_v37_worktree_sources(repository_id TEXT PRIMARY KEY,source_path TEXT NOT NULL UNIQUE,source_identity TEXT NOT NULL UNIQUE,common_path TEXT NOT NULL,common_identity TEXT NOT NULL,remote_kind TEXT NOT NULL CHECK(remote_kind IN ('HTTPS','SSH')),baseline_commit TEXT NOT NULL,git_digest TEXT NOT NULL,git_version TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision=1)) STRICT"),
    ("gogoke_v37_worktree_operations", "CREATE TABLE gogoke_v37_worktree_operations(request_id TEXT PRIMARY KEY,request_hash TEXT NOT NULL,repository_id TEXT NOT NULL,domain_id TEXT NOT NULL,seat_id TEXT NOT NULL,worktree_id TEXT NOT NULL UNIQUE,path_id TEXT NOT NULL UNIQUE,seat_incarnation TEXT NOT NULL,seat_generation INTEGER NOT NULL,seat_revision INTEGER NOT NULL,instance_id TEXT NOT NULL,permission_tier TEXT NOT NULL,phase TEXT NOT NULL CHECK(phase IN ('INTENT','UNKNOWN','REGISTERED')),cause TEXT NOT NULL DEFAULT '') STRICT"),
    ("gogoke_v37_worktrees", "CREATE TABLE gogoke_v37_worktrees(worktree_id TEXT PRIMARY KEY,path_id TEXT NOT NULL UNIQUE,repository_id TEXT NOT NULL REFERENCES gogoke_v37_worktree_sources(repository_id),domain_id TEXT NOT NULL,seat_id TEXT NOT NULL,seat_incarnation TEXT NOT NULL,seat_generation INTEGER NOT NULL,seat_revision INTEGER NOT NULL,permission_tier TEXT NOT NULL,instance_id TEXT NOT NULL,source_revision INTEGER NOT NULL,worktree_path TEXT NOT NULL UNIQUE,worktree_identity TEXT NOT NULL UNIQUE,git_pointer_hash TEXT NOT NULL,git_pointer_len INTEGER NOT NULL,git_pointer_identity TEXT NOT NULL UNIQUE,common_identity TEXT NOT NULL,baseline_commit TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('REGISTERED','UNKNOWN')),revision INTEGER NOT NULL CHECK(revision=1)) STRICT"),
    ("gogoke_v37_worktree_programs", "CREATE TABLE gogoke_v37_worktree_programs(repository_id TEXT PRIMARY KEY REFERENCES gogoke_v37_worktree_sources(repository_id),git_path TEXT NOT NULL) STRICT"),
    ("gogoke_v37_worktree_spaces", "CREATE TABLE gogoke_v37_worktree_spaces(space_id TEXT PRIMARY KEY,path_id TEXT NOT NULL UNIQUE,classification TEXT NOT NULL CHECK(classification IN ('SINGLE','MIXED')),state TEXT NOT NULL CHECK(state IN ('ACTIVE','UNKNOWN','CLEANED')),revision INTEGER NOT NULL CHECK(revision>=1)) STRICT"),
    ("gogoke_v37_worktree_members", "CREATE TABLE gogoke_v37_worktree_members(worktree_id TEXT PRIMARY KEY REFERENCES gogoke_v37_worktrees(worktree_id),space_id TEXT NOT NULL REFERENCES gogoke_v37_worktree_spaces(space_id),repository_id TEXT NOT NULL,domain_id TEXT NOT NULL,seat_id TEXT NOT NULL,UNIQUE(space_id,repository_id,domain_id,seat_id)) STRICT"),
    ("gogoke_v37_worktree_lifecycle", "CREATE TABLE gogoke_v37_worktree_lifecycle(worktree_id TEXT PRIMARY KEY REFERENCES gogoke_v37_worktrees(worktree_id),state TEXT NOT NULL CHECK(state IN ('CREATED','REGISTERED','MERGE_INTENT','MERGE_UNKNOWN','MERGED','CLEANUP_INTENT','CLEANUP_UNKNOWN','CLEANED')),revision INTEGER NOT NULL CHECK(revision>=1),merge_reason TEXT,merge_target_commit TEXT,stop_fact_id TEXT) STRICT"),
    ("gogoke_v37_worktree_lifecycle_ops", "CREATE TABLE gogoke_v37_worktree_lifecycle_ops(request_id TEXT PRIMARY KEY,request_hash TEXT NOT NULL,worktree_id TEXT NOT NULL REFERENCES gogoke_v37_worktrees(worktree_id),operation TEXT NOT NULL CHECK(operation IN ('REGISTER','MERGE','CLEANUP')),phase TEXT NOT NULL CHECK(phase IN ('INTENT','UNKNOWN','APPLIED','FAILED')),cause TEXT NOT NULL DEFAULT '',result_commit TEXT,UNIQUE(worktree_id,operation,request_id)) STRICT"),
];

#[derive(Debug)]
pub(crate) enum WorktreeError {
    Invalid(&'static str),
    Denied,
    Conflict,
    Unknown,
    SchemaDrift,
    Git(String),
    Io(io::Error),
    Root(crate::root::RootLockError),
    Store(AtomicError),
    Open(SameOpenError),
    Authority(crate::store::orchestration::OrchestrationError),
    Process(crate::process::ProcessCustodyError),
    Seat(crate::store::seat::SeatError),
    Environment(std::env::VarError),
    ParseInt(&'static str, std::num::ParseIntError),
    Utf8(&'static str, std::str::Utf8Error),
    CommitUnknown(SameOpenError),
    RollbackUnknown {
        primary: Box<WorktreeError>,
        rollback: SameOpenError,
    },
    Multiple {
        phase: &'static str,
        primary: Box<WorktreeError>,
        secondary: Box<WorktreeError>,
    },
}
impl From<io::Error> for WorktreeError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<crate::root::RootLockError> for WorktreeError {
    fn from(e: crate::root::RootLockError) -> Self {
        Self::Root(e)
    }
}
impl From<AtomicError> for WorktreeError {
    fn from(e: AtomicError) -> Self {
        Self::Store(e)
    }
}
impl From<SameOpenError> for WorktreeError {
    fn from(e: SameOpenError) -> Self {
        Self::Open(e)
    }
}
impl From<crate::store::orchestration::OrchestrationError> for WorktreeError {
    fn from(e: crate::store::orchestration::OrchestrationError) -> Self {
        Self::Authority(e)
    }
}
impl From<crate::store::seat::SeatError> for WorktreeError {
    fn from(e: crate::store::seat::SeatError) -> Self {
        Self::Seat(e)
    }
}
impl From<crate::process::ProcessCustodyError> for WorktreeError {
    fn from(e: crate::process::ProcessCustodyError) -> Self {
        Self::Process(e)
    }
}
impl From<std::env::VarError> for WorktreeError {
    fn from(e: std::env::VarError) -> Self {
        Self::Environment(e)
    }
}
type Result<T> = std::result::Result<T, WorktreeError>;

fn parse_i64(value: &str, field: &'static str) -> Result<i64> {
    value
        .parse::<i64>()
        .map_err(|error| WorktreeError::ParseInt(field, error))
}
fn parse_u64(value: &str, field: &'static str) -> Result<u64> {
    value
        .parse::<u64>()
        .map_err(|error| WorktreeError::ParseInt(field, error))
}
fn joined(phase: &'static str, primary: WorktreeError, secondary: WorktreeError) -> WorktreeError {
    WorktreeError::Multiple {
        phase,
        primary: Box::new(primary),
        secondary: Box::new(secondary),
    }
}
fn append_error(primary: &mut Option<WorktreeError>, phase: &'static str, error: WorktreeError) {
    let next = match primary.take() {
        Some(previous) => joined(phase, previous, error),
        None => error,
    };
    *primary = Some(next);
}
fn git_stdout_shape(tag: &str, output: bool, frames: usize) -> bool {
    if tag == "source_attrs" || tag == "f2_status" || tag == "child_index" || tag == "child_tree" {
        output
    } else if !output {
        frames == 0
    } else {
        frames == 1
    }
}
fn has_attribute_surface(tree_paths: &str) -> bool {
    tree_paths.lines().any(|line| {
        let name = line.trim_end_matches('\r');
        name.starts_with('"')
            || name.contains('\\')
            || name == ".gitattributes"
            || name.ends_with("/.gitattributes")
    })
}
fn process_stdout_eof(error: &crate::process::ProcessCustodyError) -> bool {
    match error {
        crate::process::ProcessCustodyError::ProtocolPipe(source) => {
            source.kind() == io::ErrorKind::UnexpectedEof
        }
        crate::process::ProcessCustodyError::ProtocolEvidence { cause, .. } => {
            process_stdout_eof(cause)
        }
        _ => false,
    }
}
fn uncertain(error: &WorktreeError) -> bool {
    match error {
        WorktreeError::CommitUnknown(_) | WorktreeError::RollbackUnknown { .. } => true,
        WorktreeError::Authority(
            crate::store::orchestration::OrchestrationError::CommitUnknown
            | crate::store::orchestration::OrchestrationError::CommitUnknownWithCause(_),
        ) => true,
        WorktreeError::Multiple {
            primary, secondary, ..
        } => uncertain(primary) || uncertain(secondary),
        _ => false,
    }
}

fn atom(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 96
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn hex_commit(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}
fn require_absent(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(WorktreeError::Denied),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
fn transaction<T>(
    db: &mut VerifiedDatabaseConnection<'_>,
    f: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<T>,
) -> Result<T> {
    db.execute("BEGIN IMMEDIATE")?;
    match f(db) {
        Ok(value) => {
            db.execute("COMMIT").map_err(WorktreeError::CommitUnknown)?;
            Ok(value)
        }
        Err(error) => match db.execute("ROLLBACK") {
            Ok(()) => Err(error),
            Err(rollback) => Err(WorktreeError::RollbackUnknown {
                primary: Box::new(error),
                rollback,
            }),
        },
    }
}
fn family(db: &VerifiedDatabaseConnection<'_>) -> Result<Vec<(String, String)>> {
    let q = Statement::prepare(db.as_ptr(), "SELECT name,sql FROM main.sqlite_schema WHERE lower(substr(name,1,19))='gogoke_v37_worktree' ORDER BY name")?;
    let mut rows = Vec::new();
    while q.step_row()? {
        rows.push((q.column_text(0)?, q.column_text(1)?));
    }
    Ok(rows)
}
fn no_shadow(db: &VerifiedDatabaseConnection<'_>) -> Result<()> {
    for sql in [
        "SELECT 1 FROM temp.sqlite_schema WHERE lower(substr(name,1,19))='gogoke_v37_worktree' OR lower(substr(tbl_name,1,19))='gogoke_v37_worktree' LIMIT 1",
        "SELECT 1 FROM main.sqlite_schema WHERE type IN ('trigger','index') AND sql IS NOT NULL AND lower(substr(tbl_name,1,19))='gogoke_v37_worktree' LIMIT 1",
    ] {
        if Statement::prepare(db.as_ptr(), sql)?.step_row()? { return Err(WorktreeError::SchemaDrift); }
    }
    Ok(())
}
/// Exact schema family verification, including TEMP shadow and trigger refusal.
pub(crate) fn initialize_schema(db: &mut VerifiedDatabaseConnection<'_>) -> Result<()> {
    no_shadow(db)?;
    let mut expected: Vec<_> = SCHEMA
        .iter()
        .map(|(n, s)| (n.to_string(), s.to_string()))
        .collect();
    expected.sort_by(|a, b| a.0.cmp(&b.0));
    let observed = family(db)?;
    if observed == expected {
        return Ok(());
    }
    let mut previous: Vec<_> = SCHEMA[..3].iter()
        .map(|(name, sql)| (name.to_string(), sql.to_string())).collect();
    previous.sort_by(|a, b| a.0.cmp(&b.0));
    let mut current_m1: Vec<_> = SCHEMA[..4].iter()
        .map(|(name, sql)| (name.to_string(), sql.to_string())).collect();
    current_m1.sort_by(|a, b| a.0.cmp(&b.0));
    // An unpinned historical source can coexist with the F.2 graph tables.
    // Match that exact family, then add only the missing empty pin table;
    // never infer a Git executable from the old source's digest or version.
    let mut unpinned_f2: Vec<_> = SCHEMA.iter()
        .filter(|(name, _)| *name != "gogoke_v37_worktree_programs")
        .map(|(name, sql)| (name.to_string(), sql.to_string())).collect();
    unpinned_f2.sort_by(|a, b| a.0.cmp(&b.0));
    if !observed.is_empty() && observed != previous && observed != current_m1 && observed != unpinned_f2 {
        return Err(WorktreeError::SchemaDrift);
    }
    transaction(db, |db| {
        no_shadow(db)?;
        if family(db)? != observed {
            return Err(WorktreeError::SchemaDrift);
        }
        // Preserve the exact earlier tables and every record. A missing
        // native program registration is denied at restore, never inferred.
        let added = if observed.is_empty() { &SCHEMA[..] }
            else if observed == previous { &SCHEMA[3..] }
            else if observed == unpinned_f2 { &SCHEMA[3..4] } else { &SCHEMA[4..] };
        for (_, sql) in added {
            db.execute(sql)?;
        }
        if family(db)? != expected {
            return Err(WorktreeError::SchemaDrift);
        }
        Ok(())
    })
}

/// The exact Git program is supplied by native private Owner configuration.
/// The held handle denies writes and deletes while a source/create operation
/// uses it. Its path and version never come from a model or worktree request.
pub(crate) struct GitProgramPin {
    path: PathBuf,
    digest: String,
    version: String,
    _file: File,
    profile_id: String,
    owner_seat_id: String,
    policy_revision: String,
}
impl GitProgramPin {
    pub(crate) fn observe(
        db: &mut VerifiedDatabaseConnection<'_>,
        owner: &OwnerIssuer,
        root: &RootLock,
        path: &Path,
        custodian: &mut ProcessCustodian,
    ) -> Result<Self> {
        Self::observe_checked(db, owner, root, path, custodian, None)
    }

    fn observe_checked(
        db: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer,
        root: &RootLock, path: &Path, custodian: &mut ProcessCustodian,
        expected: Option<(&str, &str)>,
    ) -> Result<Self> {
        let identity = read_product_identity(db, owner)?;
        if identity.root_identity != root.canonical_root().identity.opaque()
            || !identity.policy_revision.bytes().all(|b| b.is_ascii_digit())
            || identity.policy_revision.starts_with('0')
        {
            return Err(WorktreeError::Denied);
        }
        let canonical = fs::canonicalize(path)?;
        if !canonical.is_absolute()
            || fs::symlink_metadata(path)?.file_attributes() & REPARSE_POINT != 0
        {
            return Err(WorktreeError::Denied);
        }
        let mut file = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&canonical)?;
        let meta = file.metadata()?;
        if !meta.is_file() || meta.len() == 0 || meta.len() > 128 * 1024 * 1024 {
            return Err(WorktreeError::Denied);
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        if bytes.len() as u64 != meta.len() {
            return Err(WorktreeError::Denied);
        }
        let digest = content_hash(&bytes);
        // Restore must refuse changed bytes before any --version launch.
        // The same held file continues to guard the image used below.
        if expected.is_some_and(|(registered, _)| digest != registered) {
            return Err(WorktreeError::Denied);
        }
        let mut pin = Self {
            // Git derives its installation-relative resources from the image
            // path. Its mingw backend treats a verbatim prefix as //?/ rather
            // than a drive path. Keep the same held file/byte pin, but launch
            // the native local DOS spelling so its resources resolve exactly.
            path: git_launch_path(&canonical)?,
            digest,
            version: String::new(),
            _file: file,
            profile_id: identity.profile_id,
            owner_seat_id: identity.seat_id,
            policy_revision: identity.policy_revision,
        };
        let output = git(
            db,
            root,
            custodian,
            &pin,
            "git_version",
            None,
            &["--version".into()],
            true,
        )?;
        let version = output.trim_end_matches(['\r', '\n']);
        if !version.starts_with("git version ")
            || version.len() > 128
            || version.chars().any(char::is_control)
        {
            return Err(WorktreeError::Git("invalid pinned Git version".into()));
        }
        pin.version = version.to_owned();
        if expected.is_some_and(|(_, registered)| pin.version != registered) {
            return Err(WorktreeError::Denied);
        }
        Ok(pin)
    }
    fn repin(&self) -> Result<()> {
        // The original handle denies replacement; compare the launched path as
        // well, so a different image cannot satisfy the registration pin.
        let meta = fs::metadata(&self.path)?;
        if !meta.is_file() || meta.len() == 0 || meta.len() > 128 * 1024 * 1024 {
            return Err(WorktreeError::Denied);
        }
        let actual = fs::read(&self.path)?;
        if content_hash(&actual) != self.digest {
            return Err(WorktreeError::Denied);
        }
        Ok(())
    }
}

fn git_launch_path(canonical: &Path) -> Result<PathBuf> {
    match canonical.components().next() {
        Some(std::path::Component::Prefix(prefix))
            if matches!(prefix.kind(), std::path::Prefix::VerbatimDisk(_)) => {
                let wide: Vec<u16> = canonical.as_os_str().encode_wide().collect();
                Ok(PathBuf::from(std::ffi::OsString::from_wide(&wide[4..])))
            }
        Some(std::path::Component::Prefix(prefix))
            if matches!(prefix.kind(), std::path::Prefix::Disk(_)) => Ok(canonical.to_path_buf()),
        _ => Err(WorktreeError::Invalid("Git requires a native local DOS path")),
    }
}

/// Restore the Owner-registered program from native facts after restart.
/// The request selects a repository ID only, never a path/digest/version.
pub(crate) fn resolve_registered_git(
    db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock, owner: &OwnerIssuer,
    repository_id: &str, custodian: &mut ProcessCustodian,
) -> Result<GitProgramPin> {
    if !atom(repository_id) { return Err(WorktreeError::Invalid("repository_id")); }
    let (path, digest, version) = transaction(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        let row = Statement::prepare(db.as_ptr(),
            "SELECT p.git_path,s.git_digest,s.git_version FROM main.gogoke_v37_worktree_sources AS s JOIN main.gogoke_v37_worktree_programs AS p ON p.repository_id=s.repository_id WHERE s.repository_id=?1 AND s.revision=1")?;
        row.bind_text(1, repository_id)?;
        if !row.step_row()? { return Err(WorktreeError::Denied); }
        let facts = (PathBuf::from(row.column_text(0)?), row.column_text(1)?, row.column_text(2)?);
        if row.step_row()? { return Err(WorktreeError::SchemaDrift); }
        Ok(facts)
    })?;
    GitProgramPin::observe_checked(db, owner, root, &path, custodian, Some((&digest, &version)))
}

fn git(
    db: &mut VerifiedDatabaseConnection<'_>,
    root: &RootLock,
    custodian: &mut ProcessCustodian,
    pin: &GitProgramPin,
    tag: &str,
    cwd: Option<&Path>,
    args: &[String],
    output: bool,
) -> Result<String> {
    pin.repin()?;
    let custody_home = &root.canonical_root().canonical_path;
    for attributes in [
        custody_home.join("git").join("attributes"),
        custody_home.join(".config").join("git").join("attributes"),
    ] {
        require_absent(&attributes)?;
    }
    let mut launch = ProcessLaunch::new(&pin.path);
    launch.arguments = vec![
        "-c".into(),
        "core.hooksPath=NUL".into(),
        "-c".into(),
        "core.fsmonitor=false".into(),
        "-c".into(),
        "core.quotePath=true".into(),
    ];
    launch.arguments.extend_from_slice(args);
    launch.current_directory = Some(git_launch_path(
        cwd.unwrap_or(&root.canonical_root().canonical_path))?);
    // Keep bounded native stderr custody even for Git commands with no stdout.
    launch.protocol_stdio = true;
    launch.persistent_protocol_stdio = true;
    launch.environment = Some(vec![
        ("SystemRoot".into(), std::env::var("SystemRoot")?),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ("GIT_CONFIG_GLOBAL".into(), "NUL".into()),
        ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ("GIT_NO_REPLACE_OBJECTS".into(), "1".into()),
        ("GIT_NO_LAZY_FETCH".into(), "1".into()),
        ("GIT_ATTR_NOSYSTEM".into(), "1".into()),
        ("HOME".into(), custody_home.to_string_lossy().into_owned()),
        (
            "XDG_CONFIG_HOME".into(),
            custody_home.to_string_lossy().into_owned(),
        ),
    ]);
    let request = PrepareRequest {
        binding: NativeBinding {
            binary_digest_sha256: pin.digest.clone(),
            profile_id: pin.profile_id.clone(),
            domain_id: pin.owner_seat_id.clone(),
            generation: pin.policy_revision.clone(),
        },
        launch,
    };
    let prepared = custodian
        .prepare(&request)
        .map_err(|e| WorktreeError::Git(format!("Git prepare: {e}")))?;
    let operation = format!(
        "f_git_{}_{}",
        tag,
        prepared.ticket.opaque().replace('-', "_")
    );
    if let Err(error) = record_prepared_process(db, &operation, &prepared) {
        RootLock::poison_identity(&root.canonical_root().identity);
        let primary = WorktreeError::Authority(error);
        return match custodian.abort_prepared(&prepared) {
            Ok(true) => Err(primary),
            Ok(false) => Err(joined(
                "Git PREPARED abort missing",
                primary,
                WorktreeError::Unknown,
            )),
            Err(abort) => {
                RootLock::poison_identity(&root.canonical_root().identity);
                Err(joined(
                    "Git PREPARED abort",
                    primary,
                    WorktreeError::Git(abort.to_string()),
                ))
            }
        };
    }
    let activated = match custodian.activate(&prepared) {
        Ok(activated) => activated,
        Err(activate) => {
            RootLock::poison_identity(&root.canonical_root().identity);
            let primary = WorktreeError::Git(format!("Git activate: {activate}"));
            return match mark_process_unknown(db, &operation, &prepared) {
                Ok(()) => Err(primary),
                Err(persist) => Err(joined("Git activate UNKNOWN", primary, persist.into())),
            };
        }
    };
    if let Err(error) = mark_process_active(db, &operation, &activated) {
        RootLock::poison_identity(&root.canonical_root().identity);
        let mut failure = WorktreeError::Authority(error);
        if let Err(stop) = custodian.stop(&activated.ticket, StopBudgets::production(), || Ok(())) {
            failure = joined(
                "Git active stop",
                failure,
                WorktreeError::Git(stop.to_string()),
            );
        }
        if let Err(persist) = mark_process_unknown(db, &operation, &activated) {
            failure = joined("Git active UNKNOWN", failure, persist.into());
        }
        return Err(failure);
    }
    let started = Instant::now();
    let closed = custodian
        .close_child_input(&activated.ticket)
        .map_err(WorktreeError::Process);
    let captured = closed.and_then(|()| {
        let mut bytes = Vec::new();
        let mut frames = 0usize;
        loop {
            let remaining = GIT_TIMEOUT.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(WorktreeError::Io(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Git stdout total deadline",
                )));
            }
            match custodian.read_persistent_child_frame(&activated.ticket, remaining) {
                Ok(frame) => {
                    if frame.custody() != &activated {
                        return Err(WorktreeError::Denied);
                    }
                    let data = frame.bytes();
                    if data.last() != Some(&b'\n')
                        || bytes.len().saturating_add(data.len()) > 65_536
                    {
                        return Err(WorktreeError::Git(
                            "Git stdout frame or aggregate bound".into(),
                        ));
                    }
                    bytes.extend_from_slice(data);
                    frames += 1;
                    if frames > 1024 {
                        return Err(WorktreeError::Git("Git stdout line bound".into()));
                    }
                }
                Err(error) if process_stdout_eof(&error) => break,
                Err(error) => return Err(WorktreeError::Process(error)),
            }
        }
        let text = String::from_utf8(bytes)
            .map_err(|error| WorktreeError::Utf8("Git stdout", error.utf8_error()))?;
        if !git_stdout_shape(tag, output, frames) {
            return Err(WorktreeError::Git(format!(
                "Git stdout shape for {tag}: {frames} lines"
            )));
        }
        Ok(text)
    });
    let active = match custodian.active(&activated.ticket) {
        Some(active) => active,
        None => {
            RootLock::poison_identity(&root.canonical_root().identity);
            return Err(joined(
                "Git active custody missing",
                captured.err().unwrap_or(WorktreeError::Unknown),
                WorktreeError::Unknown,
            ));
        }
    };
    let exited = active.wait(GIT_TIMEOUT.saturating_sub(started.elapsed()));
    let exit = active.exit_code();
    let stderr = active.stderr_tail();
    let mut primary = None;
    let text = match captured {
        Ok(text) => Some(text),
        Err(error) => {
            append_error(
                &mut primary,
                "Git stdout",
                joined("Git stderr", error, WorktreeError::Git(stderr.clone())),
            );
            None
        }
    };
    match exited {
        Ok(true) => {}
        Ok(false) => append_error(
            &mut primary,
            "Git wait",
            WorktreeError::Git(format!("Git deadline exceeded; original stderr: {stderr}")),
        ),
        Err(error) => append_error(
            &mut primary,
            "Git wait",
            WorktreeError::Git(format!("Git wait: {error}; original stderr: {stderr}")),
        ),
    }
    match exit {
        Ok(Some(0)) => {}
        Ok(other) => append_error(
            &mut primary,
            "Git exit",
            WorktreeError::Git(format!("Git exit {other:?}; original stderr: {stderr}")),
        ),
        Err(error) => append_error(
            &mut primary,
            "Git exit",
            WorktreeError::Git(format!(
                "Git exit observation: {error}; original stderr: {stderr}"
            )),
        ),
    }
    let proof = match custodian.stop(&activated.ticket, StopBudgets::production(), || Ok(())) {
        Ok(proof) => proof,
        Err(error) => {
            RootLock::poison_identity(&root.canonical_root().identity);
            append_error(&mut primary, "Git stop", WorktreeError::Process(error));
            if let Err(persist) = mark_process_unknown(db, &operation, &activated) {
                append_error(&mut primary, "Git stop UNKNOWN", persist.into());
            }
            return Err(primary.expect("stop error recorded"));
        }
    };
    let revision = match mark_process_stopped(db, &operation, &proof) {
        Ok(revision) => revision,
        Err(error) => {
            RootLock::poison_identity(&root.canonical_root().identity);
            append_error(
                &mut primary,
                "Git stop proof",
                WorktreeError::Git(format!("native stop proof: {proof:?}")),
            );
            append_error(&mut primary, "Git STOPPED persistence", error.into());
            if let Err(persist) = mark_process_unknown(db, &operation, &activated) {
                append_error(&mut primary, "Git stop UNKNOWN", persist.into());
            }
            return Err(primary.expect("stop persistence error recorded"));
        }
    };
    if let Err(error) = custodian.confirm_stop_durable(&DurableStopConfirmation {
        ticket: proof.ticket.clone(),
        custodian_nonce: proof.custodian_nonce.clone(),
        identity: proof.identity.clone(),
        proof_hash: proof.proof_hash(),
        durable_revision: revision,
    }) {
        RootLock::poison_identity(&root.canonical_root().identity);
        append_error(
            &mut primary,
            "Git confirm STOPPED",
            WorktreeError::Process(error),
        );
        return Err(primary.expect("confirm error recorded"));
    }
    if let Some(error) = primary {
        return Err(error);
    }
    Ok(text
        .expect("successful Git output capture")
        .trim_end_matches(['\r', '\n'])
        .to_owned())
}

#[cfg(all(test, windows))]
pub(crate) mod tests {
    use super::*;
    use crate::store::same_open::{create_new, route_b_test_guard};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "gogoke-f-worktree-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    /// Cloud-only setup through the same native Git custody, with an offline
    /// synthetic source. Product-entry tests consume its path, not a fake pin.
    pub(crate) fn make_source_fixture(
        db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock,
        owner: &OwnerIssuer, custodian: &mut ProcessCustodian,
    ) -> PathBuf {
        let program = std::env::var_os("GOGOKE_CONTROLLED_GIT_PATH")
            .expect("cloud must bind actual installed Git backend");
        let pin = GitProgramPin::observe(db, owner, root, Path::new(&program), custodian).unwrap();
        let source = git_launch_path(&root.canonical_root().canonical_path.join("synthetic-source")).unwrap();
        let mut run = |tag: &str, cwd: Option<&Path>, args: &[String]| {
            git(db, root, custodian, &pin, tag, cwd, args, false).unwrap()
        };
        run("fixture_init", None, &["init".into(), "--quiet".into(), source.to_str().unwrap().into()]);
        fs::write(source.join("README.md"), b"synthetic offline product worktree\n").unwrap();
        for (tag, key, value) in [
            ("fixture_name", "user.name", "Fixture"),
            ("fixture_email", "user.email", "fixture@example.invalid"),
            ("fixture_remote", "remote.origin.url", SOURCE_REMOTE),
        ] {
            run(tag, Some(&source), &["config".into(), "--local".into(), key.into(), value.into()]);
        }
        run("fixture_add", Some(&source), &["add".into(), "--".into(), "README.md".into()]);
        run("fixture_commit", Some(&source), &["commit".into(), "--quiet".into(), "-m".into(), "fixture baseline".into()]);
        source
    }

    #[test]
    fn restore_refuses_changed_git_bytes_before_any_native_process_prepare() {
        use crate::store::authority::{initialize_profile, initialize_process_custody_schema};
        let _guard = route_b_test_guard();
        let path = scratch("changed-git");
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut db = create_new(&root, &path.join("state.sqlite")).unwrap();
        let owner = initialize_profile(&mut db, &root).unwrap();
        initialize_process_custody_schema(&mut db).unwrap();
        initialize_schema(&mut db).unwrap();
        let program = path.join("registered-git.exe");
        fs::write(&program, b"changed program bytes, must never execute").unwrap();
        let insert = Statement::prepare(db.as_ptr(), "INSERT INTO main.gogoke_v37_worktree_sources VALUES('legacyRepo','sourceA','sourceId','commonA','commonId','HTTPS','baseline',?1,'git version fixture',1)").unwrap();
        insert.bind_text(1, &content_hash(b"original registered program bytes")).unwrap();
        insert.step_done().unwrap();
        drop(insert);
        let register = Statement::prepare(db.as_ptr(), "INSERT INTO main.gogoke_v37_worktree_programs VALUES('legacyRepo',?1)").unwrap();
        register.bind_text(1, program.to_str().unwrap()).unwrap();
        register.step_done().unwrap();
        drop(register);
        let mut custodian = ProcessCustodian::new().unwrap();
        assert!(matches!(resolve_registered_git(&mut db, &root, &owner, "legacyRepo", &mut custodian), Err(WorktreeError::Denied)));
        let count = Statement::prepare(db.as_ptr(), "SELECT count(*) FROM main.gogoke_coordination_process_custody").unwrap();
        assert!(count.step_row().unwrap());
        assert_eq!(count.column_text(0).unwrap(), "0", "no prepare/version subprocess on changed bytes");
        drop(count);
        db.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn f2_schema_extends_exact_m1_rows_without_recreating_them() {
        let _guard = route_b_test_guard();
        let path = scratch("f2-schema-upgrade");
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut db = create_new(&root, &path.join("state.sqlite")).unwrap();
        for (_, sql) in &SCHEMA[..4] { db.execute(sql).unwrap(); }
        db.execute("INSERT INTO main.gogoke_v37_worktree_sources VALUES('repoA','sourceA','sourceId','commonA','commonId','HTTPS','baseline','digest','git version fixture',1)").unwrap();
        let before = family(&db).unwrap();
        assert_eq!(before.len(), 4);
        initialize_schema(&mut db).unwrap();
        initialize_schema(&mut db).unwrap();
        assert_eq!(family(&db).unwrap().len(), SCHEMA.len());
        let q = Statement::prepare(db.as_ptr(),
            "SELECT source_path,source_identity FROM main.gogoke_v37_worktree_sources WHERE repository_id='repoA'").unwrap();
        assert!(q.step_row().unwrap());
        assert_eq!((q.column_text(0).unwrap(),q.column_text(1).unwrap()),
            ("sourceA".into(),"sourceId".into()));
        drop(q);
        db.close_checked().unwrap(); drop(root); fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn schema_rejects_trigger_and_temp_shadow() {
        let _guard = route_b_test_guard();
        let path = scratch("schema");
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut db = create_new(&root, &path.join("state.sqlite")).unwrap();
        initialize_schema(&mut db).unwrap();
        initialize_schema(&mut db).unwrap();
        db.execute("CREATE TRIGGER malicious AFTER INSERT ON gogoke_v37_worktree_sources BEGIN SELECT 1; END").unwrap();
        assert!(matches!(
            initialize_schema(&mut db),
            Err(WorktreeError::SchemaDrift)
        ));
        db.execute("DROP TRIGGER malicious").unwrap();
        db.execute("CREATE TEMP TABLE gogoke_v37_worktree_sources(x TEXT)")
            .unwrap();
        assert!(matches!(
            initialize_schema(&mut db),
            Err(WorktreeError::SchemaDrift)
        ));
        drop(db);
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn pointer_accepts_only_link_into_expected_common_directory() {
        let path = scratch("pointer");
        let common = path.join("source.git");
        let gitdir = common.join("worktrees").join("seat1");
        let worktree = path.join("worktree");
        fs::create_dir_all(&gitdir).unwrap();
        fs::create_dir(&worktree).unwrap();
        let dotgit = worktree.join(".git");
        fs::write(&dotgit, format!("gitdir: {}\n", gitdir.display())).unwrap();
        assert!(pointer(&dotgit, &common).is_ok());
        fs::write(&dotgit, "gitdir: C:/tmp/unrelated/worktrees/seat1\n").unwrap();
        assert!(pointer(&dotgit, &common).is_err());
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn native_remote_classification_never_admits_inline_credentials() {
        assert_eq!(remote_kind(SOURCE_REMOTE), Some("HTTPS"));
        assert_eq!(remote_kind(SOURCE_REMOTE_SSH), Some("SSH"));
        let credential_url = format!(
            "https://user:{}@github.com/taiyun668/gogoke-seat-testbed.git",
            "placeholder"
        );
        assert_eq!(remote_kind(&credential_url), None);
        assert_eq!(
            remote_kind("https://github.com/taiyun668/gogoke-seat-testbed-copy.git"),
            Some("HTTPS")
        );
    }

    #[test]
    fn git_output_shape_never_discards_extra_lines() {
        assert!(git_stdout_shape("source_attrs", true, 0));
        assert!(git_stdout_shape("source_attrs", true, 3));
        assert!(git_stdout_shape("source_head", true, 1));
        assert!(!git_stdout_shape("source_head", true, 2));
        assert!(git_stdout_shape("worktree_add", false, 0));
        assert!(!git_stdout_shape("worktree_add", false, 1));
    }

    #[test]
    fn committed_tree_attribute_scan_uses_exact_basename() {
        assert!(!has_attribute_surface("README.md\nsrc/not.gitattributes\n"));
        assert!(has_attribute_surface("README.md\nsrc/.gitattributes\n"));
        assert!(has_attribute_surface(".gitattributes\n"));
        assert!(has_attribute_surface("\"src/ambiguous\\nname\"\n"));
    }

    #[test]
    fn real_custodied_git_creates_bound_linked_worktree_and_fences_unknown() {
        use crate::store::authority::{initialize_process_custody_schema, initialize_profile};
        use crate::store::seat::{self, CreateSeat, Kind, NativeOrigin, StoreTemplate};

        let _guard = route_b_test_guard();
        let path = scratch("real-git");
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let database = path.join("state.sqlite");
        let mut db = create_new(&root, &database).unwrap();
        db.execute("PRAGMA foreign_keys=ON").unwrap();
        let owner = initialize_profile(&mut db, &root).unwrap();
        initialize_process_custody_schema(&mut db).unwrap();
        crate::store::instance::initialize_schema(&mut db).unwrap();
        seat::initialize_schema(&mut db).unwrap();
        initialize_schema(&mut db).unwrap();
        let insert=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceA','codex','homeA','identityA','sha256:test','test','INSTALLED','LOGGED_OUT',1)").unwrap();
        insert.step_done().unwrap();
        drop(insert);
        seat::store_template(
            &mut db,
            NativeOrigin::user(&owner),
            StoreTemplate {
                domain_id: "projectA",
                template_id: "templateA",
                settings_json: br#"{"permissionTier":"ISOLATED_WRITE"}"#,
            },
        )
        .unwrap();
        let seat = seat::create(
            &mut db,
            NativeOrigin::user(&owner),
            CreateSeat {
                domain_id: "projectA",
                seat_id: "seatA",
                template_id: "templateA",
                instance_id: Some("instanceA"),
                kind: Kind::Long,
                request_id: "seat-create",
                request_bytes: br#"{"seatId":"seatA"}"#,
            },
        )
        .unwrap()
        .seat;
        let git_path = std::env::var_os("GOGOKE_CONTROLLED_GIT_PATH")
            .expect("cloud F focused job must pin installed git.exe");
        let mut custodian = ProcessCustodian::new().unwrap();
        let pin =
            GitProgramPin::observe(&mut db, &owner, &root, Path::new(&git_path), &mut custodian)
                .unwrap();
        let source = path.join("synthetic-source");
        let mut run = |tag: &str, cwd: Option<&Path>, argv: &[&str]| {
            let args: Vec<String> = argv.iter().map(|value| (*value).to_owned()).collect();
            git(&mut db, &root, &mut custodian, &pin, tag, cwd, &args, false).unwrap()
        };
        run(
            "fixture_init",
            None,
            &["init", "--quiet", source.to_str().unwrap()],
        );
        fs::write(source.join("README.md"), b"synthetic offline worktree\n").unwrap();
        run(
            "fixture_name",
            Some(&source),
            &["config", "--local", "user.name", "Fixture"],
        );
        run(
            "fixture_email",
            Some(&source),
            &["config", "--local", "user.email", "fixture@example.invalid"],
        );
        run(
            "fixture_remote",
            Some(&source),
            &["config", "--local", "remote.origin.url", SOURCE_REMOTE],
        );
        run("fixture_add", Some(&source), &["add", "--", "README.md"]);
        run(
            "fixture_commit",
            Some(&source),
            &["commit", "--quiet", "-m", "fixture baseline"],
        );
        drop(run);
        register_source(
            &mut db,
            &root,
            &owner,
            &pin,
            &mut custodian,
            SourceRegistration {
                repository_id: "fixtureRepo",
                source_path: &source,
            },
        )
        .unwrap();
        let source_row=Statement::prepare(db.as_ptr(),"SELECT source_identity,common_identity,baseline_commit,remote_kind FROM main.gogoke_v37_worktree_sources WHERE repository_id='fixtureRepo'").unwrap();
        assert!(source_row.step_row().unwrap());
        assert_eq!(
            source_row.column_text(0).unwrap(),
            inspect_root(&source).unwrap().identity.opaque()
        );
        assert_eq!(
            source_row.column_text(1).unwrap(),
            inspect_root(&source.join(".git"))
                .unwrap()
                .identity
                .opaque()
        );
        assert!(hex_commit(&source_row.column_text(2).unwrap()));
        assert_eq!(source_row.column_text(3).unwrap(), "HTTPS");
        drop(source_row);
        let restored = resolve_registered_git(&mut db, &root, &owner,
            "fixtureRepo", &mut custodian).expect("native program re-observation after registration");
        assert_eq!(restored.digest, pin.digest);
        assert_eq!(restored.version, pin.version);
        drop(pin);
        let pin = restored;
        let raw = br#"{"repositoryId":"fixtureRepo","seatId":"seatA","targetId":"visibleTreeA"}"#;
        let first = CreateWorktree {
            request_id: "worktree-create",
            request_bytes: raw,
            repository_id: "fixtureRepo",
            target_id: "visibleTreeA",
            domain_id: "projectA",
            seat_id: "seatA",
        };
        let binding = create_worktree(&mut db, &root, &owner, &pin, &mut custodian, first).unwrap();
        assert_eq!(binding.worktree_id, "visibleTreeA");
        assert_ne!(
            binding.path.file_name().unwrap().to_string_lossy(),
            "visibleTreeA"
        );
        assert_eq!(
            fs::read(binding.path.join("README.md")).unwrap(),
            b"synthetic offline worktree\n"
        );
        assert_eq!(
            binding.identity,
            inspect_root(&binding.path).unwrap().identity
        );
        let (hash, len, identity, held) =
            pointer(&binding.path.join(".git"), &source.join(".git")).unwrap();
        assert_eq!(
            (hash, len, identity),
            (
                binding.pointer_hash.clone(),
                binding.pointer_len,
                binding.pointer_identity.clone()
            )
        );
        drop(held);
        assert!(resolve_for_launch(
            &db,
            &root,
            "visibleTreeA",
            "fixtureRepo",
            "projectA",
            "seatA",
            &seat.incarnation,
            seat.generation
        )
        .is_ok());
        assert!(matches!(
            resolve_for_launch(
                &db,
                &root,
                "visibleTreeA",
                "fixtureRepo",
                "projectA",
                "seatA",
                &seat.incarnation,
                seat.generation + 1
            ),
            Err(WorktreeError::Denied)
        ));
        let before = count(
            &db,
            "SELECT COUNT(*) FROM main.gogoke_coordination_process_custody",
        );
        let replay = readback_create(&db, &root, "worktree-create", raw)
            .unwrap()
            .unwrap();
        assert_eq!(replay.worktree_id, "visibleTreeA");
        assert!(matches!(
            create_worktree(
                &mut db,
                &root,
                &owner,
                &pin,
                &mut custodian,
                CreateWorktree {
                    request_id: "worktree-create",
                    request_bytes: raw,
                    repository_id: "fixtureRepo",
                    target_id: "visibleTreeA",
                    domain_id: "projectA",
                    seat_id: "seatA"
                }
            ),
            Err(WorktreeError::Conflict)
        ));
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) FROM main.gogoke_coordination_process_custody"
            ),
            before
        );
        assert_eq!(count(&db,"SELECT COUNT(*) FROM main.gogoke_coordination_process_custody WHERE state='STOPPED'"),before);
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM main.gogoke_v37_worktrees"),
            1
        );
        // A controlled source-config change happens after the first real
        // linked tree. The second request writes INTENT, then fails the
        // source check and durably enters UNKNOWN; a new ID cannot retry.
        git(
            &mut db,
            &root,
            &mut custodian,
            &pin,
            "fixture_promisor",
            Some(&source),
            &[
                "config".into(),
                "--local".into(),
                "extensions.partialClone".into(),
                "fixture-promisor".into(),
            ],
            false,
        )
        .unwrap();
        let before_unknown = count(
            &db,
            "SELECT COUNT(*) FROM main.gogoke_coordination_process_custody",
        );
        let bad = CreateWorktree {
            request_id: "worktree-unknown",
            request_bytes: b"second request",
            repository_id: "fixtureRepo",
            target_id: "visibleTreeB",
            domain_id: "projectA",
            seat_id: "seatA",
        };
        assert!(matches!(
            create_worktree(&mut db, &root, &owner, &pin, &mut custodian, bad),
            Err(WorktreeError::Denied)
        ));
        let phase=Statement::prepare(db.as_ptr(),"SELECT phase,cause FROM main.gogoke_v37_worktree_operations WHERE request_id='worktree-unknown'").unwrap();
        assert!(phase.step_row().unwrap());
        assert_eq!(phase.column_text(0).unwrap(), "UNKNOWN");
        assert!(!phase.column_text(1).unwrap().is_empty());
        drop(phase);
        assert!(matches!(
            create_worktree(
                &mut db,
                &root,
                &owner,
                &pin,
                &mut custodian,
                CreateWorktree {
                    request_id: "worktree-new-id",
                    request_bytes: b"third request",
                    repository_id: "fixtureRepo",
                    target_id: "visibleTreeC",
                    domain_id: "projectA",
                    seat_id: "seatA"
                }
            ),
            Err(WorktreeError::Unknown)
        ));
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) FROM main.gogoke_coordination_process_custody"
            ),
            before_unknown
        );
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM main.gogoke_v37_worktrees"),
            1
        );
        drop(replay);
        drop(binding);
        drop(pin);
        drop(custodian);
        db.close_checked().unwrap();
        drop(root);
        let actual = fs::canonicalize(&path).unwrap();
        let temp = fs::canonicalize(std::env::temp_dir()).unwrap();
        assert!(actual.starts_with(&temp));
        assert!(actual
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("gogoke-f-worktree-real-git"));
        fs::remove_dir_all(actual).unwrap();
    }

    fn count(db: &VerifiedDatabaseConnection<'_>, sql: &str) -> i64 {
        let q = Statement::prepare(db.as_ptr(), sql).unwrap();
        assert!(q.step_row().unwrap());
        q.column_text(0).unwrap().parse().unwrap()
    }
}

pub(crate) struct SourceRegistration<'a> {
    pub(crate) repository_id: &'a str,
    pub(crate) source_path: &'a Path,
}

fn source_checkout_has_no_external_drivers(
    db: &mut VerifiedDatabaseConnection<'_>,
    root: &RootLock,
    custodian: &mut ProcessCustodian,
    pin: &GitProgramPin,
    source: &Path,
    common: &Path,
    commit: &str,
) -> Result<()> {
    // `worktree add` checks out files and could otherwise run configured
    // filters. Unused installed filter drivers are harmless: refuse actual
    // attribute selection on the pinned tree and per-repo attribute files.
    let config_path = common.join("config");
    let config_meta = fs::symlink_metadata(&config_path)?;
    if !config_meta.is_file()
        || config_meta.file_attributes() & REPARSE_POINT != 0
        || config_meta.len() > 1024 * 1024
    {
        return Err(WorktreeError::Denied);
    }
    let config = fs::read(&config_path)?;
    if config.len() as u64 != config_meta.len() || config.contains(&0) {
        return Err(WorktreeError::Denied);
    }
    let config = String::from_utf8(config)
        .map_err(|error| WorktreeError::Utf8("Git config", error.utf8_error()))?
        .to_ascii_lowercase();
    if [
        "include",
        "attributesfile",
        "insteadof",
        "pushurl",
        "promisor",
        "partialclone",
    ]
    .iter()
    .any(|key| config.contains(key))
    {
        return Err(WorktreeError::Denied);
    }
    require_absent(&common.join("config.worktree"))?;
    match fs::symlink_metadata(common.join("info").join("attributes")) {
        Ok(meta)
            if !meta.is_file()
                || meta.len() != 0
                || meta.file_attributes() & REPARSE_POINT != 0 =>
        {
            return Err(WorktreeError::Denied)
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let attrs = git(
        db,
        root,
        custodian,
        pin,
        "source_attrs",
        Some(source),
        &[
            "ls-tree".into(),
            "-r".into(),
            "--name-only".into(),
            commit.into(),
        ],
        true,
    )?;
    if has_attribute_surface(&attrs) {
        return Err(WorktreeError::Denied);
    }
    Ok(())
}

/// Owner-only source registration. Every Git fact comes from the held program
/// operating offline against the actual source path. Existing sources are
/// immutable; a changed path/remote/baseline requires a separate future policy.
pub(crate) fn register_source(
    db: &mut VerifiedDatabaseConnection<'_>,
    root: &RootLock,
    owner: &OwnerIssuer,
    pin: &GitProgramPin,
    custodian: &mut ProcessCustodian,
    input: SourceRegistration<'_>,
) -> Result<()> {
    if !atom(input.repository_id) {
        return Err(WorktreeError::Invalid("repository_id"));
    }
    transaction(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        Ok(())
    })?;
    let source = fs::canonicalize(input.source_path)?;
    let source_id = inspect_root(&source)?.identity;
    // The merge target is the registered primary checkout, never another
    // linked worktree or a caller supplied directory inside one.
    let dot_git = fs::symlink_metadata(source.join(".git"))?;
    if !dot_git.is_dir() || dot_git.file_attributes() & REPARSE_POINT != 0 {
        return Err(WorktreeError::Denied);
    }
    let top = git(
        db,
        root,
        custodian,
        pin,
        "source_top",
        Some(&source),
        &["rev-parse".into(), "--show-toplevel".into()],
        true,
    )?;
    if fs::canonicalize(&top)? != source {
        return Err(WorktreeError::Denied);
    }
    let common = git(
        db,
        root,
        custodian,
        pin,
        "source_common",
        Some(&source),
        &[
            "rev-parse".into(),
            "--path-format=absolute".into(),
            "--git-common-dir".into(),
        ],
        true,
    )?;
    let common = fs::canonicalize(common)?;
    let common_id = inspect_root(&common)?.identity;
    let remote = git(
        db,
        root,
        custodian,
        pin,
        "source_remote",
        Some(&source),
        &[
            "config".into(),
            "--local".into(),
            "--get-all".into(),
            "remote.origin.url".into(),
        ],
        true,
    )?;
    let remote_kind = remote_kind(&remote).ok_or(WorktreeError::Denied)?;
    let baseline = git(
        db,
        root,
        custodian,
        pin,
        "source_head",
        Some(&source),
        &[
            "rev-parse".into(),
            "--verify".into(),
            "HEAD^{commit}".into(),
        ],
        true,
    )?;
    if !hex_commit(&baseline) {
        return Err(WorktreeError::Git("invalid baseline commit".into()));
    }
    source_checkout_has_no_external_drivers(db, root, custodian, pin, &source, &common, &baseline)?;
    transaction(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        let q = Statement::prepare(db.as_ptr(), "SELECT 1 FROM main.gogoke_v37_worktree_sources WHERE repository_id=?1 OR source_identity=?2")?;
        q.bind_text(1, input.repository_id)?;
        q.bind_text(2, &source_id.opaque())?;
        if q.step_row()? {
            return Err(WorktreeError::Conflict);
        }
        let q = Statement::prepare(db.as_ptr(), "INSERT INTO main.gogoke_v37_worktree_sources(repository_id,source_path,source_identity,common_path,common_identity,remote_kind,baseline_commit,git_digest,git_version,revision) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,1)")?;
        q.bind_text(1, input.repository_id)?;
        q.bind_text(2, &source.to_string_lossy())?;
        q.bind_text(3, &source_id.opaque())?;
        q.bind_text(4, &common.to_string_lossy())?;
        q.bind_text(5, &common_id.opaque())?;
        q.bind_text(6, remote_kind)?;
        q.bind_text(7, &baseline)?;
        q.bind_text(8, &pin.digest)?;
        q.bind_text(9, &pin.version)?;
        q.step_done()?;
        let program = Statement::prepare(db.as_ptr(), "INSERT INTO main.gogoke_v37_worktree_programs(repository_id,git_path) VALUES(?1,?2)")?;
        program.bind_text(1, input.repository_id)?;
        program.bind_text(2, &pin.path.to_string_lossy())?;
        program.step_done()?;
        Ok(())
    }).map_err(|error| { if uncertain(&error) {
        RootLock::poison_identity(&root.canonical_root().identity);
    } error })
}

#[derive(Debug)]
pub(crate) struct ResolvedBinding {
    pub(crate) worktree_id: String,
    pub(crate) revision: i64,
    pub(crate) path: PathBuf,
    pub(crate) identity: RootIdentity,
    pub(crate) common_identity: RootIdentity,
    pub(crate) pointer_hash: String,
    pub(crate) pointer_len: u64,
    pub(crate) pointer_identity: RootIdentity,
    pub(crate) baseline_commit: String,
    pub(crate) seat_incarnation: String,
    pub(crate) seat_generation: i64,
    pub(crate) instance_id: String,
    pub(crate) permission_tier: String,
    /// Keep the pointer file itself nonreplaceable through H launch and stop.
    _pointer_guard: File,
}

impl ResolvedBinding {
    /// Duplicate the already held file object into actual process custody;
    /// never reopen a caller-selected `.git` pathname.
    pub(crate) fn retained_pointer(&self) -> io::Result<std::sync::Arc<File>> {
        self._pointer_guard.try_clone().map(std::sync::Arc::new)
    }
}

pub(crate) struct CreateWorktree<'a> {
    pub(crate) request_id: &'a str,
    pub(crate) request_bytes: &'a [u8],
    pub(crate) repository_id: &'a str,
    /// The User-visible v37 target identity; never used as a filesystem name.
    pub(crate) target_id: &'a str,
    pub(crate) domain_id: &'a str,
    pub(crate) seat_id: &'a str,
}

fn random_id() -> Result<String> {
    #[link(name = "bcrypt")]
    extern "system" {
        fn BCryptGenRandom(a: *mut std::ffi::c_void, b: *mut u8, n: u32, f: u32) -> i32;
    }
    let mut bytes = [0u8; 24];
    if unsafe { BCryptGenRandom(std::ptr::null_mut(), bytes.as_mut_ptr(), 24, 2) } < 0 {
        return Err(WorktreeError::Unknown);
    }
    Ok(format!(
        "wt{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}

fn file_identity(file: &File) -> Result<RootIdentity> {
    #[repr(C)]
    struct FileIdInfo {
        volume_serial: u64,
        file_id: [u8; 16],
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandleEx(
            handle: *mut std::ffi::c_void,
            class: i32,
            information: *mut std::ffi::c_void,
            length: u32,
        ) -> i32;
    }
    let mut info = FileIdInfo {
        volume_serial: 0,
        file_id: [0; 16],
    };
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            18,
            (&mut info as *mut FileIdInfo).cast(),
            std::mem::size_of::<FileIdInfo>() as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(RootIdentity {
        volume_serial: info.volume_serial,
        file_id: info.file_id,
    })
}

fn pointer(path: &Path, common: &Path) -> Result<(String, u64, RootIdentity, File)> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_attributes() & REPARSE_POINT != 0 || meta.len() > 4096 {
        return Err(WorktreeError::Denied);
    }
    let mut file = OpenOptions::new().read(true).share_mode(1).open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    if !bytes.starts_with(b"gitdir: ") || bytes.contains(&0) {
        return Err(WorktreeError::Denied);
    }
    let pointed = std::str::from_utf8(&bytes[8..])
        .map_err(|error| WorktreeError::Utf8("Git pointer", error))?;
    let pointed = pointed.trim_end_matches(['\r', '\n']);
    let gitdir = fs::canonicalize(pointed)?;
    let expected_parent = fs::canonicalize(common.join("worktrees"))?;
    if gitdir.parent() != Some(expected_parent.as_path())
        || fs::symlink_metadata(pointed)?.file_attributes() & REPARSE_POINT != 0
    {
        return Err(WorktreeError::Denied);
    }
    let identity = file_identity(&file)?;
    Ok((content_hash(&bytes), bytes.len() as u64, identity, file))
}

fn ensure_worktree_parent(root: &RootLock, mixed_path_id: Option<&str>) -> Result<PathBuf> {
    let mut parent = root.canonical_root().canonical_path.clone();
    let mut components = vec!["v37-worktrees", if mixed_path_id.is_some() { "mixed" } else { "single" }];
    if let Some(path_id) = mixed_path_id { components.push(path_id); }
    for component in components {
        parent = parent.join(component);
        match fs::create_dir(&parent) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let meta = fs::symlink_metadata(&parent)?;
        if !meta.is_dir()
            || meta.file_attributes() & REPARSE_POINT != 0
            || fs::canonicalize(&parent)? != parent
        {
            return Err(WorktreeError::Denied);
        }
        inspect_root(&parent)?;
    }
    Ok(parent)
}

/// Original request intent is committed before mkdir or Git. Any interrupted
/// intent fences every new request for this source. Replays read back only a
/// REGISTERED row; neither replay nor recovery reissues worktree add.
pub(crate) fn create_worktree(
    db: &mut VerifiedDatabaseConnection<'_>,
    root: &RootLock,
    owner: &OwnerIssuer,
    pin: &GitProgramPin,
    custodian: &mut ProcessCustodian,
    input: CreateWorktree<'_>,
) -> Result<ResolvedBinding> {
    let mut authorize = |_: &VerifiedDatabaseConnection<'_>| -> Result<()> { Ok(()) };
    create_worktree_in_space(db, root, owner, pin, custodian, None, None, false, input,
        &mut authorize)
}

/// M2's separate create/register contract uses this entry. The original M1
/// create entry above remains immediately registered for existing requests.
pub(crate) fn create_m2_single_worktree(
    db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock, owner: &OwnerIssuer,
    pin: &GitProgramPin, custodian: &mut ProcessCustodian, input: CreateWorktree<'_>,
) -> Result<ResolvedBinding> {
    let mut authorize = |_: &VerifiedDatabaseConnection<'_>| -> Result<()> { Ok(()) };
    create_worktree_in_space(db, root, owner, pin, custodian, None, None, true, input,
        &mut authorize)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeWorktreeLayout { Single, Mixed }

fn native_child_worktree_identity(
    child: &crate::store::seat::Seat,
    request: &crate::store::session_transport::V37Request,
) -> Result<(String, NativeWorktreeLayout, String, String)> {
    if request.family != "K-WORKTREE" || request.operation != "create"
        || request.expected_revision != 0 || request.payload.len() != 2
        || request.domain_id != child.domain_id || request.target_id != child.seat_id
        || !atom(&request.request_id)
        || request.raw_bytes.is_empty() || request.raw_bytes.len() > 65_536 {
        return Err(WorktreeError::Invalid("native child create"));
    }
    let repository_id = match request.payload.get(&JsonString::from_str("repositoryId")) {
        Some(Json::String(value)) => value.to_well_formed_string()
            .ok_or(WorktreeError::Invalid("native repository"))?,
        _ => return Err(WorktreeError::Invalid("native repository")),
    };
    if !atom(&repository_id) { return Err(WorktreeError::Invalid("native repository")); }
    let layout = match request.payload.get(&JsonString::from_str("layout")) {
        Some(Json::String(value)) if value.to_well_formed_string().as_deref() == Some("SINGLE") =>
            NativeWorktreeLayout::Single,
        Some(Json::String(value)) if value.to_well_formed_string().as_deref() == Some("MIXED") =>
            NativeWorktreeLayout::Mixed,
        _ => return Err(WorktreeError::Invalid("native layout")),
    };
    let identity = format!("{}\0{}\0{}\0{}\0{}", request.domain_id,
        request.request_id, repository_id, child.seat_id,
        sha256_hex(&request.raw_bytes));
    let id = sha256_hex(identity.as_bytes());
    Ok((repository_id, layout, format!("native-wt-{}", &id[..40]),
        format!("native-register-{}", &id[..40])))
}

/// A sealed H/A tool call supplies only its original bytes and the selected
/// existing repository and direct child seat. F derives the logical ID and
/// every physical path/space/branch. The same request may finish REGISTER
/// after a completed CREATE without reissuing `git worktree add`.
pub(crate) fn create_and_register_native_child_worktree(
    db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock, owner: &OwnerIssuer,
    pin: &GitProgramPin, custodian: &mut ProcessCustodian,
    caller: &crate::store::seat::NativeSeatCall,
    child: &crate::store::seat::Seat,
    request: &crate::store::session_transport::V37Request,
) -> Result<ResolvedBinding> {
    if child.state == crate::store::seat::State::Busy {
        return recover_registered_native_child_worktree(db, root, caller, child, request);
    }
    let (repository_id, layout, worktree_id, register_id) =
        native_child_worktree_identity(child, request)?;
    let input = CreateWorktree {
        request_id: &request.request_id, request_bytes: &request.raw_bytes,
        repository_id: &repository_id, target_id: &worktree_id, domain_id: &request.domain_id,
        seat_id: &child.seat_id,
    };
    if child.state != crate::store::seat::State::Idle { return Err(WorktreeError::Denied); }
    let mut authorize = |db: &VerifiedDatabaseConnection<'_>| {
        crate::store::seat::authorize_child_dispatch(db, caller, child)
            .map_err(WorktreeError::Seat)
    };
    authorize(db)?;
    let history = readback_create_receipt(db, &request.request_id, &request.raw_bytes,
        &worktree_id, &repository_id, &request.domain_id, &child.seat_id)?;
    if history.as_ref().is_some_and(|history| history.classification != match layout {
        NativeWorktreeLayout::Single => "SINGLE",
        NativeWorktreeLayout::Mixed => "MIXED",
    }) { return Err(WorktreeError::Conflict); }
    let binding = if history.is_some() {
        resolve_id(db, root, &worktree_id)?
    } else {
        let (space_id, incarnation) = match layout {
            NativeWorktreeLayout::Single => (None, None),
            NativeWorktreeLayout::Mixed => (Some(mixed_seat_space_id(&request.domain_id,
                &child.seat_id, &child.incarnation)?), Some(child.incarnation.as_str())),
        };
        create_worktree_in_space(db, root, owner, pin, custodian, space_id.as_deref(),
            incarnation, true, input, &mut authorize)?
    };
    let register = crate::store::session_transport::V37Request {
        raw_bytes: request.raw_bytes.clone(), family: "K-WORKTREE".into(),
        operation: "register".into(), request_id: register_id,
        target_id: worktree_id, domain_id: request.domain_id.clone(),
        expected_revision: 1, payload: Default::default(),
    };
    register_created_worktree_request(db, root, owner, &register, &mut authorize)?;
    Ok(binding)
}

/// Recover only a completed native child CREATE/REGISTER after the original H
/// reservation advanced that exact child to BUSY. This entry has no Git pin,
/// ProcessCustodian, create, register, or write path.
pub(crate) fn recover_registered_native_child_worktree(
    db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock,
    caller: &crate::store::seat::NativeSeatCall,
    child: &crate::store::seat::Seat,
    request: &crate::store::session_transport::V37Request,
) -> Result<ResolvedBinding> {
    let (repository_id, layout, worktree_id, register_id) =
        native_child_worktree_identity(child, request)?;
    if child.state != crate::store::seat::State::Busy { return Err(WorktreeError::Denied); }
    transaction(db, |db| {
        let host_id = caller.host_request_id().ok_or(WorktreeError::Denied)?;
        if !atom(host_id) || request.request_id != format!("{host_id}-worktree") {
            return Err(WorktreeError::Denied);
        }
        let current = crate::store::seat::current_child_dispatch_context(db, caller, child)?;
        if current != *child { return Err(WorktreeError::Denied); }
        let reservation = Statement::prepare(db.as_ptr(),
            "SELECT session_id FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND request_id=?2 AND operation='admission-reserve' AND status='APPLIED'")?;
        reservation.bind_text(1, &request.domain_id)?;
        reservation.bind_text(2, host_id)?;
        if !reservation.step_row()? { return Err(WorktreeError::Denied); }
        let session = reservation.column_text(0)?;
        if !atom(&session) || reservation.step_row()? { return Err(WorktreeError::Denied); }
        drop(reservation);
        let admission = crate::store::seat::NativeLeadAdmission::from_model_call(caller)?;
        let origin = crate::store::seat::NativeOrigin::lead(&admission);
        let claim = crate::store::session_transport::runtime::observe_claim(db, &origin,
            &request.domain_id, &child.seat_id, &session)?
            .ok_or(WorktreeError::Denied)?;
        if claim.instance_id != child.instance_id
            || claim.generation != child.generation.to_string()
            || claim.home_id.is_empty() || claim.binding_id.is_empty() {
            return Err(WorktreeError::Denied);
        }
        let history = readback_create_receipt(db, &request.request_id,
            &request.raw_bytes, &worktree_id, &repository_id,
            &request.domain_id, &child.seat_id)?.ok_or(WorktreeError::Denied)?;
        if history.classification != match layout {
            NativeWorktreeLayout::Single => "SINGLE",
            NativeWorktreeLayout::Mixed => "MIXED",
        } { return Err(WorktreeError::Conflict); }
        let binding = resolve_id(db, root, &worktree_id)?;
        if binding.seat_incarnation != child.incarnation
            || binding.seat_generation.checked_add(1) != Some(child.generation)
            || binding.instance_id != child.instance_id {
            return Err(WorktreeError::Denied);
        }
        let state = Statement::prepare(db.as_ptr(),
            "SELECT state,revision FROM main.gogoke_v37_worktree_lifecycle WHERE worktree_id=?1")?;
        state.bind_text(1, &worktree_id)?;
        if !state.step_row()? || state.column_text(0)? != "REGISTERED"
            || state.column_text(1)? != "2" || state.step_row()? {
            return Err(WorktreeError::Denied);
        }
        let register = Statement::prepare(db.as_ptr(),
            "SELECT request_hash,worktree_id,operation,phase FROM main.gogoke_v37_worktree_lifecycle_ops WHERE request_id=?1")?;
        register.bind_text(1, &register_id)?;
        if !register.step_row()? || register.column_text(0)? != sha256_hex(&request.raw_bytes)
            || register.column_text(1)? != worktree_id || register.column_text(2)? != "REGISTER"
            || register.column_text(3)? != "APPLIED" || register.step_row()? {
            return Err(WorktreeError::Denied);
        }
        Ok(binding)
    })
}

/// A mixed space has one host-generated physical parent and separate linked
/// trees for every repository edge of one native seat incarnation. No wire
/// field chooses its space, directory, or Git branch.
pub(crate) fn create_mixed_worktree(
    db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock, owner: &OwnerIssuer,
    pin: &GitProgramPin, custodian: &mut ProcessCustodian, input: CreateWorktree<'_>,
) -> Result<ResolvedBinding> {
    let seat = crate::store::seat::get(db, input.domain_id, input.seat_id)?
        .ok_or(WorktreeError::Denied)?;
    if seat.state == crate::store::seat::State::Reclaimed || seat.instance_id.is_empty()
        || !atom(&seat.incarnation) { return Err(WorktreeError::Denied); }
    let space_id = mixed_seat_space_id(input.domain_id, input.seat_id, &seat.incarnation)?;
    let mut authorize = |_: &VerifiedDatabaseConnection<'_>| -> Result<()> { Ok(()) };
    create_worktree_in_space(db, root, owner, pin, custodian, Some(&space_id),
        Some(&seat.incarnation), true, input, &mut authorize)
}

fn mixed_seat_space_id(domain: &str, seat: &str, incarnation: &str) -> Result<String> {
    if !atom(domain) || !atom(seat) || !atom(incarnation) {
        return Err(WorktreeError::Denied);
    }
    let identity = format!("gogoke.37.mixed-seat.v1\0{domain}\0{seat}\0{incarnation}");
    Ok(format!("space-{}", &sha256_hex(identity.as_bytes())[..40]))
}

fn verify_mixed_members(db: &VerifiedDatabaseConnection<'_>, space_id: &str,
    domain: &str, seat: &str, incarnation: &str) -> Result<()> {
    let members = Statement::prepare(db.as_ptr(),
        "SELECT m.domain_id,m.seat_id,COALESCE(w.domain_id,''),COALESCE(w.seat_id,''),COALESCE(w.seat_incarnation,''),COALESCE(o.domain_id,''),COALESCE(o.seat_id,''),COALESCE(o.seat_incarnation,''),COALESCE(o.phase,''),m.repository_id,COALESCE(w.repository_id,''),COALESCE(o.repository_id,''),COALESCE(w.state,'') FROM main.gogoke_v37_worktree_members m LEFT JOIN main.gogoke_v37_worktrees w ON w.worktree_id=m.worktree_id LEFT JOIN main.gogoke_v37_worktree_operations o ON o.worktree_id=m.worktree_id WHERE m.space_id=?1")?;
    members.bind_text(1, space_id)?;
    while members.step_row()? {
        for index in [0, 2, 5] {
            if members.column_text(index)? != domain { return Err(WorktreeError::Denied); }
        }
        for index in [1, 3, 6] {
            if members.column_text(index)? != seat { return Err(WorktreeError::Denied); }
        }
        for index in [4, 7] {
            if members.column_text(index)? != incarnation { return Err(WorktreeError::Denied); }
        }
        let repository = members.column_text(9)?;
        if !atom(&repository) || members.column_text(10)? != repository
            || members.column_text(11)? != repository
            || members.column_text(12)? != "REGISTERED" {
            return Err(WorktreeError::Denied);
        }
        if members.column_text(8)? != "REGISTERED" { return Err(WorktreeError::Unknown); }
    }
    Ok(())
}

fn create_worktree_in_space(
    db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock, owner: &OwnerIssuer,
    pin: &GitProgramPin, custodian: &mut ProcessCustodian, space_id: Option<&str>,
    expected_incarnation: Option<&str>,
    requires_registration: bool,
    input: CreateWorktree<'_>,
    authorize: &mut impl FnMut(&VerifiedDatabaseConnection<'_>) -> Result<()>,
) -> Result<ResolvedBinding> {
    if ![
        input.request_id,
        input.repository_id,
        input.target_id,
        input.domain_id,
        input.seat_id,
    ]
    .iter()
    .all(|s| atom(s))
        || input.request_bytes.is_empty()
        || input.request_bytes.len() > 65_536
    {
        return Err(WorktreeError::Invalid("create request"));
    }
    let fingerprint = sha256_hex(input.request_bytes);
    let path_id = random_id()?;
    let proposed_space_path = random_id()?;
    let (source, source_identity, common, common_identity, baseline, seat_snapshot, mixed_path) = transaction(
        db,
        |db| {
            check_owner_in_current_transaction(db, owner)?;
            authorize(db)?;
            let q = Statement::prepare(db.as_ptr(), "SELECT source_path,source_identity,common_path,common_identity,baseline_commit,git_digest,git_version FROM main.gogoke_v37_worktree_sources WHERE repository_id=?1")?;
            q.bind_text(1, input.repository_id)?;
            if !q.step_row()? {
                return Err(WorktreeError::Denied);
            }
            let result = (
                q.column_text(0)?,
                q.column_text(1)?,
                q.column_text(2)?,
                q.column_text(3)?,
                q.column_text(4)?,
            );
            if q.column_text(5)? != pin.digest || q.column_text(6)? != pin.version {
                return Err(WorktreeError::Denied);
            }
            let pending = Statement::prepare(db.as_ptr(), "SELECT 1 FROM main.gogoke_v37_worktree_operations WHERE repository_id=?1 AND phase<>'REGISTERED' LIMIT 1")?;
            pending.bind_text(1, input.repository_id)?;
            if pending.step_row()? {
                return Err(WorktreeError::Unknown);
            }
            let old = Statement::prepare(db.as_ptr(), "SELECT request_hash,phase,worktree_id FROM main.gogoke_v37_worktree_operations WHERE request_id=?1")?;
            old.bind_text(1, input.request_id)?;
            if old.step_row()? {
                if old.column_text(0)? != fingerprint {
                    return Err(WorktreeError::Conflict);
                }
                return Err(WorktreeError::Conflict); // readback is explicit; never re-create.
            }
            let target_used=Statement::prepare(db.as_ptr(),"SELECT 1 FROM main.gogoke_v37_worktree_operations WHERE worktree_id=?1")?;
            target_used.bind_text(1,input.target_id)?;
            if target_used.step_row()? { return Err(WorktreeError::Conflict); }
            let seat = Statement::prepare(db.as_ptr(), "SELECT incarnation,generation,revision,COALESCE(instance_id,''),state FROM main.gogoke_v37_seats WHERE domain_id=?1 AND seat_id=?2")?;
            seat.bind_text(1, input.domain_id)?;
            seat.bind_text(2, input.seat_id)?;
            if !seat.step_row()?
                || seat.column_text(4)? == "RECLAIMED"
                || seat.column_text(3)?.is_empty()
            {
                return Err(WorktreeError::Denied);
            }
            let snapshot = (
                seat.column_text(0)?,
                parse_i64(&seat.column_text(1)?,"intent seat generation")?,
                parse_i64(&seat.column_text(2)?,"intent seat revision")?,
                seat.column_text(3)?,
                format!(
                    "{:?}",
                    crate::store::seat::permission_tier(
                        &crate::store::seat::get(db, input.domain_id, input.seat_id)?
                            .ok_or(WorktreeError::Denied)?
                    )
                    ?
                ),
            );
            if expected_incarnation.is_some_and(|expected| expected != snapshot.0.as_str())
                || (space_id.is_some() != expected_incarnation.is_some()) {
                return Err(WorktreeError::Denied);
            }
            // The full typed seat and tier are checked again at final registration.
            let insert = Statement::prepare(db.as_ptr(), "INSERT INTO main.gogoke_v37_worktree_operations(request_id,request_hash,repository_id,domain_id,seat_id,worktree_id,path_id,seat_incarnation,seat_generation,seat_revision,instance_id,permission_tier,phase) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'INTENT')")?;
            insert.bind_text(1, input.request_id)?;
            insert.bind_text(2, &fingerprint)?;
            insert.bind_text(3, input.repository_id)?;
            insert.bind_text(4, input.domain_id)?;
            insert.bind_text(5, input.seat_id)?;
            insert.bind_text(6, input.target_id)?;
            insert.bind_text(7, &path_id)?;
            insert.bind_text(8, &snapshot.0)?;
            insert.bind_i64(9, snapshot.1)?;
            insert.bind_i64(10, snapshot.2)?;
            insert.bind_text(11, &snapshot.3)?;
            insert.bind_text(12, &snapshot.4)?;
            insert.step_done()?;
            let mixed_path = if let Some(space_id) = space_id {
                let space = Statement::prepare(db.as_ptr(),
                    "SELECT path_id,classification,state FROM main.gogoke_v37_worktree_spaces WHERE space_id=?1")?;
                space.bind_text(1, space_id)?;
                if space.step_row()? {
                    let path = space.column_text(0)?;
                    if space.column_text(1)? != "MIXED" || space.column_text(2)? != "ACTIVE" ||
                        space.step_row()? { return Err(WorktreeError::Denied); }
                    verify_mixed_members(db, space_id, input.domain_id, input.seat_id,
                        &snapshot.0)?;
                    Some(path)
                } else {
                    let add = Statement::prepare(db.as_ptr(),
                        "INSERT INTO main.gogoke_v37_worktree_spaces(space_id,path_id,classification,state,revision) VALUES(?1,?2,'MIXED','ACTIVE',1)")?;
                    add.bind_text(1, space_id)?;
                    add.bind_text(2, &proposed_space_path)?;
                    add.step_done()?;
                    Some(proposed_space_path.clone())
                }
            } else { None };
            Ok((result.0, result.1, result.2, result.3, result.4, snapshot, mixed_path))
        },
    ).map_err(|error| { if uncertain(&error) {
        RootLock::poison_identity(&root.canonical_root().identity);
    } error })?;
    let source = PathBuf::from(source);
    let common = PathBuf::from(common);
    let target = match &mixed_path {
        Some(space_path) => root.canonical_root().canonical_path.join("v37-worktrees")
            .join("mixed").join(space_path).join(&path_id),
        None => root.canonical_root().canonical_path.join("v37-worktrees")
            .join("single").join(&path_id),
    };
    let effect = (|| -> Result<ResolvedBinding> {
        if inspect_root(&source)?.identity.opaque() != source_identity
            || inspect_root(&common)?.identity.opaque() != common_identity
        {
            return Err(WorktreeError::Denied);
        }
        source_checkout_has_no_external_drivers(
            db, root, custodian, pin, &source, &common, &baseline,
        )?;
        ensure_worktree_parent(root, mixed_path.as_deref())?;
        match require_absent(&target) {
            Ok(()) => {}
            Err(WorktreeError::Denied) => return Err(WorktreeError::Unknown),
            Err(error) => return Err(error),
        }
        git(
            db,
            root,
            custodian,
            pin,
            "worktree_add",
            Some(&source),
            &[
                "worktree".into(),
                "add".into(),
                "--quiet".into(),
                "--detach".into(),
                // The path remains native-generated and physically checked;
                // Git's mingw path parser requires the local DOS spelling.
                git_launch_path(&target)?.to_string_lossy().into_owned(),
                baseline.clone(),
            ],
            false,
        )?;
        let identity = inspect_root(&target)?.identity;
        let (pointer_hash, pointer_len, pointer_identity, pointer_guard) =
            pointer(&target.join(".git"), &common)?;
        let common_actual = git(
            db,
            root,
            custodian,
            pin,
            "worktree_common",
            Some(&target),
            &[
                "rev-parse".into(),
                "--path-format=absolute".into(),
                "--git-common-dir".into(),
            ],
            true,
        )?;
        if inspect_root(&fs::canonicalize(common_actual)?)?
            .identity
            .opaque()
            != common_identity
        {
            return Err(WorktreeError::Denied);
        }
        let head = git(
            db,
            root,
            custodian,
            pin,
            "worktree_head",
            Some(&target),
            &[
                "rev-parse".into(),
                "--verify".into(),
                "HEAD^{commit}".into(),
            ],
            true,
        )?;
        if head != baseline {
            return Err(WorktreeError::Denied);
        }
        transaction(db, |db| {
            check_owner_in_current_transaction(db, owner)?;
            authorize(db)?;
            let op = Statement::prepare(db.as_ptr(),"SELECT phase,request_hash FROM main.gogoke_v37_worktree_operations WHERE request_id=?1 AND worktree_id=?2 AND path_id=?3")?;
            op.bind_text(1, input.request_id)?;
            op.bind_text(2, input.target_id)?;
            op.bind_text(3, &path_id)?;
            if !op.step_row()?
                || op.column_text(0)? != "INTENT"
                || op.column_text(1)? != fingerprint
            {
                return Err(WorktreeError::Unknown);
            }
            let seat = Statement::prepare(db.as_ptr(), "SELECT s.incarnation,s.generation,s.revision,COALESCE(s.instance_id,''),s.state,COALESCE(t.settings_json,'') FROM main.gogoke_v37_seats s LEFT JOIN main.gogoke_v37_seat_settings t USING(domain_id,seat_id) WHERE s.domain_id=?1 AND s.seat_id=?2")?;
            seat.bind_text(1, input.domain_id)?;
            seat.bind_text(2, input.seat_id)?;
            if !seat.step_row()? || seat.column_text(4)? == "RECLAIMED" {
                return Err(WorktreeError::Denied);
            }
            let incarnation = seat.column_text(0)?;
            let generation = parse_i64(&seat.column_text(1)?, "final seat generation")?;
            let revision = parse_i64(&seat.column_text(2)?, "final seat revision")?;
            let instance = seat.column_text(3)?;
            let settings = seat.column_text(5)?;
            let tier = crate::store::seat::permission_tier(
                &crate::store::seat::get(db, input.domain_id, input.seat_id)?
                    .ok_or(WorktreeError::Denied)?,
            )?;
            let tier = format!("{tier:?}");
            if instance.is_empty() || settings.is_empty() {
                return Err(WorktreeError::Denied);
            }
            if (
                incarnation.as_str(),
                generation,
                revision,
                instance.as_str(),
                tier.as_str(),
            ) != (
                seat_snapshot.0.as_str(),
                seat_snapshot.1,
                seat_snapshot.2,
                seat_snapshot.3.as_str(),
                seat_snapshot.4.as_str(),
            ) {
                return Err(WorktreeError::Denied);
            }
            let q=Statement::prepare(db.as_ptr(),"INSERT INTO main.gogoke_v37_worktrees(worktree_id,path_id,repository_id,domain_id,seat_id,seat_incarnation,seat_generation,seat_revision,permission_tier,instance_id,source_revision,worktree_path,worktree_identity,git_pointer_hash,git_pointer_len,git_pointer_identity,common_identity,baseline_commit,state,revision) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,1,?11,?12,?13,?14,?15,?16,?17,'REGISTERED',1)")?;
            q.bind_text(1, input.target_id)?;
            q.bind_text(2, &path_id)?;
            q.bind_text(3, input.repository_id)?;
            q.bind_text(4, input.domain_id)?;
            q.bind_text(5, input.seat_id)?;
            q.bind_text(6, &incarnation)?;
            q.bind_i64(7, generation)?;
            q.bind_i64(8, revision)?;
            q.bind_text(9, &tier)?;
            q.bind_text(10, &instance)?;
            q.bind_text(11, &target.to_string_lossy())?;
            q.bind_text(12, &identity.opaque())?;
            q.bind_text(13, &pointer_hash)?;
            q.bind_i64(14, pointer_len as i64)?;
            q.bind_text(15, &pointer_identity.opaque())?;
            q.bind_text(16, &common_identity)?;
            q.bind_text(17, &baseline)?;
            q.step_done()?;
            if let Some(space_id) = space_id {
                let member = Statement::prepare(db.as_ptr(),
                    "INSERT INTO main.gogoke_v37_worktree_members(worktree_id,space_id,repository_id,domain_id,seat_id) VALUES(?1,?2,?3,?4,?5)")?;
                member.bind_text(1, input.target_id)?; member.bind_text(2, space_id)?;
                member.bind_text(3, input.repository_id)?; member.bind_text(4, input.domain_id)?;
                member.bind_text(5, input.seat_id)?; member.step_done()?;
            }
            if requires_registration {
                let lifecycle = Statement::prepare(db.as_ptr(),
                    "INSERT INTO main.gogoke_v37_worktree_lifecycle(worktree_id,state,revision) VALUES(?1,'CREATED',1)")?;
                lifecycle.bind_text(1, input.target_id)?; lifecycle.step_done()?;
            }
            let update=Statement::prepare(db.as_ptr(),"UPDATE main.gogoke_v37_worktree_operations SET phase='REGISTERED' WHERE request_id=?1 AND phase='INTENT'")?;
            update.bind_text(1, input.request_id)?;
            update.step_done()?;
            Ok(ResolvedBinding {
                worktree_id: input.target_id.to_owned(),
                revision: 1,
                path: target.clone(),
                identity: identity.clone(),
                common_identity: inspect_root(&common)?.identity,
                pointer_hash: pointer_hash.clone(),
                pointer_len,
                pointer_identity,
                baseline_commit: baseline.clone(),
                seat_incarnation: incarnation,
                seat_generation: generation,
                instance_id: instance,
                permission_tier: tier,
                _pointer_guard: pointer_guard,
            })
        })
    })();
    match effect {
        Ok(binding) => Ok(binding),
        Err(error) => {
            let cause = format!("{error:?}");
            let persisted = transaction(db, |db| {
                check_owner_in_current_transaction(db, owner)?;
                let q=Statement::prepare(db.as_ptr(),"UPDATE main.gogoke_v37_worktree_operations SET phase='UNKNOWN',cause=?1 WHERE request_id=?2 AND phase='INTENT'")?;
                q.bind_text(1, &cause)?;
                q.bind_text(2, input.request_id)?;
                q.step_done()?;
                let changed = Statement::prepare(db.as_ptr(), "SELECT changes()")?;
                if !changed.step_row()? || changed.column_text(0)? != "1" {
                    return Err(WorktreeError::Unknown);
                }
                if let Some(space_id) = space_id {
                    let space = Statement::prepare(db.as_ptr(),
                        "UPDATE main.gogoke_v37_worktree_spaces SET state='UNKNOWN' WHERE space_id=?1 AND state='ACTIVE'")?;
                    space.bind_text(1, space_id)?; space.step_done()?;
                }
                Ok(())
            });
            match persisted {
                Ok(()) => Err(error),
                Err(persist) => {
                    RootLock::poison_identity(&root.canonical_root().identity);
                    Err(joined("worktree UNKNOWN persistence", error, persist))
                }
            }
        }
    }
}

/// Read the original request outcome. INTENT/UNKNOWN is never permission to
/// retry with a new ID, even when the directory is absent after a restart.
pub(crate) fn readback_create(
    db: &VerifiedDatabaseConnection<'_>,
    root: &RootLock,
    request_id: &str,
    request_bytes: &[u8],
) -> Result<Option<ResolvedBinding>> {
    if !atom(request_id) || request_bytes.is_empty() || request_bytes.len() > 65_536 {
        return Err(WorktreeError::Invalid("request"));
    }
    let q = Statement::prepare(db.as_ptr(), "SELECT request_hash,phase,worktree_id FROM main.gogoke_v37_worktree_operations WHERE request_id=?1")?;
    q.bind_text(1, request_id)?;
    if !q.step_row()? {
        return Ok(None);
    }
    if q.column_text(0)? != sha256_hex(request_bytes) {
        return Err(WorktreeError::Conflict);
    }
    if q.column_text(1)? != "REGISTERED" {
        return Err(WorktreeError::Unknown);
    }
    resolve_id(db, root, &q.column_text(2)?).map(Some)
}

/// H uses the registered identity and current seat incarnation/generation;
/// neither Node nor a model may provide a path. Retain this object through
/// process activation and stop so the `.git` pointer cannot be replaced.
pub(crate) fn resolve_for_launch(
    db: &VerifiedDatabaseConnection<'_>,
    root: &RootLock,
    worktree_id: &str,
    repository_id: &str,
    domain_id: &str,
    seat_id: &str,
    expected_incarnation: &str,
    expected_generation: i64,
) -> Result<ResolvedBinding> {
    f2::launch_allowed(db, worktree_id)?;
    if ![
        worktree_id,
        repository_id,
        domain_id,
        seat_id,
        expected_incarnation,
    ]
    .iter()
    .all(|s| atom(s))
        || expected_generation < 1
    {
        return Err(WorktreeError::Invalid("binding"));
    }
    let q = Statement::prepare(db.as_ptr(), "SELECT 1 FROM main.gogoke_v37_worktrees WHERE worktree_id=?1 AND repository_id=?2 AND domain_id=?3 AND seat_id=?4 AND seat_incarnation=?5 AND state='REGISTERED' AND revision=1")?;
    q.bind_text(1, worktree_id)?;
    q.bind_text(2, repository_id)?;
    q.bind_text(3, domain_id)?;
    q.bind_text(4, seat_id)?;
    q.bind_text(5, expected_incarnation)?;
    if !q.step_row()? {
        return Err(WorktreeError::Denied);
    }
    let seat = crate::store::seat::get(db, domain_id, seat_id)?.ok_or(WorktreeError::Denied)?;
    if seat.incarnation != expected_incarnation
        || seat.generation != expected_generation
        || seat.state == crate::store::seat::State::Reclaimed
    {
        return Err(WorktreeError::Denied);
    }
    resolve_id(db, root, worktree_id)
}

fn resolve_id(
    db: &VerifiedDatabaseConnection<'_>,
    root: &RootLock,
    id: &str,
) -> Result<ResolvedBinding> {
    let q=Statement::prepare(db.as_ptr(), "SELECT w.repository_id,w.domain_id,w.seat_id,w.seat_incarnation,w.seat_generation,w.seat_revision,w.permission_tier,w.instance_id,w.worktree_path,w.worktree_identity,w.git_pointer_hash,w.git_pointer_len,w.git_pointer_identity,w.common_identity,w.baseline_commit,s.source_path,s.source_identity,s.common_path,s.revision,w.revision,w.path_id FROM main.gogoke_v37_worktrees w JOIN main.gogoke_v37_worktree_sources s ON s.repository_id=w.repository_id WHERE w.worktree_id=?1 AND w.state='REGISTERED'")?;
    q.bind_text(1, id)?;
    if !q.step_row()? {
        return Err(WorktreeError::Denied);
    }
    let repository = q.column_text(0)?;
    let domain = q.column_text(1)?;
    let seat_id = q.column_text(2)?;
    let incarnation = q.column_text(3)?;
    let generation = parse_i64(&q.column_text(4)?, "resolved seat generation")?;
    let _seat_revision = parse_i64(&q.column_text(5)?, "resolved seat revision")?;
    let tier = q.column_text(6)?;
    let instance = q.column_text(7)?;
    let path = PathBuf::from(q.column_text(8)?);
    let identity_text = q.column_text(9)?;
    let pointer_hash = q.column_text(10)?;
    let pointer_len = parse_u64(&q.column_text(11)?, "pointer length")?;
    let pointer_identity_text = q.column_text(12)?;
    let common_identity_text = q.column_text(13)?;
    let baseline = q.column_text(14)?;
    let source = PathBuf::from(q.column_text(15)?);
    let source_identity = q.column_text(16)?;
    let common = PathBuf::from(q.column_text(17)?);
    let source_revision = q.column_text(18)?;
    let revision = parse_i64(&q.column_text(19)?, "worktree revision")?;
    let path_id = q.column_text(20)?;
    if !atom(&path_id) || !path_id.starts_with("wt") {
        return Err(WorktreeError::SchemaDrift);
    }
    if source_revision != "1"
        || revision != 1
        || !hex_commit(&baseline)
        || inspect_root(&source)?.identity.opaque() != source_identity
    {
        return Err(WorktreeError::Denied);
    }
    let member = Statement::prepare(db.as_ptr(),
        "SELECT s.path_id,s.classification,s.state FROM main.gogoke_v37_worktree_members m JOIN main.gogoke_v37_worktree_spaces s ON s.space_id=m.space_id WHERE m.worktree_id=?1")?;
    member.bind_text(1, id)?;
    let exact = if member.step_row()? {
        let space_path = member.column_text(0)?;
        if !atom(&space_path) || member.column_text(1)? != "MIXED" ||
            member.column_text(2)? != "ACTIVE" || member.step_row()? { return Err(WorktreeError::Denied); }
        root.canonical_root().canonical_path.join("v37-worktrees")
            .join("mixed").join(space_path).join(&path_id)
    } else {
        root.canonical_root().canonical_path.join("v37-worktrees")
            .join("single").join(&path_id)
    };
    if path != exact || fs::canonicalize(&path)? != path {
        return Err(WorktreeError::Denied);
    }
    let identity = inspect_root(&path)?.identity;
    if identity.opaque() != identity_text {
        return Err(WorktreeError::Denied);
    }
    let common_identity = inspect_root(&common)?.identity;
    if common_identity.opaque() != common_identity_text {
        return Err(WorktreeError::Denied);
    }
    let source_git = source.join(".git");
    let source_git_meta = fs::symlink_metadata(&source_git)?;
    if !source_git_meta.is_dir() || source_git_meta.file_attributes() & REPARSE_POINT != 0 ||
        fs::canonicalize(&source_git)? != common {
        return Err(WorktreeError::Denied);
    }
    let (actual_hash, actual_len, pointer_identity, pointer_guard) =
        pointer(&path.join(".git"), &common)?;
    if actual_hash != pointer_hash
        || actual_len != pointer_len
        || pointer_identity.opaque() != pointer_identity_text
    {
        return Err(WorktreeError::Denied);
    }
    // Creation fields are historical provenance. H's launch transaction binds
    // the current seat revision, generation, instance and permission tier.
    // A stale source path or another repository's worktree is never adopted.
    let count=Statement::prepare(db.as_ptr(),"SELECT 1 FROM main.gogoke_v37_worktree_sources WHERE repository_id=?1 AND source_identity=?2")?;
    count.bind_text(1, &repository)?;
    count.bind_text(2, &source_identity)?;
    if !count.step_row()? {
        return Err(WorktreeError::Denied);
    }
    Ok(ResolvedBinding {
        worktree_id: id.to_owned(),
        revision,
        path,
        identity,
        common_identity,
        pointer_hash,
        pointer_len,
        pointer_identity,
        baseline_commit: baseline,
        seat_incarnation: incarnation,
        seat_generation: generation,
        instance_id: instance,
        permission_tier: tier,
        _pointer_guard: pointer_guard,
    })
}
