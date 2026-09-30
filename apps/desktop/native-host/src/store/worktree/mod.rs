//! Native F source and linked-worktree authority. No caller supplied cwd or
//! Git result is accepted as a binding. Only the User/Owner configuration
//! route may call these crate-private entry points.

use crate::process::{
    DurableStopConfirmation, NativeBinding, PrepareRequest, ProcessCustodian, ProcessLaunch,
    StopBudgets,
};
use crate::root::{inspect_root, RootIdentity, RootLock};
use crate::store::atomic::{AtomicError, Statement};
use crate::store::authority::{
    check_owner_in_current_transaction, mark_process_active, mark_process_stopped,
    mark_process_unknown, read_product_identity, record_prepared_process, OwnerIssuer,
};
use crate::store::digest::{content_hash, sha256_hex};
use crate::store::same_open::{SameOpenError, VerifiedDatabaseConnection};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SOURCE_REMOTE: &str = "https://github.com/taiyun668/gogoke-seat-testbed.git";
const SOURCE_REMOTE_SSH: &str = "git@github.com:taiyun668/gogoke-seat-testbed.git";

fn testbed_remote_kind(remote: &str) -> Option<&'static str> {
    match remote {
        SOURCE_REMOTE | "https://github.com/taiyun668/gogoke-seat-testbed" => Some("HTTPS"),
        SOURCE_REMOTE_SSH
        | "git@github.com:taiyun668/gogoke-seat-testbed"
        | "ssh://git@github.com/taiyun668/gogoke-seat-testbed.git" => Some("SSH"),
        _ => None,
    }
}
const REPARSE_POINT: u32 = 0x400;
const GIT_TIMEOUT: Duration = Duration::from_secs(25);
const SCHEMA: [(&str, &str); 3] = [
    ("gogoke_v37_worktree_sources", "CREATE TABLE gogoke_v37_worktree_sources(repository_id TEXT PRIMARY KEY,source_path TEXT NOT NULL UNIQUE,source_identity TEXT NOT NULL UNIQUE,common_path TEXT NOT NULL,common_identity TEXT NOT NULL,remote_kind TEXT NOT NULL CHECK(remote_kind IN ('HTTPS','SSH')),baseline_commit TEXT NOT NULL,git_digest TEXT NOT NULL,git_version TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision=1)) STRICT"),
    ("gogoke_v37_worktree_operations", "CREATE TABLE gogoke_v37_worktree_operations(request_id TEXT PRIMARY KEY,request_hash TEXT NOT NULL,repository_id TEXT NOT NULL,domain_id TEXT NOT NULL,seat_id TEXT NOT NULL,worktree_id TEXT NOT NULL UNIQUE,path_id TEXT NOT NULL UNIQUE,seat_incarnation TEXT NOT NULL,seat_generation INTEGER NOT NULL,seat_revision INTEGER NOT NULL,instance_id TEXT NOT NULL,permission_tier TEXT NOT NULL,phase TEXT NOT NULL CHECK(phase IN ('INTENT','UNKNOWN','REGISTERED')),cause TEXT NOT NULL DEFAULT '') STRICT"),
    ("gogoke_v37_worktrees", "CREATE TABLE gogoke_v37_worktrees(worktree_id TEXT PRIMARY KEY,path_id TEXT NOT NULL UNIQUE,repository_id TEXT NOT NULL REFERENCES gogoke_v37_worktree_sources(repository_id),domain_id TEXT NOT NULL,seat_id TEXT NOT NULL,seat_incarnation TEXT NOT NULL,seat_generation INTEGER NOT NULL,seat_revision INTEGER NOT NULL,permission_tier TEXT NOT NULL,instance_id TEXT NOT NULL,source_revision INTEGER NOT NULL,worktree_path TEXT NOT NULL UNIQUE,worktree_identity TEXT NOT NULL UNIQUE,git_pointer_hash TEXT NOT NULL,git_pointer_len INTEGER NOT NULL,git_pointer_identity TEXT NOT NULL UNIQUE,common_identity TEXT NOT NULL,baseline_commit TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('REGISTERED','UNKNOWN')),revision INTEGER NOT NULL CHECK(revision=1)) STRICT"),
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
    if tag == "source_attrs" {
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
    if !observed.is_empty() {
        return Err(WorktreeError::SchemaDrift);
    }
    transaction(db, |db| {
        no_shadow(db)?;
        if !family(db)?.is_empty() {
            return Err(WorktreeError::SchemaDrift);
        }
        for (_, sql) in SCHEMA {
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
        let mut pin = Self {
            path: canonical,
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
    launch.current_directory = cwd
        .map(Path::to_path_buf)
        .or_else(|| Some(root.canonical_root().canonical_path.clone()));
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
mod tests {
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
    fn testbed_remote_never_admits_credentials_or_lookalike() {
        assert_eq!(testbed_remote_kind(SOURCE_REMOTE), Some("HTTPS"));
        assert_eq!(testbed_remote_kind(SOURCE_REMOTE_SSH), Some("SSH"));
        let credential_url = format!(
            "https://user:{}@github.com/taiyun668/gogoke-seat-testbed.git",
            "placeholder"
        );
        assert_eq!(testbed_remote_kind(&credential_url), None);
        assert_eq!(
            testbed_remote_kind("https://github.com/taiyun668/gogoke-seat-testbed-copy.git"),
            None
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
    let remote_kind = testbed_remote_kind(&remote).ok_or(WorktreeError::Denied)?;
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

fn ensure_worktree_parent(root: &RootLock) -> Result<PathBuf> {
    let mut parent = root.canonical_root().canonical_path.clone();
    for component in ["v37-worktrees", "single"] {
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
    let (source, source_identity, common, common_identity, baseline, seat_snapshot) = transaction(
        db,
        |db| {
            check_owner_in_current_transaction(db, owner)?;
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
            Ok((result.0, result.1, result.2, result.3, result.4, snapshot))
        },
    ).map_err(|error| { if uncertain(&error) {
        RootLock::poison_identity(&root.canonical_root().identity);
    } error })?;
    let source = PathBuf::from(source);
    let common = PathBuf::from(common);
    let target = root
        .canonical_root()
        .canonical_path
        .join("v37-worktrees")
        .join("single")
        .join(&path_id);
    let effect = (|| -> Result<ResolvedBinding> {
        if inspect_root(&source)?.identity.opaque() != source_identity
            || inspect_root(&common)?.identity.opaque() != common_identity
        {
            return Err(WorktreeError::Denied);
        }
        source_checkout_has_no_external_drivers(
            db, root, custodian, pin, &source, &common, &baseline,
        )?;
        ensure_worktree_parent(root)?;
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
                target.to_string_lossy().into_owned(),
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
    let exact = root
        .canonical_root()
        .canonical_path
        .join("v37-worktrees")
        .join("single")
        .join(&path_id);
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
