use std::collections::{HashMap, HashSet};
use std::ffi::{c_void, OsStr};
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, TryLockError};
use std::thread;
use std::time::{Duration, Instant};
use super::session::{AppContainerProfile, CompatModule, SecurityCapabilities};
use crate::ipc::PeerProcessHandle;

type Handle = *mut c_void;

const CREATE_SUSPENDED: u32 = 0x0000_0004;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
const CREATE_UNICODE_ENVIRONMENT: u32 = 0x0000_0400;
const STARTF_USESTDHANDLES: u32 = 0x0000_0100;
const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: usize = 0x0002_0002;
const PROC_THREAD_ATTRIBUTE_JOB_LIST: usize = 0x0002_000d;
const PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES: usize = 0x0002_0009;
const PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY: usize = 0x0002_000f;
const ALL_APPLICATION_PACKAGES_OPT_OUT: u32 = 1;
const HANDLE_FLAG_INHERIT: u32 = 0x0000_0001;
const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: i32 = 9;
const JOB_OBJECT_BASIC_ACCOUNTING_INFORMATION_CLASS: i32 = 1;
const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;
const STILL_ACTIVE: u32 = 259;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 258;
const WAIT_FAILED: u32 = 0xffff_ffff;
const INFINITE: u32 = 0xffff_ffff;
const RESUME_FAILED: u32 = 0xffff_ffff;
const DUPLICATE_SAME_ACCESS: u32 = 0x0000_0002;

pub const STOP_GRACE_MS: u32 = 10_000;
pub const STOP_TERMINATE_MS: u32 = 5_000;
pub const STOP_OBSERVE_MS: u32 = 5_000;
pub const HOST_STOP_DEADLINE_MS: u32 = 30_000;
pub const STOP_TIMEOUT_EXIT_CODE: u32 = 124;
pub const STOP_REFUSED_EXIT_CODE: u32 = 125;
const CONTROLLED_FIXTURE_SHA256: &str = "sha256:2e66dac4ee497e023fd8d860178c77ef5b82e01b7bcd23dc868e9db80637f85c";
/// Matches the Codex adapter's maximum JSONL frame (including LF).
const PERSISTENT_FRAME_MAX_BYTES: usize = 1024 * 1024;

#[repr(C)]
#[derive(Clone, Copy)]
struct FileTime {
    low: u32,
    high: u32,
}

#[link(name = "bcrypt")]
extern "system" {
    fn BCryptGenRandom(algorithm: Handle, buffer: *mut u8, buffer_len: u32, flags: u32) -> i32;
}

#[repr(C)]
struct StartupInfoW {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    x_size: u32,
    y_size: u32,
    x_count_chars: u32,
    y_count_chars: u32,
    fill_attribute: u32,
    flags: u32,
    show_window: u16,
    reserved2_size: u16,
    reserved2: *mut u8,
    std_input: Handle,
    std_output: Handle,
    std_error: Handle,
}

#[repr(C)]
struct StartupInfoExW {
    startup: StartupInfoW,
    attributes: *mut c_void,
}

#[repr(C)]
struct SecurityAttributes {
    length: u32,
    descriptor: *mut c_void,
    inherit: i32,
}

#[repr(C)]
struct ProcessInformation {
    process: Handle,
    thread: Handle,
    process_id: u32,
    thread_id: u32,
}

#[repr(C)]
#[derive(Default)]
struct JobObjectBasicLimitInformation {
    per_process_user_time_limit: i64,
    per_job_user_time_limit: i64,
    limit_flags: u32,
    minimum_working_set_size: usize,
    maximum_working_set_size: usize,
    active_process_limit: u32,
    affinity: usize,
    priority_class: u32,
    scheduling_class: u32,
}

#[repr(C)]
#[derive(Default)]
struct IoCounters {
    read_operation_count: u64,
    write_operation_count: u64,
    other_operation_count: u64,
    read_transfer_count: u64,
    write_transfer_count: u64,
    other_transfer_count: u64,
}

#[repr(C)]
#[derive(Default)]
struct JobObjectExtendedLimitInformation {
    basic_limit_information: JobObjectBasicLimitInformation,
    io_info: IoCounters,
    process_memory_limit: usize,
    job_memory_limit: usize,
    peak_process_memory_used: usize,
    peak_job_memory_used: usize,
}

#[repr(C)]
#[derive(Default)]
struct JobObjectBasicAccountingInformation {
    total_user_time: i64,
    total_kernel_time: i64,
    this_period_total_user_time: i64,
    this_period_total_kernel_time: i64,
    total_page_fault_count: u32,
    total_processes: u32,
    active_processes: u32,
    total_terminated_processes: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(name: *const u16, desired_access: u32, share_mode: u32,
        security_attributes: *const c_void, creation_disposition: u32,
        flags_and_attributes: u32, template_file: Handle) -> Handle;
    fn CreateProcessW(
        application_name: *const u16,
        command_line: *mut u16,
        process_attributes: *const c_void,
        thread_attributes: *const c_void,
        inherit_handles: i32,
        creation_flags: u32,
        environment: *const c_void,
        current_directory: *const u16,
        startup_info: *mut StartupInfoW,
        process_information: *mut ProcessInformation,
    ) -> i32;
    fn CreatePipe(read: *mut Handle, write: *mut Handle, attributes: *const SecurityAttributes, size: u32) -> i32;
    fn InitializeProcThreadAttributeList(list: *mut c_void, count: u32, flags: u32, size: *mut usize) -> i32;
    fn UpdateProcThreadAttribute(list: *mut c_void, flags: u32, attribute: usize, value: *mut c_void,
        value_size: usize, previous: *mut c_void, return_size: *mut usize) -> i32;
    fn DeleteProcThreadAttributeList(list: *mut c_void);
    fn ReadFile(file: Handle, buffer: *mut c_void, length: u32, read: *mut u32, overlapped: *mut c_void) -> i32;
    fn WriteFile(file: Handle, buffer: *const c_void, length: u32, written: *mut u32, overlapped: *mut c_void) -> i32;
    fn PeekNamedPipe(file: Handle, buffer: *mut c_void, buffer_size: u32, read: *mut u32,
        available: *mut u32, bytes_left: *mut u32) -> i32;
    fn GetCurrentProcess() -> Handle;
    fn GetProcessMitigationPolicy(process: Handle, policy: i32,
        buffer: *mut c_void, length: usize) -> i32;
    fn DuplicateHandle(source_process: Handle, source: Handle, target_process: Handle,
        target: *mut Handle, access: u32, inherit: i32, options: u32) -> i32;
    fn CancelSynchronousIo(thread: Handle) -> i32;
    fn GetWindowsDirectoryW(buffer: *mut u16, size: u32) -> u32;
    fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> Handle;
    fn SetInformationJobObject(
        job: Handle,
        information_class: i32,
        information: *const c_void,
        information_length: u32,
    ) -> i32;
    fn QueryInformationJobObject(
        job: Handle,
        information_class: i32,
        information: *mut c_void,
        information_length: u32,
        return_length: *mut u32,
    ) -> i32;
    fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
    fn IsProcessInJob(process: Handle, job: Handle, result: *mut i32) -> i32;
    fn TerminateJobObject(job: Handle, exit_code: u32) -> i32;
    fn TerminateProcess(process: Handle, exit_code: u32) -> i32;
    fn ResumeThread(thread: Handle) -> u32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn GetExitCodeProcess(process: Handle, exit_code: *mut u32) -> i32;
    fn GetProcessTimes(
        process: Handle,
        creation: *mut FileTime,
        exit: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;
    fn QueryFullProcessImageNameW(
        process: Handle,
        flags: u32,
        image_name: *mut u16,
        size: *mut u32,
    ) -> i32;
    fn SetHandleInformation(object: Handle, mask: u32, flags: u32) -> i32;
    fn GetHandleInformation(object: Handle, flags: *mut u32) -> i32;
    fn CloseHandle(object: Handle) -> i32;
}

#[derive(Debug)]
pub enum ProcessCustodyError {
    InvalidLaunch(&'static str),
    CreateJob(io::Error),
    ConfigureJob(io::Error),
    CreateProcess(io::Error),
    AssignJob(io::Error),
    CaptureIdentity(io::Error),
    HandlePolicy(io::Error),
    DurableCustody(String),
    Resume(io::Error),
    DuplicateTicket(String),
    TicketNotFound(String),
    DurableIdentityMismatch(String),
    ReleaseWithoutDurableStop(String),
    Random(io::Error),
    BinaryDigest(io::Error),
    BindingMismatch(&'static str),
    ProtocolPipe(io::Error),
    ProtocolAttribute(io::Error),
    ProtocolEnvironment(io::Error),
    ProtocolEvidence { cause: Box<ProcessCustodyError>, stderr_tail: String },
    Isolation(String),
    LaunchCleanup { cause: Box<ProcessCustodyError>, detail: String },
}

impl fmt::Display for ProcessCustodyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLaunch(reason) => write!(f, "PROCESS_LAUNCH_INVALID: {reason}"),
            Self::CreateJob(source) => write!(f, "PROCESS_JOB_CREATE_FAILED: {source}"),
            Self::ConfigureJob(source) => write!(f, "PROCESS_JOB_CONFIGURE_FAILED: {source}"),
            Self::CreateProcess(source) => write!(f, "PROCESS_SUSPENDED_CREATE_FAILED: {source}"),
            Self::AssignJob(source) => write!(f, "PROCESS_JOB_ASSIGN_FAILED: {source}"),
            Self::CaptureIdentity(source) => write!(f, "PROCESS_IDENTITY_FAILED: {source}"),
            Self::HandlePolicy(source) => write!(f, "PROCESS_HANDLE_POLICY_FAILED: {source}"),
            Self::DurableCustody(reason) => write!(f, "PROCESS_DURABLE_CUSTODY_FAILED: {reason}"),
            Self::Resume(source) => write!(f, "PROCESS_RESUME_FAILED: {source}"),
            Self::DuplicateTicket(ticket) => {
                write!(f, "PROCESS_TICKET_ALREADY_EXISTS: {ticket}")
            }
            Self::TicketNotFound(ticket) => write!(f, "PROCESS_TICKET_NOT_FOUND: {ticket}"),
            Self::DurableIdentityMismatch(ticket) => {
                write!(f, "PROCESS_DURABLE_IDENTITY_MISMATCH: {ticket}")
            }
            Self::ReleaseWithoutDurableStop(ticket) => {
                write!(f, "PROCESS_RELEASE_WITHOUT_DURABLE_STOP: {ticket}")
            }
            Self::Random(source) => write!(f, "PROCESS_RANDOM_FAILED: {source}"),
            Self::BinaryDigest(source) => write!(f, "PROCESS_BINARY_DIGEST_FAILED: {source}"),
            Self::BindingMismatch(field) => write!(f, "PROCESS_BINDING_MISMATCH: {field}"),
            Self::ProtocolPipe(source) => write!(f, "PROCESS_PROTOCOL_PIPE_FAILED: {source}"),
            Self::ProtocolAttribute(source) => write!(f, "PROCESS_PROTOCOL_ATTRIBUTE_FAILED: {source}"),
            Self::ProtocolEnvironment(source) => write!(f, "PROCESS_PROTOCOL_ENVIRONMENT_FAILED: {source}"),
            Self::ProtocolEvidence { cause, stderr_tail } => write!(f, "{cause}; PROCESS_STDERR_TAIL: {stderr_tail}"),
            Self::Isolation(reason) => write!(f, "PROCESS_ISOLATION_FAILED: {reason}"),
            Self::LaunchCleanup { cause, detail } => write!(f, "{cause}; PROCESS_LAUNCH_CLEANUP_UNCONFIRMED: {detail}"),
        }
    }
}

impl std::error::Error for ProcessCustodyError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub creation_time_100ns: u64,
    pub image_path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct ProcessLaunch {
    pub application: PathBuf,
    pub arguments: Vec<String>,
    pub current_directory: Option<PathBuf>,
    pub hide_window: bool,
    pub protocol_stdio: bool,
    /// Only native seat composition can opt into repeated CLI stdio. Service
    /// requests continue to use the original one-shot protocol.
    pub(crate) persistent_protocol_stdio: bool,
    /// A complete host-constructed environment. None retains the legacy R2
    /// behavior; v37 callers must supply this before admission.
    pub environment: Option<Vec<(String, String)>>,
    /// Set only by the trusted native seat composition; the old service pipe
    /// never supplies an AppContainer name or launches a v37 process.
    pub(crate) app_container_profile: Option<String>,
    /// Outbound network is a separate explicit AppContainer capability.
    pub(crate) app_container_internet_client: bool,
    /// Sealed fixed-byte compatibility custody; never populated from IPC.
    pub(crate) path_compat: Option<Arc<CompatModule>>,
}

impl ProcessLaunch {
    pub fn new(application: impl Into<PathBuf>) -> Self {
        Self {
            application: application.into(),
            arguments: Vec::new(),
            current_directory: None,
            hide_window: true,
            protocol_stdio: false,
            persistent_protocol_stdio: false,
            environment: None,
            app_container_profile: None,
            app_container_internet_client: false,
            path_compat: None,
        }
    }
}

