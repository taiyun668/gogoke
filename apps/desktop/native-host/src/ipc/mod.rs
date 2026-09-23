//! Current-user local IPC for the native host.
//!
//! This module exposes only a Windows named pipe. It does not create a TCP or
//! HTTP listener and cannot fall back to one. The pipe DACL contains one allow
//! ACE for the current user SID, rejects remote clients, and authenticates the
//! connected peer token before returning the connection.

use std::fmt;
use std::io;

#[derive(Debug)]
pub enum PrivateIpcError {
    InvalidEndpoint,
    InvalidPreface,
    FrameTooLarge {
        length: usize,
    },
    UnsupportedPlatform,
    Os {
        operation: &'static str,
        source: io::Error,
    },
    PeerIdentityMismatch {
        expected: String,
        observed: String,
    },
}

impl fmt::Display for PrivateIpcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEndpoint => write!(formatter, "PRIVATE_IPC_INVALID_ENDPOINT"),
            Self::InvalidPreface => write!(formatter, "PRIVATE_IPC_INVALID_PREFACE"),
            Self::FrameTooLarge { length } => {
                write!(formatter, "PRIVATE_IPC_FRAME_TOO_LARGE: {length}")
            }
            Self::UnsupportedPlatform => write!(formatter, "PRIVATE_IPC_UNSUPPORTED_PLATFORM"),
            Self::Os { operation, source } => {
                write!(formatter, "PRIVATE_IPC_OS_ERROR: {operation}: {source}")
            }
            Self::PeerIdentityMismatch { expected, observed } => write!(
                formatter,
                "PRIVATE_IPC_PEER_IDENTITY_MISMATCH: expected {expected}, observed {observed}"
            ),
        }
    }
}

pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
pub const PIPE_REJECT_REMOTE_CLIENTS_FLAG: u32 = 0x0000_0008;

pub const fn private_pipe_mode_flags() -> u32 {
    // PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT are all zero. Keeping
    // the remote-reject bit behind this production-used function makes its
    // security effect directly mutation-testable.
    PIPE_REJECT_REMOTE_CLIENTS_FLAG
}

pub fn peer_sid_is_admitted(expected: &str, observed: &str) -> bool {
    expected == observed
}

pub fn validate_frame_length(length: usize) -> Result<(), PrivateIpcError> {
    if length > MAX_FRAME_BYTES {
        return Err(PrivateIpcError::FrameTooLarge { length });
    }
    Ok(())
}

impl std::error::Error for PrivateIpcError {}

fn validate_endpoint(endpoint: &str) -> Result<(), PrivateIpcError> {
    if endpoint.is_empty()
        || endpoint.len() > 120
        || !endpoint
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(PrivateIpcError::InvalidEndpoint);
    }
    Ok(())
}

#[cfg(windows)]
mod platform {
    use super::{validate_endpoint, PrivateIpcError};
    use std::ffi::{c_void, OsString};
    use std::io;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::ptr::null_mut;

    type Bool = i32;
    type Dword = u32;
    type Handle = *mut c_void;
    type LocalMem = *mut c_void;
    type SecurityDescriptor = *mut c_void;
    type Sid = *mut c_void;

    const FALSE: Bool = 0;
    const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
    const ERROR_PIPE_CONNECTED: i32 = 535;
    const PIPE_ACCESS_DUPLEX: Dword = 0x0000_0003;
    const FILE_FLAG_FIRST_PIPE_INSTANCE: Dword = 0x0008_0000;
    const PIPE_TYPE_BYTE: Dword = 0x0000_0000;
    const PIPE_READMODE_BYTE: Dword = 0x0000_0000;
    const PIPE_WAIT: Dword = 0x0000_0000;
    const TOKEN_QUERY: Dword = 0x0008;
    const TOKEN_USER_CLASS: Dword = 1;
    const SECURITY_DESCRIPTOR_REVISION: Dword = 1;
    const DACL_SECURITY_INFORMATION: Dword = 0x0000_0004;
    const SE_KERNEL_OBJECT: Dword = 6;
    const ERROR_SUCCESS: Dword = 0;
    const SE_DACL_PROTECTED: u16 = 0x1000;
    const ACL_SIZE_INFORMATION_CLASS: Dword = 2;
    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
    const ACCESS_DENIED_ACE_TYPE: u8 = 1;
    const GENERIC_ALL: Dword = 0x1000_0000;

