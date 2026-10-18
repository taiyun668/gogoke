//! Read-only observation of the original OpenCode DOS alias namespace. The
//! token and QueryDosDeviceW ABI mirror store/session_transport/dos_alias.rs;
//! this module has no DefineDosDevice or removal entry point.

use crate::root::inspect_root;
use std::ffi::{c_void,OsStr,OsString};
use std::io;
use std::os::windows::ffi::{OsStrExt,OsStringExt};
use std::os::windows::fs::MetadataExt;
use std::path::Path;

const TOKEN_QUERY:u32=0x0008;
const TOKEN_STATISTICS:u32=10;
const TOKEN_SESSION_ID:u32=12;
const ERROR_NO_TOKEN:i32=1008;
type Handle=*mut c_void;

#[repr(C)]
#[derive(Clone,Copy,Default)]
struct Luid {low_part:u32,high_part:i32}
#[repr(C)]
#[derive(Clone,Copy,Default)]
struct TokenStatistics {
    token_id:Luid,authentication_id:Luid,expiration_time:i64,
    token_type:u32,impersonation_level:u32,dynamic_charged:u32,
    dynamic_available:u32,group_count:u32,privilege_count:u32,modified_id:Luid,
}
#[link(name="kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess()->Handle;
    fn GetCurrentThread()->Handle;
    fn QueryDosDeviceW(name:*const u16,target:*mut u16,max:u32)->u32;
    fn CloseHandle(handle:Handle)->i32;
}
#[link(name="advapi32")]
unsafe extern "system" {
    fn OpenProcessToken(process:Handle,access:u32,token:*mut Handle)->i32;
    fn OpenThreadToken(thread:Handle,access:u32,open_as_self:i32,token:*mut Handle)->i32;
    fn GetTokenInformation(token:Handle,class:u32,information:*mut c_void,
        information_len:u32,returned_len:*mut u32)->i32;
}
struct OwnedHandle(Handle);
impl Drop for OwnedHandle {
    fn drop(&mut self) {unsafe {CloseHandle(self.0);}}
}
fn current_namespace()->Result<String,String>{
    let mut raw=std::ptr::null_mut();
    if unsafe {OpenThreadToken(GetCurrentThread(),TOKEN_QUERY,1,&mut raw)}==0 {
        let error=io::Error::last_os_error();
        if error.raw_os_error()!=Some(ERROR_NO_TOKEN){
            return Err(format!("version alias OpenThreadToken: {error:?}"));
        }
        if unsafe {OpenProcessToken(GetCurrentProcess(),TOKEN_QUERY,&mut raw)}==0 {
            return Err(format!("version alias OpenProcessToken: {:?}",io::Error::last_os_error()));
        }
    }
    if raw.is_null(){return Err("version alias token handle is null".into());}
    let token=OwnedHandle(raw);
    let mut statistics=TokenStatistics::default();
    let mut returned=0u32;
    if unsafe {GetTokenInformation(token.0,TOKEN_STATISTICS,
        (&mut statistics as *mut TokenStatistics).cast(),
        std::mem::size_of::<TokenStatistics>() as u32,&mut returned)}==0 ||
        returned!=std::mem::size_of::<TokenStatistics>() as u32 {
        return Err(format!("version alias token statistics: {:?}",io::Error::last_os_error()));
    }
    let mut session=0u32;returned=0;
    if unsafe {GetTokenInformation(token.0,TOKEN_SESSION_ID,
        (&mut session as *mut u32).cast(),std::mem::size_of::<u32>() as u32,&mut returned)}==0 ||
        returned!=std::mem::size_of::<u32>() as u32 {
        return Err(format!("version alias token session: {:?}",io::Error::last_os_error()));
    }
    if statistics.authentication_id.low_part==0 &&
        statistics.authentication_id.high_part==0 {
        return Err("version alias zero authentication LUID".into());
    }
    Ok(format!("auth={:08x}{:08x};session={session}",
        statistics.authentication_id.high_part as u32,
        statistics.authentication_id.low_part))
}
fn mapping(name:&str)->Result<String,String>{
    if name.len()!=2 || name.as_bytes()[0].is_ascii_uppercase()==false ||
        name.as_bytes()[1]!=b':' {return Err("version alias invalid name".into());}
    let wide=OsStr::new(name).encode_wide().chain(std::iter::once(0)).collect::<Vec<_>>();
    let mut buffer=vec![0u16;32768];
    let count=unsafe {QueryDosDeviceW(wide.as_ptr(),buffer.as_mut_ptr(),buffer.len() as u32)};
    if count==0 {return Err(format!("version alias QueryDosDeviceW({name}): {:?}",
        io::Error::last_os_error()));}
    if count as usize>buffer.len() || buffer[count as usize-1]!=0 {
        return Err("version alias invalid QueryDosDeviceW length".into());
    }
    let mut entries=Vec::new();let mut begin=0usize;
    for end in 0..count as usize {
        if buffer[end]==0 {
            if end==begin {break;}
            entries.push(OsString::from_wide(&buffer[begin..end]).into_string()
                .map_err(|_|"version alias non-Unicode target")?);
            begin=end+1;
        }
    }
    if entries.len()!=1 {return Err("version alias mapping absent or ambiguous".into());}
    Ok(entries.remove(0))
}

