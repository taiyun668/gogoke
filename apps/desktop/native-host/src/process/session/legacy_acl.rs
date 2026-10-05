//! Metadata-only recovery of the exact pre-boot legacy AppContainer ACLs.
//! F persists this inventory before any ACL mutation. A restored inventory is
//! never reconstructed from the possibly partially changed live DACL.

use super::*;
use super::super::legacy_holders_gone::NativeProcessHoldersGone;
use crate::store::digest::sha256_hex;

#[derive(Clone, Debug, Eq, PartialEq)]
struct Ace { kind: u8, flags: u8, mask: u32, sid: String }

#[derive(Clone, Debug, Eq, PartialEq)]
struct Dacl { protected: bool, aces: Vec<Ace> }

#[derive(Clone, Debug, Eq, PartialEq)]
struct Object {
    relative: PathBuf,
    identity: RootIdentity,
    directory: bool,
    source: bool,
    original: Dacl,
}

/// A sealed, complete physical HOME inventory. The opaque encoding contains
/// names, file IDs and DACL metadata, never file data or credential bytes.
#[derive(Clone, Debug)]
pub(crate) struct LegacyAclInventory {
    root_identity: RootIdentity,
    home_identity: RootIdentity,
    source_identity: RootIdentity,
    historical: Vec<(String, String)>,
    baseline: Dacl,
    objects: Vec<Object>,
}

#[derive(Clone, Debug)]
pub(crate) struct LegacyHomeReceipt {
    pub(crate) home_observed_digest: String,
    pub(crate) source_observed_digest: String,
}
#[derive(Clone, Debug)]
pub(crate) struct LegacySourceReceipt {
    pub(crate) source_observed_digest: String,
    pub(crate) baseline_core_digest: String,
    pub(crate) observed_raw_digest: String,
}

#[link(name = "advapi32")]
extern "system" {
    fn IsValidSid(sid: *mut c_void) -> i32;
    fn GetLengthSid(sid: *mut c_void) -> u32;
}

fn mismatch() -> CredentialError { IsolationError::AclWitnessMismatch.into() }

fn sid_text(sid: *mut c_void) -> Result<String, IsolationError> {
    if sid.is_null() || unsafe { IsValidSid(sid) } == 0 {
        return Err(IsolationError::AclWitnessMismatch);
    }
    let mut text = ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(IsolationError::Acl(io::Error::last_os_error()));
    }
    let _allocation = LocalAllocation(text.cast());
    let mut length = 0;
    while unsafe { *text.add(length) } != 0 {
        if length >= 180 { return Err(IsolationError::AclWitnessMismatch); }
        length += 1;
    }
    Ok(std::ffi::OsString::from_wide(unsafe { std::slice::from_raw_parts(text, length) })
        .to_string_lossy().into_owned())
}

fn read_dacl(handle: Handle) -> Result<Dacl, IsolationError> {
    let mut acl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    let status = unsafe { GetSecurityInfo(handle, FILE_OBJECT, DACL_SECURITY_INFORMATION,
        ptr::null_mut(), ptr::null_mut(), &mut acl, ptr::null_mut(), &mut descriptor) };
    if status != 0 { return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32))); }
    let _descriptor = LocalAllocation(descriptor);
    if acl.is_null() || descriptor.is_null() { return Err(IsolationError::AclWitnessMismatch); }
    let mut control = 0u16;
    let mut revision = 0u32;
    if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
        return Err(IsolationError::Acl(io::Error::last_os_error()));
    }
    let mut size = AclSizeInformation { ace_count: 0, acl_bytes_in_use: 0,
        acl_bytes_free: 0 };
    if unsafe { GetAclInformation(acl, (&mut size as *mut AclSizeInformation).cast(),
        size_of::<AclSizeInformation>() as u32, ACL_SIZE_INFORMATION_CLASS) } == 0 {
        return Err(IsolationError::Acl(io::Error::last_os_error()));
    }
    let mut aces = Vec::with_capacity(size.ace_count as usize);
    for index in 0..size.ace_count {
        let mut raw = ptr::null_mut();
        if unsafe { GetAce(acl, index, &mut raw) } == 0 {
            return Err(IsolationError::Acl(io::Error::last_os_error()));
        }
        if raw.is_null() { return Err(IsolationError::AclWitnessMismatch); }
        let header = unsafe { &*raw.cast::<AceHeader>() };
        if !matches!(header.ace_type, ACCESS_ALLOWED_ACE_TYPE | ACCESS_DENIED_ACE_TYPE)
            || header.ace_size < 16 { return Err(IsolationError::AclWitnessMismatch); }
        let sid = unsafe { raw.cast::<u8>().add(8).cast() };
        // Validate the bounded ACE before converting its embedded SID.
        if unsafe { IsValidSid(sid) } == 0 ||
            unsafe { GetLengthSid(sid) } > u32::from(header.ace_size) - 8 {
            return Err(IsolationError::AclWitnessMismatch);
        }
        aces.push(Ace { kind: header.ace_type, flags: header.ace_flags,
            mask: unsafe { (*raw.cast::<AccessAce>()).mask }, sid: sid_text(sid)? });
    }
    Ok(Dacl { protected: control & SE_DACL_PROTECTED != 0, aces })
}

fn checked_profiles(names: &[String]) -> Result<Vec<(String, String)>, CredentialError> {
    if names.is_empty() { return Err(CredentialError::Invalid("empty historical profile set")); }
    let mut profiles = Vec::with_capacity(names.len());
    for name in names {
        if profiles.iter().any(|(known, _)| known == name) {
            return Err(CredentialError::Invalid("duplicate historical profile"));
        }
        let profile = AppContainerProfile::derive_for_revocation(name)?;
        let sid = profile.package_sid_string()?;
        if profiles.iter().any(|(_, known_sid)| known_sid == &sid) {
            return Err(CredentialError::Invalid("duplicate historical SID"));
        }
        profiles.push((name.clone(), sid));
    }
    profiles.sort();
    Ok(profiles)
}

fn is_package_sid(sid: &str) -> bool { sid.starts_with("S-1-15-2-") }

fn expected_legacy_ace(ace: &Ace, home: bool) -> bool {
    ace.kind == ACCESS_ALLOWED_ACE_TYPE && ace.mask == directory_rights(true)
        && if home { ace.flags == OBJECT_AND_CONTAINER_INHERIT as u8 }
        else { ace.flags & INHERITED_ACE as u8 != 0
            && ace.flags & !(INHERITED_ACE as u8 | OBJECT_AND_CONTAINER_INHERIT as u8) == 0 }
}

fn check_original(object: &Object, historical: &[(String, String)])
    -> Result<(), CredentialError> {
    let home = object.relative.as_os_str().is_empty();
    for (_, sid) in historical {
        let matches: Vec<_> = object.original.aces.iter().filter(|ace| &ace.sid == sid).collect();
        if matches.len() > 1 || matches.first().is_some_and(|ace|
            !expected_legacy_ace(ace, home)) {
            return Err(mismatch());
        }
    }
    if object.original.aces.iter().any(|ace| is_package_sid(&ace.sid)
        && !historical.iter().any(|(_, sid)| sid == &ace.sid)) {
        return Err(mismatch());
    }
    Ok(())
}

fn home_target(original: &Dacl, historical: &[(String, String)]) -> Dacl {
    Dacl { protected: original.protected,
        aces: original.aces.iter().filter(|ace|
            !historical.iter().any(|(_, sid)| sid == &ace.sid)).cloned().collect() }
}

/// A single ACL write may remove only one captured legacy SID. The observed
/// ACL is legal exactly when it equals the original after removing some whole
/// captured SID ACEs; every non-target ACE and deny ordering stay intact.
fn legal_home_progress(observed: &Dacl, original: &Dacl,
    historical: &[(String, String)]) -> bool {
    let mut expected = original.clone();
    for (_, sid) in historical {
        if !observed.aces.iter().any(|ace| &ace.sid == sid) {
            expected.aces.retain(|ace| &ace.sid != sid);
        }
    }
    acl_equal(observed, &expected)
}

fn baseline_target() -> Result<Dacl, IsolationError> {
    let (_token, _buffer, user) = host_user_sid()?;
    let system = well_known_sid("S-1-5-18")?;
    let admins = well_known_sid("S-1-5-32-544")?;
    Ok(Dacl { protected: true, aces: [user, system.0, admins.0]
        .into_iter().map(|sid| Ok(Ace { kind: ACCESS_ALLOWED_ACE_TYPE,
            flags: 0, mask: FILE_ALL_ACCESS, sid: sid_text(sid)? }))
        .collect::<Result<Vec<_>, IsolationError>>()? })
}

fn acl_equal(left: &Dacl, right: &Dacl) -> bool {
    if left.protected != right.protected || left.aces.len() != right.aces.len() { return false; }
    // Deny/allow order can change effective access. Only all-allow ACLs may
    // tolerate Windows reordering otherwise identical entries.
    if left.aces.iter().chain(&right.aces).any(|ace|
        ace.kind == ACCESS_DENIED_ACE_TYPE) { return left.aces == right.aces; }
    let mut remaining = right.aces.clone();
    for ace in &left.aces {
        let Some(index) = remaining.iter().position(|other| other == ace) else { return false; };
        remaining.remove(index);
    }
    true
}

