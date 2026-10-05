//! The stopped child supplies files, never Git authority. Hold every ordinary
//! object without write/delete sharing before Git can read it as host.user.
use super::*;
use crate::store::atomic::Parser;
use std::collections::BTreeMap;
use std::io::{Seek, SeekFrom};

#[repr(C)]
struct FileInfo {
    attributes: u32, times: [u32; 6], volume_serial: u32,
    size_high: u32, size_low: u32, links: u32, index_high: u32, index_low: u32,
}
#[link(name = "kernel32")]
extern "system" {
    fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, output: *mut FileInfo) -> i32;
}

fn held_metadata(path: &Path) -> Result<File> {
    // OPEN_REPARSE_POINT applies even if the name changes between lstat/open.
    let file = OpenOptions::new().access_mode(0x80).share_mode(1)
        .custom_flags(0x0020_0000 | 0x0200_0000).open(path)?;
    let mut info: FileInfo = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error().into());
    }
    if info.attributes & REPARSE_POINT != 0 ||
        (info.attributes & 0x10 == 0 && info.links != 1) {
        return Err(WorktreeError::Denied);
    }
    Ok(file)
}

pub(super) fn hold_directory(path: &Path) -> Result<File> {
    let file=held_metadata(path)?;
    if !file.metadata()?.is_dir() { return Err(WorktreeError::Denied); }
    Ok(file)
}

struct Entry { path: PathBuf, metadata: File, data: Option<File>, fact: String }
pub(super) struct Snapshot { entries: Vec<Entry>, pub(super) digest: String }

fn entry_fact(metadata: &File, data: Option<&mut File>) -> Result<String> {
    let m = metadata.metadata()?;
    let id = file_identity(metadata)?.opaque();
    let hash = match data {
        Some(file) => {
            file.seek(SeekFrom::Start(0))?;
            let mut bytes = Vec::new(); file.read_to_end(&mut bytes)?;
            if bytes.len() as u64 != m.len() { return Err(WorktreeError::Denied); }
            sha256_hex(&bytes)
        },
        None => String::new(),
    };
    Ok(format!("{id}:{}:{}:{}:{}:{hash}", m.file_attributes(), m.len(),
        m.creation_time(), m.last_write_time()))
}

impl Snapshot {
    pub(super) fn capture(path: &Path) -> Result<Self> {
        let mut entries = Vec::new();
        Self::walk(path, path, &mut entries)?;
        let digest = sha256_hex(&entries.iter().map(|e| format!("{}\0{}\n",
            e.path.strip_prefix(path).expect("walk root").to_string_lossy(), e.fact))
            .collect::<String>().into_bytes());
        Ok(Self { entries, digest })
    }
    fn walk(root: &Path, path: &Path, entries: &mut Vec<Entry>) -> Result<()> {
        let metadata = held_metadata(path)?;
        let directory = metadata.metadata()?.is_dir();
        if fs::canonicalize(path)? != path { return Err(WorktreeError::Denied); }
        let mut data = if directory { None } else {
            let file = OpenOptions::new().read(true).share_mode(1)
                .custom_flags(0x0020_0000).open(path)?;
            if file_identity(&file)? != file_identity(&metadata)? { return Err(WorktreeError::Denied); }
            Some(file)
        };
        let fact = entry_fact(&metadata, data.as_mut())?;
        entries.push(Entry { path: path.to_owned(), metadata, data, fact });
        if directory {
            let mut children = fs::read_dir(path)?.map(|entry| entry.map(|e| e.path()))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            children.sort();
            for child in children {
                let name = child.file_name().and_then(|v| v.to_str()).ok_or(WorktreeError::Denied)?;
                // The original .git pointer is already held by ResolvedBinding.
                if path == root && name == ".git" { continue; }
                if name.eq_ignore_ascii_case(".git") || name.eq_ignore_ascii_case(".gitattributes") ||
                    name.eq_ignore_ascii_case(".gitmodules") || name.contains(':') ||
                    name.contains('\\') || name.chars().any(char::is_control) {
                    return Err(WorktreeError::Denied);
                }
                Self::walk(root, &child, entries)?;
            }
        }
        Ok(())
    }
    pub(super) fn recheck(&mut self, path: &Path) -> Result<()> {
        for entry in &mut self.entries {
            if entry_fact(&entry.metadata, entry.data.as_mut())? != entry.fact ||
                file_identity(&held_metadata(&entry.path)?)? != file_identity(&entry.metadata)? {
                return Err(WorktreeError::Denied);
            }
        }
        // A directory handle prevents replacement, but not insertion of a new
        // name. Re-enumerate the whole root, including ignored/untracked names.
        if Self::capture(path)?.digest != self.digest { return Err(WorktreeError::Denied); }
        Ok(())
    }
}

pub(super) fn ordinary_git_entries(text: &str, index: bool) -> Result<()> {
    for line in text.lines() {
        let (header, path) = line.split_once('\t').ok_or(WorktreeError::Denied)?;
        let parts: Vec<_> = header.split_whitespace().collect();
        if parts.len() != 3 || !matches!(parts[0], "100644" | "100755") ||
            (index && parts[2] != "0") || (!index && parts[1] != "blob") ||
            !hex_commit(parts[if index { 1 } else { 2 }]) || path.is_empty() ||
            path.starts_with('"') || path.contains('\\') || path.contains(':') ||
            path.split('/').any(|p| p.is_empty() || p == "." || p == ".." ||
                p.eq_ignore_ascii_case(".git") || p.eq_ignore_ascii_case(".gitattributes") ||
                p.eq_ignore_ascii_case(".gitmodules")) {
            return Err(WorktreeError::Denied);
        }
    }
    Ok(())
}

