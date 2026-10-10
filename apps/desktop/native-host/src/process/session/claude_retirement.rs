//! Exact, metadata-only retirement of an original Claude H package SID.
//! The caller persists every object image before calling `apply`. A retry
//! accepts only that image or its one-SID deletion, never a fresh baseline.
use super::super::legacy_holders_gone::NativeProcessHoldersGone;
use super::*;
use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};
use std::{io, mem::size_of, ptr};

#[link(name = "advapi32")]
extern "system" {
    fn InitializeAcl(acl: *mut c_void, length: u32, revision: u32) -> i32;
    fn AddAce(
        acl: *mut c_void,
        revision: u32,
        index: u32,
        bytes: *const c_void,
        length: u32,
    ) -> i32;
    fn InitializeSecurityDescriptor(descriptor: *mut c_void, revision: u32) -> i32;
    fn SetSecurityDescriptorDacl(
        descriptor: *mut c_void,
        present: i32,
        acl: *mut c_void,
        defaulted: i32,
    ) -> i32;
    fn SetSecurityDescriptorControl(descriptor: *mut c_void, mask: u16, bits: u16) -> i32;
    fn GetLengthSid(sid: *mut c_void) -> u32;
    fn IsValidSid(sid: *mut c_void) -> i32;
}
#[link(name = "ntdll")]
extern "system" {
    fn NtSetSecurityObject(handle: Handle, information: u32, descriptor: *mut c_void) -> i32;
}

