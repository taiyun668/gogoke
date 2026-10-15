//! Durable version-action bytes and compare-and-swap state only. A row, its
//! source fingerprint, and a replayed receipt never establish cold eligibility.
//! The caller must reobserve the original native holders and every guard in the
//! same verified Owner transaction that performs the copy/source/switch effect.

use super::managed_cli::read_official_cli_version;
use crate::root::{inspect_root, RootLock};
use crate::store::atomic::Statement;
use crate::store::authority::{check_owner_in_current_transaction, OwnerIssuer};
use crate::store::digest::content_hash;
use crate::store::same_open::VerifiedDatabaseConnection;

pub(super) const SCHEMA: &str = "CREATE TABLE gogoke_v37_instance_version_actions(request_id TEXT PRIMARY KEY,open_driver_id TEXT UNIQUE,root_identity TEXT NOT NULL,database_identity TEXT NOT NULL,driver_id TEXT NOT NULL,operation TEXT NOT NULL CHECK(operation IN ('upgrade','rollback')),target_version TEXT NOT NULL,target_archive_sha256 TEXT NOT NULL,target_image_sha256 TEXT NOT NULL,begin_hex TEXT NOT NULL,begin_hash TEXT NOT NULL,begin_receipt_hex TEXT NOT NULL,mode TEXT NOT NULL CHECK(mode IN ('STRICT','COLD')),proof_family TEXT NOT NULL,source_hex TEXT NOT NULL,source_digest TEXT NOT NULL,pairs_hex TEXT NOT NULL,pairs_digest TEXT NOT NULL,initial_copy_fingerprint TEXT NOT NULL,initial_source_fingerprint TEXT NOT NULL,current_copy_fingerprint TEXT NOT NULL,current_source_fingerprint TEXT NOT NULL,stage_copy_fingerprint TEXT NOT NULL DEFAULT '',stage_source_fingerprint TEXT NOT NULL DEFAULT '',stage_name TEXT NOT NULL DEFAULT '',stage_hex TEXT NOT NULL DEFAULT '',stage_receipt_hex TEXT NOT NULL DEFAULT '',probe_hex TEXT NOT NULL DEFAULT '',probe_receipt_hex TEXT NOT NULL DEFAULT '',probe_copy_fingerprint TEXT NOT NULL DEFAULT '',probe_source_fingerprint TEXT NOT NULL DEFAULT '',final_pre_copy_fingerprint TEXT NOT NULL DEFAULT '',final_pre_source_fingerprint TEXT NOT NULL DEFAULT '',final_hex TEXT NOT NULL DEFAULT '',final_hash TEXT NOT NULL DEFAULT '',final_receipt_set_digest TEXT NOT NULL DEFAULT '',original_error_hex TEXT NOT NULL DEFAULT '',begin_effect_copy_fingerprint TEXT NOT NULL DEFAULT '',begin_effect_source_fingerprint TEXT NOT NULL DEFAULT '',error_copy_fingerprint TEXT NOT NULL DEFAULT '',error_source_fingerprint TEXT NOT NULL DEFAULT '',phase TEXT NOT NULL CHECK(phase IN ('BEGUN','STAGED','PROBING','PROBED','CONSUMED','FAILED')),revision INTEGER NOT NULL CHECK(revision>=1),CHECK((phase IN ('BEGUN','STAGED','PROBING','PROBED') AND open_driver_id=driver_id) OR (phase IN ('CONSUMED','FAILED') AND open_driver_id IS NULL))) STRICT";

#[derive(Debug)]
pub(crate) enum ActionError {
    Invalid(&'static str),
    Busy,
    Conflict,
    Missing,
    Identity(String),
    Authority(String),
    Store(crate::store::atomic::AtomicError),
}

#[cfg(test)]
mod family_codec_controls {
    use super::*;

    fn encode(parts:&[String])->String {
        let mut framed=Vec::new();
        for part in parts {
            framed.extend_from_slice(&(part.len() as u64).to_be_bytes());
            framed.extend_from_slice(part.as_bytes());
        }
        framed.iter().map(|byte|format!("{byte:02x}")).collect()
    }

    fn original_open_member()->(Vec<String>,Vec<String>) {
        // These are the actual H OPEN/C column positions emitted by F's
        // original source collector. This tests only journal decoding; a
        // source-shaped string never substitutes for H or kernel evidence.
        let mut source=vec![String::new();42];
        source[0]="H_SESSION_OPEN-v1".into();
        source[1]="instance-a".into();source[3]="global".into();
        source[4]="session-a".into();source[5]="1".into();
        source[6]="open-a".into();source[7]="h-open-a".into();
        source[11]="6162".into();source[32]="6162".into();
        source[16]="123".into();source[17]="456".into();
        source[35]="no-alias".into();source[36]="0".into();
        source[37]="1".into();source[38]="seat-a".into();
        source[39]="incarnation-a".into();
        source[40]="global-native-selection".into();
        source[41]="a".repeat(64);
        let proof=vec!["KERNEL_REBOOT".into(),"instance-a".into(),"global".into(),
            "session-a".into(),"1".into(),"h-open-a".into(),
            "123".into(),"456".into()];
        (source,proof)
    }

    #[test]
    fn family_codec_requires_one_canonical_source_for_each_original_member() {
        let (source,proof)=original_open_member();
        let make=|members:&[(Vec<String>,Vec<String>)]| {
            let mut sources=vec!["managed-cli-family-source-v1".into(),"codex".into()];
            let mut proofs=vec!["managed-cli-family-proof-v1".into(),"codex".into()];
            for (source,proof) in members {
                sources.push(encode(source));proofs.push(encode(proof));
            }
            (encode(&sources),encode(&proofs))
        };
        let positive=make(&[(source.clone(),proof.clone())]);
        assert!(valid_family_codec("codex",&positive.0,&positive.1).is_ok());
        for changed in [
            (vec![(source.clone(),proof.clone()),(source.clone(),proof.clone())],"duplicate"),
            (vec![],"empty"),
        ] {
            let bytes=make(&changed.0);
            assert!(valid_family_codec("codex",&bytes.0,&bytes.1).is_err(),
                "{} member must refuse",changed.1);
        }
        let mut changed=source.clone();changed[37]="0".into();
        let bytes=make(&[(changed,proof.clone())]);
        assert!(valid_family_codec("codex",&bytes.0,&bytes.1).is_err(),
            "NULL and empty source states differ");
        let mut changed=proof.clone();changed[0]="EXACT_GONE".into();
        let bytes=make(&[(source.clone(),changed)]);
        assert!(valid_family_codec("codex",&bytes.0,&bytes.1).is_err(),
            "a reboot source cannot be relabeled exact gone");
        let mut changed=proof.clone();changed[7]="457".into();
        let bytes=make(&[(source.clone(),changed)]);
        assert!(valid_family_codec("codex",&bytes.0,&bytes.1).is_err(),
            "the exact C FILETIME is part of membership");
        assert!(valid_family_codec("codex",&positive.0,&positive.1).is_ok());
    }
}
impl From<crate::store::atomic::AtomicError> for ActionError {
    fn from(value: crate::store::atomic::AtomicError) -> Self { Self::Store(value) }
}
type Result<T> = std::result::Result<T, ActionError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActionOperation { Upgrade, Rollback }
impl ActionOperation {
    fn as_str(self) -> &'static str { match self { Self::Upgrade => "upgrade", Self::Rollback => "rollback" } }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActionMode { Strict, Cold }
impl ActionMode {
    fn as_str(self) -> &'static str { match self { Self::Strict => "STRICT", Self::Cold => "COLD" } }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActionPhase { Begun, Staged, Probing, Probed, Consumed, Failed }
impl ActionPhase {
    fn as_str(self) -> &'static str { match self {
        Self::Begun => "BEGUN", Self::Staged => "STAGED", Self::Probing => "PROBING", Self::Probed => "PROBED",
        Self::Consumed => "CONSUMED", Self::Failed => "FAILED",
    }}
    fn parse(value: &str) -> Result<Self> { match value {
        "BEGUN" => Ok(Self::Begun), "STAGED" => Ok(Self::Staged),
        "PROBING" => Ok(Self::Probing), "PROBED" => Ok(Self::Probed), "CONSUMED" => Ok(Self::Consumed),
        "FAILED" => Ok(Self::Failed), _ => Err(ActionError::Conflict),
    }}
}

/// Source and pair bytes are canonical, opaque snapshots produced by the
/// caller's trusted source collector. They confer no authority in this module.
pub(crate) struct ActionBegin<'a> {
    pub request_id: &'a str,
    pub driver: &'a str,
    pub operation: ActionOperation,
    pub target_version: &'a str,
    pub begin_hex: &'a str,
    pub begin_receipt_hex: &'a str,
    pub mode: ActionMode,
    pub proof_family: &'a str,
    pub source_hex: &'a str,
    pub source_digest: &'a str,
    pub pairs_hex: &'a str,
    pub pairs_digest: &'a str,
    pub copy_fingerprint: &'a str,
    pub source_fingerprint: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActionRecord {
    pub request_id: String,
    pub open_driver_id: String,
    pub root_identity: String,
    pub database_identity: String,
    pub driver: String,
    pub operation: String,
    pub target_version: String,
    pub target_archive_sha256: String,
    pub target_image_sha256: String,
    pub begin_hex: String,
    pub begin_hash: String,
    pub begin_receipt_hex: String,
    pub mode: String,
    pub proof_family: String,
    pub source_hex: String,
    pub source_digest: String,
    pub pairs_hex: String,
    pub pairs_digest: String,
    pub initial_copy_fingerprint: String,
    pub initial_source_fingerprint: String,
    pub begin_effect_copy_fingerprint: String,
    pub begin_effect_source_fingerprint: String,
    pub current_copy_fingerprint: String,
    pub current_source_fingerprint: String,
    pub stage_copy_fingerprint: String,
    pub stage_source_fingerprint: String,
    pub stage_name: String,
    pub stage_hex: String,
    pub stage_receipt_hex: String,
    pub probe_hex: String,
    pub probe_receipt_hex: String,
    pub probe_copy_fingerprint: String,
    pub probe_source_fingerprint: String,
    pub final_pre_copy_fingerprint: String,
    pub final_pre_source_fingerprint: String,
    pub final_hex: String,
    pub final_hash: String,
    pub final_receipt_set_digest: String,
    pub original_error_hex: String,
    pub error_copy_fingerprint: String,
    pub error_source_fingerprint: String,
    pub phase: ActionPhase,
    pub revision: i64,
}

pub(crate) enum BeginOutcome { Created(ActionRecord), Replayed(ActionRecord) }

pub(crate) struct ActionStep<'a> {
    pub next_phase: ActionPhase,
    pub raw_hex: &'a str,
    pub receipt_hex: &'a str,
    pub stage_name: &'a str,
    pub expected_copy_fingerprint: &'a str,
    pub expected_source_fingerprint: &'a str,
    pub next_copy_fingerprint: &'a str,
    pub next_source_fingerprint: &'a str,
}

