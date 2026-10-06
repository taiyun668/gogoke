//! Grok's fixed F instance HOME owns its original `auth.json`.  The generic
//! tree predicate and Codex/File alias custody deliberately remain untouched.
//! These functions never request FILE_READ_DATA or interpret credential bytes.
use super::*;
use std::fs::{File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GrokHomeObject {
    pub(crate) relative_name: PathBuf,
    pub(crate) identity: RootIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GrokAclSnapshot {
    pub(crate) identity: RootIdentity,
    pub(crate) target_aces: String,
    pub(crate) dacl_protected: bool,
    pub(crate) dacl_control: u16,
    other_aces: Vec<Vec<u8>>,
}

fn snapshot(handle: Handle, profile: &AppContainerProfile) -> Result<GrokAclSnapshot, IsolationError> {
    let identity=file_identity(handle)?;
    let target=package_aces(handle,profile.sid)?;
    let mut acl=ptr::null_mut();
    let mut descriptor=ptr::null_mut();
    let status=unsafe {GetSecurityInfo(handle,FILE_OBJECT,DACL_SECURITY_INFORMATION,
        ptr::null_mut(),ptr::null_mut(),&mut acl,ptr::null_mut(),&mut descriptor)};
    if status!=0 {return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32)));}
    let _descriptor=LocalAllocation(descriptor);
    if acl.is_null(){return Err(IsolationError::AclWitnessMismatch);}
    let mut size=AclSizeInformation{ace_count:0,acl_bytes_in_use:0,acl_bytes_free:0};
    if unsafe {GetAclInformation(acl,(&mut size as *mut AclSizeInformation).cast(),
        size_of::<AclSizeInformation>() as u32,ACL_SIZE_INFORMATION_CLASS)}==0 {
        return Err(IsolationError::Acl(io::Error::last_os_error()));
    }
    let mut other_aces=Vec::new();
    for index in 0..size.ace_count {
        let mut ace=ptr::null_mut();
        if unsafe {GetAce(acl,index,&mut ace)}==0 || ace.is_null(){
            return Err(IsolationError::AclWitnessMismatch);
        }
        let header=unsafe{&*ace.cast::<AceHeader>()};
        if header.ace_size<16 {return Err(IsolationError::AclWitnessMismatch);}
        let sid=unsafe{ace.cast::<u8>().add(8).cast()};
        if unsafe{EqualSid(sid,profile.sid)}==0 {
            other_aces.push(unsafe{std::slice::from_raw_parts(ace.cast::<u8>(),
                header.ace_size as usize)}.to_vec());
        }
    }
    other_aces.sort();
    let mut control=0u16;
    let mut revision=0u32;
    if unsafe{GetSecurityDescriptorControl(descriptor,&mut control,&mut revision)}==0 {
        return Err(IsolationError::Acl(io::Error::last_os_error()));
    }
    Ok(GrokAclSnapshot {identity,target_aces:target.iter().map(|(mode,mask,flags)|
        format!("{mode}:{mask}:{flags}")).collect::<Vec<_>>().join(","),
        dacl_protected:control & SE_DACL_PROTECTED !=0,dacl_control:control,other_aces})
}

impl GrokAclSnapshot {
    pub(crate) fn preserves_other_aces(&self,after:&Self)->bool {
        self.other_aces==after.other_aces
    }
    pub(crate) fn other_aces_bytes(&self)->Vec<u8>{
        let mut bytes=Vec::new();
        for ace in &self.other_aces {
            bytes.extend_from_slice(&(ace.len() as u32).to_be_bytes());
            bytes.extend_from_slice(ace);
        }
        bytes
    }
}

pub(crate) fn grok_root_acl(profile:&AppContainerProfile,home:&Path,
    expected:&RootIdentity)->Result<GrokAclSnapshot,IsolationError>{
    let held=open_physical_object(home,true,READ_CONTROL)?;
    let result=snapshot(held.0,profile)?;
    if &result.identity!=expected {return Err(IsolationError::AclWitnessMismatch);}
    Ok(result)
}

