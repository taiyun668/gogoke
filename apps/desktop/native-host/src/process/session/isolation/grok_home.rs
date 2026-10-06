//! Grok's fixed F instance HOME owns its original `auth.json`.  The generic
//! tree predicate and Codex/File alias custody deliberately remain untouched.
//! These functions never request FILE_READ_DATA or interpret credential bytes.
use super::*;
use std::fs::{File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;

#[link(name = "advapi32")]
extern "system" {
    fn InitializeAcl(acl: *mut c_void, length: u32, revision: u32) -> i32;
    fn AddAce(acl: *mut c_void, revision: u32, index: u32,
        bytes: *const c_void, length: u32) -> i32;
}

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
    if acl.is_null() || descriptor.is_null(){return Err(IsolationError::AclWitnessMismatch);}
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
    relative_name: PathBuf,
}

impl GrokAuthMetadata {
    fn handle(&self) -> Handle { self.file.as_raw_handle().cast() }
    pub(crate) fn relative_name(&self)->&Path {&self.relative_name}
    pub(crate) fn acl(&self,profile:&AppContainerProfile)->Result<GrokAclSnapshot,IsolationError>{
        self.verify_retired_physical()?;
        snapshot(self.handle(),profile)
    }
    /// Observation is metadata only. A newly rotated FileID may inherit HOME's
    /// ACL; only F's separately journaled successor transition may normalize it.
    pub(crate) fn candidate_acl(&self,profile:&AppContainerProfile)->Result<GrokAclSnapshot,IsolationError>{
        self.verify_candidate_physical()?;
        snapshot(self.handle(),profile)
    }
    fn verify_candidate_physical(&self)->Result<(),IsolationError>{
        let info=file_information(self.handle())?;
        if file_identity(self.handle())?!=self.identity || !info.physical_file() {
            return Err(IsolationError::AclWitnessMismatch);
        }
        Ok(())
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
    let auth=observe_grok_auth_candidate(home,home_identity)?;
    auth.verify_physical()?;
    Ok(auth)
}

pub(crate) fn observe_grok_auth_candidate(home:&Path,home_identity:&RootIdentity)
    ->Result<GrokAuthMetadata,IsolationError>{
    require_bound_path(home, home_identity, true)?;
    let path = home.join("auth.json");
    open_grok_metadata_candidate(home,home_identity,&path,PathBuf::from("auth.json"))
}

fn open_grok_metadata(home:&Path,home_identity:&RootIdentity,path:&Path,
    relative_name:PathBuf)->Result<GrokAuthMetadata,IsolationError>{
    let auth=open_grok_metadata_candidate(home,home_identity,path,relative_name)?;
    auth.verify_physical()?;
    Ok(auth)
}

fn open_grok_metadata_candidate(home:&Path,home_identity:&RootIdentity,path:&Path,
    relative_name:PathBuf)->Result<GrokAuthMetadata,IsolationError>{
    if relative_name.is_absolute() || relative_name.components().next().is_none() ||
        relative_name.components().any(|part|!matches!(part,std::path::Component::Normal(_))) ||
        home.join(&relative_name).as_path()!=path {
        return Err(IsolationError::AclWitnessMismatch);
    }
    let metadata = std::fs::symlink_metadata(&path).map_err(|error| IsolationError::AclObject {
        object: path.to_path_buf(), operation: "observe Grok auth metadata", error })?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(IsolationError::DirectoryNotPhysical);
    }
    let file = OpenOptions::new().access_mode(READ_CONTROL | WRITE_DAC)
        .share_mode(FILE_SHARE_ALL).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path).map_err(|error| IsolationError::AclObject {
            object: path.to_path_buf(), operation: "hold Grok auth metadata", error })?;
    let result = GrokAuthMetadata { identity: file_identity(file.as_raw_handle().cast())?,
        file,relative_name };
    result.verify_candidate_physical()?;
    require_bound_path(home, home_identity, true)?;
    Ok(result)
}