fn bounded_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && value.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-:.".contains(&b))
}
fn digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn bytes(value: &str) -> Result<Vec<u8>> {
    if value.len() % 2 != 0 || !value.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
        return Err(ActionError::Invalid("hex"));
    }
    value.as_bytes().chunks_exact(2).map(|pair| {
        let text = std::str::from_utf8(pair).map_err(|_| ActionError::Invalid("hex"))?;
        u8::from_str_radix(text, 16).map_err(|_| ActionError::Invalid("hex"))
    }).collect()
}
fn matching_digest(hex: &str, expected: &str) -> Result<bool> {
    Ok(expected == content_hash(&bytes(hex)?))
}

/// The journal records an identity binding, never a native proof. Decode it
/// before accepting or replaying a family action so malformed, reordered and
/// ambiguous members cannot acquire a different meaning on a later read.
fn unframe(raw: &[u8]) -> Result<Vec<String>> {
    if raw.len() > 8_388_608 { return Err(ActionError::Conflict); }
    let mut parts=Vec::new();
    let mut offset=0usize;
    while offset<raw.len() {
        let end=offset.checked_add(8).ok_or(ActionError::Conflict)?;
        let size=u64::from_be_bytes(raw.get(offset..end).ok_or(ActionError::Conflict)?
            .try_into().map_err(|_|ActionError::Conflict)?);
        let size=usize::try_from(size).map_err(|_|ActionError::Conflict)?;
        offset=end;
        let end=offset.checked_add(size).ok_or(ActionError::Conflict)?;
        let part=raw.get(offset..end).ok_or(ActionError::Conflict)?;
        parts.push(std::str::from_utf8(part).map_err(|_|ActionError::Conflict)?.to_owned());
        if parts.len()>4096 {return Err(ActionError::Conflict)}
        offset=end;
    }
    Ok(parts)
}

fn valid_family_codec(driver:&str,source_hex:&str,pairs_hex:&str)->Result<()> {
    let sources=unframe(&bytes(source_hex).map_err(|_|ActionError::Conflict)?)?;
    let proofs=unframe(&bytes(pairs_hex).map_err(|_|ActionError::Conflict)?)?;
    if sources.len()<3 || sources.len()!=proofs.len() ||
        sources[0]!="managed-cli-family-source-v1" ||
        proofs[0]!="managed-cli-family-proof-v1" ||
        sources[1]!=driver || proofs[1]!=driver {return Err(ActionError::Conflict)}
    let mut previous:Option<(String,String,String,String,String)>=None;
    let mut pairs=std::collections::BTreeSet::new();
    for (source,proof) in sources[2..].iter().zip(&proofs[2..]) {
        let source=unframe(&bytes(source).map_err(|_|ActionError::Conflict)?)?;
        let proof=unframe(&bytes(proof).map_err(|_|ActionError::Conflict)?)?;
        if proof.len()!=8 {return Err(ActionError::Conflict)}
        let (key,pid_field,created_field)=match source.first().map(String::as_str) {
            Some("H_SESSION_OPEN-v1") if source.len()==42 &&
                proof[0]=="KERNEL_REBOOT" && source[35]=="no-alias" &&
                source[36]=="0" && source[37]=="1" && source[21].is_empty() &&
                source[11]==source[32] && !source[38].is_empty() &&
                !source[39].is_empty() &&
                ((source[3]=="global" && source[40]=="global-native-selection") ||
                 (source[3]!="global" && digest(&source[40]))) &&
                digest(&source[41]) =>
                ((&source[1],&source[3],&source[4],&source[5],&source[7]),
                    &source[16],&source[17]),
            Some("H_SESSION_STOP-v1") if source.len()==39 &&
                proof[0]=="ORIGINAL_STOP" && source[15]=="STOPPED" &&
                source[19]=="STOPPED" && source[14]==source[20] &&
                source[32]=="1" && source[33]=="0" &&
                source[34]=="0" && source[35]=="0" &&
                digest(&source[36]) && digest(&source[37]) &&
                ((source[2]=="global" && source[38]=="global-native-selection") ||
                 (source[2]!="global" && digest(&source[38]))) =>
                ((&source[1],&source[2],&source[3],&source[4],&source[6]),
                    &source[23],&source[24]),
            Some("GLOBAL_LOGIN_C-v1") if source.len()==17 &&
                source[9]=="global" &&
                ((source[11]=="UNKNOWN" && driver=="codex" &&
                    proof[0]=="KERNEL_REBOOT" &&
                    source[12].is_empty() && source[13]=="1") ||
                 (source[11]=="STOPPED" && proof[0]=="ORIGINAL_STOP" &&
                    source[13]=="0" && source[12].starts_with("sha256:") &&
                    source[12].len()==71)) =>
                ((&source[8],&source[9],&source[1],&source[10],&source[1]),
                    &source[4],&source[5]),
            Some("MANAGED_PROBE_C-v1") if source.len()==19 &&
                proof[0]=="ORIGINAL_STOP" && proof[2]=="managed-probe" &&
                source[8]==format!("managed-cli-{driver}") &&
                source[9]=="global" && source[11]=="STOPPED" && source[13]=="0" &&
                source[12].starts_with("sha256:") && source[12].len()==71 &&
                !source[14].is_empty() && !source[15].is_empty() &&
                !source[16].is_empty() && !source[17].is_empty() &&
                !source[18].is_empty() && source[1].starts_with(&format!(
                    "managed-probe-{driver}-{}-",
                    &crate::store::digest::sha256_hex(source[14].as_bytes())[..40])) =>
                ((&source[8],&proof[2],&source[1],&source[10],&source[1]),
                    &source[4],&source[5]),
            _=>return Err(ActionError::Conflict),
        };
        if [key.0,key.1,key.2,key.3,key.4].iter().any(|value|value.is_empty()) ||
            proof[1].as_str()!=key.0.as_str() ||
            proof[2].as_str()!=key.1.as_str() ||
            proof[3].as_str()!=key.2.as_str() ||
            proof[4].as_str()!=key.3.as_str() ||
            proof[5].as_str()!=key.4.as_str() ||
            proof[6].as_str()!=pid_field.as_str() ||
            proof[7].as_str()!=created_field.as_str() {
            return Err(ActionError::Conflict)
        }
        let pid=proof[6].parse::<u32>().map_err(|_|ActionError::Conflict)?;
        let created=proof[7].parse::<u64>().map_err(|_|ActionError::Conflict)?;
        if pid==0 || created==0 || proof[6]!=pid.to_string() ||
            proof[7]!=created.to_string() || !pairs.insert((pid,created)) {
            return Err(ActionError::Conflict);
        }
        let canonical=(key.0.clone(),key.1.clone(),key.2.clone(),
            key.3.clone(),key.4.clone());
        if previous.as_ref().is_some_and(|earlier|earlier>=&canonical) {
            return Err(ActionError::Conflict);
        }
        previous=Some(canonical);
    }
    Ok(())
}
fn checked_identity(db: &VerifiedDatabaseConnection<'_>, root: &RootLock) -> Result<(String,String)> {
    let observed = inspect_root(&root.canonical_root().canonical_path)
        .map_err(|error| ActionError::Identity(format!("root observation: {error:?}")))?;
    if observed.identity != root.canonical_root().identity || db.root_identity() != &observed.identity {
        return Err(ActionError::Identity("root file identity changed".into()));
    }
    Ok((observed.identity.opaque(), db.identity().opaque()))
}
fn mutation_authority(db: &VerifiedDatabaseConnection<'_>, root: &RootLock, owner: &OwnerIssuer) -> Result<(String,String)> {
    check_owner_in_current_transaction(db, owner)
        .map_err(|error| ActionError::Authority(format!("owner current transaction: {error:?}")))?;
    checked_identity(db, root)
}