fn check_source_original(source: &Object, historical: &[(String, String)])
    -> Result<(), CredentialError> {
    if source.original.protected { return Err(mismatch()); }
    let baseline = baseline_target()?;
    let allowed: Vec<_> = baseline.aces.iter().map(|ace| &ace.sid).collect();
    for ace in &source.original.aces {
        if historical.iter().any(|(_, sid)| sid == &ace.sid) { continue; }
        if allowed.contains(&&ace.sid) {
            if ace.kind != ACCESS_ALLOWED_ACE_TYPE { return Err(mismatch()); }
        } else if !is_exact_inherited_local_read_execute(ace) {
            return Err(mismatch());
        }
    }
    Ok(())
}

fn is_exact_inherited_local_read_execute(ace: &Ace) -> bool {
    let parts: Vec<_> = ace.sid.split('-').collect();
    let local_user_sid = parts.len() == 8 && parts.starts_with(&["S", "1", "5", "21"])
        && parts[4..].iter().all(|part| part.parse::<u32>().is_ok());
    local_user_sid && ace.kind == ACCESS_ALLOWED_ACE_TYPE
        && ace.flags == INHERITED_ACE as u8
        && ace.mask == (FILE_GENERIC_READ | FILE_GENERIC_EXECUTE)
}

/// The caller's list is the complete *permitted* current package set. A
/// baseline may precede the first observer grant; missing listed grants do
/// not widen access. Every grant actually present must match one listed SID,
/// right and no-inheritance shape, and the remaining ACL must be the baseline.
fn verify_permitted_current(actual: &Dacl, baseline: &Dacl,
    permitted_current: &[(String, u32)]) -> Result<(), CredentialError> {
    if !actual.protected || !baseline.protected { return Err(mismatch()); }
    for (index, (sid, rights)) in permitted_current.iter().enumerate() {
        if !is_package_sid(sid) || !matches!(*rights, FILE_GENERIC_READ | CREDENTIAL_FILE_RIGHTS)
            || baseline.aces.iter().any(|ace| &ace.sid == sid)
            || permitted_current[..index].iter().any(|(prior, _)| prior == sid) {
            return Err(mismatch());
        }
    }
    let mut remaining = Vec::with_capacity(actual.aces.len());
    let mut seen = Vec::new();
    for ace in &actual.aces {
        if is_package_sid(&ace.sid) {
            let Some((_, rights)) = permitted_current.iter().find(|(sid, _)| sid == &ace.sid)
                else { return Err(mismatch()); };
            if ace.kind != ACCESS_ALLOWED_ACE_TYPE || ace.flags != 0 || ace.mask != *rights
                || seen.contains(&ace.sid) { return Err(mismatch()); }
            seen.push(ace.sid.clone());
        } else {
            remaining.push(ace.clone());
        }
    }
    if !acl_equal(&Dacl { protected: true, aces: remaining }, baseline) {
        return Err(mismatch());
    }
    Ok(())
}

fn relative_path(home: &Path, path: &Path) -> Result<PathBuf, CredentialError> {
    let relative = path.strip_prefix(home).map_err(|_| mismatch())?.to_path_buf();
    if relative.components().any(|component| !matches!(component,
        std::path::Component::Normal(_))) && !relative.as_os_str().is_empty() {
        return Err(mismatch());
    }
    Ok(relative)
}

fn source_acl(binding: &CredentialBinding) -> Result<Dacl, CredentialError> {
    binding.with_source_metadata_acl(|handle| Ok(read_dacl(handle)?))
}

/// One immutable, metadata-only removal from a protected credential DACL.
/// F separately binds this snapshot and its digest to the durable recovery
/// journal. Only the named exact RW SID may be removed; all other ACEs survive.
pub(crate) struct NativeCredentialAclRecoveryStep {
    source_identity: RootIdentity,
    target_sid: String,
    known_profiles: Vec<(String, u32)>,
    before: Dacl,
    target: Dacl,
}

impl NativeCredentialAclRecoveryStep {
    pub(crate) fn capture(binding: &CredentialBinding, target_sid: &str,
        complete_known_profiles: &[(String, u32)]) -> Result<Self, CredentialError> {
        let known_profiles = recovery_profiles(target_sid, complete_known_profiles)?;
        let before = source_acl(binding)?;
        verify_permitted_current(&before, &baseline_target()?, &known_profiles)?;
        let target = remove_recovery_sid(&before, target_sid);
        Ok(Self { source_identity: binding.identity().clone(), target_sid: target_sid.into(),
            known_profiles, before, target })
    }

    pub(crate) fn before_digest(&self) -> String { canonical_dacl_digest(&self.before) }
    pub(crate) fn target_digest(&self) -> String { canonical_dacl_digest(&self.target) }
    pub(crate) fn encode_snapshot(&self) -> String { hex(&self.bytes()) }

    /// The supplied digest is the canonical before-DACL digest. F independently
    /// verifies the whole snapshot hash; the caller's binding rechecks FileID.
    pub(crate) fn restore(snapshot: &str, before_digest: &str, target_sid: &str,
        complete_known_profiles: &[(String, u32)]) -> Result<Self, CredentialError> {
        const DOMAIN: &[u8] = b"gogoke-credential-acl-remove-v1\0";
        let bytes = unhex(snapshot)?;
        let mut reader = Reader { bytes: &bytes, index: 0 };
        if reader.take(DOMAIN.len())? != DOMAIN { return Err(mismatch()); }
        let source_identity = reader.identity()?;
        let saved_sid = reader.string()?;
        let count = reader.u32()? as usize;
        if count == 0 || count > 4096 { return Err(mismatch()); }
        let mut known_profiles = Vec::with_capacity(count);
        for _ in 0..count { known_profiles.push((reader.string()?, reader.u32()?)); }
        let before = reader.dacl()?;
        let target = reader.dacl()?;
        if reader.index != bytes.len() || saved_sid != target_sid
            || known_profiles != recovery_profiles(target_sid, complete_known_profiles)?
            || canonical_dacl_digest(&before) != before_digest
            || target != remove_recovery_sid(&before, target_sid) {
            return Err(mismatch());
        }
        verify_permitted_current(&before, &baseline_target()?, &known_profiles)?;
        let step = Self { source_identity, target_sid: saved_sid, known_profiles, before, target };
        if step.bytes() != bytes { return Err(mismatch()); }
        Ok(step)
    }

    /// F persists the immutable intent before calling. A retry accepts only the
    /// captured before ACL or its exact one-SID-removal target. The target path
    /// is read-only; neither branch prepares or resets the source baseline.
    pub(crate) fn apply_or_readback(&self, binding: &CredentialBinding,
        holders_gone: &NativeProcessHoldersGone, exact_pairs: &[(u32, u64)])
        -> Result<String, CredentialError> {
        self.apply_or_readback_inner(binding, holders_gone, exact_pairs,
            &mut revoke_exact_credential_ace)
    }

    fn apply_or_readback_inner(&self, binding: &CredentialBinding,
        holders_gone: &NativeProcessHoldersGone, exact_pairs: &[(u32, u64)],
        revoke: &mut impl FnMut(Handle, *mut c_void, &RootIdentity) -> Result<(), IsolationError>)
        -> Result<String, CredentialError> {
        validate_recovery_holders(holders_gone, exact_pairs)?;
        if binding.identity() != &self.source_identity { return Err(CredentialError::IdentityChanged); }
        let observed = binding.with_source_metadata_acl(|handle| {
            if file_identity(handle)? != self.source_identity { return Err(CredentialError::IdentityChanged); }
            let actual = read_dacl(handle)?;
            if !acl_equal(&actual, &self.target) {
                if !acl_equal(&actual, &self.before) { return Err(mismatch()); }
                let sid = well_known_sid(&self.target_sid)?;
                validate_recovery_holders(holders_gone, exact_pairs)?;
                revoke(handle, sid.0, &self.source_identity)?;
            }
            let after = read_dacl(handle)?;
            if file_identity(handle)? != self.source_identity || !acl_equal(&after, &self.target) {
                return Err(mismatch());
            }
            validate_recovery_holders(holders_gone, exact_pairs)?;
            Ok(canonical_dacl_digest(&after))
        })?;
        validate_recovery_holders(holders_gone, exact_pairs)?;
        if observed != self.target_digest() { return Err(mismatch()); }
        Ok(observed)
    }

    fn bytes(&self) -> Vec<u8> {
        let mut bytes = b"gogoke-credential-acl-remove-v1\0".to_vec();
        encode_identity(&mut bytes, &self.source_identity);
        put_str(&mut bytes, &self.target_sid);
        put_u32(&mut bytes, self.known_profiles.len() as u32);
        for (sid, rights) in &self.known_profiles { put_str(&mut bytes, sid); put_u32(&mut bytes, *rights); }
        encode_dacl(&mut bytes, &self.before);
        encode_dacl(&mut bytes, &self.target);
        bytes
    }
}

fn recovery_profiles(target_sid: &str, profiles: &[(String, u32)])
    -> Result<Vec<(String, u32)>, CredentialError> {
    if profiles.is_empty() || profiles.len() > 4096
        || !profiles.iter().any(|(sid, rights)| sid == target_sid && *rights == CREDENTIAL_FILE_RIGHTS) {
        return Err(mismatch());
    }
    let baseline = baseline_target()?;
    verify_permitted_current(&baseline, &baseline, profiles)?;
    for (sid, _) in profiles {
        let parsed = well_known_sid(sid)?;
        if sid_text(parsed.0)? != *sid { return Err(mismatch()); }
    }
    let mut ordered = profiles.to_vec();
    ordered.sort();
    Ok(ordered)
}

