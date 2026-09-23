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
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

type Handle = *mut c_void;

const CREATE_SUSPENDED: u32 = 0x0000_0004;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
const STARTF_USESTDHANDLES: u32 = 0x0000_0100;
const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: usize = 0x0002_0002;
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

pub const STOP_GRACE_MS: u32 = 10_000;
pub const STOP_TERMINATE_MS: u32 = 5_000;
pub const STOP_OBSERVE_MS: u32 = 5_000;
pub const HOST_STOP_DEADLINE_MS: u32 = 30_000;
pub const STOP_TIMEOUT_EXIT_CODE: u32 = 124;
pub const STOP_REFUSED_EXIT_CODE: u32 = 125;

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
}

impl ProcessLaunch {
    pub fn new(application: impl Into<PathBuf>) -> Self {
        Self {
            application: application.into(),
            arguments: Vec::new(),
            current_directory: None,
            hide_window: true,
            protocol_stdio: false,
        }
    }
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
    stdin_write: OwnedHandle,
    stdout_read: OwnedHandle,
}

struct ChildProtocolHandles {
    stdin_read: OwnedHandle,
    stdout_write: OwnedHandle,
    parent: ProtocolPipes,
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
        stdin_write.clear_inherit().map_err(ProcessCustodyError::HandlePolicy)?;
        stdout_read.clear_inherit().map_err(ProcessCustodyError::HandlePolicy)?;
        Ok(Self {
            stdin_read,
            stdout_write,
            parent: ProtocolPipes { stdin_write, stdout_read },
        })
    }
}

struct AttributeList {
    storage: Vec<usize>,
}

impl AttributeList {
    fn handles(handles: &mut [Handle]) -> Result<Self, ProcessCustodyError> {
        let mut size = 0usize;
        unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut size); }
        if size == 0 {
            return Err(ProcessCustodyError::ProtocolAttribute(io::Error::last_os_error()));
        }
        let mut storage = vec![0usize; size.div_ceil(size_of::<usize>())];
        let list = storage.as_mut_ptr().cast();
        if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut size) } == 0 {
            return Err(ProcessCustodyError::ProtocolAttribute(io::Error::last_os_error()));
        }
        let result = Self { storage };
        if unsafe {
            UpdateProcThreadAttribute(result.raw(), 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                handles.as_mut_ptr().cast(), size_of_val(handles), ptr::null_mut(), ptr::null_mut())
        } == 0 {
            return Err(ProcessCustodyError::ProtocolAttribute(io::Error::last_os_error()));
        }
        Ok(result)
    }

    fn raw(&self) -> *mut c_void { self.storage.as_ptr().cast_mut().cast() }
}

impl Drop for AttributeList {
    fn drop(&mut self) { unsafe { DeleteProcThreadAttributeList(self.raw()); } }
}

struct PreparedProcess {
    process: OwnedHandle,
    initial_thread: OwnedHandle,
    job: OwnedHandle,
    identity: ProcessIdentity,
    protocol: Option<ProtocolPipes>,
}

