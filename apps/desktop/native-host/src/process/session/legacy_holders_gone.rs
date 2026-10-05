//! Kernel-backed proof that exact legacy process holders predate this boot.
//!
//! The persisted boot-environment GUID names a BCD entry, not a reboot. Here
//! class 3 returns the kernel's boot FILETIME, in the same epoch and units as
//! GetProcessTimes creation time. Both the process-object check and the strict
//! boot boundary are necessary for a holder-gone observation.

use std::ffi::c_void;
use std::fmt;
use std::mem::size_of;

const SYSTEM_TIME_OF_DAY_INFORMATION: u32 = 3;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const SYNCHRONIZE: u32 = 0x0010_0000;
const ERROR_INVALID_PARAMETER: i32 = 87;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 258;
const WAIT_FAILED: u32 = 0xffff_ffff;

type Handle = *mut c_void;
type NtQuerySystemInformation = unsafe extern "system" fn(u32, *mut c_void, u32, *mut u32) -> i32;

// Pinned phnt ntexapi.h ef43345b48301ada8f01b7ce194ddbf45b15bc28.
// Microsoft documents class 3 as an opaque 48-byte structure; the private
// BootTime member is fail-closed when the ABI or return length changes.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SystemTimeOfDayInformation {
    boot_time: i64,
    _current_time: i64,
    _time_zone_bias: i64,
    _time_zone_id: u32,
    _reserved: u32,
    _boot_time_bias: u64,
    _sleep_time_bias: u64,
}
const _: [(); 48] = [(); size_of::<SystemTimeOfDayInformation>()];

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FileTime {
    low: u32,
    high: u32,
}

