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

#[derive(Clone, Copy)]
enum HostMode {
    Legacy,
    Desktop { user_pid: u32 },
}

struct ServiceFrame {
    bytes: Vec<u8>,
    answer: SyncSender<(Vec<u8>, bool)>,
}

enum DesktopEvent {
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
        run_desktop(&mut product, listener, user, &endpoint, &service_capability)?;
        product.close_checked()?;
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
            if should_stop { break; }
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
            let pipe = match listener.accept_current_user() {
                Ok(pipe) => pipe,
                Err(error) => {
                    let _ = service_events.send(DesktopEvent::ServiceFailed(error.to_string()));
                    break;
                }
            };
            if service_events.send(DesktopEvent::ServiceConnected).is_err() { break; }
            loop {
                let bytes = match pipe.read_frame() {
                    Ok(bytes) => bytes,
                    Err(_) => break,
                };
                let (answer, reply) = mpsc::sync_channel(0);
                if service_events.send(DesktopEvent::ServiceFrame(ServiceFrame { bytes, answer })).is_err() {
                    return;
                }
                let (frame, disconnect) = match reply.recv() {
                    Ok(value) => value,
                    Err(_) => return,
                };
                if pipe.write_frame(&frame).is_err() || disconnect { break; }
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
    let mut user = None;
    for event in receiver {
        match event {
            DesktopEvent::ServiceConnected => {
                service = Some(product.begin_shared_service_frames(capability)?);
            }
            DesktopEvent::ServiceFrame(frame) => {
                let reply = match service.as_mut() {
                    Some(state) => match product.dispatch_service_frame(state, &frame.bytes) {
                        Ok(reply) => reply,
                        Err(error) => (format!("ERR\t{error:?}\t0us").into_bytes(), true),
                    },
                    None => (b"ERR\tSERVICE_NOT_CONNECTED\t0us".to_vec(), true),
                };
                let _ = frame.answer.send(reply);
            }
            DesktopEvent::ServiceDisconnected => service = None,
            DesktopEvent::ServiceFailed(error) => return Err(io::Error::other(error).into()),
            DesktopEvent::UserConnected(proof) => user = Some(proof),
            DesktopEvent::UserFrame { bytes, answer } => {
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