#[repr(C)]
struct DaclDescriptor {
    revision: u8,
    reserved: u8,
    control: u16,
    owner: *mut c_void,
    group: *mut c_void,
    sacl: *mut c_void,
    dacl: *mut c_void,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClaudeAclObject {
    pub(crate) root_index: usize,
    /// UTF-16 code units encoded as lowercase hexadecimal, preserving exact
    /// Windows names without interpreting file contents or credentials.
    pub(crate) relative_utf16_hex: String,
    pub(crate) identity: String,
    pub(crate) directory: bool,
    pub(crate) control: u16,
    pub(crate) before_hex: String,
    pub(crate) after_hex: String,
}

pub(crate) struct ClaudeAclRetirement;

fn mismatch() -> IsolationError {
    IsolationError::AclWitnessMismatch
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(value: &str) -> Result<Vec<u8>, IsolationError> {
    if value.len() > 2_097_152
        || value.len() % 2 != 0
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(mismatch());
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).map_err(|_| mismatch())?, 16)
                .map_err(|_| mismatch())
        })
        .collect()
}
fn relative_hex(relative: &Path) -> Result<String, IsolationError> {
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(mismatch());
    }
    let mut bytes = Vec::new();
    for unit in relative.as_os_str().encode_wide() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(hex(&bytes))
}
fn decoded_relative(value: &str) -> Result<PathBuf, IsolationError> {
    let bytes = unhex(value)?;
    if bytes.len() % 2 != 0 {
        return Err(mismatch());
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let relative = PathBuf::from(OsString::from_wide(&units));
    if relative_hex(&relative)? != value {
        return Err(mismatch());
    }
    Ok(relative)
}

#[derive(Clone)]
struct Image {
    identity: RootIdentity,
    control: u16,
    ordered: Vec<u8>,
    after: Vec<u8>,
    target_count: usize,
    target_description: String,
    package_sids: Vec<String>,
}

fn read_image(handle: Handle, profile: &AppContainerProfile) -> Result<Image, IsolationError> {
    let identity = file_identity(handle)?;
    let mut acl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    let status = unsafe {
        GetSecurityInfo(
            handle,
            FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut acl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(IsolationError::Acl(io::Error::from_raw_os_error(
            status as i32,
        )));
    }
    let _descriptor = LocalAllocation(descriptor);
    if acl.is_null() || descriptor.is_null() {
        return Err(mismatch());
    }
    let mut count = AclSizeInformation {
        ace_count: 0,
        acl_bytes_in_use: 0,
        acl_bytes_free: 0,
    };
    if unsafe {
        GetAclInformation(
            acl,
            (&mut count as *mut AclSizeInformation).cast(),
            size_of::<AclSizeInformation>() as u32,
            ACL_SIZE_INFORMATION_CLASS,
        )
    } == 0
    {
        return Err(IsolationError::Acl(io::Error::last_os_error()));
    }
    let mut ordered = Vec::new();
    let mut after = Vec::new();
    let mut target_count = 0;
    let mut target_description = String::new();
    let mut package_sids = Vec::new();
    let mut previous_class = 0u8;
    for index in 0..count.ace_count {
        let mut ace = ptr::null_mut();
        if unsafe { GetAce(acl, index, &mut ace) } == 0 || ace.is_null() {
            return Err(mismatch());
        }
        let header = unsafe { &*ace.cast::<AceHeader>() };
        if header.ace_size < 16
            || !matches!(
                header.ace_type,
                ACCESS_ALLOWED_ACE_TYPE | ACCESS_DENIED_ACE_TYPE
            )
        {
            return Err(mismatch());
        }
        let class = (if header.ace_flags & INHERITED_ACE as u8 != 0 {
            2
        } else {
            0
        }) + if header.ace_type == ACCESS_ALLOWED_ACE_TYPE {
            1
        } else {
            0
        };
        if index > 0 && class < previous_class {
            return Err(mismatch());
        }
        previous_class = class;
        let sid = unsafe { ace.cast::<u8>().add(8).cast() };
        if unsafe { IsValidSid(sid) } == 0
            || unsafe { GetLengthSid(sid) } > u32::from(header.ace_size) - 8
        {
            return Err(mismatch());
        }
        let raw = unsafe { std::slice::from_raw_parts(ace.cast::<u8>(), header.ace_size as usize) };
        let mut sid_text = ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(sid, &mut sid_text) } == 0 || sid_text.is_null() {
            return Err(mismatch());
        }
        let _sid_allocation = LocalAllocation(sid_text.cast());
        let mut length = 0;
        while unsafe { *sid_text.add(length) } != 0 {
            if length >= 180 {
                return Err(mismatch());
            }
            length += 1;
        }
        let sid_text = String::from_utf16(unsafe { std::slice::from_raw_parts(sid_text, length) })
            .map_err(|_| mismatch())?;
        if sid_text.starts_with("S-1-15-2-") {
            package_sids.push(sid_text);
        }
        ordered.extend_from_slice(&(raw.len() as u32).to_be_bytes());
        ordered.extend_from_slice(raw);
        if unsafe { EqualSid(sid, profile.sid) } != 0 {
            target_count += 1;
            target_description = format!(
                "{}:{}:{}",
                header.ace_type,
                unsafe { (*ace.cast::<AccessAce>()).mask },
                header.ace_flags
            );
        } else {
            after.extend_from_slice(&(raw.len() as u32).to_be_bytes());
            after.extend_from_slice(raw);
        }
    }
    let mut control = 0u16;
    let mut revision = 0u32;
    if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
        return Err(IsolationError::Acl(io::Error::last_os_error()));
    }
    Ok(Image {
        identity,
        control,
        ordered,
        after,
        target_count,
        target_description,
        package_sids,
    })
}

fn expected_flags(root: bool, directory: bool) -> u32 {
    if root {
        OBJECT_AND_CONTAINER_INHERIT
    } else {
        INHERITED_ACE as u32
            | if directory {
                OBJECT_AND_CONTAINER_INHERIT
            } else {
                0
            }
    }
}