fn object(fields: impl IntoIterator<Item = (&'static str, String)>) -> Json {
    Json::Object(fields.into_iter().map(|(k,v)| (JsonString::from_str(k),
        Json::String(JsonString::from_str(&v)))).collect())
}

pub(super) fn intent(request: &crate::store::session_transport::V37Request,
    repository: &str, seat: &str, instance: &str, turn: &str, binding: &ResolvedBinding,
    source_before: &str, child_before: &str, snapshot: &str, changed: bool,
    stops: &[ExactStopFact]) -> String {
    object([
        ("schema", "gogoke.37.child-seal-intent.v1".into()),
        ("requestId", request.request_id.clone()), ("requestHash", sha256_hex(&request.raw_bytes)),
        ("domainId", request.domain_id.clone()), ("worktreeId", request.target_id.clone()),
        ("repositoryId", repository.into()), ("seatId", seat.into()), ("instanceId", instance.into()),
        ("turnId", turn.into()), ("worktreeIdentity", binding.identity.opaque()),
        ("pointerHash", binding.pointer_hash.clone()), ("commonIdentity", binding.common_identity.opaque()),
        ("baselineCommit", binding.baseline_commit.clone()), ("sourceBefore", source_before.into()),
        ("childBefore", child_before.into()), ("snapshotHash", snapshot.into()),
        ("changed", if changed { "true" } else { "false" }.into()),
        ("stopFacts", Json::Array(stops.iter().map(|s| object([
            ("processOperationId", s.process_operation_id.clone()), ("stopFactId", s.stop_fact_id.clone())
        ])).collect()).canonical()),
    ]).canonical()
}

// Keep child evidence inside the existing cause column, without changing wire
// operations or a table. Legacy APPLIED records have no invented child data.
pub(super) fn record(merge: &str, intent: &str, child: Option<&str>, error: Option<&str>) -> String {
    object([("schema", "gogoke.37.worktree-merge-result.v2".into()),
        ("mergeReceipt", merge.into()), ("childSealIntent", intent.into()),
        ("childCommit", child.unwrap_or("").into()), ("error", error.unwrap_or("").into())]).canonical()
}

pub(super) fn read_record(record: &str, merge: &str,
    request: &crate::store::session_transport::V37Request) -> Result<(String, String)> {
    let Json::Object(fields) = Parser::parse(record)? else { return Err(WorktreeError::Unknown); };
    fn get(fields: &BTreeMap<JsonString, Json>, key: &str) -> Result<String> {
        match fields.get(&JsonString::from_str(key)) {
            Some(Json::String(s)) => s.to_well_formed_string().ok_or(WorktreeError::Unknown),
            _ => Err(WorktreeError::Unknown),
        }
    }
    if fields.len() != 5 || get(&fields,"schema")? != "gogoke.37.worktree-merge-result.v2" ||
        get(&fields,"mergeReceipt")? != merge || !get(&fields,"error")?.is_empty() {
        return Err(WorktreeError::Unknown);
    }
    let intent = get(&fields,"childSealIntent")?;
    let child = get(&fields,"childCommit")?;
    let Json::Object(seal) = Parser::parse(&intent)? else { return Err(WorktreeError::Unknown); };
    if seal.len() != 18 || get(&seal,"schema")? != "gogoke.37.child-seal-intent.v1" ||
        get(&seal,"requestId")? != request.request_id || get(&seal,"requestHash")? != sha256_hex(&request.raw_bytes) ||
        get(&seal,"domainId")? != request.domain_id || get(&seal,"worktreeId")? != request.target_id ||
        !hex_commit(&child) || !hex_commit(&get(&seal,"childBefore")?) ||
        !hex_commit(&get(&seal,"baselineCommit")?) || !hex_commit(&get(&seal,"sourceBefore")?) ||
        !sha(&get(&seal,"snapshotHash")?) ||
        !get(&seal,"pointerHash")?.strip_prefix("sha256:").is_some_and(sha) ||
        ["worktreeIdentity","commonIdentity"].iter().any(|k| !get(&seal,k).is_ok_and(|v| !v.is_empty())) ||
        ["repositoryId","seatId","instanceId","turnId"].iter().any(|k| !get(&seal,k).is_ok_and(|v| atom(&v))) {
        return Err(WorktreeError::Unknown);
    }
    let Json::Array(stops) = Parser::parse(&get(&seal,"stopFacts")?)? else { return Err(WorktreeError::Unknown); };
    if stops.is_empty() { return Err(WorktreeError::Unknown); }
    for stop in stops {
        let Json::Object(fact) = stop else { return Err(WorktreeError::Unknown); };
        if fact.len()!=2 || !atom(&get(&fact,"processOperationId")?) || get(&fact,"stopFactId")?.is_empty() {
            return Err(WorktreeError::Unknown);
        }
    }
    let changed = get(&seal,"changed")?;
    if (changed == "true" && child == get(&seal,"childBefore")?) ||
        (changed == "false" && child != get(&seal,"childBefore")?) ||
        !matches!(changed.as_str(), "true" | "false") ||
        record != self::record(merge,&intent,Some(&child),None) ||
        Json::Object(seal).canonical() != intent {
        return Err(WorktreeError::Unknown);
    }
    Ok((intent,child))
}

fn sha(value: &str) -> bool {
    value.len()==64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
