//! Metadata-only recovery of the exact pre-boot legacy AppContainer ACLs.
//! F persists this inventory before any ACL mutation. A restored inventory is
//! never reconstructed from the possibly partially changed live DACL.

use super::*;
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
pub(crate) struct LegacyHomeReceipt { pub(crate) observed_digest: String }
#[derive(Clone, Debug)]
pub(crate) struct LegacySourceReceipt {
    /// Present only for the initial sealed-tree preparation. Later adoption
    /// validates only the source core plus F/H's complete current grants.
    pub(crate) target_digest: Option<String>,
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
    // Windows may canonicalize allow ACE order. Compare the entire ACE multiset;
    // no principal, mask, or inheritance flag may be added or changed.
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
    if source.original.aces.iter().any(|ace| !historical.iter().any(|(_, sid)| sid == &ace.sid)
        && (!allowed.contains(&&ace.sid) || ace.kind != ACCESS_ALLOWED_ACE_TYPE)) {
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
    pub(crate) fn home_target_digest(&self) -> String {
        sha256_hex(&self.target_bytes(false))
    }
    pub(crate) fn source_target_digest(&self) -> String {
        sha256_hex(&self.target_bytes(true))
    }
    pub(crate) fn baseline_core_digest(&self) -> String { dacl_digest(&self.baseline) }

    pub(crate) fn restore(root: &RootLock, home: &Path,
        home_identity: &RootIdentity, binding: &CredentialBinding,
        historical_profile_names: &[String], payload: &str, original_digest: &str)
        -> Result<Self, CredentialError> {
        let inventory = Self::parse_saved(root, home_identity, binding,
            historical_profile_names, payload, original_digest)?;
        inventory.verify_structure(root, home, home_identity, binding)?;
        let live = inventory.verify_live(root, home, home_identity, binding, true)?;
        if live.iter().zip(&inventory.objects).any(|(acl, object)| object.source
            && acl_equal(acl, &inventory.baseline))
            && live.iter().zip(&inventory.objects).any(|(acl, object)| !object.source
                && !acl_equal(acl, &home_target(&object.original, &inventory.historical))) {
            return Err(mismatch());
        }
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
            let target = home_target(&object.original, &self.historical);
            if !acl_equal(&acl, &object.original) && !acl_equal(&acl, &target)
                && !(allow_baseline && object.source && acl_equal(&acl, baseline)) {
                return Err(mismatch());
            }
            observed.push(acl);
        }
        require_bound_path(home, home_identity, true)?;
        binding.verify_registered_aliases(&[])?;
        Ok(observed)
    }

    /// F must durably persist the HOME intent before this call. Each object is
    /// independently at its captured original or exact target ACL, so a crash
    /// resumes without broadening the historical SID set.
    pub(crate) fn reconcile_home(&self, root: &RootLock, home: &Path,
        home_identity: &RootIdentity, binding: &CredentialBinding)
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
            for profile in &profiles {
                let apply = |handle| -> Result<(), CredentialError> {
                    if &file_identity(handle)? != &object.identity { return Err(mismatch()); }
                    // The parent helper removes only the known broad inherited
                    // grant. The complete DACL is checked after every write.
                    revoke_known_legacy_owner_ace(handle, profile.sid,
                        &object.identity, index == 0)?;
                    Ok(())
                };
                if object.source {
                    binding.with_source_metadata_acl(apply)?;
                } else {
                    let handle = open_bound_object(&path, &object.identity, object.directory)?;
                    apply(handle.0)?;
                }
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
        Ok(LegacyHomeReceipt { observed_digest: self.observed_digest(&after) })
    }

    /// F/H supply boot qualification and F persists the baseline intent. The
    /// source must still be the original single-link object after HOME cleanup.
    pub(crate) fn prepare_source_baseline(&self, root: &RootLock, home: &Path,
        home_identity: &RootIdentity, binding: &CredentialBinding,
        home_receipt: &LegacyHomeReceipt) -> Result<LegacySourceReceipt, CredentialError> {
        if home_receipt.observed_digest != self.home_target_digest() { return Err(mismatch()); }
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
        Ok(LegacySourceReceipt { target_digest: Some(self.observed_digest(&after)),
            baseline_core_digest: self.baseline_core_digest(),
            observed_raw_digest: dacl_digest(&source_acl(binding)?) })
    }

    /// A later holder may inherit a completed fenced baseline together with
    /// exact current observer/model grants. F/H must supply the complete
    /// currently live SID set from original custody; this never writes a DACL.
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
        let mut exact = self.baseline.clone();
        for (sid, rights) in permitted_current {
            if !is_package_sid(sid) || !matches!(*rights, FILE_GENERIC_READ | CREDENTIAL_FILE_RIGHTS)
                || exact.aces.iter().any(|ace| &ace.sid == sid) {
                return Err(mismatch());
            }
            exact.aces.push(Ace { kind: ACCESS_ALLOWED_ACE_TYPE, flags: 0,
                mask: *rights, sid: sid.clone() });
        }
        let observed = binding.with_source_acl(|handle, _prepared| {
            if &file_identity(handle)? != &self.source_identity { return Err(mismatch()); }
            let actual = read_dacl(handle)?;
            if !acl_equal(&actual, &exact) { return Err(mismatch()); }
            Ok(actual)
        })?;
        binding.verify_registered_aliases(registered_aliases)?;
        require_bound_path(home, home_identity, true)?;
        Ok(LegacySourceReceipt { target_digest: None,
            baseline_core_digest: self.baseline_core_digest(),
            observed_raw_digest: dacl_digest(&observed) })
    }

    fn target_bytes(&self, source_baseline: bool) -> Vec<u8> {
        let mut bytes = self.bytes();
        for object in &self.objects {
            let acl = if source_baseline && object.source {
                self.baseline.clone()
            } else { home_target(&object.original, &self.historical) };
            encode_dacl_canonical(&mut bytes, &acl);
        }
        bytes
    }

    fn observed_digest(&self, observed: &[Dacl]) -> String {
        let mut bytes = self.bytes();
        for acl in observed { encode_dacl_canonical(&mut bytes, acl); }
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
    ordered.aces.sort_by(|left, right| (&left.sid, left.kind, left.mask, left.flags)
        .cmp(&(&right.sid, right.kind, right.mask, right.flags)));
    encode_dacl(bytes, &ordered);
}
fn dacl_digest(acl: &Dacl) -> String {
    let mut bytes = Vec::new(); encode_dacl(&mut bytes, acl); sha256_hex(&bytes)
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
        assert_eq!(LegacyAclInventory::from_bytes(&unhex(&payload).unwrap())
            .unwrap().bytes(), inventory.bytes());
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
    }
}