impl ClaudeAclRetirement {
    /// Read-only crash classification. PREPARED may resume only while every
    /// sealed object is still exactly its before or after image.
    pub(crate) fn verify_progress(
        profile: &AppContainerProfile,
        roots: &[(PathBuf, RootIdentity, bool)],
        objects: &[ClaudeAclObject],
        known_sids: &[String],
    ) -> Result<(), IsolationError> {
        Self::verify_inventory(roots, objects)?;
        for object in objects {
            let (root, identity, _) = roots.get(object.root_index).ok_or_else(mismatch)?;
            require_bound_path(root, identity, true)?;
            let relative = decoded_relative(&object.relative_utf16_hex)?;
            let held = open_physical_object(&root.join(relative), object.directory, READ_CONTROL)?;
            let current = read_image(held.0, profile)?;
            let before = unhex(&object.before_hex)?;
            let after = unhex(&object.after_hex)?;
            if current.identity.opaque() != object.identity
                || current.control != object.control
                || current
                    .package_sids
                    .iter()
                    .any(|sid| !known_sids.contains(sid))
                || !(current.ordered == before || current.ordered == after)
            {
                return Err(mismatch());
            }
        }
        Ok(())
    }
    pub(crate) fn readback_target(
        profile: &AppContainerProfile,
        roots: &[(PathBuf, RootIdentity, bool)],
        object: &ClaudeAclObject,
        known_sids: &[String],
        proof: &NativeProcessHoldersGone,
        pairs: &[(u32, u64)],
    ) -> Result<(), IsolationError> {
        proof.validate(pairs).map_err(|_| mismatch())?;
        let (root, identity, _) = roots.get(object.root_index).ok_or_else(mismatch)?;
        require_bound_path(root, identity, true)?;
        let relative = decoded_relative(&object.relative_utf16_hex)?;
        let held = open_physical_object(&root.join(relative), object.directory, READ_CONTROL)?;
        let current = read_image(held.0, profile)?;
        if current.identity.opaque() != object.identity
            || current.control != object.control
            || current.ordered != unhex(&object.after_hex)?
            || current.target_count != 0
            || current
                .package_sids
                .iter()
                .any(|sid| !known_sids.contains(sid))
        {
            return Err(mismatch());
        }
        proof.validate(pairs).map_err(|_| mismatch())?;
        require_bound_path(root, identity, true)
    }
    pub(crate) fn verify_inventory(
        roots: &[(PathBuf, RootIdentity, bool)],
        objects: &[ClaudeAclObject],
    ) -> Result<(), IsolationError> {
        if roots.len() != 3 || objects.is_empty() {
            return Err(mismatch());
        }
        if roots.iter().enumerate().any(|(i, (path, identity, _))| {
            roots
                .iter()
                .enumerate()
                .any(|(j, (other, other_identity, _))| {
                    i != j && (identity == other_identity || path.starts_with(other))
                })
        }) {
            return Err(mismatch());
        }
        let mut observed = Vec::new();
        for (index, (root, identity, _)) in roots.iter().enumerate() {
            require_bound_path(root, identity, true)?;
            let mut tree = vec![(root.clone(), identity.clone(), true)];
            tree.extend(collect_tree(root)?);
            for (path, physical, directory) in tree {
                observed.push((
                    index,
                    relative_hex(path.strip_prefix(root).map_err(|_| mismatch())?)?,
                    physical.opaque(),
                    directory,
                ));
            }
            require_bound_path(root, identity, true)?;
        }
        let expected: Vec<_> = objects
            .iter()
            .map(|object| {
                (
                    object.root_index,
                    object.relative_utf16_hex.clone(),
                    object.identity.clone(),
                    object.directory,
                )
            })
            .collect();
        if observed != expected {
            return Err(mismatch());
        }
        Ok(())
    }
    pub(crate) fn capture(
        profile: &AppContainerProfile,
        roots: &[(PathBuf, RootIdentity, bool)],
        known_sids: &[String],
    ) -> Result<Vec<ClaudeAclObject>, IsolationError> {
        if roots.len() != 3 || !roots[0].2 || !roots[1].2 {
            return Err(mismatch());
        }
        if roots.iter().enumerate().any(|(i, (path, identity, _))| {
            roots
                .iter()
                .enumerate()
                .any(|(j, (other, other_identity, _))| {
                    i != j && (identity == other_identity || path.starts_with(other))
                })
        }) {
            return Err(mismatch());
        }
        let sid = profile.package_sid_string()?;
        if known_sids.is_empty()
            || !known_sids.contains(&sid)
            || known_sids
                .iter()
                .enumerate()
                .any(|(i, s)| !s.starts_with("S-1-15-2-") || known_sids[..i].contains(s))
        {
            return Err(mismatch());
        }
        let mut result = Vec::new();
        for (root_index, (path, identity, writable)) in roots.iter().enumerate() {
            require_bound_path(path, identity, true)?;
            let mut objects = vec![(path.clone(), identity.clone(), true)];
            objects.extend(collect_tree(path)?);
            for (object_path, physical, directory) in objects {
                let relative = object_path.strip_prefix(path).map_err(|_| mismatch())?;
                let is_root = relative.as_os_str().is_empty();
                let held = open_physical_object(&object_path, directory, READ_CONTROL)?;
                let image = read_image(held.0, profile)?;
                if image.identity != physical
                    || image.target_count > 1
                    || image
                        .package_sids
                        .iter()
                        .any(|present| !known_sids.contains(present))
                {
                    return Err(mismatch());
                }
                let control_protected = image.control & SE_DACL_PROTECTED != 0;
                if image.target_count == 1 {
                    if image.target_description
                        != format!(
                            "{}:{}:{}",
                            ACCESS_ALLOWED_ACE_TYPE,
                            directory_rights(*writable),
                            expected_flags(is_root, directory)
                        )
                    {
                        return Err(mismatch());
                    }
                } else if is_root || !control_protected {
                    return Err(mismatch());
                }
                result.push(ClaudeAclObject {
                    root_index,
                    relative_utf16_hex: relative_hex(relative)?,
                    identity: physical.opaque(),
                    directory,
                    control: image.control,
                    before_hex: hex(&image.ordered),
                    after_hex: hex(&image.after),
                });
            }
            require_bound_path(path, identity, true)?;
        }
        Ok(result)
    }