/// Cold recovery may reacquire only a previously journaled auth FileID that
/// remains a physical single-link object inside the same registered HOME.
/// None does not prove deletion; the caller must retain an unresolved intent.
pub(crate) fn observe_grok_recorded_auth(home:&Path,home_identity:&RootIdentity,
    original:&RootIdentity)->Result<Option<GrokAuthMetadata>,IsolationError>{
    require_bound_path(home,home_identity,true)?;
    let mut found=None;
    for (path,identity,directory) in collect_tree(home)? {
        if &identity!=original {continue;}
        if found.is_some() ||directory {return Err(IsolationError::AclWitnessMismatch);}
        let relative=path.strip_prefix(home).map_err(|error|IsolationError::Acl(
            io::Error::new(io::ErrorKind::InvalidData,error.to_string())))?.to_path_buf();
        let held=open_grok_metadata(home,home_identity,&path,relative)?;
        if held.identity!=*original {return Err(IsolationError::AclWitnessMismatch);}
        found=Some(held);
    }
    require_bound_path(home,home_identity,true)?;
    Ok(found)
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

/// Called only for an unrecorded successor FileID after F/H intent. Replace
/// this exact SID's inherited ACE with the already authorized explicit grant;
/// preserve every non-target ACE byte and do not request credential data.
pub(crate) fn grant_grok_auth_successor(profile:&AppContainerProfile,
    auth:&GrokAuthMetadata)->Result<(),IsolationError>{
    auth.verify_candidate_physical()?;
    rewrite_grok_target(auth.handle(),profile,&auth.identity,true,true)?;
    verify_grok_auth(profile,auth)
}

fn rewrite_grok_target(handle:Handle,profile:&AppContainerProfile,identity:&RootIdentity,
    grant:bool,protect:bool)->Result<(),IsolationError>{
    if file_identity(handle)?!=*identity {return Err(IsolationError::AclWitnessMismatch);}
    let entries=package_aces(handle,profile.sid)?;
    if !entries.is_empty() && entries.as_slice()!=&[(GRANT_ACCESS,directory_rights(true),NO_INHERITANCE)] &&
        entries.as_slice()!=&[(GRANT_ACCESS,directory_rights(true),INHERITED_ACE as u32)] {
        return Err(IsolationError::AclWitnessMismatch);
    }
    let before=snapshot(handle,profile)?;
    let mut old_acl=ptr::null_mut();let mut descriptor=ptr::null_mut();
    let status=unsafe{GetSecurityInfo(handle,FILE_OBJECT,DACL_SECURITY_INFORMATION,
        ptr::null_mut(),ptr::null_mut(),&mut old_acl,ptr::null_mut(),&mut descriptor)};
    if status!=0 {return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32)));}
    let _descriptor=LocalAllocation(descriptor);
    if old_acl.is_null(){return Err(IsolationError::AclWitnessMismatch);}
    let mut size=AclSizeInformation{ace_count:0,acl_bytes_in_use:0,acl_bytes_free:0};
    if unsafe{GetAclInformation(old_acl,(&mut size as *mut AclSizeInformation).cast(),
        size_of::<AclSizeInformation>() as u32,ACL_SIZE_INFORMATION_CLASS)}==0 {
        return Err(IsolationError::Acl(io::Error::last_os_error()));
    }
    let mut other=Vec::new();
    for index in 0..size.ace_count {
        let mut ace=ptr::null_mut();
        if unsafe{GetAce(old_acl,index,&mut ace)}==0 ||ace.is_null(){return Err(IsolationError::AclWitnessMismatch);}
        let header=unsafe{&*ace.cast::<AceHeader>()};
        if header.ace_size<16 {return Err(IsolationError::AclWitnessMismatch);}
        if unsafe{EqualSid(ace.cast::<u8>().add(8).cast(),profile.sid)}==0 {
            other.push(unsafe{std::slice::from_raw_parts(ace.cast::<u8>(),header.ace_size as usize)}.to_vec());
        }
    }
    let length=8usize+other.iter().map(Vec::len).sum::<usize>();
    if length>u16::MAX as usize{return Err(IsolationError::AclWitnessMismatch);}
    let mut storage=vec![0usize;length.div_ceil(size_of::<usize>())];let base=storage.as_mut_ptr().cast();
    if unsafe{InitializeAcl(base,length as u32,4)}==0{return Err(IsolationError::Acl(io::Error::last_os_error()));}
    for ace in &other {if unsafe{AddAce(base,4,u32::MAX,ace.as_ptr().cast(),ace.len() as u32)}==0 {
        return Err(IsolationError::Acl(io::Error::last_os_error()));
    }}
    let mut allocated=ptr::null_mut();
    let acl=if grant {
        let mut entry=ExplicitAccessW{permissions:directory_rights(true),access_mode:GRANT_ACCESS,
            inheritance:NO_INHERITANCE,trustee:TrusteeW{multiple:ptr::null_mut(),multiple_operation:0,
                form:TRUSTEE_IS_SID,kind:TRUSTEE_IS_UNKNOWN,name:profile.sid.cast()}};
        let status=unsafe{SetEntriesInAclW(1,&mut entry,base,&mut allocated)};
        if status!=0{return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32)));}
        if allocated.is_null(){return Err(IsolationError::AclWitnessMismatch);}allocated
    }else{base};
    let _allocated=LocalAllocation(allocated);
    let security=DACL_SECURITY_INFORMATION|if protect{PROTECTED_DACL_SECURITY_INFORMATION}else{0};
    let status=unsafe{SetSecurityInfo(handle,FILE_OBJECT,security,ptr::null_mut(),ptr::null_mut(),acl,ptr::null_mut())};
    if status!=0{return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32)));}
    let after=snapshot(handle,profile)?;
    let expected=if grant{format!("1:{}:0",directory_rights(true))}else{String::new()};
    if after.identity!=*identity ||after.target_aces!=expected ||after.dacl_protected!=protect ||
        !before.preserves_other_aces(&after){return Err(IsolationError::AclWitnessMismatch);}
    Ok(())
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
            // A vendor-created child can retain this inherited ACE after the
            // parent revoke. Return its exact physical identity for a separate
            // journaled removal; parent absence alone is not revocation proof.
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
    if dacl_protected(held.0)? {
        revoke_exact(held.0,profile.sid,&object.identity,true,NO_INHERITANCE)?;
    }else{
        rewrite_grok_target(held.0,profile,&object.identity,false,false)?;
    }
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

    fn protect_file(path:&Path){
        let held=open_physical_object(path,false,READ_CONTROL|WRITE_DAC).unwrap();
        let mut acl=ptr::null_mut();
        let mut descriptor=ptr::null_mut();
        assert_eq!(unsafe{GetSecurityInfo(held.0,FILE_OBJECT,DACL_SECURITY_INFORMATION,
            ptr::null_mut(),ptr::null_mut(),&mut acl,ptr::null_mut(),&mut descriptor)},0);
        let _descriptor=LocalAllocation(descriptor);
        assert_eq!(unsafe{SetSecurityInfo(held.0,FILE_OBJECT,
            DACL_SECURITY_INFORMATION|PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),ptr::null_mut(),acl,ptr::null_mut())},0);
    }

    fn fixture() -> (PathBuf,RootIdentity) {
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let home=std::env::temp_dir().join(format!("grok-home-acl-{}-{nonce}",std::process::id()));
        std::fs::create_dir(&home).unwrap();
        std::fs::write(home.join("auth.json"),b"fixture-only").unwrap();
        let root=crate::root::inspect_root(&home).unwrap().identity;
        protect_file(&home.join("auth.json"));
        (home,root)
    }

    #[test]
    fn inherited_auth_successor_is_normalized_without_changing_peer_aces(){
        let (home,root)=fixture();
        let profile=AppContainerProfile::derived_for_test("Gogoke37.GrokInheritedSuccessor").unwrap();
        let peer=AppContainerProfile::derived_for_test("Gogoke37.GrokInheritedPeer").unwrap();
        let old=observe_grok_auth(&home,&root).unwrap();
        grant_grok_home_root(&profile,&home,&root).unwrap();
        grant_grok_home_root(&peer,&home,&root).unwrap();
        grant_grok_auth(&profile,&old).unwrap();
        let next=home.join("auth-next.json");
        // Use the vendor's real shape: a new file inside HOME inherits its
        // ACL. Do not pre-protect this replacement as the older fixture did.
        std::fs::write(&next,b"synthetic non-secret auth fixture").unwrap();
        std::fs::remove_file(home.join("auth.json")).unwrap();
        std::fs::rename(next,home.join("auth.json")).unwrap();
        assert!(matches!(observe_grok_auth(&home,&root),Err(IsolationError::AclWitnessMismatch)));
        let auth=observe_grok_auth_candidate(&home,&root).unwrap();
        assert_ne!(auth.identity,old.identity);
        let before=auth.candidate_acl(&profile).unwrap();
        assert!(!before.dacl_protected);
        assert_eq!(before.target_aces,format!("1:{}:{}",directory_rights(true),INHERITED_ACE));
        let peer_before=auth.candidate_acl(&peer).unwrap().target_aces;
        grant_grok_auth_successor(&profile,&auth).unwrap();
        let after=auth.acl(&profile).unwrap();
        assert!(after.dacl_protected);
        assert_eq!(after.dacl_control,before.dacl_control|SE_DACL_PROTECTED);
        assert!(before.preserves_other_aces(&after));
        assert_eq!(auth.acl(&peer).unwrap().target_aces,peer_before);
        verify_grok_auth(&profile,&auth).unwrap();
        // A peer's inherited target is converted only by its own transition.
        grant_grok_auth_successor(&peer,&auth).unwrap();
        verify_grok_auth(&profile,&auth).unwrap();verify_grok_auth(&peer,&auth).unwrap();
        let hardlink=home.join("auth-linked.json");std::fs::hard_link(home.join("auth.json"),&hardlink).unwrap();
        assert!(observe_grok_auth_candidate(&home,&root).is_err());
        std::fs::remove_file(hardlink).unwrap();
        revoke_grok_home_root(&profile,&home,&root).unwrap();
        revoke_grok_auth(&profile,&auth).unwrap();revoke_grok_auth(&profile,&old).unwrap();
        verify_grok_auth(&peer,&auth).unwrap();
        assert!(inspect_grok_home_residue(&profile,&home,&root,&[old.identity.clone(),auth.identity.clone()]).unwrap().is_empty());
        drop(auth);drop(old);drop(profile);drop(peer);std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn protected_original_auth_grant_replacement_and_peer_sid_survive_revoke(){
        let (home,root)=fixture();
        let profile=AppContainerProfile::derived_for_test("Gogoke37.GrokOriginalAcl").unwrap();
        let peer=AppContainerProfile::derived_for_test("Gogoke37.GrokPeerAcl").unwrap();
        let old=observe_grok_auth(&home,&root).unwrap();
        let replacement=home.with_extension("auth-next");
        std::fs::write(&replacement,b"fixture-only").unwrap();
        protect_file(&replacement);
        assert!(old.acl(&profile).unwrap().target_aces.is_empty());
        grant_grok_home_root(&profile,&home,&root).unwrap();
        grant_grok_auth(&profile,&old).unwrap();
        grant_grok_auth(&peer,&old).unwrap();
        let acl_before_verify=old.acl(&profile).unwrap();
        verify_grok_home_tree(&profile,&home,&root,&old,&[old.identity.clone()]).unwrap();
        assert_eq!(old.acl(&profile).unwrap(),acl_before_verify);
        std::fs::write(home.join("auth.json"),b"in-place-fixture").unwrap();
        assert_eq!(observe_grok_auth(&home,&root).unwrap().identity,old.identity);
        std::fs::remove_file(home.join("auth.json")).unwrap();
        std::fs::rename(&replacement,home.join("auth.json")).unwrap();
        // The metadata handle shared DELETE. A protected new FileID is not
        // granted by the old witness; the host must explicitly grant it.
        let successor=observe_grok_auth(&home,&root).unwrap();
        assert_ne!(successor.identity,old.identity);
        assert!(successor.acl(&profile).unwrap().target_aces.is_empty());
        grant_grok_auth(&profile,&successor).unwrap();
        verify_grok_home_tree(&profile,&home,&root,&successor,
            &[old.identity.clone(),successor.identity.clone()]).unwrap();
        revoke_grok_home_root(&profile,&home,&root).unwrap();
        revoke_grok_auth(&profile,&old).unwrap();
        revoke_grok_auth(&profile,&successor).unwrap();
        assert!(old.acl(&profile).unwrap().target_aces.is_empty());
        assert!(!old.acl(&peer).unwrap().target_aces.is_empty());
        drop(successor);
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

    #[test]
    fn cold_old_file_id_reacquires_only_inside_original_home(){
        let (home,root)=fixture();
        let profile=AppContainerProfile::derived_for_test("Gogoke37.GrokColdOldAcl").unwrap();
        let original=observe_grok_auth(&home,&root).unwrap();
        grant_grok_home_root(&profile,&home,&root).unwrap();
        grant_grok_auth(&profile,&original).unwrap();
        let id=original.identity.clone();
        drop(original); // Simulates loss of the old host's metadata handle.
        std::fs::rename(home.join("auth.json"),home.join("auth-backup")).unwrap();
        let found=observe_grok_recorded_auth(&home,&root,&id).unwrap().unwrap();
        assert_eq!(found.identity,id);
        assert_eq!(found.relative_name(),Path::new("auth-backup"));
        revoke_grok_home_root(&profile,&home,&root).unwrap();
        revoke_grok_auth(&profile,&found).unwrap();
        assert!(found.acl(&profile).unwrap().target_aces.is_empty());
        drop(found);
        std::fs::remove_file(home.join("auth-backup")).unwrap();
        std::fs::remove_dir(home).unwrap();
    }

    #[test]
    fn wrong_home_and_reparse_refuse_without_target_sid_write(){
        let (home,root)=fixture();
        let profile=AppContainerProfile::derived_for_test("Gogoke37.GrokWrongHomeAcl").unwrap();
        let sibling=home.with_extension("sibling");
        std::fs::create_dir(&sibling).unwrap();
        let wrong=crate::root::inspect_root(&sibling).unwrap().identity;
        let before=grok_root_acl(&profile,&home,&root).unwrap();
        assert!(observe_grok_auth(&home,&wrong).is_err());
        assert!(grant_grok_home_root(&profile,&home,&wrong).is_err());
        assert_eq!(grok_root_acl(&profile,&home,&root).unwrap(),before);
        let outside=sibling.join("target");
        std::fs::write(&outside,b"fixture-only").unwrap();
        std::fs::remove_file(home.join("auth.json")).unwrap();
        std::os::windows::fs::symlink_file(&outside,home.join("auth.json"))
            .expect("Windows cloud runner must create the reparse fixture");
        assert!(matches!(observe_grok_auth(&home,&root),Err(IsolationError::DirectoryNotPhysical)));
        assert_eq!(grok_root_acl(&profile,&home,&root).unwrap(),before);
        std::fs::remove_file(home.join("auth.json")).unwrap();
        std::fs::remove_file(outside).unwrap();
        std::fs::remove_dir(sibling).unwrap();
        std::fs::remove_dir(home).unwrap();
    }
}