impl FileTime {
    fn as_u64(self) -> u64 {
        (u64::from(self.high) << 32) | u64::from(self.low)
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> Handle;
    fn GetProcAddress(module: Handle, name: *const u8) -> *mut c_void;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    fn GetProcessTimes(
        process: Handle,
        creation: *mut FileTime,
        exit: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn CloseHandle(handle: Handle) -> i32;
}

struct OwnedHandle(Handle);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// Private, native-only observation. No wire or database field can construct it.
pub(crate) struct NativeLegacyHoldersGone {
    original_pairs: Vec<(u32, u64)>,
    boot_start: u64,
    #[cfg(test)]
    fixture_only: bool,
}

impl NativeLegacyHoldersGone {
    pub(crate) fn observe(
        original_pairs: &[(u32, u64)],
    ) -> Result<Self, NativeLegacyHoldersGoneError> {
        let original_pairs = canonical_pairs(original_pairs)?;
        let boot_start = query_boot_start()?;
        check_boot_boundary(&original_pairs, boot_start)?;
        check_holders(&original_pairs)?;
        Ok(Self {
            original_pairs,
            boot_start,
            #[cfg(test)]
            fixture_only: false,
        })
    }

    pub(crate) fn boot_start(&self) -> u64 {
        self.boot_start
    }

    /// Rechecks the persisted exact pairs and native state immediately before
    /// the caller changes an ACL. A reboot or loss of query access rejects.
    pub(crate) fn validate(
        &self,
        original_pairs: &[(u32, u64)],
    ) -> Result<(), NativeLegacyHoldersGoneError> {
        if canonical_pairs(original_pairs)? != self.original_pairs {
            return Err(NativeLegacyHoldersGoneError::OriginalPairsChanged);
        }
        #[cfg(test)]
        if self.fixture_only {
            return check_boot_boundary(&self.original_pairs, self.boot_start);
        }
        let current_boot_start = query_boot_start()?;
        if current_boot_start != self.boot_start {
            return Err(NativeLegacyHoldersGoneError::BootStartChanged {
                observed: self.boot_start,
                current: current_boot_start,
            });
        }
        check_boot_boundary(&self.original_pairs, current_boot_start)?;
        check_holders(&self.original_pairs)
    }

    #[cfg(test)]
    pub(crate) fn for_test(original_pairs: &[(u32, u64)], boot_start: u64) -> Self {
        let original_pairs = canonical_pairs(original_pairs).expect("valid fixture holder pairs");
        check_boot_boundary(&original_pairs, boot_start)
            .expect("fixture boot after holder creation");
        Self {
            original_pairs,
            boot_start,
            fixture_only: true,
        }
    }
}

#[derive(Debug)]
pub(crate) enum NativeLegacyHoldersGoneError {
    InvalidOriginalPairs(&'static str),
    OriginalPairsChanged,
    SystemModuleUnavailable {
        win32_error: Option<i32>,
    },
    QueryExportUnavailable {
        win32_error: Option<i32>,
    },
    NtStatus(i32),
    UnexpectedLength {
        actual: u32,
        expected: u32,
    },
    InvalidBootTime(i64),
    BootStartChanged {
        observed: u64,
        current: u64,
    },
    HolderCreatedThisBoot {
        pid: u32,
        creation_time_100ns: u64,
        boot_start: u64,
    },
    OpenProcess {
        pid: u32,
        win32_error: Option<i32>,
    },
    GetProcessTimes {
        pid: u32,
        win32_error: Option<i32>,
    },
    WaitForSingleObject {
        pid: u32,
        result: u32,
        win32_error: Option<i32>,
    },
    ExactHolderAlive {
        pid: u32,
        creation_time_100ns: u64,
    },
}

impl fmt::Display for NativeLegacyHoldersGoneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOriginalPairs(detail) => write!(f, "invalid original holder pairs: {detail}"),
            Self::OriginalPairsChanged => f.write_str("original holder pairs changed"),
            Self::SystemModuleUnavailable { win32_error } => write!(f, "GetModuleHandleW(ntdll.dll) failed: Win32={win32_error:?}"),
            Self::QueryExportUnavailable { win32_error } => write!(f, "GetProcAddress(NtQuerySystemInformation) failed: Win32={win32_error:?}"),
            Self::NtStatus(status) => write!(f, "NtQuerySystemInformation(class 3) failed: NTSTATUS=0x{:08X}", *status as u32),
            Self::UnexpectedLength { actual, expected } => write!(f, "NtQuerySystemInformation(class 3) returned {actual} bytes; expected {expected}"),
            Self::InvalidBootTime(value) => write!(f, "NtQuerySystemInformation(class 3) returned invalid BootTime={value}"),
            Self::BootStartChanged { observed, current } => write!(f, "kernel boot start changed: observed={observed} current={current}"),
            Self::HolderCreatedThisBoot { pid, creation_time_100ns, boot_start } => write!(f,
                "holder pid={pid} creation_time_100ns={creation_time_100ns} is not before kernel boot start={boot_start}"),
            Self::OpenProcess { pid, win32_error } => write!(f, "OpenProcess(pid={pid}) failed: Win32={win32_error:?}"),
            Self::GetProcessTimes { pid, win32_error } => write!(f, "GetProcessTimes(pid={pid}) failed: Win32={win32_error:?}"),
            Self::WaitForSingleObject { pid, result, win32_error } => write!(f,
                "WaitForSingleObject(pid={pid}, 0) failed: result={result} Win32={win32_error:?}"),
            Self::ExactHolderAlive { pid, creation_time_100ns } => write!(f,
                "exact holder remains alive: pid={pid} creation_time_100ns={creation_time_100ns}"),
        }
    }
}

impl std::error::Error for NativeLegacyHoldersGoneError {}

fn canonical_pairs(pairs: &[(u32, u64)]) -> Result<Vec<(u32, u64)>, NativeLegacyHoldersGoneError> {
    if pairs.is_empty() {
        return Err(NativeLegacyHoldersGoneError::InvalidOriginalPairs("empty"));
    }
    let mut canonical = pairs.to_vec();
    if canonical
        .iter()
        .any(|(pid, created)| *pid == 0 || *created == 0)
    {
        return Err(NativeLegacyHoldersGoneError::InvalidOriginalPairs(
            "zero pid or creation time",
        ));
    }
    canonical.sort_unstable();
    if canonical.windows(2).any(|window| window[0] == window[1]) {
        return Err(NativeLegacyHoldersGoneError::InvalidOriginalPairs(
            "duplicate exact pair",
        ));
    }
    Ok(canonical)
}