    #[repr(C)]
    struct SecurityAttributes {
        length: Dword,
        security_descriptor: SecurityDescriptor,
        inherit_handle: Bool,
    }

    #[repr(C)]
    struct SidAndAttributes {
        sid: Sid,
        attributes: Dword,
    }

    #[repr(C)]
    struct TokenUser {
        user: SidAndAttributes,
    }

    #[repr(C)]
    struct AclSizeInformation {
        ace_count: Dword,
        acl_bytes_in_use: Dword,
        acl_bytes_free: Dword,
    }

    #[repr(C)]
    struct AceHeader {
        ace_type: u8,
        ace_flags: u8,
        ace_size: u16,
    }

    #[repr(C)]
    struct AccessAllowedAce {
        header: AceHeader,
        mask: Dword,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> Handle;
        fn GetCurrentThread() -> Handle;
        fn CloseHandle(handle: Handle) -> Bool;
        fn LocalFree(memory: LocalMem) -> LocalMem;
        fn CreateNamedPipeW(
            name: *const u16,
            open_mode: Dword,
            pipe_mode: Dword,
            max_instances: Dword,
            out_buffer_size: Dword,
            in_buffer_size: Dword,
            default_timeout: Dword,
            security_attributes: *mut SecurityAttributes,
        ) -> Handle;
        fn ConnectNamedPipe(pipe: Handle, overlapped: *mut c_void) -> Bool;
        fn ReadFile(
            file: Handle,
            buffer: *mut c_void,
            bytes_to_read: Dword,
            bytes_read: *mut Dword,
            overlapped: *mut c_void,
        ) -> Bool;
        fn WriteFile(
            file: Handle,
            buffer: *const c_void,
            bytes_to_write: Dword,
            bytes_written: *mut Dword,
            overlapped: *mut c_void,
        ) -> Bool;
        fn DisconnectNamedPipe(pipe: Handle) -> Bool;
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn OpenProcessToken(process: Handle, access: Dword, token: *mut Handle) -> Bool;
        fn OpenThreadToken(
            thread: Handle,
            access: Dword,
            open_as_self: Bool,
            token: *mut Handle,
        ) -> Bool;
        fn GetTokenInformation(
            token: Handle,
            information_class: Dword,
            information: *mut c_void,
            information_length: Dword,
            return_length: *mut Dword,
        ) -> Bool;
        fn ConvertSidToStringSidW(sid: Sid, string_sid: *mut *mut u16) -> Bool;
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            string_descriptor: *const u16,
            revision: Dword,
            descriptor: *mut SecurityDescriptor,
            descriptor_size: *mut Dword,
        ) -> Bool;
        fn GetSecurityInfo(
            handle: Handle,
            object_type: Dword,
            security_information: Dword,
            owner: *mut Sid,
            group: *mut Sid,
            dacl: *mut *mut c_void,
            sacl: *mut *mut c_void,
            descriptor: *mut SecurityDescriptor,
        ) -> Dword;
        fn GetSecurityDescriptorDacl(
            descriptor: SecurityDescriptor,
            dacl_present: *mut Bool,
            dacl: *mut *mut c_void,
            dacl_defaulted: *mut Bool,
        ) -> Bool;
        fn GetSecurityDescriptorControl(
            descriptor: SecurityDescriptor,
            control: *mut u16,
            revision: *mut u32,
        ) -> Bool;
        fn GetAclInformation(
            acl: *mut c_void,
            information: *mut c_void,
            information_length: Dword,
            information_class: Dword,
        ) -> Bool;
        fn GetAce(
            acl: *mut c_void,
            ace_index: Dword,
            ace: *mut *mut c_void,
        ) -> Bool;
        fn ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor: SecurityDescriptor,
            revision: Dword,
            security_information: Dword,
            string_descriptor: *mut *mut u16,
            string_length: *mut Dword,
        ) -> Bool;
        fn ImpersonateNamedPipeClient(pipe: Handle) -> Bool;
        fn RevertToSelf() -> Bool;
    }

    struct OwnedHandle(Handle);

    impl OwnedHandle {
        fn raw(&self) -> Handle {
            self.0
        }
    }

    unsafe impl Send for OwnedHandle {}

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            if self.0 != INVALID_HANDLE_VALUE && !self.0.is_null() {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
    }

    struct OwnedLocal(LocalMem);

    impl Drop for OwnedLocal {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    LocalFree(self.0);
                }
            }
        }
    }

    fn os_error(operation: &'static str) -> PrivateIpcError {
        PrivateIpcError::Os {
            operation,
            source: io::Error::last_os_error(),
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        std::ffi::OsStr::new(value)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    unsafe fn wide_pointer_to_string(pointer: *const u16) -> String {
        let mut length = 0usize;
        while *pointer.add(length) != 0 {
            length += 1;
        }
        OsString::from_wide(std::slice::from_raw_parts(pointer, length))
            .to_string_lossy()
            .into_owned()
    }

    fn token_user_sid(token: Handle) -> Result<String, PrivateIpcError> {
        let mut required = 0;
        unsafe {
            GetTokenInformation(token, TOKEN_USER_CLASS, null_mut(), 0, &mut required);
        }
        if required == 0 {
            return Err(os_error("GetTokenInformation(size)"));
        }
        let mut buffer = vec![0u8; required as usize];
        if unsafe {
            GetTokenInformation(
                token,
                TOKEN_USER_CLASS,
                buffer.as_mut_ptr().cast(),
                required,
                &mut required,
            )
        } == FALSE
        {
            return Err(os_error("GetTokenInformation(TokenUser)"));
        }
        let user = unsafe { &*(buffer.as_ptr().cast::<TokenUser>()) };
        let mut string_sid = null_mut();
        if unsafe { ConvertSidToStringSidW(user.user.sid, &mut string_sid) } == FALSE {
            return Err(os_error("ConvertSidToStringSidW"));
        }
        let allocation = OwnedLocal(string_sid.cast());
        let sid = unsafe { wide_pointer_to_string(string_sid) };
        drop(allocation);
        Ok(sid)
    }

    pub fn current_user_sid() -> Result<String, PrivateIpcError> {
        let mut token = null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == FALSE {
            return Err(os_error("OpenProcessToken"));
        }
        let token = OwnedHandle(token);
        token_user_sid(token.raw())
    }

    struct ImpersonationGuard;

    impl Drop for ImpersonationGuard {
        fn drop(&mut self) {
            unsafe {
                RevertToSelf();
            }
        }
    }

    pub struct PrivatePipeListener {
        handle: OwnedHandle,
        path: String,
        expected_sid: String,
    }

    impl PrivatePipeListener {
        pub fn bind(endpoint: &str) -> Result<Self, PrivateIpcError> {
            validate_endpoint(endpoint)?;
            let expected_sid = current_user_sid()?;
            let path = format!(r"\\.\pipe\gogoke.current-user.v1.{endpoint}");
            let descriptor_text = format!("D:P(A;;GA;;;{expected_sid})");
            let mut descriptor = null_mut();
            if unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    wide(&descriptor_text).as_ptr(),
                    SECURITY_DESCRIPTOR_REVISION,
                    &mut descriptor,
                    null_mut(),
                )
            } == FALSE
            {
                return Err(os_error(
                    "ConvertStringSecurityDescriptorToSecurityDescriptorW",
                ));
            }
            let descriptor_allocation = OwnedLocal(descriptor);
            let mut attributes = SecurityAttributes {
                length: std::mem::size_of::<SecurityAttributes>() as Dword,
                security_descriptor: descriptor,
                inherit_handle: FALSE,
            };
            let handle = unsafe {
                CreateNamedPipeW(
                    wide(&path).as_ptr(),
                    PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                    PIPE_TYPE_BYTE
                        | PIPE_READMODE_BYTE
                        | PIPE_WAIT
                        | super::private_pipe_mode_flags(),
                    1,
                    64 * 1024,
                    64 * 1024,
                    0,
                    &mut attributes,
                )
            };
            drop(descriptor_allocation);
            if handle == INVALID_HANDLE_VALUE {
                return Err(os_error("CreateNamedPipeW"));
            }
            Ok(Self {
                handle: OwnedHandle(handle),
                path,
                expected_sid,
            })
        }

        pub fn path(&self) -> &str {
            &self.path
        }

        pub fn expected_sid(&self) -> &str {
            &self.expected_sid
        }

        pub fn applied_dacl_sddl(&self) -> Result<String, PrivateIpcError> {
            let mut descriptor = null_mut();
            let status = unsafe {
                GetSecurityInfo(
                    self.handle.raw(),
                    SE_KERNEL_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    &mut descriptor,
                )
            };
            if status != 0 {
                return Err(PrivateIpcError::Os {
                    operation: "GetSecurityInfo",
                    source: io::Error::from_raw_os_error(status as i32),
                });
            }
            let descriptor_allocation = OwnedLocal(descriptor);
            let mut text = null_mut();
            if unsafe {
                ConvertSecurityDescriptorToStringSecurityDescriptorW(
                    descriptor,
                    SECURITY_DESCRIPTOR_REVISION,
                    DACL_SECURITY_INFORMATION,
                    &mut text,
                    null_mut(),
                )
            } == FALSE
            {
                return Err(os_error(
                    "ConvertSecurityDescriptorToStringSecurityDescriptorW",
                ));
            }
            let text_allocation = OwnedLocal(text.cast());
            let result = unsafe { wide_pointer_to_string(text) };
            drop(text_allocation);
            drop(descriptor_allocation);
            Ok(result)
        }

        #[cfg(test)]
        fn applied_dacl_snapshot(&self) -> Result<(bool, Vec<(u8, Dword, String)>), PrivateIpcError> {
            let mut descriptor = null_mut();
            let status = unsafe {
                GetSecurityInfo(
                    self.handle.raw(),
                    SE_KERNEL_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    &mut descriptor,
                )
            };
            if status != ERROR_SUCCESS {
                return Err(PrivateIpcError::Os {
                    operation: "GetSecurityInfo",
                    source: io::Error::from_raw_os_error(status as i32),
                });
            }
            let descriptor_allocation = OwnedLocal(descriptor);
            let mut control = 0u16;
            let mut _revision = 0u32;
            if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut _revision) }
                == FALSE
            {
                return Err(os_error("GetSecurityDescriptorControl"));
            }
            let mut present = FALSE;
            let mut dacl = null_mut();
            let mut _defaulted = FALSE;
            if unsafe {
                GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut _defaulted)
            } == FALSE
            {
                return Err(os_error("GetSecurityDescriptorDacl"));
            }
            if present == FALSE || dacl.is_null() {
                return Ok(((control & SE_DACL_PROTECTED) != 0, Vec::new()));
            }
            let mut size = AclSizeInformation {
                ace_count: 0,
                acl_bytes_in_use: 0,
                acl_bytes_free: 0,
            };
            if unsafe {
                GetAclInformation(
                    dacl,
                    (&mut size as *mut AclSizeInformation).cast(),
                    std::mem::size_of::<AclSizeInformation>() as Dword,
                    ACL_SIZE_INFORMATION_CLASS,
                )
            } == FALSE
            {
                return Err(os_error("GetAclInformation"));
            }
            let mut entries = Vec::with_capacity(size.ace_count as usize);
            for index in 0..size.ace_count {
                let mut ace = null_mut();
                if unsafe { GetAce(dacl, index, &mut ace) } == FALSE || ace.is_null() {
                    return Err(os_error("GetAce"));
                }
                let header = unsafe { &*(ace.cast::<AceHeader>()) };
                let (mask, sid) = if header.ace_type == ACCESS_ALLOWED_ACE_TYPE
                    || header.ace_type == ACCESS_DENIED_ACE_TYPE
                {
                    let allowed = unsafe { &*(ace.cast::<AccessAllowedAce>()) };
                    let sid = unsafe {
                        (ace.cast::<u8>().add(8) as Sid)
                    };
                    let mut text = null_mut();
                    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == FALSE {
                        return Err(os_error("ConvertSidToStringSidW(ACE SID)"));
                    }
                    let allocation = OwnedLocal(text.cast());
                    let text = unsafe { wide_pointer_to_string(text) };
                    drop(allocation);
                    (allowed.mask, text)
                } else {
                    (0, String::new())
                };
                entries.push((header.ace_type, mask, sid));
            }
            drop(descriptor_allocation);
            Ok(((control & SE_DACL_PROTECTED) != 0, entries))
        }

        pub fn accept_current_user(self) -> Result<PrivatePipeConnection, PrivateIpcError> {
            if unsafe { ConnectNamedPipe(self.handle.raw(), null_mut()) } == FALSE
                && io::Error::last_os_error().raw_os_error() != Some(ERROR_PIPE_CONNECTED)
            {
                return Err(os_error("ConnectNamedPipe"));
            }
            // Windows requires at least one byte to be read from the client
            // before its named-pipe token can be impersonated. This byte is a
            // transport preface and is never forwarded to the host protocol.
            let mut preface = 0u8;
            let mut bytes_read = 0;
            if unsafe {
                ReadFile(
                    self.handle.raw(),
                    (&mut preface as *mut u8).cast(),
                    1,
                    &mut bytes_read,
                    null_mut(),
                )
            } == FALSE
            {
                return Err(os_error("ReadFile(private IPC preface)"));
            }
            if bytes_read != 1 || preface != 0x47 {
                return Err(PrivateIpcError::InvalidPreface);
            }
            if unsafe { ImpersonateNamedPipeClient(self.handle.raw()) } == FALSE {
                return Err(os_error("ImpersonateNamedPipeClient"));
            }
            let _impersonation = ImpersonationGuard;
            let mut peer_token = null_mut();
            if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, FALSE, &mut peer_token) }
                == FALSE
            {
                return Err(os_error("OpenThreadToken(peer)"));
            }
            let peer_token = OwnedHandle(peer_token);
            let observed = token_user_sid(peer_token.raw())?;
            if !super::peer_sid_is_admitted(&self.expected_sid, &observed) {
                return Err(PrivateIpcError::PeerIdentityMismatch {
                    expected: self.expected_sid,
                    observed,
                });
            }
            drop(_impersonation);
            Ok(PrivatePipeConnection {
                handle: self.handle,
                peer_sid: observed,
            })
        }
    }

    pub struct PrivatePipeConnection {
        handle: OwnedHandle,
        peer_sid: String,
    }

    impl PrivatePipeConnection {
        pub fn peer_sid(&self) -> &str {
            &self.peer_sid
        }

        pub fn read_frame(&self) -> Result<Vec<u8>, PrivateIpcError> {
            let mut header = [0u8; 4];
            self.read_exact(&mut header)?;
            let length = u32::from_le_bytes(header) as usize;
            super::validate_frame_length(length)?;
            let mut frame = vec![0u8; length];
            self.read_exact(&mut frame)?;
            Ok(frame)
        }

        pub fn write_frame(&self, frame: &[u8]) -> Result<(), PrivateIpcError> {
            super::validate_frame_length(frame.len())?;
            self.write_all(&(frame.len() as u32).to_le_bytes())?;
            self.write_all(frame)
        }

        fn read_exact(&self, buffer: &mut [u8]) -> Result<(), PrivateIpcError> {
            let mut offset = 0;
            while offset < buffer.len() {
                let mut read = 0;
                if unsafe {
                    ReadFile(
                        self.handle.raw(),
                        buffer[offset..].as_mut_ptr().cast(),
                        (buffer.len() - offset) as Dword,
                        &mut read,
                        null_mut(),
                    )
                } == FALSE
                {
                    return Err(os_error("ReadFile(private IPC frame)"));
                }
                if read == 0 {
                    return Err(PrivateIpcError::Os {
                        operation: "ReadFile(private IPC frame EOF)",
                        source: io::Error::new(io::ErrorKind::UnexpectedEof, "named pipe closed"),
                    });
                }
                offset += read as usize;
            }
            Ok(())
        }

        fn write_all(&self, buffer: &[u8]) -> Result<(), PrivateIpcError> {
            let mut offset = 0;
            while offset < buffer.len() {
                let mut written = 0;
                if unsafe {
                    WriteFile(
                        self.handle.raw(),
                        buffer[offset..].as_ptr().cast(),
                        (buffer.len() - offset) as Dword,
                        &mut written,
                        null_mut(),
                    )
                } == FALSE
                {
                    return Err(os_error("WriteFile(private IPC frame)"));
                }
                if written == 0 {
                    return Err(PrivateIpcError::Os {
                        operation: "WriteFile(private IPC frame zero progress)",
                        source: io::Error::new(io::ErrorKind::WriteZero, "named pipe wrote zero"),
                    });
                }
                offset += written as usize;
            }
            Ok(())
        }
    }

    impl Drop for PrivatePipeConnection {
        fn drop(&mut self) {
            unsafe {
                DisconnectNamedPipe(self.handle.raw());
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::fs::OpenOptions;
        use std::io::{Read, Write};
        use std::time::{SystemTime, UNIX_EPOCH};

        fn unique_endpoint(label: &str) -> String {
            format!(
                "{label}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock after epoch")
                    .as_nanos()
            )
        }

        #[test]
        fn applies_current_user_only_dacl_and_first_instance_guard() {
            let endpoint = unique_endpoint("acl");
            let listener = PrivatePipeListener::bind(&endpoint).expect("bind private pipe");
            let dacl = listener
                .applied_dacl_snapshot()
                .expect("read applied DACL");
            assert!(dacl.0, "DACL must be protected");
            assert_eq!(dacl.1.len(), 1, "DACL must contain one ACE: {dacl:?}");
            assert_eq!(dacl.1[0].0, ACCESS_ALLOWED_ACE_TYPE);
            assert_eq!(dacl.1[0].1, GENERIC_ALL);
            assert_eq!(dacl.1[0].2, listener.expected_sid());
            assert_ne!(dacl.1[0].2, "S-1-1-0", "Everyone must not be admitted");
            assert_ne!(dacl.1[0].2, "S-1-5-11", "Authenticated Users must not be admitted");
            assert!(PrivatePipeListener::bind(&endpoint).is_err());
        }

        #[test]
        fn authenticates_the_connected_peer_as_the_current_user() {
            let endpoint = unique_endpoint("peer");
            let listener = PrivatePipeListener::bind(&endpoint).expect("bind private pipe");
            let path = listener.path().to_owned();
            let expected = listener.expected_sid().to_owned();
            let server = std::thread::spawn(move || {
                let connection = listener
                    .accept_current_user()
                    .expect("accept current-user client");
                assert_eq!(connection.peer_sid(), expected);
                assert_eq!(
                    connection.read_frame().expect("read bounded client frame"),
                    b"client"
                );
                connection
                    .write_frame(b"server")
                    .expect("write bounded server frame");
                assert_eq!(
                    connection.read_frame().expect("read client close ack"),
                    b"ack"
                );
            });
            let mut client = OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .expect("open local named pipe client");
            client
                .write_all(&[0x47])
                .expect("write private IPC preface");
            client
                .write_all(&(6u32).to_le_bytes())
                .expect("write client frame length");
            client.write_all(b"client").expect("write client frame");
            let mut response_length = [0u8; 4];
            client
                .read_exact(&mut response_length)
                .expect("read server frame length");
            let mut response = vec![0u8; u32::from_le_bytes(response_length) as usize];
            client.read_exact(&mut response).expect("read server frame");
            assert_eq!(response, b"server");
            client
                .write_all(&(3u32).to_le_bytes())
                .expect("write ack frame length");
            client.write_all(b"ack").expect("write ack frame");
            server.join().expect("server thread");
            drop(client);
        }
    }
}