/// Only the installer-controlled sibling resources can back the R2-02 test
/// process. R2-04 must bundle this exact script and a Node runtime; there is
/// no PATH, caller path, cwd, or source-worktree fallback.
pub fn controlled_fixture_request(
    profile_id: &str,
    domain_id: &str,
    generation: &str,
) -> Result<PrepareRequest, ProcessCustodyError> {
    let native_host = std::env::current_exe().map_err(ProcessCustodyError::BinaryDigest)?;
    let resource_dir = native_host.parent().ok_or(ProcessCustodyError::InvalidLaunch("native host has no resource directory"))?;
    let node = resource_dir.join("gogoke-service").join("runtime").join("node.exe");
    let script = resource_dir.join("gogoke-service").join("fixtures").join("controlled-pi.mjs");
    if !node.is_file() || !script.is_file() {
        return Err(ProcessCustodyError::InvalidLaunch("controlled product resources missing"));
    }
    if file_sha256(&script)? != CONTROLLED_FIXTURE_SHA256 {
        return Err(ProcessCustodyError::BindingMismatch("controlledFixtureDigest"));
    }
    let node_digest = file_sha256(&node)?;
    if node_digest != env!("GOGOKE_CONTROLLED_NODE_SHA256") {
        return Err(ProcessCustodyError::BindingMismatch("controlledNodeDigest"));
    }
    let mut launch = ProcessLaunch::new(node.clone());
    launch.arguments = vec![script.to_string_lossy().into_owned()];
    launch.current_directory = script.parent().map(PathBuf::from);
    launch.protocol_stdio = true;
    Ok(PrepareRequest {
        binding: NativeBinding {
            binary_digest_sha256: node_digest,
            profile_id: profile_id.to_owned(),
            domain_id: domain_id.to_owned(),
            generation: generation.to_owned(),
        },
        launch,
    })
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ProcessTicket(String);

impl ProcessTicket {
    pub fn opaque(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeBinding {
    pub binary_digest_sha256: String,
    pub profile_id: String,
    pub domain_id: String,
    pub generation: String,
}

#[derive(Clone, Debug)]
pub struct PrepareRequest {
    pub launch: ProcessLaunch,
    pub binding: NativeBinding,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedCustody {
    pub ticket: ProcessTicket,
    pub custodian_nonce: String,
    pub binding: NativeBinding,
    pub identity: ProcessIdentity,
}

/// Output bytes read from the exact process pipe owned under this custody.
/// This binds output provenance only. A seat's host-operation request needs
/// its own native-origin channel; stdout alone cannot authorize that request.
pub(crate) struct OriginBoundFrame {
    custody: PreparedCustody,
    bytes: Vec<u8>,
}

impl OriginBoundFrame {
    pub(crate) fn custody(&self) -> &PreparedCustody { &self.custody }
    pub(crate) fn bytes(&self) -> &[u8] { &self.bytes }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StopBudgets {
    pub grace_ms: u32,
    pub terminate_ms: u32,
    pub observe_ms: u32,
    pub host_deadline_ms: u32,
}

impl StopBudgets {
    pub const fn production() -> Self {
        Self {
            grace_ms: STOP_GRACE_MS,
            terminate_ms: STOP_TERMINATE_MS,
            observe_ms: STOP_OBSERVE_MS,
            host_deadline_ms: HOST_STOP_DEADLINE_MS,
        }
    }

    pub const fn phases_fit_deadline(self) -> bool {
        self.host_deadline_ms > 0
            && self
                .grace_ms
                .saturating_add(self.terminate_ms)
                .saturating_add(self.observe_ms)
                <= self.host_deadline_ms
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StopDisposition {
    Stopped,
    ResidualCustody,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StopProof {
    pub identity: ProcessIdentity,
    pub parent_exited: bool,
    pub active_job_processes: Option<u32>,
    pub identity_status: &'static str,
    pub process_handle_present: bool,
    pub job_handle_present: bool,
    pub kill_attempted: bool,
    pub kill_succeeded: bool,
    pub writer_fence_verified: bool,
    pub durable_receipt_saved: bool,
    pub disposition: StopDisposition,
    pub exit_code: Option<u32>,
    pub deadline_exceeded: bool,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeStopProof {
    pub ticket: ProcessTicket,
    pub custodian_nonce: String,
    pub binding: NativeBinding,
    pub identity: ProcessIdentity,
    pub parent_exited: bool,
    pub active_job_processes: Option<u32>,
    pub identity_status: String,
    pub process_handle_present: bool,
    pub job_handle_present: bool,
    pub kill_attempted: bool,
    pub kill_succeeded: bool,
    pub writer_fence_verified: bool,
    pub exit_code: Option<u32>,
    pub deadline_exceeded: bool,
    pub errors: Vec<String>,
}

impl NativeStopProof {
    pub fn proof_hash(&self) -> String {
        let mut bytes = Vec::new();
        append_field(&mut bytes, self.ticket.opaque());
        append_field(&mut bytes, &self.custodian_nonce);
        append_field(&mut bytes, &self.binding.binary_digest_sha256);
        append_field(&mut bytes, &self.binding.profile_id);
        append_field(&mut bytes, &self.binding.domain_id);
        append_field(&mut bytes, &self.binding.generation);
        append_field(&mut bytes, &self.identity.pid.to_string());
        append_field(&mut bytes, &self.identity.creation_time_100ns.to_string());
        append_field(&mut bytes, &self.identity.image_path.to_string_lossy());
        append_field(&mut bytes, &self.parent_exited.to_string());
        append_field(
            &mut bytes,
            &self
                .active_job_processes
                .map_or_else(|| "null".to_owned(), |value| value.to_string()),
        );
        append_field(&mut bytes, &self.identity_status);
        append_field(&mut bytes, &self.process_handle_present.to_string());
        append_field(&mut bytes, &self.job_handle_present.to_string());
        append_field(&mut bytes, &self.kill_attempted.to_string());
        append_field(&mut bytes, &self.kill_succeeded.to_string());
        append_field(&mut bytes, &self.writer_fence_verified.to_string());
        append_field(
            &mut bytes,
            &self
                .exit_code
                .map_or_else(|| "null".to_owned(), |value| value.to_string()),
        );
        append_field(&mut bytes, &self.deadline_exceeded.to_string());
        for error in &self.errors {
            append_field(&mut bytes, error);
        }
        format!("sha256:{}", hex_bytes(&sha256(&bytes)))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurableStopConfirmation {
    pub ticket: ProcessTicket,
    pub custodian_nonce: String,
    pub identity: ProcessIdentity,
    pub proof_hash: String,
    pub durable_revision: u64,
}

struct OwnedHandle(Handle);

impl OwnedHandle {
    fn new(raw: Handle) -> Option<Self> {
        (!raw.is_null()).then_some(Self(raw))
    }

    fn raw(&self) -> Handle {
        self.0
    }

    fn clear_inherit(&self) -> io::Result<()> {
        if unsafe { SetHandleInformation(self.0, HANDLE_FLAG_INHERIT, 0) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn is_inheritable(&self) -> io::Result<bool> {
        let mut flags = 0;
        if unsafe { GetHandleInformation(self.0, &mut flags) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(flags & HANDLE_FLAG_INHERIT != 0)
        }
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

unsafe impl Send for OwnedHandle {}

struct ProtocolPipes {
    stdin_write: Option<OwnedHandle>,
    stdout_read: OwnedHandle,
    stderr: StderrCapture,
}

struct ChildProtocolHandles {
    stdin_read: OwnedHandle,
    stdout_write: OwnedHandle,
    stderr_write: OwnedHandle,
    parent: ProtocolPipes,
}

/// Drain the exact child's stderr independently of stdout. A bounded retained
/// tail must not cause a child blocked on a full stderr pipe to stop serving.
/// The reader owns its handle; closing the retained Job closes all writers.
struct StderrCapture {
    tail: Arc<Mutex<(Vec<u8>, Option<String>)>>,
    reader: Mutex<Option<thread::JoinHandle<()>>>,
}

impl StderrCapture {
    fn start(read: OwnedHandle) -> Result<Self, ProcessCustodyError> {
        let tail = Arc::new(Mutex::new((Vec::new(), None)));
        let target = Arc::clone(&tail);
        let reader = thread::Builder::new().name("gogoke-child-stderr".into()).spawn(move || {
            let mut bytes = [0u8; 4096];
            loop {
                let mut count = 0;
                let ok = unsafe { ReadFile(read.raw(), bytes.as_mut_ptr().cast(), bytes.len() as u32,
                    &mut count, ptr::null_mut()) };
                if ok == 0 {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() != Some(109) {
                        match target.lock() {
                            Ok(mut state) => state.1 = Some(error.to_string()),
                            Err(poisoned) => poisoned.into_inner().1 = Some(error.to_string()),
                        }
                    }
                    break;
                }
                if count == 0 { break; }
                let mut state = match target.lock() {
                    Ok(state) => state,
                    Err(poisoned) => poisoned.into_inner(),
                };
                state.0.extend_from_slice(&bytes[..count as usize]);
                let excess = state.0.len().saturating_sub(4096);
                if excess > 0 { state.0.drain(..excess); }
            }
        }).map_err(ProcessCustodyError::ProtocolPipe)?;
        Ok(Self { tail, reader: Mutex::new(Some(reader)) })
    }

    fn snapshot(&self, all_writers_stopped: bool) -> String {
        if all_writers_stopped {
            match self.reader.lock() {
                Ok(mut reader) => if let Some(reader) = reader.take() {
                    if let Err(error) = reader.join() {
                        return format!("stderr reader panicked: {error:?}");
                    }
                },
                Err(error) => return format!("stderr reader state: {error}"),
            }
        }
        match self.tail.lock() {
            Ok(state) => match &state.1 {
                Some(error) => format!("{}; stderr read failed: {error}", String::from_utf8_lossy(&state.0)),
                None => String::from_utf8_lossy(&state.0).into_owned(),
            },
            Err(error) => format!("stderr tail state: {error}"),
        }
    }
}

impl ChildProtocolHandles {
    fn open() -> Result<Self, ProcessCustodyError> {
        let attributes = SecurityAttributes {
            length: size_of::<SecurityAttributes>() as u32,
            descriptor: ptr::null_mut(),
            inherit: 1,
        };
        let pipe = || -> Result<(OwnedHandle, OwnedHandle), ProcessCustodyError> {
            let mut read = ptr::null_mut();
            let mut write = ptr::null_mut();
            if unsafe { CreatePipe(&mut read, &mut write, &attributes, 0) } == 0 {
                return Err(ProcessCustodyError::ProtocolPipe(io::Error::last_os_error()));
            }
            Ok((OwnedHandle::new(read).expect("pipe read"),
                OwnedHandle::new(write).expect("pipe write")))
        };
        let (stdin_read, stdin_write) = pipe()?;
        let (stdout_read, stdout_write) = pipe()?;
        let (stderr_read, stderr_write) = pipe()?;
        stdin_write.clear_inherit().map_err(ProcessCustodyError::HandlePolicy)?;
        stdout_read.clear_inherit().map_err(ProcessCustodyError::HandlePolicy)?;
        stderr_read.clear_inherit().map_err(ProcessCustodyError::HandlePolicy)?;
        let stderr = StderrCapture::start(stderr_read)?;
        Ok(Self {
            stdin_read,
            stdout_write,
            stderr_write,
            parent: ProtocolPipes { stdin_write: Some(stdin_write), stdout_read, stderr },
        })
    }
}

struct AttributeList {
    storage: Vec<usize>,
}

impl AttributeList {
    fn for_launch(job: &mut [Handle], handles: Option<&mut [Handle]>,
        isolation: Option<(&mut SecurityCapabilities, &mut u32)>) -> Result<Self, ProcessCustodyError> {
        let mut size = 0usize;
        let count = 1u32 + (if handles.is_some() { 1 } else { 0 })
            + (if isolation.is_some() { 2 } else { 0 });
        unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), count, 0, &mut size); }
        if size == 0 {
            return Err(ProcessCustodyError::ProtocolAttribute(io::Error::last_os_error()));
        }
        let mut storage = vec![0usize; size.div_ceil(size_of::<usize>())];
        let list = storage.as_mut_ptr().cast();
        if unsafe { InitializeProcThreadAttributeList(list, count, 0, &mut size) } == 0 {
            return Err(ProcessCustodyError::ProtocolAttribute(io::Error::last_os_error()));
        }
        let result = Self { storage };
        if unsafe {
            UpdateProcThreadAttribute(result.raw(), 0, PROC_THREAD_ATTRIBUTE_JOB_LIST,
                job.as_mut_ptr().cast(), size_of_val(job), ptr::null_mut(), ptr::null_mut())
        } == 0 {
            return Err(ProcessCustodyError::ProtocolAttribute(io::Error::last_os_error()));
        }
        if let Some(handles) = handles {
            if unsafe {
                UpdateProcThreadAttribute(result.raw(), 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                    handles.as_mut_ptr().cast(), size_of_val(handles), ptr::null_mut(), ptr::null_mut())
            } == 0 {
                return Err(ProcessCustodyError::ProtocolAttribute(io::Error::last_os_error()));
            }
        }
        if let Some((capabilities, policy)) = isolation {
            if unsafe { UpdateProcThreadAttribute(result.raw(), 0,
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
                (capabilities as *mut SecurityCapabilities).cast(),
                size_of::<SecurityCapabilities>(), ptr::null_mut(), ptr::null_mut()) } == 0 {
                return Err(ProcessCustodyError::ProtocolAttribute(io::Error::last_os_error()));
            }
            if unsafe { UpdateProcThreadAttribute(result.raw(), 0,
                PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY,
                (policy as *mut u32).cast(), size_of::<u32>(), ptr::null_mut(), ptr::null_mut()) } == 0 {
                return Err(ProcessCustodyError::ProtocolAttribute(io::Error::last_os_error()));
            }
        }
        Ok(result)
    }

    fn raw(&self) -> *mut c_void { self.storage.as_ptr().cast_mut().cast() }
}

impl Drop for AttributeList {
    fn drop(&mut self) { unsafe { DeleteProcThreadAttributeList(self.raw()); } }
}

struct PostCreateFailure {
    cause: ProcessCustodyError,
    process: OwnedHandle,
    initial_thread: OwnedHandle,
}

enum SuspendedCreateError {
    Before(ProcessCustodyError),
    After(PostCreateFailure),
}

impl From<ProcessCustodyError> for SuspendedCreateError {
    fn from(error: ProcessCustodyError) -> Self { Self::Before(error) }
}

// An unconfirmed cleanup stays attached to the exact handles and Job while
// the custodian lives; dropping the custodian closes its kill-on-close Job.
struct LaunchFailureCustody {
    _process: OwnedHandle,
    _initial_thread: OwnedHandle,
    _job: OwnedHandle,
    _path_compat: Option<Arc<CompatModule>>,
}

fn reject_suspended_child_with<T, W>(
    cause: ProcessCustodyError,
    process: OwnedHandle,
    initial_thread: OwnedHandle,
    job: OwnedHandle,
    retained: &mut Vec<LaunchFailureCustody>,
    terminate: T,
    wait: W,
) -> ProcessCustodyError
where
    T: FnOnce(Handle) -> io::Result<()>,
    W: FnOnce(Handle) -> io::Result<u32>,
{
    let termination = terminate(process.raw());
    let observation = wait(process.raw());
    if matches!(observation.as_ref(), Ok(result) if *result == WAIT_OBJECT_0) {
        return cause;
    }
    let detail = format!(
        "exact process/job handles retained; terminate={}; wait={}",
        termination.map_or_else(|error| error.to_string(), |_| "ok".to_owned()),
        observation.map_or_else(|error| error.to_string(), |result| format!("0x{result:08x}")),
    );
    retained.push(LaunchFailureCustody {
        _process: process,
        _initial_thread: initial_thread,
        _job: job,
        _path_compat: None,
    });
    ProcessCustodyError::LaunchCleanup { cause: Box::new(cause), detail }
}

fn reject_suspended_child(
    cause: ProcessCustodyError,
    process: OwnedHandle,
    initial_thread: OwnedHandle,
    job: OwnedHandle,
    retained: &mut Vec<LaunchFailureCustody>,
) -> ProcessCustodyError {
    reject_suspended_child_with(cause, process, initial_thread, job, retained,
        |process| {
            if unsafe { TerminateProcess(process, STOP_REFUSED_EXIT_CODE) } == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        },
        |process| {
            let result = unsafe { WaitForSingleObject(process, 1_000) };
            if result == WAIT_FAILED { Err(io::Error::last_os_error()) } else { Ok(result) }
        })
}

struct PreparedProcess {
    process: OwnedHandle,
    initial_thread: OwnedHandle,
    job: OwnedHandle,
    identity: ProcessIdentity,
    protocol: Option<ProtocolPipes>,
    persistent_protocol_stdio: bool,
    path_compat: Option<Arc<CompatModule>>,
}

impl PreparedProcess {
    fn prepare(launch: &ProcessLaunch, retained: &mut Vec<LaunchFailureCustody>) -> Result<Self, ProcessCustodyError> {
        validate_launch(launch)?;
        let job = create_kill_on_close_job()?;
        let (process, initial_thread, pid, protocol) = match create_suspended(launch, job.raw()) {
            Ok(created) => created,
            Err(SuspendedCreateError::Before(error)) => return Err(error),
            Err(SuspendedCreateError::After(failure)) => {
                return Err(reject_suspended_child(failure.cause, failure.process,
                    failure.initial_thread, job, retained));
            }
        };
        if unsafe { AssignProcessToJobObject(job.raw(), process.raw()) } == 0 {
            let source = io::Error::last_os_error();
            return Err(reject_suspended_child(ProcessCustodyError::AssignJob(source),
                process, initial_thread, job, retained));
        }
        process
            .clear_inherit()
            .and_then(|_| initial_thread.clear_inherit())
            .and_then(|_| job.clear_inherit())
            .map_err(ProcessCustodyError::HandlePolicy)?;
        let identity =
            capture_identity(process.raw(), pid).map_err(ProcessCustodyError::CaptureIdentity)?;
        Ok(Self {
            process,
            initial_thread,
            job,
            identity,
            protocol,
            persistent_protocol_stdio: launch.persistent_protocol_stdio,
            path_compat: launch.path_compat.clone(),
        })
    }

    fn reject(self, cause: ProcessCustodyError,
        retained: &mut Vec<LaunchFailureCustody>) -> ProcessCustodyError {
        let Self { process, initial_thread, job, path_compat, .. } = self;
        let before = retained.len();
        let error = reject_suspended_child(cause, process, initial_thread, job, retained);
        if retained.len() > before {
            retained.last_mut().expect("retained exact failed child")._path_compat = path_compat;
        }
        error
    }

    fn handles_are_non_inheritable(&self) -> Result<bool, ProcessCustodyError> {
        Ok(!self
            .process
            .is_inheritable()
            .and_then(|process| {
                self.initial_thread
                    .is_inheritable()
                    .map(|thread| process || thread)
            })
            .and_then(|either| self.job.is_inheritable().map(|job| either || job))
            .map_err(ProcessCustodyError::HandlePolicy)?)
    }

    fn activate(self) -> Result<ManagedProcess, ProcessCustodyError> {
        if unsafe { ResumeThread(self.initial_thread.raw()) } == RESUME_FAILED {
            return Err(ProcessCustodyError::Resume(io::Error::last_os_error()));
        }
        let Self {
            process,
            initial_thread,
            job,
            identity,
            protocol,
            persistent_protocol_stdio,
            path_compat,
        } = self;
        drop(initial_thread);
        Ok(ManagedProcess {
            process,
            job,
            identity,
            protocol,
            persistent_protocol_stdio,
            _path_compat: path_compat,
            persistent_writer: Mutex::new(false),
            persistent_reader: Mutex::new(PersistentReadState::default()),
            stop_attempted: AtomicBool::new(false),
            protocol_write_attempted: AtomicBool::new(false),
        })
    }
}

/// Creates the child suspended, assigns it to a kill-on-close Job, captures its
/// exact identity, durably records custody, and only then resumes its first
/// thread. Any error drops the Job while the child is still suspended.
#[cfg(test)]
fn prepare_and_activate<F>(
    launch: &ProcessLaunch,
    persist_custody: F,
) -> Result<ManagedProcess, ProcessCustodyError>
where
    F: FnOnce(&ProcessIdentity) -> Result<(), String>,
{
    let mut retained = Vec::new();
    let prepared = PreparedProcess::prepare(launch, &mut retained)?;
    if !prepared.handles_are_non_inheritable()? {
        return Err(ProcessCustodyError::HandlePolicy(io::Error::new(
            io::ErrorKind::Other,
            "process, thread, or job handle remained inheritable",
        )));
    }
    persist_custody(&prepared.identity).map_err(ProcessCustodyError::DurableCustody)?;
    prepared.activate()
}

/// Owns prepared and active Jobs across the native-host prepare/activate IPC
/// boundary. Dropping the custodian closes every Job; a prepared process has
/// never been resumed, and an active process is killed by Job close.
pub struct ProcessCustodian {
    nonce: String,
    prepared: HashMap<ProcessTicket, (PreparedCustody, PreparedProcess)>,
    active: HashMap<ProcessTicket, (PreparedCustody, ManagedProcess)>,
    pending_stop: HashMap<ProcessTicket, NativeStopProof>,
    tombstones: HashSet<ProcessTicket>,
    failed_launches: Vec<LaunchFailureCustody>,
}

impl ProcessCustodian {
    pub fn new() -> Result<Self, ProcessCustodyError> {
        Ok(Self {
            nonce: format!("pcn1_{}", random_hex_32()?),
            prepared: HashMap::new(),
            active: HashMap::new(),
            pending_stop: HashMap::new(),
            tombstones: HashSet::new(),
            failed_launches: Vec::new(),
        })
    }

    pub fn prepare(
        &mut self,
        request: &PrepareRequest,
    ) -> Result<PreparedCustody, ProcessCustodyError> {
        validate_binding(&request.binding)?;
        let actual_digest = file_sha256(&request.launch.application)?;
        if request.binding.binary_digest_sha256 != actual_digest {
            return Err(ProcessCustodyError::BindingMismatch("binaryDigestSha256"));
        }
        let prepared = PreparedProcess::prepare(&request.launch, &mut self.failed_launches)?;
        let launched_digest = file_sha256(&prepared.identity.image_path)?;
        if request.binding.binary_digest_sha256 != launched_digest {
            return Err(ProcessCustodyError::BindingMismatch(
                "launchedBinaryDigestSha256",
            ));
        }
        if !prepared.handles_are_non_inheritable()? {
            return Err(ProcessCustodyError::HandlePolicy(io::Error::new(
                io::ErrorKind::Other,
                "process, thread, or job handle remained inheritable",
            )));
        }
        if let Some(module) = &prepared.path_compat {
            if actual_digest != format!("sha256:{}", gogoke_lpac_path_compat::OBSERVED_CLI_SHA256) {
                return Err(prepared.reject(ProcessCustodyError::BindingMismatch(
                    "compatibilitySupportedCliSha256"), &mut self.failed_launches));
            }
            // This edits imports on the same verified suspended image. It
            // never runs code: activation remains after durable PREPARED.
            if let Err(source) = unsafe { module.update_suspended(prepared.process.raw()) } {
                return Err(prepared.reject(ProcessCustodyError::Isolation(format!(
                    "fixed CLI path compatibility preparation: {source}")), &mut self.failed_launches));
            }
        }
        let ticket = loop {
            let candidate = ProcessTicket(format!("pct1_{}", random_hex_32()?));
            if !self.prepared.contains_key(&candidate)
                && !self.active.contains_key(&candidate)
                && !self.tombstones.contains(&candidate)
            {
                break candidate;
            }
        };
        let custody = PreparedCustody {
            ticket: ticket.clone(),
            custodian_nonce: self.nonce.clone(),
            binding: request.binding.clone(),
            identity: prepared.identity.clone(),
        };
        self.prepared.insert(ticket, (custody.clone(), prepared));
        Ok(custody)
    }

    /// Activates only the exact identity which the service says it durably
    /// saved. A mismatch leaves the original suspended process in custody.
    pub(crate) fn activate(
        &mut self,
        durable: &PreparedCustody,
    ) -> Result<PreparedCustody, ProcessCustodyError> {
        self.verify_nonce(durable)?;
        if self.tombstones.contains(&durable.ticket) {
            return Err(ProcessCustodyError::DuplicateTicket(
                durable.ticket.opaque().to_owned(),
            ));
        }
        let (recorded, _) = self.prepared.get(&durable.ticket).ok_or_else(|| {
            ProcessCustodyError::TicketNotFound(durable.ticket.opaque().to_owned())
        })?;
        if recorded != durable {
            return Err(ProcessCustodyError::DurableIdentityMismatch(
                durable.ticket.opaque().to_owned(),
            ));
        }
        let (recorded, prepared) = self
            .prepared
            .remove(&durable.ticket)
            .expect("prepared ticket existed immediately before removal");
        let managed = prepared.activate()?;
        self.active
            .insert(durable.ticket.clone(), (recorded.clone(), managed));
        Ok(recorded)
    }

    pub fn abort_prepared(
        &mut self,
        durable: &PreparedCustody,
    ) -> Result<bool, ProcessCustodyError> {
        self.verify_nonce(durable)?;
        let Some((recorded, _)) = self.prepared.get(&durable.ticket) else {
            return if self.tombstones.contains(&durable.ticket) {
                Err(ProcessCustodyError::DuplicateTicket(
                    durable.ticket.opaque().to_owned(),
                ))
            } else {
                Err(ProcessCustodyError::TicketNotFound(
                    durable.ticket.opaque().to_owned(),
                ))
            };
        };
        if recorded != durable {
            return Err(ProcessCustodyError::DurableIdentityMismatch(
                durable.ticket.opaque().to_owned(),
            ));
        }
        let removed = self.prepared.remove(&durable.ticket).is_some();
        self.tombstones.insert(durable.ticket.clone());
        Ok(removed)
    }

    pub fn active(&self, ticket: &ProcessTicket) -> Option<&ManagedProcess> {
        self.active.get(ticket).map(|(_, process)| process)
    }

    /// Send EOF to the exact retained child's stdin before graceful stop.
    /// This changes no custody or stop facts and exposes no wire operation.
    pub(crate) fn close_child_input(&mut self, ticket: &ProcessTicket)
        -> Result<(), ProcessCustodyError> {
        let result = self.active.get_mut(ticket).ok_or_else(||
            ProcessCustodyError::TicketNotFound(ticket.opaque().to_owned()))?
            .1.close_input();
        result.map_err(|error| self.protocol_error_with_stderr(ticket,
            ProcessCustodyError::ProtocolPipe(error)))
    }

    /// Check the exact process object captured by the seat pipe against this
    /// custodian's active Job. A ticket selects the expected Job; it does not
    /// authenticate the peer. A missing or stopped Job is never a match.
    pub fn peer_in_active_job(
        &self,
        ticket: &ProcessTicket,
        peer: &PeerProcessHandle,
    ) -> io::Result<bool> {
        if self.tombstones.contains(ticket) || self.pending_stop.contains_key(ticket) {
            return Ok(false);
        }
        let Some((_, managed)) = self.active.get(ticket) else {
            return Ok(false);
        };
        let mut member = 0;
        if unsafe { IsProcessInJob(peer.raw(), managed.job.raw(), &mut member) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(member != 0)
    }

    /// Read an owned child's output without pairing Node-forwarded bytes to
    /// a caller-selected ticket. This is not host-operation admission.
    pub(crate) fn read_child_frame(&self, ticket: &ProcessTicket, deadline: Duration)
        -> Result<OriginBoundFrame, ProcessCustodyError> {
        let (custody, process) = self.active.get(ticket).ok_or_else(||
            ProcessCustodyError::TicketNotFound(ticket.opaque().to_owned()))?;
        let bytes = process.read_protocol_frame(deadline).map_err(|error|
            self.protocol_error_with_stderr(ticket, ProcessCustodyError::ProtocolPipe(error)))?;
        Ok(OriginBoundFrame { custody: custody.clone(), bytes })
    }

    /// Persistent provider stdout still comes from the exact active process
    /// object held by this custodian. A decoded Node message or a ticket string
    /// alone cannot establish this source for the ledger.
    pub(crate) fn read_persistent_child_frame(&self, ticket: &ProcessTicket, deadline: Duration)
        -> Result<OriginBoundFrame, ProcessCustodyError> {
        let (custody, process) = self.active.get(ticket).ok_or_else(||
            ProcessCustodyError::TicketNotFound(ticket.opaque().to_owned()))?;
        let bytes = process.read_persistent_frame(deadline).map_err(|error|
            self.protocol_error_with_stderr(ticket, ProcessCustodyError::ProtocolPipe(error)))?;
        Ok(OriginBoundFrame { custody: custody.clone(), bytes })
    }

    /// This is runtime evidence for the exact retained process. Do not persist
    /// it to a public artifact: provider errors can contain private account data.
    pub(crate) fn protocol_error_with_stderr(&self, ticket: &ProcessTicket,
        cause: ProcessCustodyError) -> ProcessCustodyError {
        let tail = self.active(ticket).map(ManagedProcess::stderr_tail)
            .unwrap_or_else(|| "no active stderr custody".into());
        ProcessCustodyError::ProtocolEvidence { cause: Box::new(cause), stderr_tail: tail }
    }

    pub fn stop<RequestClose>(
        &mut self,
        ticket: &ProcessTicket,
        budgets: StopBudgets,
        request_close: RequestClose,
    ) -> Result<NativeStopProof, ProcessCustodyError>
    where
        RequestClose: FnOnce() -> Result<(), String> + Send + 'static,
    {
        if let Some(existing) = self.pending_stop.get(ticket) {
            let mut repeated = existing.clone();
            repeated
                .errors
                .push("SECOND_STOP_REQUIRES_DURABLE_RECONCILIATION".to_owned());
            return Ok(repeated);
        }
        if self.tombstones.contains(ticket) {
            return Err(ProcessCustodyError::DuplicateTicket(
                ticket.opaque().to_owned(),
            ));
        }
        let (prepared, managed) = self
            .active
            .get(ticket)
            .ok_or_else(|| ProcessCustodyError::TicketNotFound(ticket.opaque().to_owned()))?;
        let observed = managed.stop(budgets, request_close);
        let pending = NativeStopProof {
            ticket: prepared.ticket.clone(),
            custodian_nonce: prepared.custodian_nonce.clone(),
            binding: prepared.binding.clone(),
            identity: prepared.identity.clone(),
            parent_exited: observed.parent_exited,
            active_job_processes: observed.active_job_processes,
            identity_status: observed.identity_status.to_owned(),
            process_handle_present: observed.process_handle_present,
            job_handle_present: observed.job_handle_present,
            kill_attempted: observed.kill_attempted,
            kill_succeeded: observed.kill_succeeded,
            writer_fence_verified: observed.writer_fence_verified,
            exit_code: observed.exit_code,
            deadline_exceeded: observed.deadline_exceeded,
            errors: observed.errors,
        };
        self.pending_stop.insert(ticket.clone(), pending.clone());
        self.tombstones.insert(ticket.clone());
        Ok(pending)
    }

    /// Confirms the exact proof that this native host observed. The service
    /// supplies only the identity it durably committed, never a caller-forged
    /// proof. Failed, repeated, or unknown stops remain owned.
    pub fn confirm_stop_durable(
        &mut self,
        confirmation: &DurableStopConfirmation,
    ) -> Result<NativeStopProof, ProcessCustodyError> {
        if confirmation.custodian_nonce != self.nonce || confirmation.durable_revision == 0 {
            return Err(ProcessCustodyError::BindingMismatch(
                "custodianNonce/revision",
            ));
        }
        let (prepared, managed) = self.active.get(&confirmation.ticket).ok_or_else(|| {
            ProcessCustodyError::TicketNotFound(confirmation.ticket.opaque().to_owned())
        })?;
        let proof = self.pending_stop.get(&confirmation.ticket).ok_or_else(|| {
            ProcessCustodyError::TicketNotFound(confirmation.ticket.opaque().to_owned())
        })?;
        if prepared.custodian_nonce != confirmation.custodian_nonce
            || managed.identity != confirmation.identity
            || proof.identity != confirmation.identity
            || proof.proof_hash() != confirmation.proof_hash
        {
            return Err(ProcessCustodyError::DurableIdentityMismatch(
                confirmation.ticket.opaque().to_owned(),
            ));
        }
        let releasable = proof.errors.is_empty()
            && proof.parent_exited
            && proof.writer_fence_verified
            && proof.active_job_processes == Some(0);
        if !releasable {
            return Err(ProcessCustodyError::ReleaseWithoutDurableStop(
                confirmation.ticket.opaque().to_owned(),
            ));
        }
        let confirmed = proof.clone();
        self.pending_stop.remove(&confirmation.ticket);
        self.active
            .remove(&confirmation.ticket)
            .map(drop)
            .expect("active ticket existed immediately before durable release");
        Ok(confirmed)
    }

    pub fn is_tombstoned(&self, ticket: &ProcessTicket) -> bool {
        self.tombstones.contains(ticket)
    }

    fn verify_nonce(&self, durable: &PreparedCustody) -> Result<(), ProcessCustodyError> {
        if durable.custodian_nonce != self.nonce {
            Err(ProcessCustodyError::BindingMismatch("custodianNonce"))
        } else {
            Ok(())
        }
    }
}

pub struct ManagedProcess {
    process: OwnedHandle,
    job: OwnedHandle,
    identity: ProcessIdentity,
    protocol: Option<ProtocolPipes>,
    persistent_protocol_stdio: bool,
    _path_compat: Option<Arc<CompatModule>>,
    persistent_writer: Mutex<bool>,
    persistent_reader: Mutex<PersistentReadState>,
    stop_attempted: AtomicBool,
    protocol_write_attempted: AtomicBool,
}

#[derive(Default)]
struct PersistentReadState {
    partial: Vec<u8>,
    pending: Vec<u8>,
    pending_offset: usize,
    terminal: Option<(io::ErrorKind, String, Option<i32>)>,
}

impl PersistentReadState {
    fn eof(&mut self, source: io::Error) -> io::Error {
        let error = if self.partial.is_empty() {
            io::Error::new(io::ErrorKind::UnexpectedEof, format!("child stdout closed: {source}"))
        } else {
            io::Error::new(io::ErrorKind::InvalidData, format!(
                "unterminated persistent frame ({} bytes); stdout closure: {source}", self.partial.len()))
        };
        self.fail(error)
    }

    fn fail(&mut self, error: io::Error) -> io::Error {
        let saved = (error.kind(), error.to_string(), error.raw_os_error());
        self.terminal = Some(saved);
        error
    }

    fn terminal_error(&self) -> Option<io::Error> {
        self.terminal.as_ref().map(|(kind, message, code)| match code {
            Some(code) => io::Error::new(*kind, format!("{message} (Windows error {code})")),
            None => io::Error::new(*kind, message.clone()),
        })
    }
}

impl ManagedProcess {
    fn close_input(&mut self) -> io::Result<()> {
        let protocol = self.protocol.as_mut().ok_or_else(||
            io::Error::new(io::ErrorKind::Unsupported, "protocol stdio was not admitted"))?;
        let mut writer = self.persistent_writer.try_lock().map_err(|error|
            io::Error::new(io::ErrorKind::Other, format!("stdin close writer state: {error}")))?;
        let Some(handle) = protocol.stdin_write.as_ref() else { return Ok(()); };
        if unsafe { CloseHandle(handle.raw()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // CloseHandle succeeded. Remove ownership without closing twice.
        std::mem::forget(protocol.stdin_write.take().expect("owned stdin handle"));
        *writer = true;
        Ok(())
    }

    pub(crate) fn stderr_tail(&self) -> String {
        let Some(protocol) = &self.protocol else { return "stderr was not admitted".into(); };
        match active_job_processes(self.job.raw()) {
            Ok(count) => protocol.stderr.snapshot(count == 0),
            Err(error) => format!("{}; stderr writer custody: {error}", protocol.stderr.snapshot(false)),
        }
    }
    pub fn identity(&self) -> &ProcessIdentity {
        &self.identity
    }

    pub fn handles_are_non_inheritable(&self) -> io::Result<bool> {
        Ok(!self.process.is_inheritable()? && !self.job.is_inheritable()?)
    }

    /// Native-owned, repeated JSONL writes to this exact Job child. A failed
    /// or timed-out write poisons the stream: a partial JSON-RPC message must
    /// never be followed by another request. The writer lock includes the
    /// worker deadline, so concurrent callers cannot interleave bytes.
    pub(crate) fn write_persistent_frame(&self, bytes: &[u8]) -> io::Result<()> {
        if !self.persistent_protocol_stdio {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "persistent stdio was not admitted"));
        }
        let protocol = self.protocol.as_ref().ok_or_else(||
            io::Error::new(io::ErrorKind::Unsupported, "protocol stdio was not admitted"))?;
        if bytes.is_empty() || bytes.len() > PERSISTENT_FRAME_MAX_BYTES
            || bytes.last() != Some(&b'\n') || bytes[..bytes.len() - 1].contains(&b'\n') {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "expected one bounded LF frame"));
        }
        let mut failed = self.persistent_writer.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => io::Error::new(io::ErrorKind::WouldBlock, "persistent writer busy"),
            TryLockError::Poisoned(_) => io::Error::new(io::ErrorKind::Other, "persistent writer state unknown"),
        })?;
        if *failed {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "persistent writer previously failed"));
        }
        let mut duplicate = ptr::null_mut();
        let current = unsafe { GetCurrentProcess() };
        let input = protocol.stdin_write.as_ref().ok_or_else(||
            io::Error::new(io::ErrorKind::BrokenPipe, "child input already closed"))?;
        if unsafe { DuplicateHandle(current, input.raw(), current, &mut duplicate,
            0, 0, DUPLICATE_SAME_ACCESS) } == 0 {
            *failed = true;
            return Err(io::Error::last_os_error());
        }
        let write_handle = OwnedHandle::new(duplicate).expect("duplicated protocol handle");
        let payload = bytes.to_vec();
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let mut offset = 0usize;
            let result = loop {
                if offset == payload.len() { break Ok(()); }
                let mut written = 0u32;
                let length = (payload.len() - offset).min(64 * 1024) as u32;
                if unsafe { WriteFile(write_handle.raw(), payload[offset..].as_ptr().cast(),
                    length, &mut written, ptr::null_mut()) } == 0 {
                    break Err(io::Error::last_os_error());
                }
                if written == 0 { break Err(io::Error::new(io::ErrorKind::WriteZero, "persistent write made no progress")); }
                offset += written as usize;
            };
            let _ = sender.send(result);
        });
        let result = match receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(result) => { let _ = worker.join(); result }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                unsafe { CancelSynchronousIo(worker.as_raw_handle().cast()); }
                let terminate = terminate_job(self.job.raw(), STOP_TIMEOUT_EXIT_CODE);
                if receiver.recv_timeout(Duration::from_secs(2)).is_ok() { let _ = worker.join(); }
                Err(io::Error::new(io::ErrorKind::TimedOut,
                    format!("persistent write deadline; job termination={}",
                        terminate.map_or_else(|error| error.to_string(), |_| "ok".to_owned()))))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = worker.join();
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "persistent writer disconnected"))
            }
        };
        if result.is_err() { *failed = true; }
        result
    }

    /// One reader owns the continuous child stdout stream. A timed-out
    /// partial frame is retained for the next call; EOF, overflow and kernel
    /// failures are terminal and retain their original error for recovery.
    pub(crate) fn read_persistent_frame(&self, deadline: Duration) -> io::Result<Vec<u8>> {
        if !self.persistent_protocol_stdio {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "persistent stdio was not admitted"));
        }
        let protocol = self.protocol.as_ref().ok_or_else(||
            io::Error::new(io::ErrorKind::Unsupported, "protocol stdio was not admitted"))?;
        if deadline.is_zero() || deadline > Duration::from_secs(30) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "persistent read deadline out of bounds"));
        }
        let mut state = self.persistent_reader.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => io::Error::new(io::ErrorKind::WouldBlock, "persistent reader busy"),
            TryLockError::Poisoned(_) => io::Error::new(io::ErrorKind::Other, "persistent reader state unknown"),
        })?;
        if let Some(error) = state.terminal_error() { return Err(error); }
        let started = Instant::now();
        loop {
            if state.pending_offset < state.pending.len() {
                let unread = &state.pending[state.pending_offset..];
                let end = unread.iter().position(|byte| *byte == b'\n')
                    .map_or(unread.len(), |index| index + 1);
                let chunk = unread[..end].to_vec();
                state.partial.extend_from_slice(&chunk);
                state.pending_offset += end;
                let complete = state.partial.last() == Some(&b'\n');
                if state.pending_offset == state.pending.len() {
                    state.pending.clear();
                    state.pending_offset = 0;
                }
                if state.partial.len() > PERSISTENT_FRAME_MAX_BYTES {
                    return Err(state.fail(io::Error::new(io::ErrorKind::InvalidData,
                        "persistent frame exceeds Codex decoder bound")));
                }
                if complete { return Ok(std::mem::take(&mut state.partial)); }
            }
            if started.elapsed() >= deadline {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "persistent frame deadline"));
            }
            let mut available = 0u32;
            if unsafe { PeekNamedPipe(protocol.stdout_read.raw(), ptr::null_mut(), 0,
                ptr::null_mut(), &mut available, ptr::null_mut()) } == 0 {
                let source = io::Error::last_os_error();
                return Err(if matches!(source.raw_os_error(), Some(109 | 233)) {
                    state.eof(source)
                } else { state.fail(source) });
            }
            if available == 0 {
                match self.wait(Duration::ZERO) {
                    Ok(true) => return Err(state.eof(io::Error::new(io::ErrorKind::UnexpectedEof,
                        "process exited before complete persistent frame"))),
                    Ok(false) => {},
                    Err(error) => return Err(state.fail(error)),
                }
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            let mut buffer = [0u8; 8192];
            let mut read = 0u32;
            if unsafe { ReadFile(protocol.stdout_read.raw(), buffer.as_mut_ptr().cast(),
                available.min(buffer.len() as u32), &mut read, ptr::null_mut()) } == 0 {
                let source = io::Error::last_os_error();
                return Err(if matches!(source.raw_os_error(), Some(109 | 233)) {
                    state.eof(source)
                } else { state.fail(source) });
            }
            if read == 0 {
                return Err(state.eof(io::Error::new(io::ErrorKind::UnexpectedEof,
                    "partial persistent frame")));
            }
            state.pending.extend_from_slice(&buffer[..read as usize]);
        }
    }

    /// Byte-bounded synchronous protocol transport. The product caller must
    /// provide write liveness, admission and the beginCommitted boundary;
    /// neither an ACK nor these bytes prove completion.
    pub fn write_protocol(&self, bytes: &[u8]) -> io::Result<()> {
        if self.persistent_protocol_stdio {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "persistent protocol requires native transport"));
        }
        let protocol = self.protocol.as_ref().ok_or_else(||
            io::Error::new(io::ErrorKind::Unsupported, "protocol stdio was not admitted"))?;
        if bytes.is_empty() || bytes.len() > 64 * 1024 || bytes.last() != Some(&b'\n') {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "protocol command must be one bounded JSONL frame"));
        }
        if self.protocol_write_attempted.swap(true, Ordering::AcqRel) {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "protocol send is single attempt"));
        }
        let mut duplicate = ptr::null_mut();
        let current = unsafe { GetCurrentProcess() };
        let input = protocol.stdin_write.as_ref().ok_or_else(||
            io::Error::new(io::ErrorKind::BrokenPipe, "child input already closed"))?;
        if unsafe { DuplicateHandle(current, input.raw(), current, &mut duplicate,
            0, 0, DUPLICATE_SAME_ACCESS) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let write_handle = OwnedHandle::new(duplicate).expect("duplicated protocol handle");
        let payload = bytes.to_vec();
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let mut written = 0u32;
            let result = if unsafe { WriteFile(write_handle.raw(), payload.as_ptr().cast(),
                payload.len() as u32, &mut written, ptr::null_mut()) } == 0 {
                Err(io::Error::last_os_error())
            } else if written as usize != payload.len() {
                Err(io::Error::new(io::ErrorKind::WriteZero, "partial protocol command"))
            } else { Ok(()) };
            let _ = sender.send(result);
        });
        match receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(result) => { let _ = worker.join(); result }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                unsafe { CancelSynchronousIo(worker.as_raw_handle().cast()); }
                let _ = terminate_job(self.job.raw(), STOP_TIMEOUT_EXIT_CODE);
                // The worker owns its duplicate handle and payload until it
                // finishes. Never join without a bounded completion signal.
                if receiver.recv_timeout(Duration::from_secs(2)).is_ok() { let _ = worker.join(); }
                Err(io::Error::new(io::ErrorKind::TimedOut, "protocol write deadline"))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = worker.join();
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "protocol writer disconnected"))
            }
        }
    }

    /// Reads one LF frame with an explicit deadline and byte bound. EOF or
    /// process exit before LF is an error, never a task result.
    pub fn read_protocol_frame(&self, deadline: Duration) -> io::Result<Vec<u8>> {
        if self.persistent_protocol_stdio {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "persistent protocol requires native transport"));
        }
        let protocol = self.protocol.as_ref().ok_or_else(||
            io::Error::new(io::ErrorKind::Unsupported, "protocol stdio was not admitted"))?;
        if deadline.is_zero() || deadline > Duration::from_secs(30) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "protocol deadline out of bounds"));
        }
        let started = Instant::now();
        let mut frame = Vec::new();
        loop {
            if started.elapsed() >= deadline {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "protocol frame deadline"));
            }
            let mut available = 0u32;
            if unsafe { PeekNamedPipe(protocol.stdout_read.raw(), ptr::null_mut(), 0,
                ptr::null_mut(), &mut available, ptr::null_mut()) } == 0 {
                return Err(io::Error::last_os_error());
            }
            if available == 0 {
                if self.wait(Duration::ZERO)? {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "process exited before protocol frame"));
                }
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            let mut byte = 0u8;
            let mut read = 0u32;
            if unsafe { ReadFile(protocol.stdout_read.raw(), (&mut byte as *mut u8).cast(),
                1, &mut read, ptr::null_mut()) } == 0 {
                return Err(io::Error::last_os_error());
            }
            if read != 1 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "partial protocol frame"));
            }
            frame.push(byte);
            if frame.len() > 64 * 1024 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "protocol frame too large"));
            }
            if byte == b'\n' { return Ok(frame); }
        }
    }

    pub fn wait(&self, timeout: Duration) -> io::Result<bool> {
        wait_handle(self.process.raw(), duration_ms(timeout))
    }

    pub fn exit_code(&self) -> io::Result<Option<u32>> {
        process_exit_code(self.process.raw())
    }

    pub fn active_job_processes(&self) -> io::Result<u32> {
        active_job_processes(self.job.raw())
    }

    /// Stops the complete Job. The close hook runs off-thread and cannot hold
    /// the host past its deadline or authorize release after returning late.
    pub fn stop<RequestClose>(&self, budgets: StopBudgets, request_close: RequestClose) -> StopProof
    where
        RequestClose: FnOnce() -> Result<(), String> + Send + 'static,
    {
        let started = Instant::now();
        let mut proof = StopProof {
            identity: self.identity.clone(),
            parent_exited: false,
            active_job_processes: None,
            identity_status: "exact",
            process_handle_present: true,
            job_handle_present: true,
            kill_attempted: false,
            kill_succeeded: false,
            writer_fence_verified: false,
            durable_receipt_saved: false,
            disposition: StopDisposition::ResidualCustody,
            exit_code: None,
            deadline_exceeded: false,
            errors: Vec::new(),
        };
        if self.stop_attempted.swap(true, Ordering::AcqRel) {
            proof.active_job_processes = self.active_job_processes().ok();
            proof.parent_exited = wait_handle(self.process.raw(), 0).unwrap_or(false);
            proof
                .errors
                .push("SECOND_STOP_REQUIRES_DURABLE_RECONCILIATION".to_owned());
            return proof;
        }
        if !budgets.phases_fit_deadline() {
            proof
                .errors
                .push("STOP_BUDGETS_EXCEED_HOST_DEADLINE".to_owned());
            return proof;
        }
        let (close_tx, close_rx) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let _ = close_tx.send(request_close());
        });
        let close_budget = remaining_phase_ms(budgets.grace_ms, budgets.host_deadline_ms, started);
        match close_rx.recv_timeout(Duration::from_millis(u64::from(close_budget))) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => proof.errors.push(format!("CLOSE_BINDING_FAILED: {error}")),
            Err(mpsc::RecvTimeoutError::Timeout) => proof
                .errors
                .push("CLOSE_BINDING_DEADLINE_EXCEEDED".to_owned()),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                proof.errors.push("CLOSE_BINDING_DISCONNECTED".to_owned())
            }
        }
        proof.parent_exited = wait_bounded(
            self.process.raw(),
            budgets.grace_ms,
            budgets.host_deadline_ms,
            started,
        )
        .unwrap_or_else(|error| {
            proof.errors.push(format!("GRACE_OBSERVE_FAILED: {error}"));
            false
        });

        let active_before = self.active_job_processes();
        match active_before {
            Ok(0) => {}
            Ok(_) => {
                proof.kill_attempted = true;
                if let Err(error) = terminate_job(self.job.raw(), STOP_TIMEOUT_EXIT_CODE) {
                    proof.errors.push(format!("TERMINATE_JOB_FAILED: {error}"));
                    proof.active_job_processes = self.active_job_processes().ok();
                    return proof;
                }
                proof.kill_succeeded = true;
            }
            Err(error) => {
                proof.errors.push(format!("JOB_IDENTITY_UNKNOWN: {error}"));
                return proof;
            }
        }

        let _ = wait_bounded(
            self.process.raw(),
            budgets.terminate_ms,
            budgets.host_deadline_ms,
            started,
        );
        let observe_deadline = Instant::now()
            + Duration::from_millis(u64::from(remaining_phase_ms(
                budgets.observe_ms,
                budgets.host_deadline_ms,
                started,
            )));
        loop {
            match self.active_job_processes() {
                Ok(active) => {
                    proof.active_job_processes = Some(active);
                    if active == 0 {
                        proof.writer_fence_verified = true;
                        break;
                    }
                }
                Err(error) => {
                    proof.errors.push(format!("JOB_OBSERVE_FAILED: {error}"));
                    return proof;
                }
            }
            if Instant::now() >= observe_deadline {
                proof.errors.push("JOB_DESCENDANTS_REMAIN".to_owned());
                return proof;
            }
            thread::sleep(Duration::from_millis(10));
        }
        proof.parent_exited = wait_handle(self.process.raw(), 0).unwrap_or(false);
        proof.exit_code = process_exit_code(self.process.raw()).ok().flatten();
        if proof.exit_code == Some(STOP_TIMEOUT_EXIT_CODE) {
            proof
                .errors
                .push("STOP_EXIT_124_REQUIRES_RECONCILIATION".to_owned());
        }
        if proof.exit_code == Some(STOP_REFUSED_EXIT_CODE) {
            proof
                .errors
                .push("STOP_EXIT_125_REQUIRES_RECONCILIATION".to_owned());
        }
        if started.elapsed() > Duration::from_millis(u64::from(budgets.host_deadline_ms)) {
            proof.deadline_exceeded = true;
            proof.errors.push("HOST_STOP_DEADLINE_EXCEEDED".to_owned());
        }
        proof.disposition = if proof.errors.is_empty() {
            StopDisposition::Stopped
        } else {
            StopDisposition::ResidualCustody
        };
        proof
    }
}