    /// Each object uses the journaled before/after pair and the original
    /// process identity. No ACL reset, inherited recomputation or new grant.
    pub(crate) fn apply(
        profile: &AppContainerProfile,
        roots: &[(PathBuf, RootIdentity, bool)],
        object: &ClaudeAclObject,
        known_sids: &[String],
        proof: &NativeProcessHoldersGone,
        pairs: &[(u32, u64)],
    ) -> Result<(), IsolationError> {
        proof.validate(pairs).map_err(|_| mismatch())?;
        let (root, identity, writable) = roots.get(object.root_index).ok_or_else(mismatch)?;
        require_bound_path(root, identity, true)?;
        let relative = decoded_relative(&object.relative_utf16_hex)?;
        if relative.as_os_str().is_empty() && !object.directory {
            return Err(mismatch());
        }
        let path = root.join(&relative);
        let held = open_physical_object(&path, object.directory, READ_CONTROL | WRITE_DAC)?;
        let before = unhex(&object.before_hex)?;
        let after = unhex(&object.after_hex)?;
        let current = read_image(held.0, profile)?;
        if current.identity.opaque() != object.identity
            || current.control != object.control
            || current
                .package_sids
                .iter()
                .any(|sid| !known_sids.contains(sid))
        {
            return Err(mismatch());
        }
        if current.ordered == after {
            if current.target_count != 0 {
                return Err(mismatch());
            }
            proof.validate(pairs).map_err(|_| mismatch())?;
            return require_bound_path(root, identity, true);
        }
        let is_root = relative.as_os_str().is_empty();
        if current.ordered != before
            || current.after != after
            || current.target_count > 1
            || current.target_count == 1
                && current.target_description
                    != format!(
                        "{}:{}:{}",
                        ACCESS_ALLOWED_ACE_TYPE,
                        directory_rights(*writable),
                        expected_flags(is_root, object.directory)
                    )
            || current.target_count == 0 && (is_root || current.control & SE_DACL_PROTECTED == 0)
        {
            return Err(mismatch());
        }
        if current.target_count == 1 {
            proof.validate(pairs).map_err(|_| mismatch())?;
            write_exact_after(held.0, &current, &after)?;
        }
        let observed = read_image(held.0, profile)?;
        if observed.identity.opaque() != object.identity
            || observed.control != object.control
            || observed.ordered != after
            || observed.target_count != 0
        {
            return Err(mismatch());
        }
        proof.validate(pairs).map_err(|_| mismatch())?;
        require_bound_path(root, identity, true)
    }
}

