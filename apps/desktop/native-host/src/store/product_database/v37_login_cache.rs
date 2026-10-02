//! Remove only the Windows profile cache junction created by ordinary login.
//! Call after the exact login Job and writer fence have been confirmed stopped.
//! The target is never opened, enumerated, or changed; credentials are untouched.

use super::{OrchestrationError, Result};
use crate::root::{RootIdentity, RootLock};
use crate::store::instance::ResolvedDirectory;
use std::ffi::c_void;
use std::io;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path};

type Handle = *mut c_void;
const DIRECTORY: u32 = 0x10;
const REPARSE: u32 = 0x400;
const MOUNT_POINT: u32 = 0xa0000003;
const CACHE: &str = "AppData/Local/Microsoft/Windows/INetCache/Content.IE5";

#[repr(C)]
#[derive(Default)]
struct TagInfo { attributes: u32, tag: u32 }
#[repr(C)]
#[derive(Default)]
struct IdInfo { volume: u64, id: [u8; 16] }
#[repr(C)]
struct Disposition { delete: u8 }

#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(path: *const u16, access: u32, share: u32, security: *const c_void,
        disposition: u32, flags: u32, template: Handle) -> Handle;
    fn GetFileInformationByHandleEx(handle: Handle, class: i32, data: *mut c_void, length: u32) -> i32;
    fn SetFileInformationByHandle(handle: Handle, class: i32, data: *const c_void, length: u32) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
}

struct Held(Handle);
impl Drop for Held {
    fn drop(&mut self) { unsafe { CloseHandle(self.0); } }
}

fn failure(stage: &str, error: io::Error) -> OrchestrationError {
    OrchestrationError::V37StoreFailure(format!(
        "login Windows cache {stage}: {error}; raw_os_error={:?}", error.raw_os_error()))
}

fn open(path: &Path, delete: bool) -> Result<Option<Held>> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // No-follow final entry; physical ancestors remain held without sharing
    // DELETE. Handles are non-inheritable and cannot rename out from under us.
    let raw = unsafe { CreateFileW(wide.as_ptr(), 0x80 | if delete { 0x10000 } else { 0 },
        3, std::ptr::null(), 3, 0x02200000, std::ptr::null_mut()) };
    if raw == -1isize as Handle {
        let error = io::Error::last_os_error();
        if matches!(error.raw_os_error(), Some(2 | 3)) { return Ok(None); }
        return Err(failure("no-follow open", error));
    }
    Ok(Some(Held(raw)))
}

fn tag(handle: &Held) -> Result<TagInfo> {
    let mut data = TagInfo::default();
    if unsafe { GetFileInformationByHandleEx(handle.0, 9, (&mut data as *mut TagInfo).cast(),
        size_of::<TagInfo>() as u32) } == 0 {
        return Err(failure("attributes/tag", io::Error::last_os_error()));
    }
    Ok(data)
}

fn identity(handle: &Held) -> Result<RootIdentity> {
    let mut data = IdInfo::default();
    if unsafe { GetFileInformationByHandleEx(handle.0, 18, (&mut data as *mut IdInfo).cast(),
        size_of::<IdInfo>() as u32) } == 0 {
        return Err(failure("identity", io::Error::last_os_error()));
    }
    Ok(RootIdentity { volume_serial: data.volume, file_id: data.id })
}