pub fn may_target_pid(recorded: &ProcessIdentity, observed: Option<&ProcessIdentity>) -> bool {
    observed.is_some_and(|current| current == recorded)
}

fn validate_launch(launch: &ProcessLaunch) -> Result<(), ProcessCustodyError> {
    if let Some(module) = &launch.path_compat {
        module.validate_launch(launch.app_container_profile.as_deref(),
            launch.environment.as_deref()).map_err(|source|
                ProcessCustodyError::Isolation(format!("fixed CLI compatibility scope: {source}")))?;
    }
    if launch.persistent_protocol_stdio && !launch.protocol_stdio {
        return Err(ProcessCustodyError::InvalidLaunch(
            "persistent stdio requires protocol pipes"));
    }
    if launch.app_container_internet_client && launch.app_container_profile.is_none() {
        return Err(ProcessCustodyError::InvalidLaunch(
            "outbound network capability requires an AppContainer identity"));
    }
    if launch.app_container_profile.is_some() && launch.environment.is_none() {
        return Err(ProcessCustodyError::InvalidLaunch(
            "isolated child requires a complete explicit environment"));
    }
    if !launch.application.is_absolute() {
        return Err(ProcessCustodyError::InvalidLaunch(
            "application path must be absolute",
        ));
    }
    if !launch.application.is_file() {
        return Err(ProcessCustodyError::InvalidLaunch(
            "application must be an existing file",
        ));
    }
    if launch
        .arguments
        .iter()
        .any(|argument| argument.contains('\0'))
    {
        return Err(ProcessCustodyError::InvalidLaunch("argument contains NUL"));
    }
    if let Some(directory) = &launch.current_directory {
        if !directory.is_absolute() || !directory.is_dir() {
            return Err(ProcessCustodyError::InvalidLaunch(
                "current directory must be an existing absolute directory",
            ));
        }
    }
    if let Some(environment) = &launch.environment {
        let mut names = HashSet::new();
        if environment.is_empty() {
            return Err(ProcessCustodyError::InvalidLaunch("empty explicit environment"));
        }
        for (name, value) in environment {
            if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') ||
                value.contains('\0') ||
                !names.insert(name.to_ascii_uppercase()) {
                return Err(ProcessCustodyError::InvalidLaunch("invalid explicit environment"));
            }
        }
        if encode_environment(environment).len() > 32_767 {
            return Err(ProcessCustodyError::InvalidLaunch("explicit environment too large"));
        }
    }
    Ok(())
}