fn remove_recovery_sid(before: &Dacl, target_sid: &str) -> Dacl {
    Dacl { protected: before.protected, aces: before.aces.iter()
        .filter(|ace| ace.sid != target_sid).cloned().collect() }
}

fn validate_recovery_holders(proof: &NativeProcessHoldersGone, pairs: &[(u32, u64)])
    -> Result<(), CredentialError> {
    proof.validate(pairs).map_err(|error| CredentialError::Io {
        operation: "validate exact gone process holders",
        source: io::Error::new(io::ErrorKind::Other, error) })
}

/// Read-only adoption after F has settled every captured recovery grant and
/// separately revalidated holder facts. This marks the held source prepared
/// only after its exact current protected baseline/permitted ACEs are checked.
pub(crate) fn adopt_holder_gone_source_baseline(binding: &CredentialBinding,
    expected_digest: &str, permitted_remaining: &[(String, u32)])
    -> Result<String, CredentialError> {
    let baseline = baseline_target()?;
    binding.with_source_acl(|handle, _prepared| {
        if &file_identity(handle)? != binding.identity() { return Err(CredentialError::IdentityChanged); }
        let actual = read_dacl(handle)?;
        verify_permitted_current(&actual, &baseline, permitted_remaining)?;
        let observed = canonical_dacl_digest(&actual);
        if observed != expected_digest { return Err(mismatch()); }
        Ok(observed)
    })
}

fn observe_object(path: &Path, identity: &RootIdentity, directory: bool,
    source: bool, binding: &CredentialBinding) -> Result<Dacl, CredentialError> {
    if source {
        binding.with_source_metadata_acl(|handle| {
            if &file_identity(handle)? != identity { return Err(mismatch()); }
            Ok(read_dacl(handle)?)
        })
    } else {
        let handle = open_physical_object(path, directory, READ_CONTROL)?;
        if &file_identity(handle.0)? != identity { return Err(mismatch()); }
        Ok(read_dacl(handle.0)?)
    }
}

impl LegacyAclInventory {
    pub(crate) fn capture(root: &RootLock, home: &Path,
        home_identity: &RootIdentity, binding: &CredentialBinding,
        historical_profile_names: &[String]) -> Result<Self, CredentialError> {
        if !home.starts_with(&root.canonical_root().canonical_path) {
            return Err(mismatch());
        }
        require_bound_path(home, home_identity, true)?;
        binding.verify_registered_aliases(&[])?;
        let (source_identity, links) = CredentialBinding::observe_source_metadata(root,
            &home.join("auth.json"), home_identity)?;
        if links != 1 || &source_identity != binding.identity() {
            return Err(mismatch());
        }
        let historical = checked_profiles(historical_profile_names)?;
        let mut objects = Vec::new();
        let home_acl = observe_object(home, home_identity, true, false, binding)?;
        objects.push(Object { relative: PathBuf::new(), identity: home_identity.clone(),
            directory: true, source: false, original: home_acl });
        let tree = collect_legacy_owner_tree(home, Some((binding, &[])))?;
        for (path, identity, directory, source) in tree {
            let acl = observe_object(&path, &identity, directory, source, binding)?;
            objects.push(Object { relative: relative_path(home, &path)?, identity,
                directory, source, original: acl });
        }
        if objects.iter().filter(|object| object.source).count() != 1 ||
            objects.iter().find(|object| object.source).is_none_or(|object|
                object.relative != Path::new("auth.json") || object.identity != source_identity) {
            return Err(mismatch());
        }
        for object in &objects { check_original(object, &historical)?; }
        check_source_original(objects.iter().find(|object| object.source).ok_or_else(mismatch)?,
            &historical)?;
        // The root's broad ACE is the causal old grant. An already-clean root
        // is not a fresh legacy fence candidate.
        if !historical.iter().any(|(_, sid)| objects[0].original.aces.iter().any(|ace|
            &ace.sid == sid)) { return Err(mismatch()); }
        require_bound_path(home, home_identity, true)?;
        binding.verify_registered_aliases(&[])?;
        Ok(Self { root_identity: root.canonical_root().identity.clone(),
            home_identity: home_identity.clone(), source_identity, historical,
            baseline: baseline_target()?, objects })
    }

    pub(crate) fn encode_snapshot(&self) -> String { hex(&self.bytes()) }
    pub(crate) fn original_digest(&self) -> String { sha256_hex(&self.bytes()) }
    pub(crate) fn home_original_digest(&self) -> String { self.home_digest(false) }
    pub(crate) fn home_target_digest(&self) -> String {
        self.home_digest(true)
    }
    pub(crate) fn source_original_digest(&self) -> String {
        self.source_digest(&self.source().original)
    }
    pub(crate) fn source_after_home_digest(&self) -> String {
        self.source_digest(&home_target(&self.source().original, &self.historical))
    }
    pub(crate) fn source_target_digest(&self) -> String {
        self.source_digest(&self.baseline)
    }
    pub(crate) fn baseline_core_digest(&self) -> String {
        canonical_dacl_digest(&self.baseline)
    }

    pub(crate) fn restore(root: &RootLock, home: &Path,
        home_identity: &RootIdentity, binding: &CredentialBinding,
        historical_profile_names: &[String], payload: &str, original_digest: &str)
        -> Result<Self, CredentialError> {
        let inventory = Self::parse_saved(root, home_identity, binding,
            historical_profile_names, payload, original_digest)?;
        inventory.verify_structure(root, home, home_identity, binding)?;
        inventory.verify_live(root, home, home_identity, binding, true)?;
        Ok(inventory)
    }

    /// A completed fence may be followed by legitimate new runtime files and
    /// current grants. Validate the immutable original snapshot, then adopt
    /// only the original physical source's exact protected current ACL.
    pub(crate) fn restore_for_adoption(root: &RootLock, home: &Path,
        home_identity: &RootIdentity, binding: &CredentialBinding,
        historical_profile_names: &[String], payload: &str, original_digest: &str,
        registered_aliases: &[CredentialAliasScope], permitted_current: &[(String, u32)])
        -> Result<(Self, LegacySourceReceipt), CredentialError> {
        let inventory = Self::parse_saved(root, home_identity, binding,
            historical_profile_names, payload, original_digest)?;
        inventory.verify_structure(root, home, home_identity, binding)?;
        let receipt = inventory.adopt_existing_protected_baseline(root, home,
            home_identity, binding, registered_aliases, permitted_current)?;
        Ok((inventory, receipt))
    }

    fn parse_saved(root: &RootLock, home_identity: &RootIdentity,
        binding: &CredentialBinding, historical_profile_names: &[String],
        payload: &str, original_digest: &str) -> Result<Self, CredentialError> {
        let bytes = unhex(payload)?;
        if sha256_hex(&bytes) != original_digest { return Err(mismatch()); }
        let inventory = Self::from_bytes(&bytes)?;
        if inventory.bytes() != bytes || inventory.root_identity != root.canonical_root().identity
            || inventory.home_identity != *home_identity
            || inventory.source_identity != *binding.identity()
            || inventory.historical != checked_profiles(historical_profile_names)?
            || !acl_equal(&inventory.baseline, &baseline_target()?) {
            return Err(mismatch());
        }
        Ok(inventory)
    }

    fn verify_structure(&self, root: &RootLock, home: &Path,
        home_identity: &RootIdentity, binding: &CredentialBinding)
        -> Result<(), CredentialError> {
        if self.root_identity != root.canonical_root().identity
            || self.home_identity != *home_identity
            || self.source_identity != *binding.identity()
            || !home.starts_with(&root.canonical_root().canonical_path)
            || self.objects.is_empty() || self.objects[0].relative != Path::new("")
            || self.objects[0].identity != *home_identity
            || !self.objects[0].directory || self.objects[0].source
            || self.objects.iter().filter(|object| object.source).count() != 1
            || self.objects.iter().find(|object| object.source).is_none_or(|object|
                object.relative != Path::new("auth.json") || object.identity != self.source_identity)
            || self.objects.iter().skip(1).any(|object| object.relative.as_os_str().is_empty()
                || object.relative.components().any(|component|
                    !matches!(component, std::path::Component::Normal(_)))) {
            return Err(mismatch());
        }
        for (index, object) in self.objects.iter().enumerate() {
            if self.objects[..index].iter().any(|prior| prior.relative == object.relative) {
                return Err(mismatch());
            }
            check_original(object, &self.historical)?;
        }
        check_source_original(self.objects.iter().find(|object| object.source)
            .ok_or_else(mismatch)?, &self.historical)?;
        Ok(())
    }

