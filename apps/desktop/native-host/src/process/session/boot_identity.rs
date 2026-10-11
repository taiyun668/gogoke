//! Read the kernel's current boot identifier for reboot-boundary custody.
//!
//! `NtQuerySystemInformation(SystemBootEnvironmentInformation)` is a private
//! Windows interface and may change or disappear. Resolution, NTSTATUS, exact
//! output size, and a nonzero identifier are required; there is no fallback to
//! caller-provided or time-derived values.
//!
//! The class value and 32-byte layout follow the pinned phnt declaration at
//! `systeminformer/phnt/include/ntexapi.h` commit
//! `ef43345b48301ada8f01b7ce194ddbf45b15bc28`. Microsoft documents the API as
//! private and changeable and recommends runtime dynamic linking.

use std::fmt;
use std::mem::size_of;

#[cfg(windows)]
const SYSTEM_BOOT_ENVIRONMENT_INFORMATION: u32 = 90;
const BOOT_IDENTIFIER_LENGTH: usize = 16;

#[repr(C)]
#[derive(Default)]
struct SystemBootEnvironmentInformation {
    boot_identifier: [u8; BOOT_IDENTIFIER_LENGTH],
    _firmware_type: u32,
    _alignment: u32,
    _boot_flags: u64,
}

const _: [(); 32] = [(); size_of::<SystemBootEnvironmentInformation>()];

/// A boot identifier observed directly from the current Windows kernel.
///
/// The private field and constructor prevent callers from promoting arbitrary
/// strings or IDs into a trusted observation. This type intentionally omits
/// `Debug` so logging it does not disclose the boot identifier.
pub(crate) struct NativeBootIdentity {
    guid_bytes: [u8; BOOT_IDENTIFIER_LENGTH],
}

impl NativeBootIdentity {
    pub(crate) fn observe() -> Result<Self, NativeBootIdentityError> {
        #[cfg(windows)]
        {
            let query = resolve_query()?;
            let mut information = SystemBootEnvironmentInformation::default();
            let mut returned_length = 0;
            let status = unsafe {
                query(
                    SYSTEM_BOOT_ENVIRONMENT_INFORMATION,
                    &mut information as *mut SystemBootEnvironmentInformation as *mut _,
                    size_of::<SystemBootEnvironmentInformation>() as u32,
                    &mut returned_length,
                )
            };
            checked_response(status, returned_length, information)
        }

        #[cfg(not(windows))]
        {
            Err(NativeBootIdentityError::UnsupportedPlatform)
        }
    }

    /// Lowercase hexadecimal of the original 16 GUID bytes, with no
    /// separators. Keep this inside the native host; it must not be logged.
    pub(crate) fn canonical_hex(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut result = String::with_capacity(BOOT_IDENTIFIER_LENGTH * 2);
        for byte in self.guid_bytes {
            result.push(HEX[(byte >> 4) as usize] as char);
            result.push(HEX[(byte & 0x0f) as usize] as char);
        }
        result
    }
}

#[derive(Debug)]
pub(crate) enum NativeBootIdentityError {
    #[cfg(not(windows))]
    UnsupportedPlatform,
    #[cfg(windows)]
    SystemModuleUnavailable {
        win32_error: Option<i32>,
    },
    #[cfg(windows)]
    QueryExportUnavailable {
        win32_error: Option<i32>,
    },
    NtStatus(i32),
    UnexpectedLength {
        actual: u32,
        expected: u32,
    },
    ZeroIdentifier,
}

impl fmt::Display for NativeBootIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            #[cfg(not(windows))]
            Self::UnsupportedPlatform => {
                formatter.write_str("native boot identity requires Windows")
            }
            #[cfg(windows)]
            Self::SystemModuleUnavailable { win32_error } => write!(
                formatter,
                "GetModuleHandleW(ntdll.dll) failed: {win32_error:?}"
            ),
            #[cfg(windows)]
            Self::QueryExportUnavailable { win32_error } => write!(
                formatter,
                "GetProcAddress(NtQuerySystemInformation) failed: {win32_error:?}"
            ),
            Self::NtStatus(status) => write!(
                formatter,
                "NtQuerySystemInformation failed: NTSTATUS=0x{:08X}",
                *status as u32
            ),
            Self::UnexpectedLength { actual, expected } => write!(
                formatter,
                "NtQuerySystemInformation returned {actual} bytes; expected {expected}"
            ),
            Self::ZeroIdentifier => {
                formatter.write_str("NtQuerySystemInformation returned a zero boot identifier")
            }
        }
    }
}

impl std::error::Error for NativeBootIdentityError {}