fn validate_record(row: &ActionRecord) -> Result<()> {
    let pin = read_official_cli_version(&row.driver, &row.target_version).ok_or(ActionError::Conflict)?;
    if row.target_archive_sha256 != pin.archive_sha256 || row.target_image_sha256 != pin.image_sha256 ||
        !super::registry::valid_managed_switch_request_id(&row.request_id) || !bounded_id(&row.driver) ||
        !matches!(row.operation.as_str(), "upgrade" | "rollback") ||
        bytes(&row.begin_hex).map_err(|_| ActionError::Conflict)?.is_empty() ||
        bytes(&row.begin_receipt_hex).map_err(|_| ActionError::Conflict)?.is_empty() ||
        row.begin_hash != content_hash(&bytes(&row.begin_hex).map_err(|_| ActionError::Conflict)?) {
        return Err(ActionError::Conflict);
    }
    if row.mode == "STRICT" {
        if !row.proof_family.is_empty() || !row.source_hex.is_empty() || !row.source_digest.is_empty() ||
            !row.pairs_hex.is_empty() || !row.pairs_digest.is_empty() { return Err(ActionError::Conflict); }
    } else if row.mode == "COLD" {
        if !matches!(row.proof_family.as_str(), "exact-gone" | "reboot" | "family-v1") ||
            !matching_digest(&row.source_hex, &row.source_digest).unwrap_or(false) ||
            !matching_digest(&row.pairs_hex, &row.pairs_digest).unwrap_or(false) ||
            row.source_hex.is_empty() || row.pairs_hex.is_empty() { return Err(ActionError::Conflict); }
        if row.proof_family=="family-v1" &&
            valid_family_codec(&row.driver,&row.source_hex,&row.pairs_hex).is_err() {
            return Err(ActionError::Conflict);
        }
    } else { return Err(ActionError::Conflict); }
    if row.initial_copy_fingerprint.is_empty() || row.initial_source_fingerprint.is_empty() ||
        row.current_copy_fingerprint.is_empty() || row.current_source_fingerprint.is_empty() {
        return Err(ActionError::Conflict);
    }
    let observed_error = !row.original_error_hex.is_empty();
    if observed_error {
        if bytes(&row.original_error_hex).map_err(|_| ActionError::Conflict)?.is_empty() ||
            (row.phase != ActionPhase::Failed &&
                (row.error_copy_fingerprint.is_empty() || row.error_source_fingerprint.is_empty())) {
            return Err(ActionError::Conflict);
        }
    } else if !row.error_copy_fingerprint.is_empty() || !row.error_source_fingerprint.is_empty() {
        return Err(ActionError::Conflict);
    }
    if row.phase == ActionPhase::Begun {
        let pristine = !observed_error && row.revision == 1 && row.begin_effect_copy_fingerprint.is_empty() &&
            row.begin_effect_source_fingerprint.is_empty() &&
            row.current_copy_fingerprint == row.initial_copy_fingerprint &&
            row.current_source_fingerprint == row.initial_source_fingerprint;
        let progressed = !observed_error && row.revision == 2 && !row.begin_effect_copy_fingerprint.is_empty() &&
            !row.begin_effect_source_fingerprint.is_empty() &&
            row.current_copy_fingerprint == row.begin_effect_copy_fingerprint &&
            row.current_source_fingerprint == row.begin_effect_source_fingerprint;
        let errored = observed_error && row.current_copy_fingerprint == row.error_copy_fingerprint &&
            row.current_source_fingerprint == row.error_source_fingerprint;
        if !pristine && !progressed && !errored { return Err(ActionError::Conflict); }
    }
    if matches!(row.phase, ActionPhase::Staged | ActionPhase::Probing | ActionPhase::Probed | ActionPhase::Consumed) &&
        (row.begin_effect_copy_fingerprint.is_empty() || row.begin_effect_source_fingerprint.is_empty()) {
        return Err(ActionError::Conflict);
    }
    if matches!(row.phase, ActionPhase::Staged | ActionPhase::Probing | ActionPhase::Probed | ActionPhase::Consumed) &&
        (row.stage_name.is_empty() || bytes(&row.stage_hex).map_err(|_| ActionError::Conflict)?.is_empty() ||
        bytes(&row.stage_receipt_hex).map_err(|_| ActionError::Conflict)?.is_empty() ||
        row.stage_copy_fingerprint.is_empty() || row.stage_source_fingerprint.is_empty()) {
        return Err(ActionError::Conflict);
    }
    if row.phase == ActionPhase::Staged &&
        (if observed_error {
            row.current_copy_fingerprint != row.error_copy_fingerprint ||
                row.current_source_fingerprint != row.error_source_fingerprint
        } else {
            row.current_copy_fingerprint != row.stage_copy_fingerprint ||
                row.current_source_fingerprint != row.stage_source_fingerprint
        }) { return Err(ActionError::Conflict); }
    if row.phase == ActionPhase::Probing {
        let untouched = !observed_error &&
            row.current_copy_fingerprint == row.stage_copy_fingerprint &&
            row.current_source_fingerprint == row.stage_source_fingerprint;
        let errored = observed_error &&
            row.current_copy_fingerprint == row.error_copy_fingerprint &&
            row.current_source_fingerprint == row.error_source_fingerprint;
        if bytes(&row.probe_hex).map_err(|_| ActionError::Conflict)?.is_empty() ||
            !row.probe_receipt_hex.is_empty() || !row.probe_copy_fingerprint.is_empty() ||
            !row.probe_source_fingerprint.is_empty() || (!untouched && !errored) {
            return Err(ActionError::Conflict);
        }
    }
    if matches!(row.phase, ActionPhase::Probed | ActionPhase::Consumed) &&
        (bytes(&row.probe_hex).map_err(|_| ActionError::Conflict)?.is_empty() ||
        bytes(&row.probe_receipt_hex).map_err(|_| ActionError::Conflict)?.is_empty() ||
        row.probe_copy_fingerprint.is_empty() || row.probe_source_fingerprint.is_empty()) {
        return Err(ActionError::Conflict);
    }
    if row.phase == ActionPhase::Probed &&
        ((row.current_copy_fingerprint != row.probe_copy_fingerprint ||
        row.current_source_fingerprint != row.probe_source_fingerprint) &&
        (!observed_error || row.current_copy_fingerprint != row.error_copy_fingerprint ||
        row.current_source_fingerprint != row.error_source_fingerprint)) { return Err(ActionError::Conflict); }
    if row.phase == ActionPhase::Consumed &&
        (bytes(&row.final_hex).map_err(|_| ActionError::Conflict)?.is_empty() ||
        row.final_hash != content_hash(&bytes(&row.final_hex).map_err(|_| ActionError::Conflict)?) ||
        !digest(&row.final_receipt_set_digest) ||
        row.final_pre_copy_fingerprint != row.probe_copy_fingerprint ||
        row.final_pre_source_fingerprint != row.probe_source_fingerprint) { return Err(ActionError::Conflict); }
    if row.phase == ActionPhase::Failed && bytes(&row.original_error_hex).map_err(|_| ActionError::Conflict)?.is_empty() {
        return Err(ActionError::Conflict);
    }
    Ok(())
}