fn encode_environment(entries: &[(String, String)]) -> Vec<u16> {
    let mut sorted = entries.to_vec();
    sorted.sort_by_key(|(name, _)| name.to_ascii_uppercase());
    let mut output = Vec::new();
    for (name, value) in sorted {
        output.extend(format!("{name}={value}\0").encode_utf16());
    }
    output.push(0);
    output
}

fn validate_binding(binding: &NativeBinding) -> Result<(), ProcessCustodyError> {
    if !binding.binary_digest_sha256.starts_with("sha256:")
        || binding.binary_digest_sha256.len() != 71
        || !binding.binary_digest_sha256[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ProcessCustodyError::BindingMismatch("binaryDigestSha256"));
    }
    for (field, value) in [
        ("profileId", binding.profile_id.as_str()),
        ("domainId", binding.domain_id.as_str()),
    ] {
        if value.is_empty() || value.len() > 256 || value.contains('\0') {
            return Err(ProcessCustodyError::BindingMismatch(field));
        }
    }
    if binding.generation.is_empty()
        || binding.generation.len() > 256
        || binding.generation.starts_with('0')
        || !binding.generation.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(ProcessCustodyError::BindingMismatch("generation"));
    }
    Ok(())
}

fn create_kill_on_close_job() -> Result<OwnedHandle, ProcessCustodyError> {
    let raw = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    let job = OwnedHandle::new(raw)
        .ok_or_else(|| ProcessCustodyError::CreateJob(io::Error::last_os_error()))?;
    let mut limits = JobObjectExtendedLimitInformation::default();
    limits.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if unsafe {
        SetInformationJobObject(
            job.raw(),
            JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
            (&limits as *const JobObjectExtendedLimitInformation).cast(),
            size_of::<JobObjectExtendedLimitInformation>() as u32,
        )
    } == 0
    {
        return Err(ProcessCustodyError::ConfigureJob(io::Error::last_os_error()));
    }
    job.clear_inherit()
        .map_err(ProcessCustodyError::HandlePolicy)?;
    Ok(job)
}

