//! Native custody of a new global instance's persistent home. This prepares a
//! directory for later login; it does not grant any seat process access.

use crate::root::{inspect_root, RootIdentity, RootLock, RootLockError};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::windows::fs::MetadataExt;
use std::path::PathBuf;

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const CONTAINER: &str = "v37-instances";
const MARKER: &str = "gogoke-instance.marker";

#[derive(Debug)]
pub(crate) enum HomeError {
    InvalidInstanceId,
    ExistingInstance,
    ReparseOrNonDirectory,
    IdentityChanged,
    Io(io::Error),
    Root(RootLockError),
}

impl From<io::Error> for HomeError {
    fn from(error: io::Error) -> Self { Self::Io(error) }
}

impl From<RootLockError> for HomeError {
    fn from(error: RootLockError) -> Self { Self::Root(error) }
}

pub(crate) struct PreparedInstanceHome {
    pub(crate) path: PathBuf,
    pub(crate) identity: RootIdentity,
    pub(crate) directory_ref: String,
}

fn valid_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 || !bytes[0].is_ascii_alphabetic() ||
        !bytes[1..].iter().all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-')) {
        return false;
    }
    let upper = value.to_ascii_uppercase();
    !matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") &&
        !(upper.len() == 4 && (upper.starts_with("COM") || upper.starts_with("LPT")) &&
          matches!(upper.as_bytes()[3], b'1'..=b'9'))
}

fn checked_directory(path: &std::path::Path) -> Result<RootIdentity, HomeError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(HomeError::ReparseOrNonDirectory);
    }
    Ok(inspect_root(path)?.identity)
}

/// Create once, never reuse a preexisting name. F records the physical identity
/// before any login or launch. Recovery of an interrupted create is separate;
/// it must inspect the marker and identity, never blindly call this again.
pub(crate) fn prepare_persistent_home(
    root: &RootLock,
    instance_id: &str,
) -> Result<PreparedInstanceHome, HomeError> {
    if !valid_id(instance_id) { return Err(HomeError::InvalidInstanceId); }
    let root_path = &root.canonical_root().canonical_path;
    if checked_directory(root_path)? != root.canonical_root().identity {
        return Err(HomeError::IdentityChanged);
    }
    let parent = root_path.join(CONTAINER);
    match fs::create_dir(&parent) {
        Ok(()) => (),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(HomeError::Io(error)),
    }
    let parent_identity = checked_directory(&parent)?;
    let path = parent.join(instance_id);
    match fs::create_dir(&path) {
        Ok(()) => (),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists =>
            return Err(HomeError::ExistingInstance),
        Err(error) => return Err(HomeError::Io(error)),
    }
    let identity = checked_directory(&path)?;
    let marker = path.join(MARKER);
    let mut output = OpenOptions::new().write(true).create_new(true).open(&marker)?;
    writeln!(output, "gogoke-v37-instance-home-v1")?;
    writeln!(output, "{}", root.canonical_root().identity.opaque())?;
    writeln!(output, "{instance_id}")?;
    output.sync_all()?;
    if checked_directory(&parent)? != parent_identity || checked_directory(&path)? != identity {
        return Err(HomeError::IdentityChanged);
    }
    Ok(PreparedInstanceHome { path, identity,
        directory_ref: format!("instance-home-{instance_id}") })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::same_open::route_b_test_guard;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn creates_one_native_home_and_never_reuses_a_name() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gogoke-v37-home-{}-{nonce}", std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        assert!(matches!(prepare_persistent_home(&root, "CON"), Err(HomeError::InvalidInstanceId)));
        let prepared = prepare_persistent_home(&root, "instanceA").unwrap();
        assert_eq!(checked_directory(&prepared.path).unwrap(), prepared.identity);
        assert_eq!(prepared.directory_ref, "instance-home-instanceA");
        assert!(matches!(prepare_persistent_home(&root, "instanceA"), Err(HomeError::ExistingInstance)));
        let marker = prepared.path.join(MARKER);
        assert!(fs::read_to_string(&marker).unwrap().contains("gogoke-v37-instance-home-v1"));
        drop(root);
        fs::remove_file(marker).unwrap();
        fs::remove_dir(prepared.path).unwrap();
        fs::remove_dir(path.join(CONTAINER)).unwrap();
        fs::remove_dir(path).unwrap();
    }
}