#[cfg(windows)]
pub use platform::{current_user_sid, PrivatePipeConnection, PrivatePipeListener};

#[cfg(not(windows))]
pub struct PrivatePipeListener;

#[cfg(not(windows))]
impl PrivatePipeListener {
    pub fn bind(endpoint: &str) -> Result<Self, PrivateIpcError> {
        validate_endpoint(endpoint)?;
        Err(PrivateIpcError::UnsupportedPlatform)
    }
}

#[cfg(test)]
mod portable_tests {
    use super::*;

    #[test]
    fn rejects_noncanonical_endpoint_names_before_any_os_call() {
        for endpoint in [
            "",
            "../escape",
            r"name\pipe",
            "white space",
            "http://listener",
        ] {
            assert!(matches!(
                PrivatePipeListener::bind(endpoint),
                Err(PrivateIpcError::InvalidEndpoint)
            ));
        }
    }

    #[test]
    fn enforces_the_fixed_four_mebibyte_frame_limit() {
        assert!(validate_frame_length(MAX_FRAME_BYTES).is_ok());
        assert!(matches!(
            validate_frame_length(MAX_FRAME_BYTES + 1),
            Err(PrivateIpcError::FrameTooLarge { .. })
        ));
    }

    #[test]
    fn production_pipe_flags_reject_remote_clients() {
        assert_ne!(
            private_pipe_mode_flags() & PIPE_REJECT_REMOTE_CLIENTS_FLAG,
            0,
            "production CreateNamedPipeW mode must include PIPE_REJECT_REMOTE_CLIENTS"
        );
    }

    #[test]
    fn peer_admission_requires_the_exact_expected_sid() {
        assert!(peer_sid_is_admitted("S-1-5-21-100", "S-1-5-21-100"));
        assert!(!peer_sid_is_admitted("S-1-5-21-100", "S-1-5-21-101"));
        assert!(!peer_sid_is_admitted("S-1-5-21-100", ""));
    }
}