pub(super) fn remove_generated_cache_junction(root: &RootLock, home: &ResolvedDirectory) -> Result<()> {
    let bound_root = root.canonical_root();
    let relative = home.path.strip_prefix(&bound_root.canonical_path)
        .map_err(|_| OrchestrationError::AccessDenied)?;
    if relative.components().any(|part| !matches!(part, Component::Normal(_))) {
        return Err(OrchestrationError::AccessDenied);
    }
    let mut path = bound_root.canonical_path.clone();
    // RootLock already holds the root namespace binding with DELETE access.
    // Reopening it without sharing DELETE would conflict with that own pin.
    let mut ancestors = Vec::new();
    for part in relative.components().chain(Path::new(CACHE).components()) {
        path.push(part.as_os_str());
        let leaf = path == home.path.join(CACHE);
        let Some(handle) = open(&path, leaf)? else {
            // Only cache descendants may be absent; the registered home may not.
            return if path.starts_with(&home.path) && path != home.path { Ok(()) }
                else { Err(OrchestrationError::AccessDenied) };
        };
        let observed = tag(&handle)?;
        if path == home.path && identity(&handle)? != home.identity {
            return Err(OrchestrationError::AccessDenied);
        }
        if observed.attributes & DIRECTORY == 0 { return Err(OrchestrationError::AccessDenied); }
        if !leaf {
            if observed.attributes & REPARSE != 0 { return Err(OrchestrationError::AccessDenied); }
            ancestors.push(handle);
            continue;
        }
        if observed.attributes & REPARSE == 0 { return Ok(()); } // Real cache directories are data.
        if observed.tag != MOUNT_POINT { return Err(OrchestrationError::AccessDenied); }
        // Delete the directory junction itself using the same verified handle.
        // Rust symlink_metadata().is_dir() is false for a directory junction;
        // selecting DeleteFileW from that value instead would give Win32 5.
        let disposition = Disposition { delete: 1 };
        if unsafe { SetFileInformationByHandle(handle.0, 4, (&disposition as *const Disposition).cast(),
            size_of::<Disposition>() as u32) } == 0 {
            return Err(failure("directory entry disposition", io::Error::last_os_error()));
        }
        drop(handle);
        // DeletePending is not evidence of removal. Keep the ancestor pins
        // until the exact original entry is observed absent, without retries.
        return match std::fs::symlink_metadata(&path) {
            Err(error) if matches!(error.raw_os_error(), Some(2 | 3)) => Ok(()),
            Err(error) => Err(failure("entry absence", error)),
            Ok(_) => Err(OrchestrationError::V37StoreFailure(
                "login Windows cache entry remains after disposition".into())),
        };
    }
    Err(OrchestrationError::AccessDenied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root::inspect_root;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn generated_cache_junction_unlinks_only_entry_and_preserves_target() {
        let _guard = crate::store::same_open::route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let base = std::env::temp_dir().join(format!("gogoke-login-cache-{}-{nonce}", std::process::id()));
        let root_path = base.join("authority");
        let home_path = root_path.join("v37-instances/instanceA");
        let entry = home_path.join(CACHE);
        let target = base.join("owned-fixture-target");
        fs::create_dir_all(entry.parent().unwrap()).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(target.join("sentinel"), b"unchanged owned fixture").unwrap();
        let root = RootLock::acquire(&root_path).unwrap();
        let home = ResolvedDirectory { identity: inspect_root(&home_path).unwrap().identity, path: home_path };
        let create = std::process::Command::new("cmd.exe").args(["/D", "/C", "mklink", "/J"])
            .arg(&entry).arg(&target).output().unwrap();
        assert!(create.status.success(), "owned junction fixture creation failed: {:?}", create.status);
        let attributes = std::os::windows::fs::MetadataExt::file_attributes(&fs::symlink_metadata(&entry).unwrap());
        assert_ne!(attributes & DIRECTORY, 0, "directory attributes select the directory deletion API");
        let mut wrong = home.clone();
        wrong.identity.file_id[0] ^= 1;
        assert!(remove_generated_cache_junction(&root, &wrong).is_err());
        assert!(fs::symlink_metadata(&entry).is_ok(), "wrong binding cannot unlink the entry");
        remove_generated_cache_junction(&root, &home).unwrap();
        assert!(matches!(fs::symlink_metadata(&entry), Err(ref error) if error.kind() == io::ErrorKind::NotFound));
        assert_eq!(fs::read(target.join("sentinel")).unwrap(), b"unchanged owned fixture");
        // Absence is idempotent; a physical directory is retained as user data.
        remove_generated_cache_junction(&root, &home).unwrap();
        fs::create_dir(&entry).unwrap();
        fs::write(entry.join("physical-cache-data"), b"retain").unwrap();
        remove_generated_cache_junction(&root, &home).unwrap();
        assert_eq!(fs::read(entry.join("physical-cache-data")).unwrap(), b"retain");
        drop(root);
        fs::remove_dir_all(&base).unwrap();
    }
}