/// Metadata custody shares DELETE so the fixed CLI can atomically replace its
/// own file.  A removed old object stays addressable for exact-SID revocation.
pub(crate) struct GrokAuthMetadata {
    file: File,
    pub(crate) identity: RootIdentity,
}

impl GrokAuthMetadata {
    fn handle(&self) -> Handle { self.file.as_raw_handle().cast() }
    pub(crate) fn acl(&self,profile:&AppContainerProfile)->Result<GrokAclSnapshot,IsolationError>{
        self.verify_retired_physical()?;
        snapshot(self.handle(),profile)
    }
    fn verify_retired_physical(&self)->Result<(),IsolationError>{
        let info=file_information(self.handle())?;
        if file_identity(self.handle())?!=self.identity ||
            info.attributes & (FILE_ATTRIBUTE_DIRECTORY|FILE_ATTRIBUTE_REPARSE_POINT)!=0 ||
            info.links>1 || !dacl_protected(self.handle())? {
            return Err(IsolationError::AclWitnessMismatch);
        }
        Ok(())
    }
    fn verify_physical(&self) -> Result<(), IsolationError> {
        self.verify_retired_physical()?;
        if file_information(self.handle())?.links!=1 {
            return Err(IsolationError::AclWitnessMismatch);
        }
        Ok(())
    }
}

pub(crate) fn observe_grok_auth(home: &Path, home_identity: &RootIdentity)
    -> Result<GrokAuthMetadata, IsolationError> {
    require_bound_path(home, home_identity, true)?;
    let path = home.join("auth.json");
    let metadata = std::fs::symlink_metadata(&path).map_err(|error| IsolationError::AclObject {
        object: path.clone(), operation: "observe Grok auth metadata", error })?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(IsolationError::DirectoryNotPhysical);
    }
    let file = OpenOptions::new().access_mode(READ_CONTROL | WRITE_DAC)
        .share_mode(FILE_SHARE_ALL).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path).map_err(|error| IsolationError::AclObject {
            object: path.clone(), operation: "hold Grok auth metadata", error })?;
    let result = GrokAuthMetadata { identity: file_identity(file.as_raw_handle().cast())?, file };
    result.verify_physical()?;
    require_bound_path(home, home_identity, true)?;
    Ok(result)
}

pub(crate) fn grant_grok_home_root(profile: &AppContainerProfile, home: &Path,
    expected: &RootIdentity) -> Result<(), IsolationError> {
    let root = open_bound_object(home, expected, true)?;
    grant_exact_acl(root.0, profile.sid, expected, directory_rights(true),
        OBJECT_AND_CONTAINER_INHERIT)?;
    profile.verify_bound_directory_grant(home, expected, true)?;
    Ok(())
}

pub(crate) fn grant_grok_auth(profile: &AppContainerProfile,
    auth: &GrokAuthMetadata) -> Result<(), IsolationError> {
    auth.verify_physical()?;
    grant_exact_acl(auth.handle(), profile.sid, &auth.identity,
        directory_rights(true), NO_INHERITANCE)?;
    verify_grok_auth(profile, auth)
}

pub(crate) fn verify_grok_auth(profile: &AppContainerProfile,
    auth: &GrokAuthMetadata) -> Result<(), IsolationError> {
    auth.verify_physical()?;
    let observed = package_aces(auth.handle(), profile.sid)?;
    if observed.as_slice() != &[(GRANT_ACCESS, directory_rights(true), NO_INHERITANCE)] {
        return Err(IsolationError::AclWitnessDetail {
            object: PathBuf::from("auth.json"), sid: profile.package_sid_string()?,
            expected: format!("protected explicit single-file grant, rights={:#x}",
                directory_rights(true)), observed });
    }
    Ok(())
}