/// A read only restores bytes. Its caller must revalidate live native facts.
pub(crate) fn read(db: &VerifiedDatabaseConnection<'_>, root: &RootLock, request_id: &str) -> Result<Option<ActionRecord>> {
    let (root_identity, database_identity) = checked_identity(db, root)?;
    if !super::registry::valid_managed_switch_request_id(request_id) { return Err(ActionError::Invalid("request id")); }
    let q = Statement::prepare(db.as_ptr(), "SELECT request_id,COALESCE(open_driver_id,''),root_identity,database_identity,driver_id,operation,target_version,target_archive_sha256,target_image_sha256,begin_hex,begin_hash,begin_receipt_hex,mode,proof_family,source_hex,source_digest,pairs_hex,pairs_digest,initial_copy_fingerprint,initial_source_fingerprint,current_copy_fingerprint,current_source_fingerprint,stage_copy_fingerprint,stage_source_fingerprint,stage_name,stage_hex,stage_receipt_hex,probe_hex,probe_receipt_hex,probe_copy_fingerprint,probe_source_fingerprint,final_pre_copy_fingerprint,final_pre_source_fingerprint,final_hex,final_hash,final_receipt_set_digest,original_error_hex,begin_effect_copy_fingerprint,begin_effect_source_fingerprint,error_copy_fingerprint,error_source_fingerprint,phase,revision FROM main.gogoke_v37_instance_version_actions WHERE request_id=?1")?;
    q.bind_text(1, request_id)?;
    if !q.step_row()? { return Ok(None); }
    let mut columns = Vec::with_capacity(43);
    for index in 0..43 { columns.push(q.column_text(index)?); }
    if q.step_row()? { return Err(ActionError::Conflict); }
    if columns[2] != root_identity || columns[3] != database_identity { return Err(ActionError::Identity("journal file identity changed".into())); }
    let phase = ActionPhase::parse(&columns[41])?;
    let revision = columns[42].parse::<i64>().map_err(|_| ActionError::Conflict)?;
    if revision < 1 || (matches!(phase, ActionPhase::Consumed | ActionPhase::Failed) != columns[1].is_empty()) {
        return Err(ActionError::Conflict);
    }
    let row = ActionRecord {
        request_id: columns[0].clone(), open_driver_id: columns[1].clone(), root_identity: columns[2].clone(),
        database_identity: columns[3].clone(), driver: columns[4].clone(), operation: columns[5].clone(),
        target_version: columns[6].clone(), target_archive_sha256: columns[7].clone(), target_image_sha256: columns[8].clone(),
        begin_hex: columns[9].clone(), begin_hash: columns[10].clone(), begin_receipt_hex: columns[11].clone(),
        mode: columns[12].clone(), proof_family: columns[13].clone(), source_hex: columns[14].clone(),
        source_digest: columns[15].clone(), pairs_hex: columns[16].clone(), pairs_digest: columns[17].clone(),
        initial_copy_fingerprint: columns[18].clone(), initial_source_fingerprint: columns[19].clone(),
        begin_effect_copy_fingerprint: columns[37].clone(), begin_effect_source_fingerprint: columns[38].clone(),
        current_copy_fingerprint: columns[20].clone(), current_source_fingerprint: columns[21].clone(),
        stage_copy_fingerprint: columns[22].clone(), stage_source_fingerprint: columns[23].clone(),
        stage_name: columns[24].clone(), stage_hex: columns[25].clone(), stage_receipt_hex: columns[26].clone(),
        probe_hex: columns[27].clone(), probe_receipt_hex: columns[28].clone(),
        probe_copy_fingerprint: columns[29].clone(), probe_source_fingerprint: columns[30].clone(),
        final_pre_copy_fingerprint: columns[31].clone(), final_pre_source_fingerprint: columns[32].clone(),
        final_hex: columns[33].clone(), final_hash: columns[34].clone(), final_receipt_set_digest: columns[35].clone(),
        original_error_hex: columns[36].clone(), phase, revision,
        error_copy_fingerprint: columns[39].clone(), error_source_fingerprint: columns[40].clone(),
    };
    validate_record(&row)?;
    Ok(Some(row))
}

fn affected_one(db: &VerifiedDatabaseConnection<'_>) -> Result<()> {
    let q = Statement::prepare(db.as_ptr(), "SELECT changes()")?;
    if !q.step_row()? || q.column_text(0)? != "1" || q.step_row()? { return Err(ActionError::Conflict); }
    Ok(())
}

pub(crate) fn begin_in_current_transaction(
    db: &VerifiedDatabaseConnection<'_>, root: &RootLock, owner: &OwnerIssuer, begin: &ActionBegin<'_>,
) -> Result<BeginOutcome> {
    let (root_identity, database_identity) = mutation_authority(db, root, owner)?;
    if !super::registry::valid_managed_switch_request_id(begin.request_id) || !bounded_id(begin.driver) ||
        !bounded_id(begin.target_version) || !bounded_id(begin.copy_fingerprint) ||
        !bounded_id(begin.source_fingerprint) { return Err(ActionError::Invalid("begin identity")); }
    let pin = read_official_cli_version(begin.driver, begin.target_version).ok_or(ActionError::Invalid("official target pin"))?;
    if bytes(begin.begin_hex)?.is_empty() || bytes(begin.begin_receipt_hex)?.is_empty() { return Err(ActionError::Invalid("begin bytes")); }
    let begin_hash = content_hash(&bytes(begin.begin_hex)?);
    match begin.mode {
        ActionMode::Strict if !begin.proof_family.is_empty() || !begin.source_hex.is_empty() ||
            !begin.source_digest.is_empty() || !begin.pairs_hex.is_empty() || !begin.pairs_digest.is_empty() =>
            return Err(ActionError::Invalid("strict proof fields")),
        ActionMode::Cold if !matches!(begin.proof_family, "exact-gone" | "reboot" | "family-v1") ||
            bytes(begin.source_hex)?.is_empty() || bytes(begin.pairs_hex)?.is_empty() ||
            !matching_digest(begin.source_hex, begin.source_digest)? ||
            !matching_digest(begin.pairs_hex, begin.pairs_digest)? =>
            return Err(ActionError::Invalid("cold proof snapshot")),
        _ => (),
    }
    if begin.mode==ActionMode::Cold && begin.proof_family=="family-v1" &&
        valid_family_codec(begin.driver,begin.source_hex,begin.pairs_hex).is_err() {
        return Err(ActionError::Invalid("family proof snapshot"));
    }
    if let Some(row) = read(db, root, begin.request_id)? {
        if row.driver != begin.driver || row.operation != begin.operation.as_str() ||
            row.target_version != pin.version || row.target_archive_sha256 != pin.archive_sha256 ||
            row.target_image_sha256 != pin.image_sha256 || row.begin_hex != begin.begin_hex ||
            row.begin_hash != begin_hash || row.begin_receipt_hex != begin.begin_receipt_hex ||
            row.mode != begin.mode.as_str() || row.proof_family != begin.proof_family ||
            row.source_hex != begin.source_hex || row.source_digest != begin.source_digest ||
            row.pairs_hex != begin.pairs_hex || row.pairs_digest != begin.pairs_digest ||
            row.initial_copy_fingerprint != begin.copy_fingerprint ||
            row.initial_source_fingerprint != begin.source_fingerprint {
            return Err(ActionError::Conflict);
        }
        return Ok(BeginOutcome::Replayed(row));
    }
    let q = Statement::prepare(db.as_ptr(), "SELECT 1 FROM main.gogoke_v37_instance_version_actions WHERE open_driver_id=?1 LIMIT 1")?;
    q.bind_text(1, begin.driver)?;
    if q.step_row()? { return Err(ActionError::Busy); }
    let insert = Statement::prepare(db.as_ptr(), "INSERT INTO main.gogoke_v37_instance_version_actions(request_id,open_driver_id,root_identity,database_identity,driver_id,operation,target_version,target_archive_sha256,target_image_sha256,begin_hex,begin_hash,begin_receipt_hex,mode,proof_family,source_hex,source_digest,pairs_hex,pairs_digest,initial_copy_fingerprint,initial_source_fingerprint,current_copy_fingerprint,current_source_fingerprint,phase,revision) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,'BEGUN',1)")?;
    let values = [begin.request_id,begin.driver,&root_identity,&database_identity,begin.driver,
        begin.operation.as_str(),pin.version,pin.archive_sha256,pin.image_sha256,
        begin.begin_hex,&begin_hash,begin.begin_receipt_hex,begin.mode.as_str(),begin.proof_family,
        begin.source_hex,begin.source_digest,begin.pairs_hex,begin.pairs_digest,
        begin.copy_fingerprint,begin.source_fingerprint,begin.copy_fingerprint,begin.source_fingerprint];
    for (index,value) in values.iter().enumerate() { insert.bind_text((index + 1) as i32,value)?; }
    insert.step_done()?;
    Ok(BeginOutcome::Created(read(db, root, begin.request_id)?.ok_or(ActionError::Conflict)?))
}

