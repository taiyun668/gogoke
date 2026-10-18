use gogoke_native_host::ipc::{PrivatePipeListener, UserOriginProof};
use gogoke_native_host::root::{RootLock, RootLockError};
use gogoke_native_host::store::product_database::ProductDatabase;
#[cfg(windows)]
use std::ffi::c_void;
#[cfg(windows)]
use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, SyncSender};
use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy)]
enum HostMode {
    Legacy,
    Desktop { user_pid: u32 },
}

struct ServiceFrame {
    bytes: Vec<u8>,
    answer: SyncSender<(Vec<u8>, bool)>,
}

static FIRST_SERVICE_WRITE_FAILURE: OnceLock<String> = OnceLock::new();

fn service_operation_class(bytes: &[u8]) -> &'static str {
    use gogoke_native_host::store::protocol::decode_operation_frame;

    match decode_operation_frame(bytes).map(|decoded| decoded.name) {
        Ok("AuthenticateService") => "AuthenticateService",
        Ok("ReadProductIdentity") => "ReadProductIdentity",
        Ok("AdmitControllerCaller") => "AdmitControllerCaller",
        Ok("Shutdown") => "Shutdown",
        Ok(_) => "OTHER_ADMITTED",
        Err(_) => "UNCLASSIFIED",
    }
}

fn utc_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

fn service_reply_peer_closed(error: &gogoke_native_host::ipc::PrivateIpcError) -> bool {
    matches!(error, gogoke_native_host::ipc::PrivateIpcError::Os {
        operation: "WriteFile(private IPC frame)", source,
    } if matches!(source.raw_os_error(), Some(109 | 232 | 233)))
}

enum DesktopEvent {
    ServiceListening,
    ServiceConnected,
    ServiceFrame(ServiceFrame),
    ServiceDisconnected,
    ServiceFailed(String),
    UserConnected(UserOriginProof),
    UserFrame { bytes: Vec<u8>, answer: SyncSender<Vec<u8>> },
    UserDisconnected,
    UserFailed(String),
}