/// At preactivation every descendant must still satisfy the ordinary tree
/// predicate, except the exact current protected auth FileID.  At active use
/// the caller may pass old, durably recorded auth FileIDs that remain in HOME.
pub(crate) fn verify_grok_home_tree(profile: &AppContainerProfile, home: &Path,
    expected: &RootIdentity, auth: &GrokAuthMetadata,
    recorded_auth: &[RootIdentity]) -> Result<(), IsolationError> {
    profile.verify_bound_directory_grant(home, expected, true)?;
    let mut found = false;
    for (path, identity, directory) in collect_tree(home)? {
        let object = open_physical_object(&path, directory, READ_CONTROL)?;
        if file_identity(object.0)? != identity { return Err(IsolationError::AclWitnessMismatch); }
        let entries = package_aces(object.0, profile.sid)?;
        if path == home.join("auth.json") {
            if identity != auth.identity || directory || !dacl_protected(object.0)? {
                return Err(IsolationError::AclWitnessMismatch);
            }
            auth.verify_physical()?;
            if entries.as_slice() != &[(GRANT_ACCESS, directory_rights(true), NO_INHERITANCE)] {
                return Err(IsolationError::AclWitnessMismatch);
            }
            found = true;
        } else if recorded_auth.contains(&identity) && !directory && dacl_protected(object.0)? {
            if entries.as_slice() != &[(GRANT_ACCESS, directory_rights(true), NO_INHERITANCE)] {
                return Err(IsolationError::AclWitnessMismatch);
            }
        } else if entries.len() != 1 || entries[0].0 != GRANT_ACCESS ||
            entries[0].1 != directory_rights(true) ||
            entries[0].2 & INHERITED_ACE == 0 ||
            entries[0].2 & INHERIT_ONLY_ACE != 0 {
            return Err(IsolationError::AclWitnessDetail {
                object: path.strip_prefix(home).map_err(|error| IsolationError::Acl(
                    io::Error::new(io::ErrorKind::InvalidData, error.to_string())))?.to_path_buf(),
                sid: profile.package_sid_string()?,
                expected: format!("inherited HOME grant, rights={:#x}", directory_rights(true)),
                observed: entries });
        }
        require_bound_path(&path, &identity, directory)?;
    }
    if !found { return Err(IsolationError::AclWitnessMismatch); }
    require_bound_path(home, expected, true)
}

fn revoke_exact(handle: Handle, sid: *mut c_void, expected: &RootIdentity,
    protected: bool, inheritance: u32) -> Result<(), IsolationError> {
    if &file_identity(handle)? != expected || dacl_protected(handle)? != protected {
        return Err(IsolationError::AclWitnessMismatch);
    }
    let entries = package_aces(handle, sid)?;
    if entries.is_empty() { return Ok(()); }
    if entries.as_slice() != &[(GRANT_ACCESS, directory_rights(true), inheritance)] {
        return Err(IsolationError::AclWitnessMismatch);
    }
    let mut old_acl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    let status = unsafe { GetSecurityInfo(handle, FILE_OBJECT, DACL_SECURITY_INFORMATION,
        ptr::null_mut(), ptr::null_mut(), &mut old_acl, ptr::null_mut(), &mut descriptor) };
    if status != 0 { return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32))); }
    let _descriptor = LocalAllocation(descriptor);
    if old_acl.is_null() { return Err(IsolationError::AclWitnessMismatch); }
    let mut entry = ExplicitAccessW { permissions: 0, access_mode: REVOKE_ACCESS,
        inheritance: NO_INHERITANCE, trustee: TrusteeW { multiple: ptr::null_mut(),
            multiple_operation: 0, form: TRUSTEE_IS_SID, kind: TRUSTEE_IS_UNKNOWN,
            name: sid.cast() } };
    let mut new_acl = ptr::null_mut();
    let status = unsafe { SetEntriesInAclW(1, &mut entry, old_acl, &mut new_acl) };
    if status != 0 { return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32))); }
    if new_acl.is_null() { return Err(IsolationError::AclWitnessMismatch); }
    let acl = LocalAllocation(new_acl);
    let security = DACL_SECURITY_INFORMATION | if protected {
        PROTECTED_DACL_SECURITY_INFORMATION
    } else { 0 };
    let status = unsafe { SetSecurityInfo(handle, FILE_OBJECT, security,
        ptr::null_mut(), ptr::null_mut(), acl.0, ptr::null_mut()) };
    if status != 0 { return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32))); }
    if &file_identity(handle)? != expected || dacl_protected(handle)? != protected ||
        !package_aces(handle, sid)?.is_empty() {
        return Err(IsolationError::AclWitnessMismatch);
    }
    Ok(())
}