fn create_suspended(
    launch: &ProcessLaunch,
    job: Handle,
) -> Result<(OwnedHandle, OwnedHandle, u32, Option<ProtocolPipes>), SuspendedCreateError> {
    if launch.protocol_stdio {
        return create_suspended_protocol(launch, job);
    }
    let mut jobs = [job];
    let profile = launch.app_container_profile.as_ref().map(|name|
        AppContainerProfile::ensure(name, launch.app_container_internet_client))
        .transpose().map_err(|error| ProcessCustodyError::Isolation(error.to_string()))?;
    let mut capabilities = profile.as_ref().map(AppContainerProfile::security_capabilities);
    let mut package_policy = ALL_APPLICATION_PACKAGES_OPT_OUT;
    let attributes = AttributeList::for_launch(&mut jobs, None,
        capabilities.as_mut().map(|value| (value, &mut package_policy)))?;
    let application = wide_null(launch.application.as_os_str());
    let mut command_line = wide_null(OsStr::new(&build_command_line(
        launch.application.as_os_str(),
        &launch.arguments,
    )));
    let current_directory = launch
        .current_directory
        .as_ref()
        .map(|path| wide_null(path.as_os_str()));
    let mut startup: StartupInfoExW = unsafe { zeroed() };
    startup.startup.cb = size_of::<StartupInfoExW>() as u32;
    startup.attributes = attributes.raw();
    let mut info: ProcessInformation = unsafe { zeroed() };
    let environment = launch.environment.as_ref().map(|entries| encode_environment(entries));
    let flags = CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT
        | if environment.is_some() { CREATE_UNICODE_ENVIRONMENT } else { 0 }
        | if launch.hide_window {
            CREATE_NO_WINDOW
        } else {
            0
        };
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            flags,
            environment.as_ref().map_or(ptr::null(), |value| value.as_ptr().cast()),
            current_directory
                .as_ref()
                .map_or(ptr::null(), |value| value.as_ptr()),
            &mut startup.startup,
            &mut info,
        )
    };
    if created == 0 {
        return Err(ProcessCustodyError::CreateProcess(
            io::Error::last_os_error(),
        ).into());
    }
    let process =
        OwnedHandle::new(info.process).expect("CreateProcessW returned null process handle");
    let initial_thread =
        OwnedHandle::new(info.thread).expect("CreateProcessW returned null thread handle");
    if let Some(profile) = &profile {
        if let Err(error) = profile.verify_suspended_process(process.raw()) {
            return Err(SuspendedCreateError::After(PostCreateFailure {
                cause: ProcessCustodyError::Isolation(error.to_string()), process, initial_thread,
            }));
        }
    }
    Ok((process, initial_thread, info.process_id, None))
}

fn create_suspended_protocol(
    launch: &ProcessLaunch,
    job: Handle,
) -> Result<(OwnedHandle, OwnedHandle, u32, Option<ProtocolPipes>), SuspendedCreateError> {
    let pipes = ChildProtocolHandles::open()?;
    let stderr_handle = pipes.stderr_write.raw();
    let mut inherited = [pipes.stdin_read.raw(), pipes.stdout_write.raw(), stderr_handle];
    let mut jobs = [job];
    let profile = launch.app_container_profile.as_ref().map(|name|
        AppContainerProfile::ensure(name, launch.app_container_internet_client))
        .transpose().map_err(|error| ProcessCustodyError::Isolation(error.to_string()))?;
    let mut capabilities = profile.as_ref().map(AppContainerProfile::security_capabilities);
    let mut package_policy = ALL_APPLICATION_PACKAGES_OPT_OUT;
    let attributes = AttributeList::for_launch(&mut jobs, Some(&mut inherited),
        capabilities.as_mut().map(|value| (value, &mut package_policy)))?;
    let application = wide_null(launch.application.as_os_str());
    let mut command_line = wide_null(OsStr::new(&build_command_line(
        launch.application.as_os_str(), &launch.arguments,
    )));
    let current_directory = launch.current_directory.as_ref().map(|path| wide_null(path.as_os_str()));
    let mut startup: StartupInfoExW = unsafe { zeroed() };
    startup.startup.cb = size_of::<StartupInfoExW>() as u32;
    startup.startup.flags = STARTF_USESTDHANDLES;
    startup.startup.std_input = pipes.stdin_read.raw();
    startup.startup.std_output = pipes.stdout_write.raw();
    startup.startup.std_error = stderr_handle;
    startup.attributes = attributes.raw();
    let environment = match &launch.environment {
        Some(entries) => encode_environment(entries),
        None => controlled_environment()?,
    };
    let mut info: ProcessInformation = unsafe { zeroed() };
    let flags = CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT
        | if launch.hide_window { CREATE_NO_WINDOW } else { 0 };
    let created = unsafe {
        CreateProcessW(application.as_ptr(), command_line.as_mut_ptr(), ptr::null(), ptr::null(),
            1, flags, environment.as_ptr().cast(),
            current_directory.as_ref().map_or(ptr::null(), |value| value.as_ptr()),
            &mut startup.startup, &mut info)
    };
    let create_error = if created == 0 { Some(io::Error::last_os_error()) } else { None };
    if let Some(error) = create_error { return Err(ProcessCustodyError::CreateProcess(error).into()); }
    let process = OwnedHandle::new(info.process).expect("CreateProcessW returned null process handle");
    let initial_thread = OwnedHandle::new(info.thread).expect("CreateProcessW returned null thread handle");
    if let Some(profile) = &profile {
        if let Err(error) = profile.verify_suspended_process(process.raw()) {
            return Err(SuspendedCreateError::After(PostCreateFailure {
                cause: ProcessCustodyError::Isolation(error.to_string()), process, initial_thread,
            }));
        }
    }
    let ChildProtocolHandles { stdin_read, stdout_write, stderr_write, parent } = pipes;
    drop(stdin_read);
    drop(stdout_write);
    drop(stderr_write);
    Ok((process, initial_thread, info.process_id, Some(parent)))
}

fn controlled_environment() -> Result<Vec<u16>, ProcessCustodyError> {
    let mut buffer = vec![0u16; 32_768];
    let length = unsafe { GetWindowsDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
    if length == 0 || length as usize >= buffer.len() {
        return Err(ProcessCustodyError::ProtocolEnvironment(io::Error::last_os_error()));
    }
    let directory = String::from_utf16(&buffer[..length as usize])
        .map_err(|error| ProcessCustodyError::ProtocolEnvironment(
            io::Error::new(io::ErrorKind::InvalidData, error)))?;
    // Do not inherit NODE_OPTIONS, PATH, credentials, provider config or
    // attacker-controlled preload/search paths from the service process.
    Ok(format!("SystemRoot={directory}\0WINDIR={directory}\0\0").encode_utf16().collect())
}

fn capture_identity(process: Handle, pid: u32) -> io::Result<ProcessIdentity> {
    let mut creation = FileTime { low: 0, high: 0 };
    let mut exit = creation;
    let mut kernel = creation;
    let mut user = creation;
    if unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let image_path = process_image_path(process)?;
    Ok(ProcessIdentity {
        pid,
        creation_time_100ns: (u64::from(creation.high) << 32) | u64::from(creation.low),
        image_path,
    })
}

fn process_image_path(process: Handle) -> io::Result<PathBuf> {
    let mut capacity = 32_768u32;
    let mut buffer = vec![0u16; capacity as usize];
    if unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut capacity) } == 0 {
        return Err(io::Error::last_os_error());
    }
    buffer.truncate(capacity as usize);
    Ok(PathBuf::from(String::from_utf16_lossy(&buffer)))
}