fn write_exact_after(handle: Handle, before: &Image, after: &[u8]) -> Result<(), IsolationError> {
    let mut cursor = 0usize;
    let mut aces = Vec::new();
    while cursor < after.len() {
        if after.len() - cursor < 4 {
            return Err(mismatch());
        }
        let size = u32::from_be_bytes(
            after[cursor..cursor + 4]
                .try_into()
                .map_err(|_| mismatch())?,
        ) as usize;
        cursor += 4;
        if size < 16 || size > after.len() - cursor {
            return Err(mismatch());
        }
        aces.push(&after[cursor..cursor + size]);
        cursor += size;
    }
    let length = 8usize + aces.iter().map(|ace| ace.len()).sum::<usize>();
    if length > u16::MAX as usize {
        return Err(mismatch());
    }
    let mut storage = vec![0usize; length.div_ceil(size_of::<usize>())];
    let acl = storage.as_mut_ptr().cast();
    if unsafe { InitializeAcl(acl, length as u32, 4) } == 0 {
        return Err(IsolationError::Acl(io::Error::last_os_error()));
    }
    for ace in aces {
        if unsafe { AddAce(acl, 4, u32::MAX, ace.as_ptr().cast(), ace.len() as u32) } == 0 {
            return Err(IsolationError::Acl(io::Error::last_os_error()));
        }
    }
    let mut sd = DaclDescriptor {
        revision: 0,
        reserved: 0,
        control: 0,
        owner: ptr::null_mut(),
        group: ptr::null_mut(),
        sacl: ptr::null_mut(),
        dacl: ptr::null_mut(),
    };
    let descriptor = (&mut sd as *mut DaclDescriptor).cast();
    let mask = 0x0100 | 0x0400 | 0x1000;
    let control = (before.control & mask)
        | if before.control & 0x0400 != 0 {
            0x0100
        } else {
            0
        };
    if unsafe { InitializeSecurityDescriptor(descriptor, 1) } == 0
        || unsafe {
            SetSecurityDescriptorDacl(descriptor, 1, acl, i32::from(before.control & 0x0008 != 0))
        } == 0
        || unsafe { SetSecurityDescriptorControl(descriptor, mask, control) } == 0
    {
        return Err(IsolationError::Acl(io::Error::last_os_error()));
    }
    let status = unsafe { NtSetSecurityObject(handle, DACL_SECURITY_INFORMATION, descriptor) };
    if status < 0 {
        return Err(IsolationError::Acl(io::Error::new(
            io::ErrorKind::Other,
            format!(
                "NtSetSecurityObject original NTSTATUS=0x{:08x}",
                status as u32
            ),
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::io::AsRawHandle;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetProcessTimes(
            process: Handle,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }
    fn exited_real_child() -> (std::process::Child, (u32, u64)) {
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/C", "exit", "/B", "0"])
            .spawn()
            .expect("ordinary cloud child");
        let pid = child.id();
        child.wait().expect("ordinary child exit");
        let mut creation = FileTime { low: 0, high: 0 };
        let mut exit = FileTime { low: 0, high: 0 };
        let mut kernel = FileTime { low: 0, high: 0 };
        let mut user = FileTime { low: 0, high: 0 };
        assert_ne!(
            unsafe {
                GetProcessTimes(
                    child.as_raw_handle().cast(),
                    &mut creation,
                    &mut exit,
                    &mut kernel,
                    &mut user,
                )
            },
            0
        );
        let created = (u64::from(creation.high) << 32) | u64::from(creation.low);
        (child, (pid, created))
    }

    fn fixture() -> (
        PathBuf,
        Vec<(PathBuf, RootIdentity, bool)>,
        AppContainerProfile,
    ) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let fixture_id = format!("{}-{nonce}-{sequence}", std::process::id());
        let base = std::env::temp_dir().join(format!("claude-holder-acl-{fixture_id}"));
        std::fs::create_dir(&base).unwrap();
        let mut roots = Vec::new();
        for (name, writable) in [("instance", true), ("session", true), ("f", true)] {
            let path = base.join(name);
            std::fs::create_dir(&path).unwrap();
            std::fs::write(path.join("leaf.txt"), b"synthetic fixture").unwrap();
            let identity = crate::root::inspect_root(&path).unwrap().identity;
            roots.push((path, identity, writable));
        }
        let profile =
            AppContainerProfile::derived_for_test(&format!("Gogoke37.ClaudeHolderFixture.{fixture_id}"))
                .unwrap();
        for (path, identity, writable) in &roots {
            profile.grant_bound_tree(path, identity, *writable).unwrap();
        }
        (base, roots, profile)
    }

    #[test]
    fn claude_holder_exact_tree_retires_only_original_sid_and_resumes_after_partial_write() {
        let (base, roots, profile) = fixture();
        let known = vec![profile.sid_identity().unwrap()];
        let sealed = ClaudeAclRetirement::capture(&profile, &roots, &known).unwrap();
        assert_eq!(sealed.len(), 6);
        let pairs = [(u32::MAX, 1)];
        let gone = NativeProcessHoldersGone::observe(&pairs).unwrap();
        ClaudeAclRetirement::apply(&profile, &roots, &sealed[0], &known, &gone, &pairs).unwrap();
        ClaudeAclRetirement::verify_inventory(&roots, &sealed).unwrap();
        for object in &sealed {
            ClaudeAclRetirement::apply(&profile, &roots, object, &known, &gone, &pairs).unwrap();
            ClaudeAclRetirement::readback_target(&profile, &roots, object, &known, &gone, &pairs)
                .unwrap();
        }
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn claude_holder_rejects_identity_or_inventory_change_before_acl_write() {
        let (base, roots, profile) = fixture();
        let known = vec![profile.sid_identity().unwrap()];
        let sealed = ClaudeAclRetirement::capture(&profile, &roots, &known).unwrap();
        let pairs = [(u32::MAX, 1)];
        let gone = NativeProcessHoldersGone::observe(&pairs).unwrap();
        let mut wrong = sealed[0].clone();
        wrong.identity.push('0');
        assert!(
            ClaudeAclRetirement::apply(&profile, &roots, &wrong, &known, &gone, &pairs).is_err()
        );
        std::fs::write(roots[2].0.join("new-leaf.txt"), b"synthetic fixture").unwrap();
        assert!(ClaudeAclRetirement::verify_inventory(&roots, &sealed).is_err());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn claude_holder_two_exact_original_processes_require_the_full_gone_set() {
        let (first, one) = exited_real_child();
        let (second, two) = exited_real_child();
        let _held = (first, second);
        let pairs = [one, two];
        let gone =
            NativeProcessHoldersGone::observe(&pairs).expect("both actual exact children exited");
        let (base, roots, profile) = fixture();
        let known = vec![profile.sid_identity().unwrap()];
        let sealed = ClaudeAclRetirement::capture(&profile, &roots, &known).unwrap();
        assert!(
            ClaudeAclRetirement::apply(&profile, &roots, &sealed[0], &known, &gone, &[one])
                .is_err(),
            "a single H identity cannot discharge a two-holder original proof"
        );
        for object in &sealed {
            ClaudeAclRetirement::apply(&profile, &roots, object, &known, &gone, &pairs).unwrap();
            ClaudeAclRetirement::readback_target(&profile, &roots, object, &known, &gone, &pairs)
                .unwrap();
        }
        std::fs::remove_dir_all(base).unwrap();
    }
}