pub(crate) fn revoke_grok_home_root(profile: &AppContainerProfile, home: &Path,
    expected: &RootIdentity) -> Result<(), IsolationError> {
    let root = open_bound_object(home, expected, true)?;
    // HOME has the same inherited state before and after this session; only
    // the exact profile SID is removed. No peer ACE is reconstructed.
    revoke_exact(root.0, profile.sid, expected, dacl_protected(root.0)?,
        OBJECT_AND_CONTAINER_INHERIT)
}

pub(crate) fn revoke_grok_auth(profile: &AppContainerProfile,
    auth: &GrokAuthMetadata) -> Result<(), IsolationError> {
    auth.verify_retired_physical()?;
    revoke_exact(auth.handle(), profile.sid, &auth.identity, true, NO_INHERITANCE)
}

/// Called only after the caller's durable revoke intent. This readback catches
/// an old auth FileID renamed within HOME and residual inherited SID ACEs.
pub(crate) fn inspect_grok_home_residue(profile: &AppContainerProfile, home: &Path,
    expected: &RootIdentity, recorded_auth: &[RootIdentity])
    -> Result<Vec<GrokHomeObject>, IsolationError> {
    require_bound_path(home, expected, true)?;
    let mut touched = Vec::new();
    for (path, identity, directory) in collect_tree(home)? {
        let object = open_physical_object(&path, directory, READ_CONTROL)?;
        let entries = package_aces(object.0, profile.sid)?;
        if entries.is_empty() { continue; }
        let protected = dacl_protected(object.0)?;
        let permitted = if protected && !directory && recorded_auth.contains(&identity) {
            NO_INHERITANCE
        } else if !protected { INHERITED_ACE } else {
            return Err(IsolationError::AclWitnessMismatch);
        };
        if permitted == INHERITED_ACE {
            if entries.len() != 1 || entries[0].0 != GRANT_ACCESS ||
                entries[0].1 != directory_rights(true) ||
                entries[0].2 & INHERITED_ACE == 0 ||
                entries[0].2 & INHERIT_ONLY_ACE != 0 {
                return Err(IsolationError::AclWitnessMismatch);
            }
            // An inherited ACE is removed by the parent; if still visible,
            // this descendant needs a separate exact physical readback.
            return Err(IsolationError::AclWitnessMismatch);
        }
        touched.push(GrokHomeObject { relative_name: path.strip_prefix(home).map_err(|error|
            IsolationError::Acl(io::Error::new(io::ErrorKind::InvalidData,error.to_string())))?.to_path_buf(),
            identity });
    }
    require_bound_path(home, expected, true)?;
    Ok(touched)
}

pub(crate) fn revoke_grok_home_residue(profile: &AppContainerProfile, home: &Path,
    expected: &RootIdentity, object: &GrokHomeObject) -> Result<(), IsolationError> {
    require_bound_path(home, expected, true)?;
    if object.relative_name.is_absolute() || object.relative_name.components().any(|part|
        !matches!(part,std::path::Component::Normal(_))) {
        return Err(IsolationError::AclWitnessMismatch);
    }
    let path = home.join(&object.relative_name);
    let held = open_bound_object(&path, &object.identity, false)?;
    if !dacl_protected(held.0)? { return Err(IsolationError::AclWitnessMismatch); }
    revoke_exact(held.0, profile.sid, &object.identity, true, NO_INHERITANCE)?;
    require_bound_path(home, expected, true)
}