fn check_boot_boundary(
    pairs: &[(u32, u64)],
    boot_start: u64,
) -> Result<(), NativeLegacyHoldersGoneError> {
    for &(pid, creation_time_100ns) in pairs {
        if boot_start <= creation_time_100ns {
            return Err(NativeLegacyHoldersGoneError::HolderCreatedThisBoot {
                pid,
                creation_time_100ns,
                boot_start,
            });
        }
    }
    Ok(())
}

fn checked_boot_response(
    status: i32,
    returned_length: u32,
    information: SystemTimeOfDayInformation,
) -> Result<u64, NativeLegacyHoldersGoneError> {
    if status < 0 {
        return Err(NativeLegacyHoldersGoneError::NtStatus(status));
    }
    let expected = size_of::<SystemTimeOfDayInformation>() as u32;
    if returned_length != expected {
        return Err(NativeLegacyHoldersGoneError::UnexpectedLength {
            actual: returned_length,
            expected,
        });
    }
    if information.boot_time <= 0 {
        return Err(NativeLegacyHoldersGoneError::InvalidBootTime(
            information.boot_time,
        ));
    }
    Ok(information.boot_time as u64)
}

fn query_boot_start() -> Result<u64, NativeLegacyHoldersGoneError> {
    const NTDLL_WIDE: &[u16] = &[110, 116, 100, 108, 108, 46, 100, 108, 108, 0];
    let module = unsafe { GetModuleHandleW(NTDLL_WIDE.as_ptr()) };
    if module.is_null() {
        return Err(NativeLegacyHoldersGoneError::SystemModuleUnavailable {
            win32_error: std::io::Error::last_os_error().raw_os_error(),
        });
    }
    let address = unsafe { GetProcAddress(module, b"NtQuerySystemInformation\0".as_ptr()) };
    if address.is_null() {
        return Err(NativeLegacyHoldersGoneError::QueryExportUnavailable {
            win32_error: std::io::Error::last_os_error().raw_os_error(),
        });
    }
    let query: NtQuerySystemInformation = unsafe { std::mem::transmute(address) };
    let mut information = SystemTimeOfDayInformation::default();
    let mut returned_length = 0;
    let status = unsafe {
        query(
            SYSTEM_TIME_OF_DAY_INFORMATION,
            &mut information as *mut _ as *mut c_void,
            size_of::<SystemTimeOfDayInformation>() as u32,
            &mut returned_length,
        )
    };
    checked_boot_response(status, returned_length, information)
}

/// Current kernel boot FILETIME for recording a new fence's metadata. The
/// numeric value alone does not authorize any legacy holder migration.
pub(crate) fn native_boot_start() -> Result<u64, NativeLegacyHoldersGoneError> {
    query_boot_start()
}

fn classify_open_error(
    pid: u32,
    win32_error: Option<i32>,
) -> Result<(), NativeLegacyHoldersGoneError> {
    if win32_error == Some(ERROR_INVALID_PARAMETER) {
        Ok(()) // The kernel reports that the PID does not name a process.
    } else {
        Err(NativeLegacyHoldersGoneError::OpenProcess { pid, win32_error })
    }
}

fn classify_identity(
    pid: u32,
    original_creation: u64,
    actual_creation: u64,
    wait: impl FnOnce() -> (u32, Option<i32>),
) -> Result<(), NativeLegacyHoldersGoneError> {
    if actual_creation != original_creation {
        return Ok(()); // The open handle pins a different, reused PID identity.
    }
    let (result, win32_error) = wait();
    match result {
        WAIT_OBJECT_0 => Ok(()),
        WAIT_TIMEOUT => Err(NativeLegacyHoldersGoneError::ExactHolderAlive {
            pid,
            creation_time_100ns: original_creation,
        }),
        _ => Err(NativeLegacyHoldersGoneError::WaitForSingleObject {
            pid,
            result,
            win32_error,
        }),
    }
}