    fn verify_live(&self, root: &RootLock, home: &Path,
        home_identity: &RootIdentity, binding: &CredentialBinding,
        allow_baseline: bool) -> Result<Vec<Dacl>, CredentialError> {
        require_bound_path(home, home_identity, true)?;
        binding.verify_registered_aliases(&[])?;
        let (source_identity, links) = CredentialBinding::observe_source_metadata(root,
            &home.join("auth.json"), home_identity)?;
        if source_identity != self.source_identity || links != 1 { return Err(mismatch()); }
        let tree = collect_legacy_owner_tree(home, Some((binding, &[])))?;
        if tree.len() + 1 != self.objects.len() { return Err(mismatch()); }
        for ((path, identity, directory, source), expected) in tree.iter()
            .zip(self.objects.iter().skip(1)) {
            if relative_path(home, path)? != expected.relative || identity != &expected.identity
                || directory != &expected.directory || source != &expected.source {
                return Err(mismatch());
            }
        }
        let baseline = &self.baseline;
        let mut observed = Vec::with_capacity(self.objects.len());
        for object in &self.objects {
            let path = home.join(&object.relative);
            let acl = observe_object(&path, &object.identity,
                object.directory, object.source, binding)?;
            if !legal_home_progress(&acl, &object.original, &self.historical)
                && !(allow_baseline && object.source && acl_equal(&acl, baseline)) {
                return Err(mismatch());
            }
            observed.push(acl);
        }
        // A protected source baseline is a separate step. It can appear only
        // after the entire noncredential HOME tree has reached its target.
        if observed.iter().zip(&self.objects).any(|(acl, object)|
            object.source && acl_equal(acl, baseline))
            && observed.iter().zip(&self.objects).any(|(acl, object)|
                !object.source && !acl_equal(acl,
                    &home_target(&object.original, &self.historical))) {
            return Err(mismatch());
        }
        require_bound_path(home, home_identity, true)?;
        binding.verify_registered_aliases(&[])?;
        Ok(observed)
    }

    /// F must durably persist the HOME intent before this call. Every object
    /// can resume from any exact subset of captured legacy SID removals.
    pub(crate) fn reconcile_home(&self, root: &RootLock, home: &Path,
        home_identity: &RootIdentity, binding: &CredentialBinding)
        -> Result<LegacyHomeReceipt, CredentialError> {
        self.reconcile_home_inner(root, home, home_identity, binding, &mut |_, _| Ok(()))
    }

    fn reconcile_home_inner(&self, root: &RootLock, home: &Path,
        home_identity: &RootIdentity, binding: &CredentialBinding,
        after_sid: &mut impl FnMut(&Path, &str) -> Result<(), CredentialError>)
        -> Result<LegacyHomeReceipt, CredentialError> {
        self.verify_structure(root, home, home_identity, binding)?;
        let before = self.verify_live(root, home, home_identity, binding, false)?;
        let profiles: Vec<_> = self.historical.iter().map(|(name, sid)| {
            let profile = AppContainerProfile::derive_for_revocation(name)?;
            if profile.package_sid_string()? != *sid { return Err(mismatch()); }
            Ok(profile)
        }).collect::<Result<_, CredentialError>>()?;
        for (index, object) in self.objects.iter().enumerate() {
            let target = home_target(&object.original, &self.historical);
            if acl_equal(&before[index], &target) { continue; }
            let path = home.join(&object.relative);
            for (profile, (name, _)) in profiles.iter().zip(&self.historical) {
                let apply = |handle| -> Result<(), CredentialError> {
                    if &file_identity(handle)? != &object.identity { return Err(mismatch()); }
                    // The parent helper removes only the known broad inherited
                    // grant. The complete DACL is checked after every write.
                    revoke_known_legacy_owner_ace(handle, profile.sid,
                        &object.identity, index == 0)?;
                    if !legal_home_progress(&read_dacl(handle)?, &object.original,
                        &self.historical) { return Err(mismatch()); }
                    Ok(())
                };
                if object.source {
                    binding.with_source_metadata_acl(apply)?;
                } else {
                    let handle = open_bound_object(&path, &object.identity, object.directory)?;
                    apply(handle.0)?;
                }
                // The hook is private; production supplies a no-op. Cloud
                // Windows tests can stop immediately after one real SID write.
                after_sid(&path, name)?;
            }
            let actual = observe_object(&path, &object.identity,
                object.directory, object.source, binding)?;
            if !acl_equal(&actual, &target) { return Err(mismatch()); }
        }
        let after = self.verify_live(root, home, home_identity, binding, false)?;
        if self.objects.iter().zip(&after).any(|(object, actual)|
            !acl_equal(actual, &home_target(&object.original, &self.historical))) {
            return Err(mismatch());
        }
        let receipt = LegacyHomeReceipt {
            home_observed_digest: self.observed_home_digest(&after),
            source_observed_digest: self.source_digest(&after[self.source_index()]),
        };
        if receipt.home_observed_digest != self.home_target_digest()
            || receipt.source_observed_digest != self.source_after_home_digest() {
            return Err(mismatch());
        }
        Ok(receipt)
    }

    /// F/H supply boot qualification and F persists the baseline intent. The
    /// source must still be the original single-link object after HOME cleanup.
    pub(crate) fn prepare_source_baseline(&self, root: &RootLock, home: &Path,
        home_identity: &RootIdentity, binding: &CredentialBinding,
        home_receipt: &LegacyHomeReceipt) -> Result<LegacySourceReceipt, CredentialError> {
        if home_receipt.home_observed_digest != self.home_target_digest()
            || home_receipt.source_observed_digest != self.source_after_home_digest() {
            return Err(mismatch());
        }
        self.verify_structure(root, home, home_identity, binding)?;
        let observed = self.verify_live(root, home, home_identity, binding, true)?;
        for (object, acl) in self.objects.iter().zip(&observed) {
            if !acl_equal(acl, &home_target(&object.original, &self.historical))
                && !(object.source && acl_equal(acl, &self.baseline)) {
                return Err(mismatch());
            }
        }
        let source = self.objects.iter().find(|object| object.source).ok_or_else(mismatch)?;
        let current = source_acl(binding)?;
        let baseline = &self.baseline;
        if acl_equal(&current, &baseline) {
            // Read-only adoption after a crash: only an already exact protected
            // ACL can set this holder's prepared bit through its custody API.
            binding.with_source_acl(|handle, _| {
                if &file_identity(handle)? != &self.source_identity
                    || !acl_equal(&read_dacl(handle)?, &baseline) { return Err(mismatch()); }
                Ok(())
            })?;
        } else {
            let target = home_target(&source.original, &self.historical);
            if !acl_equal(&current, &target) { return Err(mismatch()); }
            binding.with_source_acl(|handle, prepared| {
                if prepared || &file_identity(handle)? != &self.source_identity
                    || !acl_equal(&read_dacl(handle)?, &target) { return Err(mismatch()); }
                let (_token, _buffer, user) = host_user_sid()?;
                let system = well_known_sid("S-1-5-18")?;
                let admins = well_known_sid("S-1-5-32-544")?;
                let mut entries = [user, system.0, admins.0].map(|sid|
                    ExplicitAccessW { permissions: FILE_ALL_ACCESS, access_mode: GRANT_ACCESS,
                        inheritance: NO_INHERITANCE, trustee: TrusteeW { multiple: ptr::null_mut(),
                            multiple_operation: 0, form: TRUSTEE_IS_SID,
                            kind: TRUSTEE_IS_UNKNOWN, name: sid.cast() } });
                let mut raw_acl = ptr::null_mut();
                let status = unsafe { SetEntriesInAclW(entries.len() as u32,
                    entries.as_mut_ptr(), ptr::null_mut(), &mut raw_acl) };
                if status != 0 { return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32)).into()); }
                if raw_acl.is_null() { return Err(mismatch()); }
                let acl = LocalAllocation(raw_acl);
                let status = unsafe { SetSecurityInfo(handle, FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    ptr::null_mut(), ptr::null_mut(), acl.0, ptr::null_mut()) };
                if status != 0 { return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32)).into()); }
                if !acl_equal(&read_dacl(handle)?, &baseline) { return Err(mismatch()); }
                Ok(())
            })?;
        }
        let after = self.verify_live(root, home, home_identity, binding, true)?;
        if self.objects.iter().zip(&after).any(|(object, acl)|
            !acl_equal(acl, &if object.source { self.baseline.clone() }
                else { home_target(&object.original, &self.historical) })) {
            return Err(mismatch());
        }
        let receipt = LegacySourceReceipt {
            source_observed_digest: self.source_digest(&after[self.source_index()]),
            baseline_core_digest: self.baseline_core_digest(),
            observed_raw_digest: dacl_digest(&source_acl(binding)?) };
        if receipt.source_observed_digest != self.source_target_digest() {
            return Err(mismatch());
        }
        Ok(receipt)
    }

    /// A later holder may inherit a completed fenced baseline together with
    /// exact current observer/model grants. F/H supplies the complete allowed
    /// current SID/rights set from original custody; an allowed grant may not
    /// yet exist after a crash. This method never writes a DACL.
    pub(crate) fn adopt_existing_protected_baseline(&self, root: &RootLock,
        home: &Path, home_identity: &RootIdentity, binding: &CredentialBinding,
        registered_aliases: &[CredentialAliasScope], permitted_current: &[(String, u32)])
        -> Result<LegacySourceReceipt, CredentialError> {
        self.verify_structure(root, home, home_identity, binding)?;
        require_bound_path(home, home_identity, true)?;
        binding.verify_registered_aliases(registered_aliases)?;
        let (identity, links) = CredentialBinding::observe_source_metadata(root,
            &home.join("auth.json"), home_identity)?;
        if identity != self.source_identity
            || links as usize != registered_aliases.len() + 1 { return Err(mismatch()); }
        let observed = binding.with_source_acl(|handle, _prepared| {
            if &file_identity(handle)? != &self.source_identity { return Err(mismatch()); }
            let actual = read_dacl(handle)?;
            verify_permitted_current(&actual, &self.baseline, permitted_current)?;
            Ok(actual)
        })?;
        binding.verify_registered_aliases(registered_aliases)?;
        require_bound_path(home, home_identity, true)?;
        Ok(LegacySourceReceipt {
            source_observed_digest: self.source_digest(&observed),
            baseline_core_digest: self.baseline_core_digest(),
            observed_raw_digest: dacl_digest(&observed) })
    }

    fn source_index(&self) -> usize {
        self.objects.iter().position(|object| object.source).expect("sealed source")
    }

    fn source(&self) -> &Object { &self.objects[self.source_index()] }

    fn digest_prefix(&self, domain: &[u8]) -> Vec<u8> {
        let mut bytes = domain.to_vec();
        encode_identity(&mut bytes, &self.root_identity);
        encode_identity(&mut bytes, &self.home_identity);
        encode_identity(&mut bytes, &self.source_identity);
        bytes
    }

    fn home_digest(&self, target: bool) -> String {
        let mut bytes = self.digest_prefix(b"gogoke-legacy-home-acl-v1\0");
        for object in self.objects.iter().filter(|object| !object.source) {
            put_path(&mut bytes, &object.relative);
            encode_identity(&mut bytes, &object.identity);
            bytes.push(u8::from(object.directory));
            let acl = if target { home_target(&object.original, &self.historical) }
                else { object.original.clone() };
            encode_dacl_canonical(&mut bytes, &acl);
        }
        sha256_hex(&bytes)
    }

    fn observed_home_digest(&self, observed: &[Dacl]) -> String {
        let mut bytes = self.digest_prefix(b"gogoke-legacy-home-acl-v1\0");
        for (object, acl) in self.objects.iter().zip(observed)
            .filter(|(object, _)| !object.source) {
            put_path(&mut bytes, &object.relative);
            encode_identity(&mut bytes, &object.identity);
            bytes.push(u8::from(object.directory));
            encode_dacl_canonical(&mut bytes, acl);
        }
        sha256_hex(&bytes)
    }

    fn source_digest(&self, acl: &Dacl) -> String {
        let mut bytes = self.digest_prefix(b"gogoke-legacy-source-acl-v1\0");
        put_path(&mut bytes, &self.source().relative);
        encode_dacl_canonical(&mut bytes, acl);
        sha256_hex(&bytes)
    }

    fn bytes(&self) -> Vec<u8> {
        let mut bytes = b"gogoke-legacy-acl-v1\0".to_vec();
        encode_identity(&mut bytes, &self.root_identity);
        encode_identity(&mut bytes, &self.home_identity);
        encode_identity(&mut bytes, &self.source_identity);
        put_u32(&mut bytes, self.historical.len() as u32);
        for (name, sid) in &self.historical { put_str(&mut bytes, name); put_str(&mut bytes, sid); }
        encode_dacl(&mut bytes, &self.baseline);
        put_u32(&mut bytes, self.objects.len() as u32);
        for object in &self.objects {
            put_path(&mut bytes, &object.relative);
            encode_identity(&mut bytes, &object.identity);
            bytes.push(u8::from(object.directory));
            bytes.push(u8::from(object.source));
            encode_dacl(&mut bytes, &object.original);
        }
        bytes
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, CredentialError> {
        let mut reader = Reader { bytes, index: 0 };
        if reader.take(21)? != b"gogoke-legacy-acl-v1\0" { return Err(mismatch()); }
        let root_identity = reader.identity()?;
        let home_identity = reader.identity()?;
        let source_identity = reader.identity()?;
        let count = reader.u32()? as usize;
        if count == 0 || count > 4096 { return Err(mismatch()); }
        let mut historical = Vec::with_capacity(count);
        for _ in 0..count { historical.push((reader.string()?, reader.string()?)); }
        let baseline = reader.dacl()?;
        let count = reader.u32()? as usize;
        if count < 2 || count > 100_000 { return Err(mismatch()); }
        let mut objects = Vec::with_capacity(count);
        for _ in 0..count {
            let relative = reader.path()?;
            let identity = reader.identity()?;
            let directory = reader.byte()? == 1;
            let source = reader.byte()? == 1;
            let original = reader.dacl()?;
            objects.push(Object { relative, identity, directory, source, original });
        }
        if reader.index != bytes.len() { return Err(mismatch()); }
        Ok(Self { root_identity, home_identity, source_identity, historical, baseline, objects })
    }
}