fn active_job_processes(job: Handle) -> io::Result<u32> {
    let mut accounting = JobObjectBasicAccountingInformation::default();
    if unsafe {
        QueryInformationJobObject(
            job,
            JOB_OBJECT_BASIC_ACCOUNTING_INFORMATION_CLASS,
            (&mut accounting as *mut JobObjectBasicAccountingInformation).cast(),
            size_of::<JobObjectBasicAccountingInformation>() as u32,
            ptr::null_mut(),
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(accounting.active_processes)
    }
}

fn terminate_job(job: Handle, exit_code: u32) -> io::Result<()> {
    if unsafe { TerminateJobObject(job, exit_code) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn wait_handle(handle: Handle, milliseconds: u32) -> io::Result<bool> {
    match unsafe { WaitForSingleObject(handle, milliseconds) } {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        WAIT_FAILED => Err(io::Error::last_os_error()),
        other => Err(io::Error::new(
            io::ErrorKind::Other,
            format!("unexpected wait result {other}"),
        )),
    }
}

fn process_exit_code(process: Handle) -> io::Result<Option<u32>> {
    let mut exit_code = 0;
    if unsafe { GetExitCodeProcess(process, &mut exit_code) } == 0 {
        Err(io::Error::last_os_error())
    } else if exit_code == STILL_ACTIVE {
        Ok(None)
    } else {
        Ok(Some(exit_code))
    }
}

fn wait_bounded(
    handle: Handle,
    phase_ms: u32,
    host_deadline_ms: u32,
    started: Instant,
) -> io::Result<bool> {
    wait_handle(
        handle,
        remaining_phase_ms(phase_ms, host_deadline_ms, started),
    )
}

fn remaining_phase_ms(phase_ms: u32, host_deadline_ms: u32, started: Instant) -> u32 {
    let elapsed = started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
    phase_ms.min(host_deadline_ms.saturating_sub(elapsed))
}

fn duration_ms(duration: Duration) -> u32 {
    if duration == Duration::MAX {
        INFINITE
    } else {
        duration.as_millis().min(u128::from(u32::MAX - 1)) as u32
    }
}

fn wide_null(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

fn build_command_line(application: &OsStr, arguments: &[String]) -> String {
    let mut command = quote_windows_argument(&application.to_string_lossy());
    for argument in arguments {
        command.push(' ');
        command.push_str(&quote_windows_argument(argument));
    }
    command
}

fn quote_windows_argument(argument: &str) -> String {
    if !argument.is_empty()
        && !argument
            .chars()
            .any(|character| character == ' ' || character == '\t' || character == '"')
    {
        return argument.to_owned();
    }
    let mut quoted = String::from("\"");
    let mut backslashes = 0usize;
    for character in argument.chars() {
        if character == '\\' {
            backslashes += 1;
            continue;
        }
        if character == '"' {
            quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
            quoted.push('"');
            backslashes = 0;
            continue;
        }
        quoted.push_str(&"\\".repeat(backslashes));
        backslashes = 0;
        quoted.push(character);
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

fn random_hex_32() -> Result<String, ProcessCustodyError> {
    const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;
    let mut bytes = [0u8; 32];
    let status = unsafe {
        BCryptGenRandom(
            ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status != 0 {
        return Err(ProcessCustodyError::Random(io::Error::from_raw_os_error(
            status,
        )));
    }
    Ok(hex_bytes(&bytes))
}

fn file_sha256(path: &PathBuf) -> Result<String, ProcessCustodyError> {
    let mut file = File::open(path).map_err(ProcessCustodyError::BinaryDigest)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(ProcessCustodyError::BinaryDigest)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("sha256:{}", hex_bytes(&hasher.finish())))
}

fn append_field(output: &mut Vec<u8>, value: &str) {
    output.extend_from_slice(&(value.len() as u64).to_le_bytes());
    output.extend_from_slice(value.as_bytes());
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finish()
}

struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buffered: usize,
    length_bytes: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buffer: [0; 64],
            buffered: 0,
            length_bytes: 0,
        }
    }

    fn update(&mut self, mut input: &[u8]) {
        self.length_bytes = self.length_bytes.wrapping_add(input.len() as u64);
        if self.buffered != 0 {
            let take = (64 - self.buffered).min(input.len());
            self.buffer[self.buffered..self.buffered + take].copy_from_slice(&input[..take]);
            self.buffered += take;
            input = &input[take..];
            if self.buffered == 64 {
                let block = self.buffer;
                self.compress(&block);
                self.buffered = 0;
            }
        }
        while input.len() >= 64 {
            let block: &[u8; 64] = input[..64].try_into().expect("exact SHA-256 block");
            self.compress(block);
            input = &input[64..];
        }
        self.buffer[..input.len()].copy_from_slice(input);
        self.buffered = input.len();
    }

    fn finish(mut self) -> [u8; 32] {
        let bit_length = self.length_bytes.wrapping_mul(8);
        self.buffer[self.buffered] = 0x80;
        self.buffered += 1;
        if self.buffered > 56 {
            self.buffer[self.buffered..].fill(0);
            let block = self.buffer;
            self.compress(&block);
            self.buffer = [0; 64];
        } else {
            self.buffer[self.buffered..56].fill(0);
        }
        self.buffer[56..].copy_from_slice(&bit_length.to_be_bytes());
        let block = self.buffer;
        self.compress(&block);
        let mut output = [0u8; 32];
        for (chunk, value) in output.chunks_exact_mut(4).zip(self.state) {
            chunk.copy_from_slice(&value.to_be_bytes());
        }
        output
    }

    fn compress(&mut self, block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut words = [0u32; 64];
        for (index, chunk) in block.chunks_exact(4).enumerate() {
            words[index] = u32::from_be_bytes(chunk.try_into().expect("four-byte word"));
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for index in 0..64 {
            let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(sum1)
                .wrapping_add(choice)
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
            let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = sum0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (slot, value) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    fn powershell() -> PathBuf {
        PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"))
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe")
    }

    fn system_cmd() -> PathBuf {
        PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"))
            .join("System32")
            .join("cmd.exe")
    }

    fn unique_marker(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "gogoke-process-{name}-{}-{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ))
    }

    fn ps_literal(path: &Path) -> String {
        path.to_string_lossy().replace('\'', "''")
    }

    fn write_executable_with_directory_acl(source: &Path, destination: &Path) {
        let mut input = fs::File::open(source).expect("open executable fixture");
        let mut output = fs::OpenOptions::new().write(true).create_new(true)
            .open(destination).expect("create executable under granted directory");
        io::copy(&mut input, &mut output).expect("write executable fixture");
        output.sync_all().expect("flush executable fixture");
    }

    fn request(launch: ProcessLaunch) -> PrepareRequest {
        let digest = file_sha256(&launch.application).expect("fixture binary digest");
        PrepareRequest {
            launch,
            binding: NativeBinding {
                binary_digest_sha256: digest,
                profile_id: "profile-controlled".to_owned(),
                domain_id: "domain-controlled".to_owned(),
                generation: "1".to_owned(),
            },
        }
    }

    #[test]
    fn both_stdio_modes_are_in_the_exact_job_at_creation_and_allow_explicit_assign() {
        for protocol_stdio in [false, true] {
            let job = create_kill_on_close_job().expect("kill-on-close job");
            let mut launch = ProcessLaunch::new(system_cmd());
            launch.arguments = vec!["/D".into(), "/C".into(), "exit 0".into()];
            launch.protocol_stdio = protocol_stdio;
            let (process, initial_thread, _, protocol) =
                match create_suspended(&launch, job.raw()) {
                    Ok(created) => created,
                    Err(_) => panic!("suspended CreateProcess with JOB_LIST failed: protocol_stdio={protocol_stdio}"),
                };
            let mut member = 0;
            assert_ne!(unsafe { IsProcessInJob(process.raw(), job.raw(), &mut member) }, 0);
            assert_eq!(member, 1, "JOB_LIST must contain child before explicit assign");
            assert_ne!(unsafe { AssignProcessToJobObject(job.raw(), process.raw()) }, 0,
                "explicit same-Job assign required by C07 must succeed");
            // A forced invalid assign exercises the post-create rejection path
            // while the child remains in its exact creation-time Job.
            assert_eq!(unsafe { AssignProcessToJobObject(ptr::null_mut(), process.raw()) }, 0);
            let source = io::Error::last_os_error();
            let mut retained = Vec::new();
            let error = reject_suspended_child(ProcessCustodyError::AssignJob(source),
                process, initial_thread, job, &mut retained);
            assert!(matches!(&error, ProcessCustodyError::AssignJob(_)),
                "real kernel cleanup must confirm child exit: {error}");
            assert!(retained.is_empty());
            drop(protocol);
        }
    }

    #[test]
    fn failed_launch_cleanup_retains_exact_handles_on_timeout_or_failed_wait() {
        for wait_result in [Ok(WAIT_TIMEOUT), Err(io::Error::from_raw_os_error(6))] {
            let process = create_kill_on_close_job().expect("mock process handle");
            let initial_thread = create_kill_on_close_job().expect("mock thread handle");
            let job = create_kill_on_close_job().expect("mock job handle");
            let exact_process = process.raw();
            let exact_job = job.raw();
            let mut retained = Vec::new();
            let error = reject_suspended_child_with(
                ProcessCustodyError::InvalidLaunch("forced assign failure"),
                process, initial_thread, job, &mut retained,
                |handle| { assert_eq!(handle, exact_process); Err(io::Error::from_raw_os_error(5)) },
                |handle| { assert_eq!(handle, exact_process); wait_result },
            );
            assert!(matches!(&error, ProcessCustodyError::LaunchCleanup { .. }));
            assert_eq!(retained.len(), 1);
            assert_eq!(retained[0]._process.raw(), exact_process);
            assert_eq!(retained[0]._job.raw(), exact_job);
            assert!(error.to_string().contains("exact process/job handles retained"));
        }
    }

    #[test]
    fn already_signaled_child_is_confirmed_even_if_terminate_reports_failure() {
        let process = create_kill_on_close_job().expect("mock process handle");
        let initial_thread = create_kill_on_close_job().expect("mock thread handle");
        let job = create_kill_on_close_job().expect("mock job handle");
        let mut retained = Vec::new();
        let error = reject_suspended_child_with(
            ProcessCustodyError::InvalidLaunch("forced assign failure"),
            process, initial_thread, job, &mut retained,
            |_| Err(io::Error::from_raw_os_error(5)),
            |_| Ok(WAIT_OBJECT_0),
        );
        assert!(matches!(error, ProcessCustodyError::InvalidLaunch(_)));
        assert!(retained.is_empty());
    }

    #[test]
    fn production_stop_budget_is_ten_five_five_inside_thirty_seconds() {
        assert_eq!(StopBudgets::production().grace_ms, 10_000);
        assert_eq!(StopBudgets::production().terminate_ms, 5_000);
        assert_eq!(StopBudgets::production().observe_ms, 5_000);
        assert_eq!(StopBudgets::production().host_deadline_ms, 30_000);
        assert!(StopBudgets::production().phases_fit_deadline());
        assert!(!StopBudgets {
            grace_ms: 20_000,
            terminate_ms: 10_000,
            observe_ms: 5_000,
            host_deadline_ms: 30_000,
        }
        .phases_fit_deadline());
    }

    #[test]
    fn native_input_close_delivers_eof_and_preserves_graceful_stop_proof() {
        let mut launch = ProcessLaunch::new(powershell());
        launch.protocol_stdio = true;
        launch.persistent_protocol_stdio = true;
        launch.arguments = vec!["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(),
            "while ($null -ne [Console]::ReadLine()) {}; [Console]::Out.WriteLine('stdin-eof'); exit 0".into()];
        let mut custodian = ProcessCustodian::new().unwrap();
        let prepared = custodian.prepare(&request(launch)).unwrap();
        custodian.activate(&prepared).unwrap();
        custodian.active(&prepared.ticket).unwrap().write_persistent_frame(b"one\n").unwrap();
        custodian.close_child_input(&prepared.ticket).unwrap();
        custodian.close_child_input(&prepared.ticket).unwrap();
        assert_eq!(custodian.active(&prepared.ticket).unwrap()
            .write_persistent_frame(b"two\n").unwrap_err().kind(), io::ErrorKind::BrokenPipe);
        let frame = custodian.read_persistent_child_frame(&prepared.ticket, Duration::from_secs(10)).unwrap();
        assert_eq!(frame.bytes(), b"stdin-eof\r\n");
        let proof = custodian.stop(&prepared.ticket, StopBudgets::production(), || Ok(())).unwrap();
        assert_eq!(proof.exit_code, Some(0));
        assert!(proof.parent_exited && proof.writer_fence_verified);
        assert_eq!(proof.active_job_processes, Some(0));
        assert!(!proof.kill_attempted);
        assert!(proof.errors.is_empty(), "graceful close errors: {:?}", proof.errors);
    }

    #[test]
    fn failed_protocol_retains_direct_stderr_and_does_not_block_large_stderr() {
        let mut launch = ProcessLaunch::new(powershell());
        launch.protocol_stdio = true;
        launch.persistent_protocol_stdio = true;
        launch.arguments = vec!["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(),
            "[Console]::Error.Write(('x' * 20000)); [Console]::Error.WriteLine('DIRECT_ERROR_1234'); exit 7".into()];
        let mut custodian = ProcessCustodian::new().unwrap();
        let prepared = custodian.prepare(&request(launch)).unwrap();
        custodian.activate(&prepared).unwrap();
        let error = custodian.read_persistent_child_frame(&prepared.ticket, Duration::from_secs(10))
            .err().expect("stdout EOF is a protocol failure");
        let proof = custodian.stop(&prepared.ticket, StopBudgets::production(), || Ok(())).unwrap();
        assert!(proof.writer_fence_verified, "must stop the exact Job before final stderr read");
        let error = custodian.protocol_error_with_stderr(&prepared.ticket, error).to_string();
        assert!(error.contains("DIRECT_ERROR_1234"), "original stderr must reach error: {error}");
        let tail = custodian.active(&prepared.ticket).unwrap().stderr_tail();
        assert!(tail.len() <= 4096);
        assert!(tail.ends_with("DIRECT_ERROR_1234\r\n"));
    }

    #[test]
    fn suspended_job_child_receives_only_explicit_protocol_handles() {
        let mut launch = ProcessLaunch::new(powershell());
        launch.protocol_stdio = true;
        launch.arguments = vec![
            "-NoProfile".into(), "-NonInteractive".into(), "-Command".into(),
            "$line=[Console]::ReadLine(); [Console]::Out.WriteLine('echo:' + $line)".into(),
        ];
        let managed = prepare_and_activate(&launch, |_| Ok(())).expect("durably activate piped child");
        let pipes = managed.protocol.as_ref().expect("protocol pipes");
        assert!(!pipes.stdin_write.as_ref().unwrap().is_inheritable().expect("parent stdin handle"));
        assert!(!pipes.stdout_read.is_inheritable().expect("parent stdout handle"));
        assert_eq!(managed.write_protocol(b"no delimiter").unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(managed.read_protocol_frame(Duration::ZERO).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        managed.write_protocol(b"controlled\n").expect("bounded command write");
        assert_eq!(managed.write_protocol(b"second\n").unwrap_err().kind(),
            io::ErrorKind::AlreadyExists, "legacy protocol remains one-shot");
        let output = managed.read_protocol_frame(Duration::from_secs(5)).expect("bounded frame read");
        assert!(String::from_utf8_lossy(&output).contains("echo:controlled"));
        assert!(managed.wait(Duration::from_secs(5)).expect("child exit"));
    }

    #[test]
    fn persistent_child_exchanges_multiple_jsonl_frames_on_one_owned_job() {
        let mut launch = ProcessLaunch::new(powershell());
        launch.protocol_stdio = true;
        launch.persistent_protocol_stdio = true;
        launch.arguments = vec![
            "-NoProfile".into(), "-NonInteractive".into(), "-Command".into(),
            r#"while ($null -ne ($line = [Console]::ReadLine())) { [Console]::Out.WriteLine('{"jsonrpc":"2.0","method":"turn/started"}'); [Console]::Out.WriteLine('{"jsonrpc":"2.0","id":7,"method":"approval"}'); [Console]::Out.WriteLine($line) }"#.into(),
        ];
        let mut custodian = ProcessCustodian::new().expect("native custodian");
        let prepared = custodian.prepare(&request(launch)).expect("prepared persistent ticket");
        custodian.activate(&prepared).expect("activate exact ticket");
        let managed = custodian.active(&prepared.ticket).expect("same active Job");
        assert!(managed.handles_are_non_inheritable().unwrap());
        assert!(managed.active_job_processes().unwrap() >= 1);
        assert_eq!(managed.write_protocol(b"legacy\n").unwrap_err().kind(), io::ErrorKind::Unsupported);
        assert_eq!(managed.read_protocol_frame(Duration::from_secs(1)).unwrap_err().kind(),
            io::ErrorKind::Unsupported);
        for id in [1, 2] {
            let request = format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"thread/start\"}}\n");
            managed.write_persistent_frame(request.as_bytes()).expect("repeated request write");
            let notification = custodian.read_persistent_child_frame(&prepared.ticket,
                Duration::from_secs(5)).unwrap();
            assert_eq!(notification.custody(), &prepared);
            assert_eq!(String::from_utf8_lossy(notification.bytes()).trim_end(),
                "{\"jsonrpc\":\"2.0\",\"method\":\"turn/started\"}");
            let server_request = custodian.read_persistent_child_frame(&prepared.ticket,
                Duration::from_secs(5)).unwrap();
            assert_eq!(server_request.custody(), &prepared);
            assert_eq!(String::from_utf8_lossy(server_request.bytes()).trim_end(),
                "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"approval\"}");
            let response = custodian.read_persistent_child_frame(&prepared.ticket,
                Duration::from_secs(5)).unwrap();
            assert_eq!(response.custody(), &prepared);
            assert_eq!(String::from_utf8_lossy(response.bytes()).trim_end(), request.trim_end());
        }
        assert_eq!(managed.write_persistent_frame(b"two\nframes\n").unwrap_err().kind(),
            io::ErrorKind::InvalidInput);
        assert!(custodian.active(&prepared.ticket).is_some(), "repeated exchange keeps original ticket");
    }

    #[test]
    fn persistent_reader_keeps_eof_unknown_after_exact_child_exit() {
        let mut launch = ProcessLaunch::new(system_cmd());
        launch.protocol_stdio = true;
        launch.persistent_protocol_stdio = true;
        launch.arguments = vec!["/D".into(), "/C".into(), "exit /B 0".into()];
        let managed = prepare_and_activate(&launch, |_| Ok(())).expect("owned exiting child");
        let error = managed.read_persistent_frame(Duration::from_secs(5))
            .expect_err("exit without LF is not a response");
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        assert_eq!(managed.read_persistent_frame(Duration::from_secs(1)).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof, "terminal stream cannot regain certainty");
    }

    #[test]
    fn persistent_reader_distinguishes_unterminated_output_from_empty_eof() {
        let mut launch = ProcessLaunch::new(system_cmd());
        launch.protocol_stdio = true;
        launch.persistent_protocol_stdio = true;
        launch.arguments = vec!["/D".into(), "/C".into(), "<nul set /p =partial".into()];
        let managed = prepare_and_activate(&launch, |_| Ok(())).expect("owned no-LF child");
        let error = managed.read_persistent_frame(Duration::from_secs(5))
            .expect_err("partial output must not become a successful empty result");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("7 bytes"));
        assert_eq!(managed.read_persistent_frame(Duration::from_secs(1)).unwrap_err().kind(),
            io::ErrorKind::InvalidData, "terminal stream preserves partial-frame uncertainty");
    }

    #[test]
    fn explicit_environment_reaches_real_child_without_parent_profile() {
        let mut launch = ProcessLaunch::new(system_cmd());
        launch.protocol_stdio = true;
        launch.arguments = vec!["/D".into(), "/C".into(),
            "echo %GOGOKE_V37_ENV_MARKER%:%USERPROFILE%".into()];
        launch.environment = Some(vec![
            ("SystemRoot".into(), std::env::var("SystemRoot").expect("SystemRoot")),
            ("GOGOKE_V37_ENV_MARKER".into(), "isolated".into()),
        ]);
        let managed = prepare_and_activate(&launch, |_| Ok(())).expect("explicit environment child");
        let frame = managed.read_protocol_frame(Duration::from_secs(5)).expect("environment frame");
        assert_eq!(String::from_utf8_lossy(&frame).trim(), "isolated:%USERPROFILE%");
        assert!(managed.wait(Duration::from_secs(5)).expect("child exit"));

        launch.environment = Some(vec![("Path".into(), "x".into()), ("PATH".into(), "y".into())]);
        assert!(matches!(validate_launch(&launch), Err(ProcessCustodyError::InvalidLaunch(_))));
    }

    #[test]
    fn isolated_launch_refuses_inherited_environment_and_invalid_profile() {
        let mut launch = ProcessLaunch::new(system_cmd());
        launch.app_container_internet_client = true;
        assert!(matches!(validate_launch(&launch), Err(ProcessCustodyError::InvalidLaunch(_))));
        launch.app_container_internet_client = false;
        launch.app_container_profile = Some("invalid/name".into());
        assert!(matches!(validate_launch(&launch), Err(ProcessCustodyError::InvalidLaunch(_))));
        launch.environment = Some(vec![("SystemRoot".into(),
            std::env::var("SystemRoot").expect("SystemRoot"))]);
        assert!(matches!(prepare_and_activate(&launch, |_| Ok(())),
            Err(ProcessCustodyError::Isolation(_))));
    }

    #[test]
    fn app_container_child_writes_only_granted_fresh_directory() {
        use std::os::windows::ffi::OsStrExt;
        use std::time::{SystemTime, UNIX_EPOCH};
        #[link(name = "userenv")]
        extern "system" { fn DeleteAppContainerProfile(name: *const u16) -> i32; }
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let base = std::env::temp_dir().join(format!("gogoke-v37-lpac-{}-{nonce}", std::process::id()));
        let allowed = base.join("allowed");
        let blocked = base.join("blocked");
        let readonly = base.join("readonly");
        std::fs::create_dir(&base).unwrap();
        std::fs::create_dir(&allowed).unwrap();
        std::fs::create_dir(&blocked).unwrap();
        std::fs::create_dir(&readonly).unwrap();
        std::fs::write(blocked.join("keep.txt"), b"blocked file").unwrap();
        std::fs::write(readonly.join("keep.txt"), b"read-only file").unwrap();
        let name = format!("Gogoke37.test{}.{nonce}", std::process::id());
        let profile = AppContainerProfile::ensure(&name, false).expect("test package profile");
        profile.grant_fresh_session_directory(&allowed).expect("package directory ACL");
        let readonly_identity = crate::root::inspect_root(&readonly).unwrap().identity;
        profile.grant_bound_tree(&readonly, &readonly_identity, false).expect("read-only tree ACL");
        drop(profile);
        let executable = allowed.join("cmd.exe");
        write_executable_with_directory_acl(&system_cmd(), &executable);
        let mut launch = ProcessLaunch::new(&executable);
        launch.current_directory = Some(allowed.clone());
        launch.protocol_stdio = true;
        launch.app_container_profile = Some(name.clone());
        launch.environment = Some(vec![
            ("SystemRoot".into(), std::env::var("SystemRoot").unwrap()),
            ("USERPROFILE".into(), allowed.to_string_lossy().into_owned()),
            ("LOCALAPPDATA".into(), allowed.to_string_lossy().into_owned()),
        ]);
        launch.arguments = vec!["/D".into(), "/C".into(),
            concat!("echo transient> transient.txt & ren transient.txt renamed.txt & del renamed.txt & ",
                "del ..\\blocked\\keep.txt & del ..\\readonly\\keep.txt & ",
                "echo permitted> allowed.txt & echo forbidden> ..\\blocked\\forbidden.txt").into()];
        let managed = prepare_and_activate(&launch, |_| Ok(())).expect("real LPAC child");
        assert!(managed.wait(Duration::from_secs(10)).expect("LPAC exit"));
        let direct_evidence = format!("exit={:?}; transient={}; renamed={}; stderr={}",
            managed.exit_code().expect("LPAC exit code"), allowed.join("transient.txt").exists(),
            allowed.join("renamed.txt").exists(), managed.stderr_tail());
        assert!(allowed.join("allowed.txt").is_file(), "LPAC must write its granted directory: {direct_evidence}");
        assert!(!allowed.join("transient.txt").exists() && !allowed.join("renamed.txt").exists(),
            "the actual LPAC child must rename and delete its own writable file: {direct_evidence}");
        assert!(!blocked.join("forbidden.txt").exists(), "LPAC must not write sibling directory");
        assert_eq!(std::fs::read(blocked.join("keep.txt")).unwrap(), b"blocked file");
        assert_eq!(std::fs::read(readonly.join("keep.txt")).unwrap(), b"read-only file");
        drop(managed);
        let wide: Vec<u16> = OsStr::new(&name).encode_wide().chain(Some(0)).collect();
        assert!(unsafe { DeleteAppContainerProfile(wide.as_ptr()) } >= 0);
        std::fs::remove_file(allowed.join("allowed.txt")).unwrap();
        std::fs::remove_file(executable).unwrap();
        std::fs::remove_dir(allowed).unwrap();
        std::fs::remove_file(blocked.join("keep.txt")).unwrap();
        std::fs::remove_dir(blocked).unwrap();
        std::fs::remove_file(readonly.join("keep.txt")).unwrap();
        std::fs::remove_dir(readonly).unwrap();
        std::fs::remove_dir(base).unwrap();
    }

    #[test]
    fn seat_pipe_child_helper() {
        use std::io::{Read, Write};
        let Some(path) = std::env::var_os("GOGOKE_TEST_SEAT_PIPE") else { return; };
        let mut pipe = std::fs::OpenOptions::new().read(true).write(true)
            .open(PathBuf::from(path.as_os_str()))
            .unwrap_or_else(|error| {
                let detail = format!("OpenOptions(seat pipe {}): {error}; raw_os_error={:?}",
                    PathBuf::from(path.as_os_str()).display(), error.raw_os_error());
                std::fs::write("pipe-client-error.txt", &detail).expect("write direct pipe error");
                panic!("{detail}");
            });
        pipe.write_all(&[0x47]).expect("seat transport preface");
        let mut length = [0; 4];
        pipe.read_exact(&mut length).expect("native Job admission ack length");
        assert_eq!(u32::from_le_bytes(length), 2);
        let mut ack = [0; 2];
        pipe.read_exact(&mut ack).expect("native Job admission ack");
        assert_eq!(&ack, b"ok");
    }

    #[test]
    fn seat_pipe_parent_helper() {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetLastError() -> u32;
            fn CreateFileMappingW(file: Handle, attributes: *const c_void,
                protect: u32, maximum_size_high: u32, maximum_size_low: u32,
                name: *const u16) -> Handle;
        }
        #[link(name = "ntdll")]
        extern "system" {
            fn RtlGetLastNtStatus() -> i32;
        }
        let image_section = |file: Handle| {
            // PAGE_READONLY | SEC_IMAGE on the same opened image, before the
            // failing process creation. This observes image-section access;
            // success does not prove a child process can be created.
            let section = unsafe { CreateFileMappingW(file, ptr::null(),
                0x0000_0002 | 0x0100_0000, 0, 0, ptr::null()) };
            if section.is_null() {
                format!("WIN32_{:?}", io::Error::last_os_error().raw_os_error())
            } else {
                unsafe { CloseHandle(section) };
                "OK".to_owned()
            }
        };
        let Some(child) = std::env::var_os("GOGOKE_TEST_SEAT_CHILD") else { return; };
        let path = PathBuf::from(child);
        let read = fs::File::open(&path)
            .map(|_| "OK".to_owned())
            .unwrap_or_else(|error| format!("WIN32_{:?}", error.raw_os_error()));
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // Probe the exact primary image under the LPAC parent's token before
        // the failing CreateProcess. FILE_READ_DATA | FILE_EXECUTE | READ_CONTROL.
        let image = unsafe { CreateFileW(wide.as_ptr(), 0x0002_0021, 7,
            ptr::null(), 3, 0, ptr::null_mut()) };
        let (execute_open, image_section_result) = if image as isize == -1 {
            (format!("WIN32_{:?}", io::Error::last_os_error().raw_os_error()),
                "NOT_RUN".to_owned())
        } else {
            let section = image_section(image);
            unsafe { CloseHandle(image) };
            ("OK".to_owned(), section)
        };
        if let Some(directory) = std::env::var_os("GOGOKE_LPAC_CDB_DIR") {
            let directory = PathBuf::from(directory);
            fs::write(directory.join("parent.pid"), std::process::id().to_string())
                .expect("publish exact LPAC parent PID for cloud diagnostic");
            let start = Instant::now();
            while !directory.join("release").exists() {
                assert!(start.elapsed() < Duration::from_secs(90), "cloud debugger release timed out");
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        let helper_stderr = fs::File::create("first-helper-stderr.txt")
            .expect("capture exact first LPAC helper stderr");
        let launched = std::process::Command::new(&path)
            .args(["--exact", "process::windows::tests::seat_pipe_child_helper", "--nocapture"])
            .stderr(std::process::Stdio::from(helper_stderr))
            .spawn();
        let (outcome, spawn_failed, first_helper_status) = match launched {
            Ok(mut process) => match process.wait() {
                Ok(status) => (format!("spawn=OK; exit={status:?}"), false,
                    if status.success() { "SUCCESS".to_owned() }
                    else { format!("EXIT_{:?}", status.code()) }),
                Err(error) => (format!("spawn=OK; wait=WIN32_{:?}; detail={error}", error.raw_os_error()),
                    false, format!("WAIT_WIN32_{:?}: {error}", error.raw_os_error())),
            },
            Err(error) => (format!("spawn=WIN32_{:?}; detail={error}", error.raw_os_error()),
                true, format!("SPAWN_WIN32_{:?}: {error}", error.raw_os_error())),
        };
        fs::write("first-helper-status.txt", &first_helper_status)
            .expect("persist exact first helper completion");
        let signed_control = std::process::Command::new(system_cmd())
            .args(["/D", "/C", "exit 0"])
            .spawn();
        let control = match signed_control {
            Ok(mut process) => format!("signed_child=OK; exit={:?}", process.wait()),
            Err(error) => format!("signed_child=WIN32_{:?}; detail={error}", error.raw_os_error()),
        };
        // Separate std::process::Command's path/stdio preparation from the
        // Windows process-creation API on the same LPAC token. This control
        // uses an absolute signed image and does not connect to the seat pipe.
        let signed = system_cmd();
        let application = wide_null(signed.as_os_str());
        let signed_image = unsafe { CreateFileW(application.as_ptr(), 0x0002_0021, 7,
            ptr::null(), 3, 0, ptr::null_mut()) };
        let (signed_execute_open, signed_image_section_result) = if signed_image as isize == -1 {
            (format!("WIN32_{:?}", io::Error::last_os_error().raw_os_error()),
                "NOT_RUN".to_owned())
        } else {
            let section = image_section(signed_image);
            unsafe { CloseHandle(signed_image) };
            ("OK".to_owned(), section)
        };
        let mut command = wide_null(OsStr::new(&format!("\"{}\" /D /C exit 0", signed.display())));
        let mut startup: StartupInfoW = unsafe { zeroed() };
        startup.cb = size_of::<StartupInfoW>() as u32;
        let mut info: ProcessInformation = unsafe { zeroed() };
        let created = unsafe { CreateProcessW(application.as_ptr(), command.as_mut_ptr(),
            ptr::null(), ptr::null(), 0, CREATE_NO_WINDOW, ptr::null(), ptr::null(),
            &mut startup, &mut info) };
        let raw_control = if created == 0 {
            // Capture both thread-local values before formatting or any other
            // native call. LastNtStatus is a clue, not CreateProcess's error
            // contract; GetLastError remains the reported failure.
            let win32 = unsafe { GetLastError() };
            let ntstatus = unsafe { RtlGetLastNtStatus() } as u32;
            format!("raw_signed_child=WIN32_{win32}; last_ntstatus={ntstatus:#010x}")
        } else {
            let process = OwnedHandle::new(info.process).expect("raw child process handle");
            let _thread = OwnedHandle::new(info.thread).expect("raw child thread handle");
            let wait = unsafe { WaitForSingleObject(process.raw(), 5000) };
            if wait != 0 {
                unsafe { TerminateProcess(process.raw(), 1) };
                unsafe { WaitForSingleObject(process.raw(), 5000) };
            }
            format!("raw_signed_child=OK; wait={wait}")
        };
        // The seat listener admits one client. A second helper after a
        // successful spawn would test a closed pipe and poison the same error
        // file, so only compare raw CreateProcess when the first spawn failed.
        let raw_helper = if spawn_failed {
            let mut helper_command = wide_null(OsStr::new(&format!(
                "\"{}\" --exact process::windows::tests::seat_pipe_child_helper --nocapture",
                path.display())));
            let mut helper_startup: StartupInfoW = unsafe { zeroed() };
            helper_startup.cb = size_of::<StartupInfoW>() as u32;
            let mut helper_info: ProcessInformation = unsafe { zeroed() };
            let helper_created = unsafe { CreateProcessW(wide.as_ptr(), helper_command.as_mut_ptr(),
                ptr::null(), ptr::null(), 0, CREATE_NO_WINDOW, ptr::null(), ptr::null(),
                &mut helper_startup, &mut helper_info) };
            if helper_created == 0 {
                let win32 = unsafe { GetLastError() };
                let ntstatus = unsafe { RtlGetLastNtStatus() } as u32;
                format!("raw_helper=WIN32_{win32}; last_ntstatus={ntstatus:#010x}")
            } else {
                let process = OwnedHandle::new(helper_info.process).expect("raw helper process handle");
                let _thread = OwnedHandle::new(helper_info.thread).expect("raw helper thread handle");
                let wait = unsafe { WaitForSingleObject(process.raw(), 20_000) };
                if wait != 0 {
                    unsafe { TerminateProcess(process.raw(), 1) };
                    unsafe { WaitForSingleObject(process.raw(), 5000) };
                }
                format!("raw_helper=OK; wait={wait}")
            }
        } else {
            "raw_helper=NOT_RUN_INITIAL_SPAWN_SUCCEEDED".to_owned()
        };
        // PROCESS_MITIGATION_POLICY::ProcessChildProcessPolicy is index 13.
        // Observe the effective policy; do not alter system or process policy.
        let mut child_policy_flags = 0u32;
        let policy_read = unsafe { GetProcessMitigationPolicy(GetCurrentProcess(), 13,
            (&mut child_policy_flags as *mut u32).cast(), size_of::<u32>()) };
        let child_policy = if policy_read == 0 {
            format!("child_policy=WIN32_{:?}", io::Error::last_os_error().raw_os_error())
        } else {
            format!("child_policy_flags={child_policy_flags:#x}")
        };
        fs::write("parent-launch.txt", format!("read={read}; execute_open={execute_open}; image_section={image_section_result}; {outcome}; {control}; signed_execute_open={signed_execute_open}; signed_image_section={signed_image_section_result}; {raw_control}; {raw_helper}; {child_policy}"))
            .expect("persist direct child launch result");
    }

    #[test]
    fn real_app_container_descendant_reaches_native_seat_pipe() {
        use crate::ipc::PrivatePipeListener;
        use std::os::windows::ffi::OsStrExt;
        use std::time::{SystemTime, UNIX_EPOCH};
        #[link(name = "userenv")]
        extern "system" { fn DeleteAppContainerProfile(name: *const u16) -> i32; }
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let home = std::env::temp_dir().join(format!("gogoke-v37-seat-pipe-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&home).unwrap();
        let name = format!("Gogoke37.seatpipe{}.{nonce}", std::process::id());
        let profile = AppContainerProfile::ensure(&name, false).expect("test package profile");
        profile.grant_fresh_session_directory(&home).expect("fresh package directory");
        let package_sid = profile.package_sid_string().expect("package SID");
        drop(profile);
        let exe = home.join("seat-pipe-parent.exe");
        write_executable_with_directory_acl(&std::env::current_exe().expect("native test image"), &exe);
        let helper = home.join("seat-pipe-helper.exe");
        write_executable_with_directory_acl(&std::env::current_exe().expect("native test image"), &helper);
        let endpoint = format!("lpac-{}-{nonce}", std::process::id());
        let listener = PrivatePipeListener::bind_app_container(&endpoint, &package_sid)
            .expect("seat listener");
        let pipe_path = listener.path().to_owned();
        let (sender, receiver) = std::sync::mpsc::channel();
        let (release_sender, release_receiver) = std::sync::mpsc::channel();
        let (client_done_sender, client_done_receiver) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let mut connection = match listener.accept_app_container() {
                Ok(connection) => connection,
                Err(error) => {
                    assert!(sender.send(Err(error)).is_ok(), "seat error receiver");
                    return;
                }
            };
            let package = connection.peer_package_sid().map(str::to_owned);
            let peer = connection.take_peer_process().expect("seat peer process handle");
            assert!(connection.take_peer_process().is_none(), "peer handle moves once");
            assert!(sender.send(Ok((package, peer))).is_ok(), "seat result receiver");
            release_receiver.recv_timeout(Duration::from_secs(15)).expect("host admission result");
            connection.write_frame(b"ok").expect("native Job admission ack");
            // Keep the server end alive until the helper has consumed the ack.
            client_done_receiver.recv_timeout(Duration::from_secs(15))
                .expect("first helper completion before pipe close");
        });
        let mut launch = ProcessLaunch::new(&exe);
        launch.current_directory = Some(home.clone());
        launch.protocol_stdio = true;
        launch.app_container_profile = Some(name.clone());
        launch.environment = Some(vec![
            ("SystemRoot".into(), std::env::var("SystemRoot").unwrap()),
            ("USERPROFILE".into(), home.to_string_lossy().into_owned()),
            ("LOCALAPPDATA".into(), home.to_string_lossy().into_owned()),
            ("GOGOKE_TEST_SEAT_PIPE".into(), pipe_path),
            ("GOGOKE_TEST_SEAT_CHILD".into(), helper.to_string_lossy().into_owned()),
        ]);
        let debugger_directory = std::env::var_os("GOGOKE_LPAC_CDB_DIR").map(|pointer| {
            fs::write(PathBuf::from(pointer), home.to_string_lossy().as_bytes())
                .expect("publish ACL-granted LPAC diagnostic directory");
            home.clone()
        });
        if let Some(directory) = debugger_directory.as_ref() {
            launch.environment.as_mut().expect("test environment").push((
                "GOGOKE_LPAC_CDB_DIR".into(), directory.to_string_lossy().into_owned()));
        }
        launch.arguments = vec!["--exact".into(),
            "process::windows::tests::seat_pipe_parent_helper".into(), "--nocapture".into()];
        let mut custodian = ProcessCustodian::new().expect("seat custodian");
        let prepared = custodian.prepare(&request(launch)).expect("prepared LPAC child");
        custodian.activate(&prepared).expect("activated LPAC child");
        let diagnostic_timeout = if debugger_directory.is_some() { 120 } else { 15 };
        let accepted = receiver.recv_timeout(Duration::from_secs(diagnostic_timeout))
            .unwrap_or_else(|_| panic!("LPAC pipe connection timed out; direct child launch={:?}; client error={:?}; parent_exit={:?}",
                std::fs::read_to_string(home.join("parent-launch.txt")),
                std::fs::read_to_string(home.join("pipe-client-error.txt")),
                process_exit_code(custodian.active(&prepared.ticket).unwrap().process.raw())));
        let (accepted, peer) = accepted.expect("LPAC peer identity");
        assert_ne!(peer.pid(), 0);
        assert_ne!(peer.pid(), prepared.identity.pid,
            "pipe peer must be the managed Job descendant, not its parent");
        assert!(custodian.peer_in_active_job(&prepared.ticket, &peer)
            .expect("exact seat Job membership"));
        let mut other = ProcessCustodian::new().expect("unrelated custodian");
        let mut unrelated_launch = ProcessLaunch::new(powershell());
        unrelated_launch.arguments = vec!["-NoProfile".into(), "-NonInteractive".into(),
            "-Command".into(), "Start-Sleep -Seconds 10".into()];
        let unrelated = other.prepare(&request(unrelated_launch)).expect("unrelated child");
        other.activate(&unrelated).expect("unrelated active Job");
        assert!(!other.peer_in_active_job(&unrelated.ticket, &peer)
            .expect("different Job membership"), "same-user peer in another Job must be denied");
        drop(other);
        assert!(!custodian.peer_in_active_job(&unrelated.ticket, &peer)
            .expect("missing Job is denied"));
        release_sender.send(()).expect("admit verified Job peer");
        assert!(custodian.active(&prepared.ticket).unwrap().wait(Duration::from_secs(10))
            .expect("LPAC exit"));
        let first_helper_status = std::fs::read_to_string(home.join("first-helper-status.txt"))
            .expect("first LPAC helper completion");
        assert_eq!(first_helper_status, "SUCCESS", "first LPAC helper must complete the protocol; stderr={:?}",
            std::fs::read_to_string(home.join("first-helper-stderr.txt")));
        client_done_sender.send(()).expect("first helper completed before pipe close");
        server.join().expect("seat listener thread");
        assert!(!home.join("pipe-client-error.txt").exists(), "LPAC pipe client reported failure");
        assert_eq!(accepted.as_deref(), Some(package_sid.as_str()));
        drop(peer);
        drop(custodian);
        let wide: Vec<u16> = OsStr::new(&name).encode_wide().chain(Some(0)).collect();
        assert!(unsafe { DeleteAppContainerProfile(wide.as_ptr()) } >= 0);
        std::fs::remove_file(home.join("parent-launch.txt")).unwrap();
        std::fs::remove_file(home.join("first-helper-status.txt")).unwrap();
        std::fs::remove_file(home.join("first-helper-stderr.txt")).unwrap();
        std::fs::remove_file(helper).unwrap();
        std::fs::remove_file(exe).unwrap();
        std::fs::remove_dir(home).unwrap();
    }

    #[test]
    fn two_native_child_pipes_keep_distinct_origin_bindings() {
        let mut custodian = ProcessCustodian::new().expect("custodian");
        let mut tickets = Vec::new();
        for label in ["first-origin", "second-origin"] {
            let mut launch = ProcessLaunch::new(system_cmd());
            launch.protocol_stdio = true;
            launch.arguments = vec!["/D".into(), "/C".into(), format!("echo {label}")];
            let prepared = custodian.prepare(&request(launch)).expect("prepared child");
            custodian.activate(&prepared).expect("activated exact child");
            tickets.push((prepared, label));
        }
        for (prepared, label) in &tickets {
            let frame = custodian.read_child_frame(&prepared.ticket, Duration::from_secs(5))
                .expect("frame from exact child pipe");
            assert_eq!(frame.custody(), prepared);
            assert_eq!(String::from_utf8_lossy(frame.bytes()).trim(), *label);
        }
        assert_ne!(tickets[0].0.ticket, tickets[1].0.ticket);
    }

    #[test]
    fn protocol_write_deadline_stops_non_reader_job() {
        let mut launch = ProcessLaunch::new(powershell());
        launch.protocol_stdio = true;
        launch.arguments = vec!["-NoProfile".into(), "-NonInteractive".into(),
            "-Command".into(), "Start-Sleep -Seconds 30".into()];
        let managed = prepare_and_activate(&launch, |_| Ok(())).expect("durably activate non-reader");
        let mut frame = vec![b'x'; 64 * 1024];
        *frame.last_mut().expect("nonempty frame") = b'\n';
        let started = Instant::now();
        let error = managed.write_protocol(&frame).expect_err("non-reader must not complete write");
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(10), "write deadline was not bounded");
        assert!(managed.wait(Duration::from_secs(2)).expect("exact process stopped"));
        assert_eq!(managed.exit_code().expect("exit code"), Some(STOP_TIMEOUT_EXIT_CODE));
        assert_eq!(managed.write_protocol(b"second\n").expect_err("single attempt").kind(),
            io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn prepared_process_identity_is_durable_before_activation() {
        use crate::root::RootLock;
        use crate::store::authority::{initialize_process_custody_schema, mark_process_active,
            mark_process_stopped, mark_process_unknown, record_prepared_process};
        use crate::store::same_open::route_b_test_guard;
        use crate::store::session::open_product_database;

        let _guard = route_b_test_guard();
        let path = unique_marker("prepared-coordination");
        fs::create_dir(&path).expect("root");
        let root = RootLock::acquire(&path).expect("root lock");
        let database = path.join("state.sqlite");
        let mut connection = open_product_database(&root, &database).expect("database");
        initialize_process_custody_schema(&mut connection).expect("coordination schema");
        let mut custodian = ProcessCustodian::new().expect("custodian");
        let mut launch = ProcessLaunch::new(powershell());
        launch.arguments = vec!["-NoProfile".into(), "-NonInteractive".into(),
            "-Command".into(), "Start-Sleep -Seconds 1".into()];
        let prepared = custodian.prepare(&request(launch)).expect("suspended process");
        record_prepared_process(&mut connection, "r2-02-test", &prepared).expect("durable PREPARED");
        assert!(record_prepared_process(&mut connection, "r2-02-test", &prepared).is_err());
        custodian.activate(&prepared).expect("activate exact prepared identity");
        mark_process_active(&mut connection, "r2-02-test", &prepared).expect("durable ACTIVE");
        assert!(mark_process_active(&mut connection, "r2-02-test", &prepared).is_err());
        let proof = custodian.stop(&prepared.ticket, StopBudgets::production(), || Ok(()))
            .expect("native stop proof");
        let revision = mark_process_stopped(&mut connection, "r2-02-test", &proof)
            .expect("durable stop proof");
        custodian.confirm_stop_durable(&DurableStopConfirmation {
            ticket: prepared.ticket.clone(), custodian_nonce: prepared.custodian_nonce.clone(),
            identity: prepared.identity.clone(), proof_hash: proof.proof_hash(),
            durable_revision: revision,
        }).expect("release only after durable stop");
        assert!(mark_process_unknown(&mut connection, "r2-02-test", &prepared).is_err());
        assert!(mark_process_active(&mut connection, "r2-02-test", &prepared).is_err());
        let mut unresolved_launch = ProcessLaunch::new(powershell());
        unresolved_launch.arguments = vec!["-NoProfile".into(), "-NonInteractive".into(),
            "-Command".into(), "Start-Sleep -Seconds 1".into()];
        let unresolved = custodian.prepare(&request(unresolved_launch)).expect("second suspended process");
        record_prepared_process(&mut connection, "r2-02-unresolved", &unresolved).expect("durable unresolved intent");
        drop(custodian);
        connection.close_checked().expect("close checked");
        let mut reopened = open_product_database(&root, &database).expect("reopen coordination");
        initialize_process_custody_schema(&mut reopened).expect("recovery downgrade");
        assert!(mark_process_active(&mut reopened, "r2-02-unresolved", &unresolved).is_err());
        reopened.close_checked().expect("close recovered coordination");
        drop(root);
        fs::remove_file(&database).ok();
        fs::remove_file(format!("{}-wal", database.display())).ok();
        fs::remove_file(format!("{}-shm", database.display())).ok();
        fs::remove_file(path.join(".gogoke-state.sqlite.custody-v1")).ok();
        fs::remove_dir(&path).ok();
    }

    #[test]
    fn unknown_or_mismatched_identity_never_authorizes_pid_targeting() {
        let recorded = ProcessIdentity {
            pid: 101,
            creation_time_100ns: 202,
            image_path: PathBuf::from(r"C:\controlled\fake.exe"),
        };
        assert!(!may_target_pid(&recorded, None));
        assert!(!may_target_pid(
            &recorded,
            Some(&ProcessIdentity {
                creation_time_100ns: 203,
                ..recorded.clone()
            })
        ));
        assert!(!may_target_pid(
            &recorded,
            Some(&ProcessIdentity {
                image_path: PathBuf::from(r"C:\other\fake.exe"),
                ..recorded.clone()
            })
        ));
        assert!(may_target_pid(&recorded, Some(&recorded)));
    }

    #[test]
    fn durable_custody_failure_cannot_run_suspended_child() {
        let marker = unique_marker("durable-failure");
        let mut launch = ProcessLaunch::new(powershell());
        launch.arguments = vec![
            "-NoProfile".to_owned(),
            "-NonInteractive".to_owned(),
            "-Command".to_owned(),
            format!("[IO.File]::WriteAllText('{}','ran')", ps_literal(&marker)),
        ];
        let result =
            prepare_and_activate(
                &launch,
                |_| Err("injected durable write failure".to_owned()),
            );
        assert!(matches!(
            result,
            Err(ProcessCustodyError::DurableCustody(_))
        ));
        thread::sleep(Duration::from_millis(200));
        assert!(
            !marker.exists(),
            "suspended child must not execute before custody"
        );
    }

    #[test]
    fn native_host_prepare_and_activate_are_separate_fail_closed_phases() {
        let marker = unique_marker("two-phase");
        let marker_text = marker.to_string_lossy();
        let invalid_path_char = marker_text.chars().find(|ch| !ch.is_ascii_alphanumeric()
            && !matches!(ch, ':' | '\\' | '/' | '-' | '_' | '.' | '~'));
        assert!(invalid_path_char.is_none(),
            "cmd marker fixture has unsupported character U+{:04X}",
            invalid_path_char.unwrap_or('\0') as u32);
        let mut launch = ProcessLaunch::new(system_cmd());
        launch.arguments = vec![
            "/D".to_owned(),
            "/C".to_owned(),
            format!("echo activated>{marker_text}"),
        ];
        let mut custodian = ProcessCustodian::new().expect("custodian");
        let prepared = custodian
            .prepare(&request(launch))
            .expect("prepare suspended child");
        thread::sleep(Duration::from_millis(100));
        assert!(
            !marker.exists(),
            "prepare must not run the child"
        );

        let mismatch = PreparedCustody {
            identity: ProcessIdentity {
                creation_time_100ns: prepared.identity.creation_time_100ns + 1,
                ..prepared.identity.clone()
            },
            ..prepared.clone()
        };
        assert!(matches!(
            custodian.activate(&mismatch),
            Err(ProcessCustodyError::DurableIdentityMismatch(_))
        ));
        assert!(
            !marker.exists(),
            "identity mismatch must remain suspended"
        );

        custodian
            .activate(&prepared)
            .expect("activate exact durable identity");
        let active = custodian.active(&prepared.ticket).expect("active ticket");
        let started = Instant::now();
        let waited = active.wait(Duration::from_secs(5)).expect("wait fixture");
        let exit_code = process_exit_code(active.process.raw()).ok().flatten();
        assert!(
            waited,
            "fixture did not finish: elapsed_ms={} process_exit_code={exit_code:?} active_job_processes={:?} marker={}",
            started.elapsed().as_millis(),
            active.active_job_processes().ok(),
            marker.exists()
        );
        assert_eq!(
            fs::read_to_string(&marker).ok().map(|value| value.trim().to_owned()),
            Some("activated".to_owned()),
            "fixture activation failed: elapsed_ms={} process_exit_code={exit_code:?} active_job_processes={:?}",
            started.elapsed().as_millis(),
            active.active_job_processes().ok()
        );
        let proof = custodian
            .stop(
                &prepared.ticket,
                StopBudgets::production(),
                || Ok(()),
            )
            .expect("native stop proof");
        assert!(custodian.is_tombstoned(&prepared.ticket));
        assert!(matches!(
            custodian.confirm_stop_durable(&DurableStopConfirmation {
                ticket: prepared.ticket.clone(),
                custodian_nonce: prepared.custodian_nonce.clone(),
                identity: mismatch.identity,
                proof_hash: proof.proof_hash(),
                durable_revision: 3,
            }),
            Err(ProcessCustodyError::DurableIdentityMismatch(_))
        ));
        assert!(custodian.active(&prepared.ticket).is_some());
        assert!(proof.errors.is_empty() && proof.parent_exited && proof.writer_fence_verified
            && proof.active_job_processes == Some(0), "stop proof before durable release: {proof:?}");
        let confirmed = custodian
            .confirm_stop_durable(&DurableStopConfirmation {
                ticket: prepared.ticket.clone(),
                custodian_nonce: prepared.custodian_nonce.clone(),
                identity: prepared.identity.clone(),
                proof_hash: proof.proof_hash(),
                durable_revision: 3,
            })
            .expect("release exact durable stop");
        assert_eq!(confirmed, proof);
        assert!(custodian.active(&prepared.ticket).is_none());
        assert!(matches!(
            custodian.activate(&prepared),
            Err(ProcessCustodyError::DuplicateTicket(_))
        ));
        let _ = fs::remove_file(marker);
    }

    #[test]
    fn opaque_tickets_are_internal_instance_bound_and_tombstoned_after_abort() {
        let mut launch = ProcessLaunch::new(powershell());
        launch.arguments = vec![
            "-NoProfile".to_owned(),
            "-NonInteractive".to_owned(),
            "-Command".to_owned(),
            "Start-Sleep -Seconds 30".to_owned(),
        ];
        let mut owner = ProcessCustodian::new().expect("owner custodian");
        let mut other = ProcessCustodian::new().expect("other custodian");
        let prepared = owner.prepare(&request(launch)).expect("prepare");
        assert!(prepared.ticket.opaque().starts_with("pct1_"));
        assert_eq!(prepared.ticket.opaque().len(), 69);
        assert!(prepared.custodian_nonce.starts_with("pcn1_"));
        assert!(matches!(
            other.activate(&prepared),
            Err(ProcessCustodyError::BindingMismatch("custodianNonce"))
        ));
        assert!(owner.abort_prepared(&prepared).expect("abort"));
        assert!(owner.is_tombstoned(&prepared.ticket));
        assert!(matches!(
            owner.activate(&prepared),
            Err(ProcessCustodyError::DuplicateTicket(_))
        ));
        assert!(matches!(
            owner.abort_prepared(&prepared),
            Err(ProcessCustodyError::DuplicateTicket(_))
        ));
    }

    #[test]
    fn blocking_close_hook_cannot_hold_host_past_deadline() {
        for deadline_ms in [75u32, 1u32] {
            let mut launch = ProcessLaunch::new(powershell());
            launch.arguments = vec![
                "-NoProfile".to_owned(),
                "-NonInteractive".to_owned(),
                "-Command".to_owned(),
                "Start-Sleep -Seconds 30".to_owned(),
            ];
            let process = prepare_and_activate(&launch, |_| Ok(())).expect("controlled sleeper");
            let started = Instant::now();
            let proof = process.stop(
                StopBudgets {
                    grace_ms: deadline_ms,
                    terminate_ms: 0,
                    observe_ms: 0,
                    host_deadline_ms: deadline_ms,
                },
                || {
                    thread::sleep(Duration::from_secs(5));
                    Ok(())
                },
            );
            assert!(started.elapsed() < Duration::from_millis(u64::from(deadline_ms) + 250));
            assert!(proof
                .errors
                .iter()
                .any(|error| error == "CLOSE_BINDING_DEADLINE_EXCEEDED"));
        }
    }

    #[test]
    fn sha256_and_binary_binding_are_exact() {
        assert_eq!(
            hex_bytes(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let launch = ProcessLaunch::new(powershell());
        let mut bad = request(launch);
        bad.binding.binary_digest_sha256 = format!("sha256:{}", "0".repeat(64));
        let mut custodian = ProcessCustodian::new().expect("custodian");
        assert!(matches!(
            custodian.prepare(&bad),
            Err(ProcessCustodyError::BindingMismatch("binaryDigestSha256"))
        ));

        let proof = NativeStopProof {
            ticket: ProcessTicket(format!("pct1_{}", "a".repeat(64))),
            custodian_nonce: format!("pcn1_{}", "b".repeat(64)),
            binding: NativeBinding {
                binary_digest_sha256: format!("sha256:{}", "c".repeat(64)),
                profile_id: "profile".to_owned(),
                domain_id: "domain".to_owned(),
                generation: "7".to_owned(),
            },
            identity: ProcessIdentity {
                pid: 42,
                creation_time_100ns: 99,
                image_path: PathBuf::from(r"C:\fixture\fake.exe"),
            },
            parent_exited: true,
            active_job_processes: Some(0),
            identity_status: "exact".to_owned(),
            process_handle_present: true,
            job_handle_present: true,
            kill_attempted: false,
            kill_succeeded: false,
            writer_fence_verified: true,
            exit_code: Some(0),
            deadline_exceeded: false,
            errors: Vec::new(),
        };
        assert_eq!(
            proof.proof_hash(),
            "sha256:6de76f072fd07f424de91943871f249eb3bdc2f22b7b33dcf302cd14fdd38e7e"
        );
    }

    #[test]
    fn real_windows_job_keeps_descendant_after_parent_exit_and_then_stops_tree() {
        let marker = unique_marker("descendant");
        let entry_marker = unique_marker("descendant-entry");
        let error_marker = unique_marker("descendant-error");
        let child_command = "Start-Sleep -Seconds 30";
        let script = format!(
            "$ErrorActionPreference='Stop'; try {{ [IO.File]::WriteAllText('{}','parent-started'); $i=[Diagnostics.ProcessStartInfo]::new();",
            ps_literal(&entry_marker)
        )
            + "$i.FileName=$PSHOME+'\\powershell.exe';"
            + "$i.Arguments='-NoProfile -NonInteractive -Command \""
            + child_command
            + "\"';$i.UseShellExecute=$false;"
            + "$p=[Diagnostics.Process]::Start($i);"
            + &format!(
                "[IO.File]::WriteAllText('{}',[string]$p.Id); }} catch {{ [IO.File]::WriteAllText('{}',[string]$_); exit 17 }}",
                ps_literal(&marker),
                ps_literal(&error_marker)
            );
        let mut launch = ProcessLaunch::new(powershell());
        launch.arguments = vec![
            "-NoProfile".to_owned(),
            "-NonInteractive".to_owned(),
            "-Command".to_owned(),
            script,
        ];
        let captured = Arc::new(Mutex::new(None));
        let captured_for_callback = Arc::clone(&captured);
        let process = prepare_and_activate(&launch, move |identity| {
            *captured_for_callback.lock().expect("identity lock") = Some(identity.clone());
            Ok(())
        })
        .expect("prepare and activate controlled process tree");
        assert!(process
            .handles_are_non_inheritable()
            .expect("handle policy"));
        let started = Instant::now();
        let parent_finished = process.wait(Duration::from_secs(15)).expect("wait parent");
        assert!(
            parent_finished,
            "controlled parent did not exit: elapsed_ms={} process_exit_code={:?} active_job_processes={:?} entry_marker={} error_marker={:?}",
            started.elapsed().as_millis(),
            process_exit_code(process.process.raw()).ok().flatten(),
            process.active_job_processes().ok(),
            entry_marker.exists(),
            fs::read_to_string(&error_marker).ok()
        );
        assert!(marker.exists(), "controlled parent exited without publishing child pid: elapsed_ms={} process_exit_code={:?} active_job_processes={:?} entry_marker={} error_marker={:?}", started.elapsed().as_millis(), process_exit_code(process.process.raw()).ok().flatten(), process.active_job_processes().ok(), entry_marker.exists(), fs::read_to_string(&error_marker).ok());
        assert!(
            process.active_job_processes().expect("job accounting") >= 1,
            "descendant must remain in the owned job after parent exit: elapsed_ms={} process_exit_code={:?} active_job_processes={:?} entry_marker={} error_marker={:?}",
            started.elapsed().as_millis(),
            process_exit_code(process.process.raw()).ok().flatten(),
            process.active_job_processes().ok(),
            entry_marker.exists(),
            fs::read_to_string(&error_marker).ok()
        );
        assert_eq!(
            captured.lock().expect("identity lock").as_ref(),
            Some(process.identity())
        );

        let proof = process.stop(
            StopBudgets {
                grace_ms: 50,
                terminate_ms: 1_000,
                observe_ms: 1_000,
                host_deadline_ms: 3_000,
            },
            || Ok(()),
        );
        assert_eq!(proof.disposition, StopDisposition::Stopped);
        assert!(proof.parent_exited);
        assert_eq!(proof.active_job_processes, Some(0));
        assert!(proof.writer_fence_verified);
        assert!(!proof.durable_receipt_saved);
        let _ = fs::remove_file(marker);
        let _ = fs::remove_file(entry_marker);
        let _ = fs::remove_file(error_marker);
    }

    #[test]
    fn actual_kernel_rejects_invalid_job_handle_and_second_stop_retains_custody() {
        let error = terminate_job(ptr::null_mut(), STOP_TIMEOUT_EXIT_CODE)
            .expect_err("null job handle must fail through the actual Windows API");
        assert_ne!(error.raw_os_error(), Some(0));

        let mut launch = ProcessLaunch::new(powershell());
        launch.arguments = vec![
            "-NoProfile".to_owned(),
            "-NonInteractive".to_owned(),
            "-Command".to_owned(),
            "Start-Sleep -Seconds 30".to_owned(),
        ];
        let process = prepare_and_activate(&launch, |_| Ok(())).expect("controlled sleeper");
        let proof = process.stop(
            StopBudgets {
                grace_ms: 20,
                terminate_ms: 1_000,
                observe_ms: 1_000,
                host_deadline_ms: 3_000,
            },
            || Ok(()),
        );
        assert_eq!(proof.disposition, StopDisposition::ResidualCustody);
        assert!(proof.writer_fence_verified);
        assert!(!proof.durable_receipt_saved);
        assert_eq!(proof.exit_code, Some(STOP_TIMEOUT_EXIT_CODE));
        assert!(proof
            .errors
            .iter()
            .any(|error| error == "STOP_EXIT_124_REQUIRES_RECONCILIATION"));
        let repeated = process.stop(StopBudgets::production(), || {
            panic!("second stop must not issue another close")
        });
        assert_eq!(repeated.disposition, StopDisposition::ResidualCustody);
        assert!(repeated
            .errors
            .iter()
            .any(|error| error == "SECOND_STOP_REQUIRES_DURABLE_RECONCILIATION"));
    }

    #[test]
    fn windows_command_line_quotes_backslashes_before_quotes_and_end() {
        assert_eq!(quote_windows_argument("plain"), "plain");
        assert_eq!(quote_windows_argument(""), "\"\"");
        assert_eq!(quote_windows_argument("a b\"c"), "\"a b\\\"c\"");
        assert_eq!(
            quote_windows_argument("C:\\path with space\\"),
            "\"C:\\path with space\\\\\""
        );
    }
}