fn check_holders(pairs: &[(u32, u64)]) -> Result<(), NativeLegacyHoldersGoneError> {
    let mut held_handles = Vec::with_capacity(pairs.len());
    for &(pid, original_creation) in pairs {
        let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, pid) };
        if raw.is_null() {
            let win32_error = std::io::Error::last_os_error().raw_os_error();
            classify_open_error(pid, win32_error)?;
            continue;
        }
        let handle = OwnedHandle(raw);
        let mut creation = FileTime::default();
        let mut exit = FileTime::default();
        let mut kernel = FileTime::default();
        let mut user = FileTime::default();
        if unsafe { GetProcessTimes(handle.0, &mut creation, &mut exit, &mut kernel, &mut user) }
            == 0
        {
            return Err(NativeLegacyHoldersGoneError::GetProcessTimes {
                pid,
                win32_error: std::io::Error::last_os_error().raw_os_error(),
            });
        }
        classify_identity(pid, original_creation, creation.as_u64(), || {
            let result = unsafe { WaitForSingleObject(handle.0, 0) };
            let win32_error = if result == WAIT_FAILED {
                std::io::Error::last_os_error().raw_os_error()
            } else {
                None
            };
            (result, win32_error)
        })?;
        // A different creation time identifies PID reuse. Keep this handle
        // open until every pair has been classified, pinning that identity.
        held_handles.push(handle);
    }
    drop(held_handles);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_three_requires_exact_length_positive_boot_and_preserves_status() {
        let mut response = SystemTimeOfDayInformation {
            boot_time: 100,
            ..Default::default()
        };
        let failure = 0xc000_0001u32 as i32;
        assert!(matches!(checked_boot_response(failure, 48, response),
            Err(NativeLegacyHoldersGoneError::NtStatus(value)) if value == failure));
        assert!(matches!(
            checked_boot_response(0, 47, response),
            Err(NativeLegacyHoldersGoneError::UnexpectedLength {
                actual: 47,
                expected: 48
            })
        ));
        response.boot_time = 0;
        assert!(matches!(
            checked_boot_response(0, 48, response),
            Err(NativeLegacyHoldersGoneError::InvalidBootTime(0))
        ));
    }

    #[test]
    fn exact_boundary_and_canonical_seal_reject_changes() {
        let proof = NativeLegacyHoldersGone::for_test(&[(22, 7), (11, 5)], 8);
        proof.validate(&[(11, 5), (22, 7)]).unwrap();
        assert!(matches!(
            proof.validate(&[(11, 5), (22, 6)]),
            Err(NativeLegacyHoldersGoneError::OriginalPairsChanged)
        ));
        assert!(matches!(
            check_boot_boundary(&[(22, 8)], 8),
            Err(NativeLegacyHoldersGoneError::HolderCreatedThisBoot { .. })
        ));
        assert!(matches!(
            check_boot_boundary(&[(22, 9)], 8),
            Err(NativeLegacyHoldersGoneError::HolderCreatedThisBoot { .. })
        ));
        assert!(matches!(
            canonical_pairs(&[(11, 5), (11, 5)]),
            Err(NativeLegacyHoldersGoneError::InvalidOriginalPairs(
                "duplicate exact pair"
            ))
        ));
        canonical_pairs(&[(11, 5), (11, 6)]).expect("reused PID in historical rows is distinct");
    }

    #[test]
    fn reused_pid_requires_a_different_creation_time() {
        // A genuinely different process object may be live: its wait state is
        // irrelevant to the original holder. Equal creation is never reuse.
        classify_identity(31, 100, 101, || panic!("must not wait on reused identity")).unwrap();
        assert!(matches!(
            classify_identity(31, 100, 100, || (WAIT_TIMEOUT, None)),
            Err(NativeLegacyHoldersGoneError::ExactHolderAlive {
                pid: 31,
                creation_time_100ns: 100
            })
        ));
        classify_identity(31, 100, 100, || (WAIT_OBJECT_0, None)).unwrap();
    }

    #[test]
    fn only_invalid_parameter_proves_absent_pid() {
        classify_open_error(31, Some(ERROR_INVALID_PARAMETER)).unwrap();
        assert!(matches!(
            classify_open_error(31, Some(5)),
            Err(NativeLegacyHoldersGoneError::OpenProcess {
                pid: 31,
                win32_error: Some(5)
            })
        ));
        assert!(matches!(
            classify_open_error(31, None),
            Err(NativeLegacyHoldersGoneError::OpenProcess {
                pid: 31,
                win32_error: None
            })
        ));
        assert!(matches!(
            classify_identity(31, 100, 100, || (WAIT_FAILED, Some(6))),
            Err(NativeLegacyHoldersGoneError::WaitForSingleObject {
                pid: 31,
                result: WAIT_FAILED,
                win32_error: Some(6)
            })
        ));
    }

    #[test]
    fn current_process_cannot_be_a_legacy_holder() {
        let pid = std::process::id();
        let handle = OwnedHandle(unsafe {
            OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, pid)
        });
        assert!(!handle.0.is_null());
        let mut creation = FileTime::default();
        let mut exit = FileTime::default();
        let mut kernel = FileTime::default();
        let mut user = FileTime::default();
        assert_ne!(
            unsafe { GetProcessTimes(handle.0, &mut creation, &mut exit, &mut kernel, &mut user) },
            0
        );
        let created = creation.as_u64();
        let boot = query_boot_start().expect("kernel class 3 boot time");
        assert!(created >= boot);
        assert!(matches!(
            NativeLegacyHoldersGone::observe(&[(pid, created)]),
            Err(NativeLegacyHoldersGoneError::HolderCreatedThisBoot { .. })
        ));
        assert!(matches!(
            check_holders(&[(pid, created)]),
            Err(NativeLegacyHoldersGoneError::ExactHolderAlive { .. })
        ));
        assert_eq!(unsafe { WaitForSingleObject(handle.0, 0) }, WAIT_TIMEOUT);

        // The same live PID is attributable to a different identity only
        // when its actual creation differs from the original pair.
        let prior_boot_creation = boot.checked_sub(1).expect("positive boot FILETIME");
        let reused = NativeLegacyHoldersGone::observe(&[(pid, prior_boot_creation)])
            .expect("current PID is not the preboot holder");
        assert_eq!(reused.boot_start(), boot);
        reused.validate(&[(pid, prior_boot_creation)]).unwrap();
    }

    #[test]
    fn stopped_process_created_this_boot_still_cannot_pass() {
        use std::os::windows::io::AsRawHandle;
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/C", "exit", "/B", "0"])
            .spawn()
            .expect("spawn ordinary child");
        let pid = child.id();
        child.wait().expect("child exit");
        let mut creation = FileTime::default();
        let mut exit = FileTime::default();
        let mut kernel = FileTime::default();
        let mut user = FileTime::default();
        let handle = child.as_raw_handle() as Handle;
        assert_ne!(
            unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) },
            0
        );
        assert_eq!(unsafe { WaitForSingleObject(handle, 0) }, WAIT_OBJECT_0);
        assert!(matches!(
            NativeLegacyHoldersGone::observe(&[(pid, creation.as_u64())]),
            Err(NativeLegacyHoldersGoneError::HolderCreatedThisBoot { .. })
        ));
        check_holders(&[(pid, creation.as_u64())])
            .expect("exact stopped process is no longer live");
    }

    #[test]
    fn native_boot_query_is_stable_within_one_boot() {
        let first = query_boot_start().expect("first kernel class 3 query");
        let second = query_boot_start().expect("second kernel class 3 query");
        assert_eq!(first, second);
    }
}