pub(crate) fn grok_residue_acl(profile:&AppContainerProfile,home:&Path,
    expected:&RootIdentity,object:&GrokHomeObject)->Result<GrokAclSnapshot,IsolationError>{
    require_bound_path(home,expected,true)?;
    if object.relative_name.is_absolute() || object.relative_name.components().any(|part|
        !matches!(part,std::path::Component::Normal(_))) {
        return Err(IsolationError::AclWitnessMismatch);
    }
    let held=open_bound_object(&home.join(&object.relative_name),&object.identity,false)?;
    snapshot(held.0,profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime,UNIX_EPOCH};

    fn fixture() -> (PathBuf,RootIdentity) {
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let home=std::env::temp_dir().join(format!("grok-home-acl-{}-{nonce}",std::process::id()));
        std::fs::create_dir(&home).unwrap();
        std::fs::write(home.join("auth.json"),b"fixture-only").unwrap();
        let root=crate::root::inspect_root(&home).unwrap().identity;
        let held=open_physical_object(&home.join("auth.json"),false,READ_CONTROL|WRITE_DAC).unwrap();
        let mut acl=ptr::null_mut();
        let mut descriptor=ptr::null_mut();
        assert_eq!(unsafe{GetSecurityInfo(held.0,FILE_OBJECT,DACL_SECURITY_INFORMATION,
            ptr::null_mut(),ptr::null_mut(),&mut acl,ptr::null_mut(),&mut descriptor)},0);
        let _descriptor=LocalAllocation(descriptor);
        assert_eq!(unsafe{SetSecurityInfo(held.0,FILE_OBJECT,
            DACL_SECURITY_INFORMATION|PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),ptr::null_mut(),acl,ptr::null_mut())},0);
        (home,root)
    }

    #[test]
    fn protected_original_auth_grant_replacement_and_peer_sid_survive_revoke(){
        let (home,root)=fixture();
        let profile=AppContainerProfile::derived_for_test("Gogoke37.GrokOriginalAcl").unwrap();
        let peer=AppContainerProfile::derived_for_test("Gogoke37.GrokPeerAcl").unwrap();
        let old=observe_grok_auth(&home,&root).unwrap();
        assert!(old.acl(&profile).unwrap().target_aces.is_empty());
        grant_grok_home_root(&profile,&home,&root).unwrap();
        grant_grok_auth(&profile,&old).unwrap();
        grant_grok_auth(&peer,&old).unwrap();
        verify_grok_home_tree(&profile,&home,&root,&old,&[old.identity.clone()]).unwrap();
        std::fs::write(home.join("auth-next"),b"fixture-only").unwrap();
        std::fs::remove_file(home.join("auth.json")).unwrap();
        std::fs::rename(home.join("auth-next"),home.join("auth.json")).unwrap();
        // The metadata handle shared DELETE; the unprotected successor has
        // no implicit exception or grant.
        assert!(observe_grok_auth(&home,&root).is_err());
        revoke_grok_home_root(&profile,&home,&root).unwrap();
        revoke_grok_auth(&profile,&old).unwrap();
        assert!(old.acl(&profile).unwrap().target_aces.is_empty());
        assert!(!old.acl(&peer).unwrap().target_aces.is_empty());
        drop(old);
        std::fs::remove_file(home.join("auth.json")).unwrap();
        std::fs::remove_dir(home).unwrap();
    }

    #[test]
    fn multiple_links_refuse_original_auth_binding(){
        let (home,root)=fixture();
        std::fs::hard_link(home.join("auth.json"),home.join("second-name")).unwrap();
        assert!(matches!(observe_grok_auth(&home,&root),Err(IsolationError::AclWitnessMismatch)));
        std::fs::remove_file(home.join("second-name")).unwrap();
        std::fs::remove_file(home.join("auth.json")).unwrap();
        std::fs::remove_dir(home).unwrap();
    }
}