/// Seal the copy UPGRADING write made after action creation in this same
/// transaction. The immutable initial pair remains the pre-begin snapshot.
pub(crate) fn record_begin_effect_in_current_transaction(
    db: &VerifiedDatabaseConnection<'_>, root: &RootLock, owner: &OwnerIssuer,
    request_id: &str, expected_revision: i64,
    expected_copy_fingerprint: &str, expected_source_fingerprint: &str,
    next_copy_fingerprint: &str, next_source_fingerprint: &str,
) -> Result<ActionRecord> {
    mutation_authority(db, root, owner)?;
    if !bounded_id(expected_copy_fingerprint) || !bounded_id(expected_source_fingerprint) ||
        !bounded_id(next_copy_fingerprint) || !bounded_id(next_source_fingerprint) ||
        next_copy_fingerprint == expected_copy_fingerprint { return Err(ActionError::Invalid("begin effect fingerprints")); }
    let row = read(db, root, request_id)?.ok_or(ActionError::Missing)?;
    if row.phase == ActionPhase::Begun && row.revision == expected_revision + 1 && expected_revision == 1 &&
        row.initial_copy_fingerprint == expected_copy_fingerprint &&
        row.initial_source_fingerprint == expected_source_fingerprint &&
        row.begin_effect_copy_fingerprint == next_copy_fingerprint &&
        row.begin_effect_source_fingerprint == next_source_fingerprint &&
        row.current_copy_fingerprint == next_copy_fingerprint &&
        row.current_source_fingerprint == next_source_fingerprint { return Ok(row); }
    if row.phase != ActionPhase::Begun || row.revision != expected_revision || expected_revision != 1 ||
        row.current_copy_fingerprint != expected_copy_fingerprint ||
        row.current_source_fingerprint != expected_source_fingerprint ||
        row.initial_copy_fingerprint != expected_copy_fingerprint ||
        row.initial_source_fingerprint != expected_source_fingerprint { return Err(ActionError::Conflict); }
    let q = Statement::prepare(db.as_ptr(), "UPDATE main.gogoke_v37_instance_version_actions SET begin_effect_copy_fingerprint=?1,begin_effect_source_fingerprint=?2,current_copy_fingerprint=?1,current_source_fingerprint=?2,revision=revision+1 WHERE request_id=?3 AND revision=1 AND phase='BEGUN' AND current_copy_fingerprint=?4 AND current_source_fingerprint=?5")?;
    q.bind_text(1,next_copy_fingerprint)?; q.bind_text(2,next_source_fingerprint)?;
    q.bind_text(3,request_id)?; q.bind_text(4,expected_copy_fingerprint)?;
    q.bind_text(5,expected_source_fingerprint)?;
    q.step_done()?; affected_one(db)?;
    read(db, root, request_id)?.ok_or(ActionError::Conflict)
}

/// Persist the exact probe attempt before any H launch. Only `Created` may be
/// followed by that one physical attempt. A `Replayed` intent is read only;
/// its caller must inspect the original C/probe facts or keep the action busy.
pub(crate) fn begin_probe_in_current_transaction(
    db: &VerifiedDatabaseConnection<'_>, root: &RootLock, owner: &OwnerIssuer,
    request_id: &str, expected_revision: i64, exact_probe_hex: &str, stage_name: &str,
    expected_copy_fingerprint: &str, expected_source_fingerprint: &str,
) -> Result<BeginOutcome> {
    mutation_authority(db, root, owner)?;
    if bytes(exact_probe_hex)?.is_empty() || !bounded_id(stage_name) ||
        !bounded_id(expected_copy_fingerprint) || !bounded_id(expected_source_fingerprint) {
        return Err(ActionError::Invalid("probe intent"));
    }
    let row = read(db, root, request_id)?.ok_or(ActionError::Missing)?;
    if !row.original_error_hex.is_empty() { return Err(ActionError::Conflict); }
    if row.stage_name != stage_name || row.current_copy_fingerprint != expected_copy_fingerprint ||
        row.current_source_fingerprint != expected_source_fingerprint { return Err(ActionError::Conflict); }
    if row.phase == ActionPhase::Probing && row.revision == expected_revision + 1 &&
        row.probe_hex == exact_probe_hex { return Ok(BeginOutcome::Replayed(row)); }
    if row.phase != ActionPhase::Staged || row.revision != expected_revision { return Err(ActionError::Conflict); }
    let q = Statement::prepare(db.as_ptr(), "UPDATE main.gogoke_v37_instance_version_actions SET probe_hex=?1,phase='PROBING',revision=revision+1 WHERE request_id=?2 AND revision=?3 AND phase='STAGED' AND stage_name=?4 AND current_copy_fingerprint=?5 AND current_source_fingerprint=?6")?;
    q.bind_text(1,exact_probe_hex)?; q.bind_text(2,request_id)?;
    q.bind_i64(3,expected_revision)?; q.bind_text(4,stage_name)?;
    q.bind_text(5,expected_copy_fingerprint)?; q.bind_text(6,expected_source_fingerprint)?;
    q.step_done()?; affected_one(db)?;
    Ok(BeginOutcome::Created(read(db, root, request_id)?.ok_or(ActionError::Conflict)?))
}