fn checked_response(
    status: i32,
    returned_length: u32,
    information: SystemBootEnvironmentInformation,
) -> Result<NativeBootIdentity, NativeBootIdentityError> {
    if status < 0 {
        return Err(NativeBootIdentityError::NtStatus(status));
    }

    let expected_length = size_of::<SystemBootEnvironmentInformation>() as u32;
    if returned_length != expected_length {
        return Err(NativeBootIdentityError::UnexpectedLength {
            actual: returned_length,
            expected: expected_length,
        });
    }
    if information.boot_identifier.iter().all(|byte| *byte == 0) {
        return Err(NativeBootIdentityError::ZeroIdentifier);
    }

    Ok(NativeBootIdentity {
        guid_bytes: information.boot_identifier,
    })
}

#[cfg(windows)]
type NtQuerySystemInformation =
    unsafe extern "system" fn(u32, *mut std::ffi::c_void, u32, *mut u32) -> i32;

#[cfg(windows)]
fn resolve_query() -> Result<NtQuerySystemInformation, NativeBootIdentityError> {
    use std::ffi::c_void;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    }

    // ntdll is already loaded by Windows. Reuse the host's existing
    // GetModuleHandleW/GetProcAddress pattern; do not search the working
    // directory or load a caller-controlled copy.
    const NTDLL_WIDE: &[u16] = &[
        b'n' as u16,
        b't' as u16,
        b'd' as u16,
        b'l' as u16,
        b'l' as u16,
        b'.' as u16,
        b'd' as u16,
        b'l' as u16,
        b'l' as u16,
        0,
    ];
    let module = unsafe { GetModuleHandleW(NTDLL_WIDE.as_ptr()) };
    if module.is_null() {
        return Err(NativeBootIdentityError::SystemModuleUnavailable {
            win32_error: std::io::Error::last_os_error().raw_os_error(),
        });
    }

    let address = unsafe { GetProcAddress(module, b"NtQuerySystemInformation\0".as_ptr()) };
    if address.is_null() {
        return Err(NativeBootIdentityError::QueryExportUnavailable {
            win32_error: std::io::Error::last_os_error().raw_os_error(),
        });
    }

    // The export ABI is the NTAPI/system ABI. Microsoft documents that this
    // private API may change; any incompatibility fails closed in the caller.
    Ok(unsafe { std::mem::transmute::<*mut c_void, NtQuerySystemInformation>(address) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observe_with_query(
        query: impl FnOnce(&mut SystemBootEnvironmentInformation, &mut u32) -> i32,
    ) -> Result<NativeBootIdentity, NativeBootIdentityError> {
        let mut information = SystemBootEnvironmentInformation::default();
        let mut returned_length = 0;
        let status = query(&mut information, &mut returned_length);
        checked_response(status, returned_length, information)
    }

    #[test]
    fn rejects_ntstatus_and_preserves_the_original_status() {
        let status = 0xC000_0001u32 as i32;
        let error = observe_with_query(|information, returned_length| {
            information.boot_identifier = [0xA5; BOOT_IDENTIFIER_LENGTH];
            *returned_length = 32;
            status
        })
        .err()
        .expect("NTSTATUS failure must reject");
        assert!(matches!(error, NativeBootIdentityError::NtStatus(value) if value == status));
    }

    #[test]
    fn requires_exact_native_structure_length() {
        for actual in [31, 33] {
            let error = observe_with_query(|information, returned_length| {
                information.boot_identifier = [0xA5; BOOT_IDENTIFIER_LENGTH];
                *returned_length = actual;
                0
            })
            .err()
            .expect("unexpected output size must reject");
            assert!(matches!(error, NativeBootIdentityError::UnexpectedLength {
                actual: value, expected: 32
            } if value == actual));
        }
    }

    #[test]
    fn rejects_zero_boot_identifier() {
        let error = observe_with_query(|_, returned_length| {
            *returned_length = 32;
            0
        })
        .err()
        .expect("zero boot GUID must reject");
        assert!(matches!(error, NativeBootIdentityError::ZeroIdentifier));
    }

    #[test]
    fn canonical_hex_preserves_the_observed_guid_bytes() {
        let identity = observe_with_query(|information, returned_length| {
            information.boot_identifier = [
                0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x10, 0x32, 0x54, 0x76, 0x98, 0xba,
                0xdc, 0xfe,
            ];
            *returned_length = 32;
            0
        })
        .unwrap();
        assert_eq!(identity.canonical_hex(), "0123456789abcdef1032547698badcfe");
    }

    #[cfg(windows)]
    #[test]
    fn ordinary_user_native_observation_is_stable_within_one_boot() {
        let first = NativeBootIdentity::observe().expect("first current-boot query");
        let second = NativeBootIdentity::observe().expect("second current-boot query");
        assert_eq!(first.canonical_hex(), second.canonical_hex());
    }
}