fn main() {
    match run() {
        Ok(()) => {}
        Err(error) => {
            eprintln!("{error}");
            if let Some(diagnostic) = FIRST_SERVICE_WRITE_FAILURE.get() {
                eprintln!("{diagnostic}");
            }
            std::process::exit(1);
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let (root, database, mode) = explicit_arguments()?;
    let lock = RootLock::acquire(&root)?;
    let canonical = lock.canonical_root();
    let non_inheritable = lock.handles_are_non_inheritable()?;
    println!(
        "LOCKED\t{}\t{}\t{}\tnon_inheritable={non_inheritable}",
        canonical.requested_path.display(),
        canonical.canonical_path.display(),
        canonical.identity.opaque(),
    );
    io::stdout().flush()?;
    let mut product = ProductDatabase::open(&lock, &database)?;
    let endpoint = format!(
        "store.{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let listener = PrivatePipeListener::bind(&endpoint)?;
    let service_capability = service_capability()?;
    let user_listener = match mode {
        HostMode::Legacy => None,
        HostMode::Desktop { user_pid } => Some(PrivatePipeListener::bind_user(&endpoint, user_pid)?),
    };
    println!("PIPE\t{}", listener.path());
    println!("CAPABILITY\t{service_capability}");
    if let Some(user) = &user_listener {
        println!("USER_PIPE\t{}", user.path());
    }
    io::stdout().flush()?;
    if let Some(user) = user_listener {
        let desktop = run_desktop(&mut product, listener, user, &endpoint, &service_capability);
        finish_desktop_run(product, desktop,
            |product| product.stop_secretary_before_close().map_err(Into::into),
            |product| product.close_checked().map(|_| ()).map_err(Into::into))?;
        drop(lock);
        return Ok(());
    }
    // The pipe worker owns only transport bytes. The main thread remains the
    // sole owner of the verified database, issuer and process custodian.
    let (sender, receiver) = mpsc::channel::<ServiceFrame>();
    let worker = std::thread::spawn(move || -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let pipe = listener.accept_current_user()?;
        loop {
            let bytes = pipe.read_frame()?;
            let (answer, reply) = mpsc::sync_channel(0);
            sender.send(ServiceFrame { bytes, answer })?;
            let (frame, should_stop) = reply.recv()?;
            pipe.write_frame(&frame)?;
            if should_stop {
                pipe.flush_for_disconnect()?;
                break;
            }
        }
        Ok(())
    });
    let mut service = product.begin_service_frames(&service_capability)?;
    for frame in receiver {
        let (reply, should_stop) = match product.dispatch_service_frame(&mut service, &frame.bytes) {
            Ok(value) => value,
            Err(error) => {
                drop(frame.answer);
                drop(worker.join());
                return Err(Box::new(error));
            }
        };
        frame.answer.send((reply, should_stop))?;
        if should_stop { break; }
    }
    worker.join().map_err(|_| io::Error::other("service pipe worker panicked"))?
        .map_err(|error| io::Error::other(error.to_string()))?;
    product.close_checked()?;
    drop(lock);
    Ok(())
}

/// Both pipe workers carry transport facts only. The one verified database,
/// issuer, and process custodian remain on this main authority thread.
type DesktopRunResult = Result<(), Box<dyn std::error::Error>>;

// The authority and its real process custody remain owned until both existing
// cleanup operations have been attempted, including on a transport/origin error.
fn finish_desktop_run<A>(mut authority: A, desktop: DesktopRunResult,
    stop: impl FnOnce(&mut A) -> DesktopRunResult,
    close: impl FnOnce(A) -> DesktopRunResult,
) -> DesktopRunResult {
    let stopped = stop(&mut authority);
    let closed = close(authority);
    match (desktop, stopped, closed) {
        (Ok(()), Ok(()), Ok(())) => Ok(()),
        (Err(primary), Ok(()), Ok(())) => Err(primary),
        (Err(primary), stopped, closed) => Err(io::Error::other(format!(
            "{primary}; Secretary normal shutdown: {stopped:?}; database close: {closed:?}")).into()),
        (Ok(()), Err(error), closed) => Err(io::Error::other(format!(
            "Secretary normal shutdown: {error:?}; database close: {closed:?}")).into()),
        (Ok(()), Ok(()), Err(error)) => Err(error),
    }
}

fn run_desktop(
    product: &mut ProductDatabase<'_>,
    service_listener: PrivatePipeListener,
    user_listener: PrivatePipeListener,
    endpoint: &str,
    capability: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let (sender, receiver) = mpsc::channel::<DesktopEvent>();
    let service_events = sender.clone();
    let endpoint = endpoint.to_owned();
    std::thread::spawn(move || {
        let mut next = Some(service_listener);
        let mut connection_seq = 0u64;
        loop {
            let listener = match next.take() {
                Some(listener) => listener,
                None => match PrivatePipeListener::bind(&endpoint) {
                    Ok(listener) => listener,
                    Err(error) => {
                        let _ = service_events.send(DesktopEvent::ServiceFailed(error.to_string()));
                        break;
                    }
                },
            };
            // Publish the actual replacement listener before accepting a new
            // client. Client EOF alone does not mean the sole instance exists.
            if service_events.send(DesktopEvent::ServiceListening).is_err() { break; }
            let pipe = match listener.accept_current_user() {
                Ok(pipe) => pipe,
                Err(error) => {
                    let _ = service_events.send(DesktopEvent::ServiceFailed(error.to_string()));
                    break;
                }
            };
            connection_seq = connection_seq.saturating_add(1);
            let current_connection_seq = connection_seq;
            if service_events.send(DesktopEvent::ServiceConnected).is_err() { break; }
            let mut request_seq = 0u64;
            loop {
                let bytes = match pipe.read_frame() {
                    Ok(bytes) => bytes,
                    Err(gogoke_native_host::ipc::PrivateIpcError::Os {
                        operation: "ReadFile(private IPC header EOF)", ..
                    }) => break,
                    Err(error) => {
                        drop(service_events.send(DesktopEvent::ServiceFailed(error.to_string())));
                        return;
                    }
                };
                request_seq = request_seq.saturating_add(1);
                let operation = service_operation_class(&bytes);
                let received_utc_ms = utc_now_ms();
                let received_at = Instant::now();
                let (answer, reply) = mpsc::sync_channel(0);
                if service_events.send(DesktopEvent::ServiceFrame(ServiceFrame { bytes, answer })).is_err() {
                    return;
                }
                let (frame, disconnect) = match reply.recv() {
                    Ok(value) => value,
                    Err(_) => return,
                };
                let answer_wait_ms = received_at.elapsed().as_millis().min(u64::MAX as u128) as u64;
                if let Err(error) = pipe.write_frame(&frame) {
                    let (native_error, win32_error) = match &error {
                        gogoke_native_host::ipc::PrivateIpcError::Os { source, .. } => {
                            ("OS", source.raw_os_error())
                        }
                        _ => ("OTHER", None),
                    };
                    let summary = format!(
                        "GOGOKE_NATIVE_SERVICE_WRITE_DIAGNOSTIC schema=v1 hostPid={} connectionSeq={} requestSeq={} operation={} phase=REPLY_WRITE receivedUtcMs={} failureUtcMs={} answerWaitMs={} nativeError={} win32={}",
                        std::process::id(), current_connection_seq, request_seq, operation,
                        received_utc_ms, utc_now_ms(), answer_wait_ms, native_error,
                        win32_error.map_or_else(|| "none".to_owned(), |code| code.to_string()),
                    );
                    if FIRST_SERVICE_WRITE_FAILURE.set(summary).is_ok() {
                        eprintln!("{}", FIRST_SERVICE_WRITE_FAILURE.get().expect("first write failure stored"));
                    }
                    // The completed dispatch is not retried. A closed Node
                    // connection does not revoke the independent USER owner;
                    // take the same disconnect/re-authentication path as EOF.
                    if service_reply_peer_closed(&error) { break; }
                    drop(service_events.send(DesktopEvent::ServiceFailed(error.to_string())));
                    return;
                }
                if disconnect {
                    if let Err(error) = pipe.flush_for_disconnect() {
                        let _ = service_events.send(DesktopEvent::ServiceFailed(error.to_string()));
                    }
                    break;
                }
            }
            drop(pipe);
            if service_events.send(DesktopEvent::ServiceDisconnected).is_err() { break; }
        }
    });
    let user_events = sender.clone();
    std::thread::spawn(move || {
        let mut pipe = match user_listener.accept_user() {
            Ok(pipe) => pipe,
            Err(error) => {
                let _ = user_events.send(DesktopEvent::UserFailed(error.to_string()));
                return;
            }
        };
        let proof = match pipe.take_user_origin_proof() {
            Some(proof) => proof,
            None => {
                let _ = user_events.send(DesktopEvent::UserFailed("missing User origin proof".into()));
                return;
            }
        };
        if user_events.send(DesktopEvent::UserConnected(proof)).is_err() { return; }
        loop {
            let bytes = match pipe.read_frame() {
                Ok(bytes) => bytes,
                Err(_) => break,
            };
            let (answer, reply) = mpsc::sync_channel(0);
            if user_events.send(DesktopEvent::UserFrame { bytes, answer }).is_err() { return; }
            let frame = match reply.recv() {
                Ok(frame) => frame,
                Err(_) => return,
            };
            if pipe.write_frame(&frame).is_err() { break; }
        }
        let _ = user_events.send(DesktopEvent::UserDisconnected);
    });
    drop(sender);
    let mut service = None;
    let mut user: Option<UserOriginProof> = None;
    let mut service_listening = false;
    let mut service_ready_reply: Option<SyncSender<Vec<u8>>> = None;
    loop {
        // The native protocol reader is synchronous and nonblocking. Keep
        // its capture/dispatch on this one DB authority thread, independently
        // of whether the UI asks for output. No operation is retried here.
        product.pump_native_output()?;
        product.pump_secretary_routines()?;
        let event = match receiver.recv_timeout(std::time::Duration::from_millis(20)) {
            Ok(event) => event,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        match event {
            DesktopEvent::ServiceListening => {
                service_listening = true;
                if let Some(answer) = service_ready_reply.take() {
                    // A delayed response cannot borrow an origin that died
                    // while its pipe reader was waiting for this answer.
                    user.as_ref().ok_or_else(|| io::Error::other("USER_NOT_CONNECTED"))?
                        .verify_live_origin()?;
                    drop(answer.send(br#"{"serviceTransportReady":true}"#.to_vec()));
                }
            }
            DesktopEvent::ServiceConnected => {
                service_listening = false;
                service = Some(product.begin_shared_service_frames(capability)?);
            }
            DesktopEvent::ServiceFrame(frame) => {
                let reply = match service.as_mut() {
                    Some(state) => match product.dispatch_service_frame(state, &frame.bytes) {
                        Ok(reply) => reply,
                        // Keep the shared pipe connected until the client has
                        // read the complete denial. Immediate server-side
                        // disconnect can discard a partially read byte-pipe
                        // reply on Windows (observed Win32 233).
                        Err(error) => (format!("ERR\t{error:?}\t0us").into_bytes(), false),
                    },
                    None => (b"ERR\tSERVICE_NOT_CONNECTED\t0us".to_vec(), true),
                };
                let _ = frame.answer.send(reply);
            }
            DesktopEvent::ServiceDisconnected => service = None,
            DesktopEvent::ServiceFailed(error) => return Err(io::Error::other(error).into()),
            DesktopEvent::UserConnected(proof) => {
                user = Some(proof);
                product.start_secretary_resident();
            },
            DesktopEvent::UserFrame { bytes, answer } => {
                // Internal shell transport synchronization on the already
                // object-verified User connection, not a ledger operation.
                if user.is_some() && bytes == br#"{"operation":"ServiceTransportReady"}"# {
                    user.as_ref().expect("connected User checked").verify_live_origin()?;
                    if service_listening {
                        drop(answer.send(br#"{"serviceTransportReady":true}"#.to_vec()));
                    } else if service_ready_reply.is_none() {
                        service_ready_reply = Some(answer);
                    } else {
                        drop(answer.send(b"ERR\tSERVICE_READY_ALREADY_PENDING".to_vec()));
                    }
                    continue;
                }
                let reply = match user.as_ref() {
                    Some(proof) => product.dispatch_user_frame(proof, &bytes)
                        .unwrap_or_else(|error| format!("ERR\t{error:?}").into_bytes()),
                    None => b"ERR\tUSER_NOT_CONNECTED".to_vec(),
                };
                let _ = answer.send(reply);
            }
            DesktopEvent::UserDisconnected => break,
            DesktopEvent::UserFailed(error) => return Err(io::Error::other(error).into()),
        }
    }
    Ok(())
}

#[cfg(windows)]
#[link(name = "bcrypt")]
unsafe extern "system" {
    fn BCryptGenRandom(algorithm: *mut c_void, buffer: *mut u8, length: u32, flags: u32) -> i32;
}

#[cfg(windows)]
fn service_capability() -> io::Result<String> {
    let mut bytes = [0u8; 32];
    // System-preferred OS RNG only; failure has no predictable fallback.
    let status = unsafe { BCryptGenRandom(std::ptr::null_mut(), bytes.as_mut_ptr(), bytes.len() as u32, 2) };
    if status != 0 {
        return Err(io::Error::new(io::ErrorKind::Other, "service capability RNG failed"));
    }
    let mut value = String::with_capacity(64);
    for byte in bytes {
        write!(&mut value, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(value)
}

#[cfg(not(windows))]
fn service_capability() -> io::Result<String> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "native service capability requires Windows"))
}

fn explicit_arguments() -> Result<(PathBuf, PathBuf, HostMode), RootLockError> {
    let mut args = std::env::args_os().skip(1);
    match (
        args.next(),
        args.next(),
        args.next(),
        args.next(),
        args.next(),
        args.next(),
    ) {
        (Some(root_flag), Some(root), Some(db_flag), Some(database), None, None)
            if root_flag == "--root" && db_flag == "--database" =>
        {
            let root = PathBuf::from(root);
            let database = PathBuf::from(database);
            if !database.starts_with(&root) {
                return Err(RootLockError::MissingRoot);
            }
            Ok((root, database, HostMode::Legacy))
        }
        (Some(flag), Some(path), None, None, None, None) if flag == "--root" => {
            let root = PathBuf::from(path);
            let database = root.join("state.sqlite");
            Ok((root, database, HostMode::Legacy))
        }
        (Some(flag), Some(path), Some(mode), Some(pid_flag), Some(pid), None)
            if flag == "--root" && mode == "--desktop-session" && pid_flag == "--user-pid" => {
            let pid = pid.to_str().and_then(|text| text.parse::<u32>().ok())
                .filter(|value| *value != 0).ok_or(RootLockError::MissingRoot)?;
            let root = PathBuf::from(path);
            let database = root.join("state.sqlite");
            Ok((root, database, HostMode::Desktop { user_pid: pid }))
        }
        _ => Err(RootLockError::MissingRoot),
    }
}

#[allow(dead_code)]
fn database_is_direct_child(root: &Path, database: &Path) -> bool {
    database.parent() == Some(root)
}

#[cfg(test)]
mod desktop_shutdown_tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn outcome(error: Option<&str>) -> DesktopRunResult {
        error.map_or(Ok(()), |message| Err(io::Error::other(message).into()))
    }

    fn exercise(desktop: Option<&str>, stopped: Option<&str>, closed: Option<&str>)
        -> (Vec<&'static str>, DesktopRunResult) {
        let trace = Rc::new(RefCell::new(Vec::new()));
        let result = finish_desktop_run(trace.clone(), outcome(desktop),
            |trace| { trace.borrow_mut().push("stop"); outcome(stopped) },
            |trace| { trace.borrow_mut().push("close"); outcome(closed) });
        let calls = trace.borrow().clone();
        (calls, result)
    }

    #[test]
    fn desktop_loop_failure_still_stops_and_closes() {
        let (calls, result) = exercise(Some("UserProcessNotLive"), None, None);
        assert_eq!(calls, ["stop", "close"]);
        assert_eq!(result.unwrap_err().to_string(), "UserProcessNotLive");

        for code in [109, 232, 233] {
            let error = gogoke_native_host::ipc::PrivateIpcError::Os {
                operation: "WriteFile(private IPC frame)", source: io::Error::from_raw_os_error(code),
            };
            assert!(service_reply_peer_closed(&error));
        }
        for (operation, code) in [("WriteFile(private IPC frame)", 5),
            ("WriteFile(private IPC frame)", 995), ("ReadFile(private IPC header)", 232)] {
            let error = gogoke_native_host::ipc::PrivateIpcError::Os {
                operation, source: io::Error::from_raw_os_error(code),
            };
            assert!(!service_reply_peer_closed(&error));
        }

        let first: OnceLock<String> = OnceLock::new();
        let original = format!("operation={} phase=REPLY_WRITE win32=232",
            service_operation_class(br#"{"operation":"ReadEvents"}"#));
        let later = format!("operation={} win32=5",
            service_operation_class(br#"{"operation":"secret-private-value"}"#));
        assert!(first.set(original).is_ok());
        assert!(first.set(later).is_err());
        let line = first.get().expect("first diagnostic retained");
        assert!(line.contains("operation=OTHER_ADMITTED"));
        assert!(line.contains("phase=REPLY_WRITE"));
        assert!(line.contains("win32=232"));
        assert_eq!(
            service_operation_class(br#"{"operation":"secret-private-value"}"#),
            "UNCLASSIFIED",
        );
        assert_eq!(
            service_operation_class(br#"{"operation":"AdmitControllerCaller"}"#),
            "AdmitControllerCaller",
        );
    }

    #[test]
    fn desktop_loop_success_stops_then_closes() {
        let (calls, result) = exercise(None, None, None);
        assert_eq!(calls, ["stop", "close"]);
        assert!(result.is_ok());
    }

    #[test]
    fn desktop_loop_primary_and_cleanup_failures_are_retained() {
        let (calls, result) = exercise(Some("original transport error"),
            Some("original stop refusal"), Some("original database close error"));
        assert_eq!(calls, ["stop", "close"]);
        let error = result.unwrap_err().to_string();
        for original in ["original transport error", "original stop refusal", "original database close error"] {
            assert!(error.contains(original), "original error lost: {error}");
        }
    }

    #[test]
    fn desktop_loop_stop_failure_still_attempts_checked_close() {
        let (calls, result) = exercise(None, Some("original stop refusal"),
            Some("original database close error"));
        assert_eq!(calls, ["stop", "close"]);
        let error = result.unwrap_err().to_string();
        assert!(error.contains("original stop refusal") && error.contains("original database close error"));
    }
}