/// Record an original failure and its copy state observation without releasing
/// the open driver. UNKNOWN remains at its open phase; a confirmed terminal
/// failure may call `fail` in this same transaction.
pub(crate) fn record_error_in_current_transaction(
    db: &VerifiedDatabaseConnection<'_>, root: &RootLock, owner: &OwnerIssuer,
    request_id: &str, expected_revision: i64, original_error_hex: &str,
    expected_copy_fingerprint: &str, expected_source_fingerprint: &str,
    next_copy_fingerprint: &str, next_source_fingerprint: &str,
) -> Result<ActionRecord> {
    mutation_authority(db, root, owner)?;
    if bytes(original_error_hex)?.is_empty() || !bounded_id(expected_copy_fingerprint) ||
        !bounded_id(expected_source_fingerprint) || !bounded_id(next_copy_fingerprint) ||
        !bounded_id(next_source_fingerprint) || next_copy_fingerprint == expected_copy_fingerprint {
        return Err(ActionError::Invalid("version action error observation"));
    }
    let row = read(db, root, request_id)?.ok_or(ActionError::Missing)?;
    let prior_pair = match row.phase {
        ActionPhase::Begun if !row.begin_effect_copy_fingerprint.is_empty() =>
            (&row.begin_effect_copy_fingerprint, &row.begin_effect_source_fingerprint),
        ActionPhase::Begun => (&row.initial_copy_fingerprint, &row.initial_source_fingerprint),
        ActionPhase::Staged | ActionPhase::Probing =>
            (&row.stage_copy_fingerprint, &row.stage_source_fingerprint),
        ActionPhase::Probed => (&row.probe_copy_fingerprint, &row.probe_source_fingerprint),
        _ => return Err(ActionError::Conflict),
    };
    if row.revision == expected_revision + 1 &&
        row.original_error_hex == original_error_hex &&
        prior_pair.0 == expected_copy_fingerprint &&
        prior_pair.1 == expected_source_fingerprint &&
        row.error_copy_fingerprint == next_copy_fingerprint &&
        row.error_source_fingerprint == next_source_fingerprint &&
        row.current_copy_fingerprint == next_copy_fingerprint &&
        row.current_source_fingerprint == next_source_fingerprint { return Ok(row); }
    if row.revision != expected_revision ||
        !row.original_error_hex.is_empty() ||
        row.current_copy_fingerprint != expected_copy_fingerprint ||
        row.current_source_fingerprint != expected_source_fingerprint ||
        prior_pair.0 != expected_copy_fingerprint ||
        prior_pair.1 != expected_source_fingerprint { return Err(ActionError::Conflict); }
    let q = Statement::prepare(db.as_ptr(), "UPDATE main.gogoke_v37_instance_version_actions SET original_error_hex=?1,error_copy_fingerprint=?2,error_source_fingerprint=?3,current_copy_fingerprint=?2,current_source_fingerprint=?3,revision=revision+1 WHERE request_id=?4 AND revision=?5 AND phase=?8 AND original_error_hex='' AND current_copy_fingerprint=?6 AND current_source_fingerprint=?7")?;
    q.bind_text(1,original_error_hex)?; q.bind_text(2,next_copy_fingerprint)?;
    q.bind_text(3,next_source_fingerprint)?; q.bind_text(4,request_id)?;
    q.bind_i64(5,expected_revision)?; q.bind_text(6,expected_copy_fingerprint)?;
    q.bind_text(7,expected_source_fingerprint)?;
    q.bind_text(8,row.phase.as_str())?;
    q.step_done()?; affected_one(db)?;
    read(db, root, request_id)?.ok_or(ActionError::Conflict)
}

pub(crate) use record_error_in_current_transaction as record_probe_error_in_current_transaction;

pub(crate) fn advance_in_current_transaction(
    db: &VerifiedDatabaseConnection<'_>, root: &RootLock, owner: &OwnerIssuer,
    request_id: &str, expected_revision: i64, step: &ActionStep<'_>,
) -> Result<ActionRecord> {
    mutation_authority(db, root, owner)?;
    let row = read(db, root, request_id)?.ok_or(ActionError::Missing)?;
    if bytes(step.raw_hex)?.is_empty() || bytes(step.receipt_hex)?.is_empty() ||
        !bounded_id(step.expected_copy_fingerprint) || !bounded_id(step.expected_source_fingerprint) ||
        !bounded_id(step.next_copy_fingerprint) || !bounded_id(step.next_source_fingerprint) {
        return Err(ActionError::Invalid("step bytes/fingerprint"));
    }
    let (prior, old_raw, old_receipt) = match step.next_phase {
        ActionPhase::Staged if bounded_id(step.stage_name) => (ActionPhase::Begun, &row.stage_hex, &row.stage_receipt_hex),
        ActionPhase::Probed if step.stage_name == row.stage_name && !row.stage_name.is_empty() =>
            (ActionPhase::Probing, &row.probe_hex, &row.probe_receipt_hex),
        _ => return Err(ActionError::Invalid("step phase or stage")),
    };
    let replay_expected = if step.next_phase == ActionPhase::Staged {
        (&row.begin_effect_copy_fingerprint, &row.begin_effect_source_fingerprint)
    } else if !row.error_copy_fingerprint.is_empty() &&
        row.current_copy_fingerprint == row.probe_copy_fingerprint &&
        row.current_source_fingerprint == row.probe_source_fingerprint {
        (&row.error_copy_fingerprint, &row.error_source_fingerprint)
    } else { (&row.stage_copy_fingerprint, &row.stage_source_fingerprint) };
    if row.phase == step.next_phase && row.revision == expected_revision + 1 &&
        old_raw == step.raw_hex && old_receipt == step.receipt_hex &&
        replay_expected.0 == step.expected_copy_fingerprint &&
        replay_expected.1 == step.expected_source_fingerprint &&
        row.current_copy_fingerprint == step.next_copy_fingerprint &&
        row.current_source_fingerprint == step.next_source_fingerprint &&
        row.stage_name == step.stage_name { return Ok(row); }
    if row.phase != prior || row.revision != expected_revision ||
        row.current_copy_fingerprint != step.expected_copy_fingerprint ||
        row.current_source_fingerprint != step.expected_source_fingerprint {
        return Err(ActionError::Conflict);
    }
    if step.next_phase == ActionPhase::Staged && !row.original_error_hex.is_empty() {
        return Err(ActionError::Conflict);
    }
    if step.next_phase == ActionPhase::Staged && row.begin_effect_copy_fingerprint.is_empty() {
        return Err(ActionError::Conflict);
    }
    if step.next_phase == ActionPhase::Probed && row.probe_hex != step.raw_hex {
        return Err(ActionError::Conflict);
    }
    let sql = match step.next_phase {
        ActionPhase::Staged => "UPDATE main.gogoke_v37_instance_version_actions SET stage_name=?1,stage_hex=?2,stage_receipt_hex=?3,current_copy_fingerprint=?4,current_source_fingerprint=?5,stage_copy_fingerprint=?4,stage_source_fingerprint=?5,phase='STAGED',revision=revision+1 WHERE request_id=?6 AND revision=?7 AND phase='BEGUN' AND current_copy_fingerprint=?8 AND current_source_fingerprint=?9",
        ActionPhase::Probed => "UPDATE main.gogoke_v37_instance_version_actions SET stage_name=?1,probe_hex=?2,probe_receipt_hex=?3,current_copy_fingerprint=?4,current_source_fingerprint=?5,probe_copy_fingerprint=?4,probe_source_fingerprint=?5,phase='PROBED',revision=revision+1 WHERE request_id=?6 AND revision=?7 AND phase='PROBING' AND probe_hex=?2 AND current_copy_fingerprint=?8 AND current_source_fingerprint=?9",
        _ => unreachable!(),
    };
    let q = Statement::prepare(db.as_ptr(), sql)?;
    for (index,value) in [step.stage_name,step.raw_hex,step.receipt_hex,
        step.next_copy_fingerprint,step.next_source_fingerprint,request_id].iter().enumerate() {
        q.bind_text((index + 1) as i32,value)?;
    }
    q.bind_i64(7,expected_revision)?;
    q.bind_text(8,step.expected_copy_fingerprint)?;
    q.bind_text(9,step.expected_source_fingerprint)?;
    q.step_done()?; affected_one(db)?;
    read(db, root, request_id)?.ok_or(ActionError::Conflict)
}

pub(crate) fn fail_in_current_transaction(
    db: &VerifiedDatabaseConnection<'_>, root: &RootLock, owner: &OwnerIssuer,
    request_id: &str, expected_revision: i64, original_error_hex: &str,
) -> Result<ActionRecord> {
    mutation_authority(db, root, owner)?;
    if bytes(original_error_hex)?.is_empty() { return Err(ActionError::Invalid("original failure bytes")); }
    let row = read(db, root, request_id)?.ok_or(ActionError::Missing)?;
    if row.phase == ActionPhase::Failed && row.revision == expected_revision + 1 &&
        row.original_error_hex == original_error_hex { return Ok(row); }
    if !matches!(row.phase, ActionPhase::Begun | ActionPhase::Staged | ActionPhase::Probing | ActionPhase::Probed) ||
        row.revision != expected_revision ||
        (!row.original_error_hex.is_empty() && row.original_error_hex != original_error_hex) {
        return Err(ActionError::Conflict);
    }
    let q = Statement::prepare(db.as_ptr(), "UPDATE main.gogoke_v37_instance_version_actions SET open_driver_id=NULL,original_error_hex=?1,phase='FAILED',revision=revision+1 WHERE request_id=?2 AND revision=?3 AND phase=?4")?;
    q.bind_text(1, original_error_hex)?; q.bind_text(2, request_id)?;
    q.bind_i64(3, expected_revision)?; q.bind_text(4, row.phase.as_str())?;
    q.step_done()?; affected_one(db)?;
    read(db, root, request_id)?.ok_or(ActionError::Conflict)
}