fn put_u32(bytes: &mut Vec<u8>, value: u32) { bytes.extend(value.to_le_bytes()); }
fn put_str(bytes: &mut Vec<u8>, value: &str) {
    put_u32(bytes, value.len() as u32); bytes.extend(value.as_bytes());
}
fn put_path(bytes: &mut Vec<u8>, path: &Path) {
    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    put_u32(bytes, wide.len() as u32);
    for unit in wide { bytes.extend(unit.to_le_bytes()); }
}
fn encode_identity(bytes: &mut Vec<u8>, identity: &RootIdentity) {
    bytes.extend(identity.volume_serial.to_le_bytes()); bytes.extend(identity.file_id);
}
fn encode_dacl(bytes: &mut Vec<u8>, acl: &Dacl) {
    bytes.push(u8::from(acl.protected)); put_u32(bytes, acl.aces.len() as u32);
    for ace in &acl.aces {
        bytes.push(ace.kind); bytes.push(ace.flags); put_u32(bytes, ace.mask); put_str(bytes, &ace.sid);
    }
}
fn encode_dacl_canonical(bytes: &mut Vec<u8>, acl: &Dacl) {
    let mut ordered = acl.clone();
    if ordered.aces.iter().all(|ace| ace.kind == ACCESS_ALLOWED_ACE_TYPE) {
        ordered.aces.sort_by(|left, right| (&left.sid, left.mask, left.flags)
            .cmp(&(&right.sid, right.mask, right.flags)));
    }
    encode_dacl(bytes, &ordered);
}
fn dacl_digest(acl: &Dacl) -> String {
    let mut bytes = Vec::new(); encode_dacl(&mut bytes, acl); sha256_hex(&bytes)
}
fn canonical_dacl_digest(acl: &Dacl) -> String {
    let mut bytes = Vec::new(); encode_dacl_canonical(&mut bytes, acl); sha256_hex(&bytes)
}
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes { output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 15) as usize] as char); }
    output
}
fn unhex(value: &str) -> Result<Vec<u8>, CredentialError> {
    if value.len() % 2 != 0 || value.len() > 32_000_000 { return Err(mismatch()); }
    let mut output = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().chunks_exact(2) {
        let digit = |byte: u8| -> Option<u8> { match byte {
            b'0'..=b'9' => Some(byte - b'0'), b'a'..=b'f' => Some(byte - b'a' + 10), _ => None } };
        output.push((digit(pair[0]).ok_or_else(mismatch)? << 4)
            | digit(pair[1]).ok_or_else(mismatch)?);
    }
    Ok(output)
}
struct Reader<'a> { bytes: &'a [u8], index: usize }
impl Reader<'_> {
    fn take(&mut self, length: usize) -> Result<&[u8], CredentialError> {
        let end = self.index.checked_add(length).ok_or_else(mismatch)?;
        if end > self.bytes.len() { return Err(mismatch()); }
        let part = &self.bytes[self.index..end]; self.index = end; Ok(part)
    }
    fn byte(&mut self) -> Result<u8, CredentialError> { Ok(self.take(1)?[0]) }
    fn u32(&mut self) -> Result<u32, CredentialError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| mismatch())?))
    }
    fn string(&mut self) -> Result<String, CredentialError> {
        let length = self.u32()? as usize;
        if length > 1_000_000 { return Err(mismatch()); }
        String::from_utf8(self.take(length)?.to_vec()).map_err(|_| mismatch())
    }
    fn path(&mut self) -> Result<PathBuf, CredentialError> {
        let length = self.u32()? as usize;
        if length > 32_767 { return Err(mismatch()); }
        let mut wide = Vec::with_capacity(length);
        for _ in 0..length {
            wide.push(u16::from_le_bytes(self.take(2)?.try_into().map_err(|_| mismatch())?));
        }
        Ok(PathBuf::from(std::ffi::OsString::from_wide(&wide)))
    }
    fn identity(&mut self) -> Result<RootIdentity, CredentialError> {
        let volume_serial = u64::from_le_bytes(self.take(8)?.try_into().map_err(|_| mismatch())?);
        let file_id = self.take(16)?.try_into().map_err(|_| mismatch())?;
        Ok(RootIdentity { volume_serial, file_id })
    }
    fn dacl(&mut self) -> Result<Dacl, CredentialError> {
        let protected = match self.byte()? { 0 => false, 1 => true, _ => return Err(mismatch()) };
        let count = self.u32()? as usize;
        if count > 100_000 { return Err(mismatch()); }
        let mut aces = Vec::with_capacity(count);
        for _ in 0..count {
            let kind = self.byte()?;
            let flags = self.byte()?;
            let mask = self.u32()?;
            let sid = self.string()?;
            if !matches!(kind, ACCESS_ALLOWED_ACE_TYPE | ACCESS_DENIED_ACE_TYPE)
                || !sid.starts_with("S-1-") { return Err(mismatch()); }
            aces.push(Ace { kind, flags, mask, sid });
        }
        Ok(Dacl { protected, aces })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recovery_fixture(other_rights: u32) -> (RootLock, PathBuf, PathBuf, RootIdentity,
        std::sync::Arc<CredentialBinding>, AppContainerProfile, AppContainerProfile) {
        use crate::root::inspect_root;
        use std::time::{SystemTime, UNIX_EPOCH};
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let requested = std::env::temp_dir().join(format!(
            "gogoke-holder-acl-recovery-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&requested).unwrap();
        let root = RootLock::acquire(&requested).unwrap();
        let home = root.canonical_root().canonical_path.join("home");
        std::fs::create_dir(&home).unwrap();
        let home_identity = inspect_root(&home).unwrap().identity;
        let source = home.join("auth.json");
        std::fs::File::create(&source).unwrap(); // Metadata-only empty synthetic file.
        let (identity, _) = CredentialBinding::observe_source_metadata(&root, &source, &home_identity).unwrap();
        let binding = CredentialBinding::open_registered(&root, &source, &home_identity, &identity, &[]).unwrap();
        protect_credential_source_acl(&binding).unwrap();
        let target = AppContainerProfile::derive_for_revocation(&format!("Gogoke37.RecoveryA.{nonce}")).unwrap();
        let other = AppContainerProfile::derive_for_revocation(&format!("Gogoke37.RecoveryB.{nonce}")).unwrap();
        binding.with_source_metadata_acl(|handle| {
            grant_exact_acl(handle, target.sid, &identity, CREDENTIAL_FILE_RIGHTS, NO_INHERITANCE)?;
            grant_exact_acl(handle, other.sid, &identity, other_rights, NO_INHERITANCE)?;
            Ok(())
        }).unwrap();
        // A new holder must recover without invoking the fresh-baseline writer.
        drop(binding);
        let binding = CredentialBinding::open_registered(&root, &source, &home_identity, &identity, &[]).unwrap();
        assert!(!binding.acl_prepared_in_this_holder().unwrap());
        (root, requested, home, home_identity, binding, target, other)
    }

    #[test]
    fn holder_gone_recovery_removes_one_real_rw_sid_preserves_other_and_reads_target_without_write() {
        let (root, requested, home, home_identity, binding, target, other) = recovery_fixture(FILE_GENERIC_READ);
        let target_sid = target.package_sid_string().unwrap();
        let other_sid = other.package_sid_string().unwrap();
        let known = [(target_sid.clone(), CREDENTIAL_FILE_RIGHTS), (other_sid.clone(), FILE_GENERIC_READ)];
        let captured = NativeCredentialAclRecoveryStep::capture(&binding, &target_sid, &known).unwrap();
        let before = source_acl(&binding).unwrap();
        let snapshot = captured.encode_snapshot();
        let step = NativeCredentialAclRecoveryStep::restore(&snapshot, &captured.before_digest(),
            &target_sid, &known).unwrap();
        let pairs = [(u32::MAX, 1)];
        let proof = NativeProcessHoldersGone::observe(&pairs).unwrap();
        assert_eq!(step.apply_or_readback(&binding, &proof, &pairs).unwrap(), step.target_digest());
        let after = source_acl(&binding).unwrap();
        assert_eq!(after.aces, before.aces.iter().filter(|ace| ace.sid != target_sid)
            .cloned().collect::<Vec<_>>(), "every other ACE and its order is preserved");
        assert!(after.aces.iter().any(|ace| ace.sid == other_sid && ace.mask == FILE_GENERIC_READ));
        assert!(!binding.acl_prepared_in_this_holder().unwrap(), "removal never adopts/resets a baseline");
        step.apply_or_readback_inner(&binding, &proof, &pairs, &mut |_, _, _|
            panic!("an already exact target must not call the DACL writer")).unwrap();
        assert_eq!(source_acl(&binding).unwrap(), after);
        assert!(adopt_holder_gone_source_baseline(&binding, &step.before_digest(),
            &[(other_sid.clone(), FILE_GENERIC_READ)]).is_err());
        assert!(!binding.acl_prepared_in_this_holder().unwrap());
        assert_eq!(adopt_holder_gone_source_baseline(&binding, &step.target_digest(),
            &[(other_sid, FILE_GENERIC_READ)]).unwrap(), step.target_digest());
        assert!(binding.acl_prepared_in_this_holder().unwrap());
        assert_eq!(source_acl(&binding).unwrap(), after, "adoption is read-only");
        // Capture from the already-target shape is a legitimate no-write step.
        let absent = NativeCredentialAclRecoveryStep::capture(&binding, &target_sid, &known).unwrap();
        assert_eq!(absent.before_digest(), absent.target_digest());
        absent.apply_or_readback_inner(&binding, &proof, &pairs, &mut |_, _, _|
            panic!("absent target SID must not rewrite the DACL")).unwrap();
        binding.verify_registered_aliases(&[]).unwrap();
        assert_eq!(CredentialBinding::observe_source_metadata(&root, &home.join("auth.json"),
            &home_identity).unwrap().0, *binding.identity());
        drop(binding); drop(root);
        std::fs::remove_dir_all(requested).unwrap();
    }

    #[test]
    fn holder_gone_recovery_rejects_unknown_sid_wrong_rights_and_changed_full_acl() {
        let (root, requested, _home, _home_identity, binding, target, other) = recovery_fixture(CREDENTIAL_FILE_RIGHTS);
        let target_sid = target.package_sid_string().unwrap();
        let other_sid = other.package_sid_string().unwrap();
        let known = [(target_sid.clone(), CREDENTIAL_FILE_RIGHTS), (other_sid.clone(), CREDENTIAL_FILE_RIGHTS)];
        assert!(NativeCredentialAclRecoveryStep::capture(&binding, &target_sid,
            &[(target_sid.clone(), FILE_GENERIC_READ), (other_sid.clone(), CREDENTIAL_FILE_RIGHTS)]).is_err(),
            "the new recovery must not target an old READ observer");
        assert!(NativeCredentialAclRecoveryStep::capture(&binding, &target_sid,
            &[(other_sid.clone(), CREDENTIAL_FILE_RIGHTS)]).is_err());
        assert!(NativeCredentialAclRecoveryStep::capture(&binding, &target_sid,
            &[(target_sid.clone(), FILE_ALL_ACCESS), (other_sid.clone(), CREDENTIAL_FILE_RIGHTS)]).is_err());
        let step = NativeCredentialAclRecoveryStep::capture(&binding, &target_sid, &known).unwrap();
        let before = source_acl(&binding).unwrap();
        let unknown = AppContainerProfile::derive_for_revocation("Gogoke37.UnknownRecoverySid").unwrap();
        binding.with_source_metadata_acl(|handle| {
            grant_exact_acl(handle, unknown.sid, binding.identity(), CREDENTIAL_FILE_RIGHTS, NO_INHERITANCE)?;
            Ok(())
        }).unwrap();
        let polluted = source_acl(&binding).unwrap();
        assert!(NativeCredentialAclRecoveryStep::capture(&binding, &target_sid, &known).is_err());
        assert!(adopt_holder_gone_source_baseline(&binding, &canonical_dacl_digest(&polluted), &known).is_err());
        assert!(!binding.acl_prepared_in_this_holder().unwrap());
        let pairs = [(u32::MAX, 1)];
        let proof = NativeProcessHoldersGone::observe(&pairs).unwrap();
        assert!(step.apply_or_readback_inner(&binding, &proof, &pairs, &mut |_, _, _|
            panic!("an unknown current ACE must reject before any write")).is_err());
        assert_eq!(source_acl(&binding).unwrap(), polluted, "failure cannot reset the unknown SID");
        binding.with_source_metadata_acl(|handle| {
            revoke_exact_credential_ace(handle, unknown.sid, binding.identity())?;
            Ok(())
        }).unwrap();
        assert_eq!(source_acl(&binding).unwrap(), before);
        // Removing a different captured SID is neither this step's before nor
        // target, even though every remaining ACE is independently permitted.
        let mut changed = before.clone();
        changed.aces.retain(|ace| ace.sid != other_sid);
        binding.with_source_metadata_acl(|handle| {
            revoke_exact_credential_ace(handle, other.sid, binding.identity())?;
            Ok(())
        }).unwrap();
        assert!(acl_equal(&source_acl(&binding).unwrap(), &changed));
        assert!(step.apply_or_readback_inner(&binding, &proof, &pairs, &mut |_, _, _|
            panic!("an unrelated ACE removal cannot authorize this write")).is_err());
        assert!(acl_equal(&source_acl(&binding).unwrap(), &changed));
        drop(binding); drop(root);
        std::fs::remove_dir_all(requested).unwrap();
    }

    #[test]
    fn holder_gone_recovery_snapshot_and_current_proof_remain_bound() {
        let (root, requested, home, _home_identity, binding, target, other) = recovery_fixture(FILE_GENERIC_READ);
        let target_sid = target.package_sid_string().unwrap();
        let other_sid = other.package_sid_string().unwrap();
        let known = [(target_sid.clone(), CREDENTIAL_FILE_RIGHTS), (other_sid.clone(), FILE_GENERIC_READ)];
        let mut step = NativeCredentialAclRecoveryStep::capture(&binding, &target_sid, &known).unwrap();
        let snapshot = step.encode_snapshot();
        let before_digest = step.before_digest();
        assert!(NativeCredentialAclRecoveryStep::restore(&snapshot, &"0".repeat(64), &target_sid, &known).is_err());
        assert!(NativeCredentialAclRecoveryStep::restore(&snapshot, &before_digest, &other_sid, &known).is_err());
        assert!(NativeCredentialAclRecoveryStep::restore(&snapshot, &before_digest, &target_sid,
            &[(target_sid.clone(), CREDENTIAL_FILE_RIGHTS)]).is_err());
        for change in 0..4 {
            let mut malformed = NativeCredentialAclRecoveryStep::restore(&snapshot, &before_digest,
                &target_sid, &known).unwrap();
            let target_ace = malformed.before.aces.iter_mut().find(|ace| ace.sid == target_sid).unwrap();
            match change {
                0 => target_ace.flags = OBJECT_AND_CONTAINER_INHERIT as u8,
                1 => target_ace.mask = FILE_GENERIC_READ,
                2 => target_ace.kind = ACCESS_DENIED_ACE_TYPE,
                _ => malformed.before.protected = false,
            }
            malformed.target = remove_recovery_sid(&malformed.before, &target_sid);
            assert!(NativeCredentialAclRecoveryStep::restore(&malformed.encode_snapshot(),
                &malformed.before_digest(), &target_sid, &known).is_err(),
                "even a matching digest cannot qualify altered rights/inheritance/protection");
        }
        step.target.aces.retain(|ace| ace.sid != other_sid);
        assert!(NativeCredentialAclRecoveryStep::restore(&step.encode_snapshot(), &before_digest,
            &target_sid, &known).is_err(), "snapshot must encode exactly one removal");
        let step = NativeCredentialAclRecoveryStep::restore(&snapshot, &before_digest, &target_sid, &known).unwrap();
        let before = source_acl(&binding).unwrap();
        let proof = NativeProcessHoldersGone::observe(&[(u32::MAX, 1)]).unwrap();
        assert!(step.apply_or_readback(&binding, &proof, &[(u32::MAX, 2)]).is_err());
        #[link(name = "kernel32")]
        extern "system" { fn GetProcessTimes(process: Handle, creation: *mut FileTime,
            exit: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32; }
        let [mut creation, mut exit, mut kernel, mut user] = [FileTime { low: 0, high: 0 }; 4];
        assert_ne!(unsafe { GetProcessTimes(GetCurrentProcess(), &mut creation, &mut exit,
            &mut kernel, &mut user) }, 0);
        let exact = [(std::process::id(), (u64::from(creation.high) << 32) | u64::from(creation.low))];
        let live = NativeProcessHoldersGone::for_test(&exact);
        let error = step.apply_or_readback(&binding, &live, &exact).unwrap_err();
        assert!(format!("{error}").contains("exact holder remains alive"), "native failure preserved: {error}");
        assert_eq!(source_acl(&binding).unwrap(), before, "live proof cannot authorize writing");
        let mut different = NativeCredentialAclRecoveryStep::restore(&snapshot, &before_digest, &target_sid, &known).unwrap();
        different.source_identity.file_id[0] ^= 1;
        assert!(matches!(different.apply_or_readback(&binding, &proof, &[(u32::MAX, 1)]),
            Err(CredentialError::IdentityChanged)));
        // A namespace alias absent from F's complete registry fails closed.
        std::fs::hard_link(home.join("auth.json"), home.join("unregistered.json")).unwrap();
        assert!(NativeCredentialAclRecoveryStep::capture(&binding, &target_sid, &known).is_err());
        assert!(step.apply_or_readback(&binding, &proof, &[(u32::MAX, 1)]).is_err());
        assert!(adopt_holder_gone_source_baseline(&binding, &before_digest, &known).is_err());
        drop(binding); drop(root);
        std::fs::remove_dir_all(requested).unwrap();
    }

    #[test]
    fn snapshot_rejects_tampering_and_unknown_package_grants() {
        let known = vec![("old".to_owned(), "S-1-15-2-123".to_owned())];
        let object = Object { relative: PathBuf::new(), identity: RootIdentity {
            volume_serial: 1, file_id: [2; 16] }, directory: true, source: false,
            original: Dacl { protected: false, aces: vec![Ace {
                kind: ACCESS_ALLOWED_ACE_TYPE, flags: 3,
                mask: directory_rights(true), sid: "S-1-15-2-456".into() }] } };
        assert!(check_original(&object, &known).is_err());
        let inventory = LegacyAclInventory { root_identity: object.identity.clone(),
            home_identity: object.identity.clone(), source_identity: object.identity.clone(),
            historical: known, baseline: Dacl { protected: true, aces: vec![] },
            objects: vec![object] };
        let payload = inventory.encode_snapshot();
        assert_eq!(unhex(&payload).unwrap(), inventory.bytes());
        assert!(LegacyAclInventory::from_bytes(&unhex(&payload).unwrap()).is_err(),
            "a one-object fixture cannot encode both HOME and the auth source");
        assert!(unhex(&format!("{}z", payload)).is_err());
    }

    #[test]
    fn exact_acl_target_rejects_inheritance_or_extra_principal() {
        let original = Dacl { protected: false, aces: vec![
            Ace { kind: ACCESS_ALLOWED_ACE_TYPE, flags: 3,
                mask: directory_rights(true), sid: "S-1-15-2-123".into() },
            Ace { kind: ACCESS_ALLOWED_ACE_TYPE, flags: 0,
                mask: FILE_ALL_ACCESS, sid: "S-1-5-18".into() },
        ] };
        let historical = [("old".into(), "S-1-15-2-123".into())];
        let target = home_target(&original, &historical);
        assert_eq!(target.aces.len(), 1);
        assert!(!acl_equal(&target, &original));
        let mut changed = target.clone();
        changed.aces[0].flags = INHERITED_ACE as u8;
        assert!(!acl_equal(&changed, &target));
        changed = target.clone();
        changed.aces.push(Ace { kind: ACCESS_ALLOWED_ACE_TYPE, flags: 0,
            mask: FILE_GENERIC_READ, sid: "S-1-15-2-999".into() });
        assert!(!acl_equal(&changed, &target));
        let deny = Ace { kind: ACCESS_DENIED_ACE_TYPE, flags: 0,
            mask: FILE_GENERIC_READ, sid: "S-1-5-21-123".into() };
        let ordered = Dacl { protected: false, aces: vec![deny.clone(), target.aces[0].clone()] };
        let reordered = Dacl { protected: false, aces: vec![target.aces[0].clone(), deny] };
        assert!(!acl_equal(&ordered, &reordered));
    }

    #[test]
    fn partial_legacy_subset_preserves_all_other_aces_and_deny_order() {
        let a = Ace { kind: ACCESS_ALLOWED_ACE_TYPE, flags: 3,
            mask: directory_rights(true), sid: "S-1-15-2-123".into() };
        let b = Ace { sid: "S-1-15-2-456".into(), ..a.clone() };
        let deny = Ace { kind: ACCESS_DENIED_ACE_TYPE, flags: 0,
            mask: FILE_GENERIC_READ, sid: "S-1-5-21-123".into() };
        let owner = Ace { kind: ACCESS_ALLOWED_ACE_TYPE, flags: 0,
            mask: FILE_ALL_ACCESS, sid: "S-1-5-18".into() };
        let original = Dacl { protected: false,
            aces: vec![deny.clone(), a, owner.clone(), b.clone()] };
        let historical = [("a".into(), "S-1-15-2-123".into()),
            ("b".into(), "S-1-15-2-456".into())];
        let partial = Dacl { protected: false,
            aces: vec![deny.clone(), owner.clone(), b.clone()] };
        assert!(legal_home_progress(&partial, &original, &historical));
        assert!(!acl_equal(&partial, &original));
        assert!(!acl_equal(&partial, &home_target(&original, &historical)));
        let mut changed = partial.clone();
        changed.aces[2].flags = 0;
        assert!(!legal_home_progress(&changed, &original, &historical));
        changed = partial.clone();
        changed.aces.remove(1);
        assert!(!legal_home_progress(&changed, &original, &historical));
        changed = Dacl { protected: false, aces: vec![owner, deny, b] };
        assert!(!legal_home_progress(&changed, &original, &historical));
    }

    #[test]
    fn home_and_source_digests_bind_separate_acl_steps() {
        let identity = RootIdentity { volume_serial: 1, file_id: [2; 16] };
        let legacy = Ace { kind: ACCESS_ALLOWED_ACE_TYPE, flags: 3,
            mask: directory_rights(true), sid: "S-1-15-2-123".into() };
        let home = Object { relative: PathBuf::new(), identity: identity.clone(),
            directory: true, source: false,
            original: Dacl { protected: false, aces: vec![legacy.clone()] } };
        let source = Object { relative: PathBuf::from("auth.json"), identity: identity.clone(),
            directory: false, source: true,
            original: Dacl { protected: false, aces: vec![Ace {
                flags: INHERITED_ACE as u8, ..legacy
            }] } };
        let mut inventory = LegacyAclInventory { root_identity: identity.clone(),
            home_identity: identity.clone(), source_identity: identity,
            historical: vec![("old".into(), "S-1-15-2-123".into())],
            baseline: Dacl { protected: true, aces: vec![] },
            objects: vec![home, source] };
        let home_digest = inventory.home_original_digest();
        let source_digest = inventory.source_original_digest();
        assert_ne!(home_digest, inventory.home_target_digest());
        assert_ne!(source_digest, inventory.source_after_home_digest());
        inventory.objects[1].original.aces[0].mask = FILE_GENERIC_READ;
        assert_eq!(home_digest, inventory.home_original_digest());
        assert_ne!(source_digest, inventory.source_original_digest());
    }

    #[test]
    fn protected_baseline_adopts_optional_exact_current_read_only() {
        let baseline = Dacl { protected: true, aces: ["S-1-5-21-123", "S-1-5-18",
            "S-1-5-32-544"].into_iter().map(|sid| Ace {
                kind: ACCESS_ALLOWED_ACE_TYPE, flags: 0,
                mask: FILE_ALL_ACCESS, sid: sid.into(),
            }).collect() };
        let observer = ("S-1-15-2-123".to_owned(), FILE_GENERIC_READ);
        let permitted = [observer.clone()];
        // Crash after baseline APPLIED but before the optional new observer
        // grant is a valid protected baseline, even with a permitted SID.
        assert!(verify_permitted_current(&baseline, &baseline, &permitted).is_ok());
        let mut with_read = baseline.clone();
        with_read.aces.push(Ace { kind: ACCESS_ALLOWED_ACE_TYPE, flags: 0,
            mask: FILE_GENERIC_READ, sid: observer.0.clone() });
        assert!(verify_permitted_current(&with_read, &baseline, &permitted).is_ok());
        let mut unknown = with_read.clone();
        unknown.aces[3].sid = "S-1-15-2-999".into();
        assert!(verify_permitted_current(&unknown, &baseline, &permitted).is_err());
        let mut broad = with_read.clone();
        broad.aces[3].mask = directory_rights(true);
        broad.aces[3].flags = INHERITED_ACE as u8;
        assert!(verify_permitted_current(&broad, &baseline, &permitted).is_err());
        let mut duplicate = with_read.clone();
        duplicate.aces.push(duplicate.aces[3].clone());
        assert!(verify_permitted_current(&duplicate, &baseline, &permitted).is_err());
    }

    #[test]
    fn original_source_accepts_only_exact_inherited_local_read_execute() {
        let mut source = Object { relative: PathBuf::from("auth.json"),
            identity: RootIdentity { volume_serial: 1, file_id: [2; 16] },
            directory: false, source: true,
            original: baseline_target().unwrap() };
        source.original.protected = false;
        let extra = Ace { kind: ACCESS_ALLOWED_ACE_TYPE,
            flags: INHERITED_ACE as u8,
            mask: FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
            sid: "S-1-5-21-111-222-333-1002".into() };
        source.original.aces.push(extra.clone());
        assert!(check_source_original(&source, &[]).is_ok());
        source.original.aces.push(Ace { sid: "S-1-5-21-111-222-333-1003".into(),
            ..extra.clone() });
        assert!(check_source_original(&source, &[]).is_ok(),
            "the captured set, rather than an incidental count, is authoritative");
        source.original.aces.pop();
        for changed in [
            Ace { mask: FILE_GENERIC_READ | FILE_GENERIC_WRITE, ..extra.clone() },
            Ace { flags: 0, ..extra.clone() },
            Ace { kind: ACCESS_DENIED_ACE_TYPE, ..extra.clone() },
            Ace { sid: "S-1-15-2-999".into(), ..extra.clone() },
            Ace { sid: "S-1-5-32-545".into(), ..extra.clone() },
        ] {
            *source.original.aces.last_mut().unwrap() = changed;
            assert!(check_source_original(&source, &[]).is_err());
        }
    }

    #[test]
    fn two_real_legacy_sids_resume_after_first_acl_write_and_baseline_readback() {
        use crate::root::inspect_root;
        use std::fs;
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let requested = std::env::temp_dir().join(format!(
            "gogoke-legacy-acl-resume-{}-{nonce}", std::process::id()));
        fs::create_dir(&requested).unwrap();
        let root = RootLock::acquire(&requested).unwrap();
        let home = root.canonical_root().canonical_path.join("legacy-home");
        fs::create_dir(&home).unwrap();
        let home_identity = inspect_root(&home).unwrap().identity;
        let names = vec![
            format!("Gogoke37.LegacyA.{}.{nonce}", std::process::id()),
            format!("Gogoke37.LegacyB.{}.{nonce}", std::process::id()),
        ];
        for name in &names {
            AppContainerProfile::derive_for_revocation(name).unwrap()
                .grant_bound_tree(&home, &home_identity, true).unwrap();
        }
        let extra = well_known_sid("S-1-5-21-111-222-333-1002").unwrap();
        let extra_text = sid_text(extra.0).unwrap();
        let home_handle = open_bound_object(&home, &home_identity, true).unwrap();
        grant_exact_acl(home_handle.0, extra.0, &home_identity,
            FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
            OBJECT_AND_CONTAINER_INHERIT).unwrap();
        drop(home_handle);
        // Creating ordinary children after both real HOME grants makes the
        // inherited ACE shapes deterministic on Windows. The synthetic extra
        // local-user SID is nonpackage, read/execute only, and is not a model.
        let history = home.join("sessions").join("old.jsonl");
        fs::create_dir_all(history.parent().unwrap()).unwrap();
        fs::File::create(&history).unwrap();
        let source = home.join("auth.json");
        fs::File::create(&source).unwrap();
        let (source_identity, links) = CredentialBinding::observe_source_metadata(
            &root, &source, &home_identity).unwrap();
        assert_eq!(links, 1);
        let binding = CredentialBinding::open_registered(&root, &source,
            &home_identity, &source_identity, &[]).unwrap();
        let inventory = LegacyAclInventory::capture(&root, &home,
            &home_identity, &binding, &names).unwrap();
        let original_source = &inventory.source().original;
        assert!(original_source.aces.iter().any(|ace|
            ace.sid == extra_text && is_exact_inherited_local_read_execute(ace)));
        let encoded = inventory.encode_snapshot();
        let original_digest = inventory.original_digest();
        let mut writes = 0;
        let interrupted = inventory.reconcile_home_inner(&root, &home,
            &home_identity, &binding, &mut |_, _| {
                writes += 1;
                Err(CredentialError::Invalid("test interruption after first SID write"))
            });
        assert!(interrupted.is_err());
        assert_eq!(writes, 1);
        let partial = read_dacl(open_physical_object(&home, true, READ_CONTROL).unwrap().0)
            .unwrap();
        assert!(legal_home_progress(&partial, &inventory.objects[0].original,
            &inventory.historical));
        assert!(!acl_equal(&partial, &inventory.objects[0].original));
        assert!(!acl_equal(&partial,
            &home_target(&inventory.objects[0].original, &inventory.historical)));
        drop(inventory);
        drop(binding);

        let binding = CredentialBinding::open_registered(&root, &source,
            &home_identity, &source_identity, &[]).unwrap();
        let restored = LegacyAclInventory::restore(&root, &home, &home_identity,
            &binding, &names, &encoded, &original_digest).unwrap();
        let receipt = restored.reconcile_home(&root, &home,
            &home_identity, &binding).unwrap();
        assert_eq!(receipt.home_observed_digest, restored.home_target_digest());
        assert_eq!(receipt.source_observed_digest, restored.source_after_home_digest());
        let target = open_physical_object(&home, true, READ_CONTROL).unwrap();
        assert!(acl_equal(&read_dacl(target.0).unwrap(),
            &home_target(&restored.objects[0].original, &restored.historical)));
        drop(target);
        for object in &restored.objects {
            let actual = observe_object(&home.join(&object.relative), &object.identity,
                object.directory, object.source, &binding).unwrap();
            assert!(acl_equal(&actual,
                &home_target(&object.original, &restored.historical)),
                "non-target ACEs changed on {:?}", object.relative);
        }
        let migrated_source = source_acl(&binding).unwrap();
        assert!(migrated_source.aces.iter().any(|ace|
            ace.sid == extra_text && is_exact_inherited_local_read_execute(ace)),
            "HOME migration must retain the sealed nonpackage read/execute ACE");
        restored.prepare_source_baseline(&root, &home, &home_identity,
            &binding, &receipt).unwrap();
        let protected_source = source_acl(&binding).unwrap();
        assert!(acl_equal(&protected_source, &baseline_target().unwrap()));
        assert!(!protected_source.aces.iter().any(|ace| ace.sid == extra_text));
        drop(binding);

        // Simulate a crash after the protected ACL write but before F's result
        // journal: a fresh holder must read back the exact already-written ACL.
        let binding = CredentialBinding::open_registered(&root, &source,
            &home_identity, &source_identity, &[]).unwrap();
        assert!(!binding.acl_prepared_in_this_holder().unwrap());
        let restored = LegacyAclInventory::restore(&root, &home, &home_identity,
            &binding, &names, &encoded, &original_digest).unwrap();
        let baseline = restored.prepare_source_baseline(&root, &home,
            &home_identity, &binding, &receipt).unwrap();
        assert_eq!(baseline.source_observed_digest, restored.source_target_digest());
        assert!(binding.acl_prepared_in_this_holder().unwrap());
        drop(binding);
        drop(root);
        fs::remove_dir_all(requested).unwrap();
    }
}