impl PreparedProcess {
    fn prepare(launch: &ProcessLaunch) -> Result<Self, ProcessCustodyError> {
        validate_launch(launch)?;
        let job = create_kill_on_close_job()?;
        let (process, initial_thread, pid, protocol) = create_suspended(launch)?;
        if unsafe { AssignProcessToJobObject(job.raw(), process.raw()) } == 0 {
            let source = io::Error::last_os_error();
            // The exact process handle is still suspended, so this cannot hit
            // a reused PID and the child has never had an instruction run.
            unsafe {
                TerminateProcess(process.raw(), STOP_REFUSED_EXIT_CODE);
                WaitForSingleObject(process.raw(), 1_000);
            }
            return Err(ProcessCustodyError::AssignJob(source));
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
        })
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
        } = self;
        drop(initial_thread);
        Ok(ManagedProcess {
            process,
            job,
            identity,
            protocol,
            stop_attempted: AtomicBool::new(false),
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
    let prepared = PreparedProcess::prepare(launch)?;
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
}

impl ProcessCustodian {
    pub fn new() -> Result<Self, ProcessCustodyError> {
        Ok(Self {
            nonce: format!("pcn1_{}", random_hex_32()?),
            prepared: HashMap::new(),
            active: HashMap::new(),
            pending_stop: HashMap::new(),
            tombstones: HashSet::new(),
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
        let prepared = PreparedProcess::prepare(&request.launch)?;
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
    pub fn activate(
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
    stop_attempted: AtomicBool,
}

impl ManagedProcess {
    pub fn identity(&self) -> &ProcessIdentity {
        &self.identity
    }

    pub fn handles_are_non_inheritable(&self) -> io::Result<bool> {
        Ok(!self.process.is_inheritable()? && !self.job.is_inheritable()?)
    }

    /// Byte-bounded synchronous protocol transport. The product caller must
    /// provide write liveness, admission and the beginCommitted boundary;
    /// neither an ACK nor these bytes prove completion.
    pub fn write_protocol(&self, bytes: &[u8]) -> io::Result<()> {
        let protocol = self.protocol.as_ref().ok_or_else(||
            io::Error::new(io::ErrorKind::Unsupported, "protocol stdio was not admitted"))?;
        if bytes.is_empty() || bytes.len() > 64 * 1024 || bytes.last() != Some(&b'\n') {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "protocol command must be one bounded JSONL frame"));
        }
        let mut written = 0u32;
        if unsafe { WriteFile(protocol.stdin_write.raw(), bytes.as_ptr().cast(),
            bytes.len() as u32, &mut written, ptr::null_mut()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if written as usize != bytes.len() {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "partial protocol command"));
        }
        Ok(())
    }

    /// Reads one LF frame with an explicit deadline and byte bound. EOF or
    /// process exit before LF is an error, never a task result.
    pub fn read_protocol_frame(&self, deadline: Duration) -> io::Result<Vec<u8>> {
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
    Ok(())
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
) -> Result<(OwnedHandle, OwnedHandle, u32, Option<ProtocolPipes>), ProcessCustodyError> {
    if launch.protocol_stdio {
        return create_suspended_protocol(launch);
    }
    let application = wide_null(launch.application.as_os_str());
    let mut command_line = wide_null(OsStr::new(&build_command_line(
        launch.application.as_os_str(),
        &launch.arguments,
    )));
    let current_directory = launch
        .current_directory
        .as_ref()
        .map(|path| wide_null(path.as_os_str()));
    let mut startup: StartupInfoW = unsafe { zeroed() };
    startup.cb = size_of::<StartupInfoW>() as u32;
    let mut info: ProcessInformation = unsafe { zeroed() };
    let flags = CREATE_SUSPENDED
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
            ptr::null(),
            current_directory
                .as_ref()
                .map_or(ptr::null(), |value| value.as_ptr()),
            &mut startup,
            &mut info,
        )
    };
    if created == 0 {
        return Err(ProcessCustodyError::CreateProcess(
            io::Error::last_os_error(),
        ));
    }
    let process =
        OwnedHandle::new(info.process).expect("CreateProcessW returned null process handle");
    let initial_thread =
        OwnedHandle::new(info.thread).expect("CreateProcessW returned null thread handle");
    Ok((process, initial_thread, info.process_id, None))
}

fn create_suspended_protocol(
    launch: &ProcessLaunch,
) -> Result<(OwnedHandle, OwnedHandle, u32, Option<ProtocolPipes>), ProcessCustodyError> {
    let pipes = ChildProtocolHandles::open()?;
    let stderr = OpenOptions::new().write(true).open("NUL")
        .map_err(ProcessCustodyError::ProtocolPipe)?;
    let stderr_handle = stderr.as_raw_handle().cast();
    if unsafe { SetHandleInformation(stderr_handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0 {
        return Err(ProcessCustodyError::HandlePolicy(io::Error::last_os_error()));
    }
    let mut inherited = [pipes.stdin_read.raw(), pipes.stdout_write.raw(), stderr_handle];
    let attributes = AttributeList::handles(&mut inherited)?;
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
    let mut info: ProcessInformation = unsafe { zeroed() };
    let flags = CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT
        | if launch.hide_window { CREATE_NO_WINDOW } else { 0 };
    let created = unsafe {
        CreateProcessW(application.as_ptr(), command_line.as_mut_ptr(), ptr::null(), ptr::null(),
            1, flags, ptr::null(),
            current_directory.as_ref().map_or(ptr::null(), |value| value.as_ptr()),
            &mut startup.startup, &mut info)
    };
    let create_error = if created == 0 { Some(io::Error::last_os_error()) } else { None };
    let cleared = unsafe { SetHandleInformation(stderr_handle, HANDLE_FLAG_INHERIT, 0) };
    if let Some(error) = create_error { return Err(ProcessCustodyError::CreateProcess(error)); }
    let process = OwnedHandle::new(info.process).expect("CreateProcessW returned null process handle");
    let initial_thread = OwnedHandle::new(info.thread).expect("CreateProcessW returned null thread handle");
    if cleared == 0 {
        let error = io::Error::last_os_error();
        unsafe { TerminateProcess(process.raw(), STOP_REFUSED_EXIT_CODE); }
        return Err(ProcessCustodyError::HandlePolicy(error));
    }
    let ChildProtocolHandles { stdin_read, stdout_write, parent } = pipes;
    drop(stdin_read);
    drop(stdout_write);
    Ok((process, initial_thread, info.process_id, Some(parent)))
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
    fn suspended_job_child_receives_only_explicit_protocol_handles() {
        let mut launch = ProcessLaunch::new(powershell());
        launch.protocol_stdio = true;
        launch.arguments = vec![
            "-NoProfile".into(), "-NonInteractive".into(), "-Command".into(),
            "$line=[Console]::ReadLine(); [Console]::Out.WriteLine('echo:' + $line)".into(),
        ];
        let managed = prepare_and_activate(&launch, |_| Ok(())).expect("durably activate piped child");
        let pipes = managed.protocol.as_ref().expect("protocol pipes");
        assert!(!pipes.stdin_write.is_inheritable().expect("parent stdin handle"));
        assert!(!pipes.stdout_read.is_inheritable().expect("parent stdout handle"));
        assert_eq!(managed.write_protocol(b"no delimiter").unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(managed.read_protocol_frame(Duration::ZERO).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        managed.write_protocol(b"controlled\n").expect("bounded command write");
        let output = managed.read_protocol_frame(Duration::from_secs(5)).expect("bounded frame read");
        assert!(String::from_utf8_lossy(&output).contains("echo:controlled"));
        assert!(managed.wait(Duration::from_secs(5)).expect("child exit"));
    }

    #[test]
    fn prepared_process_identity_is_durable_before_activation() {
        use crate::root::RootLock;
        use crate::store::authority::{initialize_process_custody_schema, mark_process_active,
            mark_process_unknown, record_prepared_process};
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
        mark_process_unknown(&mut connection, "r2-02-test", &prepared).expect("unknown on unresolved outcome");
        assert!(mark_process_active(&mut connection, "r2-02-test", &prepared).is_err());
        drop(custodian);
        connection.close_checked().expect("close checked");
        drop(root);
        fs::remove_file(&database).ok();
        fs::remove_file(format!("{}-wal", database.display())).ok();
        fs::remove_file(format!("{}-shm", database.display())).ok();
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
        let entry_marker = unique_marker("two-phase-entry");
        let error_marker = unique_marker("two-phase-error");
        let mut launch = ProcessLaunch::new(powershell());
        launch.arguments = vec![
            "-NoProfile".to_owned(),
            "-NonInteractive".to_owned(),
            "-Command".to_owned(),
            format!(
                "$ErrorActionPreference='Stop'; try {{ [IO.File]::WriteAllText('{}','entered'); [IO.File]::WriteAllText('{}','activated'); exit 0 }} catch {{ [IO.File]::WriteAllText('{}',[string]$_); exit 17 }}",
                ps_literal(&entry_marker),
                ps_literal(&marker),
                ps_literal(&error_marker)
            ),
        ];
        let mut custodian = ProcessCustodian::new().expect("custodian");
        let prepared = custodian
            .prepare(&request(launch))
            .expect("prepare suspended child");
        thread::sleep(Duration::from_millis(100));
        assert!(
            !marker.exists(),
            "prepare must not run the child; entry={:?} error={:?}",
            entry_marker.exists(),
            fs::read_to_string(&error_marker).ok()
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
            "identity mismatch must remain suspended; entry={:?} error={:?}",
            entry_marker.exists(),
            fs::read_to_string(&error_marker).ok()
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
            "fixture did not finish: elapsed_ms={} process_exit_code={exit_code:?} active_job_processes={:?} entry_marker={} error_marker={:?}",
            started.elapsed().as_millis(),
            active.active_job_processes().ok(),
            entry_marker.exists(),
            fs::read_to_string(&error_marker).ok()
        );
        assert_eq!(
            fs::read_to_string(&marker).ok().as_deref(),
            Some("activated"),
            "fixture activation failed: elapsed_ms={} process_exit_code={exit_code:?} active_job_processes={:?} entry_marker={} error_marker={:?}",
            started.elapsed().as_millis(),
            active.active_job_processes().ok(),
            entry_marker.exists(),
            fs::read_to_string(&error_marker).ok()
        );
        let proof = custodian
            .stop(
                &prepared.ticket,
                StopBudgets {
                    grace_ms: 10,
                    terminate_ms: 10,
                    observe_ms: 100,
                    host_deadline_ms: 500,
                },
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
        let _ = fs::remove_file(entry_marker);
        let _ = fs::remove_file(error_marker);
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
        let marker_deadline = started + Duration::from_secs(5);
        while !marker.exists() && Instant::now() < marker_deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(marker.exists(), "controlled parent did not publish child pid: elapsed_ms={} process_exit_code={:?} active_job_processes={:?} entry_marker={} error_marker={:?}", started.elapsed().as_millis(), process_exit_code(process.process.raw()).ok().flatten(), process.active_job_processes().ok(), entry_marker.exists(), fs::read_to_string(&error_marker).ok());
        let parent_finished = process.wait(Duration::from_secs(5)).expect("wait parent");
        assert!(
            parent_finished,
            "controlled parent did not exit: elapsed_ms={} process_exit_code={:?} active_job_processes={:?} entry_marker={} error_marker={:?}",
            started.elapsed().as_millis(),
            process_exit_code(process.process.raw()).ok().flatten(),
            process.active_job_processes().ok(),
            entry_marker.exists(),
            fs::read_to_string(&error_marker).ok()
        );
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