/// The caller has already written and checked its original switch receipts in
/// this IMMEDIATE transaction. This only seals their exact set digest and the
/// caller-observed post-switch copy/source fingerprints in the same commit.
pub(crate) fn consume_in_current_transaction(
    db: &VerifiedDatabaseConnection<'_>, root: &RootLock, owner: &OwnerIssuer,
    request_id: &str, expected_revision: i64, final_hex: &str,
    final_receipt_set_digest: &str, expected_copy_fingerprint: &str,
    expected_source_fingerprint: &str, next_copy_fingerprint: &str,
    next_source_fingerprint: &str,
) -> Result<ActionRecord> {
    mutation_authority(db, root, owner)?;
    if bytes(final_hex)?.is_empty() || !digest(final_receipt_set_digest) ||
        !bounded_id(expected_copy_fingerprint) || !bounded_id(expected_source_fingerprint) ||
        !bounded_id(next_copy_fingerprint) || !bounded_id(next_source_fingerprint) {
        return Err(ActionError::Invalid("final bytes/receipt set/fingerprint"));
    }
    let row = read(db, root, request_id)?.ok_or(ActionError::Missing)?;
    let final_hash = content_hash(&bytes(final_hex)?);
    if row.phase == ActionPhase::Consumed && row.revision == expected_revision + 1 &&
        row.final_hex == final_hex && row.final_hash == final_hash &&
        row.final_receipt_set_digest == final_receipt_set_digest &&
        row.final_pre_copy_fingerprint == expected_copy_fingerprint &&
        row.final_pre_source_fingerprint == expected_source_fingerprint &&
        row.current_copy_fingerprint == next_copy_fingerprint &&
        row.current_source_fingerprint == next_source_fingerprint { return Ok(row); }
    if row.phase != ActionPhase::Probed || row.revision != expected_revision ||
        row.current_copy_fingerprint != expected_copy_fingerprint ||
        row.current_source_fingerprint != expected_source_fingerprint { return Err(ActionError::Conflict); }
    let q = Statement::prepare(db.as_ptr(), "UPDATE main.gogoke_v37_instance_version_actions SET open_driver_id=NULL,final_pre_copy_fingerprint=?1,final_pre_source_fingerprint=?2,final_hex=?3,final_hash=?4,final_receipt_set_digest=?5,current_copy_fingerprint=?6,current_source_fingerprint=?7,phase='CONSUMED',revision=revision+1 WHERE request_id=?8 AND revision=?9 AND phase='PROBED' AND current_copy_fingerprint=?10 AND current_source_fingerprint=?11")?;
    for (index,value) in [expected_copy_fingerprint,expected_source_fingerprint,final_hex,&final_hash,
        final_receipt_set_digest,next_copy_fingerprint,next_source_fingerprint,request_id].iter().enumerate() {
        q.bind_text((index+1) as i32,value)?;
    }
    q.bind_i64(9,expected_revision)?;
    q.bind_text(10,expected_copy_fingerprint)?; q.bind_text(11,expected_source_fingerprint)?;
    q.step_done()?; affected_one(db)?;
    read(db, root, request_id)?.ok_or(ActionError::Conflict)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::store::same_open::{create_new, route_b_test_guard};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture(run: impl FnOnce(&RootLock, &mut VerifiedDatabaseConnection<'_>, &OwnerIssuer)) {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("gogoke-version-action-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut db = create_new(&root, &path.join("state.sqlite")).unwrap();
        super::super::initialize_schema(&mut db).unwrap();
        let owner = crate::store::authority::initialize_profile(&mut db, &root).unwrap();
        run(&root, &mut db, &owner);
        db.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }

    fn begin<'a>(request_id: &'a str) -> ActionBegin<'a> {
        ActionBegin { request_id, driver: "codex", operation: ActionOperation::Upgrade,
            target_version: "0.162.1", begin_hex: "7b7d", begin_receipt_hex: "aa",
            mode: ActionMode::Strict, proof_family: "", source_hex: "", source_digest: "",
            pairs_hex: "", pairs_digest: "", copy_fingerprint: "copy-1",
            source_fingerprint: "source-1" }
    }
    fn created(value: BeginOutcome) -> ActionRecord {
        match value { BeginOutcome::Created(row) => row, BeginOutcome::Replayed(_) => panic!("unexpected replay") }
    }

    #[test]
    fn one_open_per_driver_exact_replay_and_tamper_rejection() {
        fixture(|root, db, owner| {
            db.execute("BEGIN IMMEDIATE").unwrap();
            let original = created(begin_in_current_transaction(db, root, owner, &begin("req-1")).unwrap());
            assert_eq!(original.phase, ActionPhase::Begun);
            assert!(matches!(begin_in_current_transaction(db, root, owner, &begin("req-1")).unwrap(), BeginOutcome::Replayed(_)));
            let mut tampered = begin("req-1"); tampered.begin_hex = "7b20";
            assert!(matches!(begin_in_current_transaction(db, root, owner, &tampered), Err(ActionError::Conflict)));
            assert!(matches!(begin_in_current_transaction(db, root, owner, &begin("req-2")), Err(ActionError::Busy)));
            db.execute("COMMIT").unwrap();
            assert_eq!(read(db, root, "req-1").unwrap().unwrap().begin_hex, "7b7d");
        });
    }

    #[test]
    fn staged_probed_consume_chain_and_transaction_rollback() {
        fixture(|root, db, owner| {
            db.execute("BEGIN IMMEDIATE").unwrap();
            let begun = created(begin_in_current_transaction(db, root, owner, &begin("req-1")).unwrap());
            assert!(matches!(advance_in_current_transaction(db, root, owner, "req-1", begun.revision,
                &ActionStep { next_phase: ActionPhase::Staged, raw_hex: "01", receipt_hex: "02",
                    stage_name: "stage-1", expected_copy_fingerprint: "copy-1", expected_source_fingerprint: "source-1",
                    next_copy_fingerprint: "copy-2", next_source_fingerprint: "source-1" }), Err(ActionError::Conflict)));
            let begun_effect = record_begin_effect_in_current_transaction(db, root, owner, "req-1", begun.revision,
                "copy-1", "source-1", "copy-1b", "source-1").unwrap();
            assert_eq!(begun_effect.phase, ActionPhase::Begun);
            assert_eq!(begun_effect.initial_copy_fingerprint, "copy-1");
            assert!(matches!(record_begin_effect_in_current_transaction(db, root, owner, "req-1", begun.revision,
                "copy-1", "source-1", "copy-1b", "source-1"), Ok(_)));
            let stage = ActionStep { next_phase: ActionPhase::Staged, raw_hex: "01", receipt_hex: "02",
                stage_name: "stage-1", expected_copy_fingerprint: "copy-1b", expected_source_fingerprint: "source-1",
                next_copy_fingerprint: "copy-2", next_source_fingerprint: "source-1" };
            let staged = advance_in_current_transaction(db, root, owner, "req-1", begun_effect.revision, &stage).unwrap();
            assert_eq!(staged.phase, ActionPhase::Staged);
            assert!(matches!(advance_in_current_transaction(db, root, owner, "req-1", begun_effect.revision, &stage), Ok(_)));
            let changed_pre_stage = ActionStep { expected_copy_fingerprint: "changed", ..stage };
            assert!(matches!(advance_in_current_transaction(db, root, owner, "req-1", begun_effect.revision, &changed_pre_stage), Err(ActionError::Conflict)));
            let intent = match begin_probe_in_current_transaction(db, root, owner, "req-1", staged.revision,
                "04", "stage-1", "copy-2", "source-1").unwrap() {
                BeginOutcome::Created(row) => row, BeginOutcome::Replayed(_) => panic!("unexpected probe replay"),
            };
            assert_eq!(intent.phase, ActionPhase::Probing);
            assert!(matches!(begin_probe_in_current_transaction(db, root, owner, "req-1", staged.revision,
                "04", "stage-1", "copy-2", "source-1"), Ok(BeginOutcome::Replayed(_))));
            assert!(matches!(begin_probe_in_current_transaction(db, root, owner, "req-1", staged.revision,
                "06", "stage-1", "copy-2", "source-1"), Err(ActionError::Conflict)));
            assert!(matches!(consume_in_current_transaction(db, root, owner, "req-1", staged.revision,
                "03", &"a".repeat(64), "copy-2", "source-1", "copy-4", "source-2"), Err(ActionError::Conflict)));
            let probe = ActionStep { next_phase: ActionPhase::Probed, raw_hex: "04", receipt_hex: "05",
                stage_name: "stage-1", expected_copy_fingerprint: "copy-2", expected_source_fingerprint: "source-1",
                next_copy_fingerprint: "copy-3", next_source_fingerprint: "source-1" };
            let probed = advance_in_current_transaction(db, root, owner, "req-1", intent.revision, &probe).unwrap();
            assert!(matches!(advance_in_current_transaction(db, root, owner, "req-1", intent.revision, &probe), Ok(_)));
            let consumed = consume_in_current_transaction(db, root, owner, "req-1", probed.revision,
                "03", &"a".repeat(64), "copy-3", "source-1", "copy-4", "source-2").unwrap();
            assert_eq!(consumed.phase, ActionPhase::Consumed);
            assert!(consumed.open_driver_id.is_empty());
            assert!(matches!(consume_in_current_transaction(db, root, owner, "req-1", probed.revision,
                "03", &"a".repeat(64), "copy-3", "source-1", "copy-4", "source-2"), Ok(_)));
            db.execute("ROLLBACK").unwrap();
            assert!(read(db, root, "req-1").unwrap().is_none());
        });
    }

    #[test]
    fn failed_and_consumed_ids_cannot_start_new_effects() {
        fixture(|root, db, owner| {
            db.execute("BEGIN IMMEDIATE").unwrap();
            let begun = created(begin_in_current_transaction(db, root, owner, &begin("req-fail")).unwrap());
            let failed = fail_in_current_transaction(db, root, owner, "req-fail", begun.revision, "4551").unwrap();
            assert_eq!(failed.phase, ActionPhase::Failed);
            assert!(matches!(fail_in_current_transaction(db, root, owner, "req-fail", begun.revision, "4551"), Ok(_)));
            assert!(matches!(advance_in_current_transaction(db, root, owner, "req-fail", failed.revision,
                &ActionStep { next_phase: ActionPhase::Staged, raw_hex: "01", receipt_hex: "02",
                    stage_name: "stage-1", expected_copy_fingerprint: "copy-1", expected_source_fingerprint: "source-1",
                    next_copy_fingerprint: "copy-2", next_source_fingerprint: "source-1" }), Err(ActionError::Conflict)));
            assert!(matches!(begin_in_current_transaction(db, root, owner, &begin("req-next")), Ok(BeginOutcome::Created(_))));
            db.execute("COMMIT").unwrap();
        });
    }

    #[test]
    fn cold_snapshot_digest_and_journal_tamper_are_rejected() {
        fixture(|root, db, owner| {
            let mut cold = begin("req-cold");
            cold.mode = ActionMode::Cold;
            cold.proof_family = "exact-gone";
            cold.source_hex = "6162";
            cold.source_digest = "sha256:fb8e20fc2e4c3f248c60c39bd652f3c1347298bb977b8b4d5903b85055620603";
            cold.pairs_hex = "0102";
            cold.pairs_digest = "sha256:a12871fee210fb8619291eaea194581cbd2531e4b23759d225f6806923f63222";
            db.execute("BEGIN IMMEDIATE").unwrap();
            let row = created(begin_in_current_transaction(db, root, owner, &cold).unwrap());
            assert_eq!(row.mode, "COLD");
            db.execute("COMMIT").unwrap();
            db.execute("UPDATE main.gogoke_v37_instance_version_actions SET begin_hash='sha256:tampered' WHERE request_id='req-cold'").unwrap();
            assert!(matches!(read(db, root, "req-cold"), Err(ActionError::Conflict)));
        });
    }

    #[test]
    fn probe_unknown_preserves_original_error_and_blocks_second_attempt() {
        fixture(|root, db, owner| {
            db.execute("BEGIN IMMEDIATE").unwrap();
            let begun = created(begin_in_current_transaction(db, root, owner, &begin("req-unknown")).unwrap());
            let progressed = record_begin_effect_in_current_transaction(db, root, owner, "req-unknown", begun.revision,
                "copy-1", "source-1", "copy-upgrading", "source-1").unwrap();
            let staged = advance_in_current_transaction(db, root, owner, "req-unknown", progressed.revision,
                &ActionStep { next_phase: ActionPhase::Staged, raw_hex: "01", receipt_hex: "02", stage_name: "stage-1",
                    expected_copy_fingerprint: "copy-upgrading", expected_source_fingerprint: "source-1",
                    next_copy_fingerprint: "copy-staged", next_source_fingerprint: "source-1" }).unwrap();
            let intent = created(begin_probe_in_current_transaction(db, root, owner, "req-unknown", staged.revision,
                "03", "stage-1", "copy-staged", "source-1").unwrap());
            let unknown = record_probe_error_in_current_transaction(db, root, owner, "req-unknown", intent.revision,
                "4551", "copy-staged", "source-1", "copy-unknown", "source-1").unwrap();
            assert_eq!(unknown.phase, ActionPhase::Probing);
            assert_eq!(unknown.original_error_hex, "4551");
            assert_eq!(unknown.open_driver_id, "codex");
            assert!(matches!(record_probe_error_in_current_transaction(db, root, owner, "req-unknown", intent.revision,
                "4551", "copy-staged", "source-1", "copy-unknown", "source-1"), Ok(_)));
            assert!(matches!(begin_probe_in_current_transaction(db, root, owner, "req-unknown", staged.revision,
                "03", "stage-1", "copy-staged", "source-1"), Err(ActionError::Conflict)));
            assert!(matches!(begin_in_current_transaction(db, root, owner, &begin("req-next")), Err(ActionError::Busy)));
            assert!(matches!(fail_in_current_transaction(db, root, owner, "req-unknown", unknown.revision, "dead"), Err(ActionError::Conflict)));
            let failed = fail_in_current_transaction(db, root, owner, "req-unknown", unknown.revision, "4551").unwrap();
            assert_eq!(failed.phase, ActionPhase::Failed);
            db.execute("ROLLBACK").unwrap();
            assert!(read(db, root, "req-unknown").unwrap().is_none());
        });
    }

    #[test]
    fn begin_and_stage_errors_keep_phase_and_cas_chain() {
        fixture(|root, db, owner| {
            db.execute("BEGIN IMMEDIATE").unwrap();
            let begun = created(begin_in_current_transaction(db, root, owner, &begin("req-begin-error")).unwrap());
            let observed = record_error_in_current_transaction(db, root, owner, "req-begin-error", begun.revision,
                "aa", "copy-1", "source-1", "copy-begin-error", "source-1").unwrap();
            assert_eq!(observed.phase, ActionPhase::Begun);
            assert_eq!(observed.current_copy_fingerprint, "copy-begin-error");
            assert!(matches!(record_begin_effect_in_current_transaction(db, root, owner, "req-begin-error",
                begun.revision, "copy-1", "source-1", "copy-upgrading", "source-1"), Err(ActionError::Conflict)));
            fail_in_current_transaction(db, root, owner, "req-begin-error", observed.revision, "aa").unwrap();

            let second = created(begin_in_current_transaction(db, root, owner, &begin("req-stage-error")).unwrap());
            let progress = record_begin_effect_in_current_transaction(db, root, owner, "req-stage-error", second.revision,
                "copy-1", "source-1", "copy-upgrading", "source-1").unwrap();
            let staged = advance_in_current_transaction(db, root, owner, "req-stage-error", progress.revision,
                &ActionStep { next_phase: ActionPhase::Staged, raw_hex: "01", receipt_hex: "02", stage_name: "stage-1",
                    expected_copy_fingerprint: "copy-upgrading", expected_source_fingerprint: "source-1",
                    next_copy_fingerprint: "copy-staged", next_source_fingerprint: "source-1" }).unwrap();
            let observed = record_error_in_current_transaction(db, root, owner, "req-stage-error", staged.revision,
                "bb", "copy-staged", "source-1", "copy-stage-error", "source-1").unwrap();
            assert_eq!(observed.phase, ActionPhase::Staged);
            assert!(matches!(begin_probe_in_current_transaction(db, root, owner, "req-stage-error", staged.revision,
                "03", "stage-1", "copy-stage-error", "source-1"), Err(ActionError::Conflict)));
            fail_in_current_transaction(db, root, owner, "req-stage-error", observed.revision, "bb").unwrap();
            db.execute("COMMIT").unwrap();
        });
    }
}