/// A sealed native witness. Every validate repeats effective-token namespace,
/// exact mapping and requested physical FileID observations.
pub(crate) struct NativeVersionAliasNamespace {
    namespace:String,name:String,requested_target:String,observed_target:String,
    volume_serial:String,file_id:String,
}
impl NativeVersionAliasNamespace {
    pub(crate) fn observe(namespace:&str,name:&str,requested_target:&str,
        observed_target:&str,volume_serial:&str,file_id:&str)->Result<Self,String>{
        let value=Self{namespace:namespace.into(),name:name.into(),
            requested_target:requested_target.into(),observed_target:observed_target.into(),
            volume_serial:volume_serial.into(),file_id:file_id.into()};
        value.validate(namespace,name,requested_target,observed_target,volume_serial,file_id)?;
        Ok(value)
    }
    pub(crate) fn validate(&self,namespace:&str,name:&str,requested_target:&str,
        observed_target:&str,volume_serial:&str,file_id:&str)->Result<(),String>{
        if self.namespace!=namespace || self.name!=name ||
            self.requested_target!=requested_target || self.observed_target!=observed_target ||
            self.volume_serial!=volume_serial || self.file_id!=file_id ||
            namespace.is_empty() || requested_target.is_empty() || observed_target.is_empty() ||
            current_namespace()?!=namespace || mapping(name)?!=observed_target {
            return Err("version alias original namespace or target changed".into());
        }
        let path=Path::new(requested_target);
        if !path.is_absolute() || requested_target.contains('\0') {
            return Err("version alias invalid requested target".into());
        }
        let metadata=std::fs::symlink_metadata(path)
            .map_err(|error|format!("version alias target metadata: {error:?}"))?;
        if !metadata.is_dir() || metadata.file_attributes()&0x400!=0 {
            return Err("version alias target is not a plain directory".into());
        }
        let identity=inspect_root(path)
            .map_err(|error|format!("version alias physical target: {error:?}"))?.identity;
        let alias_root=std::fs::canonicalize(format!(r"{}\",name))
            .map_err(|error|format!("version alias canonical target: {error:?}"))?;
        let alias_identity=inspect_root(&alias_root)
            .map_err(|error|format!("version alias mapped FileID: {error:?}"))?.identity;
        if alias_identity!=identity {
            return Err("version alias mapping no longer reaches original F".into());
        }
        let observed_volume=format!("{:016x}",identity.volume_serial);
        let observed_file=identity.file_id.iter().map(|byte|format!("{byte:02x}"))
            .collect::<String>();
        if observed_volume!=volume_serial || observed_file!=file_id {
            return Err("version alias physical FileID changed".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_logon_namespace_never_authorizes_an_alias() {
        let actual=current_namespace().expect("native effective token");
        let stale=format!("{actual}-different");
        let error=NativeVersionAliasNamespace::observe(&stale,"Z:",
            r"C:\missing-original-F","recorded-mapping","0000000000000001",
            "00000000000000000000000000000001").err()
            .expect("different original namespace is refused");
        assert!(error.contains("namespace or target changed"));
    }
}
